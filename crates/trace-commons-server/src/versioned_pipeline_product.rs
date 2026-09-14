//! Phase 6 product views over authoritative versioned-pipeline records.
//!
//! This module does not persist contributor status projections. Each status
//! response is derived from the run, immutable outcomes, credit ledger,
//! settlement batch, and payout state at read time.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use deadpool_postgres::Transaction;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_postgres::Row;
use trace_commons_gate_api::pipeline::Phase;
use uuid::Uuid;

use crate::db::postgres::PgBackend;
use crate::error::DatabaseError;
use crate::versioned_pipeline::{PipelineRunState, phase_from_db};

pub const PIPELINE_STATUS_BATCH_MAX: usize = 500;
pub const PIPELINE_EXPORT_ITEM_MAX: usize = 500;
pub const PIPELINE_EXPORT_SELECTION_POLICY_ID: &str = "trace_commons.pipeline_export_selection.v1";
pub const PIPELINE_AUTHORIZED_VIEW_SCHEMA_ID: &str = "trace_commons.authorized_trace_view.v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PipelineProcessingStatus {
    Pending,
    Retry,
    Blocked,
    Complete,
    Rejected,
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
pub struct PipelineContributorStatus {
    pub submission_id: Uuid,
    pub run_id: Uuid,
    pub bundle_id: String,
    pub processing: PipelineProcessingStatus,
    pub current_phase: Option<Phase>,
    pub responsible_phase: Option<Phase>,
    pub reason_label: Option<String>,
    pub credit: PipelineCreditStatus,
    pub score_microcredits: Option<u64>,
    pub score_outcome_id: Option<Uuid>,
    pub settlement_batch_id: Option<Uuid>,
    pub payout: Option<String>,
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
pub struct PipelineExportSnapshotItem {
    pub ordinal: u32,
    pub run_id: Uuid,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub registry_revision_id: Uuid,
    pub source_object_ref_id: Uuid,
    pub source_content_hash: String,
    pub bundle_id: String,
    pub outcome_schema_id: String,
    pub outcome_schema_version: u32,
    pub authorized_view_schema_id: String,
    pub consent_scopes: serde_json::Value,
    pub allowed_uses: serde_json::Value,
    pub invalidation_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineExportSnapshot {
    #[serde(skip_serializing, default)]
    pub tenant_id: String,
    pub snapshot_id: Uuid,
    pub request_idempotency_key: String,
    pub requester_principal_ref: String,
    pub allowed_use: String,
    pub purpose_hash: String,
    pub selection_policy_id: String,
    pub source_list_hash: String,
    pub state: String,
    pub export_manifest_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub invalidated_at: Option<DateTime<Utc>>,
    pub items: Vec<PipelineExportSnapshotItem>,
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
        if submission_ids.len() > PIPELINE_STATUS_BATCH_MAX {
            return Err(DatabaseError::Constraint(
                "submission status batch limit exceeded".to_string(),
            ));
        }
        if submission_ids.is_empty() {
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
                        batch.status AS settlement_batch_status
                   FROM unnest($3::uuid[]) WITH ORDINALITY requested(submission_id, ordinal)
                   JOIN trace_submissions s
                     ON s.tenant_id = $1
                    AND s.submission_id = requested.submission_id
                    AND s.auth_principal_ref = $2
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
                   LEFT JOIN trace_credit_settlement_batches batch
                     ON batch.tenant_id = r.tenant_id
                    AND batch.settlement_batch_id = r.settlement_batch_id
                  ORDER BY requested.ordinal",
                &[&tenant_id, &principal_ref, &submission_ids],
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
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT submission_id
                   FROM trace_submissions
                  WHERE tenant_id = $1 AND auth_principal_ref = $2
                    AND EXISTS (
                        SELECT 1 FROM pipeline_runs r
                         WHERE r.tenant_id = trace_submissions.tenant_id
                           AND r.submission_id = trace_submissions.submission_id
                    )
                  ORDER BY received_at DESC, submission_id DESC
                  LIMIT $3",
                &[
                    &tenant_id,
                    &principal_ref,
                    &(PIPELINE_STATUS_BATCH_MAX as i64),
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
        let scored_microcredits = statuses
            .iter()
            .filter_map(|status| status.score_microcredits)
            .sum();
        let finalized_microcredits = statuses
            .iter()
            .filter(|status| status.credit == PipelineCreditStatus::Finalized)
            .filter_map(|status| status.score_microcredits)
            .sum();
        let pending_microcredits = statuses
            .iter()
            .filter(|status| status.credit == PipelineCreditStatus::Pending)
            .filter_map(|status| status.score_microcredits)
            .sum();
        let held_microcredits = statuses
            .iter()
            .filter(|status| status.credit == PipelineCreditStatus::Held)
            .filter_map(|status| status.score_microcredits)
            .sum();
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

    pub async fn create_export_snapshot(
        &self,
        tenant_id: &str,
        requester_principal_ref: &str,
        request_idempotency_key: &str,
        allowed_use: &str,
        purpose_hash: &str,
        max_items: usize,
    ) -> Result<PipelineExportSnapshot, DatabaseError> {
        if max_items == 0 || max_items > PIPELINE_EXPORT_ITEM_MAX {
            return Err(DatabaseError::Constraint(
                "export item limit is invalid".to_string(),
            ));
        }
        if !is_safe_label(allowed_use)
            || !is_sha256(request_idempotency_key)
            || !is_sha256(purpose_hash)
        {
            return Err(DatabaseError::Constraint(
                "export request metadata is invalid".to_string(),
            ));
        }
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
            &[&format!("{tenant_id}:{request_idempotency_key}")],
        )
        .await?;
        if let Some(existing) =
            load_snapshot_by_request_key(&tx, tenant_id, request_idempotency_key).await?
        {
            if existing.requester_principal_ref != requester_principal_ref
                || existing.allowed_use != allowed_use
                || existing.purpose_hash != purpose_hash
            {
                return Err(DatabaseError::Constraint(
                    "export idempotency content conflict".to_string(),
                ));
            }
            tx.commit().await?;
            return Ok(existing);
        }
        let limit = i64::try_from(max_items)
            .map_err(|_| DatabaseError::Constraint("export item limit is invalid".to_string()))?;
        let rows = tx
            .query(
                "SELECT r.run_id, r.submission_id, r.trace_id, r.approved_revision_id,
                        r.transformed_object_ref_id, r.transformed_content_hash, r.bundle_id,
                        review.outcome_schema_id, review.outcome_schema_version,
                        s.consent_scopes, s.allowed_uses
                   FROM pipeline_runs r
                   JOIN trace_submissions s
                     ON s.tenant_id = r.tenant_id
                    AND s.submission_id = r.submission_id
                   JOIN trace_derived_records d
                     ON d.tenant_id = r.tenant_id
                    AND d.derived_id = r.approved_revision_id
                    AND d.submission_id = r.submission_id
                    AND d.status = 'current'
                   JOIN phase_outcomes review
                     ON review.tenant_id = r.tenant_id
                    AND review.run_id = r.run_id
                    AND review.phase = 'review'
                   LEFT JOIN trace_withdrawals w
                     ON w.tenant_id = r.tenant_id
                    AND w.submission_id = r.submission_id
                  WHERE r.tenant_id = $1
                    AND r.state = 'complete'
                    AND r.approved_revision_id IS NOT NULL
                    AND r.transformed_object_ref_id IS NOT NULL
                    AND r.transformed_content_hash IS NOT NULL
                    AND s.status = 'accepted'
                    AND s.revoked_at IS NULL
                    AND s.purged_at IS NULL
                    AND (s.expires_at IS NULL OR s.expires_at > NOW())
                    AND w.submission_id IS NULL
                    AND s.allowed_uses ? $2
                  ORDER BY r.created_at ASC, r.run_id ASC
                  LIMIT $3",
                &[&tenant_id, &allowed_use, &limit],
            )
            .await?;
        let snapshot_id = Uuid::new_v4();
        let source_list_hash = source_list_hash(&rows);
        let item_count = i32::try_from(rows.len())
            .map_err(|_| DatabaseError::Constraint("export item count overflow".to_string()))?;
        tx.execute(
            "INSERT INTO pipeline_export_snapshots (
                tenant_id, snapshot_id, request_idempotency_key,
                requester_principal_ref, allowed_use, purpose_hash,
                selection_policy_id, source_list_hash, item_count
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
            &[
                &tenant_id,
                &snapshot_id,
                &request_idempotency_key,
                &requester_principal_ref,
                &allowed_use,
                &purpose_hash,
                &PIPELINE_EXPORT_SELECTION_POLICY_ID,
                &source_list_hash,
                &item_count,
            ],
        )
        .await?;
        for (ordinal, row) in rows.iter().enumerate() {
            let ordinal = i32::try_from(ordinal).map_err(|_| {
                DatabaseError::Constraint("export item ordinal overflow".to_string())
            })?;
            tx.execute(
                "INSERT INTO pipeline_export_snapshot_items (
                    tenant_id, snapshot_id, ordinal, run_id, submission_id, trace_id,
                    registry_revision_id, source_object_ref_id, source_content_hash,
                    bundle_id, outcome_schema_id, outcome_schema_version,
                    authorized_view_schema_id, consent_scopes, allowed_uses
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)",
                &[
                    &tenant_id,
                    &snapshot_id,
                    &ordinal,
                    &row.get::<_, Uuid>("run_id"),
                    &row.get::<_, Uuid>("submission_id"),
                    &row.get::<_, Uuid>("trace_id"),
                    &row.get::<_, Uuid>("approved_revision_id"),
                    &row.get::<_, Uuid>("transformed_object_ref_id"),
                    &row.get::<_, String>("transformed_content_hash"),
                    &row.get::<_, String>("bundle_id"),
                    &row.get::<_, String>("outcome_schema_id"),
                    &row.get::<_, i32>("outcome_schema_version"),
                    &PIPELINE_AUTHORIZED_VIEW_SCHEMA_ID,
                    &row.get::<_, serde_json::Value>("consent_scopes"),
                    &row.get::<_, serde_json::Value>("allowed_uses"),
                ],
            )
            .await?;
        }
        let snapshot = load_snapshot(&tx, tenant_id, snapshot_id, requester_principal_ref)
            .await?
            .ok_or_else(|| DatabaseError::NotFound {
                entity: "pipeline_export_snapshot".to_string(),
                id: snapshot_id.to_string(),
            })?;
        tx.commit().await?;
        Ok(snapshot)
    }

    pub async fn complete_export_snapshot(
        &self,
        tenant_id: &str,
        requester_principal_ref: &str,
        snapshot_id: Uuid,
    ) -> Result<PipelineExportSnapshot, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let snapshot = load_snapshot(&tx, tenant_id, snapshot_id, requester_principal_ref)
            .await?
            .ok_or_else(|| DatabaseError::NotFound {
                entity: "pipeline_export_snapshot".to_string(),
                id: snapshot_id.to_string(),
            })?;
        if snapshot.state == "invalidated" {
            return Err(DatabaseError::Constraint(
                "export snapshot is invalidated".to_string(),
            ));
        }
        if snapshot.state == "complete" {
            tx.commit().await?;
            return Ok(snapshot);
        }
        if snapshot
            .items
            .iter()
            .any(|item| item.invalidation_reason.is_some())
        {
            return Err(DatabaseError::Constraint(
                "export snapshot contains an invalidated source".to_string(),
            ));
        }
        let submission_ids = snapshot
            .items
            .iter()
            .map(|item| item.submission_id)
            .collect::<Vec<_>>();
        let item_count = i32::try_from(snapshot.items.len())
            .map_err(|_| DatabaseError::Constraint("export item count overflow".to_string()))?;
        tx.execute(
            "INSERT INTO trace_export_manifests (
                tenant_id, export_manifest_id, artifact_kind, purpose_code,
                source_submission_ids, source_submission_ids_hash, item_count, generated_at
             ) VALUES ($1,$2,'pipeline_authorized_view',$3,$4,$5,$6,NOW())
             ON CONFLICT (tenant_id, export_manifest_id) DO NOTHING",
            &[
                &tenant_id,
                &snapshot_id,
                &snapshot.allowed_use,
                &submission_ids,
                &snapshot.source_list_hash,
                &item_count,
            ],
        )
        .await?;
        for item in &snapshot.items {
            tx.execute(
                "INSERT INTO trace_export_manifest_items (
                    tenant_id, export_manifest_id, submission_id, trace_id,
                    derived_id, object_ref_id, source_status_at_export,
                    source_hash_at_export
                 ) VALUES ($1,$2,$3,$4,$5,$6,'accepted',$7)
                 ON CONFLICT (tenant_id, export_manifest_id, submission_id) DO NOTHING",
                &[
                    &tenant_id,
                    &snapshot_id,
                    &item.submission_id,
                    &item.trace_id,
                    &item.registry_revision_id,
                    &item.source_object_ref_id,
                    &item.source_content_hash,
                ],
            )
            .await?;
        }
        tx.execute(
            "UPDATE pipeline_export_snapshots
                SET state = 'complete', export_manifest_id = $3,
                    completed_at = COALESCE(completed_at, NOW())
              WHERE tenant_id = $1 AND snapshot_id = $2 AND state = 'ready'",
            &[&tenant_id, &snapshot_id, &snapshot_id],
        )
        .await?;
        let completed = load_snapshot(&tx, tenant_id, snapshot_id, requester_principal_ref)
            .await?
            .ok_or_else(|| DatabaseError::NotFound {
                entity: "pipeline_export_snapshot".to_string(),
                id: snapshot_id.to_string(),
            })?;
        tx.commit().await?;
        Ok(completed)
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

    pub async fn operational_summary(
        &self,
        tenant_id: &str,
    ) -> Result<PipelineOperationalSummary, DatabaseError> {
        let generated_at = Utc::now();
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let work_rows = tx
            .query(
                "SELECT next_phase, state, COUNT(*) AS item_count,
                        GREATEST(
                            0,
                            EXTRACT(EPOCH FROM (NOW() - MIN(phase_started_at)))::bigint
                        ) AS oldest_age_seconds
                   FROM pipeline_runs
                  WHERE tenant_id = $1
                  GROUP BY next_phase, state
                  ORDER BY next_phase, state",
                &[&tenant_id],
            )
            .await?;
        let summary = tx
            .query_one(
                "SELECT
                    (SELECT COUNT(*) FROM pipeline_bundle_policy_status
                      WHERE tenant_id = $1 AND operational_status = 'suspended')
                        AS suspended_policies,
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
                    (SELECT COUNT(*) FROM pipeline_runs
                      WHERE tenant_id = $1 AND credit_write_state = 'held')
                        AS held_credit,
                    (SELECT COUNT(*) FROM pipeline_runs
                      WHERE tenant_id = $1 AND credit_write_state = 'pending')
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
                        "pipeline_bundle_qualifications",
                        "pipeline_tenant_routing",
                        "pipeline_activation_events",
                        "pipeline_receipt_ownership",
                        "pipeline_legacy_owned_work",
                        "pipeline_legacy_writer_status",
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
            suspended_policy_count: count_from_row(&summary, "suspended_policies")?,
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
                        index_write_state, credit_event_id, settlement_batch_id,
                        payout_state, index_invalidation_state
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
        let intervention_rows = tx
            .query(
                "SELECT i.evidence_hash
                   FROM pipeline_policy_interventions i
                  WHERE i.tenant_id = $1 AND i.bundle_id = $2
                  ORDER BY i.recorded_at, i.intervention_id",
                &[&tenant_id, &run.get::<_, String>("bundle_id")],
            )
            .await?;
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
        Ok(Some(PipelineForensicTrace {
            run_id: run.get("run_id"),
            submission_id: run.get("submission_id"),
            bundle_id: run.get("bundle_id"),
            phases,
            index_command_hash: run.get("index_command_hash"),
            index_write_state: run.get("index_write_state"),
            score_outcome_id,
            credit_event_id: run.get("credit_event_id"),
            settlement_batch_id: run.get("settlement_batch_id"),
            payout_state: run.get("payout_state"),
            intervention_evidence_hashes: intervention_rows
                .iter()
                .map(|row| row.get("evidence_hash"))
                .collect(),
            index_invalidation_state: run.get("index_invalidation_state"),
        }))
    }
}

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
    let processing = if submission_status == "rejected" {
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
            PipelineRunState::Complete => PipelineProcessingStatus::Complete,
            PipelineRunState::Failed => PipelineProcessingStatus::Failed,
        }
    };
    let score_outcome_id = row.get("score_outcome_id");
    let score_microcredits = row
        .get::<_, Option<serde_json::Value>>("score_decision")
        .as_ref()
        .and_then(score_microcredits);
    let credit_write_state: String = row.get("credit_write_state");
    let settlement_batch_status: Option<String> = row.get("settlement_batch_status");
    let credit = match (score_outcome_id, score_microcredits) {
        (None, _) => PipelineCreditStatus::Unscored,
        (Some(_), Some(0)) => PipelineCreditStatus::Zero,
        (Some(_), Some(_)) if credit_write_state == "held" => PipelineCreditStatus::Held,
        (Some(_), Some(_)) if credit_write_state == "failed" => PipelineCreditStatus::Failed,
        (Some(_), Some(_)) if settlement_batch_status.as_deref() == Some("finalized") => {
            PipelineCreditStatus::Finalized
        }
        _ => PipelineCreditStatus::Pending,
    };
    let payout = match row.get::<_, String>("payout_state").as_str() {
        "none" => None,
        value => Some(value.to_string()),
    };
    Ok(PipelineContributorStatus {
        submission_id: row.get("submission_id"),
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
        settlement_batch_id: row.get("settlement_batch_id"),
        payout,
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

async fn load_snapshot_by_request_key(
    tx: &Transaction<'_>,
    tenant_id: &str,
    request_idempotency_key: &str,
) -> Result<Option<PipelineExportSnapshot>, DatabaseError> {
    let row = tx
        .query_opt(
            "SELECT snapshot_id, requester_principal_ref
               FROM pipeline_export_snapshots
              WHERE tenant_id = $1 AND request_idempotency_key = $2",
            &[&tenant_id, &request_idempotency_key],
        )
        .await?;
    match row {
        Some(row) => {
            let snapshot_id = row.get("snapshot_id");
            let principal: String = row.get("requester_principal_ref");
            load_snapshot(tx, tenant_id, snapshot_id, &principal).await
        }
        None => Ok(None),
    }
}

async fn load_snapshot(
    tx: &Transaction<'_>,
    tenant_id: &str,
    snapshot_id: Uuid,
    requester_principal_ref: &str,
) -> Result<Option<PipelineExportSnapshot>, DatabaseError> {
    let Some(row) = tx
        .query_opt(
            "SELECT *
               FROM pipeline_export_snapshots
              WHERE tenant_id = $1 AND snapshot_id = $2
                AND requester_principal_ref = $3",
            &[&tenant_id, &snapshot_id, &requester_principal_ref],
        )
        .await?
    else {
        return Ok(None);
    };
    let item_rows = tx
        .query(
            "SELECT *
               FROM pipeline_export_snapshot_items
              WHERE tenant_id = $1 AND snapshot_id = $2
              ORDER BY ordinal",
            &[&tenant_id, &snapshot_id],
        )
        .await?;
    let items = item_rows
        .iter()
        .map(snapshot_item_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(PipelineExportSnapshot {
        tenant_id: row.get("tenant_id"),
        snapshot_id: row.get("snapshot_id"),
        request_idempotency_key: row.get("request_idempotency_key"),
        requester_principal_ref: row.get("requester_principal_ref"),
        allowed_use: row.get("allowed_use"),
        purpose_hash: row.get("purpose_hash"),
        selection_policy_id: row.get("selection_policy_id"),
        source_list_hash: row.get("source_list_hash"),
        state: row.get("state"),
        export_manifest_id: row.get("export_manifest_id"),
        created_at: row.get("created_at"),
        completed_at: row.get("completed_at"),
        invalidated_at: row.get("invalidated_at"),
        items,
    }))
}

fn snapshot_item_from_row(row: &Row) -> Result<PipelineExportSnapshotItem, DatabaseError> {
    let ordinal: i32 = row.get("ordinal");
    let version: i32 = row.get("outcome_schema_version");
    Ok(PipelineExportSnapshotItem {
        ordinal: u32::try_from(ordinal)
            .map_err(|_| DatabaseError::Serialization("invalid export ordinal".to_string()))?,
        run_id: row.get("run_id"),
        submission_id: row.get("submission_id"),
        trace_id: row.get("trace_id"),
        registry_revision_id: row.get("registry_revision_id"),
        source_object_ref_id: row.get("source_object_ref_id"),
        source_content_hash: row.get("source_content_hash"),
        bundle_id: row.get("bundle_id"),
        outcome_schema_id: row.get("outcome_schema_id"),
        outcome_schema_version: u32::try_from(version).map_err(|_| {
            DatabaseError::Serialization("invalid export outcome schema version".to_string())
        })?,
        authorized_view_schema_id: row.get("authorized_view_schema_id"),
        consent_scopes: row.get("consent_scopes"),
        allowed_uses: row.get("allowed_uses"),
        invalidation_reason: row.get("invalidation_reason"),
    })
}

fn source_list_hash(rows: &[Row]) -> String {
    let mut hasher = Sha256::new();
    for row in rows {
        hasher.update(row.get::<_, Uuid>("approved_revision_id").as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

pub fn sha256_prefixed(value: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(value))
}

fn score_microcredits(decision: &serde_json::Value) -> Option<u64> {
    decision.get("credit_microcredits")?.as_u64()
}

fn is_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    })
}

fn is_safe_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

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
            score_microcredits(&serde_json::json!({"credit_microcredits": 7})),
            Some(7)
        );
        assert_eq!(score_microcredits(&serde_json::json!({"credit": 7})), None);
    }
}
