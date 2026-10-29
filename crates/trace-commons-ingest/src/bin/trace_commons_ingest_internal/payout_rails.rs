// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The payout rails in ingest (docs/superpowers/specs/2026-10-21-nearai-
//! credit-payout-design.md, sections 4 and 7): boot, the per-rail workers,
//! and the operator routes.
//!
//! Boot reads each rail's settings fail-closed (a rail in `dry_run` or
//! `live` with a setting missing or malformed refuses the start), records
//! the rail's rate epoch, builds the rails, and probes each `live` rail's
//! capacity once; a failed probe marks the rail unqualified in readiness and
//! does not refuse the start. An enabled rail whose rate source has no rate
//! (no settlement table, or one without a positive rate) does not refuse the
//! start either: refusing would take every route down for one rail. It runs
//! no rate instead -- its pass stops as `payout_rate_unavailable` before
//! accrual or any call, and readiness reports it unqualified with that
//! reason -- whatever epochs an earlier boot stored. Every rail defaults to
//! `disabled`, which needs nothing and runs nothing.
//!
//! The rail-wide controls are the operator's alone. They accept only the
//! deployment's payout control token (`TRACE_COMMONS_PAYOUT_CONTROL_TOKEN`),
//! never a tenant token -- a tenant admin's included -- and refuse every
//! caller while it is unset. Boot refuses a `live` rail without it
//! (`payout_control_token_unconfigured`), so a live rail can always be
//! halted; `dry_run` sends nothing and boots without it:
//!
//! - `POST /v1/admin/payout-rails/{rail_id}/halt` and `.../resume` set and
//!   clear that rail's kill switch. Each appends a control row naming the
//!   operator by hash; there is no global halt.
//!
//! - `GET /v1/admin/payout-rails/{rail_id}/tenants/{tenant_id}/holds` lists
//!   that tenant's open holds on the rail, each as an account id hash and a
//!   label, and `POST .../holds/clear` with `{account_id_hash, label}`
//!   clears one, with an audit row naming the operator by hash. A hold
//!   about one account stays until it is cleared here; the ceiling hold
//!   also lifts itself once the account fits the ceiling again. The
//!   operator token is deployment-wide, so the tenant is named in the path;
//!   every read and write still runs in that tenant's RLS context.
//!
//! Link release moves one link of the caller's own tenant, so it keeps the
//! admin drill routes' tenant auth (`authenticate_with_tenant_access_grant`,
//! then `require_admin`):
//!
//! - `POST /v1/admin/payout-rails/{rail_id}/links/{link_id_hash}/release`
//!   moves one of the operator tenant's active links on that rail to
//!   `closing` (for example after a contributor reports lost access to the
//!   recipient), with an audit row naming the operator by hash and a reason
//!   label; the rail's pass then closes it by reads.
//!
//! An unknown rail id is refused (`payout_rail_unknown`), never defaulted.
//! Responses and logs carry the rail id, labels and hashes only.

use super::*;
use axum::extract::Path;
use trace_commons_server::db::{
    PayoutAuditAction, PayoutLinkState, PayoutLinkTransition, PayoutStore, payout_account_id_hash,
    payout_link_id_hash,
};
use trace_commons_server::near_ai_rail_config::{NearAiRailConfig, NearAiReferralConfig};
use trace_commons_server::payout_pass::{PayoutRecipientOpener, UnconfiguredRecipientOpener};
use trace_commons_server::payout_rate::{
    NEAR_AI_PAYOUT_RAIL_LABEL, near_ai_rate_epoch_at_boot, near_ai_rate_version_in_force,
};
use trace_commons_server::payout_recipient_seal::SealedRecipientOpener;
use trace_commons_server::payout_worker::PayoutRails;

const PAYOUT_HALT_LABEL: &str = "operator_halt";
const PAYOUT_RESUME_LABEL: &str = "operator_resume";
const PAYOUT_RELEASE_LABEL: &str = "operator_release";

/// Boots the payout rails from the environment. Refuses the start on a
/// malformed setting, a rate version re-installed with another rate, or an
/// enabled rail with no payout store.
pub(crate) async fn payout_rails_from_env(
    settlement_table: Option<&trace_commons_protocol::settlement_estimate::SettlementEstimateTable>,
    payout_store: Option<&dyn PayoutStore>,
) -> anyhow::Result<PayoutRails> {
    let near_ai = NearAiRailConfig::from_env().map_err(|error| {
        anyhow::anyhow!(
            "near_ai payout refused to start: {} ({})",
            error.label(),
            error.setting()
        )
    })?;
    match near_ai_rate_epoch_at_boot(near_ai.mode(), settlement_table, payout_store).await {
        Ok(Some(outcome)) => tracing::info!(
            rail = NEAR_AI_PAYOUT_RAIL_LABEL,
            outcome = outcome.as_label(),
            "payout rate epoch at boot"
        ),
        Ok(None) => {}
        Err(error) => anyhow::bail!("near_ai payout refused to start: {}", error.label()),
    }
    // The referral stream (referral rewards spec section 5): `disabled`
    // unless set, and refused at boot when `live` without the rail `live`
    // and its own credential (`near_ai_referral_config_missing`).
    let referral = NearAiReferralConfig::from_env(&near_ai).map_err(|error| {
        anyhow::anyhow!(
            "near_ai referral payout refused to start: {} ({})",
            error.label(),
            error.setting()
        )
    })?;
    let rails = PayoutRails::from_configs(&near_ai, &referral)
        .map_err(|error| anyhow::anyhow!("near_ai payout refused to start: {}", error.label()))?;
    // The version of the rate in force now, from the rate source this boot
    // recorded, for readiness and for the pass. Never the latest stored
    // epoch: a rail whose source is gone must not keep paying on a rate an
    // earlier boot left behind (it stops as `payout_rate_unavailable`).
    if near_ai.mode() != trace_commons_server::payout_rate::PayoutMode::Disabled {
        let version = near_ai_rate_version_in_force(settlement_table).map_err(|error| {
            anyhow::anyhow!("near_ai payout refused to start: {}", error.label())
        })?;
        if version.is_none() {
            tracing::warn!(
                rail = NEAR_AI_PAYOUT_RAIL_LABEL,
                label = trace_commons_server::payout_pass::PAYOUT_RATE_UNAVAILABLE,
                "payout rail has no rate in force; it accrues and sends nothing"
            );
        }
        rails.set_rate_epoch(NEAR_AI_PAYOUT_RAIL_LABEL, version);
    }
    rails.startup_probe().await;
    for entry in rails.entries() {
        tracing::info!(
            rail = %entry.rail_id(),
            mode = entry.mode().as_label(),
            referral_mode = entry.referral_mode().as_label(),
            qualified = entry.qualified(),
            "payout rail configured"
        );
    }
    Ok(rails)
}

/// Refuses the start when a rail is enabled but the recipient key (the
/// NEAR account identity's pepper and sealing key, which the link routes
/// seal recipients with) is not configured: no link could be made, and
/// none already stored could be opened.
pub(crate) fn require_payout_recipient_key(state: &AppState) -> anyhow::Result<()> {
    let enabled = state
        .payout_rails
        .entries()
        .any(|entry| entry.mode() != trace_commons_server::payout_rate::PayoutMode::Disabled);
    if enabled && state.near_account_identity.is_none() {
        anyhow::bail!("near_ai payout refused to start: payout_recipient_key_unconfigured");
    }
    Ok(())
}

/// The opener the pass uses: the recipient key when the deployment has it,
/// else one that opens nothing (any link found is held
/// `payout_link_unreadable`; an enabled rail without the key has already
/// refused the start).
fn recipient_opener(state: &AppState) -> Arc<dyn PayoutRecipientOpener> {
    match state.near_account_identity.clone() {
        Some(identity) => Arc::new(SealedRecipientOpener::new(identity)),
        None => Arc::new(UnconfiguredRecipientOpener),
    }
}

/// One worker per enabled rail. Without a payout store or a database
/// mirror nothing runs (and an enabled rail has already refused the start
/// without a store). Recipients are opened with the key the link routes
/// sealed them under.
pub(crate) fn spawn_payout_rail_workers(state: &Arc<AppState>) -> Vec<tokio::task::JoinHandle<()>> {
    let (Some(payouts), Some(corpus)) = (state.payout_store.clone(), state.db_mirror.clone())
    else {
        return Vec::new();
    };
    state
        .payout_rails
        .spawn_workers(payouts, corpus, recipient_opener(state))
}

/// The operator's hash on audit rows a tenant admin's action writes (link
/// release). Hash-only: never the principal or the token.
fn operator_actor_hash(auth: &TenantAuth) -> String {
    sha256_prefixed(&format!("payout-operator-v1:{}", auth.principal_ref))
}

/// The setting holding the operator's payout control token: the one
/// credential the rail-wide controls accept. It is deployment-wide, not a
/// tenant token, so no tenant (and no tenant admin) can mint it.
pub(crate) const TRACE_COMMONS_PAYOUT_CONTROL_TOKEN: &str = "TRACE_COMMONS_PAYOUT_CONTROL_TOKEN";

/// The shortest control token boot accepts.
const PAYOUT_CONTROL_TOKEN_MIN_LEN: usize = 32;

/// The stored form of a control token: its sha256, never the token.
pub(crate) fn payout_control_token_hash(token: &str) -> String {
    sha256_prefixed(&format!("payout-control-token-v1:{token}"))
}

/// The label boot refuses (any rail `live`) or logs (a rail in `dry_run`)
/// when [`TRACE_COMMONS_PAYOUT_CONTROL_TOKEN`] is unset.
const PAYOUT_CONTROL_TOKEN_UNCONFIGURED: &str = "payout_control_token_unconfigured";

/// Reads [`TRACE_COMMONS_PAYOUT_CONTROL_TOKEN`] at boot, against the modes
/// the rails booted in. See [`payout_control_token_hash_for_modes`].
pub(crate) fn payout_control_token_hash_from_env(
    rails: &PayoutRails,
) -> anyhow::Result<Option<String>> {
    let token = optional_trimmed_env(TRACE_COMMONS_PAYOUT_CONTROL_TOKEN)?;
    payout_control_token_hash_for_modes(token.as_deref(), rails.entries().map(|entry| entry.mode()))
}

/// The control token's stored hash, checked against the rails' modes. A
/// token shorter than 32 characters refuses the start rather than guarding
/// the kill switch weakly. Unset refuses the start when any rail is `live`
/// (`payout_control_token_unconfigured`): a live rail must be haltable. A
/// rail in `dry_run` sends nothing, so it boots without one and logs the
/// label; `disabled` needs nothing. Unset is `None`, and the controls then
/// refuse every caller.
pub(crate) fn payout_control_token_hash_for_modes(
    token: Option<&str>,
    modes: impl IntoIterator<Item = trace_commons_server::payout_rate::PayoutMode>,
) -> anyhow::Result<Option<String>> {
    use trace_commons_server::payout_rate::PayoutMode;
    if let Some(token) = token {
        if token.len() < PAYOUT_CONTROL_TOKEN_MIN_LEN {
            anyhow::bail!("payout control refused to start: payout_control_token_too_short");
        }
        return Ok(Some(payout_control_token_hash(token)));
    }
    let modes: Vec<PayoutMode> = modes.into_iter().collect();
    if modes.contains(&PayoutMode::Live) {
        anyhow::bail!("payout control refused to start: {PAYOUT_CONTROL_TOKEN_UNCONFIGURED}");
    }
    if modes.contains(&PayoutMode::DryRun) {
        tracing::warn!(
            label = PAYOUT_CONTROL_TOKEN_UNCONFIGURED,
            "payout rail in dry_run without a control token; halt and resume refuse every caller"
        );
    }
    Ok(None)
}

/// Equal-length comparison that does not stop at the first differing byte.
fn same_digest(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .fold(0_u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

/// The gate on every rail-wide payout control (halt, resume, hold list and
/// clear): the operator's control token and nothing else. It never consults
/// tenant tokens, so a tenant admin is refused like anyone else. Unset
/// refuses every caller (`payout_control_unconfigured`). Returns the
/// operator's actor hash for audit rows: a hash of the token's hash.
pub(crate) fn require_payout_operator(state: &AppState, headers: &HeaderMap) -> ApiResult<String> {
    let Some(expected) = state.payout_control_token_hash.as_deref() else {
        return Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "payout_control_unconfigured",
        ));
    };
    let token = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "missing bearer token"))?;
    let presented = payout_control_token_hash(token);
    if !same_digest(&presented, expected) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "payout_control_token_required",
        ));
    }
    Ok(sha256_prefixed(&format!("payout-operator-v2:{expected}")))
}

#[derive(Debug, Serialize)]
pub(crate) struct PayoutRailControlResponse {
    rail_id: String,
    halted: bool,
}

pub(crate) async fn payout_rail_halt_handler(
    State(state): State<Arc<AppState>>,
    Path(rail_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Json<PayoutRailControlResponse>> {
    set_payout_rail_control(state, rail_id, headers, true).await
}

pub(crate) async fn payout_rail_resume_handler(
    State(state): State<Arc<AppState>>,
    Path(rail_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Json<PayoutRailControlResponse>> {
    set_payout_rail_control(state, rail_id, headers, false).await
}

async fn set_payout_rail_control(
    state: Arc<AppState>,
    rail_id: String,
    headers: HeaderMap,
    halted: bool,
) -> ApiResult<Json<PayoutRailControlResponse>> {
    let actor = require_payout_operator(state.as_ref(), &headers)?;
    let label = if halted {
        PAYOUT_HALT_LABEL
    } else {
        PAYOUT_RESUME_LABEL
    };
    if rail_id == trace_commons_server::referral_bonus::NEAR_AI_REFERRAL_CONTROL_RAIL {
        return set_referral_stream_control(state, actor, halted, label).await;
    }
    let entry = state
        .payout_rails
        .get(&rail_id)
        .map_err(|error| api_error(StatusCode::NOT_FOUND, error.label()))?;
    let store = state
        .payout_store
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "payout_store_unavailable"))?;
    store
        .set_payout_control(entry.rail_id(), halted, Some(label), &actor)
        .await
        .map_err(|_| api_error(StatusCode::SERVICE_UNAVAILABLE, "payout_control_unrecorded"))?;
    entry.record_halted(halted);
    tracing::info!(rail = %entry.rail_id(), halted, label, "payout rail control set");
    if !halted {
        // Pending lines resume now rather than at the next tick.
        state.payout_rails.notifier().notify_all();
    }
    Ok(Json(PayoutRailControlResponse {
        rail_id: entry.rail_id().as_str().to_owned(),
        halted,
    }))
}

/// The referral stream's pseudo-rail (`near_ai.referral`, referral spec
/// section 7): the bonus pass halts it when too many codes are suspended in
/// a day, which runs the stream as `dry_run`, and this is how the operator
/// resumes it (or halts it by hand). The same token and the same
/// hash-only control row as a rail's halt and resume. Unknown, like any
/// rail id, while the referral stream is not attached.
async fn set_referral_stream_control(
    state: Arc<AppState>,
    actor: String,
    halted: bool,
    label: &'static str,
) -> ApiResult<Json<PayoutRailControlResponse>> {
    let hook = state.payout_rails.referral_bonus().ok_or_else(|| {
        api_error(
            StatusCode::NOT_FOUND,
            trace_commons_server::payout_worker::PayoutRailsError::Unknown.label(),
        )
    })?;
    let control = trace_commons_server::referral_bonus::near_ai_referral_control_rail();
    hook.payouts()
        .set_payout_control(&control, halted, Some(label), &actor)
        .await
        .map_err(|_| api_error(StatusCode::SERVICE_UNAVAILABLE, "payout_control_unrecorded"))?;
    hook.record_operator_control(halted);
    tracing::info!(rail = %control, halted, label, "referral stream control set");
    Ok(Json(PayoutRailControlResponse {
        rail_id: control.as_str().to_owned(),
        halted,
    }))
}

/// The shape a tenant id takes in a hold route's path.
pub(crate) fn is_tenant_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'))
}

/// The database's rule for a hold label (`trace_payout_holds.label`).
fn is_hold_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.' | b'-')
        })
}

#[derive(Debug, Serialize)]
pub(crate) struct PayoutHoldView {
    account_id_hash: String,
    label: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct PayoutHoldsResponse {
    rail_id: String,
    holds: Vec<PayoutHoldView>,
}

/// The operator gate, the rail, the tenant shape and the store, in that
/// order, for both hold routes.
fn payout_hold_route<'a>(
    state: &'a AppState,
    headers: &HeaderMap,
    rail_id: &str,
    tenant_id: &str,
) -> ApiResult<(
    String,
    &'a Arc<trace_commons_server::payout_worker::PayoutRailEntry>,
    &'a Arc<dyn PayoutStore>,
)> {
    let actor = require_payout_operator(state, headers)?;
    let entry = state
        .payout_rails
        .get(rail_id)
        .map_err(|error| api_error(StatusCode::NOT_FOUND, error.label()))?;
    if !is_tenant_id(tenant_id) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "payout_tenant_id_invalid",
        ));
    }
    let store = state
        .payout_store
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "payout_store_unavailable"))?;
    Ok((actor, entry, store))
}

pub(crate) async fn payout_holds_list_handler(
    State(state): State<Arc<AppState>>,
    Path((rail_id, tenant_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Json<PayoutHoldsResponse>> {
    let (_, entry, store) = payout_hold_route(state.as_ref(), &headers, &rail_id, &tenant_id)?;
    let holds = store
        .open_payout_holds(&tenant_id, entry.rail_id())
        .await
        .map_err(|_| api_error(StatusCode::SERVICE_UNAVAILABLE, "payout_store_unavailable"))?
        .into_iter()
        .map(|hold| PayoutHoldView {
            account_id_hash: payout_account_id_hash(hold.account_id),
            label: hold.label,
        })
        .collect();
    Ok(Json(PayoutHoldsResponse {
        rail_id: entry.rail_id().as_str().to_owned(),
        holds,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PayoutHoldClearRequest {
    account_id_hash: String,
    label: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct PayoutHoldClearResponse {
    rail_id: String,
    account_id_hash: String,
    label: String,
    cleared: bool,
}

pub(crate) async fn payout_hold_clear_handler(
    State(state): State<Arc<AppState>>,
    Path((rail_id, tenant_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<PayoutHoldClearRequest>,
) -> ApiResult<Json<PayoutHoldClearResponse>> {
    let (actor, entry, store) = payout_hold_route(state.as_ref(), &headers, &rail_id, &tenant_id)?;
    if !is_link_id_hash(&request.account_id_hash) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "payout_account_id_hash_invalid",
        ));
    }
    if !is_hold_label(&request.label) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "payout_hold_label_invalid",
        ));
    }
    let not_found = || api_error(StatusCode::NOT_FOUND, "payout_hold_not_found");
    let account = store
        .open_payout_holds(&tenant_id, entry.rail_id())
        .await
        .map_err(|_| api_error(StatusCode::SERVICE_UNAVAILABLE, "payout_store_unavailable"))?
        .into_iter()
        .find(|hold| {
            hold.label == request.label
                && payout_account_id_hash(hold.account_id) == request.account_id_hash
        })
        .map(|hold| hold.account_id)
        .ok_or_else(not_found)?;
    let cleared = store
        .clear_payout_hold(&tenant_id, account, entry.rail_id(), &request.label, &actor)
        .await
        .map_err(|_| api_error(StatusCode::SERVICE_UNAVAILABLE, "payout_store_unavailable"))?;
    if !cleared {
        return Err(not_found());
    }
    tracing::info!(
        rail = %entry.rail_id(),
        label = %request.label,
        "payout hold cleared by an operator"
    );
    // The account may be paid now rather than at the next tick.
    state.payout_rails.notifier().notify_all();
    Ok(Json(PayoutHoldClearResponse {
        rail_id: entry.rail_id().as_str().to_owned(),
        account_id_hash: request.account_id_hash,
        label: request.label,
        cleared: true,
    }))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PayoutLinkReleaseRequest {
    /// A safe label saying why; `operator_release` when absent.
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct PayoutLinkReleaseResponse {
    rail_id: String,
    link_id_hash: String,
    state: &'static str,
}

pub(crate) fn is_link_id_hash(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

pub(crate) fn is_reason_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

pub(crate) async fn payout_link_release_handler(
    State(state): State<Arc<AppState>>,
    Path((rail_id, link_id_hash)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<PayoutLinkReleaseRequest>,
) -> ApiResult<Json<PayoutLinkReleaseResponse>> {
    let auth = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_admin(&auth)?;
    let entry = state
        .payout_rails
        .get(&rail_id)
        .map_err(|error| api_error(StatusCode::NOT_FOUND, error.label()))?;
    if !is_link_id_hash(&link_id_hash) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "payout_link_id_hash_invalid",
        ));
    }
    let reason = match request.reason.as_deref() {
        None => PAYOUT_RELEASE_LABEL,
        Some(reason) if is_reason_label(reason) => reason,
        Some(_) => {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "payout_release_reason_invalid",
            ));
        }
    };
    let store = state
        .payout_store
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "payout_store_unavailable"))?;
    // The operator's own tenant only: the auth-derived tenant scopes the
    // read and the move, as on every other route.
    let tenant = auth.tenant_id.as_str();
    let link = store
        .payout_links(tenant, entry.rail_id(), &[PayoutLinkState::Active])
        .await
        .map_err(|_| api_error(StatusCode::SERVICE_UNAVAILABLE, "payout_store_unavailable"))?
        .into_iter()
        .find(|link| payout_link_id_hash(link.link_id) == link_id_hash)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "payout_link_not_found"))?;
    let moved = store
        .transition_payout_link(
            tenant,
            link.link_id,
            PayoutLinkState::Active,
            PayoutLinkState::Closing,
            PayoutAuditAction::LinkReleased,
            &operator_actor_hash(&auth),
            Some(reason),
        )
        .await
        .map_err(|_| api_error(StatusCode::SERVICE_UNAVAILABLE, "payout_store_unavailable"))?;
    if moved != PayoutLinkTransition::Moved {
        return Err(api_error(StatusCode::NOT_FOUND, "payout_link_not_found"));
    }
    tracing::info!(
        rail = %entry.rail_id(),
        link = %link_id_hash,
        reason,
        "payout link released by an operator"
    );
    state.payout_rails.notifier().notify_all();
    Ok(Json(PayoutLinkReleaseResponse {
        rail_id: entry.rail_id().as_str().to_owned(),
        link_id_hash,
        state: PayoutLinkState::Closing.as_str(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_inputs_are_checked_by_shape() {
        assert!(is_link_id_hash(&format!("sha256:{}", "ab".repeat(32))));
        for bad in [
            "",
            "sha256:",
            &format!("sha256:{}", "AB".repeat(32)),
            &format!("sha256:{}", "ab".repeat(31)),
            &"ab".repeat(32),
            &format!("sha256:{}g", "a".repeat(63)),
        ] {
            assert!(!is_link_id_hash(bad), "{bad}");
        }
        assert!(same_digest("sha256:ab", "sha256:ab"));
        assert!(!same_digest("sha256:ab", "sha256:ac"));
        assert!(!same_digest("sha256:ab", "sha256:abc"));
        assert_ne!(
            payout_control_token_hash("a"),
            payout_control_token_hash("b")
        );
        assert!(!payout_control_token_hash("secret-token").contains("secret-token"));
        assert!(is_reason_label("contributor_lost_org_access"));
        for bad in ["", "Lost", "lost access", "a-b", &"a".repeat(65)] {
            assert!(!is_reason_label(bad), "{bad}");
        }
    }

    #[test]
    fn a_live_rail_needs_the_control_token() {
        use trace_commons_server::payout_rate::PayoutMode;
        let token = "t".repeat(32);
        // Live with no token refuses the start: the kill switch would be
        // unreachable.
        let error = payout_control_token_hash_for_modes(None, [PayoutMode::Live])
            .expect_err("live without a token refuses");
        assert!(
            error
                .to_string()
                .contains("payout_control_token_unconfigured"),
            "{error}"
        );
        let error = payout_control_token_hash_for_modes(
            None,
            [PayoutMode::Disabled, PayoutMode::DryRun, PayoutMode::Live],
        )
        .expect_err("any live rail refuses");
        assert!(
            error
                .to_string()
                .contains("payout_control_token_unconfigured"),
            "{error}"
        );
        // A short token refuses whatever the mode.
        for mode in [PayoutMode::Disabled, PayoutMode::DryRun, PayoutMode::Live] {
            let error = payout_control_token_hash_for_modes(Some(&"t".repeat(31)), [mode])
                .expect_err("short token refuses");
            assert!(
                error.to_string().contains("payout_control_token_too_short"),
                "{error}"
            );
        }
        // Dry run and disabled boot without it; the controls refuse callers.
        for mode in [PayoutMode::Disabled, PayoutMode::DryRun] {
            assert_eq!(
                payout_control_token_hash_for_modes(None, [mode]).expect("boots"),
                None
            );
        }
        assert_eq!(
            payout_control_token_hash_for_modes(None, []).expect("no rails boot"),
            None
        );
        // Live with a long enough token boots and stores only its hash.
        let stored = payout_control_token_hash_for_modes(Some(&token), [PayoutMode::Live])
            .expect("live with a token boots")
            .expect("hash stored");
        assert_eq!(stored, payout_control_token_hash(&token));
        assert!(!stored.contains(&token));
    }
}
