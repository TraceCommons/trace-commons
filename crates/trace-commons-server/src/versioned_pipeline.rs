//! Isolated Phase 2 implementation of the versioned four-phase pipeline.
//!
//! This module is local/test-only until later phases add production policies
//! and index and credit operation recovery.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use base64::Engine as _;
use chrono::{DateTime, Duration, Utc};
use deadpool_postgres::Transaction;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_postgres::Row;
use trace_commons_gate_api::pipeline::{
    AdmissionDecision, AdmissionEvaluation, AdmissionEvidence, AdmissionInput, AdmissionPolicy,
    BUNDLE_MANIFEST_FORMAT_VERSION, BundleManifest, BundlePackage, IndexMembershipDecision,
    Microcredits, PIPELINE_OUTCOME_SCHEMA_ID, PIPELINE_OUTCOME_SCHEMA_VERSION, Phase, PhaseResult,
    PolicyError, PolicyRef, ReviewDecision, ReviewEvaluation, ReviewEvidence, ReviewInput,
    ReviewPolicy, SchemaRef, ScoreDecision, ScoreEvaluation, ScoreEvidence, ScoreInput,
    ScorePolicy, SettleDecision, SettleEvaluation, SettleEvidence, SettleInput, SettlePolicy,
};
use uuid::Uuid;

use crate::db::postgres::PgBackend;
use crate::error::DatabaseError;
use crate::trace_artifact_store::{
    EncryptedTraceArtifactReceipt, TraceArtifactKind, TraceArtifactStore,
};
use crate::trace_corpus_storage::{
    TraceCorpusStatus, TraceCorpusStore, TraceObjectArtifactKind, TraceObjectRefWrite,
    TraceSubmissionWrite,
};
use trace_commons_protocol::trace_contribution::TraceContributionEnvelope;

pub const MINIMAL_PIPELINE_BUNDLE_LABEL: &str = "minimal-local-v1";
pub const PIPELINE_OPERATIONAL_ERROR_LABEL: &str = "minimal_policy_failed";
pub const PIPELINE_ATTEMPTS_EXHAUSTED_LABEL: &str = "attempts_exhausted";
pub const PIPELINE_BUNDLE_MISSING_LABEL: &str = "bundle_package_missing";
pub const PIPELINE_BUNDLE_INVALID_LABEL: &str = "bundle_package_invalid";
pub const PIPELINE_POLICY_NOT_RUNNABLE_LABEL: &str = "bundle_policy_not_runnable";
const DEFAULT_LEASE_SECONDS: i64 = 30;
const DEFAULT_RETRY_MILLISECONDS: i64 = 50;
const INJECTED_PIPELINE_CRASH: &str = "injected_pipeline_crash";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineCrashPoint {
    AfterArtifactStorage,
    AfterAdmissionWork,
    AfterReviewWork,
    AfterReviewCommit,
    AfterScoreWork,
}

fn sha256_prefixed(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

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

    fn from_db(value: &str) -> Result<Self, DatabaseError> {
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

fn phase_from_db(value: &str) -> Result<Option<Phase>, DatabaseError> {
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

#[derive(Debug, Clone)]
pub struct NewPipelineRun {
    pub tenant_id: String,
    pub run_id: Uuid,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub bundle_id: String,
    pub request_idempotency_key: String,
    pub request_content_hash: String,
    pub source_object_ref_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineReceiptArtifactRecord {
    #[serde(skip_serializing, default)]
    pub tenant_id: String,
    pub run_id: Uuid,
    pub request_idempotency_key: String,
    pub request_content_hash: String,
    pub object_key: Option<String>,
    pub ciphertext_sha256: Option<String>,
    pub cleanup_after: DateTime<Utc>,
    pub staged_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineLeaseClaim {
    pub tenant_id: String,
    pub run_id: Uuid,
    pub lease_token: Uuid,
    pub lease_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PipelineReceiptResult {
    Created(PipelineRunRecord),
    Replayed(PipelineRunRecord),
    ContentConflict,
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

    pub async fn stage_receipt_artifact(
        &self,
        run: &NewPipelineRun,
        receipt: Option<&EncryptedTraceArtifactReceipt>,
    ) -> Result<(), DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let object_key = receipt.map(|value| value.object_key.as_str());
        let ciphertext_sha256 = receipt.map(|value| value.ciphertext_sha256.as_str());
        tx.execute(
            "INSERT INTO pipeline_receipt_artifacts (
                tenant_id, run_id, request_idempotency_key, request_content_hash,
                object_key, ciphertext_sha256
             ) VALUES ($1,$2,$3,$4,$5,$6)
             ON CONFLICT (tenant_id, run_id) DO UPDATE
             SET object_key = COALESCE(EXCLUDED.object_key, pipeline_receipt_artifacts.object_key),
                 ciphertext_sha256 = COALESCE(
                    EXCLUDED.ciphertext_sha256,
                    pipeline_receipt_artifacts.ciphertext_sha256
                 )",
            &[
                &run.tenant_id,
                &run.run_id,
                &run.request_idempotency_key,
                &run.request_content_hash,
                &object_key,
                &ciphertext_sha256,
            ],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_cleanup_orphans(
        &self,
        tenant_id: &str,
        before: DateTime<Utc>,
    ) -> Result<Vec<PipelineReceiptArtifactRecord>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT tenant_id, run_id, request_idempotency_key, request_content_hash,
                        object_key, ciphertext_sha256, cleanup_after, staged_at
                 FROM pipeline_receipt_artifacts
                 WHERE tenant_id = $1 AND state = 'staged' AND cleanup_after <= $2
                 ORDER BY staged_at ASC",
                &[&tenant_id, &before],
            )
            .await?;
        tx.commit().await?;
        Ok(rows
            .into_iter()
            .map(|row| PipelineReceiptArtifactRecord {
                tenant_id: row.get("tenant_id"),
                run_id: row.get("run_id"),
                request_idempotency_key: row.get("request_idempotency_key"),
                request_content_hash: row.get("request_content_hash"),
                object_key: row.get("object_key"),
                ciphertext_sha256: row.get("ciphertext_sha256"),
                cleanup_after: row.get("cleanup_after"),
                staged_at: row.get("staged_at"),
            })
            .collect())
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

    pub async fn get_run_by_request_key(
        &self,
        tenant_id: &str,
        request_idempotency_key: &str,
    ) -> Result<Option<PipelineRunRecord>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT * FROM pipeline_runs
                 WHERE tenant_id = $1 AND request_idempotency_key = $2",
                &[&tenant_id, &request_idempotency_key],
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

    pub async fn claim_next_cross_tenant(
        &self,
        lease_duration: Duration,
    ) -> Result<Option<PipelineLeaseClaim>, DatabaseError> {
        if lease_duration <= Duration::zero() || lease_duration > Duration::minutes(5) {
            return Err(DatabaseError::Constraint(
                "pipeline lease duration is invalid".to_string(),
            ));
        }
        let lease_seconds = i32::try_from(lease_duration.num_seconds())
            .map_err(|_| DatabaseError::Constraint("pipeline lease duration is invalid".into()))?;
        let lease_token = Uuid::new_v4();
        let client = self.backend.trace_pool().get().await?;
        let row = client
            .query_opt(
                "SELECT tenant_id, run_id, lease_token, lease_expires_at
                 FROM claim_pipeline_run($1, $2)",
                &[&lease_token, &lease_seconds],
            )
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(PipelineLeaseClaim {
            tenant_id: row.get("tenant_id"),
            run_id: row.get("run_id"),
            lease_token: row.get("lease_token"),
            lease_expires_at: row.get("lease_expires_at"),
        }))
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
            let revision_id = approved_revision_id.ok_or_else(|| {
                DatabaseError::Constraint("approved Review requires a revision".to_string())
            })?;
            tx.execute(
                "INSERT INTO trace_derived_records (
                    tenant_id, derived_id, submission_id, trace_id, status,
                    worker_kind, worker_version, input_object_ref_id, input_hash,
                    output_object_ref_id, summary_model
                 ) VALUES ($1,$2,$3,$4,'current','summary',$5,$6,$7,$6,$5)
                 ON CONFLICT (tenant_id, derived_id) DO NOTHING",
                &[
                    &run.tenant_id,
                    &revision_id,
                    &run.submission_id,
                    &run.trace_id,
                    &MINIMAL_PIPELINE_BUNDLE_LABEL,
                    &run.source_object_ref_id,
                    &run.request_content_hash,
                ],
            )
            .await?;
            tx.execute(
                "UPDATE trace_submissions
                 SET status = 'accepted', reviewed_at = NOW(), updated_at = NOW()
                 WHERE tenant_id = $1 AND submission_id = $2",
                &[&run.tenant_id, &run.submission_id],
            )
            .await?;
        }
        insert_outcome(
            &tx,
            &run.tenant_id,
            run.run_id,
            run.trace_id,
            &run.bundle_id,
            outcome,
        )
        .await?;
        let terminal = next_phase.is_none();
        let state = if terminal {
            PipelineRunState::Complete
        } else {
            PipelineRunState::Pending
        };
        let index_membership = if terminal { "excluded" } else { "undecided" };
        let row = tx
            .query_one(
                "UPDATE pipeline_runs
                 SET next_phase = $3, state = $4,
                     approved_revision_id = COALESCE($5, approved_revision_id),
                     index_membership = $6, lease_token = NULL, lease_expires_at = NULL,
                     next_attempt_at = NOW(), phase_started_at = NOW(), updated_at = NOW()
                 WHERE tenant_id = $1 AND run_id = $2
                   AND lease_token = $7 AND lease_expires_at > NOW()
                 RETURNING *",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &phase_as_db(next_phase),
                    &state.as_db(),
                    &approved_revision_id,
                    &index_membership,
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
}

async fn insert_outcome(
    tx: &Transaction<'_>,
    tenant_id: &str,
    run_id: Uuid,
    trace_id: Uuid,
    bundle_id: &str,
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
            &Uuid::new_v4(),
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

async fn insert_receipt_records(
    tx: &Transaction<'_>,
    run: &NewPipelineRun,
    submission: &TraceSubmissionWrite,
    object_ref: &TraceObjectRefWrite,
    outcome: StoredPhaseResult,
) -> Result<(), DatabaseError> {
    let consent_scopes = serde_json::to_value(&submission.consent_scopes).map_err(|_| {
        DatabaseError::Serialization("trace consent scopes encode failed".to_string())
    })?;
    let allowed_uses = serde_json::to_value(&submission.allowed_uses).map_err(|_| {
        DatabaseError::Serialization("trace allowed uses encode failed".to_string())
    })?;
    let redaction_counts = serde_json::to_value(&submission.redaction_counts).map_err(|_| {
        DatabaseError::Serialization("trace redaction counts encode failed".to_string())
    })?;
    let inserted = tx
        .execute(
            "INSERT INTO trace_submissions (
                tenant_id, submission_id, trace_id, auth_principal_ref, contributor_pseudonym,
                submitted_tenant_scope_ref, schema_version, consent_policy_version,
                consent_scopes, allowed_uses, retention_policy_id, status, privacy_risk,
                redaction_pipeline_version, redaction_hash, redaction_counts,
                canonical_summary_hash, submission_score, credit_points_pending,
                credit_points_final, expires_at
             ) VALUES (
                $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,'received',$12,$13,$14,$15,
                $16,$17,$18,$19,$20
             )
             ON CONFLICT (tenant_id, submission_id) DO NOTHING",
            &[
                &submission.tenant_id,
                &submission.submission_id,
                &submission.trace_id,
                &submission.auth_principal_ref,
                &submission.contributor_pseudonym,
                &submission.submitted_tenant_scope_ref,
                &submission.schema_version,
                &submission.consent_policy_version,
                &consent_scopes,
                &allowed_uses,
                &submission.retention_policy_id,
                &submission.privacy_risk,
                &submission.redaction_pipeline_version,
                &submission.redaction_hash,
                &redaction_counts,
                &submission.canonical_summary_hash,
                &submission.submission_score,
                &submission.credit_points_pending,
                &submission.credit_points_final,
                &submission.expires_at,
            ],
        )
        .await?;
    if inserted != 1 {
        return Err(DatabaseError::Constraint(
            "submission identity is already bound to another receipt".to_string(),
        ));
    }
    tx.execute(
        "INSERT INTO trace_object_refs (
            tenant_id, submission_id, object_ref_id, artifact_kind, object_store,
            object_key, content_sha256, encryption_key_ref, size_bytes, compression,
            created_by_job_id
         ) VALUES ($1,$2,$3,'submitted_envelope',$4,$5,$6,$7,$8,$9,$10)",
        &[
            &object_ref.tenant_id,
            &object_ref.submission_id,
            &object_ref.object_ref_id,
            &object_ref.object_store,
            &object_ref.object_key,
            &object_ref.content_sha256,
            &object_ref.encryption_key_ref,
            &object_ref.size_bytes,
            &object_ref.compression,
            &object_ref.created_by_job_id,
        ],
    )
    .await?;
    tx.execute(
        "INSERT INTO pipeline_runs (
            tenant_id, run_id, submission_id, trace_id, bundle_id,
            request_idempotency_key, request_content_hash, source_object_ref_id,
            next_phase, state
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'review','pending')",
        &[
            &run.tenant_id,
            &run.run_id,
            &run.submission_id,
            &run.trace_id,
            &run.bundle_id,
            &run.request_idempotency_key,
            &run.request_content_hash,
            &run.source_object_ref_id,
        ],
    )
    .await?;
    insert_outcome(
        tx,
        &run.tenant_id,
        run.run_id,
        run.trace_id,
        &run.bundle_id,
        outcome,
    )
    .await?;
    let committed = tx
        .execute(
            "UPDATE pipeline_receipt_artifacts
             SET state = 'committed', committed_at = NOW()
             WHERE tenant_id = $1 AND run_id = $2
               AND request_idempotency_key = $3
               AND request_content_hash = $4
               AND object_key = $5
               AND ciphertext_sha256 = $6
               AND state = 'staged'",
            &[
                &run.tenant_id,
                &run.run_id,
                &run.request_idempotency_key,
                &run.request_content_hash,
                &object_ref.object_key,
                &object_ref.content_sha256.strip_prefix("sha256:"),
            ],
        )
        .await?;
    if committed != 1 {
        return Err(DatabaseError::Constraint(
            "receipt artifact staging record is missing".to_string(),
        ));
    }
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
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
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

pub struct MinimalAdmissionPolicy;

#[async_trait]
impl AdmissionPolicy for MinimalAdmissionPolicy {
    async fn execute(
        &self,
        input: &AdmissionInput,
    ) -> Result<PhaseResult<AdmissionDecision, AdmissionEvidence, AdmissionEvaluation>, PolicyError>
    {
        if !input.authenticated || !input.authority_valid {
            return Err(PolicyError::new("authority_missing").expect("static safe label"));
        }
        if input.schema_version != "ironclaw.trace_contribution.v1" {
            return Err(PolicyError::new("schema_invalid").expect("static safe label"));
        }
        Ok(PhaseResult {
            decision: AdmissionDecision::Admit,
            evidence: AdmissionEvidence {
                request_content_hash: input.request_content_hash.clone(),
                schema_valid: true,
                authority_valid: true,
            },
            evaluation: AdmissionEvaluation {
                rule_id: "minimal_admission_v1".to_string(),
            },
        })
    }
}

pub struct MinimalReviewPolicy;

#[async_trait]
impl ReviewPolicy for MinimalReviewPolicy {
    async fn execute(
        &self,
        input: &ReviewInput,
    ) -> Result<PhaseResult<ReviewDecision, ReviewEvidence, ReviewEvaluation>, PolicyError> {
        let result_hash = sha256_prefixed(&input.source_artifact);
        if result_hash != input.source_content_hash {
            return Err(PolicyError::new("source_hash_mismatch").expect("static safe label"));
        }
        let registry_revision_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("tracecommons:pipeline-review:{}", input.run_id).as_bytes(),
        );
        Ok(PhaseResult {
            decision: ReviewDecision::Approved {
                registry_revision_id,
            },
            evidence: ReviewEvidence {
                source_content_hash: input.source_content_hash.clone(),
                result_content_hash: result_hash,
                content_changed: false,
            },
            evaluation: ReviewEvaluation {
                rule_id: "minimal_review_passthrough_v1".to_string(),
            },
        })
    }
}

pub struct MinimalScorePolicy;

#[async_trait]
impl ScorePolicy for MinimalScorePolicy {
    async fn execute(
        &self,
        _input: &ScoreInput,
    ) -> Result<PhaseResult<ScoreDecision, ScoreEvidence, ScoreEvaluation>, PolicyError> {
        Ok(PhaseResult {
            decision: ScoreDecision {
                credit_microcredits: Microcredits::ZERO,
            },
            evidence: ScoreEvidence {
                fixed_credit_microcredits: Microcredits::ZERO,
            },
            evaluation: ScoreEvaluation {
                rule_id: "minimal_fixed_zero_v1".to_string(),
                credit_microcredits: Microcredits::ZERO,
            },
        })
    }
}

pub struct MinimalSettlePolicy;

#[async_trait]
impl SettlePolicy for MinimalSettlePolicy {
    async fn execute(
        &self,
        input: &SettleInput,
    ) -> Result<PhaseResult<SettleDecision, SettleEvidence, SettleEvaluation>, PolicyError> {
        if input.score.credit_microcredits != Microcredits::ZERO {
            return Err(PolicyError::new("credit_operation_required").expect("static safe label"));
        }
        Ok(PhaseResult {
            decision: SettleDecision {
                index_membership: IndexMembershipDecision::Exclude {
                    reason: trace_commons_gate_api::pipeline::ReasonCode::new(
                        "minimal_bundle_exclusion",
                    )
                    .expect("static safe label"),
                },
                credit_microcredits_finalized: Microcredits::ZERO,
                settlement_batch_ref_hash: None,
            },
            evidence: SettleEvidence {
                index_operation_required: false,
                credit_operation_required: false,
            },
            evaluation: SettleEvaluation {
                rule_id: "minimal_settle_exclude_v1".to_string(),
            },
        })
    }
}

pub struct MinimalPolicyBundle {
    pub package: BundlePackage,
    pub admission: Arc<dyn AdmissionPolicy>,
    pub review: Arc<dyn ReviewPolicy>,
    pub score: Arc<dyn ScorePolicy>,
    pub settle: Arc<dyn SettlePolicy>,
}

impl MinimalPolicyBundle {
    pub fn build() -> anyhow::Result<Self> {
        Self::build_variant("minimal-local-test-only-v1")
    }

    pub fn build_variant(configuration_label: &str) -> anyhow::Result<Self> {
        let specifications = [
            ("admission", b"minimal-admission-policy-v1".as_slice()),
            ("review", b"minimal-review-policy-v1".as_slice()),
            ("score", b"minimal-score-policy-v1".as_slice()),
            ("settle", b"minimal-settle-policy-v1".as_slice()),
        ];
        anyhow::ensure!(
            !configuration_label.trim().is_empty(),
            "minimal bundle configuration label cannot be empty"
        );
        let configuration = configuration_label.as_bytes();
        let config_hash = sha256_prefixed(configuration);
        let policy_ref = |(name, bytes): (&str, &[u8])| PolicyRef {
            policy_id: format!("trace_commons.{name}.minimal"),
            implementation_id: format!("trace_commons.{name}.minimal.v1"),
            code_artifact_hash: sha256_prefixed(bytes),
            configuration_hash: config_hash.clone(),
            data_artifact_hashes: Vec::new(),
            projection_ids: Vec::new(),
        };
        let manifest = BundleManifest {
            format_version: BUNDLE_MANIFEST_FORMAT_VERSION,
            admission: policy_ref(specifications[0]),
            review: policy_ref(specifications[1]),
            score: policy_ref(specifications[2]),
            settle: policy_ref(specifications[3]),
        };
        let mut artifacts = BTreeMap::new();
        artifacts.insert(config_hash, configuration.to_vec());
        for (_, bytes) in specifications {
            artifacts.insert(sha256_prefixed(bytes), bytes.to_vec());
        }
        let package = BundlePackage {
            bundle_id: manifest.bundle_id()?,
            manifest,
            artifacts,
        };
        package.validate()?;
        Self::from_package(package)
    }

    fn from_package(package: BundlePackage) -> anyhow::Result<Self> {
        package.validate()?;
        let implementations = [
            &package.manifest.admission.implementation_id,
            &package.manifest.review.implementation_id,
            &package.manifest.score.implementation_id,
            &package.manifest.settle.implementation_id,
        ];
        anyhow::ensure!(
            implementations
                == [
                    "trace_commons.admission.minimal.v1",
                    "trace_commons.review.minimal.v1",
                    "trace_commons.score.minimal.v1",
                    "trace_commons.settle.minimal.v1",
                ],
            "bundle policy implementation is unavailable"
        );
        Ok(Self {
            package,
            admission: Arc::new(MinimalAdmissionPolicy),
            review: Arc::new(MinimalReviewPolicy),
            score: Arc::new(MinimalScorePolicy),
            settle: Arc::new(MinimalSettlePolicy),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineSubmitReceipt {
    pub run_id: Uuid,
    pub submission_id: Uuid,
    pub bundle_id: String,
    pub request_content_hash: String,
    pub replayed: bool,
    pub state: PipelineRunState,
    pub next_phase: Option<Phase>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PipelineInspection {
    pub run: PipelineRunRecord,
    pub outcomes: Vec<PhaseOutcomeRecord>,
}

pub struct PipelineService {
    backend: Arc<PgBackend>,
    store: PgPipelineStore,
    artifact_store: Arc<dyn TraceArtifactStore>,
    default_bundle: MinimalPolicyBundle,
    fail_phase: Option<Phase>,
    crash_point: Option<PipelineCrashPoint>,
    crash_pending: AtomicBool,
}

impl PipelineService {
    pub fn new(
        backend: Arc<PgBackend>,
        artifact_store: Arc<dyn TraceArtifactStore>,
        fail_phase: Option<Phase>,
    ) -> anyhow::Result<Self> {
        Self::new_with_crash_point(backend, artifact_store, fail_phase, None)
    }

    pub fn new_with_crash_point(
        backend: Arc<PgBackend>,
        artifact_store: Arc<dyn TraceArtifactStore>,
        fail_phase: Option<Phase>,
        crash_point: Option<PipelineCrashPoint>,
    ) -> anyhow::Result<Self> {
        let bundle = MinimalPolicyBundle::build()?;
        bundle.package.validate()?;
        Ok(Self {
            store: PgPipelineStore::new(backend.clone()),
            backend,
            artifact_store,
            default_bundle: bundle,
            fail_phase,
            crash_point,
            crash_pending: AtomicBool::new(crash_point.is_some()),
        })
    }

    pub fn bundle_id(&self) -> &str {
        &self.default_bundle.package.bundle_id
    }

    pub async fn register_bundle(
        &self,
        tenant_id: &str,
        package: &BundlePackage,
    ) -> anyhow::Result<()> {
        self.store.register_bundle(tenant_id, package).await?;
        Ok(())
    }

    pub async fn activate_bundle(&self, tenant_id: &str, bundle_id: &str) -> anyhow::Result<()> {
        self.store.activate_bundle(tenant_id, bundle_id).await?;
        Ok(())
    }

    pub async fn active_bundle_id(&self, tenant_id: &str) -> anyhow::Result<Option<String>> {
        Ok(self.store.active_bundle_id(tenant_id).await?)
    }

    pub async fn list_cleanup_orphans(
        &self,
        tenant_id: &str,
        before: DateTime<Utc>,
    ) -> anyhow::Result<Vec<PipelineReceiptArtifactRecord>> {
        Ok(self.store.list_cleanup_orphans(tenant_id, before).await?)
    }

    async fn ensure_default_bundle(&self, tenant_id: &str) -> anyhow::Result<()> {
        self.store
            .register_bundle(tenant_id, &self.default_bundle.package)
            .await?;
        self.store
            .activate_bundle_if_none(tenant_id, self.bundle_id())
            .await?;
        Ok(())
    }

    fn inject_crash(&self, point: PipelineCrashPoint) -> anyhow::Result<()> {
        if self.crash_point == Some(point) && self.crash_pending.swap(false, Ordering::SeqCst) {
            anyhow::bail!(INJECTED_PIPELINE_CRASH);
        }
        Ok(())
    }

    pub async fn submit(
        &self,
        tenant_id: &str,
        actor_principal_ref: &str,
        request_idempotency_key: &str,
        request_bytes: &[u8],
    ) -> anyhow::Result<PipelineReceiptResult> {
        anyhow::ensure!(
            !request_idempotency_key.trim().is_empty() && request_idempotency_key.len() <= 200,
            "invalid idempotency key"
        );
        let request_content_hash = sha256_prefixed(request_bytes);
        let request_idempotency_key_hash = sha256_prefixed(request_idempotency_key.as_bytes());
        let envelope: TraceContributionEnvelope = serde_json::from_slice(request_bytes)
            .map_err(|_| anyhow::anyhow!("invalid envelope"))?;
        self.ensure_default_bundle(tenant_id).await?;
        let run_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("tracecommons:pipeline-run:{tenant_id}:{request_idempotency_key}").as_bytes(),
        );
        let object_ref_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("tracecommons:pipeline-source-object:{run_id}").as_bytes(),
        );

        let mut client = self.backend.trace_pool().get().await?;
        let tx = PgPipelineStore::tenant_transaction(&mut client, tenant_id).await?;
        let receipt_lock = format!("pipeline-receipt:{tenant_id}:{request_idempotency_key_hash}");
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
            &[&receipt_lock],
        )
        .await?;
        if let Some(row) = tx
            .query_opt(
                "SELECT * FROM pipeline_runs
                 WHERE tenant_id = $1 AND request_idempotency_key = $2",
                &[&tenant_id, &request_idempotency_key_hash],
            )
            .await?
        {
            let existing = pipeline_run_from_row(&row)?;
            tx.commit().await?;
            return if existing.request_content_hash == request_content_hash {
                Ok(PipelineReceiptResult::Replayed(existing))
            } else {
                Ok(PipelineReceiptResult::ContentConflict)
            };
        }
        if let Some(staged_hash) = tx
            .query_opt(
                "SELECT request_content_hash
                 FROM pipeline_receipt_artifacts
                 WHERE tenant_id = $1 AND request_idempotency_key = $2",
                &[&tenant_id, &request_idempotency_key_hash],
            )
            .await?
            .map(|row| row.get::<_, String>("request_content_hash"))
            && staged_hash != request_content_hash
        {
            tx.commit().await?;
            return Ok(PipelineReceiptResult::ContentConflict);
        }
        let bundle_id: String = tx
            .query_opt(
                "SELECT bundle_id FROM pipeline_active_bundles WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await?
            .ok_or_else(|| anyhow::anyhow!(PIPELINE_BUNDLE_MISSING_LABEL))?
            .get("bundle_id");
        let package = load_bundle_from_transaction(&tx, tenant_id, &bundle_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!(PIPELINE_BUNDLE_MISSING_LABEL))?;
        let runnable = tx
            .query_opt(
                "SELECT runnable FROM pipeline_bundle_policy_status
                 WHERE tenant_id = $1 AND bundle_id = $2 AND phase = 'admission'",
                &[&tenant_id, &bundle_id],
            )
            .await?
            .map(|row| row.get::<_, bool>("runnable"))
            .unwrap_or(false);
        anyhow::ensure!(runnable, "bound policy is not runnable");
        let bundle = MinimalPolicyBundle::from_package(package)
            .map_err(|_| anyhow::anyhow!(PIPELINE_BUNDLE_INVALID_LABEL))?;
        let run = NewPipelineRun {
            tenant_id: tenant_id.to_string(),
            run_id,
            submission_id: envelope.submission_id,
            trace_id: envelope.trace_id,
            bundle_id,
            request_idempotency_key: request_idempotency_key_hash,
            request_content_hash: request_content_hash.clone(),
            source_object_ref_id: object_ref_id,
        };
        self.store.stage_receipt_artifact(&run, None).await?;

        let tenant_storage_ref = tenant_storage_ref(tenant_id);
        let wrapper = serde_json::to_vec(&serde_json::json!({
            "schema": "trace_commons.pipeline_source_bytes.v1",
            "request_bytes_base64": base64::engine::general_purpose::STANDARD.encode(request_bytes),
        }))?;
        let artifact_receipt = self.artifact_store.put_serialized_json(
            &tenant_storage_ref,
            TraceArtifactKind::ContributionEnvelope,
            &run_id.to_string(),
            &wrapper,
        )?;
        self.store
            .stage_receipt_artifact(&run, Some(&artifact_receipt))
            .await?;
        self.inject_crash(PipelineCrashPoint::AfterArtifactStorage)?;

        let admission = bundle
            .admission
            .execute(&AdmissionInput {
                run_id,
                trace_id: envelope.trace_id,
                request_content_hash: request_content_hash.clone(),
                schema_version: envelope.schema_version.clone(),
                authenticated: true,
                authority_valid: true,
            })
            .await
            .map_err(|error| anyhow::anyhow!(error.label().to_string()))?;
        self.inject_crash(PipelineCrashPoint::AfterAdmissionWork)?;
        let stored = StoredPhaseResult::from_result(Phase::Admission, &admission)?;
        let submission = TraceSubmissionWrite {
            tenant_id: tenant_id.to_string(),
            submission_id: envelope.submission_id,
            trace_id: envelope.trace_id,
            auth_principal_ref: actor_principal_ref.to_string(),
            contributor_pseudonym: envelope.contributor.pseudonymous_contributor_id,
            submitted_tenant_scope_ref: None,
            schema_version: envelope.schema_version,
            consent_policy_version: envelope.consent.policy_version,
            consent_scopes: enum_strings(&envelope.consent.scopes)?,
            allowed_uses: enum_strings(&envelope.trace_card.allowed_uses)?,
            retention_policy_id: envelope.trace_card.retention_policy,
            status: TraceCorpusStatus::Received,
            privacy_risk: enum_string(&envelope.privacy.residual_pii_risk)?,
            redaction_pipeline_version: envelope.privacy.redaction_pipeline_version,
            redaction_counts: envelope.privacy.redaction_counts,
            redaction_hash: envelope.privacy.redaction_hash,
            canonical_summary_hash: None,
            submission_score: None,
            credit_points_pending: None,
            credit_points_final: None,
            expires_at: None,
        };
        let object_ref = TraceObjectRefWrite {
            object_ref_id,
            tenant_id: tenant_id.to_string(),
            submission_id: envelope.submission_id,
            artifact_kind: TraceObjectArtifactKind::SubmittedEnvelope,
            object_store: "pipeline_local_encrypted".to_string(),
            object_key: artifact_receipt.object_key,
            content_sha256: format!("sha256:{}", artifact_receipt.ciphertext_sha256),
            encryption_key_ref: format!("tenant:{tenant_storage_ref}"),
            size_bytes: i64::try_from(request_bytes.len()).unwrap_or(i64::MAX),
            compression: None,
            created_by_job_id: None,
        };
        insert_receipt_records(&tx, &run, &submission, &object_ref, stored).await?;
        let row = tx
            .query_one(
                "SELECT * FROM pipeline_runs WHERE tenant_id = $1 AND run_id = $2",
                &[&tenant_id, &run_id],
            )
            .await?;
        let created = pipeline_run_from_row(&row)?;
        tx.commit().await?;
        Ok(PipelineReceiptResult::Created(created))
    }

    pub async fn inspect(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> anyhow::Result<Option<PipelineInspection>> {
        let Some(run) = self.store.get_run(tenant_id, run_id).await? else {
            return Ok(None);
        };
        let outcomes = self.store.list_outcomes(tenant_id, run_id).await?;
        Ok(Some(PipelineInspection { run, outcomes }))
    }

    pub async fn process_one(
        &self,
        tenant_id: &str,
        stop_before: Option<Phase>,
    ) -> anyhow::Result<Option<PipelineRunRecord>> {
        let Some(run) = self.store.claim_next(tenant_id).await? else {
            return Ok(None);
        };
        if stop_before == run.next_phase {
            self.store.release_claim(&run).await?;
            return Ok(Some(run));
        }
        if self.fail_phase == run.next_phase {
            self.store
                .mark_failed(&run, PIPELINE_OPERATIONAL_ERROR_LABEL)
                .await?;
            return self
                .store
                .get_run(tenant_id, run.run_id)
                .await
                .map_err(Into::into);
        }
        let phase = run
            .next_phase
            .ok_or_else(|| anyhow::anyhow!("claimed run has no phase"))?;
        let bundle = match self.load_bound_bundle(&run, phase).await {
            Ok(bundle) => bundle,
            Err(label) if label == PIPELINE_POLICY_NOT_RUNNABLE_LABEL => {
                return Ok(Some(self.store.mark_retry(&run, &label).await?));
            }
            Err(label) => {
                self.store.mark_failed(&run, &label).await?;
                return self
                    .store
                    .get_run(tenant_id, run.run_id)
                    .await
                    .map_err(Into::into);
            }
        };
        match self.process_claimed(&run, &bundle).await {
            Ok(updated) => Ok(Some(updated)),
            Err(error) if error.to_string() == INJECTED_PIPELINE_CRASH => Err(error),
            Err(_) => Ok(Some(
                self.store
                    .mark_retry(&run, PIPELINE_OPERATIONAL_ERROR_LABEL)
                    .await?,
            )),
        }
    }

    async fn load_bound_bundle(
        &self,
        run: &PipelineRunRecord,
        phase: Phase,
    ) -> Result<MinimalPolicyBundle, String> {
        let package = self
            .store
            .load_bundle(&run.tenant_id, &run.bundle_id)
            .await
            .map_err(|_| PIPELINE_BUNDLE_INVALID_LABEL.to_string())?
            .ok_or_else(|| PIPELINE_BUNDLE_MISSING_LABEL.to_string())?;
        let runnable = self
            .store
            .policy_is_runnable(&run.tenant_id, &run.bundle_id, phase)
            .await
            .map_err(|_| PIPELINE_BUNDLE_INVALID_LABEL.to_string())?;
        if !runnable {
            return Err(PIPELINE_POLICY_NOT_RUNNABLE_LABEL.to_string());
        }
        MinimalPolicyBundle::from_package(package)
            .map_err(|_| PIPELINE_BUNDLE_INVALID_LABEL.to_string())
    }

    async fn process_claimed(
        &self,
        run: &PipelineRunRecord,
        bundle: &MinimalPolicyBundle,
    ) -> anyhow::Result<PipelineRunRecord> {
        let phase = run
            .next_phase
            .ok_or_else(|| anyhow::anyhow!("claimed run has no phase"))?;
        match phase {
            Phase::Admission => anyhow::bail!("Admission cannot run asynchronously"),
            Phase::Review => {
                let source_artifact = self.load_source_bytes(run).await?;
                let result = bundle
                    .review
                    .execute(&ReviewInput {
                        run_id: run.run_id,
                        trace_id: run.trace_id,
                        source_content_hash: run.request_content_hash.clone(),
                        source_artifact,
                    })
                    .await?;
                self.inject_crash(PipelineCrashPoint::AfterReviewWork)?;
                let revision_id = match result.decision {
                    ReviewDecision::Approved {
                        registry_revision_id,
                    } => Some(registry_revision_id),
                    ReviewDecision::Rejected { .. } => None,
                };
                let next = revision_id.map(|_| Phase::Score);
                let updated = self
                    .store
                    .commit_phase(
                        run,
                        StoredPhaseResult::from_result(Phase::Review, &result)?,
                        next,
                        revision_id,
                    )
                    .await
                    .map_err(anyhow::Error::from)?;
                self.inject_crash(PipelineCrashPoint::AfterReviewCommit)?;
                Ok(updated)
            }
            Phase::Score => {
                let revision_id = run
                    .approved_revision_id
                    .ok_or_else(|| anyhow::anyhow!("approved revision is missing"))?;
                let result = bundle
                    .score
                    .execute(&ScoreInput {
                        run_id: run.run_id,
                        trace_id: run.trace_id,
                        registry_revision_id: revision_id,
                        source_content_hash: run.request_content_hash.clone(),
                    })
                    .await?;
                self.inject_crash(PipelineCrashPoint::AfterScoreWork)?;
                self.store
                    .commit_phase(
                        run,
                        StoredPhaseResult::from_result(Phase::Score, &result)?,
                        Some(Phase::Settle),
                        None,
                    )
                    .await
                    .map_err(Into::into)
            }
            Phase::Settle => {
                let revision_id = run
                    .approved_revision_id
                    .ok_or_else(|| anyhow::anyhow!("approved revision is missing"))?;
                let score_outcome = self
                    .store
                    .list_outcomes(&run.tenant_id, run.run_id)
                    .await?
                    .into_iter()
                    .find(|outcome| outcome.phase == Phase::Score)
                    .ok_or_else(|| anyhow::anyhow!("Score outcome is missing"))?;
                let score = serde_json::from_value::<ScoreDecision>(score_outcome.decision)
                    .map_err(|_| anyhow::anyhow!("Score outcome is malformed"))?;
                let result = bundle
                    .settle
                    .execute(&SettleInput {
                        run_id: run.run_id,
                        trace_id: run.trace_id,
                        registry_revision_id: revision_id,
                        source_content_hash: run.request_content_hash.clone(),
                        score,
                    })
                    .await?;
                self.store
                    .commit_phase(
                        run,
                        StoredPhaseResult::from_result(Phase::Settle, &result)?,
                        None,
                        None,
                    )
                    .await
                    .map_err(Into::into)
            }
        }
    }

    async fn load_source_bytes(&self, run: &PipelineRunRecord) -> anyhow::Result<Vec<u8>> {
        let object_ref = self
            .backend
            .list_trace_object_refs(&run.tenant_id, run.submission_id)
            .await?
            .into_iter()
            .find(|object_ref| object_ref.object_ref_id == run.source_object_ref_id)
            .ok_or_else(|| anyhow::anyhow!("source artifact reference is missing"))?;
        let receipt = EncryptedTraceArtifactReceipt {
            tenant_storage_ref: tenant_storage_ref(&run.tenant_id),
            artifact_kind: TraceArtifactKind::ContributionEnvelope,
            object_key: object_ref.object_key,
            ciphertext_sha256: object_ref
                .content_sha256
                .strip_prefix("sha256:")
                .ok_or_else(|| anyhow::anyhow!("source artifact hash is malformed"))?
                .to_string(),
            encrypted_at: object_ref.created_at,
        };
        let wrapper = self
            .artifact_store
            .read_json(&receipt.tenant_storage_ref, &receipt)?;
        let encoded = wrapper
            .get("request_bytes_base64")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("source artifact payload is malformed"))?;
        let bytes = base64::engine::general_purpose::STANDARD.decode(encoded)?;
        anyhow::ensure!(
            sha256_prefixed(&bytes) == run.request_content_hash,
            "source artifact content hash mismatch"
        );
        Ok(bytes)
    }
}

fn tenant_storage_ref(tenant_id: &str) -> String {
    format!("tenant_sha256:{:x}", Sha256::digest(tenant_id.as_bytes()))
}

fn enum_string<T: Serialize>(value: &T) -> anyhow::Result<String> {
    serde_json::to_value(value)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("enum did not serialize as a string"))
}

fn enum_strings<T: Serialize>(values: &[T]) -> anyhow::Result<Vec<String>> {
    values.iter().map(enum_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_outcome_reader_rejects_malformed_required_payloads() {
        for phase in [Phase::Admission, Phase::Review, Phase::Score, Phase::Settle] {
            assert!(
                validate_outcome_payload(
                    phase,
                    &serde_json::json!({}),
                    &serde_json::json!({}),
                    &serde_json::json!({})
                )
                .is_err()
            );
        }
    }

    #[tokio::test]
    async fn minimal_policies_produce_explicit_zero_credit_completion() {
        let bundle = MinimalPolicyBundle::build().unwrap();
        bundle.package.validate().unwrap();
        assert_eq!(
            bundle.package.bundle_id,
            "sha256:bc448580ee3d883fea4955ffbc68c5a64c14c9c4e048d5861f50c31c50eb0a32"
        );
        let bytes = br#"{"schema_version":"ironclaw.trace_contribution.v1"}"#.to_vec();
        let hash = sha256_prefixed(&bytes);
        let run_id = Uuid::new_v4();
        let trace_id = Uuid::new_v4();
        let review = bundle
            .review
            .execute(&ReviewInput {
                run_id,
                trace_id,
                source_content_hash: hash.clone(),
                source_artifact: bytes,
            })
            .await
            .unwrap();
        let ReviewDecision::Approved {
            registry_revision_id,
        } = review.decision
        else {
            panic!("minimal review must approve");
        };
        let score = bundle
            .score
            .execute(&ScoreInput {
                run_id,
                trace_id,
                registry_revision_id,
                source_content_hash: hash.clone(),
            })
            .await
            .unwrap();
        let settle = bundle
            .settle
            .execute(&SettleInput {
                run_id,
                trace_id,
                registry_revision_id,
                source_content_hash: hash,
                score: score.decision,
            })
            .await
            .unwrap();
        assert_eq!(
            settle.decision.credit_microcredits_finalized,
            Microcredits::ZERO
        );
        assert!(settle.decision.settlement_batch_ref_hash.is_none());
        assert!(matches!(
            settle.decision.index_membership,
            IndexMembershipDecision::Exclude { .. }
        ));
    }
}
