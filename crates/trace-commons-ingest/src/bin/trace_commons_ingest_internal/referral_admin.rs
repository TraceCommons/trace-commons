// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The referral bonus in ingest (docs/superpowers/specs/2026-10-29-referral-
//! rewards-design.md, sections 4, 6 and 7): boot, and the operator routes.
//!
//! Boot reads the bonus settings fail-closed
//! ([`ReferralBonusSettings::from_env`]: a malformed setting, an earning
//! period under 92 days, or `live` with only one of the referral
//! credential and its id refuses the start; with neither, `live` sends with
//! the payout credential) and, outside `disabled`, attaches the bonus work to the
//! `near_ai` pass. Outside `disabled`, no database refuses the start
//! (`referral_store_unavailable`). In `live` the payout pass's referral
//! stream sends what is owed; `dry_run` sends nothing. While referrals are
//! on, boot also refuses a login that cannot assume
//! `trace_referral_resolver` ([`require_referral_resolver`]).
//!
//! The operator routes take the payout control token
//! (`TRACE_COMMONS_PAYOUT_CONTROL_TOKEN`) and nothing else, as the payout
//! hold routes do, and refuse every caller while it is unset:
//!
//! - `GET /v1/admin/referrals/suspended`: every suspended code, by tenant
//!   and code index, with its signal labels, referee count and units held.
//! - `POST /v1/admin/referrals/codes/clear` `{tenant_id, code_index,
//!   exempt_labels?}`: clears a suspension; an exempt label never trips the
//!   code again. Lifts `referral_code_suspended` once the owner has no other
//!   suspended code.
//! - `POST /v1/admin/referrals/codes/void` `{tenant_id, code_index,
//!   reason?}`: voids a code and takes back its referees' unsent bonus.
//! - `POST /v1/admin/referrals/void` `{tenant_id, account_id_hash}`: voids
//!   one referral, named by the referee's tenant and account id hash
//!   (`payout_account_id_hash`), in both tenants.
//! - `POST /v1/admin/referrals/dry-run-report`: counts by code state and
//!   signal, total bonus owed, sendable and held, and whether the stream
//!   tripped.
//!
//! A tripped stream is resumed through the payout rail control route,
//! `POST /v1/admin/payout-rails/near_ai.referral/resume` (`payout_rails`).
//!
//! The token is deployment-wide, so a tenant is named in the body; every
//! read and write still runs in that tenant's RLS context, and the list
//! and report enumerate tenants through `trace_referral_resolver` only.
//! Responses and logs carry labels, counts, tenant ids and hashes only.

use super::*;
use trace_commons_server::db::{
    PayoutStore, ReferralBonusStore, ReferralTenantReport, payout_account_id_hash,
};
use trace_commons_server::payout_worker::PayoutRails;
use trace_commons_server::referral_abuse::{REFERRAL_CODE_SUSPENDED, REFERRAL_SIGNAL_LABELS};
use trace_commons_server::referral_bonus::ReferralBonusHook;
use trace_commons_server::referral_config::ReferralBonusSettings;

const REFERRAL_OPERATOR_VOID: &str = "operator_void";

/// Boot's check, while referrals are on (the program has a start, or the
/// bonus is outside `disabled`), that the ingest login can assume
/// `trace_referral_resolver`. Resolving a code and the bonus pass's
/// deployment-wide reads take that role, and V142 grants it to no one: the
/// operator grants it (docs/operator/referral-resolver-role.md). Without
/// the grant every apply and every pass would fail, so the start is
/// refused with a safe label instead. Without a database there is nothing
/// to check: the referral routes refuse on their own, and the bonus already
/// refuses to start (`referral_store_unavailable`).
pub(crate) async fn require_referral_resolver(
    postgres: Option<&PgBackend>,
    referrals_on: bool,
) -> anyhow::Result<()> {
    let Some(postgres) = postgres.filter(|_| referrals_on) else {
        return Ok(());
    };
    match postgres.referral_resolver_assumable().await {
        Ok(true) => Ok(()),
        Ok(false) => anyhow::bail!("referrals refused to start: referral_resolver_unprovisioned"),
        Err(_) => anyhow::bail!("referrals refused to start: referral_resolver_check_failed"),
    }
}

/// Attaches the referral bonus work to `rails` from the environment.
pub(crate) fn with_referral_bonus_from_env(
    rails: PayoutRails,
    postgres: Option<Arc<PgBackend>>,
) -> anyhow::Result<PayoutRails> {
    let settings = ReferralBonusSettings::from_env().map_err(|error| {
        anyhow::anyhow!(
            "referral bonus refused to start: {} ({})",
            error.label(),
            error.setting()
        )
    })?;
    with_referral_bonus(rails, postgres, settings)
}

pub(crate) fn with_referral_bonus(
    rails: PayoutRails,
    postgres: Option<Arc<PgBackend>>,
    settings: ReferralBonusSettings,
) -> anyhow::Result<PayoutRails> {
    use trace_commons_server::payout_rate::PayoutMode;
    if settings.mode == PayoutMode::Disabled {
        return Ok(rails);
    }
    let Some(postgres) = postgres else {
        anyhow::bail!("referral bonus refused to start: referral_store_unavailable");
    };
    let near_ai_mode = rails
        .get("near_ai")
        .map(|entry| entry.mode())
        .unwrap_or(PayoutMode::Disabled);
    if near_ai_mode == PayoutMode::Disabled {
        tracing::warn!(
            mode = settings.mode.as_label(),
            "referral bonus configured while the near_ai rail is disabled; no pass runs it"
        );
    }
    tracing::info!(
        mode = settings.mode.as_label(),
        bps = settings.bps,
        earn_days = settings.earn_days,
        "referral bonus configured"
    );
    let hook = ReferralBonusHook::new(
        postgres.clone() as Arc<dyn ReferralBonusStore>,
        postgres as Arc<dyn PayoutStore>,
        settings,
    );
    Ok(rails.with_referral_bonus(Arc::new(hook)))
}

fn store_unavailable() -> (StatusCode, Json<ApiError>) {
    api_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "referral_store_unavailable",
    )
}

/// The operator gate, then the attached bonus work.
fn referral_operator<'a>(
    state: &'a AppState,
    headers: &HeaderMap,
) -> ApiResult<(String, &'a Arc<ReferralBonusHook>)> {
    let actor = super::payout_rails::require_payout_operator(state, headers)?;
    let hook = state
        .payout_rails
        .referral_bonus()
        .ok_or_else(store_unavailable)?;
    Ok((actor, hook))
}

fn check_tenant(tenant_id: &str) -> ApiResult<()> {
    if super::payout_rails::is_tenant_id(tenant_id) {
        Ok(())
    } else {
        Err(api_error(
            StatusCode::BAD_REQUEST,
            "referral_tenant_id_invalid",
        ))
    }
}

fn check_hash(value: &str, label: &'static str) -> ApiResult<()> {
    if super::payout_rails::is_link_id_hash(value) {
        Ok(())
    } else {
        Err(api_error(StatusCode::BAD_REQUEST, label))
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct ReferralSuspendedView {
    tenant_id: String,
    code_index: String,
    suspension_labels: Vec<String>,
    suspended_at: DateTime<Utc>,
    referees: u64,
    /// Rail atomic units, as text: a `u128` does not fit a JSON number.
    held_units: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ReferralSuspendedResponse {
    rail_id: String,
    codes: Vec<ReferralSuspendedView>,
}

pub(crate) async fn referral_suspended_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<ReferralSuspendedResponse>> {
    let (_, hook) = referral_operator(state.as_ref(), &headers)?;
    let store = hook.store();
    let pairs = store
        .referral_suspended_code_tenants()
        .await
        .map_err(|_| store_unavailable())?;
    let tenants: BTreeSet<String> = pairs.into_iter().map(|(tenant, _)| tenant).collect();
    let mut codes = Vec::new();
    for tenant in tenants {
        for code in store
            .referral_suspended_codes(&tenant, hook.rail_id())
            .await
            .map_err(|_| store_unavailable())?
        {
            codes.push(ReferralSuspendedView {
                tenant_id: tenant.clone(),
                code_index: code.code_index,
                suspension_labels: code.suspension_labels,
                suspended_at: code.suspended_at,
                referees: code.referees,
                held_units: code.held_units.to_string(),
            });
        }
    }
    Ok(Json(ReferralSuspendedResponse {
        rail_id: hook.rail_id().as_str().to_owned(),
        codes,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReferralCodeClearRequest {
    tenant_id: String,
    code_index: String,
    #[serde(default)]
    exempt_labels: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ReferralCodeClearResponse {
    tenant_id: String,
    code_index: String,
    exempt_labels: Vec<String>,
    cleared: bool,
}

pub(crate) async fn referral_code_clear_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<ReferralCodeClearRequest>,
) -> ApiResult<Json<ReferralCodeClearResponse>> {
    let (actor, hook) = referral_operator(state.as_ref(), &headers)?;
    check_tenant(&request.tenant_id)?;
    check_hash(&request.code_index, "referral_code_index_invalid")?;
    let exempt: BTreeSet<String> = request.exempt_labels.iter().cloned().collect();
    if exempt.len() > REFERRAL_SIGNAL_LABELS.len()
        || exempt
            .iter()
            .any(|label| !REFERRAL_SIGNAL_LABELS.contains(&label.as_str()))
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "referral_exempt_label_invalid",
        ));
    }
    let exempt: Vec<String> = exempt.into_iter().collect();
    let tenant = request.tenant_id.as_str();
    let account = hook
        .store()
        .resume_referral_code(tenant, &request.code_index, &exempt, &actor)
        .await
        .map_err(|_| store_unavailable())?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "referral_code_not_suspended"))?;
    // The code-suspended hold lifts once none of the owner's codes is
    // suspended. Other referral holds stay until cleared on their own.
    let still_suspended = hook
        .store()
        .referral_codes(tenant)
        .await
        .map_err(|_| store_unavailable())?
        .iter()
        .any(|code| code.account_id == account && !code.voided && code.suspended_at.is_some());
    if !still_suspended {
        hook.payouts()
            .clear_payout_hold(
                tenant,
                account,
                hook.rail_id(),
                REFERRAL_CODE_SUSPENDED,
                &actor,
            )
            .await
            .map_err(|_| store_unavailable())?;
    }
    tracing::info!(
        exempt = exempt.len(),
        "referral code suspension cleared by an operator"
    );
    Ok(Json(ReferralCodeClearResponse {
        tenant_id: request.tenant_id,
        code_index: request.code_index,
        exempt_labels: exempt,
        cleared: true,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReferralCodeVoidRequest {
    tenant_id: String,
    code_index: String,
    /// A safe label saying why; `operator_void` when absent.
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ReferralVoidResponse {
    tenant_id: String,
    voided: bool,
    referees_voided: u64,
    /// Rail atomic units, as text.
    taken_back_units: String,
}

fn void_reason(reason: Option<&str>) -> ApiResult<&str> {
    match reason {
        None => Ok(REFERRAL_OPERATOR_VOID),
        Some(reason) if super::payout_rails::is_reason_label(reason) => Ok(reason),
        Some(_) => Err(api_error(
            StatusCode::BAD_REQUEST,
            "referral_void_reason_invalid",
        )),
    }
}

pub(crate) async fn referral_code_void_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<ReferralCodeVoidRequest>,
) -> ApiResult<Json<ReferralVoidResponse>> {
    let (actor, hook) = referral_operator(state.as_ref(), &headers)?;
    check_tenant(&request.tenant_id)?;
    check_hash(&request.code_index, "referral_code_index_invalid")?;
    let reason = void_reason(request.reason.as_deref())?;
    let outcome = hook
        .store()
        .void_referral_code(
            &request.tenant_id,
            &request.code_index,
            reason,
            hook.rail_id(),
            &actor,
        )
        .await
        .map_err(|_| store_unavailable())?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "referral_code_not_found"))?;
    tracing::info!(
        reason,
        referees = outcome.referees_voided,
        "referral code voided by an operator"
    );
    Ok(Json(ReferralVoidResponse {
        tenant_id: request.tenant_id,
        voided: true,
        referees_voided: outcome.referees_voided,
        taken_back_units: outcome.taken_back.to_string(),
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReferralVoidRequest {
    /// The referee's tenant.
    tenant_id: String,
    /// The referee's account, as `payout_account_id_hash`.
    account_id_hash: String,
    #[serde(default)]
    reason: Option<String>,
}

pub(crate) async fn referral_void_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<ReferralVoidRequest>,
) -> ApiResult<Json<ReferralVoidResponse>> {
    let (actor, hook) = referral_operator(state.as_ref(), &headers)?;
    check_tenant(&request.tenant_id)?;
    check_hash(&request.account_id_hash, "referral_account_id_hash_invalid")?;
    let reason = void_reason(request.reason.as_deref())?;
    let store = hook.store();
    let not_found = || api_error(StatusCode::NOT_FOUND, "referral_not_found");
    let attribution = store
        .referral_attributions(&request.tenant_id)
        .await
        .map_err(|_| store_unavailable())?
        .into_iter()
        .find(|attribution| {
            payout_account_id_hash(attribution.account_id) == request.account_id_hash
        })
        .ok_or_else(not_found)?;
    // The referee's tenant first: once the attribution is void the pass
    // writes nothing new for it, and finishes the referrer's side itself if
    // the second write below does not land.
    store
        .void_referral_attribution(&request.tenant_id, attribution.account_id, reason, &actor)
        .await
        .map_err(|_| store_unavailable())?;
    let outcome = store
        .void_referral_referee(
            &attribution.referrer_tenant_id,
            &attribution.referee_ref_hash,
            reason,
            hook.rail_id(),
            &actor,
        )
        .await
        .map_err(|_| store_unavailable())?
        .unwrap_or_default();
    tracing::info!(reason, "referral voided by an operator");
    Ok(Json(ReferralVoidResponse {
        tenant_id: request.tenant_id,
        voided: true,
        referees_voided: outcome.referees_voided,
        taken_back_units: outcome.taken_back.to_string(),
    }))
}

#[derive(Debug, Serialize)]
pub(crate) struct ReferralDryRunReport {
    rail_id: String,
    mode: &'static str,
    tripped: bool,
    tenants: u64,
    codes_by_state: BTreeMap<String, u64>,
    referees: u64,
    referees_by_signal: BTreeMap<String, u64>,
    /// Rail atomic units, as text.
    owed_units: String,
    sendable_units: String,
    held_units: String,
}

pub(crate) async fn referral_dry_run_report_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<ReferralDryRunReport>> {
    let (_, hook) = referral_operator(state.as_ref(), &headers)?;
    let store = hook.store();
    let tenants = store
        .referral_code_tenants()
        .await
        .map_err(|_| store_unavailable())?;
    let mut total = ReferralTenantReport::default();
    for tenant in &tenants {
        let report = store
            .referral_tenant_report(tenant, hook.rail_id())
            .await
            .map_err(|_| store_unavailable())?;
        for (state, count) in report.codes_by_state {
            *total.codes_by_state.entry(state).or_default() += count;
        }
        for (label, count) in report.referees_by_signal {
            *total.referees_by_signal.entry(label).or_default() += count;
        }
        total.referees += report.referees;
        total.owed = total.owed.saturating_add(report.owed);
        total.sendable = total.sendable.saturating_add(report.sendable);
        total.held = total.held.saturating_add(report.held);
    }
    let tripped = hook
        .payouts()
        .payout_control(&trace_commons_server::referral_bonus::near_ai_referral_control_rail())
        .await
        .map_err(|_| store_unavailable())?
        .halted;
    tracing::info!(
        tenants = tenants.len(),
        referees = total.referees,
        tripped,
        "referral dry-run report"
    );
    Ok(Json(ReferralDryRunReport {
        rail_id: hook.rail_id().as_str().to_owned(),
        mode: hook.mode().as_label(),
        tripped,
        tenants: tenants.len() as u64,
        codes_by_state: total.codes_by_state,
        referees: total.referees,
        referees_by_signal: total.referees_by_signal,
        owed_units: total.owed.to_string(),
        sendable_units: total.sendable.to_string(),
        held_units: total.held.to_string(),
    }))
}
