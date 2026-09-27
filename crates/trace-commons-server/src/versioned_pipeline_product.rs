// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Product views over authoritative versioned-pipeline records.
//!
//! This module does not persist contributor status projections. Each status
//! response is derived from the run, immutable outcomes, credit ledger,
//! settlement batch, and payout state at read time (P3-D13: no mutable
//! status table).
//!
//! Withdrawal (`withdraw_submission`) is not read through this module --
//! Task 7 owns it on `PgPipelineStore`. Export snapshots, the forensic
//! trace, and the operational summary are not implemented here yet; their
//! record types and a few private helpers below are already ported ahead of
//! Tasks 12 and 13, which add the reads that use them.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use deadpool_postgres::Transaction;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_postgres::Row;
use trace_commons_gate_api::pipeline::{AtomicUnits, Phase};
use uuid::Uuid;

use crate::db::postgres::PgBackend;
use crate::error::DatabaseError;
use crate::versioned_pipeline::{PipelineRunState, phase_from_db, sha256_prefixed};

pub const PIPELINE_STATUS_BATCH_MAX: usize = 500;
pub const PIPELINE_EXPORT_ITEM_MAX: usize = 500;
pub const PIPELINE_EXPORT_SELECTION_POLICY_ID: &str = "trace_commons.pipeline_export_selection.v1";
pub const PIPELINE_AUTHORIZED_VIEW_SCHEMA_ID: &str = "trace_commons.authorized_trace_view.v1";

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
    pub credit_microcredits: u64,
    pub decision: serde_json::Value,
}

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
    pub count: u64,
    pub oldest_age_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineOperationalSummary {
    pub generated_at: DateTime<Utc>,
    pub work: Vec<PipelineWorkSummary>,
    pub suspended_policy_count: u64,
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
    pub intervention_evidence_hashes: Vec<String>,
    pub index_invalidation_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineContributorCredit {
    pub scored_microcredits: u64,
    pub finalized_microcredits: u64,
    pub pending_microcredits: u64,
    pub held_microcredits: u64,
    pub submission_count: usize,
}

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
        let rows = tx
            .query(
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
                                'internal_settlement_state',
                                    CASE
                                        WHEN settlement.instrument_id <> 'trace_credit'
                                            THEN 'not_applicable'
                                        WHEN batch.status IS NOT NULL THEN batch.status
                                        WHEN settlement.credit_event_id IS NOT NULL THEN 'pending'
                                        ELSE settlement.operation_state
                                    END,
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
                         WHERE settlement.tenant_id = r.tenant_id
                           AND settlement.run_id = r.run_id
                   ) settlements ON TRUE
                  ORDER BY requested.ordinal",
                &[&tenant_id, &principal_refs, &submission_ids],
            )
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

/// Used by Task 12's forensic trace read (not implemented in this module
/// yet), ported ahead of time per plan.
#[allow(dead_code)]
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
        responsible_phase: if processing == PipelineProcessingStatus::Rejected {
            Some(if row.get::<_, String>("admission_decision") == "reject" {
                Phase::Admission
            } else {
                Phase::Review
            })
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

/// Used by Task 12's forensic trace read (not implemented in this module
/// yet), ported ahead of time per plan.
#[allow(dead_code)]
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

/// Used by Task 12's export snapshot read (not implemented in this module
/// yet), ported ahead of time per plan.
#[allow(dead_code)]
fn source_list_hash(rows: &[Row]) -> String {
    let mut hasher = Sha256::new();
    for row in rows {
        hasher.update(row.get::<_, Uuid>("approved_revision_id").as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

/// Reads the committed Score decision's `trace_credit` award. Under the
/// #971 settlement shape an award's `atomic_units` serializes as a decimal
/// string, not a JSON number (`AtomicUnits` holds a `u128`), so this parses
/// it as `AtomicUnits` and converts to `u64`: `trace_credit` settlement rows
/// are bounded at `i64::MAX` by `pipeline_run_settlements_trace_credit_bound`,
/// so a stored award that does not fit `u64` cannot exist.
fn score_microcredits(decision: &serde_json::Value) -> Option<u64> {
    decision
        .get("awards")?
        .as_array()?
        .iter()
        .find(|award| {
            award
                .get("instrument_id")
                .and_then(serde_json::Value::as_str)
                == Some("trace_credit")
        })?
        .get("atomic_units")?
        .as_str()?
        .parse::<AtomicUnits>()
        .ok()
        .and_then(|units| u64::try_from(units.get()).ok())
}

/// Used by Task 12's export snapshot request validation (not implemented in
/// this module yet), ported ahead of time per plan.
#[allow(dead_code)]
fn is_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    })
}

/// Used by Task 12's export snapshot request validation (not implemented in
/// this module yet), ported ahead of time per plan.
#[allow(dead_code)]
fn is_safe_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// Used by Task 13's summary reads (not implemented in this module yet),
/// ported ahead of time per plan.
#[allow(dead_code)]
fn count_from_row(row: &Row, column: &str) -> Result<u64, DatabaseError> {
    let count: i64 = row.get(column);
    u64::try_from(count)
        .map_err(|_| DatabaseError::Serialization("negative aggregate count".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_export_request_metadata_is_bounded() {
        assert!(is_safe_label("ranking_model_training"));
        assert!(!is_safe_label("Evaluation"));
        assert!(is_sha256(&sha256_prefixed(b"request")));
        assert!(!is_sha256("request"));
    }

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
}
