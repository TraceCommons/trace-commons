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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountAdmissionStatus {
    pub authority: &'static str,
    pub trust_version: i64,
    pub policy_version: String,
    pub ready: bool,
    pub retry_after_seconds: Option<i64>,
}

/// Decision and advisory metadata observed in the same transaction as the debit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountAdmissionResult {
    pub decision: AdmissionDecision,
    pub authority: Option<&'static str>,
    pub retry_after_seconds: Option<i64>,
}
impl From<AdmissionDecision> for AccountAdmissionResult {
    fn from(decision: AdmissionDecision) -> Self {
        Self {
            decision,
            authority: None,
            retry_after_seconds: None,
        }
    }
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
            _ => AdmissionDecision::Refused,
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
        let used: i64 = if let Some(budget) = budget {
            if budget.get::<_, i64>(1) != policy.bounded_allowance()
                || budget.get::<_, i64>(2) != policy.processing_cost_bound()
                || budget.get::<_, String>(3) != policy.version()
            {
                return Ok(None);
            }
            budget.get(0)
        } else {
            0
        };
        let used = if matches!(policy.period(), PolicyPeriod::Lifetime) {
            tx.query_one("SELECT COALESCE(sum(cost_used),0)::bigint FROM trace_account_admission_budget WHERE tenant_id=$1 AND account_id=$2 AND period_id LIKE '%:lifetime'", &[&tenant,&account_id]).await.map_err(|_|database_refused())?.get::<_,i64>(0)
        } else {
            used
        };
        let ready = if authority == "invited" {
            true
        } else {
            used.checked_add(policy.processing_cost_bound())
                .is_some_and(|next| next <= policy.bounded_allowance())
        };
        tx.commit().await.map_err(|_| database_refused())?;
        Ok(Some(AccountAdmissionStatus {
            authority,
            trust_version: version,
            policy_version: policy.version().into(),
            ready,
            retry_after_seconds: if ready { None } else { retry_after_seconds },
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
            return Ok(AdmissionDecision::Refused.into());
        }
        let tenant = r.account.tenant_id();
        let account = r.account.account_id();
        let mut client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| database_refused())?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let account_live = tx
            .query_opt(
                "SELECT 1 FROM trace_accounts WHERE tenant_id=$1 AND account_id=$2
             AND closed_at IS NULL FOR UPDATE",
                &[&tenant, &account],
            )
            .await
            .map_err(|_| database_refused())?;
        if account_live.is_none() {
            return Ok(AdmissionDecision::Refused.into());
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
            .map_err(|_| database_refused())?
            .get(0);
        if !device_live {
            return Ok(AdmissionDecision::Refused.into());
        }

        // A verified first use creates bounded trust only with a fully parsed,
        // explicitly activated policy. Invite grants remain independent facts.
        tx.execute(
            "INSERT INTO trace_account_trust(tenant_id,account_id,authority,trust_version)
             VALUES($1,$2,'bounded',1) ON CONFLICT DO NOTHING",
            &[&tenant, &account],
        )
        .await
        .map_err(|_| database_refused())?;
        let trust = tx
            .query_opt(
                "SELECT authority,trust_version FROM trace_account_trust
             WHERE tenant_id=$1 AND account_id=$2 FOR UPDATE",
                &[&tenant, &account],
            )
            .await
            .map_err(|_| database_refused())?;
        let Some(trust) = trust else {
            return Ok(AdmissionDecision::Refused.into());
        };
        let mut authority: String = trust.get(0);
        let mut version: i64 = trust.get(1);
        if version <= 0 || !matches!(authority.as_str(), "bounded" | "invited") {
            return Ok(AdmissionDecision::Refused.into());
        }
        if let Some(expected) = r.expected_trust_version {
            if expected != version {
                return Ok(AdmissionDecision::Refused.into());
            }
        }
        let active_grant: bool = tx
            .query_one(
                "SELECT trace_account_admission_active_grant($1,$2)",
                &[&tenant, &account],
            )
            .await
            .map_err(|_| database_refused())?
            .get(0);
        if authority == "invited" && !active_grant {
            version = version.checked_add(1).ok_or_else(database_refused)?;
            tx.execute("UPDATE trace_account_trust SET authority='bounded',trust_version=$3,updated_at=clock_timestamp() WHERE tenant_id=$1 AND account_id=$2", &[&tenant,&account,&version]).await.map_err(|_|database_refused())?;
            authority = "bounded".into();
        } else if authority == "bounded" && active_grant {
            return Ok(AdmissionDecision::Refused.into());
        }
        let (raw_period_id, retry_after_seconds) =
            account_policy_period(&tx, r.policy.period()).await?;
        let period_id = format!("{}:{raw_period_id}", r.policy.version());
        // Every submission ID binds once to an account and exact body, even
        // when a lease expires or the policy changes.
        let prior = tx.query_opt(
            "SELECT account_id,body_hash,status,lease_expires_at > clock_timestamp() FROM trace_account_admission_submissions
             WHERE tenant_id=$1 AND submission_id=$2 FOR UPDATE",
            &[&tenant, &r.submission_id],
        ).await.map_err(|_| database_refused())?;
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
        let charged = authority != "invited";
        if charged {
            tx.execute("INSERT INTO trace_account_admission_budget(tenant_id,account_id,period_id,policy_version,cost_limit,cost_bound) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING", &[&tenant,&account,&period_id,&r.policy.version(),&r.policy.bounded_allowance(),&r.policy.processing_cost_bound()]).await.map_err(|_|database_refused())?;
            let budget = tx.query_one("SELECT cost_used,cost_limit,cost_bound,policy_version FROM trace_account_admission_budget WHERE tenant_id=$1 AND account_id=$2 AND period_id=$3 FOR UPDATE", &[&tenant,&account,&period_id]).await.map_err(|_|database_refused())?;
            let used: i64 = budget.get(0);
            if budget.get::<_, i64>(1) != r.policy.bounded_allowance()
                || budget.get::<_, i64>(2) != r.policy.processing_cost_bound()
                || budget.get::<_, String>(3) != r.policy.version()
            {
                return Ok(AdmissionDecision::Refused.into());
            }
            let total_used = if matches!(r.policy.period(), PolicyPeriod::Lifetime) {
                tx.query_one("SELECT COALESCE(sum(cost_used),0)::bigint FROM trace_account_admission_budget WHERE tenant_id=$1 AND account_id=$2 AND period_id LIKE '%:lifetime'", &[&tenant,&account]).await.map_err(|_|database_refused())?.get::<_,i64>(0)
            } else {
                used
            };
            let next = total_used
                .checked_add(r.policy.processing_cost_bound())
                .ok_or_else(database_refused)?;
            if next > r.policy.bounded_allowance() {
                return Ok(AccountAdmissionResult {
                    decision: AdmissionDecision::Exhausted,
                    authority: Some("bounded"),
                    retry_after_seconds,
                });
            }
            tx.execute("UPDATE trace_account_admission_budget SET cost_used=$4 WHERE tenant_id=$1 AND account_id=$2 AND period_id=$3", &[&tenant,&account,&period_id,&(used + r.policy.processing_cost_bound())]).await.map_err(|_|database_refused())?;
        }
        let lease_expires: DateTime<Utc> = tx
            .query_one(
                "SELECT clock_timestamp()+make_interval(secs => $1::bigint::double precision)",
                &[&r.lease_seconds],
            )
            .await
            .map_err(|_| database_refused())?
            .get(0);
        if prior.is_some() {
            tx.execute("UPDATE trace_account_admission_submissions SET status='reserved',lease_id=$3,lease_expires_at=$4,last_cost_bound=$5,last_charged=$6,trust_version=$7,policy_version=$8,period_id=$9 WHERE tenant_id=$1 AND submission_id=$2",
                &[&tenant,&r.submission_id,&r.lease_id,&lease_expires,&r.policy.processing_cost_bound(),&charged,&version,&r.policy.version(),&period_id]).await.map_err(|_|database_refused())?;
        } else {
            tx.execute("INSERT INTO trace_account_admission_submissions(tenant_id,submission_id,account_id,body_hash,authority_kind,trust_version,policy_version,period_id,status,lease_id,lease_expires_at,last_cost_bound,last_charged) VALUES($1,$2,$3,$4,'account',$5,$6,$7,'reserved',$8,$9,$10,$11)",
                &[&tenant,&r.submission_id,&account,&r.body_hash,&version,&r.policy.version(),&period_id,&r.lease_id,&lease_expires,&r.policy.processing_cost_bound(),&charged]).await.map_err(|_|database_refused())?;
        }
        tx.commit().await.map_err(|_| database_refused())?;
        Ok(AccountAdmissionResult {
            decision: AdmissionDecision::Reserved,
            authority: Some(if charged { "bounded" } else { "invited" }),
            retry_after_seconds: None,
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
            "released" if status == "reserved" => {
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
 AND has_column_privilege(current_user,'public.trace_accounts','account_id','UPDATE')
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
            _ => AdmissionDecision::Refused,
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
