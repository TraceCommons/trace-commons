// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Earned account trust (Flow 3) worker routes and their scheduler. See
//! `docs/superpowers/specs/2026-09-26-earned-account-trust-design.md`.
//!
//! Decision 7 of that spec: fact recording and evaluation accept an admin
//! token or the scoped `account_trust_worker` token, which is what the
//! in-process scheduler presents; explain and the drill stay admin-only.
//!
//! Two policies can be evaluated. The shadow candidate
//! (`TRACE_COMMONS_ACCOUNT_TRUST_SHADOW_POLICY_*`) writes `shadow` rows that
//! admission never reads. When the admission policy itself names
//! `"growth_rule":"tiered-v1"`, the evaluator also writes `applied` rows under
//! it, and admission reads the latest fresh one (V114, V126). An independent
//! client-upload trust policy also uses the existing `applied` mode, while
//! both admission ledgers remain disabled.

use super::*;
use trace_commons_server::account_trust::GrowthSource;
use trace_commons_server::account_trust_growth::{
    EvaluateSummary, EvaluationMode, ExplainReport, RecordFactsSummary, ReproductionDrillSummary,
    account_trust_reproduction_drill, database_error_label, evaluate_account_trust,
    explain_account_trust, find_account_by_ref, record_account_trust_facts,
};
use trace_commons_server::account_trust_rule::{GrowthPolicy, parse_growth_policy};

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
        (Some(json), Some(version)) => parse_growth_policy(&json, &[version.as_str()])
            .map(Some)
            .map_err(|_| anyhow::anyhow!("account_trust_shadow_policy_invalid")),
        (Some(_), None) => anyhow::bail!("account_trust_shadow_policy_version_missing"),
        (None, Some(_)) => anyhow::bail!("account_trust_shadow_policy_missing"),
    }
}

/// The admission policy's own growth rule, when it names `tiered-v1`: the
/// policy the evaluator's `applied` mode runs. Parsed from the same JSON and
/// version admission accepted, so the two cannot disagree. `None` for
/// `"none"` and `"external"`.
pub(super) fn applied_policy_from_env(
    admission: Option<&admission::AccountAdmissionConfig>,
) -> anyhow::Result<Option<GrowthPolicy>> {
    applied_policy_from_values(admission, |key| std::env::var(key))
}

pub(super) fn applied_policy_from_values(
    admission: Option<&admission::AccountAdmissionConfig>,
    read: impl Fn(&str) -> Result<String, std::env::VarError>,
) -> anyhow::Result<Option<GrowthPolicy>> {
    let tiered = admission
        .and_then(|config| config.policy.external_growth())
        .is_some_and(|growth| growth.source == GrowthSource::TieredV1);
    if !tiered {
        return Ok(None);
    }
    let invalid = || anyhow::anyhow!("account_admission_policy_invalid");
    let json = read("TRACE_COMMONS_ACCOUNT_ADMISSION_POLICY_JSON").map_err(|_| invalid())?;
    let version = read("TRACE_COMMONS_ACCOUNT_ADMISSION_POLICY_VERSION").map_err(|_| invalid())?;
    parse_growth_policy(&json, &[version.as_str()])
        .map(Some)
        .map_err(|_| invalid())
}

/// The independent mode changes advertised client caps only. Reject a
/// configured admission ledger rather than silently turning one off.
pub(super) fn require_independent_upload_trust_controls(
    independent: bool,
    account_admission_enabled: bool,
    legacy_admission_enabled: bool,
    database_available: bool,
    durable_writes: bool,
    forced_rls_ready: bool,
) -> anyhow::Result<()> {
    if !independent {
        return Ok(());
    }
    anyhow::ensure!(
        !account_admission_enabled && !legacy_admission_enabled,
        "client_upload_trust_admission_conflict"
    );
    anyhow::ensure!(
        database_available,
        "client_upload_trust_database_unavailable"
    );
    anyhow::ensure!(
        durable_writes,
        "client_upload_trust_durable_writes_required"
    );
    anyhow::ensure!(forced_rls_ready, "client_upload_trust_rls_required");
    Ok(())
}

/// Reuse the scheduler's one applied-policy slot. The independent writer and
/// reader were parsed together; admission-backed deployments retain their
/// existing policy source when independent upload trust is absent.
pub(super) fn select_applied_policy(
    admission: Option<&admission::AccountAdmissionConfig>,
    independent: Option<&upload_allowance::ClientUploadTrustPolicy>,
    read: impl Fn(&str) -> Result<String, std::env::VarError>,
) -> anyhow::Result<Option<GrowthPolicy>> {
    if let Some(independent) = independent {
        anyhow::ensure!(
            admission.is_none(),
            "client_upload_trust_admission_conflict"
        );
        return Ok(Some(independent.growth.clone()));
    }
    applied_policy_from_values(admission, read)
}

const CLIENT_UPLOAD_TRUST_DATABASE_URL: &str = "TRACE_COMMONS_CLIENT_UPLOAD_TRUST_DATABASE_URL";

/// Optional narrow trust pool, pointed at the same database as the unchanged
/// ingest source pool. Auxiliary worker credentials never flow into it.
pub(super) fn trust_database_config_from_values(
    main: Option<&DatabaseConfig>,
    configured_url: Option<&str>,
    independent: bool,
    shadow: bool,
    account_admission_enabled: bool,
    legacy_admission_enabled: bool,
    durable_writes: bool,
) -> anyhow::Result<Option<DatabaseConfig>> {
    let Some(configured_url) = configured_url else {
        return Ok(None);
    };
    anyhow::ensure!(
        independent || shadow,
        "client_upload_trust_database_policy_required"
    );
    anyhow::ensure!(
        !account_admission_enabled && !legacy_admission_enabled,
        "client_upload_trust_admission_conflict"
    );
    let main = main.ok_or_else(|| anyhow::anyhow!("client_upload_trust_database_unavailable"))?;
    anyhow::ensure!(
        durable_writes,
        "client_upload_trust_durable_writes_required"
    );
    let parse = |value: &str| -> anyhow::Result<reqwest::Url> {
        let url = reqwest::Url::parse(value)
            .map_err(|_| anyhow::anyhow!("client_upload_trust_database_url_invalid"))?;
        anyhow::ensure!(
            matches!(url.scheme(), "postgres" | "postgresql")
                && url.host_str().is_some()
                && !url.path().is_empty()
                && url.path() != "/"
                && url.fragment().is_none()
                && !url.query_pairs().any(|(key, _)| matches!(
                    key.as_ref(),
                    "host" | "hostaddr" | "port" | "dbname" | "service" | "servicefile"
                )),
            "client_upload_trust_database_url_invalid"
        );
        Ok(url)
    };
    let source = parse(main.url.expose_secret())?;
    let trust = parse(configured_url)?;
    anyhow::ensure!(
        source.host_str() == trust.host_str()
            && source.port().unwrap_or(5432) == trust.port().unwrap_or(5432)
            && source.path() == trust.path(),
        "client_upload_trust_database_endpoint_mismatch"
    );
    let mut config = main.clone();
    config.url = configured_url.to_owned().into();
    config.login_resolver_url = None;
    config.gate_driver_url = None;
    config.pii_backstop_driver_url = None;
    config.invite_registry_url = None;
    Ok(Some(config))
}

pub(super) async fn trust_database_from_env(
    main: Option<&DatabaseConfig>,
    independent: bool,
    shadow: bool,
    account_admission_enabled: bool,
    legacy_admission_enabled: bool,
    durable_writes: bool,
) -> anyhow::Result<Option<Arc<dyn Database>>> {
    let configured_url = match std::env::var(CLIENT_UPLOAD_TRUST_DATABASE_URL) {
        Ok(url) => Some(url),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => {
            anyhow::bail!("client_upload_trust_database_url_invalid")
        }
    };
    let Some(config) = trust_database_config_from_values(
        main,
        configured_url.as_deref(),
        independent,
        shadow,
        account_admission_enabled,
        legacy_admission_enabled,
        durable_writes,
    )?
    else {
        return Ok(None);
    };
    // The source pool owns migration. This login must remain a narrow
    // nonowner worker, and readiness is checked before the server listens.
    let database = PgBackend::new(&config)
        .await
        .map_err(|_| anyhow::anyhow!("client_upload_trust_database_unavailable"))?;
    Ok(Some(Arc::new(database)))
}

/// Applied evaluations must be invalidated by gate/outcome changes and
/// readable/writable through the existing forced-RLS trust seams. This does
/// not enable admission, witness requirements, or a processing-cost ledger.
pub(super) async fn require_independent_upload_trust_runtime_ready(
    db: &dyn Database,
) -> anyhow::Result<()> {
    db.enable_external_account_trust_growth()
        .await
        .map_err(|_| anyhow::anyhow!("external_account_trust_readiness_unavailable"))?;
    anyhow::ensure!(
        db.client_upload_trust_runtime_ready()
            .await
            .map_err(|_| anyhow::anyhow!("client_upload_trust_readiness_unavailable"))?,
        "client_upload_trust_contract_not_ready"
    );
    anyhow::ensure!(
        db.tiered_account_trust_runtime_ready()
            .await
            .map_err(|_| anyhow::anyhow!("tiered_account_trust_readiness_unavailable"))?,
        "tiered_account_trust_contract_not_ready"
    );
    Ok(())
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
        state
            .client_upload_trust_database
            .as_ref()
            .or(state.db_mirror.as_ref()),
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
    state
        .client_upload_trust_database
        .as_ref()
        .or(state.db_mirror.as_ref())
        .ok_or_else(|| {
            api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "earned account trust requires a configured DB mirror",
            )
        })
}

fn require_policy(state: &AppState, mode: EvaluationMode) -> ApiResult<&GrowthPolicy> {
    match mode {
        EvaluationMode::Shadow => state.account_trust_shadow_policy.as_deref().ok_or_else(|| {
            api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "account_trust_shadow_policy_missing",
            )
        }),
        EvaluationMode::Applied => state
            .account_trust_applied_policy
            .as_deref()
            .ok_or_else(|| {
                api_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "account_trust_applied_policy_missing",
                )
            }),
    }
}

/// `shadow` (the default) or `applied`. `production` is accepted as another
/// name for `applied`; the stored mode is always `applied`.
fn parse_mode(raw: Option<&str>) -> ApiResult<EvaluationMode> {
    match raw {
        None | Some("shadow") => Ok(EvaluationMode::Shadow),
        Some("applied") | Some("production") => Ok(EvaluationMode::Applied),
        Some(_) => Err(api_error(
            StatusCode::BAD_REQUEST,
            "account_trust_evaluation_mode_invalid",
        )),
    }
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
/// through the V85/V126 definer function. Synchronous and bounded by `limit`
/// (default 1000 sources, at most 10000), so the acknowledgement carries the
/// counts. Idempotent: a re-run records nothing new. The response is
/// label-only counts; no account, tenant, submission or source identifier.
/// Admin or `account_trust_worker` token.
pub(super) async fn record_account_trust_facts_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<RecordAccountTrustFactsQuery>,
) -> ApiResult<Json<RecordFactsSummary>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_account_trust_operator(&tenant)?;
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

/// `POST /v1/admin/evaluate-account-trust?limit=N&mode=shadow|applied&as_of=RFC3339`
///
/// Evaluates each open account with facts and stores an evaluation row,
/// appending a hash-only `account_trust_tier_changed` audit row when the tier
/// moves. Admin or `account_trust_worker` token.
///
/// - `shadow` (the default) runs the shadow candidate. `as_of` defaults to
///   now and may not be in the future.
/// - `applied` (also spelled `production`) runs the admission policy's own
///   `tiered-v1` rule, and is refused unless the admission policy names one.
///   Each account is evaluated at the database's transaction time, bound to
///   the inputs it read, so `as_of` is refused.
///
/// Run it on a schedule so that decay takes effect without new facts; an
/// applied row older than the policy's `evaluation_max_age_seconds` is read
/// as tier 0.
pub(super) async fn evaluate_account_trust_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<EvaluateAccountTrustQuery>,
) -> ApiResult<Json<EvaluateSummary>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_account_trust_operator(&tenant)?;
    let mode = parse_mode(query.mode.as_deref())?;
    if mode == EvaluationMode::Applied && query.as_of.is_some() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "account_trust_as_of_not_allowed_when_applied",
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
    let policy = require_policy(state.as_ref(), mode)?;
    let db = require_db(state.as_ref())?;
    let summary = evaluate_account_trust(db.as_ref(), policy, mode, as_of, query.limit)
        .await
        .map_err(|error| worker_unavailable(&error, "earned-trust evaluation refused"))?;
    tracing::info!(
        mode = summary.mode,
        accounts_scanned = summary.accounts_scanned,
        accounts_evaluated = summary.accounts_evaluated,
        tier_changed = summary.tier_changed,
        failed = summary.failed,
        "earned-trust evaluation pass completed"
    );
    Ok(Json(summary))
}

#[derive(Debug, Deserialize)]
pub(super) struct ExplainAccountTrustQuery {
    account_ref: String,
    #[serde(default)]
    mode: Option<String>,
}

fn is_account_ref(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}

/// `GET /v1/admin/account-trust/explain?account_ref=sha256:<hex>&mode=shadow|applied`
///
/// Recomputes the account's latest stored evaluation in `mode` (default
/// `shadow`) at its stored `as_of` and returns both, side by side, with
/// `reproduced`. Counts, tier,
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
    let mode = parse_mode(query.mode.as_deref())?;
    let db = require_db(state.as_ref())?;
    let policy = require_policy(state.as_ref(), mode)?;
    let account = find_account_by_ref(db.as_ref(), &query.account_ref)
        .await
        .map_err(|error| worker_unavailable(&error, "earned-trust explain refused"))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "account_trust_account_not_found"))?;
    let report = explain_account_trust(db.as_ref(), policy, mode, &account)
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
    /// `shadow` (the default) or `applied`.
    #[serde(default)]
    mode: Option<String>,
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
/// Reproduces every stored evaluation in `mode` (default `shadow`) from
/// hash-only facts and reports
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
    let mode = parse_mode(request.mode.as_deref())?;
    let db = require_db(state.as_ref())?;
    let policy = require_policy(state.as_ref(), mode)?;
    let summary = account_trust_reproduction_drill(db.as_ref(), policy, mode, request.limit)
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

// ---------------------------------------------------------------------------
// The scheduler: unattended recording and evaluation under the scoped token.
// ---------------------------------------------------------------------------

const SCHEDULER_ENABLED: &str = "TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_ENABLED";
const SCHEDULER_TOKEN: &str = "TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_TOKEN";
const SCHEDULER_INTERVAL_SECONDS: &str = "TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_INTERVAL_SECONDS";
const SCHEDULER_LIMIT: &str = "TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_LIMIT";
const SCHEDULER_DEFAULT_INTERVAL_SECONDS: u64 = 6 * 3600;
/// Recorder passes one tick makes before evaluating, each bounded by the
/// limit; a backlog larger than this is finished by the next tick.
const SCHEDULER_MAX_RECORD_PASSES: usize = 10;

#[derive(Clone)]
pub(super) struct AccountTrustSchedulerConfig {
    worker_token: SecretString,
    interval: StdDuration,
    limit: i64,
}

/// Each tick records missing facts, then evaluates every configured policy:
/// `applied` under a `tiered-v1` admission policy, and `shadow` under a
/// shadow candidate. Absent or off is no scheduler; enabled without a token
/// refuses startup.
pub(super) fn scheduler_config_from_env() -> anyhow::Result<Option<AccountTrustSchedulerConfig>> {
    let enabled = env_truthy(SCHEDULER_ENABLED);
    let worker_token = optional_trimmed_env(SCHEDULER_TOKEN)?;
    if !enabled && worker_token.is_none() {
        return Ok(None);
    }
    let Some(worker_token) = worker_token else {
        anyhow::bail!("{SCHEDULER_ENABLED}=true requires {SCHEDULER_TOKEN}");
    };
    let interval_seconds = parse_optional_scheduler_u64_env(
        SCHEDULER_INTERVAL_SECONDS,
        SCHEDULER_DEFAULT_INTERVAL_SECONDS,
        60,
        86_400,
    )?;
    let limit = parse_optional_scheduler_u64_env(
        SCHEDULER_LIMIT,
        trace_commons_server::account_trust_growth::RECORD_FACTS_MAX_LIMIT as u64,
        1,
        trace_commons_server::account_trust_growth::RECORD_FACTS_MAX_LIMIT as u64,
    )?;
    Ok(Some(AccountTrustSchedulerConfig::new(
        worker_token,
        interval_seconds,
        limit as i64,
    )))
}

impl AccountTrustSchedulerConfig {
    pub(super) fn new(worker_token: String, interval_seconds: u64, limit: i64) -> Self {
        Self {
            worker_token: SecretString::from(worker_token),
            interval: StdDuration::from_secs(interval_seconds),
            limit,
        }
    }
}

/// Fails closed at startup: the token must authenticate as an admin or
/// `account_trust_worker`, a DB mirror and at least one policy must be
/// configured, and the interval must leave every evaluation fresh until the
/// next tick (shorter than each policy's `evaluation_max_age_seconds`).
pub(super) async fn validate_scheduler_config(
    state: &AppState,
    config: Option<&AccountTrustSchedulerConfig>,
) -> anyhow::Result<()> {
    let Some(config) = config else {
        return Ok(());
    };
    let refused = |error: (StatusCode, Json<ApiError>)| {
        anyhow::anyhow!(
            "invalid Trace Commons account trust scheduler configuration: status={}, error={}",
            error.0,
            error.1.0.error
        )
    };
    let headers =
        bearer_auth_headers_from_token(config.worker_token.expose_secret()).map_err(refused)?;
    let auth = authenticate(state, &headers).map_err(refused)?;
    require_account_trust_operator(&auth).map_err(refused)?;
    anyhow::ensure!(
        state.db_mirror.is_some(),
        "invalid Trace Commons account trust scheduler configuration: TRACE_COMMONS_DB_DUAL_WRITE is required"
    );
    let policies: Vec<&GrowthPolicy> = [
        state.account_trust_applied_policy.as_deref(),
        state.account_trust_shadow_policy.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect();
    anyhow::ensure!(
        !policies.is_empty(),
        "invalid Trace Commons account trust scheduler configuration: no applied tiered-v1 policy or shadow policy to evaluate"
    );
    for policy in policies {
        anyhow::ensure!(
            config.interval.as_secs() < policy.evaluation_max_age_seconds() as u64,
            "invalid Trace Commons account trust scheduler configuration: {SCHEDULER_INTERVAL_SECONDS} must be shorter than evaluation_max_age_seconds"
        );
    }
    Ok(())
}

pub(super) fn spawn_scheduler_task(
    state: &Arc<AppState>,
    config: Option<AccountTrustSchedulerConfig>,
) {
    let Some(config) = config else {
        return;
    };
    tracing::info!(
        interval_seconds = config.interval.as_secs(),
        limit = config.limit,
        applied = state.account_trust_applied_policy.is_some(),
        shadow = state.account_trust_shadow_policy.is_some(),
        "Trace Commons account trust scheduler enabled"
    );
    let tick_config = config.clone();
    spawn_driver_loop(
        state,
        ACCOUNT_TRUST_SCHEDULER_DRIVER_NAME,
        config.interval,
        move |state| {
            let config = tick_config.clone();
            async move {
                let summary = run_scheduler_tick(state, &config).await.map_err(|error| {
                    worker_route_error(ACCOUNT_TRUST_SCHEDULER_DRIVER_NAME, error)
                })?;
                tracing::info!(
                    record_passes = summary.record_passes,
                    facts_recorded = summary.facts_recorded,
                    facts_failed = summary.facts_failed,
                    record_limit_reached = summary.record_limit_reached,
                    evaluated = summary.evaluated,
                    evaluation_failed = summary.evaluation_failed,
                    "Trace Commons account trust scheduler tick completed"
                );
                all_attempts_failed_outcome(
                    ACCOUNT_TRUST_SCHEDULER_DRIVER_NAME,
                    "evaluate",
                    summary.evaluated as usize,
                    summary.evaluation_failed as usize,
                    DriverFailureClass::DependencyUnavailable,
                )
            }
        },
    );
}

/// Label-only counts from one scheduler tick.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct AccountTrustSchedulerTickSummary {
    pub record_passes: u64,
    pub facts_recorded: u64,
    pub facts_failed: u64,
    pub record_limit_reached: bool,
    pub evaluated: u64,
    pub evaluation_failed: u64,
}

/// One tick, through the same handlers and the same token check an operator
/// call gets: record until the backlog is drained (or the pass bound), then
/// evaluate `applied` and `shadow`, whichever are configured.
pub(super) async fn run_scheduler_tick(
    state: Arc<AppState>,
    config: &AccountTrustSchedulerConfig,
) -> ApiResult<AccountTrustSchedulerTickSummary> {
    let headers = bearer_auth_headers_from_token(config.worker_token.expose_secret())?;
    let mut summary = AccountTrustSchedulerTickSummary::default();
    for _ in 0..SCHEDULER_MAX_RECORD_PASSES {
        let Json(pass) = record_account_trust_facts_handler(
            State(state.clone()),
            headers.clone(),
            Query(RecordAccountTrustFactsQuery {
                limit: Some(config.limit),
                dry_run: Some(false),
            }),
        )
        .await?;
        summary.record_passes += 1;
        summary.facts_recorded += pass.recorded_by_outcome.values().sum::<u64>();
        summary.facts_failed += pass.failed;
        summary.record_limit_reached = pass.limit_reached;
        if !pass.limit_reached {
            break;
        }
    }
    for (configured, mode) in [
        (state.account_trust_applied_policy.is_some(), "applied"),
        (state.account_trust_shadow_policy.is_some(), "shadow"),
    ] {
        if !configured {
            continue;
        }
        let Json(run) = evaluate_account_trust_handler(
            State(state.clone()),
            headers.clone(),
            Query(EvaluateAccountTrustQuery {
                limit: Some(trace_commons_server::account_trust_growth::RECORD_FACTS_MAX_LIMIT),
                mode: Some(mode.to_string()),
                as_of: None,
            }),
        )
        .await?;
        summary.evaluated += run.accounts_evaluated;
        summary.evaluation_failed += run.failed;
    }
    Ok(summary)
}
