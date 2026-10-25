// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Durable per-submission admission and conservative processing-cost reservations.
//! A cost bound is an operator-configured unit, never a fabricated USD conversion.
use crate::{
    account_trust::{BoundedPolicy, PolicyPeriod, TrustAccount},
    db::postgres::PgBackend,
    error::DatabaseError,
};
use chrono::{DateTime, Utc};
use tokio_postgres::Transaction;
use uuid::Uuid;

#[derive(Clone)]
pub struct AdmissionLimits {
    pub window_attempts: i64,
    pub account_cost_limit: i64,
    pub global_cost_limit: i64,
    pub processing_cost_bound: i64,
    pub lease_seconds: i64,
    pub challenge_ttl_seconds: i64,
}
impl AdmissionLimits {
    pub fn from_env() -> Result<Option<Self>, &'static str> {
        match std::env::var("TRACE_COMMONS_ADMISSION_ENABLED").as_deref() {
            Err(std::env::VarError::NotPresent) | Ok("false") | Ok("0") => return Ok(None),
            Ok("true") | Ok("1") => {}
            _ => return Err("admission_configuration_invalid"),
        }
        let integer = |key| {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse::<i64>().ok())
                .ok_or("admission_configuration_missing")
        };
        let limits = Self {
            window_attempts: integer("TRACE_COMMONS_ADMISSION_WINDOW_ATTEMPTS")?,
            account_cost_limit: integer("TRACE_COMMONS_ADMISSION_ACCOUNT_COST_LIMIT")?,
            global_cost_limit: integer("TRACE_COMMONS_ADMISSION_GLOBAL_COST_LIMIT")?,
            processing_cost_bound: integer("TRACE_COMMONS_ADMISSION_PROCESSING_COST_BOUND")?,
            lease_seconds: integer("TRACE_COMMONS_ADMISSION_LEASE_SECONDS")?,
            challenge_ttl_seconds: integer("TRACE_COMMONS_ADMISSION_CHALLENGE_TTL_SECONDS")?,
        };
        if limits.window_attempts < 0
            || limits.account_cost_limit <= 0
            || limits.global_cost_limit <= 0
            || limits.processing_cost_bound <= 0
            || !(1..=86400).contains(&limits.lease_seconds)
            || !(1..=86400).contains(&limits.challenge_ttl_seconds)
        {
            return Err("admission_configuration_invalid");
        }
        Ok(Some(limits))
    }
}

/// A reservation for a *new* submission. A terminal retry is a read of an
/// already-admitted request and never builds one.
///
/// The two evidence hashes stay `Option` because `trace_reserve_admission`
/// still models both row kinds -- `window` (both null) and `attested` (both
/// set) -- and `admission_ledger_pg` covers that SQL contract. The ingest
/// path no longer produces a `window` row: see `admission::evidence_binding`,
/// which is where the refusal lives and where it is tested.
#[derive(Clone)]
pub struct AdmissionReservation {
    pub tenant_id: String,
    pub anchor_hash: String,
    pub submission_id: Uuid,
    pub body_hash: String,
    pub receipt_hash: Option<String>,
    pub challenge_hash: Option<String>,
    pub lease_id: Uuid,
    pub limits: AdmissionLimits,
}

/// Authenticated account authority. The principal is obtained from the signed
/// device credential; the database rechecks its live linkage in each transition.
/// This cannot be constructed by omitting legacy evidence hashes.
#[derive(Clone)]
pub struct AccountAdmissionReservation {
    pub account: TrustAccount,
    pub principal_ref: String,
    pub expected_trust_version: Option<i64>,
    pub submission_id: Uuid,
    pub body_hash: String,
    pub lease_id: Uuid,
    pub policy: BoundedPolicy,
    pub lease_seconds: i64,
    /// The caller verified fresh NEAR AI admission evidence for this exact
    /// account and body (`admission_evidence::verify_admission_evidence`).
    /// A new reservation made on it debits no allowance and is recorded as
    /// `authority_kind = 'attested'`. Ignored for a submission that already
    /// has a row: the kind its first reservation recorded is kept.
    pub attested: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountAdmissionStatus {
    pub authority: &'static str,
    pub trust_version: i64,
    pub policy_version: String,
    pub ready: bool,
    pub retry_after_seconds: Option<i64>,
    /// The validated effective allowance used to decide `ready`, including
    /// freshness, policy, generation and ceiling checks. Invited authority
    /// has a separate bypass and therefore reports no bounded allowance.
    pub effective_allowance: Option<i64>,
    /// The account's own earned-growth view: present only for `bounded`
    /// authority under a `tiered-v1` policy (spec rule 7). It is advisory and
    /// read from the same transaction as `ready`, which it never changes.
    pub growth: Option<AccountGrowthStatus>,
}

/// One account's allowance under a `tiered-v1` policy, read the way a
/// reservation reads it: tier 0 at the base allowance unless a fresh,
/// generation-current applied evaluation says otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountGrowthStatus {
    pub tier: i32,
    pub effective_allowance: i64,
    /// Spend in the current period, across policy versions sharing it.
    pub period_spend: i64,
    /// The fixed period's next boundary; `None` for a lifetime period.
    pub period_resets_at: Option<DateTime<Utc>>,
    /// The applied evaluation the tier came from; `None` when none applied.
    pub evaluation: Option<AppliedEvaluationProgress>,
}

/// Counts from the applied evaluation the account's tier came from, as of
/// its `as_of`. Counts only: no fact, submission or source identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedEvaluationProgress {
    pub as_of: DateTime<Utc>,
    pub units: i32,
    pub active_weeks: i32,
    /// `None` for a row written before V126 stored it.
    pub age_seconds: Option<i64>,
    pub attested_units: i32,
    /// When the account last reached a different tier under this policy:
    /// the newest applied evaluation, at or before this one, whose tier
    /// differed from the evaluation before it. `None` if it never moved.
    pub tier_changed_at: Option<DateTime<Utc>>,
}

/// Decision and advisory metadata observed in the same transaction as the debit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountAdmissionResult {
    pub decision: AdmissionDecision,
    pub authority: Option<&'static str>,
    pub retry_after_seconds: Option<i64>,
    /// On `Exhausted` only: the account's spend in the refused period, in
    /// processing-cost units. Read by the earned-trust shadow counter, which
    /// never changes the decision.
    pub period_spend: Option<i64>,
}
impl From<AdmissionDecision> for AccountAdmissionResult {
    fn from(decision: AdmissionDecision) -> Self {
        Self {
            decision,
            authority: None,
            retry_after_seconds: None,
            period_spend: None,
        }
    }
}

/// Immutable binding established by the first account reservation. Reads of
/// this row authorize only exact retries, never a different account or body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountAdmissionRecord {
    pub account_id: Uuid,
    pub body_hash: String,
}

/// Immutable identity recorded by the pre-cutover admission ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyAdmissionRecord {
    pub anchor_hash: String,
    pub body_hash: String,
    pub status: String,
    pub receipt_hash: Option<String>,
    pub challenge_hash: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionDecision {
    Reserved,
    Completed,
    Busy,
    Exhausted,
    Conflict,
    Refused,
}
fn database_refused() -> DatabaseError {
    DatabaseError::Pool("admission_database_unavailable".into())
}

/// One operator log line naming the check that refused an admission.
///
/// Label-only by construction: the argument is a `&'static str`, so no
/// tenant, account, principal, submission, header value or database error
/// text can reach the line. The client still sees only `admission_refused`.
pub fn log_admission_refusal(label: &'static str) {
    tracing::warn!(admission_refusal = label, "admission refused");
}

/// A `Refused` decision from `reserve_account_admission`, logged by check.
fn refused_at(label: &'static str) -> AccountAdmissionResult {
    log_admission_refusal(label);
    AdmissionDecision::Refused.into()
}

/// [`database_refused`], logged by stage. The error text is never logged.
fn database_refused_at(stage: &'static str) -> DatabaseError {
    log_admission_refusal(stage);
    database_refused()
}

/// Logs the stage of an error some other function produced, and passes the
/// error through unchanged.
fn logged_at(stage: &'static str) -> impl FnOnce(DatabaseError) -> DatabaseError {
    move |error| {
        log_admission_refusal(stage);
        error
    }
}

/// The static label for a ledger function's result that maps to `Refused`.
/// The SQL text itself is never logged; an unrecognised value gets its own
/// label rather than being echoed.
fn sql_refusal_label(ledger: SqlLedger, result: &str) -> &'static str {
    match (ledger, result) {
        (SqlLedger::Evidence, "refused") => "evidence_ledger_sql:refused",
        (SqlLedger::Evidence, "configuration_changed") => {
            "evidence_ledger_sql:configuration_changed"
        }
        (SqlLedger::Evidence, "evidence_refused") => "evidence_ledger_sql:evidence_refused",
        (SqlLedger::Evidence, _) => "evidence_ledger_sql:unrecognised_result",
        (SqlLedger::LegacyResume, "refused") => "legacy_resume_sql:refused",
        (SqlLedger::LegacyResume, "configuration_changed") => {
            "legacy_resume_sql:configuration_changed"
        }
        (SqlLedger::LegacyResume, "evidence_refused") => "legacy_resume_sql:evidence_refused",
        (SqlLedger::LegacyResume, _) => "legacy_resume_sql:unrecognised_result",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SqlLedger {
    Evidence,
    LegacyResume,
}

/// Every label this module logs, for the uniqueness test.
#[cfg(test)]
const LEDGER_REFUSAL_LABELS: &[&str] = &[
    "account_args_invalid",
    "account_live_missing",
    "device_not_live",
    "trust_row_missing",
    "trust_authority_invalid",
    "trust_version_mismatch",
    "bounded_with_active_grant",
    "budget_policy_frozen_mismatch",
    "db_error:account_pool",
    "db_error:account_begin",
    "db_error:account_live_query",
    "db_error:account_device_query",
    "db_error:account_trust_insert",
    "db_error:account_trust_select",
    "db_error:account_grant_query",
    "db_error:account_trust_version_overflow",
    "db_error:account_trust_update",
    "db_error:account_policy_period",
    "db_error:account_prior_select",
    "db_error:account_effective_allowance",
    "db_error:account_budget_insert",
    "db_error:account_budget_select",
    "db_error:account_period_spend",
    "db_error:account_budget_overflow",
    "db_error:account_budget_update",
    "db_error:account_lease_clock",
    "db_error:account_submission_update",
    "db_error:account_submission_insert",
    "db_error:account_commit",
    "evidence_ledger_sql:refused",
    "evidence_ledger_sql:configuration_changed",
    "evidence_ledger_sql:evidence_refused",
    "evidence_ledger_sql:unrecognised_result",
    "legacy_resume_sql:refused",
    "legacy_resume_sql:configuration_changed",
    "legacy_resume_sql:evidence_refused",
    "legacy_resume_sql:unrecognised_result",
];

async fn account_policy_period(
    tx: &Transaction<'_>,
    period: &PolicyPeriod,
) -> Result<(String, Option<i64>), DatabaseError> {
    match period {
        PolicyPeriod::Lifetime => Ok(("lifetime".into(), None)),
        PolicyPeriod::Fixed { seconds } => {
            let row = tx.query_one(
                "SELECT floor(extract(epoch FROM clock_timestamp()) / ($1::bigint)::numeric)::bigint,
                        ($1::bigint - mod(floor(extract(epoch FROM clock_timestamp()))::bigint, $1::bigint))::bigint",
                &[seconds],
            ).await.map_err(DatabaseError::Postgres)?;
            let bucket: i64 = row.get(0);
            let delay: i64 = row.get(1);
            Ok((format!("fixed:{seconds}:{bucket}"), Some(delay)))
        }
    }
}

/// Policy versions retain their own refund rows but share spending within the
/// same lifetime or fixed-duration bucket. The first colon separates the version.
async fn account_period_spend(
    tx: &Transaction<'_>,
    tenant: &str,
    account: Uuid,
    raw_period: &str,
) -> Result<i64, DatabaseError> {
    Ok(tx.query_one("SELECT COALESCE(sum(cost_used),0)::bigint FROM trace_account_admission_budget WHERE tenant_id=$1 AND account_id=$2 AND substring(period_id FROM position(':' IN period_id)+1)=$3", &[&tenant,&account,&raw_period]).await.map_err(|_|database_refused())?.get(0))
}

/// Owns an isolated database session for one in-flight submission. Dropping the
/// guard removes its connection from the pool and closes it, including cancellation;
/// a session advisory lock can never leak into a recycled pooled connection.
pub struct AdmissionProcessingGuard(Option<deadpool_postgres::ClientWrapper>);
impl Drop for AdmissionProcessingGuard {
    fn drop(&mut self) {
        if let Some(client) = self.0.take() {
            drop(client);
        }
    }
}

impl PgBackend {
    /// Caller holds the processing guard while checking the binding and
    /// verifying any offered proof. Atomic reserve still rechecks the row.
    pub async fn account_admission_record(
        &self,
        tenant: &str,
        submission: Uuid,
    ) -> Result<Option<AccountAdmissionRecord>, DatabaseError> {
        let mut client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| database_refused())?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let row=tx.query_opt("SELECT account_id,body_hash FROM trace_account_admission_submissions WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&submission]).await.map_err(|_|database_refused())?;
        tx.commit().await.map_err(|_| database_refused())?;
        Ok(row.map(|row| AccountAdmissionRecord {
            account_id: row.get(0),
            body_hash: row.get(1),
        }))
    }

    pub async fn legacy_admission_record(
        &self,
        tenant: &str,
        submission: Uuid,
    ) -> Result<Option<LegacyAdmissionRecord>, DatabaseError> {
        let mut client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| database_refused())?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let row = tx.query_opt("SELECT anchor_hash,body_hash,status,receipt_hash,challenge_hash FROM trace_admission_submissions WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&submission]).await.map_err(|_|database_refused())?;
        tx.commit().await.map_err(|_| database_refused())?;
        Ok(row.map(|row| LegacyAdmissionRecord {
            anchor_hash: row.get(0),
            body_hash: row.get(1),
            status: row.get(2),
            receipt_hash: row.get(3),
            challenge_hash: row.get(4),
        }))
    }
    pub async fn resume_legacy_admission(
        &self,
        tenant: &str,
        anchor: &str,
        submission: Uuid,
        body_hash: &str,
        lease: Uuid,
        lease_seconds: i64,
    ) -> Result<AdmissionDecision, DatabaseError> {
        let mut client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| database_refused())?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let row = tx
            .query_one(
                "SELECT trace_resume_legacy_admission($1,$2,$3,$4,$5,$6)",
                &[
                    &tenant,
                    &anchor,
                    &submission,
                    &body_hash,
                    &lease,
                    &lease_seconds,
                ],
            )
            .await
            .map_err(|_| database_refused())?;
        let decision = match row.get::<_, &str>(0) {
            "reserved" => AdmissionDecision::Reserved,
            "completed" => AdmissionDecision::Completed,
            "busy" => AdmissionDecision::Busy,
            "budget_exhausted" | "window_exhausted" => AdmissionDecision::Exhausted,
            "conflict" => AdmissionDecision::Conflict,
            other => {
                log_admission_refusal(sql_refusal_label(SqlLedger::LegacyResume, other));
                AdmissionDecision::Refused
            }
        };
        tx.commit().await.map_err(|_| database_refused())?;
        Ok(decision)
    }
    /// Advisory only. The submit transaction repeats every identity and budget
    /// check; this read never promises a processing lease.
    pub async fn account_admission_status(
        &self,
        account: &TrustAccount,
        principal: &str,
        policy: &BoundedPolicy,
    ) -> Result<Option<AccountAdmissionStatus>, DatabaseError> {
        let tenant = account.tenant_id();
        let account_id = account.account_id();
        let mut client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| database_refused())?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let live: bool = tx.query_one(
            "SELECT EXISTS(SELECT 1 FROM trace_accounts a JOIN trace_near_provisioned_devices n ON n.tenant_id=a.tenant_id AND n.account_id=a.account_id JOIN device_keys d ON d.tenant_id=n.tenant_id AND d.device_key_id=n.device_key_id JOIN trace_account_principals p ON p.tenant_id=n.tenant_id AND p.account_id=n.account_id AND p.principal_ref=n.principal_ref WHERE a.tenant_id=$1 AND a.account_id=$2 AND n.principal_ref=$3 AND a.closed_at IS NULL AND d.revoked_at IS NULL AND d.onboarding_origin IN ('near','near_ai') AND p.unlinked_at IS NULL)",
            &[&tenant,&account_id,&principal],
        ).await.map_err(|_|database_refused())?.get(0);
        if !live {
            return Ok(None);
        }
        let trust = tx.query_opt("SELECT authority,trust_version FROM trace_account_trust WHERE tenant_id=$1 AND account_id=$2", &[&tenant,&account_id]).await.map_err(|_|database_refused())?;
        let grant: bool = tx.query_one("SELECT EXISTS(SELECT 1 FROM trace_account_invite_grants WHERE tenant_id=$1 AND account_id=$2 AND revoked_at IS NULL)", &[&tenant,&account_id]).await.map_err(|_|database_refused())?.get(0);
        let (authority, version) = if let Some(trust) = trust {
            let authority: String = trust.get(0);
            let version: i64 = trust.get(1);
            if version <= 0
                || !matches!(authority.as_str(), "bounded" | "invited")
                || (authority == "bounded" && grant)
            {
                return Ok(None);
            }
            (
                if authority == "invited" && grant {
                    "invited"
                } else {
                    "bounded"
                },
                version,
            )
        } else if grant {
            return Ok(None);
        } else {
            ("bounded", 1)
        };
        let (raw_period_id, retry_after_seconds) =
            account_policy_period(&tx, policy.period()).await?;
        let period_id = format!("{}:{raw_period_id}", policy.version());
        let budget = tx.query_opt("SELECT cost_used,cost_limit,cost_bound,policy_version FROM trace_account_admission_budget WHERE tenant_id=$1 AND account_id=$2 AND period_id=$3", &[&tenant,&account_id,&period_id]).await.map_err(|_|database_refused())?;
        if let Some(budget) = budget {
            if budget.get::<_, i64>(1) != policy.bounded_allowance()
                || budget.get::<_, i64>(2) != policy.processing_cost_bound()
                || budget.get::<_, String>(3) != policy.version()
            {
                return Ok(None);
            }
        }
        let used = account_period_spend(&tx, tenant, account_id, &raw_period_id).await?;
        let mut growth = None;
        let mut effective_allowance = None;
        let ready = if authority == "invited" {
            true
        } else {
            let allowance = account_effective_allowance(
                &tx,
                tenant,
                account_id,
                policy,
                FrontierRead::Advisory,
            )
            .await?;
            effective_allowance = Some(allowance.limit);
            if policy
                .external_growth()
                .is_some_and(|g| g.source == crate::account_trust::GrowthSource::TieredV1)
            {
                growth = Some(
                    account_growth_status(
                        &tx,
                        tenant,
                        account_id,
                        policy,
                        &allowance,
                        used,
                        &raw_period_id,
                    )
                    .await?,
                );
            }
            used.checked_add(policy.processing_cost_bound())
                .is_some_and(|next| next <= allowance.limit)
        };
        tx.commit().await.map_err(|_| database_refused())?;
        Ok(Some(AccountAdmissionStatus {
            authority,
            trust_version: version,
            policy_version: policy.version().into(),
            ready,
            retry_after_seconds: if ready { None } else { retry_after_seconds },
            effective_allowance,
            growth,
        }))
    }

    /// Account, trust, submission, then budget is the common lock order used
    /// by reserve and transition. The account lock also serializes invite
    /// redemption and a future administrative grant revocation.
    pub async fn reserve_account_admission(
        &self,
        r: &AccountAdmissionReservation,
    ) -> Result<AccountAdmissionResult, DatabaseError> {
        if r.principal_ref.is_empty()
            || r.body_hash.len() != 64
            || !r
                .body_hash
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || !(1..=86400).contains(&r.lease_seconds)
        {
            return Ok(refused_at("account_args_invalid"));
        }
        let tenant = r.account.tenant_id();
        let account = r.account.account_id();
        let mut client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| database_refused_at("db_error:account_pool"))?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant)
            .await
            .map_err(logged_at("db_error:account_begin"))?;
        let account_live = tx
            .query_opt(
                "SELECT 1 FROM trace_accounts WHERE tenant_id=$1 AND account_id=$2
             AND closed_at IS NULL FOR UPDATE",
                &[&tenant, &account],
            )
            .await
            .map_err(|_| database_refused_at("db_error:account_live_query"))?;
        if account_live.is_none() {
            return Ok(refused_at("account_live_missing"));
        }
        // A separate statement sees revocation committed while we waited for
        // the account lock. Row locks serialize any later revoke/unlink with
        // this reservation transaction.
        let device_live: bool = tx
            .query_one(
                "SELECT trace_account_admission_live_device($1,$2,$3)",
                &[&tenant, &account, &r.principal_ref],
            )
            .await
            .map_err(|_| database_refused_at("db_error:account_device_query"))?
            .get(0);
        if !device_live {
            return Ok(refused_at("device_not_live"));
        }

        // A verified first use creates bounded trust only with a fully parsed,
        // explicitly activated policy. Invite grants remain independent facts.
        tx.execute(
            "INSERT INTO trace_account_trust(tenant_id,account_id,authority,trust_version)
             VALUES($1,$2,'bounded',1) ON CONFLICT DO NOTHING",
            &[&tenant, &account],
        )
        .await
        .map_err(|_| database_refused_at("db_error:account_trust_insert"))?;
        let trust = tx
            .query_opt(
                "SELECT authority,trust_version FROM trace_account_trust
             WHERE tenant_id=$1 AND account_id=$2 FOR UPDATE",
                &[&tenant, &account],
            )
            .await
            .map_err(|_| database_refused_at("db_error:account_trust_select"))?;
        let Some(trust) = trust else {
            return Ok(refused_at("trust_row_missing"));
        };
        let mut authority: String = trust.get(0);
        let mut version: i64 = trust.get(1);
        if version <= 0 || !matches!(authority.as_str(), "bounded" | "invited") {
            return Ok(refused_at("trust_authority_invalid"));
        }
        if let Some(expected) = r.expected_trust_version {
            if expected != version {
                return Ok(refused_at("trust_version_mismatch"));
            }
        }
        let active_grant: bool = tx
            .query_one(
                "SELECT trace_account_admission_active_grant($1,$2)",
                &[&tenant, &account],
            )
            .await
            .map_err(|_| database_refused_at("db_error:account_grant_query"))?
            .get(0);
        if authority == "invited" && !active_grant {
            version = version
                .checked_add(1)
                .ok_or_else(|| database_refused_at("db_error:account_trust_version_overflow"))?;
            tx.execute("UPDATE trace_account_trust SET authority='bounded',trust_version=$3,updated_at=clock_timestamp() WHERE tenant_id=$1 AND account_id=$2", &[&tenant,&account,&version]).await.map_err(|_|database_refused_at("db_error:account_trust_update"))?;
            authority = "bounded".into();
        } else if authority == "bounded" && active_grant {
            return Ok(refused_at("bounded_with_active_grant"));
        }
        let (raw_period_id, retry_after_seconds) = account_policy_period(&tx, r.policy.period())
            .await
            .map_err(logged_at("db_error:account_policy_period"))?;
        let period_id = format!("{}:{raw_period_id}", r.policy.version());
        // Every submission ID binds once to an account and exact body, even
        // when a lease expires or the policy changes.
        let prior = tx.query_opt(
            "SELECT account_id,body_hash,status,lease_expires_at > clock_timestamp(),authority_kind FROM trace_account_admission_submissions
             WHERE tenant_id=$1 AND submission_id=$2 FOR UPDATE",
            &[&tenant, &r.submission_id],
        ).await.map_err(|_| database_refused_at("db_error:account_prior_select"))?;
        if let Some(ref prior) = prior {
            if prior.get::<_, Uuid>(0) != account || prior.get::<_, String>(1) != r.body_hash {
                return Ok(AdmissionDecision::Conflict.into());
            }
            let status: String = prior.get(2);
            if status == "completed" {
                return Ok(AdmissionDecision::Completed.into());
            }
            let live_lease: bool = prior.get(3);
            if status != "released" && live_lease {
                return Ok(AdmissionDecision::Busy.into());
            }
        }
        // The admitting authority is fixed by the first reservation, like the
        // account and body binding: a retry neither gains nor loses it.
        let attested = match prior {
            Some(ref prior) => prior.get::<_, String>(4) == "attested",
            None => r.attested,
        };
        let authority_kind = if attested { "attested" } else { "account" };
        // Attested uploads buy volume directly: no budget row is created or
        // read, so neither the allowance nor its freeze rule below applies.
        let charged = authority != "invited" && !attested;
        let allowance = if charged {
            account_effective_allowance(&tx, tenant, account, &r.policy, FrontierRead::Locked)
                .await
                .map_err(logged_at("db_error:account_effective_allowance"))?
        } else {
            EffectiveAllowance {
                limit: r.policy.bounded_allowance(),
                tier: None,
                digest: None,
                evaluation_id: None,
            }
        };
        if charged {
            tx.execute("INSERT INTO trace_account_admission_budget(tenant_id,account_id,period_id,policy_version,cost_limit,cost_bound) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING", &[&tenant,&account,&period_id,&r.policy.version(),&r.policy.bounded_allowance(),&r.policy.processing_cost_bound()]).await.map_err(|_|database_refused_at("db_error:account_budget_insert"))?;
            let budget = tx.query_one("SELECT cost_used,cost_limit,cost_bound,policy_version FROM trace_account_admission_budget WHERE tenant_id=$1 AND account_id=$2 AND period_id=$3 FOR UPDATE", &[&tenant,&account,&period_id]).await.map_err(|_|database_refused_at("db_error:account_budget_select"))?;
            let used: i64 = budget.get(0);
            if budget.get::<_, i64>(1) != r.policy.bounded_allowance()
                || budget.get::<_, i64>(2) != r.policy.processing_cost_bound()
                || budget.get::<_, String>(3) != r.policy.version()
            {
                return Ok(refused_at("budget_policy_frozen_mismatch"));
            }
            let total_used = account_period_spend(&tx, tenant, account, &raw_period_id)
                .await
                .map_err(logged_at("db_error:account_period_spend"))?;
            let next = total_used
                .checked_add(r.policy.processing_cost_bound())
                .ok_or_else(|| database_refused_at("db_error:account_budget_overflow"))?;
            if next > allowance.limit {
                return Ok(AccountAdmissionResult {
                    decision: AdmissionDecision::Exhausted,
                    authority: Some("bounded"),
                    retry_after_seconds,
                    period_spend: Some(total_used),
                });
            }
            tx.execute("UPDATE trace_account_admission_budget SET cost_used=$4 WHERE tenant_id=$1 AND account_id=$2 AND period_id=$3", &[&tenant,&account,&period_id,&(used + r.policy.processing_cost_bound())]).await.map_err(|_|database_refused_at("db_error:account_budget_update"))?;
        }
        let lease_expires: DateTime<Utc> = tx
            .query_one(
                "SELECT clock_timestamp()+make_interval(secs => $1::bigint::double precision)",
                &[&r.lease_seconds],
            )
            .await
            .map_err(|_| database_refused_at("db_error:account_lease_clock"))?
            .get(0);
        if prior.is_some() {
            tx.execute("UPDATE trace_account_admission_submissions SET status='reserved',lease_id=$3,lease_expires_at=$4,last_cost_bound=$5,last_charged=$6,trust_version=$7,policy_version=$8,period_id=$9,earned_tier=$10,trust_evaluation_digest=$11 WHERE tenant_id=$1 AND submission_id=$2",
                &[&tenant,&r.submission_id,&r.lease_id,&lease_expires,&r.policy.processing_cost_bound(),&charged,&version,&r.policy.version(),&period_id,&allowance.tier,&allowance.digest]).await.map_err(|_|database_refused_at("db_error:account_submission_update"))?;
        } else {
            tx.execute("INSERT INTO trace_account_admission_submissions(tenant_id,submission_id,account_id,body_hash,authority_kind,trust_version,policy_version,period_id,status,lease_id,lease_expires_at,last_cost_bound,last_charged,earned_tier,trust_evaluation_digest) VALUES($1,$2,$3,$4,$14,$5,$6,$7,'reserved',$8,$9,$10,$11,$12,$13)",
                &[&tenant,&r.submission_id,&account,&r.body_hash,&version,&r.policy.version(),&period_id,&r.lease_id,&lease_expires,&r.policy.processing_cost_bound(),&charged,&allowance.tier,&allowance.digest,&authority_kind]).await.map_err(|_|database_refused_at("db_error:account_submission_insert"))?;
        }
        tx.commit()
            .await
            .map_err(|_| database_refused_at("db_error:account_commit"))?;
        Ok(AccountAdmissionResult {
            decision: AdmissionDecision::Reserved,
            authority: Some(if attested {
                "attested"
            } else if charged {
                "bounded"
            } else {
                "invited"
            }),
            retry_after_seconds: None,
            period_spend: None,
        })
    }

    pub async fn transition_account_admission(
        &self,
        tenant: &str,
        principal: &str,
        account: Uuid,
        submission: Uuid,
        lease: Uuid,
        next: &str,
    ) -> Result<bool, DatabaseError> {
        let mut client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| database_refused())?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let account_row = tx.query_opt(
            "SELECT closed_at FROM trace_accounts WHERE tenant_id=$1 AND account_id=$2 FOR UPDATE",
            &[&tenant,&account],
        ).await.map_err(|_|database_refused())?;
        let Some(account_row) = account_row else {
            return Ok(false);
        };
        let trust = tx.query_opt("SELECT trust_version,authority FROM trace_account_trust WHERE tenant_id=$1 AND account_id=$2 FOR UPDATE", &[&tenant,&account]).await.map_err(|_|database_refused())?;
        let Some(trust) = trust else {
            return Ok(false);
        };
        let prior = tx.query_opt("SELECT account_id,trust_version,status,lease_expires_at > clock_timestamp(),last_cost_bound,last_charged,period_id FROM trace_account_admission_submissions WHERE tenant_id=$1 AND submission_id=$2 AND lease_id=$3 FOR UPDATE", &[&tenant,&submission,&lease]).await.map_err(|_|database_refused())?;
        let Some(prior) = prior else {
            return Ok(false);
        };
        if prior.get::<_, Uuid>(0) != account {
            return Ok(false);
        }
        let status: String = prior.get(2);
        let changed = match next {
            "processing" if status == "reserved" => {
                let closed: Option<DateTime<Utc>> = account_row.get(0);
                let live_lease: bool = prior.get(3);
                let version: i64 = trust.get(0);
                let authority: String = trust.get(1);
                let grant: bool = tx
                    .query_one(
                        "SELECT trace_account_admission_active_grant($1,$2)",
                        &[&tenant, &account],
                    )
                    .await
                    .map_err(|_| database_refused())?
                    .get(0);
                let live: bool = tx
                    .query_one(
                        "SELECT trace_account_admission_live_device($1,$2,$3)",
                        &[&tenant, &account, &principal],
                    )
                    .await
                    .map_err(|_| database_refused())?
                    .get(0);
                if closed.is_some()
                    || !live
                    || !live_lease
                    || version != prior.get::<_, i64>(1)
                    || (authority == "invited") != grant
                {
                    return Ok(false);
                }
                tx.execute("UPDATE trace_account_admission_submissions SET status='processing',ever_processed=TRUE WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&submission]).await.map_err(|_|database_refused())?;
                true
            }
            "completed" if status == "processing" || status == "completed" => {
                tx.execute("UPDATE trace_account_admission_submissions SET status='completed' WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&submission]).await.map_err(|_|database_refused())?;
                true
            }
            // `released` gives a `reserved` attempt's charge back. A
            // `processing` attempt keeps its charge, unless the caller knows
            // the upload's refusal wrote nothing (`released_without_effect`,
            // #1237): then it is released and refunded the same way.
            "released" | "released_without_effect"
                if (next == "released" && status == "reserved")
                    || (next == "released_without_effect" && status == "processing") =>
            {
                let charged: bool = prior.get(5);
                if charged {
                    let period_id: String = prior.get(6);
                    let cost: i64 = prior.get(4);
                    let updated = tx.execute("UPDATE trace_account_admission_budget SET cost_used=cost_used-$4 WHERE tenant_id=$1 AND account_id=$2 AND period_id=$3 AND cost_used >= $4", &[&tenant,&account,&period_id,&cost]).await.map_err(|_|database_refused())?;
                    if updated != 1 {
                        return Ok(false);
                    }
                }
                tx.execute("UPDATE trace_account_admission_submissions SET status='released',last_charged=FALSE WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&submission]).await.map_err(|_|database_refused())?;
                true
            }
            _ => false,
        };
        if changed {
            tx.commit().await.map_err(|_| database_refused())?;
        }
        Ok(changed)
    }
    /// Turns on the gate-write invalidation that external growth depends on.
    ///
    /// Until this runs, a gate write takes no dependency locks and scans no
    /// facts, and the evaluation writer refuses. The first call advances every
    /// frontier, so no evaluation recorded before it can be consumed; later
    /// calls change nothing. There is deliberately no runtime path back.
    pub async fn enable_external_account_trust_growth(&self) -> Result<(), DatabaseError> {
        let client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| database_refused())?;
        client
            .query_one("SELECT trace_account_trust_enable_external_growth()", &[])
            .await
            .map_err(|_| database_refused())?;
        Ok(())
    }

    /// Deletes dependency-lock rows nobody has touched for a week, at most
    /// `limit` of them; a dry run only counts them. The rows exist to
    /// serialize concurrent gate and fact writes, so a row idle that long
    /// guards no transaction still running. Returns a count only.
    pub async fn prune_account_trust_dependency_locks(
        &self,
        limit: i32,
        dry_run: bool,
    ) -> Result<u64, DatabaseError> {
        let client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| database_refused())?;
        let count: i64 = client
            .query_one(
                "SELECT trace_account_trust_prune_dependency_locks($1,$2)",
                &[&limit, &dry_run],
            )
            .await
            .map_err(|_| database_refused())?
            .get(0);
        u64::try_from(count).map_err(|_| database_refused())
    }

    /// Additional contract checks only required by an external growth policy.
    pub async fn external_account_trust_runtime_ready(&self) -> Result<bool, DatabaseError> {
        self.account_trust_runtime_ready(false).await
    }

    /// The independent client-cap worker has no reservation/transition or
    /// direct write authority. Keep the admission contract above unchanged.
    pub async fn client_upload_trust_runtime_ready(&self) -> Result<bool, DatabaseError> {
        self.account_trust_runtime_ready(true).await
    }

    async fn account_trust_runtime_ready(
        &self,
        client_upload_only: bool,
    ) -> Result<bool, DatabaseError> {
        let client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| database_refused())?;
        let ready: Option<bool> = client.query_one(r#"SELECT
            NOT r.rolsuper AND NOT r.rolbypassrls
            AND e.rolname IS NOT NULL AND NOT e.rolsuper AND NOT e.rolbypassrls AND NOT e.rolcanlogin
            AND NOT pg_has_role(e.oid,to_regrole('trace_account_trust_worker'),'MEMBER')
            AND has_function_privilege(e.oid,to_regprocedure('public.trace_account_trust_worker_accounts(text,uuid,bigint)'),'EXECUTE')
            AND has_function_privilege(e.oid,to_regprocedure('public.trace_account_trust_evaluation_inputs(text,uuid)'),'EXECUTE')
            AND has_function_privilege(e.oid,to_regprocedure('public.trace_account_trust_input_generation(text,uuid)'),'EXECUTE')
            AND ($1::boolean OR (
                has_function_privilege(current_user,to_regprocedure('public.trace_account_trust_lock_input_generation(text,uuid)'),'EXECUTE')
                AND has_function_privilege(current_user,to_regprocedure('public.trace_account_trust_prune_dependency_locks(integer,boolean)'),'EXECUTE')))
            AND has_function_privilege(current_user,to_regprocedure('public.trace_account_trust_input_generation(text,uuid)'),'EXECUTE')
            AND (NOT $1::boolean OR (
                NOT r.rolcreatedb AND NOT r.rolcreaterole AND NOT r.rolreplication
                AND NOT pg_has_role(current_user,to_regrole('trace_account_admission_runtime'),'MEMBER')
                AND has_schema_privilege(current_user,'public','USAGE')
                AND NOT has_schema_privilege(current_user,'public','CREATE')
                AND NOT EXISTS (
                    SELECT 1 FROM pg_class direct_table JOIN pg_namespace direct_schema
                      ON direct_schema.oid=direct_table.relnamespace
                    WHERE direct_schema.nspname='public' AND direct_table.relkind IN ('r','p','v','m','f')
                      AND (direct_table.relowner=r.oid
                        OR has_table_privilege(current_user,direct_table.oid,'INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES')
                        OR has_any_column_privilege(current_user,direct_table.oid,'INSERT,UPDATE,REFERENCES')))
                AND NOT EXISTS (
                    SELECT 1 FROM pg_class readable JOIN pg_namespace readable_schema
                      ON readable_schema.oid=readable.relnamespace
                      JOIN pg_attribute column_access ON column_access.attrelid=readable.oid
                        AND column_access.attnum>0 AND NOT column_access.attisdropped
                    WHERE readable_schema.nspname='public' AND readable.relkind IN ('r','p','v','m','f')
                      AND readable.relname<>'trace_account_trust_evaluations'
                      AND EXISTS (
                        SELECT 1 FROM pg_roles readable_actor
                        WHERE (readable_actor.oid=r.oid OR pg_has_role(r.oid,readable_actor.oid,'MEMBER'))
                          AND has_column_privilege(readable_actor.oid,readable.oid,column_access.attnum,'SELECT'))
                      AND NOT EXISTS (
                        SELECT 1 FROM (VALUES
                            ('trace_accounts',ARRAY['tenant_id','account_id','closed_at']),
                            ('trace_account_principals',ARRAY['tenant_id','account_id','principal_ref','unlinked_at']),
                            ('trace_near_provisioned_devices',ARRAY['tenant_id','account_id','principal_ref','device_key_id','anchor_hash']),
                            ('device_keys',ARRAY['tenant_id','device_key_id','revoked_at','onboarding_origin']),
                            ('trace_account_invite_grants',ARRAY['tenant_id','account_id','revoked_at']),
                            ('trace_account_trust',ARRAY['tenant_id','account_id','authority','trust_version']),
                            ('trace_account_admission_budget',ARRAY['tenant_id','account_id','period_id','cost_used','cost_limit','cost_bound','policy_version'])
                        ) allowed(relation_name,column_names)
                        WHERE readable.relname=allowed.relation_name
                          AND column_access.attname=ANY(allowed.column_names)))
                AND NOT EXISTS (
                    SELECT 1 FROM (VALUES
                        ('trace_accounts',ARRAY['tenant_id','account_id','closed_at']),
                        ('trace_account_principals',ARRAY['tenant_id','account_id','principal_ref','unlinked_at']),
                        ('trace_near_provisioned_devices',ARRAY['tenant_id','account_id','principal_ref','device_key_id','anchor_hash']),
                        ('device_keys',ARRAY['tenant_id','device_key_id','revoked_at','onboarding_origin']),
                        ('trace_account_invite_grants',ARRAY['tenant_id','account_id','revoked_at']),
                        ('trace_account_trust',ARRAY['tenant_id','account_id','authority','trust_version']),
                        ('trace_account_admission_budget',ARRAY['tenant_id','account_id','period_id','cost_used','cost_limit','cost_bound','policy_version'])
                    ) required(relation_name,column_names)
                    CROSS JOIN LATERAL unnest(required.column_names) required_column(column_name)
                    LEFT JOIN pg_class status_table ON status_table.oid=to_regclass('public.'||required.relation_name)
                    LEFT JOIN pg_attribute status_column ON status_column.attrelid=status_table.oid
                      AND status_column.attname=required_column.column_name
                      AND status_column.attnum>0 AND NOT status_column.attisdropped
                    WHERE NOT COALESCE(status_table.relrowsecurity AND status_table.relforcerowsecurity
                      AND status_table.relowner<>r.oid
                      AND has_column_privilege(current_user,status_table.oid,status_column.attnum,'SELECT'),FALSE))
                AND NOT EXISTS (
                    SELECT 1 FROM pg_roles reachable
                    WHERE reachable.oid<>r.oid AND pg_has_role(r.oid,reachable.oid,'MEMBER')
                      AND (reachable.rolsuper OR reachable.rolbypassrls OR reachable.rolcreatedb
                        OR reachable.rolcreaterole OR reachable.rolreplication
                        OR has_schema_privilege(reachable.oid,'public','CREATE')
                        OR EXISTS(SELECT 1 FROM pg_proc owned WHERE owned.proowner=reachable.oid)
                        OR EXISTS(SELECT 1 FROM pg_namespace owned WHERE owned.nspowner=reachable.oid)
                        OR EXISTS(SELECT 1 FROM pg_database owned WHERE owned.datdba=reachable.oid)
                        OR EXISTS (
                            SELECT 1 FROM pg_class reachable_table JOIN pg_namespace reachable_schema
                              ON reachable_schema.oid=reachable_table.relnamespace
                            WHERE reachable_schema.nspname='public' AND reachable_table.relkind IN ('r','p','v','m','f')
                              AND (reachable_table.relowner=reachable.oid
                                OR has_table_privilege(reachable.oid,reachable_table.oid,'INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES')
                                OR has_any_column_privilege(reachable.oid,reachable_table.oid,'INSERT,UPDATE,REFERENCES')))))
                AND NOT EXISTS(SELECT 1 FROM pg_proc owned_function WHERE owned_function.proowner=r.oid)
                AND NOT EXISTS(SELECT 1 FROM pg_namespace owned_schema WHERE owned_schema.nspowner=r.oid)
                AND NOT EXISTS(SELECT 1 FROM pg_database owned_database WHERE owned_database.datdba=r.oid)
                AND NOT EXISTS (
                    SELECT 1 FROM pg_proc forbidden JOIN pg_namespace forbidden_schema
                      ON forbidden_schema.oid=forbidden.pronamespace
                    WHERE forbidden_schema.nspname='public'
                      AND forbidden.proname IN ('trace_reserve_admission','trace_transition_admission',
                        'trace_resume_legacy_admission','trace_account_admission_live_device',
                        'trace_account_admission_active_grant','trace_account_admission_linkage_ready',
                        'trace_account_trust_lock_input_generation','trace_account_trust_prune_dependency_locks',
                        'trace_record_external_account_trust_evaluation')
                      AND EXISTS (
                        SELECT 1 FROM pg_roles executable_actor
                        WHERE (executable_actor.oid=r.oid OR pg_has_role(r.oid,executable_actor.oid,'MEMBER'))
                          AND has_function_privilege(executable_actor.oid,forbidden.oid,'EXECUTE')))))
            AND COALESCE(has_function_privilege(current_user,to_regprocedure('public.trace_account_trust_external_growth_enabled()'),'EXECUTE')
                AND public.trace_account_trust_external_growth_enabled(),FALSE)
            AND has_function_privilege(e.oid,f.oid,'EXECUTE')
            AND f.prosecdef AND f.proowner=to_regrole('trace_account_trust_evaluation_guard')
            AND NOT has_function_privilege(current_user,f.oid,'EXECUTE')
            AND has_table_privilege(current_user,c.oid,'SELECT')
            AND NOT has_table_privilege(current_user,c.oid,'INSERT,UPDATE,DELETE')
            AND c.relrowsecurity AND c.relforcerowsecurity AND c.relowner<>r.oid
            AND g.relrowsecurity AND g.relforcerowsecurity AND g.relowner<>r.oid
            AND NOT has_table_privilege(e.oid,g.oid,'INSERT,UPDATE,DELETE')
            AND NOT has_table_privilege(e.oid,c.oid,'INSERT,UPDATE,DELETE')
            AND NOT has_table_privilege(current_user,g.oid,'INSERT,UPDATE,DELETE')
            AND l.relrowsecurity AND l.relforcerowsecurity AND l.relowner<>r.oid
            AND NOT has_table_privilege(current_user,l.oid,'SELECT,INSERT,UPDATE,DELETE')
            AND NOT has_table_privilege(e.oid,l.oid,'SELECT,INSERT,UPDATE,DELETE')
            AND NOT i.rolsuper AND NOT i.rolbypassrls AND NOT i.rolcanlogin
            AND a.rolname IS NOT NULL AND NOT a.rolsuper AND NOT a.rolbypassrls AND NOT a.rolcanlogin
            AND h.prosecdef AND h.proowner=a.oid
            AND h.proconfig @> ARRAY['search_path=pg_catalog']::text[]
            AND has_function_privilege(a.oid,h.oid,'EXECUTE')
            AND NOT has_function_privilege(current_user,h.oid,'EXECUTE')
            AND NOT EXISTS(SELECT 1 FROM aclexplode(COALESCE(h.proacl,acldefault('f',h.proowner))) acl
                WHERE acl.grantee=0 AND acl.privilege_type='EXECUTE')
            AND EXISTS (SELECT 1 FROM pg_proc q
                WHERE q.oid=to_regprocedure('public.trace_account_trust_status_acceptance_outcome(text,uuid,uuid)')
                  AND q.prosecdef AND q.proowner=a.oid
                  AND q.proconfig @> ARRAY['search_path=pg_catalog']::text[]
                  AND has_function_privilege(a.oid,q.oid,'EXECUTE')
                  AND NOT has_function_privilege(current_user,q.oid,'EXECUTE')
                  AND NOT EXISTS (SELECT 1 FROM aclexplode(COALESCE(q.proacl,acldefault('f',q.proowner))) acl
                      WHERE acl.grantee=0 AND acl.privilege_type='EXECUTE'))
            AND NOT EXISTS (
                SELECT 1 FROM (VALUES
                    ('public.trace_device_upload_principals','account_trust_upload_alias_frontier'),
                    ('public.device_keys','account_trust_upload_device_frontier'),
                    ('public.trace_submissions','account_trust_upload_submission_frontier')
                ) expected(relation_name,trigger_name)
                WHERE NOT EXISTS (
                    SELECT 1 FROM pg_trigger t JOIN pg_proc p ON p.oid=t.tgfoid
                    WHERE t.tgrelid=to_regclass(expected.relation_name)
                      AND t.tgname=expected.trigger_name AND t.tgenabled='O'
                      AND t.tgfoid=to_regprocedure('public.trace_account_trust_advance_upload_frontiers()')
                      AND p.prosecdef AND p.proowner=i.oid
                      AND p.proconfig @> ARRAY['search_path=pg_catalog']::text[]))
            AND EXISTS(SELECT 1 FROM pg_trigger t JOIN pg_proc p ON p.oid=t.tgfoid
                WHERE t.tgrelid=to_regclass('public.trace_gate_decisions')
                AND t.tgname='account_trust_gate_frontier' AND t.tgenabled='O'
                AND p.prosecdef AND p.proowner=i.oid)
            FROM pg_roles r LEFT JOIN pg_roles e ON e.rolname='trace_account_trust_evaluator'
            LEFT JOIN pg_proc f ON f.oid=to_regprocedure('public.trace_record_external_account_trust_evaluation(text,uuid,uuid,text,text,timestamp with time zone,integer,bigint,text,bigint)')
            LEFT JOIN pg_class c ON c.oid=to_regclass('public.trace_account_trust_evaluations')
            LEFT JOIN pg_class g ON g.oid=to_regclass('public.trace_account_trust_frontiers')
            LEFT JOIN pg_class l ON l.oid=to_regclass('public.trace_account_trust_dependency_locks')
            LEFT JOIN pg_roles i ON i.rolname='trace_account_trust_input_guard'
            LEFT JOIN pg_roles a ON a.rolname='trace_account_admission_guard'
            LEFT JOIN pg_proc h ON h.oid=to_regprocedure('public.trace_account_trust_live_owned_principal(text,uuid,text)')
            WHERE r.rolname=current_user"#, &[&client_upload_only]).await.map_err(|_| database_refused())?.get(0);
        Ok(ready == Some(true))
    }

    /// Checks the runtime login itself, then the boolean-only fleet linkage seam.
    pub async fn account_admission_runtime_ready(&self) -> Result<bool, DatabaseError> {
        let client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| database_refused())?;
        let ready: Option<bool> = client.query_one(r#"SELECT NOT r.rolsuper AND NOT r.rolbypassrls
 AND has_schema_privilege(current_user,'public','USAGE')
 AND NOT COALESCE(pg_has_role(current_user,to_regrole('trace_admission_guard'),'MEMBER'),FALSE)
 AND NOT COALESCE(pg_has_role(current_user,to_regrole('trace_account_admission_guard'),'MEMBER'),FALSE)
 AND NOT COALESCE(pg_has_role(current_user,to_regrole('trace_account_readiness_guard'),'MEMBER'),FALSE)
 AND NOT COALESCE(pg_has_role(current_user,to_regrole('trace_onboarding_retention_guard'),'MEMBER'),FALSE)
 AND has_column_privilege(current_user,'public.trace_accounts','tenant_id','SELECT')
 AND has_column_privilege(current_user,'public.trace_accounts','account_id','SELECT')
 AND has_column_privilege(current_user,'public.trace_accounts','closed_at','SELECT')
 AND has_column_privilege(current_user,'public.trace_account_principals','tenant_id','SELECT')
 AND has_column_privilege(current_user,'public.trace_account_principals','account_id','SELECT')
 AND has_column_privilege(current_user,'public.trace_account_principals','principal_ref','SELECT')
 AND has_column_privilege(current_user,'public.trace_account_principals','unlinked_at','SELECT')
 AND has_column_privilege(current_user,'public.trace_near_provisioned_devices','tenant_id','SELECT')
 AND has_column_privilege(current_user,'public.trace_near_provisioned_devices','account_id','SELECT')
 AND has_column_privilege(current_user,'public.trace_near_provisioned_devices','principal_ref','SELECT')
 AND has_column_privilege(current_user,'public.trace_near_provisioned_devices','device_key_id','SELECT')
 AND has_column_privilege(current_user,'public.trace_near_provisioned_devices','anchor_hash','SELECT')
 AND has_column_privilege(current_user,'public.device_keys','tenant_id','SELECT')
 AND has_column_privilege(current_user,'public.device_keys','device_key_id','SELECT')
 AND has_column_privilege(current_user,'public.device_keys','revoked_at','SELECT')
 AND has_column_privilege(current_user,'public.device_keys','onboarding_origin','SELECT')
 AND has_column_privilege(current_user,'public.trace_account_invite_grants','tenant_id','SELECT')
 AND has_column_privilege(current_user,'public.trace_account_invite_grants','account_id','SELECT')
 AND has_column_privilege(current_user,'public.trace_account_invite_grants','revoked_at','SELECT')
 AND has_column_privilege(current_user,'public.trace_admission_submissions','tenant_id','SELECT')
 AND has_column_privilege(current_user,'public.trace_admission_submissions','submission_id','SELECT')
 AND has_column_privilege(current_user,'public.trace_admission_submissions','anchor_hash','SELECT')
 AND has_column_privilege(current_user,'public.trace_admission_submissions','body_hash','SELECT')
 AND has_column_privilege(current_user,'public.trace_admission_submissions','status','SELECT')
 AND has_column_privilege(current_user,'public.trace_admission_submissions','receipt_hash','SELECT')
 AND has_column_privilege(current_user,'public.trace_admission_submissions','challenge_hash','SELECT')
 AND has_column_privilege(current_user,'public.trace_accounts','created_at','UPDATE')
 AND NOT has_column_privilege(current_user,'public.trace_accounts','account_id','UPDATE')
 AND has_table_privilege(current_user,'public.trace_account_trust','SELECT')
 AND has_table_privilege(current_user,'public.trace_account_trust','INSERT')
 AND has_column_privilege(current_user,'public.trace_account_trust','authority','UPDATE')
 AND has_column_privilege(current_user,'public.trace_account_trust','trust_version','UPDATE')
 AND has_column_privilege(current_user,'public.trace_account_trust','updated_at','UPDATE')
 AND has_table_privilege(current_user,'public.trace_account_admission_budget','SELECT')
 AND has_table_privilege(current_user,'public.trace_account_admission_budget','INSERT')
 AND has_table_privilege(current_user,'public.trace_account_admission_budget','UPDATE')
 AND has_table_privilege(current_user,'public.trace_account_admission_submissions','SELECT')
 AND has_table_privilege(current_user,'public.trace_account_admission_submissions','INSERT')
 AND has_table_privilege(current_user,'public.trace_account_admission_submissions','UPDATE')
 AND has_function_privilege(current_user,'public.trace_account_admission_live_device(text,uuid,text)','EXECUTE')
 AND has_function_privilege(current_user,'public.trace_account_admission_active_grant(text,uuid)','EXECUTE')
 AND has_function_privilege(current_user,'public.trace_resume_legacy_admission(text,text,uuid,text,uuid,bigint)','EXECUTE')
 AND has_function_privilege(current_user,'public.trace_transition_admission(text,uuid,uuid,text)','EXECUTE')
 AND has_function_privilege(current_user,'public.trace_account_admission_linkage_ready()','EXECUTE')
 AND (SELECT count(*)=9 AND bool_and(c.relrowsecurity AND c.relforcerowsecurity AND c.relowner<>r.oid) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relname IN ('trace_accounts','trace_account_principals','trace_near_provisioned_devices','device_keys','trace_account_invite_grants','trace_admission_submissions','trace_account_trust','trace_account_admission_budget','trace_account_admission_submissions')) FROM pg_roles r WHERE r.rolname=current_user"#, &[]).await.map_err(|_|database_refused())?.get(0);
        if ready != Some(true) {
            return Ok(false);
        }
        Ok(client
            .query_one("SELECT trace_account_admission_linkage_ready()", &[])
            .await
            .map_err(|_| database_refused())?
            .get(0))
    }
    pub(crate) async fn check_admission_runtime(&self) -> Result<bool, DatabaseError> {
        let client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| database_refused())?;
        let row=client.query_one(r#"
        SELECT NOT r.rolsuper AND NOT r.rolbypassrls
          AND NOT pg_has_role(current_user,'trace_admission_guard','MEMBER')
          AND NOT COALESCE(pg_has_role(current_user,to_regrole('trace_onboarding_retention_guard'),'MEMBER'),FALSE)
          AND NOT has_table_privilege(current_user,'public.trace_admission_receipts','SELECT,INSERT,UPDATE,DELETE')
          AND NOT has_table_privilege(current_user,'public.trace_admission_global_budget','SELECT,INSERT,UPDATE,DELETE')
          AND has_function_privilege(current_user,'public.trace_reserve_admission(text,text,uuid,text,text,text,bigint,bigint,bigint,bigint,uuid,bigint)','EXECUTE')
          AND has_function_privilege(current_user,'public.trace_transition_admission(text,uuid,uuid,text)','EXECUTE')
          AND (SELECT bool_and(c.relrowsecurity AND c.relforcerowsecurity AND c.relowner<>r.oid)
            FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
            WHERE n.nspname='public' AND c.relname IN ('trace_admission_challenges','trace_admission_accounts','trace_admission_submissions','trace_admission_receipts','trace_admission_global_budget'))
          AND (SELECT bool_and(has_table_privilege(current_user,t,p)) FROM
            unnest(ARRAY['public.trace_admission_challenges','public.trace_admission_accounts','public.trace_admission_submissions']) t
            CROSS JOIN unnest(ARRAY['SELECT','INSERT','UPDATE']) p)
        FROM pg_roles r WHERE r.rolname=current_user
        "#,&[]).await.map_err(|_|database_refused())?;
        Ok(row.get::<_, Option<bool>>(0).unwrap_or(false))
    }
    pub(crate) async fn completed_admission(
        &self,
        tenant: &str,
        anchor: &str,
        submission: Uuid,
        body_hash: &str,
    ) -> Result<bool, DatabaseError> {
        let mut client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| database_refused())?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let found=tx.query_opt("SELECT 1 FROM trace_admission_submissions WHERE tenant_id=$1 AND anchor_hash=$2 AND submission_id=$3 AND body_hash=$4 AND status='completed'", &[&tenant,&anchor,&submission,&body_hash]).await.map_err(|_|database_refused())?.is_some();
        tx.commit().await.map_err(|_| database_refused())?;
        Ok(found)
    }
    pub(crate) async fn lock_admission(
        &self,
        tenant: &str,
        submission: Uuid,
    ) -> Result<Option<AdmissionProcessingGuard>, DatabaseError> {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(b"trace-admission-processing.v1:");
        hash.update((tenant.len() as u64).to_be_bytes());
        hash.update(tenant.as_bytes());
        hash.update(submission.as_bytes());
        let digest = hash.finalize();
        let key = i64::from_be_bytes(digest[..8].try_into().map_err(|_| database_refused())?);
        let client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| database_refused())?;
        let acquired: bool = client
            .query_one("SELECT pg_try_advisory_lock($1)", &[&key])
            .await
            .map_err(|_| database_refused())?
            .get(0);
        Ok(acquired
            .then(|| AdmissionProcessingGuard(Some(deadpool_postgres::Object::take(client)))))
    }

    pub(crate) async fn insert_admission_challenge(
        &self,
        tenant: &str,
        anchor: &str,
        challenge: &str,
        expires: DateTime<Utc>,
    ) -> Result<(), DatabaseError> {
        let mut client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| database_refused())?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        tx.execute("INSERT INTO trace_admission_challenges(tenant_id,anchor_hash,challenge_hash,expires_at) VALUES($1,$2,$3,$4)",
            &[&tenant,&anchor,&challenge,&expires]).await.map_err(|_| database_refused())?;
        tx.commit().await.map_err(|_| database_refused())
    }
    pub(crate) async fn reserve_admission(
        &self,
        r: &AdmissionReservation,
    ) -> Result<AdmissionDecision, DatabaseError> {
        let mut client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| database_refused())?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, &r.tenant_id).await?;
        let row = tx
            .query_one(
                "SELECT trace_reserve_admission($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
                &[
                    &r.tenant_id,
                    &r.anchor_hash,
                    &r.submission_id,
                    &r.body_hash,
                    &r.receipt_hash,
                    &r.challenge_hash,
                    &r.limits.window_attempts,
                    &r.limits.account_cost_limit,
                    &r.limits.global_cost_limit,
                    &r.limits.processing_cost_bound,
                    &r.lease_id,
                    &r.limits.lease_seconds,
                ],
            )
            .await
            .map_err(|_| database_refused())?;
        let decision = match row.get::<_, &str>(0) {
            "reserved" => AdmissionDecision::Reserved,
            "completed" => AdmissionDecision::Completed,
            "busy" => AdmissionDecision::Busy,
            "budget_exhausted" | "window_exhausted" => AdmissionDecision::Exhausted,
            "conflict" => AdmissionDecision::Conflict,
            other => {
                log_admission_refusal(sql_refusal_label(SqlLedger::Evidence, other));
                AdmissionDecision::Refused
            }
        };
        tx.commit().await.map_err(|_| database_refused())?;
        Ok(decision)
    }
    pub(crate) async fn transition_admission(
        &self,
        tenant: &str,
        submission: Uuid,
        lease: Uuid,
        next: &str,
    ) -> Result<bool, DatabaseError> {
        let mut client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| database_refused())?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let changed = tx
            .query_one(
                "SELECT trace_transition_admission($1,$2,$3,$4)",
                &[&tenant, &submission, &lease, &next],
            )
            .await
            .map_err(|_| database_refused())?
            .get(0);
        tx.commit().await.map_err(|_| database_refused())?;
        Ok(changed)
    }
}

struct EffectiveAllowance {
    limit: i64,
    tier: Option<i32>,
    digest: Option<String>,
    /// The applied evaluation the limit came from, when one applied.
    evaluation_id: Option<Uuid>,
}

/// How a read binds to the account's input frontier.
#[derive(Clone, Copy)]
enum FrontierRead {
    /// Reservation: holds the frontier `FOR SHARE` until the debit commits,
    /// so no evaluation can be invalidated underneath it.
    Locked,
    /// Status: advisory, so it takes no lock and never makes the evaluator
    /// or a fact or gate trigger wait.
    Advisory,
}

/// Reads only the configured policy's applied evaluations, on the transaction
/// clock. The budget's stored limit stays at the base: growth never erases spend.
async fn account_effective_allowance(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
    account: Uuid,
    policy: &BoundedPolicy,
    read: FrontierRead,
) -> Result<EffectiveAllowance, DatabaseError> {
    let mut allowance = EffectiveAllowance {
        limit: policy.bounded_allowance(),
        tier: None,
        digest: None,
        evaluation_id: None,
    };
    let Some(growth) = policy.external_growth() else {
        return Ok(allowance);
    };
    allowance.tier = Some(0);
    let generation: Option<i64> = tx
        .query_one(
            match read {
                FrontierRead::Locked => "SELECT trace_account_trust_lock_input_generation($1,$2)",
                FrontierRead::Advisory => "SELECT trace_account_trust_input_generation($1,$2)",
            },
            &[&tenant, &account],
        )
        .await
        .map_err(|_| database_refused())?
        .get(0);
    let Some(generation) = generation else {
        return Ok(allowance);
    };
    let row = tx
        .query_opt(
            "SELECT effective_allowance,tier,facts_digest,
                extract(epoch FROM (transaction_timestamp()-as_of)) <= $4::bigint::numeric,
                evaluation_id
         FROM trace_account_trust_evaluations
         WHERE tenant_id=$1 AND account_id=$2 AND growth_policy_version=$3 AND mode='applied'
           AND as_of <= transaction_timestamp() AND isfinite(as_of) AND input_generation=$5
         ORDER BY as_of DESC,recorded_at DESC,evaluation_id DESC LIMIT 1",
            &[
                &tenant,
                &account,
                &growth.policy_version,
                &growth.evaluation_max_age_seconds,
                &generation,
            ],
        )
        .await
        .map_err(|_| database_refused())?;
    if let Some(row) = row.filter(|row| row.get::<_, bool>(3)) {
        allowance.limit = row
            .get::<_, i64>(0)
            .clamp(policy.bounded_allowance(), growth.allowance_ceiling);
        allowance.tier = Some(row.get(1));
        allowance.digest = Some(row.get(2));
        allowance.evaluation_id = Some(row.get(4));
    }
    Ok(allowance)
}

/// The account's own growth view (spec rule 7), inside the status
/// transaction. Reads only this account's evaluation rows, under the
/// caller's tenant RLS; nothing about another account.
async fn account_growth_status(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
    account: Uuid,
    policy: &BoundedPolicy,
    allowance: &EffectiveAllowance,
    period_spend: i64,
    raw_period_id: &str,
) -> Result<AccountGrowthStatus, DatabaseError> {
    let period_resets_at = match policy.period() {
        PolicyPeriod::Lifetime => None,
        PolicyPeriod::Fixed { seconds } => raw_period_id
            .rsplit(':')
            .next()
            .and_then(|bucket| bucket.parse::<i64>().ok())
            .and_then(|bucket| bucket.checked_add(1)?.checked_mul(*seconds))
            .and_then(|at| DateTime::<Utc>::from_timestamp(at, 0)),
    };
    let evaluation = match allowance.evaluation_id {
        None => None,
        Some(evaluation_id) => {
            let row = tx
                .query_one(
                    "SELECT e.as_of, COALESCE(e.units,0), COALESCE(e.active_weeks,0),
                            e.age_seconds, COALESCE(e.attested_units,0),
                            (SELECT max(c.as_of) FROM trace_account_trust_evaluations c
                              WHERE c.tenant_id=e.tenant_id AND c.account_id=e.account_id
                                AND c.growth_policy_version=e.growth_policy_version
                                AND c.mode='applied' AND c.as_of <= e.as_of
                                AND c.tier IS DISTINCT FROM COALESCE(c.previous_tier,0))
                       FROM trace_account_trust_evaluations e
                      WHERE e.tenant_id=$1 AND e.account_id=$2 AND e.evaluation_id=$3",
                    &[&tenant, &account, &evaluation_id],
                )
                .await
                .map_err(|_| database_refused())?;
            Some(AppliedEvaluationProgress {
                as_of: row.get(0),
                units: row.get(1),
                active_weeks: row.get(2),
                age_seconds: row.get(3),
                attested_units: row.get(4),
                tier_changed_at: row.get(5),
            })
        }
    };
    Ok(AccountGrowthStatus {
        tier: allowance.tier.unwrap_or(0),
        effective_allowance: allowance.limit,
        period_spend,
        period_resets_at,
        evaluation,
    })
}

#[cfg(test)]
mod refusal_label_tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Every `<helper>("label")` literal in this file's production code.
    /// Scanning the source is what ties the list to the call sites: a site
    /// added with a copied label fails here.
    fn source_labels() -> Vec<String> {
        let source = include_str!("admission_ledger.rs");
        let production = source
            .split(concat!("mod refusal_label", "_tests"))
            .next()
            .unwrap();
        let mut found = Vec::new();
        for needle in [
            concat!("Ok(refused_at", "(\""),
            concat!("database_refused_at", "(\""),
            concat!("logged_at", "(\""),
        ] {
            let mut rest = production;
            while let Some(at) = rest.find(needle) {
                let tail = &rest[at + needle.len()..];
                let end = tail.find('"').unwrap();
                found.push(tail[..end].to_string());
                rest = &tail[end..];
            }
        }
        found
    }

    #[test]
    fn every_ledger_refusal_has_its_own_listed_label() {
        let listed: BTreeSet<&str> = LEDGER_REFUSAL_LABELS.iter().copied().collect();
        assert_eq!(listed.len(), LEDGER_REFUSAL_LABELS.len(), "duplicate label");
        let mut seen = BTreeSet::new();
        for label in source_labels() {
            assert!(listed.contains(label.as_str()), "{label} is not listed");
            assert!(seen.insert(label.clone()), "{label} names two sites");
        }
        let mut sql = BTreeSet::new();
        for ledger in [SqlLedger::Evidence, SqlLedger::LegacyResume] {
            for result in ["refused", "configuration_changed", "evidence_refused", "x"] {
                let label = sql_refusal_label(ledger, result);
                assert!(sql.insert(label), "{label} names two results");
                assert!(listed.contains(label), "{label} is not listed");
                assert!(!seen.contains(label), "{label} is also a call site");
            }
        }
        assert_eq!(
            seen.len() + sql.len(),
            listed.len(),
            "a listed label is unused"
        );
    }

    #[test]
    fn an_unrecognised_sql_result_is_never_echoed() {
        assert_eq!(
            sql_refusal_label(SqlLedger::Evidence, "tenant-secret-value"),
            "evidence_ledger_sql:unrecognised_result"
        );
    }

    #[test]
    fn a_labelled_refusal_is_still_the_same_refusal() {
        assert_eq!(
            refused_at("device_not_live").decision,
            AdmissionDecision::Refused
        );
        assert!(matches!(
            database_refused_at("db_error:account_pool"),
            DatabaseError::Pool(ref label) if label == "admission_database_unavailable"
        ));
    }
}
