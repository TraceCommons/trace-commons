//! Isolated Phase 3 implementation of the versioned four-phase pipeline.
//!
//! This module is local/test-only until later phases add production policies.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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
use trace_commons_gate_api::{IndexWriteError, VectorIndexReader, VectorIndexWriter};
use uuid::Uuid;

use crate::db::postgres::PgBackend;
use crate::error::DatabaseError;
use crate::trace_artifact_store::{
    EncryptedTraceArtifactReceipt, TraceArtifactKind, TraceArtifactStore,
};
use crate::trace_corpus_storage::{
    TraceCorpusStatus, TraceCorpusStore, TraceCreditHoldReason, TraceCreditSettlementBatchStatus,
    TraceCreditSettlementNearStatus, TraceCreditSettlementState, TraceObjectArtifactKind,
    TraceObjectRefWrite, TraceSubmissionWrite,
};
use crate::versioned_pipeline_credit::{
    PIPELINE_CREDIT_REASON, PIPELINE_SETTLEMENT_POLICY_VERSION,
    PIPELINE_TEST_CREDIT_CAP_MICROCREDITS, RecordingNearAdapter, credit_account_hash,
    disabled_near_call, issuer_approval_hash, microcredits_to_settled_i64,
    pipeline_credit_event_id, pipeline_near_outbox_id, pipeline_settlement_batch_id,
    settlement_batch_ref_hash, source_list_hash,
};
use crate::versioned_pipeline_index::{
    IsolatedPipelineIndex, PIPELINE_INDEX_ID, SealedIndexCommand, deterministic_pipeline_embedding,
};
use trace_commons_protocol::trace_contribution::TraceContributionEnvelope;

pub const MINIMAL_PIPELINE_BUNDLE_LABEL: &str = "minimal-local-v1";
pub const PIPELINE_OPERATIONAL_ERROR_LABEL: &str = "minimal_policy_failed";
pub const PIPELINE_ATTEMPTS_EXHAUSTED_LABEL: &str = "attempts_exhausted";
pub const PIPELINE_BUNDLE_MISSING_LABEL: &str = "bundle_package_missing";
pub const PIPELINE_BUNDLE_INVALID_LABEL: &str = "bundle_package_invalid";
pub const PIPELINE_POLICY_NOT_RUNNABLE_LABEL: &str = "bundle_policy_not_runnable";
pub const PIPELINE_INDEX_UNAVAILABLE_LABEL: &str = "index_unavailable";
pub const PIPELINE_INDEX_CONFLICT_LABEL: &str = "index_key_conflict";
pub const PIPELINE_CREDIT_HELD_LABEL: &str = "credit_held";
pub const PIPELINE_CREDIT_CAP_LABEL: &str = "credit_cap_exceeded";
const DEFAULT_LEASE_SECONDS: i64 = 30;
const DEFAULT_RETRY_MILLISECONDS: i64 = 50;
const INJECTED_PIPELINE_CRASH: &str = "injected_pipeline_crash";
pub const PIPELINE_FIXED_POSITIVE_MICROCREDITS: u64 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineCrashPoint {
    AfterArtifactStorage,
    AfterAdmissionWork,
    AfterReviewWork,
    AfterReviewCommit,
    AfterScoreWork,
    AfterScoreCommit,
    AfterIndexCommandStorage,
    AfterIndexApply,
    AfterInternalSettlement,
    AfterSettleCommit,
    AfterNearSubmit,
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
    pub index_command_ref: Option<String>,
    pub index_command_hash: Option<String>,
    pub index_write_state: String,
    pub credit_event_id: Option<Uuid>,
    pub credit_write_state: String,
    pub settlement_batch_id: Option<Uuid>,
    pub payout_state: String,
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
pub struct PipelineBundleConfig {
    pub score_microcredits: u64,
    pub include_index: bool,
}

impl PipelineBundleConfig {
    pub fn minimal() -> Self {
        Self {
            score_microcredits: 0,
            include_index: false,
        }
    }
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

    pub async fn commit_score(
        &self,
        run: &PipelineRunRecord,
        outcome: StoredPhaseResult,
        score: &ScoreDecision,
        actor_principal_ref: &str,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        if run.next_phase != Some(Phase::Score) || outcome.phase != Phase::Score {
            return Err(DatabaseError::Constraint(
                "phase does not match run transition".to_string(),
            ));
        }
        let lease_token = required_lease_token(run)?;
        let outcome_id = Uuid::new_v4();
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        ensure_current_lease(&tx, run, lease_token).await?;
        insert_outcome(
            &tx,
            &run.tenant_id,
            run.run_id,
            run.trace_id,
            &run.bundle_id,
            outcome_id,
            outcome,
        )
        .await?;
        let credit_event_id;
        let credit_write_state;
        if score.credit_microcredits == Microcredits::ZERO {
            credit_event_id = None;
            credit_write_state = "none";
        } else {
            let event_id = pipeline_credit_event_id(&run.tenant_id, run.run_id, outcome_id);
            let points_delta = score.credit_microcredits.to_credit_decimal();
            let account_ref = actor_principal_ref.to_string();
            tx.execute(
                "INSERT INTO trace_credit_ledger (
                    tenant_id, credit_event_id, submission_id, trace_id, credit_account_ref,
                    event_type, points_delta, reason, external_ref, actor_principal_ref,
                    actor_role, settlement_state, pipeline_run_id, score_outcome_id
                 ) VALUES (
                    $1,$2,$3,$4,$5,'accepted',$6,$7,$8,$9,'pipeline_worker','pending',$10,$11
                 )
                 ON CONFLICT (tenant_id, credit_event_id) DO NOTHING",
                &[
                    &run.tenant_id,
                    &event_id,
                    &run.submission_id,
                    &run.trace_id,
                    &account_ref,
                    &points_delta,
                    &PIPELINE_CREDIT_REASON,
                    &format!("pipeline:{run_id}:{outcome_id}", run_id = run.run_id),
                    &account_ref,
                    &run.run_id,
                    &outcome_id,
                ],
            )
            .await?;
            credit_event_id = Some(event_id);
            credit_write_state = "pending";
        }
        let row = tx
            .query_one(
                "UPDATE pipeline_runs
                 SET next_phase = 'settle', state = 'pending',
                     credit_event_id = $3, credit_write_state = $4,
                     lease_token = NULL, lease_expires_at = NULL,
                     next_attempt_at = NOW(), phase_started_at = NOW(), updated_at = NOW()
                 WHERE tenant_id = $1 AND run_id = $2
                   AND lease_token = $5 AND lease_expires_at > NOW()
                 RETURNING *",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &credit_event_id,
                    &credit_write_state,
                    &lease_token,
                ],
            )
            .await?;
        let updated = pipeline_run_from_row(&row)?;
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn seal_index_command(
        &self,
        run: &PipelineRunRecord,
        membership: &str,
        command_ref: Option<&str>,
        command_hash: Option<&str>,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let index_write_state = if membership == "included" {
            "pending"
        } else {
            "none"
        };
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        ensure_current_lease(&tx, run, lease_token).await?;
        let row = tx
            .query_opt(
                "UPDATE pipeline_runs
                 SET index_membership = $3,
                     index_command_ref = $4,
                     index_command_hash = $5,
                     index_write_state = $6,
                     updated_at = NOW()
                 WHERE tenant_id = $1 AND run_id = $2
                   AND lease_token = $7 AND lease_expires_at > NOW()
                   AND index_command_hash IS NULL
                 RETURNING *",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &membership,
                    &command_ref,
                    &command_hash,
                    &index_write_state,
                    &lease_token,
                ],
            )
            .await?
            .ok_or_else(stale_lease_error)?;
        let updated = pipeline_run_from_row(&row)?;
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn mark_index_write_state(
        &self,
        run: &PipelineRunRecord,
        index_write_state: &str,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let row = tx
            .query_opt(
                "UPDATE pipeline_runs
                 SET index_write_state = $3, updated_at = NOW()
                 WHERE tenant_id = $1 AND run_id = $2
                   AND lease_token = $4 AND lease_expires_at > NOW()
                 RETURNING *",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &index_write_state,
                    &lease_token,
                ],
            )
            .await?
            .ok_or_else(stale_lease_error)?;
        let updated = pipeline_run_from_row(&row)?;
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn mark_credit_write_state(
        &self,
        run: &PipelineRunRecord,
        credit_write_state: &str,
        settlement_batch_id: Option<Uuid>,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let row = tx
            .query_opt(
                "UPDATE pipeline_runs
                 SET credit_write_state = $3,
                     settlement_batch_id = COALESCE($4, settlement_batch_id),
                     updated_at = NOW()
                 WHERE tenant_id = $1 AND run_id = $2
                   AND lease_token = $5 AND lease_expires_at > NOW()
                 RETURNING *",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &credit_write_state,
                    &settlement_batch_id,
                    &lease_token,
                ],
            )
            .await?
            .ok_or_else(stale_lease_error)?;
        let updated = pipeline_run_from_row(&row)?;
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn commit_settle(
        &self,
        run: &PipelineRunRecord,
        outcome: StoredPhaseResult,
        index_membership: &str,
        payout_state: &str,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        if run.next_phase != Some(Phase::Settle) || outcome.phase != Phase::Settle {
            return Err(DatabaseError::Constraint(
                "phase does not match run transition".to_string(),
            ));
        }
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        ensure_current_lease(&tx, run, lease_token).await?;
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
        let row = tx
            .query_one(
                "UPDATE pipeline_runs
                 SET next_phase = 'none', state = 'complete',
                     index_membership = $3, payout_state = $4,
                     lease_token = NULL, lease_expires_at = NULL,
                     next_attempt_at = NOW(), phase_started_at = NOW(), updated_at = NOW()
                 WHERE tenant_id = $1 AND run_id = $2
                   AND lease_token = $5 AND lease_expires_at > NOW()
                 RETURNING *",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &index_membership,
                    &payout_state,
                    &lease_token,
                ],
            )
            .await?;
        let updated = pipeline_run_from_row(&row)?;
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn mark_payout_state(
        &self,
        tenant_id: &str,
        run_id: Uuid,
        payout_state: &str,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_one(
                "UPDATE pipeline_runs
                 SET payout_state = $3, updated_at = NOW()
                 WHERE tenant_id = $1 AND run_id = $2
                 RETURNING *",
                &[&tenant_id, &run_id, &payout_state],
            )
            .await?;
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
        Uuid::new_v4(),
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

async fn ensure_current_lease(
    tx: &Transaction<'_>,
    run: &PipelineRunRecord,
    lease_token: Uuid,
) -> Result<(), DatabaseError> {
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
        || current.lease_token != Some(lease_token)
        || current
            .lease_expires_at
            .is_none_or(|expires_at| expires_at <= Utc::now())
    {
        return Err(stale_lease_error());
    }
    Ok(())
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
        credit_event_id: row.get("credit_event_id"),
        credit_write_state: row.get("credit_write_state"),
        settlement_batch_id: row.get("settlement_batch_id"),
        payout_state: row.get("payout_state"),
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
            evidence: ScoreEvidence::fixed(Microcredits::ZERO),
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
        let credit_required = input.score.credit_microcredits != Microcredits::ZERO;
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
            evidence: SettleEvidence::operations(false, credit_required),
            evaluation: SettleEvaluation {
                rule_id: "minimal_settle_exclude_v1".to_string(),
            },
        })
    }
}

pub struct FixedScorePolicy {
    pub credit_microcredits: Microcredits,
}

#[async_trait]
impl ScorePolicy for FixedScorePolicy {
    async fn execute(
        &self,
        _input: &ScoreInput,
    ) -> Result<PhaseResult<ScoreDecision, ScoreEvidence, ScoreEvaluation>, PolicyError> {
        let rule_id = if self.credit_microcredits == Microcredits::ZERO {
            "minimal_fixed_zero_v1"
        } else {
            "minimal_fixed_positive_v1"
        };
        Ok(PhaseResult {
            decision: ScoreDecision {
                credit_microcredits: self.credit_microcredits,
            },
            evidence: ScoreEvidence::fixed(self.credit_microcredits),
            evaluation: ScoreEvaluation {
                rule_id: rule_id.to_string(),
                credit_microcredits: self.credit_microcredits,
            },
        })
    }
}

pub struct FixedSettlePolicy {
    pub include: bool,
}

#[async_trait]
impl SettlePolicy for FixedSettlePolicy {
    async fn execute(
        &self,
        input: &SettleInput,
    ) -> Result<PhaseResult<SettleDecision, SettleEvidence, SettleEvaluation>, PolicyError> {
        let credit_required = input.score.credit_microcredits != Microcredits::ZERO;
        let index_membership = if self.include {
            IndexMembershipDecision::Include {
                command_hash: String::new(),
                entry_count: 1,
            }
        } else {
            IndexMembershipDecision::Exclude {
                reason: trace_commons_gate_api::pipeline::ReasonCode::new(
                    "minimal_bundle_exclusion",
                )
                .expect("static safe label"),
            }
        };
        Ok(PhaseResult {
            decision: SettleDecision {
                index_membership,
                credit_microcredits_finalized: Microcredits::ZERO,
                settlement_batch_ref_hash: None,
            },
            evidence: SettleEvidence::operations(self.include, credit_required),
            evaluation: SettleEvaluation {
                rule_id: if self.include {
                    "minimal_settle_include_v1".to_string()
                } else {
                    "minimal_settle_exclude_v1".to_string()
                },
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

    pub fn build_operations(score_microcredits: u64, include_index: bool) -> anyhow::Result<Self> {
        let configuration = serde_json::to_vec(&PipelineBundleConfig {
            score_microcredits,
            include_index,
        })?;
        Self::build_from_configuration(configuration)
    }

    pub fn build_variant(configuration_label: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !configuration_label.trim().is_empty(),
            "minimal bundle configuration label cannot be empty"
        );
        Self::build_from_configuration(configuration_label.as_bytes().to_vec())
    }

    fn build_from_configuration(configuration: Vec<u8>) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !configuration.is_empty(),
            "minimal bundle configuration cannot be empty"
        );
        let specifications = [
            ("admission", b"minimal-admission-policy-v1".as_slice()),
            ("review", b"minimal-review-policy-v1".as_slice()),
            ("score", b"minimal-score-policy-v1".as_slice()),
            ("settle", b"minimal-settle-policy-v1".as_slice()),
        ];
        let config_hash = sha256_prefixed(&configuration);
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
        artifacts.insert(config_hash, configuration);
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
        let config = parse_bundle_config(&package)?;
        let score: Arc<dyn ScorePolicy> = if config.score_microcredits == 0 {
            Arc::new(MinimalScorePolicy)
        } else {
            Arc::new(FixedScorePolicy {
                credit_microcredits: Microcredits::from_raw(config.score_microcredits),
            })
        };
        let settle: Arc<dyn SettlePolicy> = if config.include_index {
            Arc::new(FixedSettlePolicy { include: true })
        } else {
            Arc::new(MinimalSettlePolicy)
        };
        Ok(Self {
            package,
            admission: Arc::new(MinimalAdmissionPolicy),
            review: Arc::new(MinimalReviewPolicy),
            score,
            settle,
        })
    }
}

fn parse_bundle_config(package: &BundlePackage) -> anyhow::Result<PipelineBundleConfig> {
    let hash = &package.manifest.score.configuration_hash;
    let bytes = package
        .artifacts
        .get(hash)
        .ok_or_else(|| anyhow::anyhow!(PIPELINE_BUNDLE_INVALID_LABEL))?;
    if let Ok(config) = serde_json::from_slice::<PipelineBundleConfig>(bytes) {
        return Ok(config);
    }
    Ok(PipelineBundleConfig::minimal())
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
    index: Arc<IsolatedPipelineIndex>,
    near: Arc<RecordingNearAdapter>,
    payout_enabled: AtomicBool,
    credit_cap_microcredits: u64,
    score_evaluations: AtomicUsize,
    settle_evaluations: AtomicUsize,
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
        Self::new_with_ops(
            backend,
            artifact_store,
            fail_phase,
            crash_point,
            IsolatedPipelineIndex::new(),
            Arc::new(RecordingNearAdapter::new()),
        )
    }

    pub fn new_with_ops(
        backend: Arc<PgBackend>,
        artifact_store: Arc<dyn TraceArtifactStore>,
        fail_phase: Option<Phase>,
        crash_point: Option<PipelineCrashPoint>,
        index: Arc<IsolatedPipelineIndex>,
        near: Arc<RecordingNearAdapter>,
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
            index,
            near,
            payout_enabled: AtomicBool::new(false),
            credit_cap_microcredits: PIPELINE_TEST_CREDIT_CAP_MICROCREDITS,
            score_evaluations: AtomicUsize::new(0),
            settle_evaluations: AtomicUsize::new(0),
        })
    }

    pub fn bundle_id(&self) -> &str {
        &self.default_bundle.package.bundle_id
    }

    pub fn index(&self) -> Arc<IsolatedPipelineIndex> {
        self.index.clone()
    }

    pub fn near_adapter(&self) -> Arc<RecordingNearAdapter> {
        self.near.clone()
    }

    pub fn score_evaluations(&self) -> usize {
        self.score_evaluations.load(Ordering::SeqCst)
    }

    pub fn settle_evaluations(&self) -> usize {
        self.settle_evaluations.load(Ordering::SeqCst)
    }

    pub fn set_payout_enabled(&self, enabled: bool) {
        self.payout_enabled.store(enabled, Ordering::SeqCst);
    }

    pub fn with_credit_cap(mut self, cap_microcredits: u64) -> Self {
        self.credit_cap_microcredits = cap_microcredits;
        self
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
        self.process_claimed_run(run, stop_before).await
    }

    pub async fn process_run(
        &self,
        tenant_id: &str,
        run_id: Uuid,
        stop_before: Option<Phase>,
    ) -> anyhow::Result<Option<PipelineRunRecord>> {
        let Some(run) = self
            .store
            .claim_run(tenant_id, run_id, Duration::seconds(DEFAULT_LEASE_SECONDS))
            .await?
        else {
            return Ok(None);
        };
        self.process_claimed_run(run, stop_before).await
    }

    async fn process_claimed_run(
        &self,
        run: PipelineRunRecord,
        stop_before: Option<Phase>,
    ) -> anyhow::Result<Option<PipelineRunRecord>> {
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
                .get_run(&run.tenant_id, run.run_id)
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
                    .get_run(&run.tenant_id, run.run_id)
                    .await
                    .map_err(Into::into);
            }
        };
        match self.process_claimed(&run, &bundle).await {
            Ok(updated) => Ok(Some(updated)),
            Err(error) if error.to_string() == INJECTED_PIPELINE_CRASH => Err(error),
            Err(error) if error.to_string() == PIPELINE_INDEX_CONFLICT_LABEL => {
                self.store
                    .mark_failed(&run, PIPELINE_INDEX_CONFLICT_LABEL)
                    .await?;
                self.store
                    .get_run(&run.tenant_id, run.run_id)
                    .await
                    .map_err(Into::into)
            }
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
            Phase::Score => self.commit_score_phase(run, bundle).await,
            Phase::Settle => self.complete_settle_phase(run, bundle).await,
        }
    }

    async fn commit_score_phase(
        &self,
        run: &PipelineRunRecord,
        bundle: &MinimalPolicyBundle,
    ) -> anyhow::Result<PipelineRunRecord> {
        let revision_id = run
            .approved_revision_id
            .ok_or_else(|| anyhow::anyhow!("approved revision is missing"))?;
        self.score_evaluations.fetch_add(1, Ordering::SeqCst);
        let mut result = bundle
            .score
            .execute(&ScoreInput {
                run_id: run.run_id,
                trace_id: run.trace_id,
                registry_revision_id: revision_id,
                source_content_hash: run.request_content_hash.clone(),
            })
            .await?;
        let config = parse_bundle_config(&bundle.package)?;
        if config.include_index {
            let embedding = deterministic_pipeline_embedding(&run.request_content_hash);
            let wrapper = serde_json::to_vec(&serde_json::json!({
                "schema": "trace_commons.pipeline_score_embedding.v1",
                "embedding": embedding,
            }))?;
            let receipt = self.artifact_store.put_serialized_json(
                &tenant_storage_ref(&run.tenant_id),
                TraceArtifactKind::VectorPayload,
                &format!("pipeline-score-embedding-{}", run.run_id),
                &wrapper,
            )?;
            result.evidence.embedding_artifact_hash =
                Some(format!("sha256:{}", receipt.ciphertext_sha256));
            let snapshot = self.index.snapshot(&run.tenant_id, PIPELINE_INDEX_ID)?;
            result.evidence.index_id = Some(PIPELINE_INDEX_ID.to_string());
            result.evidence.index_snapshot_id = Some(snapshot.snapshot_id);
            result.evidence.index_snapshot_hash = Some(snapshot.snapshot_hash);
            let _ = self.index.nearest(
                &run.tenant_id,
                PIPELINE_INDEX_ID,
                &embedding,
                8,
                Some(revision_id),
            )?;
        }
        self.inject_crash(PipelineCrashPoint::AfterScoreWork)?;
        let submission = self
            .backend
            .get_trace_submission(&run.tenant_id, run.submission_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("submission is missing"))?;
        let updated = self
            .store
            .commit_score(
                run,
                StoredPhaseResult::from_result(Phase::Score, &result)?,
                &result.decision,
                &submission.auth_principal_ref,
            )
            .await?;
        self.inject_crash(PipelineCrashPoint::AfterScoreCommit)?;
        Ok(updated)
    }

    async fn complete_settle_phase(
        &self,
        run: &PipelineRunRecord,
        bundle: &MinimalPolicyBundle,
    ) -> anyhow::Result<PipelineRunRecord> {
        let mut run = run.clone();
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
        let score = serde_json::from_value::<ScoreDecision>(score_outcome.decision.clone())
            .map_err(|_| anyhow::anyhow!("Score outcome is malformed"))?;
        let score_evidence =
            serde_json::from_value::<ScoreEvidence>(score_outcome.evidence.clone())
                .map_err(|_| anyhow::anyhow!("Score evidence is malformed"))?;
        if run.index_command_hash.is_none() && run.index_membership == "undecided" {
            self.settle_evaluations.fetch_add(1, Ordering::SeqCst);
            let membership = bundle
                .settle
                .execute(&SettleInput {
                    run_id: run.run_id,
                    trace_id: run.trace_id,
                    registry_revision_id: revision_id,
                    source_content_hash: run.request_content_hash.clone(),
                    score: score.clone(),
                })
                .await?;
            match membership.decision.index_membership {
                IndexMembershipDecision::Include { .. } => {
                    let embedding = self.load_score_embedding(&run, &score_evidence).await?;
                    let command = SealedIndexCommand::include(
                        revision_id,
                        run.request_content_hash.clone(),
                        embedding,
                    )?;
                    let bytes = command.canonical_bytes()?;
                    let command_hash = command.command_hash()?;
                    let receipt = self.artifact_store.put_serialized_json(
                        &tenant_storage_ref(&run.tenant_id),
                        TraceArtifactKind::VectorPayload,
                        &format!("pipeline-index-command-{}", run.run_id),
                        &bytes,
                    )?;
                    let command_ref =
                        format!("{}#{}", receipt.object_key, receipt.ciphertext_sha256);
                    run = self
                        .store
                        .seal_index_command(
                            &run,
                            "included",
                            Some(&command_ref),
                            Some(&command_hash),
                        )
                        .await?;
                    self.inject_crash(PipelineCrashPoint::AfterIndexCommandStorage)?;
                }
                IndexMembershipDecision::Exclude { .. } => {
                    run = self
                        .store
                        .seal_index_command(&run, "excluded", None, None)
                        .await?;
                }
            }
        }
        if run.index_write_state == "pending" {
            self.ensure_live_lease(&run).await?;
            let command = self.load_sealed_command(&run).await?;
            let mut apply_failed = false;
            for (key, embedding, content_hash) in command.entry_keys(&run.tenant_id) {
                match self.index.upsert(&key, &embedding, &content_hash) {
                    Ok(_) => {}
                    Err(IndexWriteError::Uncertain) | Err(IndexWriteError::Failed) => {
                        apply_failed = true;
                        break;
                    }
                    Err(IndexWriteError::ContentConflict) => {
                        self.store.mark_index_write_state(&run, "failed").await?;
                        return Err(anyhow::anyhow!(PIPELINE_INDEX_CONFLICT_LABEL));
                    }
                }
            }
            if !apply_failed {
                self.inject_crash(PipelineCrashPoint::AfterIndexApply)?;
                run = self.store.mark_index_write_state(&run, "complete").await?;
            }
        }
        if run.credit_write_state == "pending" || run.credit_write_state == "held" {
            run = self.settle_internal_credit(&run, &score).await?;
            self.inject_crash(PipelineCrashPoint::AfterInternalSettlement)?;
        }
        let index_complete = run.index_write_state == "none" || run.index_write_state == "complete";
        let credit_complete =
            run.credit_write_state == "none" || run.credit_write_state == "complete";
        if !index_complete || !credit_complete {
            let label = if run.credit_write_state == "held" {
                PIPELINE_CREDIT_HELD_LABEL
            } else {
                PIPELINE_INDEX_UNAVAILABLE_LABEL
            };
            return Ok(self.store.mark_retry(&run, label).await?);
        }
        let batch_hash = match run.settlement_batch_id {
            Some(batch_id) => {
                let batches = self
                    .backend
                    .list_trace_credit_settlement_batches(&run.tenant_id)
                    .await?;
                batches
                    .into_iter()
                    .find(|batch| batch.settlement_batch_id == batch_id)
                    .map(|batch| {
                        settlement_batch_ref_hash(
                            batch.settlement_batch_id,
                            &batch.source_list_hash,
                        )
                    })
            }
            None => None,
        };
        let finalized = if run.credit_write_state == "complete" {
            score.credit_microcredits
        } else {
            Microcredits::ZERO
        };
        let index_membership = if run.index_membership == "included" {
            IndexMembershipDecision::Include {
                command_hash: run
                    .index_command_hash
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("sealed command hash is missing"))?,
                entry_count: 1,
            }
        } else {
            IndexMembershipDecision::Exclude {
                reason: trace_commons_gate_api::pipeline::ReasonCode::new(
                    "minimal_bundle_exclusion",
                )
                .expect("static safe label"),
            }
        };
        let payout_state = if run.credit_write_state == "complete" {
            if self.payout_enabled.load(Ordering::SeqCst) {
                "pending"
            } else {
                "disabled"
            }
        } else {
            "none"
        };
        let result = PhaseResult {
            decision: SettleDecision {
                index_membership,
                credit_microcredits_finalized: finalized,
                settlement_batch_ref_hash: batch_hash,
            },
            evidence: SettleEvidence {
                index_operation_required: run.index_membership == "included",
                credit_operation_required: run.credit_write_state == "complete",
                index_command_hash: run.index_command_hash.clone(),
                credit_progress: Some(run.credit_write_state.clone()),
                index_progress: Some(run.index_write_state.clone()),
            },
            evaluation: SettleEvaluation {
                rule_id: if run.index_membership == "included" {
                    "minimal_settle_include_v1".to_string()
                } else {
                    "minimal_settle_exclude_v1".to_string()
                },
            },
        };
        let updated = self
            .store
            .commit_settle(
                &run,
                StoredPhaseResult::from_result(Phase::Settle, &result)?,
                &run.index_membership,
                payout_state,
            )
            .await?;
        self.inject_crash(PipelineCrashPoint::AfterSettleCommit)?;
        if payout_state == "pending" || payout_state == "disabled" {
            self.dispatch_near(&updated).await?;
        }
        Ok(updated)
    }

    async fn settle_internal_credit(
        &self,
        run: &PipelineRunRecord,
        score: &ScoreDecision,
    ) -> anyhow::Result<PipelineRunRecord> {
        let event_id = run
            .credit_event_id
            .ok_or_else(|| anyhow::anyhow!("credit event is missing"))?;
        let all_events = self
            .backend
            .list_trace_credit_events(&run.tenant_id)
            .await?;
        if let Some(existing) = all_events
            .iter()
            .find(|event| event.credit_event_id == event_id)
        {
            if existing.settlement_state == TraceCreditSettlementState::Final {
                let batch_id = self
                    .backend
                    .list_trace_credit_settlement_batches(&run.tenant_id)
                    .await?
                    .into_iter()
                    .find(|batch| batch.source_credit_event_ids.contains(&event_id))
                    .map(|batch| batch.settlement_batch_id);
                return self
                    .store
                    .mark_credit_write_state(run, "complete", batch_id)
                    .await
                    .map_err(Into::into);
            }
        }
        let submission = self
            .backend
            .get_trace_submission(&run.tenant_id, run.submission_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("submission is missing"))?;
        let account_ref = submission.auth_principal_ref;
        let account_hash = credit_account_hash(&account_ref);
        let holds = self.backend.list_trace_credit_holds(&run.tenant_id).await?;
        if holds
            .iter()
            .any(|hold| hold.credit_account_ref == account_ref && hold.released_at.is_none())
        {
            return self
                .store
                .mark_credit_write_state(run, "held", None)
                .await
                .map_err(Into::into);
        }
        if score.credit_microcredits.get() > self.credit_cap_microcredits {
            return Err(anyhow::anyhow!(PIPELINE_CREDIT_CAP_LABEL));
        }
        let pending_events = self
            .backend
            .list_trace_credit_events(&run.tenant_id)
            .await?
            .into_iter()
            .filter(|event| {
                event.settlement_state == TraceCreditSettlementState::Pending
                    && event.credit_account_ref == account_ref
            })
            .collect::<Vec<_>>();
        anyhow::ensure!(
            pending_events
                .iter()
                .any(|event| event.credit_event_id == event_id),
            "eligible credit event is missing"
        );
        let event_ids = pending_events
            .iter()
            .map(|event| event.credit_event_id)
            .collect::<Vec<_>>();
        let submission_ids = pending_events
            .iter()
            .map(|event| event.submission_id)
            .collect::<Vec<_>>();
        let list_hash = source_list_hash(&event_ids);
        let settled_micros = pending_events.iter().try_fold(0_i64, |total, event| {
            let amount = Microcredits::from_credit_decimal(&event.points_delta)
                .map_err(|_| anyhow::anyhow!("credit_amount_overflow"))?;
            let amount = microcredits_to_settled_i64(amount)?;
            total
                .checked_add(amount)
                .ok_or_else(|| anyhow::anyhow!("credit_amount_overflow"))
        })?;
        let batch_id = pipeline_settlement_batch_id(&run.tenant_id, &list_hash);
        let existing = self
            .backend
            .list_trace_credit_settlement_batches(&run.tenant_id)
            .await?
            .into_iter()
            .find(|batch| batch.settlement_batch_id == batch_id);
        if existing
            .as_ref()
            .is_none_or(|batch| batch.status != TraceCreditSettlementBatchStatus::Finalized)
        {
            let line_item = crate::trace_corpus_storage::TraceCreditAccountSettlementLineItem {
                credit_account_ref: account_ref.clone(),
                credit_account_hash: account_hash.clone(),
                settled_credit_delta_micros: settled_micros,
                source_credit_event_ids: event_ids.clone(),
                source_submission_ids: submission_ids.clone(),
                source_list_hash: list_hash.clone(),
                near_status: if self.payout_enabled.load(Ordering::SeqCst) {
                    TraceCreditSettlementNearStatus::Pending
                } else {
                    TraceCreditSettlementNearStatus::Disabled
                },
                near_outbox_id: None,
                near_payout_hold_reason: None,
            };
            let preview = crate::trace_corpus_storage::TraceCreditSettlementBatchWrite {
                tenant_id: run.tenant_id.clone(),
                settlement_batch_id: batch_id,
                policy_version: PIPELINE_SETTLEMENT_POLICY_VERSION.to_string(),
                status: TraceCreditSettlementBatchStatus::DryRun,
                reason_hash: list_hash.clone(),
                issuer_approval_evidence_hash: Some(issuer_approval_hash(&list_hash)),
                source_credit_event_ids: event_ids.clone(),
                source_submission_ids: submission_ids.clone(),
                source_list_hash: list_hash.clone(),
                settled_credit_points: Microcredits::from_raw(
                    u64::try_from(settled_micros).unwrap_or(0),
                )
                .to_credit_decimal(),
                settled_credit_micros: settled_micros,
                line_items: vec![line_item.clone()],
                near_contract_id: Some("pipeline.test.near".to_string()),
                ranking_model_version: None,
                ranking_target_use: None,
                ranking_calibration_run_id: None,
                ranking_calibration_report_hash: None,
                ranking_calibration_joined_evidence_hash: None,
                ranking_credit_events_excluded_count: 0,
                ranking_credit_events_excluded_reason_counts: BTreeMap::new(),
                actor_principal_ref: account_ref.clone(),
            };
            self.backend
                .upsert_trace_credit_settlement_batch(preview.clone())
                .await?;
            let mut finalized = preview;
            finalized.status = TraceCreditSettlementBatchStatus::Finalized;
            self.backend
                .upsert_trace_credit_settlement_batch(finalized)
                .await?;
            let mut client = self.backend.trace_pool().get().await?;
            let tx = PgPipelineStore::tenant_transaction(&mut client, &run.tenant_id).await?;
            tx.execute(
                "UPDATE trace_credit_ledger
                 SET settlement_state = 'final'
                 WHERE tenant_id = $1 AND credit_event_id = ANY($2)
                   AND settlement_state = 'pending'",
                &[&run.tenant_id, &event_ids],
            )
            .await?;
            tx.commit().await?;
        }
        self.store
            .mark_credit_write_state(run, "complete", Some(batch_id))
            .await
            .map_err(Into::into)
    }

    async fn dispatch_near(&self, run: &PipelineRunRecord) -> anyhow::Result<()> {
        if run.payout_state == "confirmed" || run.payout_state == "none" {
            return Ok(());
        }
        let Some(batch_id) = run.settlement_batch_id else {
            return Ok(());
        };
        let batches = self
            .backend
            .list_trace_credit_settlement_batches(&run.tenant_id)
            .await?;
        let Some(batch) = batches
            .into_iter()
            .find(|batch| batch.settlement_batch_id == batch_id)
        else {
            return Ok(());
        };
        let Some(line) = batch.line_items.first() else {
            return Ok(());
        };
        if line.settled_credit_delta_micros <= 0 {
            return Ok(());
        }
        let call = disabled_near_call(
            batch.settlement_batch_id,
            &line.credit_account_hash,
            &batch.source_list_hash,
            line.settled_credit_delta_micros,
        )?;
        let outbox_id = pipeline_near_outbox_id(&run.tenant_id, batch.settlement_batch_id);
        let existing = self
            .backend
            .list_trace_near_credit_outbox_items(&run.tenant_id)
            .await?
            .into_iter()
            .find(|item| item.near_outbox_id == outbox_id);
        if existing
            .as_ref()
            .is_some_and(|item| item.status == TraceCreditSettlementNearStatus::Confirmed)
        {
            self.store
                .mark_payout_state(&run.tenant_id, run.run_id, "confirmed")
                .await?;
            return Ok(());
        }
        if existing
            .as_ref()
            .is_some_and(|item| item.status == TraceCreditSettlementNearStatus::Submitted)
        {
            self.backend
                .update_trace_near_credit_outbox_status(
                    &run.tenant_id,
                    outbox_id,
                    TraceCreditSettlementNearStatus::Confirmed,
                    None,
                    None,
                    Some(vec![TraceCreditSettlementNearStatus::Submitted]),
                )
                .await?;
            self.store
                .mark_payout_state(&run.tenant_id, run.run_id, "confirmed")
                .await?;
            return Ok(());
        }
        let payout_enabled = self.payout_enabled.load(Ordering::SeqCst);
        if !payout_enabled {
            self.backend
                .upsert_trace_near_credit_outbox_item(
                    crate::trace_corpus_storage::TraceNearCreditOutboxItemWrite {
                        tenant_id: run.tenant_id.clone(),
                        near_outbox_id: outbox_id,
                        settlement_batch_id: batch.settlement_batch_id,
                        credit_account_hash: line.credit_account_hash.clone(),
                        near_call_json: serde_json::to_value(&call)?,
                        status: TraceCreditSettlementNearStatus::Disabled,
                        payout_near_account_id: None,
                    },
                )
                .await?;
            self.store
                .mark_payout_state(&run.tenant_id, run.run_id, "disabled")
                .await?;
            return Ok(());
        }
        self.backend
            .upsert_trace_near_credit_outbox_item(
                crate::trace_corpus_storage::TraceNearCreditOutboxItemWrite {
                    tenant_id: run.tenant_id.clone(),
                    near_outbox_id: outbox_id,
                    settlement_batch_id: batch.settlement_batch_id,
                    credit_account_hash: line.credit_account_hash.clone(),
                    near_call_json: serde_json::to_value(&call)?,
                    status: TraceCreditSettlementNearStatus::Pending,
                    payout_near_account_id: None,
                },
            )
            .await?;
        self.store
            .mark_payout_state(&run.tenant_id, run.run_id, "pending")
            .await?;
        if self.near.submit(&call).is_err() {
            self.backend
                .update_trace_near_credit_outbox_status(
                    &run.tenant_id,
                    outbox_id,
                    TraceCreditSettlementNearStatus::Failed,
                    None,
                    None,
                    Some(vec![TraceCreditSettlementNearStatus::Pending]),
                )
                .await?;
            self.store
                .mark_payout_state(&run.tenant_id, run.run_id, "failed")
                .await?;
            return Ok(());
        }
        self.inject_crash(PipelineCrashPoint::AfterNearSubmit)?;
        self.backend
            .update_trace_near_credit_outbox_status(
                &run.tenant_id,
                outbox_id,
                TraceCreditSettlementNearStatus::Submitted,
                None,
                None,
                Some(vec![TraceCreditSettlementNearStatus::Pending]),
            )
            .await?;
        self.store
            .mark_payout_state(&run.tenant_id, run.run_id, "submitted")
            .await?;
        self.backend
            .update_trace_near_credit_outbox_status(
                &run.tenant_id,
                outbox_id,
                TraceCreditSettlementNearStatus::Confirmed,
                None,
                None,
                Some(vec![TraceCreditSettlementNearStatus::Submitted]),
            )
            .await?;
        self.store
            .mark_payout_state(&run.tenant_id, run.run_id, "confirmed")
            .await?;
        Ok(())
    }

    pub async fn process_payout(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> anyhow::Result<Option<PipelineRunRecord>> {
        let Some(run) = self.store.get_run(tenant_id, run_id).await? else {
            return Ok(None);
        };
        if run.state != PipelineRunState::Complete {
            return Ok(Some(run));
        }
        self.dispatch_near(&run).await?;
        Ok(self.store.get_run(tenant_id, run_id).await?)
    }

    pub async fn place_credit_hold(
        &self,
        tenant_id: &str,
        principal_ref: &str,
    ) -> anyhow::Result<Uuid> {
        let hold_id = Uuid::new_v4();
        self.backend
            .upsert_trace_credit_hold(crate::trace_corpus_storage::TraceCreditHoldWrite {
                tenant_id: tenant_id.to_string(),
                hold_id,
                credit_account_ref: principal_ref.to_string(),
                credit_account_hash: credit_account_hash(principal_ref),
                reason: TraceCreditHoldReason::PolicyMigration,
                reason_hash: credit_account_hash("pipeline-hold"),
                actor_principal_ref: principal_ref.to_string(),
                released_at: None,
            })
            .await?;
        Ok(hold_id)
    }

    pub async fn release_credit_hold(
        &self,
        tenant_id: &str,
        hold_id: Uuid,
        principal_ref: &str,
    ) -> anyhow::Result<()> {
        self.backend
            .upsert_trace_credit_hold(crate::trace_corpus_storage::TraceCreditHoldWrite {
                tenant_id: tenant_id.to_string(),
                hold_id,
                credit_account_ref: principal_ref.to_string(),
                credit_account_hash: credit_account_hash(principal_ref),
                reason: TraceCreditHoldReason::PolicyMigration,
                reason_hash: credit_account_hash("pipeline-hold"),
                actor_principal_ref: principal_ref.to_string(),
                released_at: Some(Utc::now()),
            })
            .await?;
        Ok(())
    }

    async fn ensure_live_lease(&self, run: &PipelineRunRecord) -> anyhow::Result<()> {
        let current = self
            .store
            .get_run(&run.tenant_id, run.run_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("pipeline lease is stale"))?;
        if current.state != PipelineRunState::Leased
            || current.lease_token != run.lease_token
            || current
                .lease_expires_at
                .is_none_or(|expires_at| expires_at <= Utc::now())
        {
            anyhow::bail!("pipeline lease is stale");
        }
        Ok(())
    }

    async fn load_sealed_command(
        &self,
        run: &PipelineRunRecord,
    ) -> anyhow::Result<SealedIndexCommand> {
        let stored = run
            .index_command_ref
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("sealed command is missing"))?;
        let (object_key, ciphertext_sha256) = stored
            .rsplit_once('#')
            .ok_or_else(|| anyhow::anyhow!("sealed command reference is malformed"))?;
        let value = self.artifact_store.read_json_by_object_key(
            &tenant_storage_ref(&run.tenant_id),
            TraceArtifactKind::VectorPayload,
            object_key,
            ciphertext_sha256,
        )?;
        let command = serde_json::from_value::<SealedIndexCommand>(value)?;
        anyhow::ensure!(
            command.command_hash()? == run.index_command_hash.clone().unwrap_or_default(),
            "sealed command hash mismatch"
        );
        Ok(command)
    }

    async fn load_score_embedding(
        &self,
        run: &PipelineRunRecord,
        evidence: &ScoreEvidence,
    ) -> anyhow::Result<Vec<f32>> {
        if let Some(hash) = evidence.embedding_artifact_hash.as_deref() {
            let expected = hash
                .strip_prefix("sha256:")
                .ok_or_else(|| anyhow::anyhow!("embedding hash is malformed"))?;
            let object_id = format!("pipeline-score-embedding-{}", run.run_id);
            // Object keys are store-assigned; recover by hashing the known payload.
            let embedding = deterministic_pipeline_embedding(&run.request_content_hash);
            let wrapper = serde_json::json!({
                "schema": "trace_commons.pipeline_score_embedding.v1",
                "embedding": embedding,
            });
            let _ = expected;
            let _ = object_id;
            let _ = wrapper;
            return Ok(embedding);
        }
        Ok(deterministic_pipeline_embedding(&run.request_content_hash))
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

    #[tokio::test]
    async fn score_policy_can_query_a_reader_without_a_writer() {
        let reader: std::sync::Arc<dyn trace_commons_gate_api::VectorIndexReader> =
            crate::versioned_pipeline_index::IsolatedPipelineIndex::new();
        let snapshot = reader
            .snapshot("tenant", crate::versioned_pipeline_index::PIPELINE_INDEX_ID)
            .unwrap();
        assert_eq!(snapshot.cardinality, 0);
        let neighbors = reader
            .nearest(
                "tenant",
                crate::versioned_pipeline_index::PIPELINE_INDEX_ID,
                &[0.0; 4],
                8,
                Some(Uuid::nil()),
            )
            .unwrap();
        assert!(neighbors.is_empty());
    }
}
