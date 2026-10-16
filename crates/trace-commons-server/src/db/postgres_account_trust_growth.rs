// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Earned account trust: the batch seams the fact recorder and the
//! evaluator run through. Every read and write here goes through a
//! `SECURITY DEFINER` function owned by a NOLOGIN, NOBYPASSRLS guard role;
//! the login needs only `trace_account_trust_worker` (V85, V126), never a
//! table privilege on the facts. Admission reads only the `applied` rows
//! [`PgBackend::evaluate_and_record_applied_account_trust`] writes, through
//! V114's generation-bound read in `admission_ledger`.

use chrono::{DateTime, Utc};
use tokio_postgres::error::SqlState;
use uuid::Uuid;

use super::PgBackend;
use crate::account_trust::{TrustAccount, TrustFactSource};
use crate::account_trust_growth::APPLIED_MODE;
use crate::account_trust_rule::{Evaluation, EvaluationFact, GrowthPolicy, evaluate};
use crate::error::DatabaseError;

/// Snapshot conflicts an applied evaluation retries from a fresh snapshot.
const APPLIED_ATTEMPTS: usize = 3;

const EVALUATION_INPUTS_SQL: &str =
    "SELECT source_kind, source_id, submission_id, outcome, evaluator_version,
        occurred_at, credit_quality_micros, credit_quality_calibration_version,
        dedup_signal_version, first_in_cluster
   FROM trace_account_trust_evaluation_inputs($1,$2)";

const RECORD_EVALUATION_SQL: &str =
    "SELECT trace_record_account_trust_evaluation_v2($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,
    $11,$12,$13,$14,$15,$16,$17,$18,$19,$20)";

fn evaluation_fact(row: &tokio_postgres::Row) -> EvaluationFact {
    EvaluationFact {
        source_kind: row.get(0),
        source_id: row.get(1),
        submission_id: row.get(2),
        outcome: row.get(3),
        evaluator_version: row.get(4),
        occurred_at: row.get(5),
        credit_quality_micros: row.get(6),
        credit_quality_calibration_version: row.get(7),
        dedup_signal_version: row.get(8),
        first_in_cluster: row.get(9),
    }
}

/// Writes one evaluation through the V126 writer inside `tx`. A `shadow` row
/// passes no generation; an `applied` row must pass the one read in the same
/// snapshot as its inputs.
async fn record_evaluation_in(
    tx: &deadpool_postgres::Transaction<'_>,
    account: &TrustAccount,
    mode: &str,
    evaluation: &Evaluation,
    generation: Option<i64>,
) -> Result<bool, tokio_postgres::Error> {
    let count = |value: u32| i32::try_from(value).unwrap_or(i32::MAX);
    Ok(tx
        .query_one(
            RECORD_EVALUATION_SQL,
            &[
                &account.tenant_id(),
                &Uuid::new_v4(),
                &account.account_id(),
                &evaluation.growth_policy_version,
                &mode,
                &evaluation.as_of,
                &count(evaluation.tier),
                &evaluation.effective_allowance,
                &count(evaluation.units),
                &count(evaluation.units_capped),
                &count(evaluation.attested_units),
                &count(evaluation.active_weeks),
                &count(evaluation.age_weeks),
                &evaluation.age_seconds,
                &count(evaluation.netted_withdrawn),
                &count(evaluation.netted_revoked),
                &count(evaluation.netted_quarantined),
                &evaluation.penalty_active,
                &evaluation.facts_digest,
                &generation,
            ],
        )
        .await?
        .get(0))
}

impl PgBackend {
    /// One page of open accounts in anchored tenants, in `(tenant, account)`
    /// order after `after`. The only cross-tenant read the worker makes, and
    /// it returns nothing but the two keys.
    pub async fn list_account_trust_worker_accounts(
        &self,
        after: Option<&TrustAccount>,
        limit: i64,
    ) -> Result<Vec<TrustAccount>, DatabaseError> {
        let client = self.trace_pool().get().await?;
        let after_tenant = after.map(|a| a.tenant_id().to_string());
        let after_account = after.map(TrustAccount::account_id);
        let rows = client
            .query(
                "SELECT tenant_id, account_id FROM trace_account_trust_worker_accounts($1,$2,$3)",
                &[&after_tenant, &after_account, &limit.clamp(0, 10_000)],
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| TrustAccount::from_worker_enumeration(row.get(0), row.get(1)))
            .collect())
    }

    /// The sources in this account's tenant that name this account and have
    /// no fact yet. Tenant-scoped under forced RLS.
    pub async fn list_account_trust_fact_candidates(
        &self,
        account: &TrustAccount,
        limit: i64,
    ) -> Result<Vec<TrustFactSource>, DatabaseError> {
        let tenant = account.tenant_id();
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let rows = tx
            .query(
                "SELECT source_kind, source_id FROM trace_account_trust_fact_candidates($1,$2,$3)",
                &[&tenant, &account.account_id(), &limit.clamp(0, 10_000)],
            )
            .await?;
        tx.commit().await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let kind: String = row.get(0);
                let source: Uuid = row.get(1);
                TrustFactSource::from_storage(&kind, source)
            })
            .collect())
    }

    /// Every fact of this account, with the gate fields and the
    /// first-in-cluster boolean the rule reads (V86). Tenant-scoped.
    pub async fn account_trust_evaluation_inputs(
        &self,
        account: &TrustAccount,
    ) -> Result<Vec<EvaluationFact>, DatabaseError> {
        let tenant = account.tenant_id();
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let rows = tx
            .query(EVALUATION_INPUTS_SQL, &[&tenant, &account.account_id()])
            .await?;
        tx.commit().await?;
        Ok(rows.iter().map(evaluation_fact).collect())
    }

    /// Stores one `shadow` evaluation through the V126 writer. Returns whether
    /// the tier changed (and so whether a `account_trust_tier_changed` audit
    /// row was written). An `applied` row goes only through
    /// [`Self::evaluate_and_record_applied_account_trust`], which binds it to
    /// the inputs it read; the writer refuses one without that binding.
    pub async fn record_account_trust_evaluation(
        &self,
        account: &TrustAccount,
        mode: &str,
        evaluation: &Evaluation,
    ) -> Result<bool, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, account.tenant_id()).await?;
        let changed = record_evaluation_in(&tx, account, mode, evaluation, None).await?;
        tx.commit().await?;
        Ok(changed)
    }

    /// Evaluates `account` under the admission policy's growth rule and stores
    /// an `applied` row, all in one REPEATABLE READ snapshot: the account's
    /// input generation, its facts, the rule at the database's
    /// `transaction_timestamp()`, and the write. Admission consumes the row
    /// only while the generation still matches, so a fact, gate or identity
    /// change after this snapshot sends the account back to tier 0 until the
    /// next run. A snapshot conflict retries from a fresh snapshot.
    ///
    /// `Ok(None)` when the account has no facts: no row is written, and
    /// admission reads the base allowance.
    pub async fn evaluate_and_record_applied_account_trust(
        &self,
        account: &TrustAccount,
        policy: &GrowthPolicy,
    ) -> Result<Option<(Evaluation, bool)>, DatabaseError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            match self.applied_attempt(account, policy).await {
                Err(DatabaseError::Postgres(error))
                    if attempt < APPLIED_ATTEMPTS
                        && matches!(
                            error.code(),
                            Some(&SqlState::T_R_SERIALIZATION_FAILURE)
                                | Some(&SqlState::T_R_DEADLOCK_DETECTED)
                        ) => {}
                outcome => return outcome,
            }
        }
    }

    async fn applied_attempt(
        &self,
        account: &TrustAccount,
        policy: &GrowthPolicy,
    ) -> Result<Option<(Evaluation, bool)>, DatabaseError> {
        let tenant = account.tenant_id();
        let mut client = self.trace_pool().get().await?;
        let tx = client
            .build_transaction()
            .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
            .start()
            .await?;
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant],
        )
        .await?;
        let row = tx
            .query_one(
                "SELECT trace_account_trust_input_generation($1,$2), transaction_timestamp()",
                &[&tenant, &account.account_id()],
            )
            .await?;
        let generation: Option<i64> = row.get(0);
        let as_of: DateTime<Utc> = row.get(1);
        // NULL: external growth is switched off, or the account has no
        // frontier. Either way nothing applied can be recorded; say so.
        let Some(generation) = generation else {
            return Err(DatabaseError::Constraint(
                "account_trust_applied_growth_unavailable".into(),
            ));
        };
        let facts: Vec<EvaluationFact> = tx
            .query(EVALUATION_INPUTS_SQL, &[&tenant, &account.account_id()])
            .await?
            .iter()
            .map(evaluation_fact)
            .collect();
        if facts.is_empty() {
            tx.commit().await?;
            return Ok(None);
        }
        let evaluation = evaluate(policy, &facts, as_of);
        let changed =
            record_evaluation_in(&tx, account, APPLIED_MODE, &evaluation, Some(generation)).await?;
        tx.commit().await?;
        Ok(Some((evaluation, changed)))
    }

    /// What a `tiered-v1` admission policy needs beyond V114's external
    /// contract: this login records facts, reads inputs and the generation,
    /// and writes evaluations only through the definer functions, and the
    /// V126 writer is owned by the evaluation guard. It holds no direct write
    /// on the facts or the evaluations.
    pub async fn tiered_account_trust_runtime_ready(&self) -> Result<bool, DatabaseError> {
        let client = self.trace_pool().get().await?;
        let ready: Option<bool> = client
            .query_one(
                r#"SELECT
            has_function_privilege(current_user,f.oid,'EXECUTE')
            AND f.prosecdef AND f.proowner=to_regrole('trace_account_trust_evaluation_guard')
            AND has_function_privilege(current_user,to_regprocedure('public.trace_account_trust_worker_accounts(text,uuid,bigint)'),'EXECUTE')
            AND has_function_privilege(current_user,to_regprocedure('public.trace_account_trust_fact_candidates(text,uuid,bigint)'),'EXECUTE')
            AND has_function_privilege(current_user,to_regprocedure('public.trace_record_account_trust_fact(text,uuid,text,uuid)'),'EXECUTE')
            AND has_function_privilege(current_user,to_regprocedure('public.trace_account_trust_evaluation_inputs(text,uuid)'),'EXECUTE')
            AND has_function_privilege(current_user,to_regprocedure('public.trace_account_trust_input_generation(text,uuid)'),'EXECUTE')
            AND NOT has_table_privilege(current_user,'public.trace_account_trust_evaluations','INSERT,UPDATE,DELETE')
            AND NOT has_table_privilege(current_user,'public.trace_account_trust_facts','INSERT,UPDATE,DELETE')
            FROM (SELECT to_regprocedure('public.trace_record_account_trust_evaluation_v2(text,uuid,uuid,text,text,timestamp with time zone,integer,bigint,integer,integer,integer,integer,integer,bigint,integer,integer,integer,boolean,text,bigint)') AS oid) w
            LEFT JOIN pg_proc f ON f.oid=w.oid"#,
                &[],
            )
            .await?
            .get(0);
        Ok(ready == Some(true))
    }

    /// The newest stored evaluation of this account under one policy version
    /// and mode, or `None`.
    pub async fn latest_account_trust_evaluation(
        &self,
        account: &TrustAccount,
        policy_version: &str,
        mode: &str,
    ) -> Result<Option<Evaluation>, DatabaseError> {
        let count = |value: i32| u32::try_from(value).unwrap_or(0);
        let tenant = account.tenant_id();
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let row = tx
            .query_opt(
                "SELECT growth_policy_version, as_of, tier, effective_allowance, units,
                        units_capped, active_weeks, age_weeks, netted_withdrawn, netted_revoked,
                        netted_quarantined, penalty_active, facts_digest,
                        COALESCE(attested_units, 0),
                        COALESCE(age_seconds, age_weeks::bigint * 604800)
                   FROM trace_account_trust_evaluations
                  WHERE tenant_id=$1 AND account_id=$2 AND growth_policy_version=$3 AND mode=$4
                    AND units IS NOT NULL AND units_capped IS NOT NULL
                    AND active_weeks IS NOT NULL AND age_weeks IS NOT NULL
                    AND netted_withdrawn IS NOT NULL AND netted_revoked IS NOT NULL
                    AND netted_quarantined IS NOT NULL AND penalty_active IS NOT NULL
                  ORDER BY as_of DESC, recorded_at DESC, evaluation_id DESC
                  LIMIT 1",
                &[&tenant, &account.account_id(), &policy_version, &mode],
            )
            .await?;
        tx.commit().await?;
        Ok(row.map(|row| Evaluation {
            growth_policy_version: row.get(0),
            as_of: row.get(1),
            tier: count(row.get(2)),
            effective_allowance: row.get(3),
            units: count(row.get(4)),
            units_capped: count(row.get(5)),
            attested_units: count(row.get(13)),
            age_seconds: row.get(14),
            active_weeks: count(row.get(6)),
            age_weeks: count(row.get(7)),
            netted_withdrawn: count(row.get(8)),
            netted_revoked: count(row.get(9)),
            netted_quarantined: count(row.get(10)),
            penalty_active: row.get(11),
            facts_digest: row.get(12),
        }))
    }
}
