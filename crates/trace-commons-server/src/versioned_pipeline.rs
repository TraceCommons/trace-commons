//! Isolated Phase 1 implementation of the versioned four-phase pipeline.
//!
//! This module is local/test-only until later phases add fenced leases,
//! production policies, and operation recovery.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use chrono::{DateTime, Utc};
use deadpool_postgres::Transaction;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_postgres::Row;
use trace_commons_gate_api::pipeline::{
    AdmissionDecision, AdmissionEvaluation, AdmissionEvidence, AdmissionInput, AdmissionPolicy,
    BUNDLE_MANIFEST_FORMAT_VERSION, BundleManifest, BundlePackage, IndexMembershipDecision,
    Microcredits, Phase, PhaseResult, PolicyError, PolicyRef, ReviewDecision, ReviewEvaluation,
    ReviewEvidence, ReviewInput, ReviewPolicy, SchemaRef, ScoreDecision, ScoreEvaluation,
    ScoreEvidence, ScoreInput, ScorePolicy, SettleDecision, SettleEvaluation, SettleEvidence,
    SettleInput, SettlePolicy,
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

fn sha256_prefixed(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PipelineRunState {
    Pending,
    Leased,
    Complete,
    Failed,
}

impl PipelineRunState {
    fn as_db(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Leased => "leased",
            Self::Complete => "complete",
            Self::Failed => "failed",
        }
    }

    fn from_db(value: &str) -> Result<Self, DatabaseError> {
        match value {
            "pending" => Ok(Self::Pending),
            "leased" => Ok(Self::Leased),
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

    pub async fn commit_admission(
        &self,
        run: NewPipelineRun,
        outcome: StoredPhaseResult,
        next_phase: Option<Phase>,
    ) -> Result<PipelineReceiptResult, DatabaseError> {
        debug_assert_eq!(outcome.phase, Phase::Admission);
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        if let Some(row) = tx
            .query_opt(
                "SELECT * FROM pipeline_runs
                 WHERE tenant_id = $1 AND request_idempotency_key = $2
                 FOR UPDATE",
                &[&run.tenant_id, &run.request_idempotency_key],
            )
            .await?
        {
            let existing = pipeline_run_from_row(&row)?;
            tx.commit().await?;
            return if existing.request_content_hash == run.request_content_hash {
                Ok(PipelineReceiptResult::Replayed(existing))
            } else {
                Ok(PipelineReceiptResult::ContentConflict)
            };
        }
        tx.execute(
            "INSERT INTO pipeline_runs (
                tenant_id, run_id, submission_id, trace_id, bundle_id,
                request_idempotency_key, request_content_hash, source_object_ref_id,
                next_phase, state
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,'pending')",
            &[
                &run.tenant_id,
                &run.run_id,
                &run.submission_id,
                &run.trace_id,
                &run.bundle_id,
                &run.request_idempotency_key,
                &run.request_content_hash,
                &run.source_object_ref_id,
                &phase_as_db(next_phase),
            ],
        )
        .await?;
        insert_outcome(
            &tx,
            &run.tenant_id,
            run.run_id,
            run.trace_id,
            &run.bundle_id,
            outcome,
        )
        .await?;
        let row = tx
            .query_one(
                "SELECT * FROM pipeline_runs WHERE tenant_id = $1 AND run_id = $2",
                &[&run.tenant_id, &run.run_id],
            )
            .await?;
        let created = pipeline_run_from_row(&row)?;
        tx.commit().await?;
        Ok(PipelineReceiptResult::Created(created))
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
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let acquired: bool = tx
            .query_one(
                "SELECT pg_try_advisory_xact_lock(hashtext('pipeline-phase1:' || $1))",
                &[&tenant_id],
            )
            .await?
            .get(0);
        if !acquired {
            tx.commit().await?;
            return Ok(None);
        }
        let row = tx
            .query_opt(
                "WITH candidate AS (
                    SELECT run_id FROM pipeline_runs
                    WHERE tenant_id = $1
                      AND state = 'pending'
                      AND next_phase <> 'none'
                      AND NOT EXISTS (
                          SELECT 1 FROM pipeline_runs
                          WHERE tenant_id = $1 AND state = 'leased'
                      )
                    ORDER BY created_at ASC, run_id ASC
                    FOR UPDATE SKIP LOCKED
                    LIMIT 1
                 )
                 UPDATE pipeline_runs p
                 SET state = 'leased', updated_at = NOW()
                 FROM candidate
                 WHERE p.tenant_id = $1 AND p.run_id = candidate.run_id
                 RETURNING p.*",
                &[&tenant_id],
            )
            .await?;
        tx.commit().await?;
        row.as_ref().map(pipeline_run_from_row).transpose()
    }

    pub async fn release_claim(&self, run: &PipelineRunRecord) -> Result<(), DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        tx.execute(
            "UPDATE pipeline_runs
             SET state = 'pending', updated_at = NOW()
             WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'",
            &[&run.tenant_id, &run.run_id],
        )
        .await?;
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
        if current.state != PipelineRunState::Leased || current.next_phase != Some(outcome.phase) {
            return Err(DatabaseError::Constraint(
                "pipeline run is not leased for this phase".to_string(),
            ));
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
                     index_membership = $6, updated_at = NOW()
                 WHERE tenant_id = $1 AND run_id = $2
                 RETURNING *",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &phase_as_db(next_phase),
                    &state.as_db(),
                    &approved_revision_id,
                    &index_membership,
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
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        tx.execute(
            "UPDATE pipeline_runs
             SET state = 'failed', last_error_label = $3, updated_at = NOW()
             WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'",
            &[&run.tenant_id, &run.run_id, &error_label],
        )
        .await?;
        tx.commit().await?;
        Ok(())
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

fn pipeline_run_from_row(row: &Row) -> Result<PipelineRunRecord, DatabaseError> {
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
        last_error_label: row.get("last_error_label"),
        index_membership: row.get("index_membership"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn phase_outcome_from_row(row: &Row) -> Result<PhaseOutcomeRecord, DatabaseError> {
    let version: i32 = row.get("outcome_schema_version");
    Ok(PhaseOutcomeRecord {
        tenant_id: row.get("tenant_id"),
        outcome_id: row.get("outcome_id"),
        run_id: row.get("run_id"),
        trace_id: row.get("trace_id"),
        phase: phase_from_db(row.get("phase"))?.ok_or_else(|| {
            DatabaseError::Serialization("outcome phase cannot be none".to_string())
        })?,
        bundle_id: row.get("bundle_id"),
        outcome_schema: SchemaRef {
            id: row.get("outcome_schema_id"),
            version: u32::try_from(version).map_err(|_| {
                DatabaseError::Serialization("invalid outcome schema version".to_string())
            })?,
        },
        decision: row.get("decision"),
        evidence: row.get("evidence"),
        evaluation: row.get("evaluation"),
        recorded_at: row.get("recorded_at"),
    })
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
        let specifications = [
            ("admission", b"minimal-admission-policy-v1".as_slice()),
            ("review", b"minimal-review-policy-v1".as_slice()),
            ("score", b"minimal-score-policy-v1".as_slice()),
            ("settle", b"minimal-settle-policy-v1".as_slice()),
        ];
        let configuration = b"minimal-local-test-only-v1".as_slice();
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
    bundle: MinimalPolicyBundle,
    fail_phase: Option<Phase>,
}

impl PipelineService {
    pub fn new(
        backend: Arc<PgBackend>,
        artifact_store: Arc<dyn TraceArtifactStore>,
        fail_phase: Option<Phase>,
    ) -> anyhow::Result<Self> {
        let bundle = MinimalPolicyBundle::build()?;
        bundle.package.validate()?;
        Ok(Self {
            store: PgPipelineStore::new(backend.clone()),
            backend,
            artifact_store,
            bundle,
            fail_phase,
        })
    }

    pub fn bundle_id(&self) -> &str {
        &self.bundle.package.bundle_id
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
        if let Some(existing) = self
            .store
            .get_run_by_request_key(tenant_id, &request_idempotency_key_hash)
            .await?
        {
            return if existing.request_content_hash == request_content_hash {
                Ok(PipelineReceiptResult::Replayed(existing))
            } else {
                Ok(PipelineReceiptResult::ContentConflict)
            };
        }

        let envelope: TraceContributionEnvelope = serde_json::from_slice(request_bytes)
            .map_err(|_| anyhow::anyhow!("invalid envelope"))?;
        let run_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("tracecommons:pipeline-run:{tenant_id}:{request_idempotency_key}").as_bytes(),
        );
        let object_ref_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("tracecommons:pipeline-source-object:{run_id}").as_bytes(),
        );
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

        self.backend
            .upsert_trace_submission(TraceSubmissionWrite {
                tenant_id: tenant_id.to_string(),
                submission_id: envelope.submission_id,
                trace_id: envelope.trace_id,
                auth_principal_ref: actor_principal_ref.to_string(),
                contributor_pseudonym: envelope.contributor.pseudonymous_contributor_id.clone(),
                submitted_tenant_scope_ref: None,
                schema_version: envelope.schema_version.clone(),
                consent_policy_version: envelope.consent.policy_version.clone(),
                consent_scopes: enum_strings(&envelope.consent.scopes)?,
                allowed_uses: enum_strings(&envelope.trace_card.allowed_uses)?,
                retention_policy_id: envelope.trace_card.retention_policy.clone(),
                status: TraceCorpusStatus::Received,
                privacy_risk: enum_string(&envelope.privacy.residual_pii_risk)?,
                redaction_pipeline_version: envelope.privacy.redaction_pipeline_version.clone(),
                redaction_counts: envelope.privacy.redaction_counts.clone(),
                redaction_hash: envelope.privacy.redaction_hash.clone(),
                canonical_summary_hash: None,
                submission_score: None,
                credit_points_pending: None,
                credit_points_final: None,
                expires_at: None,
            })
            .await?;
        self.backend
            .append_trace_object_ref(TraceObjectRefWrite {
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
            })
            .await?;

        let admission = self
            .bundle
            .admission
            .execute(&AdmissionInput {
                run_id,
                trace_id: envelope.trace_id,
                request_content_hash: request_content_hash.clone(),
                schema_version: envelope.schema_version,
                authenticated: true,
                authority_valid: true,
            })
            .await
            .map_err(|error| anyhow::anyhow!(error.label().to_string()))?;
        let stored = StoredPhaseResult::from_result(Phase::Admission, &admission)?;
        self.store
            .commit_admission(
                NewPipelineRun {
                    tenant_id: tenant_id.to_string(),
                    run_id,
                    submission_id: envelope.submission_id,
                    trace_id: envelope.trace_id,
                    bundle_id: self.bundle.package.bundle_id.clone(),
                    request_idempotency_key: request_idempotency_key_hash,
                    request_content_hash,
                    source_object_ref_id: object_ref_id,
                },
                stored,
                Some(Phase::Review),
            )
            .await
            .map_err(Into::into)
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
        let result = self.process_claimed(&run).await;
        if result.is_err() {
            self.store
                .mark_failed(&run, PIPELINE_OPERATIONAL_ERROR_LABEL)
                .await?;
        }
        result.map(Some)
    }

    async fn process_claimed(&self, run: &PipelineRunRecord) -> anyhow::Result<PipelineRunRecord> {
        let phase = run
            .next_phase
            .ok_or_else(|| anyhow::anyhow!("claimed run has no phase"))?;
        match phase {
            Phase::Admission => anyhow::bail!("Admission cannot run asynchronously"),
            Phase::Review => {
                let source_artifact = self.load_source_bytes(run).await?;
                let result = self
                    .bundle
                    .review
                    .execute(&ReviewInput {
                        run_id: run.run_id,
                        trace_id: run.trace_id,
                        source_content_hash: run.request_content_hash.clone(),
                        source_artifact,
                    })
                    .await?;
                let revision_id = match result.decision {
                    ReviewDecision::Approved {
                        registry_revision_id,
                    } => Some(registry_revision_id),
                    ReviewDecision::Rejected { .. } => None,
                };
                let next = revision_id.map(|_| Phase::Score);
                self.store
                    .commit_phase(
                        run,
                        StoredPhaseResult::from_result(Phase::Review, &result)?,
                        next,
                        revision_id,
                    )
                    .await
                    .map_err(Into::into)
            }
            Phase::Score => {
                let revision_id = run
                    .approved_revision_id
                    .ok_or_else(|| anyhow::anyhow!("approved revision is missing"))?;
                let result = self
                    .bundle
                    .score
                    .execute(&ScoreInput {
                        run_id: run.run_id,
                        trace_id: run.trace_id,
                        registry_revision_id: revision_id,
                        source_content_hash: run.request_content_hash.clone(),
                    })
                    .await?;
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
                let result = self
                    .bundle
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

    #[tokio::test]
    async fn minimal_policies_produce_explicit_zero_credit_completion() {
        let bundle = MinimalPolicyBundle::build().unwrap();
        bundle.package.validate().unwrap();
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
