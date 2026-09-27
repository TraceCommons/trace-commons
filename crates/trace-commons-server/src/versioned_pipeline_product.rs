// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Product views over authoritative versioned-pipeline records.
//!
//! This module does not persist contributor status projections. Each status
//! response is derived from the run, immutable outcomes, credit ledger,
//! settlement batch, and payout state at read time: there is no mutable
//! status table to fall out of sync with them.
//!
//! Withdrawal (`withdraw_submission`) is not read through this module --
//! `PgPipelineStore` owns it. Export-snapshot lifecycle operations (create,
//! complete, invalidate, load one snapshot's items) are not ported here
//! either -- this module only reads the aggregate counts those snapshots
//! contribute to `lifecycle_summary` and `operational_summary`.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use deadpool_postgres::Transaction;
use serde::{Deserialize, Serialize};
use tokio_postgres::Row;
use trace_commons_gate_api::pipeline::{AtomicUnits, Phase};
use uuid::Uuid;

use crate::db::postgres::PgBackend;
use crate::error::DatabaseError;
use crate::versioned_pipeline::{PipelineRunState, phase_from_db, sha256_prefixed};

pub const PIPELINE_STATUS_BATCH_MAX: usize = 500;

/// Amounts cross the API as decimal strings, because a JavaScript client
/// (the Tauri app) cannot hold an integer above `2^53` exactly. `AtomicUnits`
/// already serializes this way on its own; these two
/// helpers give the same wire shape to the plain-`u64` amount fields in this
/// module's product record types (`score_microcredits`, `credit_microcredits`,
/// and `PipelineContributorCredit`'s totals) via `#[serde(with = "...")]`.
mod decimal_amount {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        String::deserialize(deserializer)?
            .parse::<u64>()
            .map_err(serde::de::Error::custom)
    }

    /// The `Option<u64>` counterpart: `None` stays JSON `null`; `Some` is
    /// the same decimal string as `decimal_amount`.
    pub mod option {
        use serde::{Deserialize, Deserializer, Serialize, Serializer};

        pub fn serialize<S: Serializer>(
            value: &Option<u64>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            value.map(|value| value.to_string()).serialize(serializer)
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<u64>, D::Error> {
            Option::<String>::deserialize(deserializer)?
                .map(|raw| raw.parse::<u64>().map_err(serde::de::Error::custom))
                .transpose()
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PipelineProcessingStatus {
    Pending,
    Retry,
    /// Review quarantined the run and it waits, parked, for a human
    /// assessment (`PipelineRunState::AwaitingReview`). It is neither a
    /// retry nor a failure.
    AwaitingReview,
    Blocked,
    Complete,
    Rejected,
    Withdrawn,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PipelineCreditStatus {
    Unscored,
    Zero,
    Pending,
    Held,
    Finalized,
    Failed,
    /// Ruling S11: a compatibility bundle's `NoveltyUtility` Trace Credit
    /// leg. It completes like any other leg, but the ledger event type it
    /// wrote is one `main` never batches or pays
    /// (`trace_credit_event_type_is_settlement_eligible` excludes it), so it
    /// would otherwise sit at `Pending` forever.
    NotSettlementEligible,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineInstrumentStatus {
    pub instrument_id: String,
    pub atomic_units: AtomicUnits,
    pub operation_state: String,
    pub internal_settlement_state: String,
    pub credit_event_id: Option<Uuid>,
    pub settlement_batch_id: Option<Uuid>,
    pub payout_rail: String,
    pub payout_state: String,
    pub reason_label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineContributorStatus {
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub run_id: Uuid,
    pub bundle_id: String,
    pub processing: PipelineProcessingStatus,
    pub current_phase: Option<Phase>,
    pub responsible_phase: Option<Phase>,
    pub reason_label: Option<String>,
    pub credit: PipelineCreditStatus,
    #[serde(with = "decimal_amount::option")]
    pub score_microcredits: Option<u64>,
    pub score_outcome_id: Option<Uuid>,
    /// Compatibility projection for clients that only understand Trace Credit.
    pub settlement_batch_id: Option<Uuid>,
    /// Compatibility projection for clients that only understand one payout.
    pub payout: Option<String>,
    pub instruments: Vec<PipelineInstrumentStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PipelineScoreAttestationEntry {
    pub submission_id: Uuid,
    pub run_id: Uuid,
    pub score_outcome_id: Uuid,
    pub bundle_id: String,
    pub outcome_schema_id: String,
    pub outcome_schema_version: u32,
    #[serde(with = "decimal_amount")]
    pub credit_microcredits: u64,
    pub decision: serde_json::Value,
}

// Every `u64` field below is checked against the decimal-string amount rule.
// None of them is an amount -- `PipelineLifecycleSummary`,
// `PipelineWorkSummary`, and `PipelineOperationalSummary`'s fields are all
// counts or durations (of invalidations, snapshots, errors, commands, credit
// events, outbox rows -- never a credit or token amount), and
// `PipelinePhaseTrace`'s are hashes and an outcome version. None gets
// `decimal_amount`. `PipelineForensicTrace`'s only amount-bearing field is
// `instruments: Vec<PipelineInstrumentStatus>`, whose own `atomic_units` is
// already `AtomicUnits` (decimal-string on its own), so it needs no
// additional attribute either.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineLifecycleSummary {
    pub pending_index_invalidations: u64,
    pub terminal_index_invalidation_failures: u64,
    pub active_export_snapshots: u64,
    pub invalidated_export_snapshots: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineWorkSummary {
    pub phase: String,
    pub state: String,
    /// The run's `last_error_label` for this bucket, matching
    /// `^[a-z0-9_]{1,64}$` -- a retry or a suspension always carries a safe
    /// reason label here, so the bucket stays label-only, never a raw
    /// message.
    pub reason_label: Option<String>,
    pub count: u64,
    pub oldest_age_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineOperationalSummary {
    pub generated_at: DateTime<Utc>,
    pub work: Vec<PipelineWorkSummary>,
    pub retryable_error_count: u64,
    pub terminal_error_count: u64,
    pub pending_index_command_count: u64,
    pub failed_index_command_count: u64,
    pub held_credit_count: u64,
    pub delayed_credit_count: u64,
    pub near_outbox_by_state: BTreeMap<String, u64>,
    pub pending_invalidation_count: u64,
    pub failed_invalidation_count: u64,
    pub incomplete_export_count: u64,
    pub tenant_isolation_control_passed: bool,
    pub audit_immutability_control_passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelinePhaseTrace {
    pub phase: String,
    pub outcome_id: Uuid,
    pub outcome_schema_id: String,
    pub outcome_schema_version: u32,
    pub decision_hash: String,
    pub evidence_hash: String,
    pub evaluation_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineForensicTrace {
    pub run_id: Uuid,
    pub submission_id: Uuid,
    pub bundle_id: String,
    pub phases: Vec<PipelinePhaseTrace>,
    pub index_command_hash: Option<String>,
    pub index_write_state: String,
    pub score_outcome_id: Option<Uuid>,
    pub credit_event_id: Option<Uuid>,
    pub settlement_batch_id: Option<Uuid>,
    pub payout_state: String,
    pub instruments: Vec<PipelineInstrumentStatus>,
    pub index_invalidation_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineContributorCredit {
    #[serde(with = "decimal_amount")]
    pub scored_microcredits: u64,
    #[serde(with = "decimal_amount")]
    pub finalized_microcredits: u64,
    #[serde(with = "decimal_amount")]
    pub pending_microcredits: u64,
    #[serde(with = "decimal_amount")]
    pub held_microcredits: u64,
    pub submission_count: usize,
}

/// Shared by `contributor_statuses_for_principals` and `forensic_trace`
/// (Ruling T5-2, following S11): a `pipeline_run_settlements` row's internal
/// lifecycle state, given its own row aliased `settlement`, its optional
/// carrying batch aliased `batch`, and its optional ledger event aliased
/// `ledger`. A `trace_credit` row whose ledger event is `NoveltyUtility`
/// (the stored string `TraceCreditEventType::NoveltyUtility`'s own serde
/// produces, `trace_corpus_storage.rs`, `rename_all = "snake_case"`; the
/// same string `versioned_pipeline.rs`'s `settle_internal_credit` writes,
/// Ruling S10) reports `not_settlement_eligible` rather than `pending`
/// forever, since `main` never batches or pays that event type. Every query
/// that embeds this must alias its rows exactly as this text assumes, and
/// must also embed `TRACE_CREDIT_LEDGER_JOIN`.
const INTERNAL_SETTLEMENT_STATE_CASE: &str = "
                                    CASE
                                        WHEN settlement.instrument_id <> 'trace_credit'
                                            THEN 'not_applicable'
                                        WHEN ledger.event_type = 'novelty_utility'
                                            THEN 'not_settlement_eligible'
                                        WHEN batch.status IS NOT NULL THEN batch.status
                                        WHEN settlement.credit_event_id IS NOT NULL THEN 'pending'
                                        ELSE settlement.operation_state
                                    END";

/// The join `INTERNAL_SETTLEMENT_STATE_CASE` depends on for its
/// `ledger.event_type` read. Must appear in a query that already joins
/// `pipeline_run_settlements` as `settlement`.
const TRACE_CREDIT_LEDGER_JOIN: &str = "
                          LEFT JOIN trace_credit_ledger ledger
                            ON ledger.tenant_id = settlement.tenant_id
                           AND ledger.credit_event_id = settlement.credit_event_id
                           AND ledger.instrument_id = settlement.instrument_id";

#[derive(Clone)]
pub struct PipelineProductStore {
    backend: Arc<PgBackend>,
}

impl PipelineProductStore {
    pub fn new(backend: Arc<PgBackend>) -> Self {
        Self { backend }
    }

    async fn tenant_transaction<'a>(
        client: &'a mut deadpool_postgres::Client,
        tenant_id: &str,
    ) -> Result<Transaction<'a>, DatabaseError> {
        let tx = client.transaction().await?;
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant_id],
        )
        .await?;
        Ok(tx)
    }

    pub async fn contributor_statuses(
        &self,
        tenant_id: &str,
        principal_ref: &str,
        submission_ids: &[Uuid],
    ) -> Result<Vec<PipelineContributorStatus>, DatabaseError> {
        self.contributor_statuses_for_principals(
            tenant_id,
            &[principal_ref.to_string()],
            submission_ids,
        )
        .await
    }

    pub async fn contributor_statuses_for_principals(
        &self,
        tenant_id: &str,
        principal_refs: &[String],
        submission_ids: &[Uuid],
    ) -> Result<Vec<PipelineContributorStatus>, DatabaseError> {
        if submission_ids.len() > PIPELINE_STATUS_BATCH_MAX {
            return Err(DatabaseError::Constraint(
                "submission status batch limit exceeded".to_string(),
            ));
        }
        if submission_ids.is_empty() {
            return Ok(Vec::new());
        }
        if principal_refs.is_empty() {
            return Ok(Vec::new());
        }
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let sql = format!(
            "SELECT r.*, s.status AS submission_status,
                        score.outcome_id AS score_outcome_id,
                        score.decision AS score_decision,
                        score.outcome_schema_id AS score_schema_id,
                        score.outcome_schema_version AS score_schema_version,
                        latest.phase AS latest_outcome_phase,
                        latest.decision AS latest_outcome_decision,
                        COALESCE(settlements.items, '[]'::jsonb) AS instrument_statuses
                   FROM unnest($3::uuid[]) WITH ORDINALITY requested(submission_id, ordinal)
                   JOIN trace_submissions s
                     ON s.tenant_id = $1
                    AND s.submission_id = requested.submission_id
                    AND s.auth_principal_ref = ANY($2)
                   JOIN LATERAL (
                        SELECT pr.*
                          FROM pipeline_runs pr
                         WHERE pr.tenant_id = s.tenant_id
                           AND pr.submission_id = s.submission_id
                         ORDER BY pr.created_at DESC
                         LIMIT 1
                   ) r ON TRUE
                   LEFT JOIN phase_outcomes score
                     ON score.tenant_id = r.tenant_id
                    AND score.run_id = r.run_id
                    AND score.phase = 'score'
                   LEFT JOIN LATERAL (
                        SELECT po.phase, po.decision
                          FROM phase_outcomes po
                         WHERE po.tenant_id = r.tenant_id
                           AND po.run_id = r.run_id
                         ORDER BY CASE po.phase
                            WHEN 'admission' THEN 1
                            WHEN 'review' THEN 2
                            WHEN 'score' THEN 3
                            WHEN 'settle' THEN 4
                         END DESC
                         LIMIT 1
                   ) latest ON TRUE
                   LEFT JOIN LATERAL (
                        SELECT jsonb_agg(
                            jsonb_build_object(
                                'instrument_id', settlement.instrument_id,
                                'atomic_units', settlement.atomic_units::text,
                                'operation_state', settlement.operation_state,
                                'internal_settlement_state', {INTERNAL_SETTLEMENT_STATE_CASE},
                                'credit_event_id', settlement.credit_event_id,
                                'settlement_batch_id', settlement.settlement_batch_id,
                                'payout_rail', settlement.payout_rail,
                                'payout_state', settlement.payout_state,
                                'reason_label', settlement.last_error_label
                            )
                            ORDER BY settlement.instrument_id
                        ) AS items
                          FROM pipeline_run_settlements settlement
                          LEFT JOIN trace_credit_settlement_batches batch
                            ON batch.tenant_id = settlement.tenant_id
                           AND batch.settlement_batch_id = settlement.settlement_batch_id
                           AND batch.instrument_id = settlement.instrument_id
                          {TRACE_CREDIT_LEDGER_JOIN}
                         WHERE settlement.tenant_id = r.tenant_id
                           AND settlement.run_id = r.run_id
                   ) settlements ON TRUE
                  ORDER BY requested.ordinal"
        );
        let rows = tx
            .query(&sql, &[&tenant_id, &principal_refs, &submission_ids])
            .await?;
        tx.commit().await?;
        rows.iter().map(status_from_row).collect()
    }

    pub async fn own_contributor_statuses(
        &self,
        tenant_id: &str,
        principal_ref: &str,
    ) -> Result<Vec<PipelineContributorStatus>, DatabaseError> {
        self.own_contributor_statuses_page(
            tenant_id,
            principal_ref,
            None,
            PIPELINE_STATUS_BATCH_MAX,
        )
        .await
    }

    pub async fn own_contributor_statuses_page(
        &self,
        tenant_id: &str,
        principal_ref: &str,
        after: Option<(DateTime<Utc>, Uuid)>,
        limit: usize,
    ) -> Result<Vec<PipelineContributorStatus>, DatabaseError> {
        if limit == 0 || limit > PIPELINE_STATUS_BATCH_MAX {
            return Err(DatabaseError::Constraint(
                "submission status page limit is invalid".to_string(),
            ));
        }
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let (after_received_at, after_submission_id) = after
            .map(|(received_at, submission_id)| (Some(received_at), Some(submission_id)))
            .unwrap_or((None, None));
        let limit = i64::try_from(limit).map_err(|_| {
            DatabaseError::Constraint("submission status page limit is invalid".to_string())
        })?;
        let rows = tx
            .query(
                "SELECT submission_id
                   FROM trace_submissions
                  WHERE tenant_id = $1 AND auth_principal_ref = $2
                    AND (
                        $3::timestamptz IS NULL
                        OR (received_at, submission_id) < ($3, $4)
                    )
                    AND EXISTS (
                        SELECT 1 FROM pipeline_runs r
                         WHERE r.tenant_id = trace_submissions.tenant_id
                           AND r.submission_id = trace_submissions.submission_id
                    )
                  ORDER BY received_at DESC, submission_id DESC
                  LIMIT $5",
                &[
                    &tenant_id,
                    &principal_ref,
                    &after_received_at,
                    &after_submission_id,
                    &limit,
                ],
            )
            .await?;
        tx.commit().await?;
        let submission_ids = rows
            .iter()
            .map(|row| row.get::<_, Uuid>("submission_id"))
            .collect::<Vec<_>>();
        self.contributor_statuses(tenant_id, principal_ref, &submission_ids)
            .await
    }

    pub async fn contributor_credit(
        &self,
        tenant_id: &str,
        principal_ref: &str,
    ) -> Result<PipelineContributorCredit, DatabaseError> {
        let statuses = self
            .own_contributor_statuses(tenant_id, principal_ref)
            .await?;
        let scored_microcredits = sum_trace_credit_atomic_units(&statuses, |_| true)?;
        let finalized_microcredits = sum_trace_credit_atomic_units(&statuses, |instrument| {
            instrument.internal_settlement_state == "finalized"
        })?;
        let pending_microcredits = sum_trace_credit_atomic_units(&statuses, |instrument| {
            matches!(
                instrument.internal_settlement_state.as_str(),
                "pending" | "approved"
            )
        })?;
        let held_microcredits = sum_trace_credit_atomic_units(&statuses, |instrument| {
            instrument.operation_state == "held"
        })?;
        Ok(PipelineContributorCredit {
            scored_microcredits,
            finalized_microcredits,
            pending_microcredits,
            held_microcredits,
            submission_count: statuses.len(),
        })
    }

    pub async fn score_attestation_entries(
        &self,
        tenant_id: &str,
        principal_ref: &str,
        submission_ids: &[Uuid],
    ) -> Result<Vec<PipelineScoreAttestationEntry>, DatabaseError> {
        if submission_ids.len() > PIPELINE_STATUS_BATCH_MAX {
            return Err(DatabaseError::Constraint(
                "score attestation batch limit exceeded".to_string(),
            ));
        }
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT r.run_id, r.submission_id, r.bundle_id,
                        score.outcome_id, score.outcome_schema_id,
                        score.outcome_schema_version, score.decision
                   FROM unnest($3::uuid[]) WITH ORDINALITY requested(submission_id, ordinal)
                   JOIN trace_submissions s
                     ON s.tenant_id = $1
                    AND s.submission_id = requested.submission_id
                    AND s.auth_principal_ref = $2
                   JOIN LATERAL (
                        SELECT pr.run_id, pr.submission_id, pr.bundle_id, pr.created_at
                          FROM pipeline_runs pr
                         WHERE pr.tenant_id = s.tenant_id
                           AND pr.submission_id = s.submission_id
                         ORDER BY pr.created_at DESC
                         LIMIT 1
                   ) r ON TRUE
                   JOIN phase_outcomes score
                     ON score.tenant_id = $1
                    AND score.run_id = r.run_id
                    AND score.phase = 'score'
                  ORDER BY requested.ordinal",
                &[&tenant_id, &principal_ref, &submission_ids],
            )
            .await?;
        tx.commit().await?;
        rows.iter().map(attestation_from_row).collect()
    }

    pub async fn own_score_attestation_entries(
        &self,
        tenant_id: &str,
        principal_ref: &str,
    ) -> Result<Vec<PipelineScoreAttestationEntry>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT r.run_id, r.submission_id, r.bundle_id,
                        score.outcome_id, score.outcome_schema_id,
                        score.outcome_schema_version, score.decision
                   FROM trace_submissions s
                   JOIN LATERAL (
                        SELECT pr.run_id, pr.submission_id, pr.bundle_id, pr.created_at
                          FROM pipeline_runs pr
                         WHERE pr.tenant_id = s.tenant_id
                           AND pr.submission_id = s.submission_id
                         ORDER BY pr.created_at DESC
                         LIMIT 1
                   ) r ON TRUE
                   JOIN phase_outcomes score
                     ON score.tenant_id = s.tenant_id
                    AND score.run_id = r.run_id
                    AND score.phase = 'score'
                  WHERE s.tenant_id = $1 AND s.auth_principal_ref = $2
                  ORDER BY r.created_at DESC, r.run_id DESC
                  LIMIT $3",
                &[
                    &tenant_id,
                    &principal_ref,
                    &(PIPELINE_STATUS_BATCH_MAX as i64),
                ],
            )
            .await?;
        tx.commit().await?;
        rows.iter().map(attestation_from_row).collect()
    }

    pub async fn lifecycle_summary(
        &self,
        tenant_id: &str,
    ) -> Result<PipelineLifecycleSummary, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_one(
                "SELECT
                    (SELECT COUNT(*) FROM pipeline_index_invalidations
                      WHERE tenant_id = $1 AND state = 'pending') AS pending_index,
                    (SELECT COUNT(*) FROM pipeline_index_invalidations
                      WHERE tenant_id = $1 AND state = 'failed') AS failed_index,
                    (SELECT COUNT(*) FROM pipeline_export_snapshots
                      WHERE tenant_id = $1 AND state IN ('ready','complete')) AS active_exports,
                    (SELECT COUNT(*) FROM pipeline_export_snapshots
                      WHERE tenant_id = $1 AND state = 'invalidated') AS invalidated_exports",
                &[&tenant_id],
            )
            .await?;
        tx.commit().await?;
        Ok(PipelineLifecycleSummary {
            pending_index_invalidations: count_from_row(&row, "pending_index")?,
            terminal_index_invalidation_failures: count_from_row(&row, "failed_index")?,
            active_export_snapshots: count_from_row(&row, "active_exports")?,
            invalidated_export_snapshots: count_from_row(&row, "invalidated_exports")?,
        })
    }

    /// The tenant-isolation and audit-immutability booleans below are a live
    /// health signal, not a cache: every versioned-pipeline table this
    /// codebase defines must carry `FORCE ROW LEVEL SECURITY`, and
    /// `phase_outcomes` must still carry both of its immutability triggers.
    /// The relation list below names each such table by hand -- it does not
    /// discover them from the catalog -- so a future migration that adds one
    /// must extend this list too, or the check silently stops covering it.
    pub async fn operational_summary(
        &self,
        tenant_id: &str,
    ) -> Result<PipelineOperationalSummary, DatabaseError> {
        let generated_at = Utc::now();
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let work_rows = tx
            .query(
                "SELECT next_phase, state, last_error_label, COUNT(*) AS item_count,
                        GREATEST(
                            0,
                            EXTRACT(EPOCH FROM (NOW() - MIN(phase_started_at)))::bigint
                        ) AS oldest_age_seconds
                   FROM pipeline_runs
                  WHERE tenant_id = $1
                  GROUP BY next_phase, state, last_error_label
                  ORDER BY next_phase, state, last_error_label",
                &[&tenant_id],
            )
            .await?;
        let summary = tx
            .query_one(
                "SELECT
                    (SELECT COUNT(*) FROM pipeline_runs
                      WHERE tenant_id = $1 AND state = 'retry') AS retryable_errors,
                    (SELECT COUNT(*) FROM pipeline_runs
                      WHERE tenant_id = $1 AND state = 'failed') AS terminal_errors,
                    (SELECT COUNT(*) FROM pipeline_runs
                      WHERE tenant_id = $1 AND index_write_state = 'pending')
                        AS pending_index,
                    (SELECT COUNT(*) FROM pipeline_runs
                      WHERE tenant_id = $1 AND index_write_state = 'failed')
                        AS failed_index,
                    (SELECT COUNT(*) FROM pipeline_run_settlements
                      WHERE tenant_id = $1 AND operation_state = 'held')
                        AS held_credit,
                    (SELECT COUNT(*) FROM pipeline_run_settlements
                      WHERE tenant_id = $1
                        AND operation_state IN ('pending', 'retry', 'leased'))
                        AS delayed_credit,
                    (SELECT COUNT(*) FROM pipeline_index_invalidations
                      WHERE tenant_id = $1 AND state = 'pending')
                        AS pending_invalidation,
                    (SELECT COUNT(*) FROM pipeline_index_invalidations
                      WHERE tenant_id = $1 AND state = 'failed')
                        AS failed_invalidation,
                    (SELECT COUNT(*) FROM pipeline_export_snapshots
                      WHERE tenant_id = $1 AND state = 'ready')
                        AS incomplete_exports,
                    (
                        SELECT COUNT(*) = 0
                          FROM pg_class c
                          JOIN pg_namespace n ON n.oid = c.relnamespace
                         WHERE n.nspname = current_schema()
                           AND c.relname = ANY($2)
                           AND (NOT c.relrowsecurity OR NOT c.relforcerowsecurity)
                    ) AS tenant_isolation_passed,
                    (
                        SELECT COUNT(*) = 2
                          FROM pg_trigger t
                          JOIN pg_class c ON c.oid = t.tgrelid
                         WHERE c.relname = 'phase_outcomes'
                           AND NOT t.tgisinternal
                           AND t.tgname = ANY($3)
                    ) AS audit_immutability_passed",
                &[
                    &tenant_id,
                    &vec![
                        "pipeline_runs",
                        "phase_outcomes",
                        "pipeline_bundle_packages",
                        "pipeline_active_bundles",
                        "pipeline_bundle_policy_status",
                        "pipeline_receipt_artifacts",
                        "pipeline_run_settlements",
                        "pipeline_admission_usage",
                        "pipeline_review_claims",
                        "pipeline_review_assessments",
                        "pipeline_index_invalidations",
                        "pipeline_export_snapshots",
                        "pipeline_export_snapshot_items",
                    ],
                    &vec![
                        "phase_outcomes_reject_update",
                        "phase_outcomes_reject_delete",
                    ],
                ],
            )
            .await?;
        let near_rows = tx
            .query(
                "SELECT status, COUNT(*) AS item_count
                   FROM trace_near_credit_outbox
                  WHERE tenant_id = $1
                  GROUP BY status
                  ORDER BY status",
                &[&tenant_id],
            )
            .await?;
        tx.commit().await?;
        let work = work_rows
            .iter()
            .map(|row| {
                Ok(PipelineWorkSummary {
                    phase: row.get("next_phase"),
                    state: row.get("state"),
                    reason_label: row.get("last_error_label"),
                    count: count_from_row(row, "item_count")?,
                    oldest_age_seconds: u64::try_from(row.get::<_, i64>("oldest_age_seconds"))
                        .map_err(|_| {
                            DatabaseError::Serialization(
                                "pipeline work age is outside the supported range".to_string(),
                            )
                        })?,
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;
        let near_outbox_by_state = near_rows
            .iter()
            .map(|row| {
                Ok((
                    row.get::<_, String>("status"),
                    count_from_row(row, "item_count")?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, DatabaseError>>()?;
        Ok(PipelineOperationalSummary {
            generated_at,
            work,
            retryable_error_count: count_from_row(&summary, "retryable_errors")?,
            terminal_error_count: count_from_row(&summary, "terminal_errors")?,
            pending_index_command_count: count_from_row(&summary, "pending_index")?,
            failed_index_command_count: count_from_row(&summary, "failed_index")?,
            held_credit_count: count_from_row(&summary, "held_credit")?,
            delayed_credit_count: count_from_row(&summary, "delayed_credit")?,
            near_outbox_by_state,
            pending_invalidation_count: count_from_row(&summary, "pending_invalidation")?,
            failed_invalidation_count: count_from_row(&summary, "failed_invalidation")?,
            incomplete_export_count: count_from_row(&summary, "incomplete_exports")?,
            tenant_isolation_control_passed: summary.get("tenant_isolation_passed"),
            audit_immutability_control_passed: summary.get("audit_immutability_passed"),
        })
    }

    pub async fn forensic_trace(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> Result<Option<PipelineForensicTrace>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let Some(run) = tx
            .query_opt(
                "SELECT run_id, submission_id, bundle_id, index_command_hash,
                        index_write_state, index_invalidation_state
                   FROM pipeline_runs
                  WHERE tenant_id = $1 AND run_id = $2",
                &[&tenant_id, &run_id],
            )
            .await?
        else {
            tx.commit().await?;
            return Ok(None);
        };
        let phase_rows = tx
            .query(
                "SELECT phase, outcome_id, outcome_schema_id,
                        outcome_schema_version, decision, evidence, evaluation
                   FROM phase_outcomes
                  WHERE tenant_id = $1 AND run_id = $2
                  ORDER BY CASE phase
                    WHEN 'admission' THEN 1 WHEN 'review' THEN 2
                    WHEN 'score' THEN 3 WHEN 'settle' THEN 4 END",
                &[&tenant_id, &run_id],
            )
            .await?;
        let settlement_sql = format!(
            "SELECT settlement.instrument_id,
                        settlement.atomic_units::TEXT AS atomic_units_text,
                        settlement.operation_state,
                        {INTERNAL_SETTLEMENT_STATE_CASE} AS internal_settlement_state,
                        settlement.credit_event_id, settlement.settlement_batch_id,
                        settlement.payout_rail, settlement.payout_state,
                        settlement.last_error_label
                   FROM pipeline_run_settlements settlement
                   LEFT JOIN trace_credit_settlement_batches batch
                     ON batch.tenant_id = settlement.tenant_id
                    AND batch.settlement_batch_id = settlement.settlement_batch_id
                    AND batch.instrument_id = settlement.instrument_id
                   {TRACE_CREDIT_LEDGER_JOIN}
                  WHERE settlement.tenant_id = $1 AND settlement.run_id = $2
                  ORDER BY settlement.instrument_id"
        );
        let settlement_rows = tx.query(&settlement_sql, &[&tenant_id, &run_id]).await?;
        tx.commit().await?;
        let phases = phase_rows
            .iter()
            .map(|row| {
                let version =
                    u32::try_from(row.get::<_, i32>("outcome_schema_version")).map_err(|_| {
                        DatabaseError::Serialization(
                            "outcome schema version is outside the supported range".to_string(),
                        )
                    })?;
                Ok(PipelinePhaseTrace {
                    phase: row.get("phase"),
                    outcome_id: row.get("outcome_id"),
                    outcome_schema_id: row.get("outcome_schema_id"),
                    outcome_schema_version: version,
                    decision_hash: json_hash(row.get("decision"))?,
                    evidence_hash: json_hash(row.get("evidence"))?,
                    evaluation_hash: json_hash(row.get("evaluation"))?,
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;
        let score_outcome_id = phases
            .iter()
            .find(|phase| phase.phase == "score")
            .map(|phase| phase.outcome_id);
        let instruments = settlement_rows
            .iter()
            .map(instrument_status_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let trace_credit = instruments
            .iter()
            .find(|instrument| instrument.instrument_id == "trace_credit");
        Ok(Some(PipelineForensicTrace {
            run_id: run.get("run_id"),
            submission_id: run.get("submission_id"),
            bundle_id: run.get("bundle_id"),
            phases,
            index_command_hash: run.get("index_command_hash"),
            index_write_state: run.get("index_write_state"),
            score_outcome_id,
            credit_event_id: trace_credit.and_then(|instrument| instrument.credit_event_id),
            settlement_batch_id: trace_credit.and_then(|instrument| instrument.settlement_batch_id),
            payout_state: trace_credit
                .map(|instrument| instrument.payout_state.clone())
                .unwrap_or_else(|| "none".to_string()),
            instruments,
            index_invalidation_state: run.get("index_invalidation_state"),
        }))
    }
}

/// Sums `trace_credit` amounts across `statuses`' instruments matching
/// `predicate`. `PipelineContributorCredit`'s totals stay `u64` (they mirror
/// the Trace Credit ledger's own signed 64-bit column, bounded by
/// `pipeline_run_settlements_trace_credit_bound`), while
/// `PipelineInstrumentStatus.atomic_units` is `AtomicUnits` (a `u128`, #971)
/// -- this helper is the boundary between the two, and fails closed rather
/// than truncating if a total somehow will not fit.
fn sum_trace_credit_atomic_units(
    statuses: &[PipelineContributorStatus],
    predicate: impl Fn(&PipelineInstrumentStatus) -> bool,
) -> Result<u64, DatabaseError> {
    statuses
        .iter()
        .flat_map(|status| status.instruments.iter())
        .filter(|instrument| instrument.instrument_id == "trace_credit")
        .filter(|instrument| predicate(instrument))
        .try_fold(0u64, |total, instrument| {
            let units = u64::try_from(instrument.atomic_units.get()).map_err(|_| {
                DatabaseError::Serialization(
                    "trace credit settlement amount exceeds the ledger's range".to_string(),
                )
            })?;
            total.checked_add(units).ok_or_else(|| {
                DatabaseError::Serialization(
                    "trace credit total overflowed the ledger's range".to_string(),
                )
            })
        })
}

/// Hashes a JSON value for a hash-only operational surface: the forensic
/// trace stores this digest instead of the phase outcome's raw decision,
/// evidence, or evaluation payload.
fn json_hash(value: serde_json::Value) -> Result<String, DatabaseError> {
    serde_json::to_vec(&value)
        .map(|bytes| sha256_prefixed(&bytes))
        .map_err(|_| DatabaseError::Serialization("operational record is malformed".to_string()))
}

fn status_from_row(row: &Row) -> Result<PipelineContributorStatus, DatabaseError> {
    let run_state = PipelineRunState::from_db(row.get("state"))?;
    let current_phase = phase_from_db(row.get("next_phase"))?;
    let latest_outcome_phase = row
        .get::<_, Option<String>>("latest_outcome_phase")
        .map(|phase| phase_from_db(&phase))
        .transpose()?
        .flatten();
    let submission_status: String = row.get("submission_status");
    let reason_label = row
        .get::<_, Option<String>>("last_error_label")
        .or_else(|| {
            (row.get::<_, String>("admission_decision") != "admit")
                .then(|| row.get::<_, Option<String>>("admission_reason"))
                .flatten()
        })
        .or_else(|| {
            row.get::<_, Option<serde_json::Value>>("latest_outcome_decision")
                .as_ref()
                .and_then(|decision| decision.get("Rejected"))
                .and_then(|rejected| rejected.get("reason"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        });
    let processing = if matches!(submission_status.as_str(), "revoked" | "purged") {
        PipelineProcessingStatus::Withdrawn
    } else if submission_status == "rejected" {
        PipelineProcessingStatus::Rejected
    } else {
        match run_state {
            PipelineRunState::Pending | PipelineRunState::Leased => {
                if reason_label.as_deref() == Some("bound_policy_suspended") {
                    PipelineProcessingStatus::Blocked
                } else {
                    PipelineProcessingStatus::Pending
                }
            }
            PipelineRunState::Retry => PipelineProcessingStatus::Retry,
            PipelineRunState::AwaitingReview => PipelineProcessingStatus::AwaitingReview,
            PipelineRunState::Complete => PipelineProcessingStatus::Complete,
            PipelineRunState::Failed => PipelineProcessingStatus::Failed,
        }
    };
    let score_outcome_id = row.get("score_outcome_id");
    let score_microcredits = row
        .get::<_, Option<serde_json::Value>>("score_decision")
        .as_ref()
        .and_then(score_microcredits);
    let instruments =
        serde_json::from_value::<Vec<PipelineInstrumentStatus>>(row.get("instrument_statuses"))
            .map_err(|_| {
                DatabaseError::Serialization("pipeline instrument status is malformed".to_string())
            })?;
    let trace_credit = instruments
        .iter()
        .find(|instrument| instrument.instrument_id == "trace_credit");
    let credit = match (score_outcome_id, score_microcredits) {
        (None, _) => PipelineCreditStatus::Unscored,
        (Some(_), Some(0)) => PipelineCreditStatus::Zero,
        (Some(_), Some(_))
            if trace_credit.is_some_and(|instrument| instrument.operation_state == "held") =>
        {
            PipelineCreditStatus::Held
        }
        (Some(_), Some(_))
            if trace_credit.is_some_and(|instrument| instrument.operation_state == "failed") =>
        {
            PipelineCreditStatus::Failed
        }
        (Some(_), Some(_))
            if trace_credit
                .is_some_and(|instrument| instrument.internal_settlement_state == "finalized") =>
        {
            PipelineCreditStatus::Finalized
        }
        (Some(_), Some(_))
            if trace_credit.is_some_and(|instrument| {
                instrument.internal_settlement_state == "not_settlement_eligible"
            }) =>
        {
            PipelineCreditStatus::NotSettlementEligible
        }
        _ => PipelineCreditStatus::Pending,
    };
    let payout = match trace_credit.map(|instrument| instrument.payout_state.as_str()) {
        None | Some("none") => None,
        Some(value) => Some(value.to_string()),
    };
    let settlement_batch_id = trace_credit.and_then(|instrument| instrument.settlement_batch_id);
    Ok(PipelineContributorStatus {
        submission_id: row.get("submission_id"),
        trace_id: row.get("trace_id"),
        run_id: row.get("run_id"),
        bundle_id: row.get("bundle_id"),
        processing,
        current_phase,
        // The phase "responsible" for the run's current state is the
        // one whose own attempt set `last_error_label` (e.g. Review's
        // `review_assessment_required` on a quarantine wait) -- not
        // unconditionally the latest phase to have committed an outcome,
        // which pairs a Review label with an Admission phase and misleads a
        // contributor into thinking Admission is still holding the run. A
        // Reject keeps its own rule (Admission or Review, from
        // `admission_decision`); with no label set, the latest committed
        // outcome's phase still applies.
        responsible_phase: if processing == PipelineProcessingStatus::Rejected {
            Some(if row.get::<_, String>("admission_decision") == "reject" {
                Phase::Admission
            } else {
                Phase::Review
            })
        } else if row.get::<_, Option<String>>("last_error_label").is_some() {
            current_phase
        } else {
            latest_outcome_phase
        },
        reason_label,
        credit,
        score_microcredits,
        score_outcome_id,
        settlement_batch_id,
        payout,
        instruments,
    })
}

/// Reads one settlement row as an instrument status. The caller selects
/// `atomic_units::TEXT AS atomic_units_text` so this never parses a raw
/// numeric column directly (`AtomicUnits` is a `u128`, and Postgres `NUMERIC`
/// does not fit any integer type `tokio_postgres` decodes natively).
fn instrument_status_from_row(row: &Row) -> Result<PipelineInstrumentStatus, DatabaseError> {
    let atomic_units = row
        .get::<_, String>("atomic_units_text")
        .parse::<AtomicUnits>()
        .map_err(|_| {
            DatabaseError::Serialization("pipeline instrument amount is invalid".to_string())
        })?;
    Ok(PipelineInstrumentStatus {
        instrument_id: row.get("instrument_id"),
        atomic_units,
        operation_state: row.get("operation_state"),
        internal_settlement_state: row.get("internal_settlement_state"),
        credit_event_id: row.get("credit_event_id"),
        settlement_batch_id: row.get("settlement_batch_id"),
        payout_rail: row.get("payout_rail"),
        payout_state: row.get("payout_state"),
        reason_label: row.get("last_error_label"),
    })
}

fn attestation_from_row(row: &Row) -> Result<PipelineScoreAttestationEntry, DatabaseError> {
    let decision = row.get::<_, serde_json::Value>("decision");
    let credit_microcredits = score_microcredits(&decision).ok_or_else(|| {
        DatabaseError::Serialization("score outcome has no microcredit amount".to_string())
    })?;
    let version: i32 = row.get("outcome_schema_version");
    Ok(PipelineScoreAttestationEntry {
        submission_id: row.get("submission_id"),
        run_id: row.get("run_id"),
        score_outcome_id: row.get("outcome_id"),
        bundle_id: row.get("bundle_id"),
        outcome_schema_id: row.get("outcome_schema_id"),
        outcome_schema_version: u32::try_from(version).map_err(|_| {
            DatabaseError::Serialization("score outcome schema version is invalid".to_string())
        })?,
        credit_microcredits,
        decision,
    })
}

/// Reads the committed Score decision's `trace_credit` award. Under the
/// #971 settlement shape an award's `atomic_units` serializes as a decimal
/// string, not a JSON number (`AtomicUnits` holds a `u128`), so this parses
/// it as `AtomicUnits` and converts to `u64`: `trace_credit` settlement rows
/// are bounded at `i64::MAX` by `pipeline_run_settlements_trace_credit_bound`,
/// so a stored award that does not fit `u64` cannot exist.
///
/// Finding I1: `InstrumentAward::new` refuses a zero amount
/// (`ZeroInstrumentAward`), so a scored-but-zero `trace_credit` leg is
/// represented as *no* `trace_credit` entry in `awards`, not a zero-valued
/// one. A valid `awards` array with no `trace_credit` entry therefore reads
/// as `Some(0)`, not `None` -- `None` is reserved for `awards` missing or
/// malformed, or a `trace_credit` entry whose amount does not parse.
fn score_microcredits(decision: &serde_json::Value) -> Option<u64> {
    let awards = decision.get("awards")?.as_array()?;
    let Some(award) = awards.iter().find(|award| {
        award
            .get("instrument_id")
            .and_then(serde_json::Value::as_str)
            == Some("trace_credit")
    }) else {
        return Some(0);
    };
    award
        .get("atomic_units")?
        .as_str()?
        .parse::<AtomicUnits>()
        .ok()
        .and_then(|units| u64::try_from(units.get()).ok())
}

/// Reads a `COUNT(*)` column (always `bigint`/`i64` in Postgres) as the
/// unsigned count it actually represents.
fn count_from_row(row: &Row, column: &str) -> Result<u64, DatabaseError> {
    let count: i64 = row.get(column);
    u64::try_from(count)
        .map_err(|_| DatabaseError::Serialization("negative aggregate count".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_amount_requires_the_versioned_decision_field() {
        assert_eq!(
            score_microcredits(&serde_json::json!({
                "awards": [
                    {"instrument_id": "trace_credit", "atomic_units": "7"}
                ]
            })),
            Some(7)
        );
        assert_eq!(score_microcredits(&serde_json::json!({"credit": 7})), None);
    }

    /// Finding I1: a valid, empty `awards` array, and a valid `awards`
    /// array with only a non-`trace_credit` entry, both mean "scored zero
    /// trace credit" (`Some(0)`) -- `InstrumentAward::new` refuses a zero
    /// amount, so "no `trace_credit` award" is the only way a policy can
    /// express a zero `trace_credit` leg. `None` is reserved for a missing
    /// or malformed `awards` field.
    #[test]
    fn a_missing_trace_credit_award_reads_as_zero_not_unscored() {
        assert_eq!(
            score_microcredits(&serde_json::json!({"awards": []})),
            Some(0)
        );
        assert_eq!(
            score_microcredits(&serde_json::json!({
                "awards": [
                    {"instrument_id": "storage_rebate", "atomic_units": "5"}
                ]
            })),
            Some(0)
        );
    }

    #[test]
    fn amount_fields_serialize_as_decimal_strings() {
        // 2^53 + 1: the smallest integer a JavaScript `Number` cannot hold
        // exactly, so this is the value that would actually expose a
        // regression back to a raw JSON number.
        const UNSAFE_FOR_JS_NUMBER: u64 = 9_007_199_254_740_993;

        let status = PipelineContributorStatus {
            submission_id: Uuid::nil(),
            trace_id: Uuid::nil(),
            run_id: Uuid::nil(),
            bundle_id: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .to_string(),
            processing: PipelineProcessingStatus::Complete,
            current_phase: None,
            responsible_phase: None,
            reason_label: None,
            credit: PipelineCreditStatus::Finalized,
            score_microcredits: Some(UNSAFE_FOR_JS_NUMBER),
            score_outcome_id: Some(Uuid::nil()),
            settlement_batch_id: None,
            payout: None,
            instruments: Vec::new(),
        };
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(
            value["score_microcredits"],
            serde_json::json!(UNSAFE_FOR_JS_NUMBER.to_string())
        );
        let round_tripped: PipelineContributorStatus = serde_json::from_value(value).unwrap();
        assert_eq!(round_tripped, status);

        // `None` stays JSON `null`, not the string `"null"` or a number.
        let unscored = PipelineContributorStatus {
            score_microcredits: None,
            ..status.clone()
        };
        let unscored_value = serde_json::to_value(&unscored).unwrap();
        assert_eq!(
            unscored_value["score_microcredits"],
            serde_json::Value::Null
        );
        let unscored_round_tripped: PipelineContributorStatus =
            serde_json::from_value(unscored_value).unwrap();
        assert_eq!(unscored_round_tripped, unscored);

        let entry = PipelineScoreAttestationEntry {
            submission_id: Uuid::nil(),
            run_id: Uuid::nil(),
            score_outcome_id: Uuid::nil(),
            bundle_id: status.bundle_id.clone(),
            outcome_schema_id: "trace_commons.pipeline_score_outcome.v1".to_string(),
            outcome_schema_version: 1,
            credit_microcredits: UNSAFE_FOR_JS_NUMBER,
            decision: serde_json::json!({"awards": []}),
        };
        let entry_value = serde_json::to_value(&entry).unwrap();
        assert_eq!(
            entry_value["credit_microcredits"],
            serde_json::json!(UNSAFE_FOR_JS_NUMBER.to_string())
        );
        let entry_round_tripped: PipelineScoreAttestationEntry =
            serde_json::from_value(entry_value).unwrap();
        assert_eq!(entry_round_tripped, entry);

        let credit = PipelineContributorCredit {
            scored_microcredits: UNSAFE_FOR_JS_NUMBER,
            finalized_microcredits: UNSAFE_FOR_JS_NUMBER,
            pending_microcredits: 0,
            held_microcredits: 0,
            submission_count: 1,
        };
        let credit_value = serde_json::to_value(&credit).unwrap();
        assert_eq!(
            credit_value["scored_microcredits"],
            serde_json::json!(UNSAFE_FOR_JS_NUMBER.to_string())
        );
        assert_eq!(credit_value["pending_microcredits"], serde_json::json!("0"));
        assert_eq!(
            credit_value["submission_count"],
            serde_json::json!(1),
            "submission_count is a count, not an amount, and stays a JSON number"
        );
        let credit_round_tripped: PipelineContributorCredit =
            serde_json::from_value(credit_value).unwrap();
        assert_eq!(credit_round_tripped, credit);
    }
}
