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
//! does not refuse the start. Every rail defaults to `disabled`, which needs
//! nothing and runs nothing.
//!
//! The operator routes are admin-only and copy the admin drill routes' auth
//! shape (`authenticate_with_tenant_access_grant`, then `require_admin`):
//!
//! - `POST /v1/admin/payout-rails/{rail_id}/halt` and `.../resume` set and
//!   clear that rail's kill switch. Each appends a control row naming the
//!   operator by hash; there is no global halt.
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
    PayoutAuditAction, PayoutLinkState, PayoutLinkTransition, PayoutStore, payout_link_id_hash,
};
use trace_commons_server::near_ai_rail_config::NearAiRailConfig;
use trace_commons_server::payout_pass::UnconfiguredRecipientOpener;
use trace_commons_server::payout_rate::{NEAR_AI_PAYOUT_RAIL_LABEL, near_ai_rate_epoch_at_boot};
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
    let rails = PayoutRails::from_config(&near_ai)
        .map_err(|error| anyhow::anyhow!("near_ai payout refused to start: {}", error.label()))?;
    if let Some(store) = payout_store {
        for entry in rails.entries() {
            if entry.mode() == trace_commons_server::payout_rate::PayoutMode::Disabled {
                continue;
            }
            // The version label of the latest epoch, for readiness; never
            // the rate.
            let latest = store
                .payout_rate_epochs(entry.rail_id())
                .await
                .map_err(|_| {
                    anyhow::anyhow!("near_ai payout refused to start: payout_rate_epoch_unread")
                })?
                .into_iter()
                .max_by_key(|epoch| epoch.observed_at)
                .map(|epoch| epoch.rate.version().to_owned());
            rails.set_rate_epoch(entry.rail_id().as_str(), latest);
        }
    }
    rails.startup_probe().await;
    for entry in rails.entries() {
        tracing::info!(
            rail = %entry.rail_id(),
            mode = entry.mode().as_label(),
            qualified = entry.qualified(),
            "payout rail configured"
        );
    }
    Ok(rails)
}

/// One worker per enabled rail. Without a payout store or a database
/// mirror nothing runs (and an enabled rail has already refused the start
/// without a store). Recipients are sealed by the link routes, which this
/// build does not have yet, so the opener opens nothing: any link found is
/// held `payout_link_unreadable` and nothing is sent to it.
pub(crate) fn spawn_payout_rail_workers(state: &Arc<AppState>) -> Vec<tokio::task::JoinHandle<()>> {
    let (Some(payouts), Some(corpus)) = (state.payout_store.clone(), state.db_mirror.clone())
    else {
        return Vec::new();
    };
    state
        .payout_rails
        .spawn_workers(payouts, corpus, Arc::new(UnconfiguredRecipientOpener))
}

/// The operator's hash on control and audit rows. Hash-only: never the
/// principal or the token.
fn operator_actor_hash(auth: &TenantAuth) -> String {
    sha256_prefixed(&format!("payout-operator-v1:{}", auth.principal_ref))
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
    let auth = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_admin(&auth)?;
    let entry = state
        .payout_rails
        .get(&rail_id)
        .map_err(|error| api_error(StatusCode::NOT_FOUND, error.label()))?;
    let store = state
        .payout_store
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "payout_store_unavailable"))?;
    let label = if halted {
        PAYOUT_HALT_LABEL
    } else {
        PAYOUT_RESUME_LABEL
    };
    store
        .set_payout_control(
            entry.rail_id(),
            halted,
            Some(label),
            &operator_actor_hash(&auth),
        )
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

fn is_link_id_hash(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

fn is_reason_label(value: &str) -> bool {
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
        assert!(is_reason_label("contributor_lost_org_access"));
        for bad in ["", "Lost", "lost access", "a-b", &"a".repeat(65)] {
            assert!(!is_reason_label(bad), "{bad}");
        }
    }
}
