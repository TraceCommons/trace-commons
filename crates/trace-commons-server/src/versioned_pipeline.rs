// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Runtime for the versioned Trace Commons pipeline.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use deadpool_postgres::Transaction;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_postgres::Row;
use trace_commons_gate_api::IndexWriteError;
use trace_commons_gate_api::pipeline::{
    AdmissionDecision, AdmissionEvaluation, AdmissionEvidence, AdmissionInput, AtomicUnits,
    BundlePackage, IndexMembershipDecision, InstrumentAwards, PIPELINE_OUTCOME_SCHEMA_ID,
    PIPELINE_OUTCOME_SCHEMA_VERSION, Phase, PhaseResult, PrivacyRisk, ReasonCode, ReviewDecision,
    ReviewEvaluation, ReviewEvidence, ReviewInput, SchemaRef, ScoreDecision, ScoreEvaluation,
    ScoreEvidence, ScoreInput, SealedIndexCommand, SettleDecision, SettleEvaluation,
    SettleEvidence, SettleInput, TenantStorageRef,
};
use trace_commons_protocol::trace_contribution::{
    ResidualPiiRisk, ResidualRiskCondition, TraceContributionEnvelope,
};
use uuid::Uuid;

use crate::db::postgres::PgBackend;
use crate::error::DatabaseError;
use crate::trace_artifact_store::{
    EncryptedTraceArtifactReceipt, TraceArtifactKind, TraceArtifactStore,
};
use crate::trace_corpus_storage::{
    TraceCorpusStatus, TraceObjectArtifactKind, TraceObjectRefWrite, TraceSubmissionWrite,
    safe_residual_risk_basis_labels,
};
use crate::versioned_pipeline_bundle::{
    IdentifiedEmbedder, IdentifiedIndexReader, IdentifiedIndexWriter, IdentifiedPerplexityScorer,
    MinimalPolicyBundle, PIPELINE_BUNDLE_INVALID_LABEL, PIPELINE_DEPENDENCY_MISSING_LABEL,
    dependency_content_hash, pipeline_operation_ref, pipeline_result_ref,
};
use crate::versioned_pipeline_credit::SettlementAdapterRegistry;

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

fn sha256_prefixed(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
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

/// A crash point a test build can inject a failure at, to prove the
/// receipt/runner logic resumes correctly from durable state rather than
/// from in-memory continuation. Only `AfterArtifactStorage` has a caller in
/// this task; the rest exist so later tasks share one enum shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineCrashPoint {
    AfterArtifactStorage,
    AfterReviewArtifactStorage,
    AfterReviewCommit,
    AfterScoreArtifactStorage,
    AfterScoreCommit,
    AfterSettleSelection,
    AfterIndexApply,
    AfterInstrumentOperation,
    AfterSettleCommit,
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

/// Also the shape `settle_selection` persists (decision D4): the Settle
/// policy's raw result, durable before any external effect, so a retry can
/// reload it (`PgPipelineStore::load_settle_selection`) instead of running
/// the policy again.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Approved content ready to commit with its Review outcome: the encrypted
/// object it was written to, and the provenance `commit_review` records
/// alongside it (decision D7 -- approved content is its own object, never a
/// re-use of the source object ref).
#[derive(Debug, Clone)]
pub struct ApprovedRevision {
    pub revision_id: Uuid,
    pub object_ref: TraceObjectRefWrite,
    pub content_hash: String,
    pub source_content_hash: String,
    pub worker_identity: String,
}

/// A run identity not yet persisted: computed deterministically from the
/// tenant and idempotency key before the receipt transaction opens, and
/// staged (`PgPipelineStore::stage_receipt_artifact`) before it is created.
#[derive(Debug, Clone)]
struct NewPipelineRun {
    tenant_id: String,
    run_id: Uuid,
    submission_id: Uuid,
    trace_id: Uuid,
    bundle_id: String,
    request_idempotency_key: String,
    request_content_hash: String,
    source_object_ref_id: Uuid,
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
    /// Review no longer commits through here -- see `commit_review`, which
    /// stores the approved object reference and its derived-record
    /// provenance in the same transaction as the outcome and the
    /// transition, rather than setting `approved_revision_id` alone (that
    /// alone violates the `pipeline_runs_approved_content_shape` CHECK).
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

    /// Commits the Review outcome together with its approved-content
    /// provenance and the phase transition, in one transaction (decision
    /// D7). On approval: the approved object ref, its `trace_derived_records`
    /// row, the submission's `accepted` status, the outcome row, and the
    /// run's `next_phase = score` / `approved_*` columns all commit or none
    /// do. On rejection: only the submission's `rejected` status, the
    /// outcome row, and the run's terminal transition commit -- the run's
    /// `approved_*` columns stay NULL, satisfying
    /// `pipeline_runs_approved_content_shape`.
    pub async fn commit_review(
        &self,
        run: &PipelineRunRecord,
        outcome: StoredPhaseResult,
        approved: Option<ApprovedRevision>,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        if outcome.phase != Phase::Review || run.next_phase != Some(Phase::Review) {
            return Err(DatabaseError::Constraint(
                "phase does not match run transition".to_string(),
            ));
        }
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        ensure_current_lease(&tx, run, lease_token).await?;

        let (approved_revision_id, approved_object_ref_id, approved_content_hash) =
            if let Some(approved) = &approved {
                tx.execute(
                    "INSERT INTO trace_object_refs (
                        tenant_id, submission_id, object_ref_id, artifact_kind, object_store,
                        object_key, content_sha256, encryption_key_ref, size_bytes, compression,
                        created_by_job_id
                     ) VALUES ($1,$2,$3,'review_snapshot',$4,$5,$6,$7,$8,$9,$10)
                     ON CONFLICT (tenant_id, submission_id, object_ref_id) DO NOTHING",
                    &[
                        &approved.object_ref.tenant_id,
                        &approved.object_ref.submission_id,
                        &approved.object_ref.object_ref_id,
                        &approved.object_ref.object_store,
                        &approved.object_ref.object_key,
                        &approved.object_ref.content_sha256,
                        &approved.object_ref.encryption_key_ref,
                        &approved.object_ref.size_bytes,
                        &approved.object_ref.compression,
                        &approved.object_ref.created_by_job_id,
                    ],
                )
                .await?;
                tx.execute(
                    "INSERT INTO trace_derived_records (
                        tenant_id, derived_id, submission_id, trace_id, status,
                        worker_kind, worker_version, input_object_ref_id, input_hash,
                        output_object_ref_id, summary_model
                     ) VALUES ($1,$2,$3,$4,'current','summary',$5,$6,$7,$8,$5)
                     ON CONFLICT (tenant_id, derived_id) DO NOTHING",
                    &[
                        &run.tenant_id,
                        &approved.revision_id,
                        &run.submission_id,
                        &run.trace_id,
                        &approved.worker_identity,
                        &run.source_object_ref_id,
                        &approved.source_content_hash,
                        &approved.object_ref.object_ref_id,
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
                (
                    Some(approved.revision_id),
                    Some(approved.object_ref.object_ref_id),
                    Some(approved.content_hash.clone()),
                )
            } else {
                tx.execute(
                    "UPDATE trace_submissions
                     SET status = 'rejected', reviewed_at = NOW(), updated_at = NOW()
                     WHERE tenant_id = $1 AND submission_id = $2",
                    &[&run.tenant_id, &run.submission_id],
                )
                .await?;
                (None, None, None)
            };

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
        let next_phase = approved_revision_id.map(|_| Phase::Score);
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
                     approved_revision_id = $5,
                     approved_object_ref_id = $6,
                     approved_content_hash = $7,
                     lease_token = NULL, lease_expires_at = NULL,
                     next_attempt_at = NOW(), phase_started_at = NOW(), updated_at = NOW()
                 WHERE tenant_id = $1 AND run_id = $2
                   AND lease_token = $8 AND lease_expires_at > NOW()
                 RETURNING *",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &phase_as_db(next_phase),
                    &state.as_db(),
                    &approved_revision_id,
                    &approved_object_ref_id,
                    &approved_content_hash,
                    &lease_token,
                ],
            )
            .await?;
        let updated = pipeline_run_from_row(&row)?;
        tx.commit().await?;
        Ok(updated)
    }

    /// Commits the Score outcome together with the exact index command it
    /// proposed (if any) and one pending settlement row per award, in one
    /// transaction (decision D5). `command`/`neighbor` are each `(object
    /// ref, hash)`, already stored by the caller before this call opens its
    /// transaction. Every award's settlement row starts `operation_state =
    /// 'pending'` (the column default) with `result_ref_hash` NULL --
    /// Settle records a result only once a leg completes. An award naming
    /// an instrument that `payout_rails` does not cover fails the whole
    /// commit with the safe label `settlement_adapter_missing`.
    pub async fn commit_score(
        &self,
        run: &PipelineRunRecord,
        outcome: StoredPhaseResult,
        awards: &InstrumentAwards,
        command: Option<(&str, &str)>,
        neighbor: Option<(&str, &str)>,
        payout_rails: &BTreeMap<String, String>,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        if outcome.phase != Phase::Score || run.next_phase != Some(Phase::Score) {
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

        for award in awards.iter() {
            let instrument_id = award.instrument_id().as_str();
            let payout_rail = payout_rails.get(instrument_id).ok_or_else(|| {
                DatabaseError::Constraint("settlement_adapter_missing".to_string())
            })?;
            let payout_state = if payout_rail == "none" {
                "disabled"
            } else {
                "pending"
            };
            let atomic_units = award.atomic_units().to_string();
            let operation_ref_hash = pipeline_operation_ref(run.run_id, award);
            tx.execute(
                "INSERT INTO pipeline_run_settlements (
                    tenant_id, run_id, instrument_id, atomic_units,
                    operation_ref_hash, result_ref_hash, payout_rail, payout_state
                 ) VALUES ($1,$2,$3,$4::TEXT::NUMERIC,$5,$6,$7,$8)",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &instrument_id,
                    &atomic_units,
                    &operation_ref_hash,
                    &Option::<&str>::None,
                    &payout_rail.as_str(),
                    &payout_state,
                ],
            )
            .await?;
        }

        let (command_ref, command_hash) = match command {
            Some((object_ref, hash)) => (Some(object_ref), Some(hash)),
            None => (None, None),
        };
        let (neighbor_ref, neighbor_hash) = match neighbor {
            Some((object_ref, hash)) => (Some(object_ref), Some(hash)),
            None => (None, None),
        };
        let row = tx
            .query_one(
                "UPDATE pipeline_runs
                 SET next_phase = 'settle', state = 'pending',
                     index_command_ref = $3, index_command_hash = $4,
                     score_neighbor_ref = $5, score_neighbor_hash = $6,
                     lease_token = NULL, lease_expires_at = NULL,
                     next_attempt_at = NOW(), phase_started_at = NOW(), updated_at = NOW()
                 WHERE tenant_id = $1 AND run_id = $2
                   AND lease_token = $7 AND lease_expires_at > NOW()
                 RETURNING *",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &command_ref,
                    &command_hash,
                    &neighbor_ref,
                    &neighbor_hash,
                    &lease_token,
                ],
            )
            .await?;
        let updated = pipeline_run_from_row(&row)?;
        tx.commit().await?;
        Ok(updated)
    }

    /// Persists the Settle policy's raw result before any external effect
    /// (decision D4): `settle_selection`, its hash, the decided
    /// `index_membership`, and -- only for an include -- `index_write_state
    /// = 'pending'`, all in one transaction, and only the first time (the
    /// `settle_selection IS NULL` guard). A later call for the same run,
    /// after a crash and retry, finds the guard already tripped and updates
    /// nothing; the caller checks `load_settle_selection` first and skips
    /// this call entirely on a retry, so that path is never exercised here.
    pub async fn persist_settle_selection(
        &self,
        run: &PipelineRunRecord,
        selection: &StoredPhaseResult,
        selection_hash: &str,
        membership: &str,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let selection_json = serde_json::to_value(selection).map_err(|_| {
            DatabaseError::Serialization("settle selection encode failed".to_string())
        })?;
        let index_write_state = if membership == "included" {
            "pending"
        } else {
            "none"
        };
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let row = tx
            .query_opt(
                "UPDATE pipeline_runs
                 SET settle_selection = $3, settle_selection_hash = $4,
                     index_membership = $5, index_write_state = $6, updated_at = NOW()
                 WHERE tenant_id = $1 AND run_id = $2
                   AND lease_token = $7 AND lease_expires_at > NOW()
                   AND settle_selection IS NULL
                 RETURNING *",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &selection_json,
                    &selection_hash,
                    &membership,
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

    /// Reloads a persisted Settle selection, if this run has one. `None`
    /// before the first attempt has run the Settle policy.
    pub async fn load_settle_selection(
        &self,
        run: &PipelineRunRecord,
    ) -> Result<Option<StoredPhaseResult>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT settle_selection FROM pipeline_runs WHERE tenant_id = $1 AND run_id = $2",
                &[&run.tenant_id, &run.run_id],
            )
            .await?;
        tx.commit().await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let value: Option<serde_json::Value> = row.get("settle_selection");
        value
            .map(|value| {
                serde_json::from_value::<StoredPhaseResult>(value).map_err(|_| {
                    DatabaseError::Serialization("malformed settle selection".to_string())
                })
            })
            .transpose()
    }

    /// Records progress on the run's index dispatch (port 2258 to 2285):
    /// `pending` after `persist_settle_selection` decides an include,
    /// `complete` once every entry has been applied, `failed` on a content
    /// conflict, `cancelled` when the submission stopped being operable
    /// before dispatch could run.
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

    /// Commits the Settle outcome and completes the run (port 2287 to
    /// 2328), once every settlement operation this task handles (there are
    /// none yet -- Task 13 adds the instrument legs) has reached a terminal
    /// state. `index_membership` was already durable from
    /// `persist_settle_selection`; this call sets it again on the same row
    /// as part of the same transition guard the other `commit_*` methods
    /// use, together with the outcome row and the terminal state change.
    pub async fn commit_settle(
        &self,
        run: &PipelineRunRecord,
        outcome: StoredPhaseResult,
        index_membership: &str,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        if outcome.phase != Phase::Settle || run.next_phase != Some(Phase::Settle) {
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
                     index_membership = $3,
                     lease_token = NULL, lease_expires_at = NULL,
                     next_attempt_at = NOW(), phase_started_at = NOW(), updated_at = NOW()
                 WHERE tenant_id = $1 AND run_id = $2
                   AND lease_token = $4 AND lease_expires_at > NOW()
                 RETURNING *",
                &[&run.tenant_id, &run.run_id, &index_membership, &lease_token],
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

    /// Records (or updates) the receipt-artifact staging row inside the
    /// caller's tenant transaction, before the object-store write and again
    /// after it. A crash between the two calls leaves a `staged` row with a
    /// `NULL` object key/hash, which orphan cleanup (a later task) can find.
    async fn stage_receipt_artifact(
        tx: &Transaction<'_>,
        run: &NewPipelineRun,
        receipt: Option<&EncryptedTraceArtifactReceipt>,
    ) -> Result<(), DatabaseError> {
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
        Ok(())
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

/// Commits the receipt's durable records inside the caller's tenant
/// transaction: the submission, its source object ref, the run itself, the
/// Admission outcome, and the staged-artifact row's transition to
/// `committed`. Unlike the port, this does not insert a
/// `pipeline_receipt_ownership` row (PR 5 / cross-pipeline ownership is
/// deferred).
async fn insert_receipt_records(
    tx: &Transaction<'_>,
    run: &NewPipelineRun,
    submission: &TraceSubmissionWrite,
    object_ref: &TraceObjectRefWrite,
    outcome: StoredPhaseResult,
    admission: &AdmissionDecision,
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
    let residual_risk_basis = submission
        .residual_risk_basis
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|_| {
            DatabaseError::Serialization("trace residual risk basis encode failed".to_string())
        })?;
    let (submission_status, admission_decision, admission_reason, next_phase, run_state) =
        match admission {
            AdmissionDecision::Admit => ("received", "admit", None, "review", "pending"),
            AdmissionDecision::Quarantine { reason } => (
                "quarantined",
                "quarantine",
                Some(reason.as_str()),
                "review",
                "pending",
            ),
            AdmissionDecision::Reject { reason } => (
                "rejected",
                "reject",
                Some(reason.as_str()),
                "none",
                "complete",
            ),
        };
    let inserted = tx
        .execute(
            "INSERT INTO trace_submissions (
                tenant_id, submission_id, trace_id, auth_principal_ref, contributor_pseudonym,
                submitted_tenant_scope_ref, schema_version, consent_policy_version,
                consent_scopes, allowed_uses, retention_policy_id, status, privacy_risk,
                redaction_pipeline_version, redaction_hash, redaction_counts,
                residual_risk_basis,
                canonical_summary_hash, submission_score, credit_points_pending,
                credit_points_final, expires_at
             ) VALUES (
                $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,
                $17,$18,$19,$20,$21,$22
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
                &submission_status,
                &submission.privacy_risk,
                &submission.redaction_pipeline_version,
                &submission.redaction_hash,
                &redaction_counts,
                &residual_risk_basis,
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
            next_phase, state, admission_decision, admission_reason
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
        &[
            &run.tenant_id,
            &run.run_id,
            &run.submission_id,
            &run.trace_id,
            &run.bundle_id,
            &run.request_idempotency_key,
            &run.request_content_hash,
            &run.source_object_ref_id,
            &next_phase,
            &run_state,
            &admission_decision,
            &admission_reason,
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

/// Re-reads the run row `FOR UPDATE` and confirms the caller's lease is
/// still the current one before it writes anything durable under it. Unlike
/// `commit_phase`'s inline check, this does not re-validate `next_phase`:
/// a phase transition always clears `lease_token`, so the token match alone
/// already proves the lease has not moved on.
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

/// Per-instrument settlement caps the service enforces. Not yet read: the
/// cap-enforcement path is added by Task 13.
#[derive(Debug, Clone)]
pub struct PipelineCaps {
    pub per_instrument_atomic_units: BTreeMap<String, AtomicUnits>,
}

/// The submission-operability check Settle runs before deciding index
/// membership and again immediately before it dispatches to the index
/// (`PipelineService::submission_guard`).
#[derive(Debug, Clone, Copy)]
pub struct SubmissionGuard {
    pub operable: bool,
}

/// Whether each held dependency is production-qualified. `scorer` and
/// `embedder` are true only when every scorer/embedder the service holds is
/// (decision P4); a bundle can name any one of them by content hash, so a
/// single unqualified reference dependency disqualifies the whole set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineDependencyQualification {
    pub scorer: bool,
    pub embedder: bool,
    pub index_reader: bool,
    pub index_writer: bool,
    pub settlement_adapters: BTreeMap<String, bool>,
}

/// The per-tenant and per-principal hourly receipt limits. A limit of `0` is
/// disabled, matching `TraceSubmissionQuotaConfig::is_disabled`.
#[derive(Debug, Clone, Copy)]
pub struct PipelineAdmissionLimits {
    pub max_per_tenant_per_hour: usize,
    pub max_per_principal_per_hour: usize,
}

/// Which limit a refused receipt hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineQuotaScope {
    Tenant,
    Principal,
}

/// A receipt request. `request_bytes` is what the caller submitted (its hash
/// is the identity the idempotency key and replay checks are keyed to);
/// `server_envelope` is the envelope the caller has already redacted and
/// wants stored. `residual_risk_basis` is the set of conditions that held
/// when the caller derived `server_envelope.privacy.residual_pii_risk`.
pub struct PipelineReceiptRequest<'a> {
    pub tenant_id: &'a str,
    pub actor_principal_ref: &'a str,
    pub counts_toward_quota: bool,
    pub request_idempotency_key: &'a str,
    pub request_bytes: &'a [u8],
    pub server_envelope: &'a TraceContributionEnvelope,
    pub residual_risk_basis: &'a [ResidualRiskCondition],
    pub limits: PipelineAdmissionLimits,
}

/// The outcome of a receipt submission.
#[derive(Debug)]
pub enum PipelineReceiptResult {
    Created(PipelineRunRecord),
    Replayed(PipelineRunRecord),
    ContentConflict,
    Tombstoned,
    QuotaExceeded(PipelineQuotaScope),
}

/// A run's full state: the run row, its recorded phase outcomes, and its
/// settlements.
pub struct PipelineInspection {
    pub run: PipelineRunRecord,
    pub outcomes: Vec<PhaseOutcomeRecord>,
    pub settlements: Vec<PipelineSettlementRecord>,
}

/// Builds a [`PipelineService`]. Scorers and embedders are registered by
/// content hash (`with_scorer`/`with_embedder`) so `PipelineService::submit`
/// can resolve whichever one a bound bundle package names, rather than the
/// service holding a single fixed pair.
pub struct PipelineServiceBuilder {
    backend: Arc<PgBackend>,
    artifact_store: Arc<dyn TraceArtifactStore>,
    default_package: BundlePackage,
    scorers: BTreeMap<String, Arc<dyn IdentifiedPerplexityScorer>>,
    embedders: BTreeMap<String, Arc<dyn IdentifiedEmbedder>>,
    index_reader: Arc<dyn IdentifiedIndexReader>,
    index_writer: Arc<dyn IdentifiedIndexWriter>,
    settlement_adapters: SettlementAdapterRegistry,
    caps: PipelineCaps,
    crash_point: Option<PipelineCrashPoint>,
}

impl PipelineServiceBuilder {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        backend: Arc<PgBackend>,
        artifact_store: Arc<dyn TraceArtifactStore>,
        default_package: BundlePackage,
        index_reader: Arc<dyn IdentifiedIndexReader>,
        index_writer: Arc<dyn IdentifiedIndexWriter>,
        settlement_adapters: SettlementAdapterRegistry,
        caps: PipelineCaps,
    ) -> Self {
        Self {
            backend,
            artifact_store,
            default_package,
            scorers: BTreeMap::new(),
            embedders: BTreeMap::new(),
            index_reader,
            index_writer,
            settlement_adapters,
            caps,
            crash_point: None,
        }
    }

    pub fn with_scorer(mut self, scorer: Arc<dyn IdentifiedPerplexityScorer>) -> Self {
        self.scorers.insert(
            dependency_content_hash(&scorer.content_descriptor()),
            scorer,
        );
        self
    }

    pub fn with_embedder(mut self, embedder: Arc<dyn IdentifiedEmbedder>) -> Self {
        self.embedders.insert(
            dependency_content_hash(&embedder.content_descriptor()),
            embedder,
        );
        self
    }

    #[doc(hidden)]
    pub fn with_crash_point(mut self, point: PipelineCrashPoint) -> Self {
        self.crash_point = Some(point);
        self
    }

    /// Resolves the default package once, so a service that cannot run its
    /// own default bundle fails at construction rather than on the first
    /// receipt.
    pub fn build(self) -> anyhow::Result<PipelineService> {
        let service = PipelineService {
            store: PgPipelineStore::new(self.backend.clone()),
            backend: self.backend,
            artifact_store: self.artifact_store,
            default_package: self.default_package,
            scorers: self.scorers,
            embedders: self.embedders,
            index_reader: self.index_reader,
            index_writer: self.index_writer,
            settlement_adapters: self.settlement_adapters,
            caps: self.caps,
            crash_point: self.crash_point,
            crash_pending: AtomicBool::new(self.crash_point.is_some()),
            score_evaluations: AtomicUsize::new(0),
            settle_evaluations: AtomicUsize::new(0),
        };
        service
            .construct(service.default_package.clone())
            .map_err(|label| anyhow::anyhow!(label))?;
        Ok(service)
    }
}

pub struct PipelineService {
    backend: Arc<PgBackend>,
    store: PgPipelineStore,
    artifact_store: Arc<dyn TraceArtifactStore>,
    default_package: BundlePackage,
    scorers: BTreeMap<String, Arc<dyn IdentifiedPerplexityScorer>>,
    embedders: BTreeMap<String, Arc<dyn IdentifiedEmbedder>>,
    index_reader: Arc<dyn IdentifiedIndexReader>,
    index_writer: Arc<dyn IdentifiedIndexWriter>,
    settlement_adapters: SettlementAdapterRegistry,
    #[expect(dead_code, reason = "first used by Task 13")]
    caps: PipelineCaps,
    crash_point: Option<PipelineCrashPoint>,
    crash_pending: AtomicBool,
    score_evaluations: AtomicUsize,
    settle_evaluations: AtomicUsize,
}

impl PipelineService {
    pub fn bundle_id(&self) -> &str {
        &self.default_package.bundle_id
    }

    /// How many times the Settle policy actually ran for this service. A
    /// retry that reuses a persisted selection (`load_settle_selection`)
    /// does not increment this -- the policy runs at most once per run.
    pub fn settle_evaluations(&self) -> usize {
        self.settle_evaluations.load(Ordering::SeqCst)
    }

    pub fn dependency_qualification(&self) -> PipelineDependencyQualification {
        PipelineDependencyQualification {
            scorer: self
                .scorers
                .values()
                .all(|scorer| scorer.production_qualified()),
            embedder: self
                .embedders
                .values()
                .all(|embedder| embedder.production_qualified()),
            index_reader: self.index_reader.production_qualified(),
            index_writer: self.index_writer.production_qualified(),
            settlement_adapters: self.settlement_adapters.production_qualifications(),
        }
    }

    /// Connectivity probe: a bare `SELECT 1` through the trace pool. Not
    /// tenant-scoped -- it proves the pool can reach PostgreSQL, nothing
    /// about tenant data.
    pub async fn readiness(&self) -> anyhow::Result<()> {
        self.backend
            .trace_pool()
            .get()
            .await?
            .simple_query("SELECT 1")
            .await?;
        Ok(())
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

    pub async fn inspect(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> anyhow::Result<Option<PipelineInspection>> {
        let Some(run) = self.store.get_run(tenant_id, run_id).await? else {
            return Ok(None);
        };
        let outcomes = self.store.list_outcomes(tenant_id, run_id).await?;
        let settlements = self.store.list_settlements(tenant_id, run_id).await?;
        Ok(Some(PipelineInspection {
            run,
            outcomes,
            settlements,
        }))
    }

    #[doc(hidden)]
    pub fn store(&self) -> &PgPipelineStore {
        &self.store
    }

    async fn ensure_default_bundle(&self, tenant_id: &str) -> anyhow::Result<()> {
        self.store
            .register_bundle(tenant_id, &self.default_package)
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

    /// Whether the submission behind a run is still operable right now:
    /// accepted, not revoked, not purged, not expired, and not withdrawn.
    /// Settle reads this fresh (under `FOR SHARE OF s`) both before it
    /// decides index membership and again immediately before it dispatches
    /// to the index, since the two checks can be far apart in wall-clock
    /// time across a crash and retry.
    async fn submission_guard(&self, run: &PipelineRunRecord) -> anyhow::Result<SubmissionGuard> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = PgPipelineStore::tenant_transaction(&mut client, &run.tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT s.status = 'accepted' AND s.revoked_at IS NULL AND s.purged_at IS NULL
                        AND (s.expires_at IS NULL OR s.expires_at > NOW())
                        AND NOT EXISTS (
                            SELECT 1 FROM trace_withdrawals w
                             WHERE w.tenant_id = s.tenant_id AND w.submission_id = s.submission_id
                        )
                   FROM trace_submissions s
                  WHERE s.tenant_id = $1 AND s.submission_id = $2
                  FOR SHARE OF s",
                &[&run.tenant_id, &run.submission_id],
            )
            .await?;
        tx.commit().await?;
        let operable = row.map(|row| row.get::<_, bool>(0)).unwrap_or(false);
        Ok(SubmissionGuard { operable })
    }

    /// Confirms the lease this call still holds is the current one,
    /// immediately before an effect on the external index that a
    /// transaction rollback cannot undo (port line 5367). Unlike
    /// `ensure_current_lease`, this has no open transaction to read
    /// through -- it goes back through the store, which is the "service
    /// level" check the port's version also is.
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

    /// Resolve the dependencies the package names and construct its bundle.
    /// A hash the service does not hold fails closed.
    fn construct(&self, package: BundlePackage) -> Result<MinimalPolicyBundle, &'static str> {
        let named = &package.manifest.score.data_artifact_hashes;
        let scorer = named
            .iter()
            .find_map(|hash| self.scorers.get(hash))
            .cloned();
        let embedder = named
            .iter()
            .find_map(|hash| self.embedders.get(hash))
            .cloned();
        let (Some(scorer), Some(embedder)) = (scorer, embedder) else {
            return Err(PIPELINE_DEPENDENCY_MISSING_LABEL);
        };
        MinimalPolicyBundle::from_package_with_runtime(
            package,
            scorer,
            embedder,
            self.index_reader.clone(),
        )
        .map_err(|error| match error.to_string().as_str() {
            PIPELINE_DEPENDENCY_MISSING_LABEL => PIPELINE_DEPENDENCY_MISSING_LABEL,
            "bundle_policy_not_runnable" => PIPELINE_POLICY_NOT_RUNNABLE_LABEL,
            _ => PIPELINE_BUNDLE_INVALID_LABEL,
        })
    }

    /// Loads and constructs the bundle a run is bound to, checking the
    /// operator-controlled runnable flag for the run's current phase.
    async fn load_bound_bundle(
        &self,
        run: &PipelineRunRecord,
    ) -> Result<MinimalPolicyBundle, &'static str> {
        let phase = run.next_phase.ok_or(PIPELINE_BUNDLE_INVALID_LABEL)?;
        let package = self
            .store
            .load_bundle(&run.tenant_id, &run.bundle_id)
            .await
            .map_err(|_| PIPELINE_BUNDLE_INVALID_LABEL)?
            .ok_or(PIPELINE_BUNDLE_MISSING_LABEL)?;
        if !self
            .store
            .policy_is_runnable(&run.tenant_id, &run.bundle_id, phase)
            .await
            .map_err(|_| PIPELINE_BUNDLE_INVALID_LABEL)?
        {
            return Err(PIPELINE_POLICY_NOT_RUNNABLE_LABEL);
        }
        self.construct(package)
    }

    /// Accepts a receipt in one tenant transaction: replay, staged-conflict,
    /// bound-bundle, tombstone, and quota are all checked -- in that order --
    /// before any content is stored. `ensure_default_bundle` runs before the
    /// transaction opens (it takes its own pool connections), which is what
    /// keeps a pool of size one safe: the transaction itself never needs a
    /// second connection.
    pub async fn submit(
        &self,
        request: PipelineReceiptRequest<'_>,
    ) -> anyhow::Result<PipelineReceiptResult> {
        anyhow::ensure!(
            !request.request_idempotency_key.trim().is_empty()
                && request.request_idempotency_key.len() <= 200,
            "invalid idempotency key"
        );
        let tenant_id = request.tenant_id;
        let envelope = request.server_envelope;
        let request_content_hash = sha256_prefixed(request.request_bytes);
        let request_idempotency_key_hash =
            sha256_prefixed(request.request_idempotency_key.as_bytes());

        self.ensure_default_bundle(tenant_id).await?;
        let run_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!(
                "tracecommons:pipeline-run:{tenant_id}:{}",
                request.request_idempotency_key
            )
            .as_bytes(),
        );
        let object_ref_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("tracecommons:pipeline-source-object:{run_id}").as_bytes(),
        );

        let mut client = self.backend.trace_pool().get().await?;
        let tx = PgPipelineStore::tenant_transaction(&mut client, tenant_id).await?;

        // 3. Receipt identity lock, then the tenant quota lock. Both are
        // transaction-scoped advisory locks: they serialize concurrent
        // receipts for the same key (or tenant) without holding a row lock
        // that would block unrelated tenants.
        let receipt_lock = format!("pipeline-receipt:{tenant_id}:{request_idempotency_key_hash}");
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
            &[&receipt_lock],
        )
        .await?;
        let quota_lock = format!("pipeline-quota:{tenant_id}");
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 1))",
            &[&quota_lock],
        )
        .await?;

        // 4. Replay / content-conflict checks (already-created run, then a
        // staged-but-not-yet-committed artifact row from a crashed receipt).
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

        // 5. Bound bundle: the active bundle id (from pipeline_active_bundles
        // only -- routing-table lookups are PR 5), then construct it.
        let bundle_id: String = tx
            .query_opt(
                "SELECT bundle_id FROM pipeline_active_bundles WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await?
            .map(|row| row.get::<_, String>("bundle_id"))
            .ok_or_else(|| anyhow::anyhow!(PIPELINE_BUNDLE_MISSING_LABEL))?;
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
        anyhow::ensure!(runnable, PIPELINE_POLICY_NOT_RUNNABLE_LABEL);
        let bundle = self
            .construct(package)
            .map_err(|error| anyhow::anyhow!(error))?;

        // 6. Tombstone check. A hit is a normal outcome, not an error:
        // nothing has been stored yet, so committing here leaves no trace.
        let tombstoned: bool = tx
            .query_one(
                "SELECT
                    EXISTS (
                        SELECT 1
                          FROM trace_withdrawals
                         WHERE tenant_id = $1 AND submission_id = $2
                    )
                    OR EXISTS (
                        SELECT 1
                          FROM trace_tombstones
                         WHERE tenant_id = $1
                           AND (
                                submission_id = $2
                                OR trace_id = $3
                                OR redaction_hash = $4
                                OR redaction_hash = $5
                           )
                    )",
                &[
                    &tenant_id,
                    &envelope.submission_id,
                    &envelope.trace_id,
                    &envelope.privacy.redaction_hash,
                    &request_content_hash,
                ],
            )
            .await?
            .get(0);
        if tombstoned {
            tx.commit().await?;
            return Ok(PipelineReceiptResult::Tombstoned);
        }

        // 7. Quota, only when the caller wants this receipt counted.
        if request.counts_toward_quota {
            let principal_ref_hash = sha256_prefixed(request.actor_principal_ref.as_bytes());
            // Quota counts pipeline receipts only (pipeline_admission_usage). Legacy
            // submission records are not counted, so in the first hour after a
            // tenant moves to the pipeline it can receive up to one extra hourly
            // quota. The owner accepted this on 2026-09-23; see the activation
            // runbook, "Submission quota at switch-over".
            let counts = tx
                .query_one(
                    "SELECT COUNT(*) FILTER (WHERE counted_at > NOW() - INTERVAL '1 hour') AS tenant_count,
                            COUNT(*) FILTER (WHERE counted_at > NOW() - INTERVAL '1 hour' AND principal_ref_hash = $2) AS principal_count
                       FROM pipeline_admission_usage WHERE tenant_id = $1",
                    &[&tenant_id, &principal_ref_hash],
                )
                .await?;
            let tenant_count: i64 = counts.get("tenant_count");
            let principal_count: i64 = counts.get("principal_count");
            if request.limits.max_per_tenant_per_hour != 0
                && tenant_count >= request.limits.max_per_tenant_per_hour as i64
            {
                tx.commit().await?;
                return Ok(PipelineReceiptResult::QuotaExceeded(
                    PipelineQuotaScope::Tenant,
                ));
            }
            if request.limits.max_per_principal_per_hour != 0
                && principal_count >= request.limits.max_per_principal_per_hour as i64
            {
                tx.commit().await?;
                return Ok(PipelineReceiptResult::QuotaExceeded(
                    PipelineQuotaScope::Principal,
                ));
            }
            tx.execute(
                "INSERT INTO pipeline_admission_usage (
                    tenant_id, request_idempotency_key, principal_ref_hash
                 ) VALUES ($1, $2, $3)",
                &[
                    &tenant_id,
                    &request_idempotency_key_hash,
                    &principal_ref_hash,
                ],
            )
            .await?;
        }

        // 8. Stage, then store the server (already-redacted) envelope bytes,
        // wrapped per decision P1.
        let tenant_storage_ref = pipeline_tenant_storage_ref(tenant_id);
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
        PgPipelineStore::stage_receipt_artifact(&tx, &run, None).await?;
        let server_envelope_bytes = serde_json::to_vec(envelope)?;
        let wrapper = encode_pipeline_artifact_bytes(&server_envelope_bytes)?;
        let artifact_receipt = self.artifact_store.put_serialized_json(
            tenant_storage_ref.as_str(),
            TraceArtifactKind::ContributionEnvelope,
            &run_id.to_string(),
            &wrapper,
        )?;
        PgPipelineStore::stage_receipt_artifact(&tx, &run, Some(&artifact_receipt)).await?;
        self.inject_crash(PipelineCrashPoint::AfterArtifactStorage)?;

        // 9. Admission. `authenticated`/`authority_valid`/`grant_valid` are
        // fixed `true` in PR 2 (decision D15): the legacy handler already
        // checked them, and authority is not ported until PR 3.
        let privacy_risk = match envelope.privacy.residual_pii_risk {
            ResidualPiiRisk::Low => PrivacyRisk::Low,
            ResidualPiiRisk::Medium
                if matches!(
                    request.residual_risk_basis,
                    [ResidualRiskCondition::ConsentContentFlag]
                ) =>
            {
                PrivacyRisk::Low
            }
            ResidualPiiRisk::Medium => PrivacyRisk::Medium,
            ResidualPiiRisk::High => PrivacyRisk::High,
        };
        let admission_input = AdmissionInput {
            run_id,
            tenant_storage_ref: tenant_storage_ref.clone(),
            trace_id: envelope.trace_id,
            request_content_hash,
            schema_version: envelope.schema_version.clone(),
            authenticated: true,
            authority_valid: true,
            contribution_path_valid: !envelope.ironclaw.version.trim().is_empty()
                && !envelope
                    .privacy
                    .redaction_pipeline_version
                    .trim()
                    .is_empty(),
            grant_valid: true,
            consent_valid: envelope.consent.revocable,
            allowed_uses_valid: true,
            tombstoned: false,
            quota_available: true,
            privacy_risk,
        };
        let admission = bundle
            .admission
            .execute(&admission_input)
            .await
            .map_err(|error| anyhow::anyhow!(error.label().to_string()))?;
        let stored = StoredPhaseResult::from_result(Phase::Admission, &admission)?;

        // 10. Durable records (no pipeline_receipt_ownership insert -- PR 5).
        let submission = TraceSubmissionWrite {
            tenant_id: tenant_id.to_string(),
            submission_id: envelope.submission_id,
            trace_id: envelope.trace_id,
            auth_principal_ref: request.actor_principal_ref.to_string(),
            contributor_pseudonym: envelope.contributor.pseudonymous_contributor_id.clone(),
            submitted_tenant_scope_ref: None,
            schema_version: envelope.schema_version.clone(),
            consent_policy_version: envelope.consent.policy_version.clone(),
            consent_scopes: enum_strings(&envelope.consent.scopes)?,
            allowed_uses: enum_strings(&envelope.trace_card.allowed_uses)?,
            retention_policy_id: envelope.trace_card.retention_policy.clone(),
            status: TraceCorpusStatus::Received,
            privacy_risk: enum_string(&envelope.privacy.residual_pii_risk)?,
            residual_risk_basis: Some(safe_residual_risk_basis_labels(request.residual_risk_basis)),
            redaction_pipeline_version: envelope.privacy.redaction_pipeline_version.clone(),
            redaction_counts: envelope.privacy.redaction_counts.clone(),
            redaction_hash: envelope.privacy.redaction_hash.clone(),
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
            encryption_key_ref: format!("tenant:{}", tenant_storage_ref.as_str()),
            size_bytes: i64::try_from(server_envelope_bytes.len()).unwrap_or(i64::MAX),
            compression: None,
            created_by_job_id: None,
        };
        insert_receipt_records(
            &tx,
            &run,
            &submission,
            &object_ref,
            stored,
            &admission.decision,
        )
        .await?;
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

    /// Loads a run's committed outcome for `phase` and decodes its decision.
    async fn committed_decision<T: serde::de::DeserializeOwned>(
        &self,
        run: &PipelineRunRecord,
        phase: Phase,
    ) -> anyhow::Result<T> {
        let outcome = self
            .store
            .list_outcomes(&run.tenant_id, run.run_id)
            .await?
            .into_iter()
            .find(|outcome| outcome.phase == phase)
            .ok_or_else(|| anyhow::anyhow!("{phase:?} outcome is missing"))?;
        serde_json::from_value(outcome.decision)
            .map_err(|_| anyhow::anyhow!("{phase:?} outcome is malformed"))
    }

    /// Reads and decrypts the bytes stored at `object_ref_id` for `run`,
    /// under the port's operability predicate (the submission must not be
    /// revoked/expired/purged, the object ref must not be invalidated or
    /// deleted), inside one tenant-scoped `FOR SHARE` transaction, then
    /// decodes the P1 wrapper back to the exact bytes. Shared by
    /// `load_source_bytes` and `load_approved_bytes`, which differ only in
    /// which object ref id they read and what they do with the bytes
    /// afterward.
    async fn load_object_bytes(
        &self,
        run: &PipelineRunRecord,
        object_ref_id: Uuid,
    ) -> anyhow::Result<Vec<u8>> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = PgPipelineStore::tenant_transaction(&mut client, &run.tenant_id).await?;
        let object_ref = tx
            .query_opt(
                "SELECT object_ref.object_key, object_ref.content_sha256
                   FROM trace_submissions submission
                   JOIN trace_object_refs object_ref
                     ON object_ref.tenant_id = submission.tenant_id
                    AND object_ref.submission_id = submission.submission_id
                  WHERE submission.tenant_id = $1
                    AND submission.submission_id = $2
                    AND object_ref.object_ref_id = $3
                    AND submission.status NOT IN ('revoked', 'expired', 'purged')
                    AND submission.revoked_at IS NULL
                    AND submission.purged_at IS NULL
                    AND (submission.expires_at IS NULL OR submission.expires_at > NOW())
                    AND object_ref.invalidated_at IS NULL
                    AND object_ref.deleted_at IS NULL
                  FOR SHARE OF submission, object_ref",
                &[&run.tenant_id, &run.submission_id, &object_ref_id],
            )
            .await?
            .ok_or_else(|| anyhow::anyhow!(PIPELINE_SUBMISSION_INOPERABLE_LABEL))?;
        let object_key: String = object_ref.get("object_key");
        let content_sha256: String = object_ref.get("content_sha256");
        let ciphertext_sha256 = content_sha256
            .strip_prefix("sha256:")
            .ok_or_else(|| anyhow::anyhow!("object artifact hash is malformed"))?;
        let tenant_storage_ref = pipeline_tenant_storage_ref(&run.tenant_id);
        let wrapper = self.artifact_store.read_json_by_object_key(
            tenant_storage_ref.as_str(),
            TraceArtifactKind::ContributionEnvelope,
            &object_key,
            ciphertext_sha256,
        )?;
        tx.commit().await?;
        decode_pipeline_artifact_bytes(&wrapper)
    }

    /// Reads the source envelope bytes the receipt staged at Admission,
    /// decoding the P1 wrapper back to the exact bytes Task 9 stored.
    async fn load_source_bytes(&self, run: &PipelineRunRecord) -> anyhow::Result<Vec<u8>> {
        self.load_object_bytes(run, run.source_object_ref_id).await
    }

    /// Reads the approved-content object the run's Review commit wrote,
    /// decoding the P1 wrapper and requiring the decoded bytes hash to the
    /// `approved_content_hash` the run committed -- never trusting a
    /// re-serialization of whatever the store handed back.
    pub async fn load_approved_bytes(&self, run: &PipelineRunRecord) -> anyhow::Result<Vec<u8>> {
        let approved_object_ref_id = run
            .approved_object_ref_id
            .ok_or_else(|| anyhow::anyhow!(PIPELINE_SUBMISSION_INOPERABLE_LABEL))?;
        let approved_content_hash = run
            .approved_content_hash
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!(PIPELINE_SUBMISSION_INOPERABLE_LABEL))?;
        let bytes = self.load_object_bytes(run, approved_object_ref_id).await?;
        anyhow::ensure!(
            sha256_prefixed(&bytes) == approved_content_hash,
            "approved_content_mismatch"
        );
        Ok(bytes)
    }

    /// Reads and validates the sealed index command a committed Score
    /// outcome stored, per decision P1 (the byte wrapper) and the runtime
    /// plan's ruling A7. `evidence` is the same Score outcome's own
    /// evidence; `None` when Score proposed no command
    /// (`embedding_artifact_hash` absent). Any failure -- a missing or
    /// malformed reference, a decode failure, or a mismatch against the
    /// evidence or the run's own recorded hash/revision -- is the safe
    /// label `index_command_invalid`.
    pub async fn load_index_command(
        &self,
        run: &PipelineRunRecord,
        evidence: &ScoreEvidence,
    ) -> anyhow::Result<Option<SealedIndexCommand>> {
        let Some(expected_hash) = evidence.embedding_artifact_hash.as_deref() else {
            return Ok(None);
        };
        let stored = run
            .index_command_ref
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("index_command_invalid"))?;
        let (object_key, ciphertext_sha256) = stored
            .rsplit_once('#')
            .ok_or_else(|| anyhow::anyhow!("index_command_invalid"))?;
        let tenant = pipeline_tenant_storage_ref(&run.tenant_id);
        let wrapper = self
            .artifact_store
            .read_json_by_object_key(
                tenant.as_str(),
                TraceArtifactKind::VectorPayload,
                object_key,
                ciphertext_sha256,
            )
            .map_err(|_| anyhow::anyhow!("index_command_invalid"))?;
        let bytes = decode_pipeline_artifact_bytes(&wrapper)
            .map_err(|_| anyhow::anyhow!("index_command_invalid"))?;
        let command = serde_json::from_slice::<SealedIndexCommand>(&bytes)
            .map_err(|_| anyhow::anyhow!("index_command_invalid"))?;
        let command_hash = command
            .content_hash()
            .map_err(|_| anyhow::anyhow!("index_command_invalid"))?;
        anyhow::ensure!(
            command_hash == expected_hash
                && Some(command_hash.as_str()) == run.index_command_hash.as_deref()
                && Some(command.revision_id()) == run.approved_revision_id
                && Some(command.index_id()) == evidence.index_id.as_deref()
                && Some(command.model_id()) == evidence.embedder_model_id.as_deref(),
            "index_command_invalid"
        );
        Ok(Some(command))
    }

    /// Claims the next due run for `tenant_id` and advances it one phase.
    pub async fn process_one(&self, tenant_id: &str) -> anyhow::Result<Option<PipelineRunRecord>> {
        let Some(run) = self.store.claim_next(tenant_id).await? else {
            return Ok(None);
        };
        self.process_claimed_run(run).await
    }

    /// Claims a specific run and advances it one phase.
    pub async fn process_run(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> anyhow::Result<Option<PipelineRunRecord>> {
        let Some(run) = self
            .store
            .claim_run(tenant_id, run_id, Duration::seconds(DEFAULT_LEASE_SECONDS))
            .await?
        else {
            return Ok(None);
        };
        self.process_claimed_run(run).await
    }

    /// Loads the run's bound bundle and dispatches its current phase,
    /// mapping every outcome to a durable state change per decision P2. An
    /// injected crash propagates unchanged: the caller is expected to have
    /// simulated a real process crash, so nothing here gets to clean up
    /// after it.
    async fn process_claimed_run(
        &self,
        run: PipelineRunRecord,
    ) -> anyhow::Result<Option<PipelineRunRecord>> {
        let bundle = match self.load_bound_bundle(&run).await {
            Ok(bundle) => bundle,
            Err(label) if label == PIPELINE_POLICY_NOT_RUNNABLE_LABEL => {
                return Ok(Some(self.store.mark_retry(&run, label).await?));
            }
            Err(label) => {
                self.store.mark_failed(&run, label).await?;
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
            Err(error) => {
                let label = error.to_string();
                if label == PIPELINE_INDEX_CONFLICT_LABEL {
                    self.store
                        .mark_failed(&run, PIPELINE_INDEX_CONFLICT_LABEL)
                        .await?;
                    return self
                        .store
                        .get_run(&run.tenant_id, run.run_id)
                        .await
                        .map_err(Into::into);
                }
                // The fixed allowlist from decision P2: a charged retry
                // (the attempt already taken by the claim stays charged)
                // labeled with the safe message the dispatch code raised.
                // Anything else -- including a raw, unlabeled error -- is a
                // charged retry under the generic operational label, never
                // the raw message itself.
                let retry_label = match label.as_str() {
                    "index_command_invalid"
                    | "approved_content_mismatch"
                    | "score_outcome_invalid"
                    | "settlement_operation_mismatch"
                    | "review_output_invalid"
                    | "settlement_adapter_missing"
                    | "submission_inoperable" => label.as_str(),
                    _ => PIPELINE_OPERATIONAL_ERROR_LABEL,
                };
                Ok(Some(self.store.mark_retry(&run, retry_label).await?))
            }
        }
    }

    /// Runs the claimed run's current phase against its bound bundle.
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
                let admission = self
                    .committed_decision::<AdmissionDecision>(run, Phase::Admission)
                    .await?;
                let source_artifact = self.load_source_bytes(run).await?;
                let source_content_hash = sha256_prefixed(&source_artifact);
                let output = bundle
                    .review
                    .execute(&ReviewInput {
                        run_id: run.run_id,
                        tenant_storage_ref: pipeline_tenant_storage_ref(&run.tenant_id),
                        trace_id: run.trace_id,
                        source_content_hash: source_content_hash.clone(),
                        source_artifact,
                        admission,
                        human_assessment: None,
                    })
                    .await?;
                let (result, content) = output.into_parts();
                let approved = match (&result.decision, content) {
                    (
                        ReviewDecision::Approved {
                            registry_revision_id,
                        },
                        Some(content),
                    ) => {
                        anyhow::ensure!(
                            sha256_prefixed(content.bytes()) == content.content_hash(),
                            "review_output_invalid"
                        );
                        let object_id = format!("pipeline-approved-{}", run.run_id);
                        let wrapper = encode_pipeline_artifact_bytes(content.bytes())?;
                        let receipt = self.artifact_store.put_serialized_json(
                            pipeline_tenant_storage_ref(&run.tenant_id).as_str(),
                            TraceArtifactKind::ContributionEnvelope,
                            &object_id,
                            &wrapper,
                        )?;
                        self.inject_crash(PipelineCrashPoint::AfterReviewArtifactStorage)?;
                        Some(ApprovedRevision {
                            revision_id: *registry_revision_id,
                            object_ref: approved_object_ref(run, &receipt, content.bytes().len()),
                            content_hash: content.content_hash().to_string(),
                            source_content_hash,
                            worker_identity: content.worker_identity().to_string(),
                        })
                    }
                    (ReviewDecision::Rejected { .. }, None) => None,
                    _ => anyhow::bail!("review_output_invalid"),
                };
                let updated = self
                    .store
                    .commit_review(
                        run,
                        StoredPhaseResult::from_result(Phase::Review, &result)?,
                        approved,
                    )
                    .await?;
                self.inject_crash(PipelineCrashPoint::AfterReviewCommit)?;
                Ok(updated)
            }
            Phase::Score => self.commit_score_phase(run, bundle).await,
            Phase::Settle => self.complete_settle_phase(run, bundle).await,
        }
    }

    /// Runs Score over the approved bytes, stores the exact index command
    /// and neighbor artifact it proposed (each wrapped per decision P1),
    /// and commits the Score outcome together with one pending settlement
    /// operation per award (decision D5).
    async fn commit_score_phase(
        &self,
        run: &PipelineRunRecord,
        bundle: &MinimalPolicyBundle,
    ) -> anyhow::Result<PipelineRunRecord> {
        let revision_id = run
            .approved_revision_id
            .ok_or_else(|| anyhow::anyhow!("approved revision is missing"))?;
        self.score_evaluations.fetch_add(1, Ordering::SeqCst);
        let reviewed_artifact = self.load_approved_bytes(run).await?;
        let tenant = pipeline_tenant_storage_ref(&run.tenant_id);
        let output = bundle
            .score
            .execute(&ScoreInput {
                run_id: run.run_id,
                tenant_storage_ref: tenant.clone(),
                trace_id: run.trace_id,
                registry_revision_id: revision_id,
                source_content_hash: run.approved_content_hash.clone().unwrap_or_default(),
                reviewed_artifact,
            })
            .await?;
        let (result, command, neighbor) = output.into_parts();
        // A7: before any artifact is stored, every award must name an
        // instrument this bundle's manifest pins.
        bundle
            .package
            .manifest
            .require_pinned(&result.decision.awards)
            .map_err(|_| anyhow::anyhow!("score_outcome_invalid"))?;
        let command_ref = match &command {
            None => None,
            Some(command) => {
                anyhow::ensure!(
                    command.revision_id() == revision_id,
                    "index_command_invalid"
                );
                let bytes = serde_json::to_vec(command)?;
                let wrapper = encode_pipeline_artifact_bytes(&bytes)?;
                let receipt = self.artifact_store.put_serialized_json(
                    tenant.as_str(),
                    TraceArtifactKind::VectorPayload,
                    &format!("pipeline-index-command-{}", run.run_id),
                    &wrapper,
                )?;
                Some((
                    format!("{}#{}", receipt.object_key, receipt.ciphertext_sha256),
                    command.content_hash()?,
                ))
            }
        };
        let neighbor_ref = match &neighbor {
            None => None,
            Some(bytes) => {
                let wrapper = encode_pipeline_artifact_bytes(bytes)?;
                let receipt = self.artifact_store.put_serialized_json(
                    tenant.as_str(),
                    TraceArtifactKind::VectorPayload,
                    &format!("pipeline-score-neighbors-{}", run.run_id),
                    &wrapper,
                )?;
                Some((
                    format!("{}#{}", receipt.object_key, receipt.ciphertext_sha256),
                    sha256_prefixed(bytes),
                ))
            }
        };
        self.inject_crash(PipelineCrashPoint::AfterScoreArtifactStorage)?;
        let updated = self
            .store
            .commit_score(
                run,
                StoredPhaseResult::from_result(Phase::Score, &result)?,
                &result.decision.awards,
                command_ref.as_ref().map(|(r, h)| (r.as_str(), h.as_str())),
                neighbor_ref.as_ref().map(|(r, h)| (r.as_str(), h.as_str())),
                &self.settlement_adapters.payout_rails(),
            )
            .await
            .map_err(|error| match &error {
                // The store's Display prefixes every Constraint error
                // ("Constraint violation: ..."), which would not match
                // decision P2's fixed allowlist verbatim; re-raise the one
                // label the allowlist expects as a bare anyhow error.
                DatabaseError::Constraint(label) if label == "settlement_adapter_missing" => {
                    anyhow::anyhow!("settlement_adapter_missing")
                }
                _ => anyhow::Error::from(error),
            })?;
        self.inject_crash(PipelineCrashPoint::AfterScoreCommit)?;
        Ok(updated)
    }

    /// Settle's first half (brief 3B/3C, port 4547 to 4723 under the #971
    /// settlement shape): validates the committed Score outcome and its
    /// seeded settlement operations, persists the Settle selection before
    /// any external effect (decision D4), and applies the stored index
    /// command to the index without re-querying it (ruling P1). For a run
    /// with no settlement operations at all, this also completes the run --
    /// Task 13 extends the final commit for the case with real per-
    /// instrument legs (amendments-971 A9: each leg is independent).
    async fn complete_settle_phase(
        &self,
        run: &PipelineRunRecord,
        bundle: &MinimalPolicyBundle,
    ) -> anyhow::Result<PipelineRunRecord> {
        let mut run = run.clone();
        let revision_id = run
            .approved_revision_id
            .ok_or_else(|| anyhow::anyhow!("approved revision is missing"))?;

        // Step 1 (brief 3C): the three parts of the committed Score outcome
        // must still agree -- each is stored as an independent JSONB
        // column, so this is a defensive re-check, not a repeat of the
        // `ScoreOutput::new` invariant that already held at commit time.
        let score_outcome = self
            .store
            .list_outcomes(&run.tenant_id, run.run_id)
            .await?
            .into_iter()
            .find(|outcome| outcome.phase == Phase::Score)
            .ok_or_else(|| anyhow::anyhow!("score_outcome_invalid"))?;
        let score_decision = serde_json::from_value::<ScoreDecision>(score_outcome.decision)
            .map_err(|_| anyhow::anyhow!("score_outcome_invalid"))?;
        let score_evidence = serde_json::from_value::<ScoreEvidence>(score_outcome.evidence)
            .map_err(|_| anyhow::anyhow!("score_outcome_invalid"))?;
        let score_evaluation = serde_json::from_value::<ScoreEvaluation>(score_outcome.evaluation)
            .map_err(|_| anyhow::anyhow!("score_outcome_invalid"))?;
        anyhow::ensure!(
            score_decision.awards == score_evidence.fixed_awards
                && score_decision.awards == score_evaluation.awards,
            "score_outcome_invalid"
        );

        // Step 2 (brief 3C, `ensure_operations_match_committed_awards`):
        // the settlement rows Score seeded (decision D5) must still be
        // exactly one per award.
        let settlements = self
            .store
            .list_settlements(&run.tenant_id, run.run_id)
            .await?;
        ensure_operations_match_committed_awards(run.run_id, &score_decision.awards, &settlements)?;

        // Step 3: the submission-operability guard, read fresh.
        let mut guard = self.submission_guard(&run).await?;

        // Step 4: reuse a persisted selection on retry; otherwise run the
        // Settle policy once and persist its selection before any external
        // effect.
        let selection = match self.store.load_settle_selection(&run).await? {
            Some(selection) => selection,
            None => {
                let index_command = self.load_index_command(&run, &score_evidence).await?;
                let tenant = pipeline_tenant_storage_ref(&run.tenant_id);
                self.settle_evaluations.fetch_add(1, Ordering::SeqCst);
                let result = bundle
                    .settle
                    .execute(&SettleInput {
                        run_id: run.run_id,
                        tenant_storage_ref: tenant,
                        trace_id: run.trace_id,
                        registry_revision_id: revision_id,
                        source_content_hash: run.approved_content_hash.clone().unwrap_or_default(),
                        score: score_decision.clone(),
                        score_evidence: score_evidence.clone(),
                        index_command,
                    })
                    .await?;
                result
                    .decision
                    .matches_score(&score_decision)
                    .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
                for operation in result.decision.settlement_operations() {
                    let award = score_decision
                        .awards
                        .iter()
                        .find(|award| award.instrument_id() == operation.instrument_id())
                        .ok_or_else(|| anyhow::anyhow!("settlement_operation_mismatch"))?;
                    anyhow::ensure!(
                        operation.operation_ref_hash()
                            == pipeline_operation_ref(run.run_id, award).as_str(),
                        "settlement_operation_mismatch"
                    );
                    if let Some(result_ref_hash) = operation.result_ref_hash() {
                        anyhow::ensure!(
                            result_ref_hash == pipeline_result_ref(run.run_id, award).as_str(),
                            "settlement_operation_mismatch"
                        );
                    }
                }
                let membership = match &result.decision.index_membership {
                    IndexMembershipDecision::Include { command_hash, .. } if guard.operable => {
                        anyhow::ensure!(
                            Some(command_hash.as_str()) == run.index_command_hash.as_deref(),
                            "index_command_invalid"
                        );
                        "included"
                    }
                    _ => "excluded",
                };
                let stored = StoredPhaseResult::from_result(Phase::Settle, &result)?;
                let selection_hash = sha256_prefixed(&serde_json::to_vec(&stored)?);
                run = self
                    .store
                    .persist_settle_selection(&run, &stored, &selection_hash, membership)
                    .await?;
                self.inject_crash(PipelineCrashPoint::AfterSettleSelection)?;
                stored
            }
        };

        // Step 5: dispatch to the index only when a prior attempt left it
        // pending (an include whose entries are not yet all applied).
        if run.index_write_state == "pending" {
            guard = self.submission_guard(&run).await?;
            if !guard.operable {
                run = self.store.mark_index_write_state(&run, "cancelled").await?;
            } else {
                self.ensure_live_lease(&run).await?;
                let command = self
                    .load_index_command(&run, &score_evidence)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("index_command_invalid"))?;
                let tenant = pipeline_tenant_storage_ref(&run.tenant_id);
                for entry in command.entries() {
                    let key = command.entry_key(&tenant, entry);
                    match self
                        .index_writer
                        .upsert(&key, &entry.embedding, &entry.content_hash)
                    {
                        Ok(_) => {}
                        Err(IndexWriteError::Uncertain) | Err(IndexWriteError::Failed) => {
                            // Ruling (Task 12 ledger): the Settle code
                            // itself retries this label -- it is not in
                            // the P2 allowlist, and never reaches
                            // `process_claimed_run` as an `Err`.
                            return Ok(self
                                .store
                                .mark_retry(&run, PIPELINE_INDEX_UNAVAILABLE_LABEL)
                                .await?);
                        }
                        Err(IndexWriteError::ContentConflict) => {
                            self.store.mark_index_write_state(&run, "failed").await?;
                            return Err(anyhow::anyhow!(PIPELINE_INDEX_CONFLICT_LABEL));
                        }
                    }
                }
                self.inject_crash(PipelineCrashPoint::AfterIndexApply)?;
                run = self.store.mark_index_write_state(&run, "complete").await?;
            }
        }

        // Every settlement row this task handles is already terminal --
        // there are none -- so the run can complete now. Task 13 extends
        // `commit_settle_from_progress` for the case with real legs.
        self.commit_settle_from_progress(&run, &selection, guard)
            .await
    }

    /// Commits the Settle outcome once every settlement row is terminal.
    /// This task completes only the case with no settlement rows at all (no
    /// instrument was awarded to this run); Task 13 extends it to build the
    /// final decision from real per-instrument outcomes (amendments-971 A9).
    async fn commit_settle_from_progress(
        &self,
        run: &PipelineRunRecord,
        selection: &StoredPhaseResult,
        guard: SubmissionGuard,
    ) -> anyhow::Result<PipelineRunRecord> {
        let score_decision = self
            .committed_decision::<ScoreDecision>(run, Phase::Score)
            .await?;
        let stored_decision = serde_json::from_value::<SettleDecision>(selection.decision.clone())
            .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
        let final_decision =
            SettleDecision::new(stored_decision.index_membership, &score_decision, vec![])?;
        let mut evidence = serde_json::from_value::<SettleEvidence>(selection.evidence.clone())
            .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
        evidence.index_command_hash = run.index_command_hash.clone();
        evidence.index_progress = Some(run.index_write_state.clone());
        evidence.submission_operable = Some(guard.operable);
        evidence.guard_reason = if guard.operable {
            None
        } else {
            Some(ReasonCode::new(PIPELINE_SUBMISSION_INOPERABLE_LABEL)?)
        };
        let evaluation = serde_json::from_value::<SettleEvaluation>(selection.evaluation.clone())
            .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
        let final_membership = match &final_decision.index_membership {
            IndexMembershipDecision::Include { .. } => "included",
            IndexMembershipDecision::Exclude { .. } => "excluded",
        };
        let result = PhaseResult {
            decision: final_decision,
            evidence,
            evaluation,
        };
        let outcome = StoredPhaseResult::from_result(Phase::Settle, &result)?;
        let updated = self
            .store
            .commit_settle(run, outcome, final_membership)
            .await?;
        self.inject_crash(PipelineCrashPoint::AfterSettleCommit)?;
        Ok(updated)
    }
}

/// Brief 3C's `ensure_operations_match_committed_awards`: the settlement
/// rows Score seeded (decision D5) must still be exactly one per award,
/// each with the instrument, units, and `pipeline_operation_ref` the award
/// determines. Anything else is the safe label `settlement_operation_mismatch`.
fn ensure_operations_match_committed_awards(
    run_id: Uuid,
    awards: &InstrumentAwards,
    settlements: &[PipelineSettlementRecord],
) -> anyhow::Result<()> {
    anyhow::ensure!(
        awards.iter().len() == settlements.len(),
        "settlement_operation_mismatch"
    );
    for award in awards.iter() {
        let expected_ref = pipeline_operation_ref(run_id, award);
        let matched = settlements.iter().any(|settlement| {
            settlement.instrument_id == award.instrument_id().as_str()
                && settlement.atomic_units == award.atomic_units()
                && settlement.operation_ref_hash == expected_ref
        });
        anyhow::ensure!(matched, "settlement_operation_mismatch");
    }
    Ok(())
}

/// Wraps `bytes` per decision P1: every byte artifact the pipeline stores
/// (starting with the source envelope) is `put_serialized_json`'d as this
/// fixed wrapper, so the exact bytes -- and their hash -- survive a restart
/// unchanged. The matching decode helper is `decode_pipeline_artifact_bytes`.
fn encode_pipeline_artifact_bytes(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    Ok(serde_json::to_vec(&serde_json::json!({
        "schema": "trace_commons.pipeline_artifact_bytes.v1",
        "bytes_base64": base64::engine::general_purpose::STANDARD.encode(bytes),
    }))?)
}

/// Decodes the P1 wrapper back to the exact bytes it was built from. Refuses
/// a wrong `schema` or a missing/invalid `bytes_base64` with a safe label,
/// never a message built from the stored value.
fn decode_pipeline_artifact_bytes(wrapper: &serde_json::Value) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        wrapper.get("schema").and_then(serde_json::Value::as_str)
            == Some("trace_commons.pipeline_artifact_bytes.v1"),
        "pipeline_artifact_wrapper_invalid"
    );
    let encoded = wrapper
        .get("bytes_base64")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("pipeline_artifact_wrapper_invalid"))?;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| anyhow::anyhow!("pipeline_artifact_wrapper_invalid"))
}

/// Builds the `trace_object_refs` write for the approved content a Review
/// approval stores. The object ref id is derived from the run id alone, so a
/// retry after a crash between the artifact write and the commit recomputes
/// the identical id and overwrites the same encrypted object with equal
/// bytes, rather than orphaning a second one.
fn approved_object_ref(
    run: &PipelineRunRecord,
    receipt: &EncryptedTraceArtifactReceipt,
    size_bytes: usize,
) -> TraceObjectRefWrite {
    TraceObjectRefWrite {
        object_ref_id: Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("tracecommons:pipeline-approved-object:{}", run.run_id).as_bytes(),
        ),
        tenant_id: run.tenant_id.clone(),
        submission_id: run.submission_id,
        artifact_kind: TraceObjectArtifactKind::ReviewSnapshot,
        object_store: "pipeline_local_encrypted".to_string(),
        object_key: receipt.object_key.clone(),
        content_sha256: format!("sha256:{}", receipt.ciphertext_sha256),
        encryption_key_ref: format!(
            "tenant:{}",
            pipeline_tenant_storage_ref(&run.tenant_id).as_str()
        ),
        size_bytes: i64::try_from(size_bytes).unwrap_or(i64::MAX),
        compression: None,
        created_by_job_id: None,
    }
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
