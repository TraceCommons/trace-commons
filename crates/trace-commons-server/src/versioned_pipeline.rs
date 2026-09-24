// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Runtime for the versioned Trace Commons pipeline.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use deadpool_postgres::Transaction;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_postgres::Row;
use trace_commons_gate_api::pipeline::{
    AdmissionDecision, AdmissionEvaluation, AdmissionEvidence, AtomicUnits, BundlePackage,
    PIPELINE_OUTCOME_SCHEMA_ID, PIPELINE_OUTCOME_SCHEMA_VERSION, Phase, PhaseResult,
    ReviewDecision, ReviewEvaluation, ReviewEvidence, SchemaRef, ScoreDecision, ScoreEvaluation,
    ScoreEvidence, SettleDecision, SettleEvaluation, SettleEvidence, TenantStorageRef,
};
use uuid::Uuid;

use crate::db::postgres::PgBackend;
use crate::error::DatabaseError;
use crate::versioned_pipeline_bundle::PIPELINE_BUNDLE_INVALID_LABEL;

/// The tenant's derived storage reference, the same value ingest's
/// `tenant_storage_ref` produces: the first 16 bytes of SHA-256, as hex.
/// Every artifact and index call in the pipeline is keyed by it.
pub fn pipeline_tenant_storage_ref(tenant_id: &str) -> TenantStorageRef {
    let digest = Sha256::digest(tenant_id.as_bytes());
    TenantStorageRef::new(format!("tenant_sha256:{}", hex::encode(&digest[..16])))
        .expect("derived storage reference has the contract shape")
}

pub const PIPELINE_OPERATIONAL_ERROR_LABEL: &str = "minimal_policy_failed";
pub const PIPELINE_ATTEMPTS_EXHAUSTED_LABEL: &str = "attempts_exhausted";
pub const PIPELINE_BUNDLE_MISSING_LABEL: &str = "bundle_package_missing";
pub const PIPELINE_POLICY_NOT_RUNNABLE_LABEL: &str = "bundle_policy_not_runnable";
pub const PIPELINE_INDEX_UNAVAILABLE_LABEL: &str = "index_unavailable";
pub const PIPELINE_INDEX_CONFLICT_LABEL: &str = "index_key_conflict";
pub const PIPELINE_CREDIT_HELD_LABEL: &str = "credit_held";
pub const PIPELINE_CREDIT_CAP_LABEL: &str = "credit_cap_exceeded";
pub const PIPELINE_SUBMISSION_INOPERABLE_LABEL: &str = "submission_inoperable";
pub const PIPELINE_TOMBSTONE_LABEL: &str = "content_tombstoned";
pub const INJECTED_PIPELINE_CRASH: &str = "injected_pipeline_crash";
const DEFAULT_LEASE_SECONDS: i64 = 30;
const DEFAULT_RETRY_MILLISECONDS: i64 = 50;
/// Delay before a transient (dependency-failure) retry. Short and fixed,
/// unlike `mark_retry`'s exponential backoff, because the failure is not the
/// run's fault and the attempt budget is not charged for it (decision D9).
const TRANSIENT_RETRY_MILLISECONDS: i64 = 1_000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PipelineRunState {
    Pending,
    Leased,
    Retry,
    Complete,
    Failed,
}

impl PipelineRunState {
    fn as_db(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Leased => "leased",
            Self::Retry => "retry",
            Self::Complete => "complete",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn from_db(value: &str) -> Result<Self, DatabaseError> {
        match value {
            "pending" => Ok(Self::Pending),
            "leased" => Ok(Self::Leased),
            "retry" => Ok(Self::Retry),
            "complete" => Ok(Self::Complete),
            "failed" => Ok(Self::Failed),
            _ => Err(DatabaseError::Serialization(
                "unknown pipeline run state".to_string(),
            )),
        }
    }
}

fn phase_as_db(phase: Option<Phase>) -> &'static str {
    match phase {
        Some(Phase::Admission) => "admission",
        Some(Phase::Review) => "review",
        Some(Phase::Score) => "score",
        Some(Phase::Settle) => "settle",
        None => "none",
    }
}

pub(crate) fn phase_from_db(value: &str) -> Result<Option<Phase>, DatabaseError> {
    match value {
        "admission" => Ok(Some(Phase::Admission)),
        "review" => Ok(Some(Phase::Review)),
        "score" => Ok(Some(Phase::Score)),
        "settle" => Ok(Some(Phase::Settle)),
        "none" => Ok(None),
        _ => Err(DatabaseError::Serialization(
            "unknown pipeline phase".to_string(),
        )),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineRunRecord {
    #[serde(skip_serializing, default)]
    pub tenant_id: String,
    pub run_id: Uuid,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub bundle_id: String,
    pub request_idempotency_key: String,
    pub request_content_hash: String,
    pub source_object_ref_id: Uuid,
    pub approved_revision_id: Option<Uuid>,
    pub next_phase: Option<Phase>,
    pub state: PipelineRunState,
    pub lease_token: Option<Uuid>,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub attempt_count: u32,
    pub max_attempts: u32,
    pub next_attempt_at: DateTime<Utc>,
    pub phase_started_at: DateTime<Utc>,
    pub last_error_label: Option<String>,
    pub index_membership: String,
    pub index_command_ref: Option<String>,
    pub index_command_hash: Option<String>,
    pub index_write_state: String,
    pub admission_decision: String,
    pub approved_object_ref_id: Option<Uuid>,
    pub approved_content_hash: Option<String>,
    pub score_neighbor_ref: Option<String>,
    pub score_neighbor_hash: Option<String>,
    pub settle_selection_hash: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhaseOutcomeRecord {
    #[serde(skip_serializing, default)]
    pub tenant_id: String,
    pub outcome_id: Uuid,
    pub run_id: Uuid,
    pub trace_id: Uuid,
    pub phase: Phase,
    pub bundle_id: String,
    pub outcome_schema: SchemaRef,
    pub decision: serde_json::Value,
    pub evidence: serde_json::Value,
    pub evaluation: serde_json::Value,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineSettlementRecord {
    #[serde(skip_serializing, default)]
    pub tenant_id: String,
    pub run_id: Uuid,
    pub instrument_id: String,
    pub atomic_units: AtomicUnits,
    pub operation_ref_hash: String,
    pub result_ref_hash: Option<String>,
    pub operation_state: String,
    pub credit_event_id: Option<Uuid>,
    pub settlement_batch_id: Option<Uuid>,
    pub payout_rail: String,
    pub payout_state: String,
    pub attempt_count: u32,
    pub max_attempts: u32,
    pub last_error_label: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StoredPhaseResult {
    pub phase: Phase,
    pub decision: serde_json::Value,
    pub evidence: serde_json::Value,
    pub evaluation: serde_json::Value,
}

impl StoredPhaseResult {
    pub fn from_result<D, E, V>(
        phase: Phase,
        result: &PhaseResult<D, E, V>,
    ) -> Result<Self, DatabaseError>
    where
        D: Serialize,
        E: Serialize,
        V: Serialize,
    {
        Ok(Self {
            phase,
            decision: serde_json::to_value(&result.decision)
                .map_err(|_| DatabaseError::Serialization("decision encode failed".to_string()))?,
            evidence: serde_json::to_value(&result.evidence)
                .map_err(|_| DatabaseError::Serialization("evidence encode failed".to_string()))?,
            evaluation: serde_json::to_value(&result.evaluation).map_err(|_| {
                DatabaseError::Serialization("evaluation encode failed".to_string())
            })?,
        })
    }
}

pub struct PgPipelineStore {
    backend: Arc<PgBackend>,
}

impl PgPipelineStore {
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

    pub async fn register_bundle(
        &self,
        tenant_id: &str,
        package: &BundlePackage,
    ) -> Result<(), DatabaseError> {
        package
            .validate()
            .map_err(|_| DatabaseError::Serialization(PIPELINE_BUNDLE_INVALID_LABEL.to_string()))?;
        let package_json = serde_json::to_value(package)
            .map_err(|_| DatabaseError::Serialization(PIPELINE_BUNDLE_INVALID_LABEL.to_string()))?;
        let format_version = i32::try_from(package.manifest.format_version)
            .map_err(|_| DatabaseError::Serialization(PIPELINE_BUNDLE_INVALID_LABEL.to_string()))?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)
             ON CONFLICT (tenant_id) DO NOTHING",
            &[&tenant_id],
        )
        .await?;
        // Serialize the descriptor-conflict check below against a concurrent
        // registration for this tenant, so two packages racing to pin a new
        // instrument cannot both read past each other and both commit.
        let lock_key = format!("pipeline-bundle-registry:{tenant_id}");
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 1))",
            &[&lock_key],
        )
        .await?;
        let registered = tx
            .query(
                "SELECT package FROM pipeline_bundle_packages WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await?;
        for row in &registered {
            let registered_package: BundlePackage = serde_json::from_value(
                row.get::<_, serde_json::Value>("package"),
            )
            .map_err(|_| DatabaseError::Serialization(PIPELINE_BUNDLE_INVALID_LABEL.to_string()))?;
            for (instrument_id, descriptor) in &package.manifest.instruments {
                if let Some(registered_descriptor) =
                    registered_package.manifest.instruments.get(instrument_id)
                {
                    if registered_descriptor != descriptor {
                        return Err(DatabaseError::Constraint(
                            "bundle_instrument_conflict".to_string(),
                        ));
                    }
                }
            }
        }
        tx.execute(
            "INSERT INTO pipeline_bundle_packages (
                tenant_id, bundle_id, manifest_format_version, package
             ) VALUES ($1,$2,$3,$4)
             ON CONFLICT (tenant_id, bundle_id) DO NOTHING",
            &[
                &tenant_id,
                &package.bundle_id,
                &format_version,
                &package_json,
            ],
        )
        .await?;
        let stored: serde_json::Value = tx
            .query_one(
                "SELECT package FROM pipeline_bundle_packages
                 WHERE tenant_id = $1 AND bundle_id = $2",
                &[&tenant_id, &package.bundle_id],
            )
            .await?
            .get("package");
        if stored != package_json {
            return Err(DatabaseError::Constraint(
                "bundle identifier already has different package bytes".to_string(),
            ));
        }
        for phase in [Phase::Admission, Phase::Review, Phase::Score, Phase::Settle] {
            tx.execute(
                "INSERT INTO pipeline_bundle_policy_status (
                    tenant_id, bundle_id, phase, runnable
                 ) VALUES ($1,$2,$3,TRUE)
                 ON CONFLICT (tenant_id, bundle_id, phase) DO NOTHING",
                &[&tenant_id, &package.bundle_id, &phase_as_db(Some(phase))],
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn activate_bundle(
        &self,
        tenant_id: &str,
        bundle_id: &str,
    ) -> Result<(), DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let row_count = tx
            .execute(
                "INSERT INTO pipeline_active_bundles (tenant_id, bundle_id)
                 SELECT $1, bundle_id
                 FROM pipeline_bundle_packages
                 WHERE tenant_id = $1 AND bundle_id = $2
                 ON CONFLICT (tenant_id) DO UPDATE
                 SET bundle_id = EXCLUDED.bundle_id, selected_at = NOW()",
                &[&tenant_id, &bundle_id],
            )
            .await?;
        if row_count != 1 {
            return Err(DatabaseError::NotFound {
                entity: "pipeline_bundle".to_string(),
                id: bundle_id.to_string(),
            });
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn activate_bundle_if_none(
        &self,
        tenant_id: &str,
        bundle_id: &str,
    ) -> Result<(), DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "INSERT INTO pipeline_active_bundles (tenant_id, bundle_id)
             SELECT $1, bundle_id
             FROM pipeline_bundle_packages
             WHERE tenant_id = $1 AND bundle_id = $2
             ON CONFLICT (tenant_id) DO NOTHING",
            &[&tenant_id, &bundle_id],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn active_bundle_id(&self, tenant_id: &str) -> Result<Option<String>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT bundle_id FROM pipeline_active_bundles WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await?;
        tx.commit().await?;
        Ok(row.map(|row| row.get("bundle_id")))
    }

    pub async fn load_bundle(
        &self,
        tenant_id: &str,
        bundle_id: &str,
    ) -> Result<Option<BundlePackage>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let package = load_bundle_from_transaction(&tx, tenant_id, bundle_id).await?;
        tx.commit().await?;
        Ok(package)
    }

    pub async fn policy_is_runnable(
        &self,
        tenant_id: &str,
        bundle_id: &str,
        phase: Phase,
    ) -> Result<bool, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let runnable = tx
            .query_opt(
                "SELECT runnable FROM pipeline_bundle_policy_status
                 WHERE tenant_id = $1 AND bundle_id = $2 AND phase = $3",
                &[&tenant_id, &bundle_id, &phase_as_db(Some(phase))],
            )
            .await?
            .map(|row| row.get("runnable"))
            .unwrap_or(false);
        tx.commit().await?;
        Ok(runnable)
    }

    pub async fn get_run(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> Result<Option<PipelineRunRecord>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT * FROM pipeline_runs WHERE tenant_id = $1 AND run_id = $2",
                &[&tenant_id, &run_id],
            )
            .await?;
        tx.commit().await?;
        row.as_ref().map(pipeline_run_from_row).transpose()
    }

    pub async fn list_outcomes(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> Result<Vec<PhaseOutcomeRecord>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT * FROM phase_outcomes
                 WHERE tenant_id = $1 AND run_id = $2
                 ORDER BY CASE phase
                    WHEN 'admission' THEN 1
                    WHEN 'review' THEN 2
                    WHEN 'score' THEN 3
                    WHEN 'settle' THEN 4
                 END",
                &[&tenant_id, &run_id],
            )
            .await?;
        tx.commit().await?;
        rows.iter().map(phase_outcome_from_row).collect()
    }

    pub async fn claim_next(
        &self,
        tenant_id: &str,
    ) -> Result<Option<PipelineRunRecord>, DatabaseError> {
        self.claim_next_with_lease(tenant_id, Duration::seconds(DEFAULT_LEASE_SECONDS))
            .await
    }

    pub async fn claim_next_with_lease(
        &self,
        tenant_id: &str,
        lease_duration: Duration,
    ) -> Result<Option<PipelineRunRecord>, DatabaseError> {
        if lease_duration <= Duration::zero() || lease_duration > Duration::minutes(5) {
            return Err(DatabaseError::Constraint(
                "pipeline lease duration is invalid".to_string(),
            ));
        }
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "UPDATE pipeline_runs
             SET state = 'failed', lease_token = NULL, lease_expires_at = NULL,
                 last_error_label = $2, updated_at = NOW()
             WHERE tenant_id = $1
               AND state = 'leased'
               AND lease_expires_at <= NOW()
               AND attempt_count >= max_attempts",
            &[&tenant_id, &PIPELINE_ATTEMPTS_EXHAUSTED_LABEL],
        )
        .await?;
        let lease_token = Uuid::new_v4();
        let lease_milliseconds = lease_duration.num_milliseconds();
        let row = tx
            .query_opt(
                "WITH candidate AS (
                    SELECT run_id FROM pipeline_runs
                    WHERE tenant_id = $1
                      AND next_phase <> 'none'
                      AND attempt_count < max_attempts
                      AND (
                          (state IN ('pending', 'retry') AND next_attempt_at <= NOW())
                          OR (state = 'leased' AND lease_expires_at <= NOW())
                      )
                    ORDER BY next_attempt_at ASC, created_at ASC, run_id ASC
                    FOR UPDATE SKIP LOCKED
                    LIMIT 1
                 )
                 UPDATE pipeline_runs p
                 SET state = 'leased',
                     lease_token = $2,
                     lease_expires_at = NOW() + ($3::bigint * INTERVAL '1 millisecond'),
                     attempt_count = p.attempt_count + 1,
                     last_error_label = NULL,
                     updated_at = NOW()
                 FROM candidate
                 WHERE p.tenant_id = $1 AND p.run_id = candidate.run_id
                 RETURNING p.*",
                &[&tenant_id, &lease_token, &lease_milliseconds],
            )
            .await?;
        tx.commit().await?;
        row.as_ref().map(pipeline_run_from_row).transpose()
    }

    pub async fn claim_run(
        &self,
        tenant_id: &str,
        run_id: Uuid,
        lease_duration: Duration,
    ) -> Result<Option<PipelineRunRecord>, DatabaseError> {
        if lease_duration <= Duration::zero() || lease_duration > Duration::minutes(5) {
            return Err(DatabaseError::Constraint(
                "pipeline lease duration is invalid".to_string(),
            ));
        }
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let lease_token = Uuid::new_v4();
        let lease_milliseconds = lease_duration.num_milliseconds();
        let row = tx
            .query_opt(
                "UPDATE pipeline_runs
                 SET state = 'leased',
                     lease_token = $3,
                     lease_expires_at = NOW() + ($4::bigint * INTERVAL '1 millisecond'),
                     attempt_count = attempt_count + 1,
                     last_error_label = NULL,
                     updated_at = NOW()
                 WHERE tenant_id = $1 AND run_id = $2
                   AND next_phase <> 'none'
                   AND attempt_count < max_attempts
                   AND (
                       (state IN ('pending', 'retry') AND next_attempt_at <= NOW())
                       OR (state = 'leased' AND lease_expires_at <= NOW())
                   )
                 RETURNING *",
                &[&tenant_id, &run_id, &lease_token, &lease_milliseconds],
            )
            .await?;
        tx.commit().await?;
        row.as_ref().map(pipeline_run_from_row).transpose()
    }

    pub async fn release_claim(&self, run: &PipelineRunRecord) -> Result<(), DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let updated = tx
            .execute(
                "UPDATE pipeline_runs
             SET state = 'pending', lease_token = NULL, lease_expires_at = NULL,
                 attempt_count = GREATEST(attempt_count - 1, 0), updated_at = NOW()
             WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'
               AND lease_token = $3 AND lease_expires_at > NOW()",
                &[&run.tenant_id, &run.run_id, &lease_token],
            )
            .await?;
        if updated != 1 {
            return Err(stale_lease_error());
        }
        tx.commit().await?;
        Ok(())
    }

    /// Commits a phase outcome, advances `next_phase`, and clears the lease.
    // Task 10 replaces the Review branch.
    pub async fn commit_phase(
        &self,
        run: &PipelineRunRecord,
        outcome: StoredPhaseResult,
        next_phase: Option<Phase>,
        approved_revision_id: Option<Uuid>,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        if run.next_phase != Some(outcome.phase) {
            return Err(DatabaseError::Constraint(
                "phase does not match run transition".to_string(),
            ));
        }
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let current = tx
            .query_opt(
                "SELECT * FROM pipeline_runs
                 WHERE tenant_id = $1 AND run_id = $2
                 FOR UPDATE",
                &[&run.tenant_id, &run.run_id],
            )
            .await?
            .ok_or_else(|| DatabaseError::NotFound {
                entity: "pipeline_run".to_string(),
                id: run.run_id.to_string(),
            })?;
        let current = pipeline_run_from_row(&current)?;
        if current.state != PipelineRunState::Leased
            || current.next_phase != Some(outcome.phase)
            || current.lease_token != Some(lease_token)
            || current
                .lease_expires_at
                .is_none_or(|expires_at| expires_at <= Utc::now())
        {
            return Err(stale_lease_error());
        }
        if outcome.phase == Phase::Review {
            if approved_revision_id.is_some() {
                tx.execute(
                    "UPDATE trace_submissions
                     SET status = 'accepted', reviewed_at = NOW(), updated_at = NOW()
                     WHERE tenant_id = $1 AND submission_id = $2",
                    &[&run.tenant_id, &run.submission_id],
                )
                .await?;
            } else {
                tx.execute(
                    "UPDATE trace_submissions
                     SET status = 'rejected', reviewed_at = NOW(), updated_at = NOW()
                     WHERE tenant_id = $1 AND submission_id = $2",
                    &[&run.tenant_id, &run.submission_id],
                )
                .await?;
            }
        }
        insert_outcome(
            &tx,
            &run.tenant_id,
            run.run_id,
            run.trace_id,
            &run.bundle_id,
            Uuid::new_v4(),
            outcome,
        )
        .await?;
        let terminal = next_phase.is_none();
        let state = if terminal {
            PipelineRunState::Complete
        } else {
            PipelineRunState::Pending
        };
        let row = tx
            .query_one(
                "UPDATE pipeline_runs
                 SET next_phase = $3, state = $4,
                     approved_revision_id = COALESCE($5, approved_revision_id),
                     lease_token = NULL, lease_expires_at = NULL,
                     next_attempt_at = NOW(), phase_started_at = NOW(), updated_at = NOW()
                 WHERE tenant_id = $1 AND run_id = $2
                   AND lease_token = $6 AND lease_expires_at > NOW()
                 RETURNING *",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &phase_as_db(next_phase),
                    &state.as_db(),
                    &approved_revision_id,
                    &lease_token,
                ],
            )
            .await?;
        let updated = pipeline_run_from_row(&row)?;
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn mark_failed(
        &self,
        run: &PipelineRunRecord,
        error_label: &str,
    ) -> Result<(), DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let updated = tx
            .execute(
                "UPDATE pipeline_runs
             SET state = 'failed', lease_token = NULL, lease_expires_at = NULL,
                 last_error_label = $3, updated_at = NOW()
             WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'
               AND lease_token = $4 AND lease_expires_at > NOW()",
                &[&run.tenant_id, &run.run_id, &error_label, &lease_token],
            )
            .await?;
        if updated != 1 {
            return Err(stale_lease_error());
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn mark_retry(
        &self,
        run: &PipelineRunRecord,
        error_label: &str,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let exponent = run.attempt_count.saturating_sub(1).min(9);
        let multiplier = 1_i64 << exponent;
        let delay_milliseconds = DEFAULT_RETRY_MILLISECONDS.saturating_mul(multiplier);
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let row = tx
            .query_opt(
                "UPDATE pipeline_runs
                 SET state = CASE
                         WHEN attempt_count >= max_attempts THEN 'failed'
                         ELSE 'retry'
                     END,
                     lease_token = NULL,
                     lease_expires_at = NULL,
                     next_attempt_at = CASE
                         WHEN attempt_count >= max_attempts THEN next_attempt_at
                         ELSE NOW() + ($5::bigint * INTERVAL '1 millisecond')
                     END,
                     last_error_label = CASE
                         WHEN attempt_count >= max_attempts THEN $4
                         ELSE $3
                     END,
                     updated_at = NOW()
                 WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'
                   AND lease_token = $6 AND lease_expires_at > NOW()
                 RETURNING *",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &error_label,
                    &PIPELINE_ATTEMPTS_EXHAUSTED_LABEL,
                    &delay_milliseconds,
                    &lease_token,
                ],
            )
            .await?
            .ok_or_else(stale_lease_error)?;
        let updated = pipeline_run_from_row(&row)?;
        tx.commit().await?;
        Ok(updated)
    }

    /// A dependency failure: release the claim and schedule a retry without
    /// charging the attempt the claim took.
    pub async fn mark_transient_retry(
        &self,
        run: &PipelineRunRecord,
        error_label: &str,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let row = tx
            .query_opt(
                "UPDATE pipeline_runs
                    SET state = 'retry', lease_token = NULL, lease_expires_at = NULL,
                        attempt_count = GREATEST(attempt_count - 1, 0),
                        next_attempt_at = NOW() + ($4::bigint * INTERVAL '1 millisecond'),
                        last_error_label = $3, updated_at = NOW()
                  WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'
                    AND lease_token = $5 AND lease_expires_at > NOW()
                  RETURNING *",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &error_label,
                    &TRANSIENT_RETRY_MILLISECONDS,
                    &lease_token,
                ],
            )
            .await?
            .ok_or_else(stale_lease_error)?;
        let updated = pipeline_run_from_row(&row)?;
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn list_settlements(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> Result<Vec<PipelineSettlementRecord>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT *, atomic_units::TEXT AS atomic_units_text
                   FROM pipeline_run_settlements
                  WHERE tenant_id = $1 AND run_id = $2
                  ORDER BY instrument_id",
                &[&tenant_id, &run_id],
            )
            .await?;
        tx.commit().await?;
        rows.iter().map(pipeline_settlement_from_row).collect()
    }
}

async fn insert_outcome(
    tx: &Transaction<'_>,
    tenant_id: &str,
    run_id: Uuid,
    trace_id: Uuid,
    bundle_id: &str,
    outcome_id: Uuid,
    outcome: StoredPhaseResult,
) -> Result<(), DatabaseError> {
    let schema = SchemaRef::pipeline_v1();
    tx.execute(
        "INSERT INTO phase_outcomes (
            tenant_id, outcome_id, run_id, trace_id, phase, bundle_id,
            outcome_schema_id, outcome_schema_version, decision, evidence, evaluation
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
        &[
            &tenant_id,
            &outcome_id,
            &run_id,
            &trace_id,
            &phase_as_db(Some(outcome.phase)),
            &bundle_id,
            &schema.id,
            &(schema.version as i32),
            &outcome.decision,
            &outcome.evidence,
            &outcome.evaluation,
        ],
    )
    .await?;
    Ok(())
}

async fn load_bundle_from_transaction(
    tx: &Transaction<'_>,
    tenant_id: &str,
    bundle_id: &str,
) -> Result<Option<BundlePackage>, DatabaseError> {
    let row = tx
        .query_opt(
            "SELECT package FROM pipeline_bundle_packages
             WHERE tenant_id = $1 AND bundle_id = $2",
            &[&tenant_id, &bundle_id],
        )
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let package = serde_json::from_value::<BundlePackage>(row.get("package"))
        .map_err(|_| DatabaseError::Serialization(PIPELINE_BUNDLE_INVALID_LABEL.to_string()))?;
    package
        .validate()
        .map_err(|_| DatabaseError::Serialization(PIPELINE_BUNDLE_INVALID_LABEL.to_string()))?;
    if package.bundle_id != bundle_id {
        return Err(DatabaseError::Serialization(
            PIPELINE_BUNDLE_INVALID_LABEL.to_string(),
        ));
    }
    Ok(Some(package))
}

fn required_lease_token(run: &PipelineRunRecord) -> Result<Uuid, DatabaseError> {
    run.lease_token.ok_or_else(stale_lease_error)
}

fn stale_lease_error() -> DatabaseError {
    DatabaseError::Constraint("pipeline lease is stale".to_string())
}

fn pipeline_run_from_row(row: &Row) -> Result<PipelineRunRecord, DatabaseError> {
    let attempt_count: i32 = row.get("attempt_count");
    let max_attempts: i32 = row.get("max_attempts");
    Ok(PipelineRunRecord {
        tenant_id: row.get("tenant_id"),
        run_id: row.get("run_id"),
        submission_id: row.get("submission_id"),
        trace_id: row.get("trace_id"),
        bundle_id: row.get("bundle_id"),
        request_idempotency_key: row.get("request_idempotency_key"),
        request_content_hash: row.get("request_content_hash"),
        source_object_ref_id: row.get("source_object_ref_id"),
        approved_revision_id: row.get("approved_revision_id"),
        next_phase: phase_from_db(row.get("next_phase"))?,
        state: PipelineRunState::from_db(row.get("state"))?,
        lease_token: row.get("lease_token"),
        lease_expires_at: row.get("lease_expires_at"),
        attempt_count: u32::try_from(attempt_count).map_err(|_| {
            DatabaseError::Serialization("invalid pipeline attempt count".to_string())
        })?,
        max_attempts: u32::try_from(max_attempts).map_err(|_| {
            DatabaseError::Serialization("invalid pipeline maximum attempts".to_string())
        })?,
        next_attempt_at: row.get("next_attempt_at"),
        phase_started_at: row.get("phase_started_at"),
        last_error_label: row.get("last_error_label"),
        index_membership: row.get("index_membership"),
        index_command_ref: row.get("index_command_ref"),
        index_command_hash: row.get("index_command_hash"),
        index_write_state: row.get("index_write_state"),
        admission_decision: row.get("admission_decision"),
        approved_object_ref_id: row.get("approved_object_ref_id"),
        approved_content_hash: row.get("approved_content_hash"),
        score_neighbor_ref: row.get("score_neighbor_ref"),
        score_neighbor_hash: row.get("score_neighbor_hash"),
        settle_selection_hash: row.get("settle_selection_hash"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn pipeline_settlement_from_row(row: &Row) -> Result<PipelineSettlementRecord, DatabaseError> {
    let atomic_units = row
        .get::<_, String>("atomic_units_text")
        .parse::<AtomicUnits>()
        .map_err(|_| DatabaseError::Serialization("invalid settlement atomic units".to_string()))?;
    let attempt_count = u32::try_from(row.get::<_, i32>("attempt_count")).map_err(|_| {
        DatabaseError::Serialization("invalid settlement attempt count".to_string())
    })?;
    let max_attempts = u32::try_from(row.get::<_, i32>("max_attempts")).map_err(|_| {
        DatabaseError::Serialization("invalid settlement maximum attempts".to_string())
    })?;
    Ok(PipelineSettlementRecord {
        tenant_id: row.get("tenant_id"),
        run_id: row.get("run_id"),
        instrument_id: row.get("instrument_id"),
        atomic_units,
        operation_ref_hash: row.get("operation_ref_hash"),
        result_ref_hash: row.get("result_ref_hash"),
        operation_state: row.get("operation_state"),
        credit_event_id: row.get("credit_event_id"),
        settlement_batch_id: row.get("settlement_batch_id"),
        payout_rail: row.get("payout_rail"),
        payout_state: row.get("payout_state"),
        attempt_count,
        max_attempts,
        last_error_label: row.get("last_error_label"),
    })
}

fn phase_outcome_from_row(row: &Row) -> Result<PhaseOutcomeRecord, DatabaseError> {
    let version: i32 = row.get("outcome_schema_version");
    let schema_id: String = row.get("outcome_schema_id");
    let version = u32::try_from(version)
        .map_err(|_| DatabaseError::Serialization("invalid outcome schema version".to_string()))?;
    if schema_id != PIPELINE_OUTCOME_SCHEMA_ID || version != PIPELINE_OUTCOME_SCHEMA_VERSION {
        return Err(DatabaseError::Serialization(
            "unsupported required pipeline outcome schema".to_string(),
        ));
    }
    let phase = phase_from_db(row.get("phase"))?
        .ok_or_else(|| DatabaseError::Serialization("outcome phase cannot be none".to_string()))?;
    let decision = row.get("decision");
    let evidence = row.get("evidence");
    let evaluation = row.get("evaluation");
    validate_outcome_payload(phase, &decision, &evidence, &evaluation)?;
    Ok(PhaseOutcomeRecord {
        tenant_id: row.get("tenant_id"),
        outcome_id: row.get("outcome_id"),
        run_id: row.get("run_id"),
        trace_id: row.get("trace_id"),
        phase,
        bundle_id: row.get("bundle_id"),
        outcome_schema: SchemaRef {
            id: schema_id,
            version,
        },
        decision,
        evidence,
        evaluation,
        recorded_at: row.get("recorded_at"),
    })
}

fn validate_outcome_payload(
    phase: Phase,
    decision: &serde_json::Value,
    evidence: &serde_json::Value,
    evaluation: &serde_json::Value,
) -> Result<(), DatabaseError> {
    fn decode<T: serde::de::DeserializeOwned>(
        value: &serde_json::Value,
    ) -> Result<T, DatabaseError> {
        serde_json::from_value(value.clone()).map_err(|_| {
            DatabaseError::Serialization("malformed pipeline outcome payload".to_string())
        })
    }

    match phase {
        Phase::Admission => {
            let _: AdmissionDecision = decode(decision)?;
            let _: AdmissionEvidence = decode(evidence)?;
            let _: AdmissionEvaluation = decode(evaluation)?;
        }
        Phase::Review => {
            let _: ReviewDecision = decode(decision)?;
            let _: ReviewEvidence = decode(evidence)?;
            let _: ReviewEvaluation = decode(evaluation)?;
        }
        Phase::Score => {
            let _: ScoreDecision = decode(decision)?;
            let _: ScoreEvidence = decode(evidence)?;
            let _: ScoreEvaluation = decode(evaluation)?;
        }
        Phase::Settle => {
            let _: SettleDecision = decode(decision)?;
            let _: SettleEvidence = decode(evidence)?;
            let _: SettleEvaluation = decode(evaluation)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Expected value computed outside Rust: SHA-256("tenant-a"), first 16 bytes.
    #[test]
    fn tenant_storage_ref_uses_the_ingest_derivation() {
        assert_eq!(
            pipeline_tenant_storage_ref("tenant-a").as_str(),
            "tenant_sha256:80a707af7dc77ee1228f9127180f3964"
        );
    }
}
