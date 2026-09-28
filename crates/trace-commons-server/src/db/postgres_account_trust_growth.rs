// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Earned account trust: the batch seams the fact recorder and the shadow
//! evaluator run through. Every read and write here goes through a
//! `SECURITY DEFINER` function owned by a NOLOGIN, NOBYPASSRLS guard role;
//! the login needs only `trace_account_trust_worker` (V85), never a table
//! privilege on the facts. Nothing in this file is read by admission.

use uuid::Uuid;

use super::PgBackend;
use crate::account_trust::{TrustAccount, TrustFactSource};
use crate::account_trust_rule::{Evaluation, EvaluationFact};
use crate::error::DatabaseError;

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
            .query(
                "SELECT source_kind, source_id, submission_id, outcome, evaluator_version,
                        occurred_at, credit_quality_micros, credit_quality_calibration_version,
                        dedup_signal_version, first_in_cluster
                   FROM trace_account_trust_evaluation_inputs($1,$2)",
                &[&tenant, &account.account_id()],
            )
            .await?;
        tx.commit().await?;
        Ok(rows
            .into_iter()
            .map(|row| EvaluationFact {
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
            })
            .collect())
    }

    /// Stores one evaluation through the V86 definer function, which refuses
    /// every mode but `shadow`. Returns whether the tier changed (and so
    /// whether a `account_trust_tier_changed` audit row was written).
    pub async fn record_account_trust_evaluation(
        &self,
        account: &TrustAccount,
        mode: &str,
        evaluation: &Evaluation,
    ) -> Result<bool, DatabaseError> {
        let count = |value: u32| i32::try_from(value).unwrap_or(i32::MAX);
        let tenant = account.tenant_id();
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let changed: bool = tx
            .query_one(
                "SELECT trace_record_account_trust_evaluation($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,
                    $11,$12,$13,$14,$15,$16,$17)",
                &[
                    &tenant,
                    &Uuid::new_v4(),
                    &account.account_id(),
                    &evaluation.growth_policy_version,
                    &mode,
                    &evaluation.as_of,
                    &count(evaluation.tier),
                    &evaluation.effective_allowance,
                    &count(evaluation.units),
                    &count(evaluation.units_capped),
                    &count(evaluation.active_weeks),
                    &count(evaluation.age_weeks),
                    &count(evaluation.netted_withdrawn),
                    &count(evaluation.netted_revoked),
                    &count(evaluation.netted_quarantined),
                    &evaluation.penalty_active,
                    &evaluation.facts_digest,
                ],
            )
            .await?
            .get(0);
        tx.commit().await?;
        Ok(changed)
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
                        netted_quarantined, penalty_active, facts_digest
                   FROM trace_account_trust_evaluations
                  WHERE tenant_id=$1 AND account_id=$2 AND growth_policy_version=$3 AND mode=$4
                  ORDER BY as_of DESC, recorded_at DESC
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
