// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Earned account trust (Flow 3) worker routes, shadow only. See
//! `docs/superpowers/specs/2026-09-26-earned-account-trust-design.md`.
//!
//! Decision 7 of that spec: during shadow these run under `require_admin`, as
//! the contributor-cap and dedup passes do. A scoped worker bearer is required
//! before any of them runs unattended in production. Nothing here changes an
//! allowance: the evaluator writes `shadow` rows only (the V86 write function
//! refuses anything else), and admission reads none of them.

use super::*;
use trace_commons_server::account_trust_growth::{
    EvaluateSummary, ExplainReport, RecordFactsSummary, ReproductionDrillSummary,
    account_trust_reproduction_drill, database_error_label, evaluate_account_trust,
    explain_account_trust, find_account_by_ref, record_account_trust_facts,
};
use trace_commons_server::account_trust_rule::{GrowthPolicy, parse_shadow_growth_policy};

const SHADOW_POLICY_JSON: &str = "TRACE_COMMONS_ACCOUNT_TRUST_SHADOW_POLICY_JSON";
const SHADOW_POLICY_VERSION: &str = "TRACE_COMMONS_ACCOUNT_TRUST_SHADOW_POLICY_VERSION";
/// The rollout-smoke evidence check name the reproduction drill records.
pub(super) const ACCOUNT_TRUST_EXPLAIN_CHECK: &str = "account_trust_explain";

/// The candidate growth policy the shadow evaluator runs against. Supplied
/// separately from the admission policy and parsed by the same base rules;
/// admission never reads it. Absent is off. Half-configured or malformed
/// refuses startup, with a label that names the missing control.
pub(super) fn shadow_policy_from_env() -> anyhow::Result<Option<GrowthPolicy>> {
    shadow_policy_from_values(|key| std::env::var(key))
}

pub(super) fn shadow_policy_from_values(
    read: impl Fn(&str) -> Result<String, std::env::VarError>,
) -> anyhow::Result<Option<GrowthPolicy>> {
    let json = read(SHADOW_POLICY_JSON).ok();
    let version = read(SHADOW_POLICY_VERSION).ok();
    match (json, version) {
        (None, None) => Ok(None),
        (Some(json), Some(version)) => parse_shadow_growth_policy(&json, &[version.as_str()])
            .map(Some)
            .map_err(|_| anyhow::anyhow!("account_trust_shadow_policy_invalid")),
        (Some(_), None) => anyhow::bail!("account_trust_shadow_policy_version_missing"),
        (None, Some(_)) => anyhow::bail!("account_trust_shadow_policy_missing"),
    }
}

/// Refusals since process start that the candidate policy would have
/// admitted. Process-local and label-only; it resets on restart.
static SHADOW_WOULD_ADMIT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Called on an `account_limit_reached` refusal. Reads the account's latest
/// shadow evaluation under the candidate policy and, when that tier's
/// allowance would have fitted the refused reservation, counts it and logs a
/// label-only line: the number the switch-on decision turns on. Never
/// changes the refusal, and swallows every error.
pub(super) async fn observe_shadow_would_admit(
    state: &AppState,
    account: &trace_commons_server::account_trust::TrustAccount,
    period_spend: i64,
) {
    let (Some(policy), Some(db)) = (
        state.account_trust_shadow_policy.as_deref(),
        state.db_mirror.as_ref(),
    ) else {
        return;
    };
    let stored = match db
        .latest_account_trust_evaluation(
            account,
            policy.version(),
            trace_commons_server::account_trust_growth::SHADOW_MODE,
        )
        .await
    {
        Ok(stored) => stored,
        Err(error) => {
            tracing::debug!(
                error_label = database_error_label(&error),
                "earned-trust shadow read skipped"
            );
            return;
        }
    };
    let head = stored.as_ref().map(|evaluation| {
        trace_commons_server::account_trust_rule::StoredEvaluationHead {
            growth_policy_version: &evaluation.growth_policy_version,
            mode: trace_commons_server::account_trust_growth::SHADOW_MODE,
            as_of: evaluation.as_of,
            tier: evaluation.tier,
        }
    });
    if trace_commons_server::account_trust_rule::shadow_would_admit(
        policy,
        head,
        period_spend,
        Utc::now(),
    ) {
        let total = SHADOW_WOULD_ADMIT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        tracing::info!(
            shadow_tier = head.map_or(0, |h| h.tier),
            growth_policy_version = policy.version(),
            shadow_would_admit_since_start = total,
            "account_trust_shadow_would_admit"
        );
    }
}

fn worker_unavailable(
    error: &DatabaseError,
    context: &'static str,
) -> (StatusCode, Json<ApiError>) {
    tracing::warn!(error_label = database_error_label(error), "{context}");
    api_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "account_trust_worker_unavailable",
    )
}

fn require_db(state: &AppState) -> ApiResult<&Arc<dyn Database>> {
    state.db_mirror.as_ref().ok_or_else(|| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "earned account trust requires a configured DB mirror",
        )
    })
}

fn require_shadow_policy(state: &AppState) -> ApiResult<&GrowthPolicy> {
    state.account_trust_shadow_policy.as_deref().ok_or_else(|| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "account_trust_shadow_policy_missing",
        )
    })
}

#[derive(Debug, Deserialize)]
pub(super) struct RecordAccountTrustFactsQuery {
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    dry_run: Option<bool>,
}

/// `POST /v1/admin/record-account-trust-facts?limit=N&dry_run=true|false`
///
/// Walks open accounts in anchored tenants and records each missing fact
/// through the V85 definer function. Synchronous and bounded by `limit`
/// (default 1000 sources, at most 10000), so the acknowledgement carries the
/// counts. Idempotent: a re-run records nothing new. The response is
/// label-only counts; no account, tenant, submission or source identifier.
pub(super) async fn record_account_trust_facts_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<RecordAccountTrustFactsQuery>,
) -> ApiResult<Json<RecordFactsSummary>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_admin(&tenant)?;
    let db = require_db(state.as_ref())?;
    let summary =
        record_account_trust_facts(db.as_ref(), query.limit, query.dry_run.unwrap_or(false))
            .await
            .map_err(|error| worker_unavailable(&error, "earned-trust fact recording refused"))?;
    tracing::info!(
        accounts_scanned = summary.accounts_scanned,
        candidates = summary.candidates,
        declined = summary.declined,
        failed = summary.failed,
        dry_run = summary.dry_run,
        "earned-trust fact recording pass completed"
    );
    Ok(Json(summary))
}

#[derive(Debug, Deserialize)]
pub(super) struct EvaluateAccountTrustQuery {
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    as_of: Option<String>,
}

/// `POST /v1/admin/evaluate-account-trust?limit=N&mode=shadow&as_of=RFC3339`
///
/// Evaluates each open account with facts against the configured shadow
/// policy and stores a `shadow` evaluation row, appending a hash-only
/// `account_trust_tier_changed` audit row when the tier moves. `mode` may be
/// omitted or `shadow`; anything else is refused. `as_of` defaults to now and
/// may not be in the future. Run it on a schedule so that decay takes effect
/// without new facts.
pub(super) async fn evaluate_account_trust_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<EvaluateAccountTrustQuery>,
) -> ApiResult<Json<EvaluateSummary>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_admin(&tenant)?;
    if query.mode.as_deref().is_some_and(|mode| mode != "shadow") {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "account_trust_evaluation_shadow_only",
        ));
    }
    let now = Utc::now();
    let as_of = match query.as_of.as_deref() {
        None => now,
        Some(raw) => DateTime::parse_from_rfc3339(raw)
            .map(|at| at.with_timezone(&Utc))
            .ok()
            .filter(|at| *at <= now)
            .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "account_trust_as_of_invalid"))?,
    };
    let db = require_db(state.as_ref())?;
    let policy = require_shadow_policy(state.as_ref())?;
    let summary = evaluate_account_trust(db.as_ref(), policy, as_of, query.limit)
        .await
        .map_err(|error| worker_unavailable(&error, "earned-trust evaluation refused"))?;
    tracing::info!(
        accounts_scanned = summary.accounts_scanned,
        accounts_evaluated = summary.accounts_evaluated,
        tier_changed = summary.tier_changed,
        failed = summary.failed,
        "earned-trust shadow evaluation pass completed"
    );
    Ok(Json(summary))
}

#[derive(Debug, Deserialize)]
pub(super) struct ExplainAccountTrustQuery {
    account_ref: String,
}

fn is_account_ref(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}

/// `GET /v1/admin/account-trust/explain?account_ref=sha256:<hex>`
///
/// Recomputes the account's latest stored shadow evaluation at its stored
/// `as_of` and returns both, side by side, with `reproduced`. Counts, tier,
/// digest and policy version only. `account_ref` is
/// `account_trust_growth::account_trust_ref`, never a tenant or account ID.
pub(super) async fn explain_account_trust_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ExplainAccountTrustQuery>,
) -> ApiResult<Json<ExplainReport>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_admin(&tenant)?;
    if !is_account_ref(&query.account_ref) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "account_trust_account_ref_invalid",
        ));
    }
    let db = require_db(state.as_ref())?;
    let policy = require_shadow_policy(state.as_ref())?;
    let account = find_account_by_ref(db.as_ref(), &query.account_ref)
        .await
        .map_err(|error| worker_unavailable(&error, "earned-trust explain refused"))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "account_trust_account_not_found"))?;
    let report = explain_account_trust(db.as_ref(), policy, &account)
        .await
        .map_err(|error| worker_unavailable(&error, "earned-trust explain refused"))?;
    Ok(Json(report))
}

#[derive(Debug, Deserialize)]
pub(super) struct AccountTrustDrillRequest {
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    record_evidence: bool,
    /// Bounds the accounts visited. Switch-on condition 4 asks for the whole
    /// population, which is the default; a bounded run never passes.
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub(super) struct AccountTrustDrillResponse {
    purpose_hash: String,
    generated_at: DateTime<Utc>,
    passed: bool,
    summary: ReproductionDrillSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    recorded_evidence: Option<TraceRolloutSmokeEvidenceResponse>,
}

/// `POST /v1/admin/account-trust-drill`
///
/// Reproduces every stored shadow evaluation from hash-only facts and reports
/// the reproduced count as hash-only evidence. With `record_evidence`, it
/// appends an `account_trust_explain` rollout-smoke evidence row, `passed`
/// only when at least one evaluation was checked and every one reproduced.
pub(super) async fn account_trust_drill_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<AccountTrustDrillRequest>,
) -> ApiResult<Json<AccountTrustDrillResponse>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_admin(&tenant)?;
    let purpose = request
        .purpose
        .as_deref()
        .map(str::trim)
        .filter(|purpose| !purpose.is_empty())
        .unwrap_or("trace_commons_account_trust_drill")
        .to_string();
    let db = require_db(state.as_ref())?;
    let policy = require_shadow_policy(state.as_ref())?;
    let summary = account_trust_reproduction_drill(db.as_ref(), policy, request.limit)
        .await
        .map_err(|error| worker_unavailable(&error, "earned-trust drill refused"))?;
    let passed = summary.passed();
    let mut response = AccountTrustDrillResponse {
        purpose_hash: sha256_prefixed(&purpose),
        generated_at: Utc::now(),
        passed,
        summary,
        recorded_evidence: None,
    };
    if request.record_evidence {
        let evidence = TraceRolloutSmokeEvidenceResponse {
            event_id: Uuid::new_v4(),
            tenant_id: tenant.tenant_id.clone(),
            tenant_storage_ref: tenant_storage_ref(&tenant.tenant_id),
            check_name: ACCOUNT_TRUST_EXPLAIN_CHECK.to_string(),
            status: if passed {
                TraceRolloutSmokeEvidenceStatus::Passed
            } else {
                TraceRolloutSmokeEvidenceStatus::Failed
            },
            evidence_hash: response.summary.evidence_hash.clone(),
            evidence_ref_hash: Some(sha256_prefixed(&purpose)),
            actor_principal_ref: tenant.principal_ref.clone(),
            recorded_at: Utc::now(),
        };
        append_audit_event_with_db_mirror(
            state.as_ref(),
            &tenant,
            TraceCommonsAuditEvent::rollout_smoke_evidence(&evidence),
            StorageTraceAuditAction::Read,
            StorageTraceAuditSafeMetadata::Empty,
        )
        .await
        .map_err(internal_error)?;
        response.recorded_evidence = Some(evidence);
    }
    Ok(Json(response))
}
