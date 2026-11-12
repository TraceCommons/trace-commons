// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The public record, configuration and outcome types of the versioned
//! pipeline, re-exported from `crate::versioned_pipeline`.

use super::*;

/// The columns of a pipeline gate decision row that come from the Score
/// evidence alone (spec C-D3), checked before Settle pays any leg and
/// written by its commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineGateDecisionScoreValues {
    pub perplexity_micros: i64,
    pub tail_fraction_micros: i64,
    pub novelty_score_micros: i64,
    pub perplexity_passed: bool,
    pub novelty_passed: bool,
    pub nearest_neighbor_hash: String,
    pub embedding_evidence_hash: String,
    pub peak_perplexity_micros: Option<i64>,
    pub peak_novelty_micros: Option<i64>,
    pub chunk_count: Option<i32>,
    pub chunks_capped: Option<bool>,
    pub total_chunk_count: Option<i32>,
    pub credit_quality_micros: Option<i64>,
    pub credit_quality_version: Option<i32>,
    pub index_cardinality: Option<i64>,
}

impl PipelineGateDecisionScoreValues {
    /// Refuses with `PIPELINE_GATE_DECISION_EVIDENCE_INCOMPLETE_LABEL` only
    /// when a field the row requires is absent. A value too large for its
    /// column saturates, as `main`'s gate writer stores it: the chunk
    /// aggregate saturates a perplexity to `u64::MAX` on purpose.
    pub fn from_evidence(evidence: &ScoreEvidence) -> Result<Self, &'static str> {
        let micros = |value: u64| i64::try_from(value).unwrap_or(i64::MAX);
        let count = |value: u32| i32::try_from(value).unwrap_or(i32::MAX);
        let required = |value: Option<u64>| {
            value
                .map(micros)
                .ok_or(PIPELINE_GATE_DECISION_EVIDENCE_INCOMPLETE_LABEL)
        };
        Ok(Self {
            perplexity_micros: required(evidence.perplexity_micros)?,
            tail_fraction_micros: required(evidence.tail_fraction_micros)?,
            novelty_score_micros: required(evidence.novelty_score_micros)?,
            perplexity_passed: evidence
                .quality_passed
                .ok_or(PIPELINE_GATE_DECISION_EVIDENCE_INCOMPLETE_LABEL)?,
            novelty_passed: evidence
                .novelty_passed
                .ok_or(PIPELINE_GATE_DECISION_EVIDENCE_INCOMPLETE_LABEL)?,
            nearest_neighbor_hash: evidence
                .nearest_neighbor_hash
                .clone()
                .ok_or(PIPELINE_GATE_DECISION_EVIDENCE_INCOMPLETE_LABEL)?,
            embedding_evidence_hash: evidence
                .embedding_artifact_hash
                .clone()
                .unwrap_or_else(|| sha256_prefixed(PIPELINE_NO_INDEX_COMMAND.as_bytes())),
            peak_perplexity_micros: evidence.peak_perplexity_micros.map(micros),
            peak_novelty_micros: evidence.peak_novelty_micros.map(micros),
            chunk_count: evidence.chunk_count.map(count),
            chunks_capped: evidence.chunks_capped,
            total_chunk_count: evidence.total_chunk_count.map(count),
            credit_quality_micros: evidence.credit_quality_micros.map(micros),
            credit_quality_version: evidence.credit_quality_version,
            index_cardinality: evidence.index_cardinality.map(micros),
        })
    }
}

/// What `main`'s retention maintenance did to a submission with a pipeline
/// run (ruling RB-30; Zaki review 1, round 2, finding 18): the pipeline's
/// half of it is `PgPipelineStore::follow_up_retention`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineRetentionAction {
    /// `main` marks the submission `expired`, as for its expiry date.
    Expired,
    /// `main` marks an expired submission `purged`, as for its purge cutoff.
    Purged,
}

impl PipelineRetentionAction {
    /// The reason code of the index invalidation and of the end of the
    /// submission's runs, `main`'s own lifecycle reason.
    pub(super) fn reason_code(self) -> &'static str {
        match self {
            Self::Expired => "retention_expired",
            Self::Purged => "retention_purged",
        }
    }

    /// The `pipeline_export_snapshot_items.invalidation_reason` (V106).
    pub(super) fn export_invalidation_reason(self) -> &'static str {
        match self {
            Self::Expired => "expired",
            Self::Purged => "purged",
        }
    }
}

/// A crash point a test build can inject a failure at, to prove the
/// receipt/runner logic resumes correctly from durable state rather than
/// from in-memory continuation (`PipelineServiceBuilder::with_crash_point`).
/// Each variant names the durable step it follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineCrashPoint {
    AfterArtifactStorage,
    /// In the Review-start privacy pass, after its output object is staged
    /// and published and before `record_privacy_pass` commits.
    AfterPrivacyPassArtifactStorage,
    /// In the Review-start privacy pass, right after `record_privacy_pass`
    /// commits and before the Review policy runs.
    AfterPrivacyPassCommit,
    AfterReviewArtifactStorage,
    AfterReviewCommit,
    AfterScoreArtifactStorage,
    AfterScoreCommit,
    AfterSettleSelection,
    AfterIndexApply,
    AfterInstrumentOperation,
    AfterCreditLedgerInsert,
    AfterCreditBatchFinalize,
    AfterSettleCommit,
    /// In the payout pass, right after the outbox line records the NEAR
    /// submit (`submitted`).
    AfterNearSubmit,
    /// In the payout pass, right after the outbox line records the NEAR
    /// confirmation (`confirmed`).
    AfterNearConfirm,
}

/// An operator's decision on a run held in `awaiting_operator` (#1313,
/// `PgPipelineStore::resolve_operator_held_run`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineOperatorHeldRunResolution {
    /// Fail the run: N-7 applies, and its revision is queued for removal.
    Fail,
    /// Give the run one more Settle attempt, due now, once the cause of the
    /// refusal is fixed. Its paid legs are terminal, so the attempt pays
    /// nothing again.
    Release,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PipelineRunState {
    Pending,
    Leased,
    Retry,
    /// A run Review quarantined with no human assessment yet
    /// (`PgPipelineStore::mark_awaiting_review`, label
    /// `review_assessment_required`), or a run its Score policy held for
    /// operator review instead of pricing it, parked at Score under the
    /// policy's label (`PgPipelineStore::hold_score_for_review_on_tx`). No claim query selects this state, so
    /// the run does not retry hourly forever while it waits.
    /// `PgPipelineStore::record_review_assessment` moves it back to
    /// `Pending`, due at once, once an assessment lands (Ruling T3-3); a
    /// parked run whose submission has since become inoperable is released
    /// the same way by `claim_review` or `record_review_assessment`
    /// themselves (Ruling T3-6).
    AwaitingReview,
    /// A Settle run with a paid leg whose commit was refused for good
    /// (`pipeline_gate_decision_conflict` or
    /// `pipeline_gate_decision_evidence_incomplete`; #1313, V128). It is not
    /// failed, so its paid revision stays in the index: N-7 removes only
    /// the revision of a run that fails. No claim query selects it; an
    /// operator ends it (`PgPipelineStore::resolve_operator_held_run`).
    AwaitingOperator,
    Complete,
    Failed,
}

impl PipelineRunState {
    pub(super) fn as_db(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Leased => "leased",
            Self::Retry => "retry",
            Self::AwaitingReview => "awaiting_review",
            Self::AwaitingOperator => "awaiting_operator",
            Self::Complete => "complete",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn from_db(value: &str) -> Result<Self, DatabaseError> {
        match value {
            "pending" => Ok(Self::Pending),
            "leased" => Ok(Self::Leased),
            "retry" => Ok(Self::Retry),
            "awaiting_review" => Ok(Self::AwaitingReview),
            "awaiting_operator" => Ok(Self::AwaitingOperator),
            "complete" => Ok(Self::Complete),
            "failed" => Ok(Self::Failed),
            _ => Err(DatabaseError::Serialization(
                "unknown pipeline run state".to_string(),
            )),
        }
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
    /// The Admission decision's quarantine reason label (Quarantine and
    /// Reject alike, V75's shape constraint requires one for each); `None`
    /// for Admit. Read by `list_review_queue`'s callers and by
    /// `record_review_assessment`'s approval check.
    pub admission_reason: Option<String>,
    pub approved_object_ref_id: Option<Uuid>,
    pub approved_content_hash: Option<String>,
    pub score_neighbor_ref: Option<String>,
    pub score_neighbor_hash: Option<String>,
    pub settle_selection_hash: Option<String>,
    /// Whether Review may approve this run only once a privacy pass is
    /// recorded (V117). `false` on every run received before V117; the
    /// column default makes it `true` on every run received after, by
    /// either binary.
    pub privacy_pass_required: bool,
    /// The Review-start privacy pass's record (V117). The six `privacy_pass_*`
    /// columns are set together when the pass commits and are `None` until
    /// then (and on every run that passed Review before V117). Ids, hashes,
    /// labels and a timestamp only: never envelope text or classifier spans.
    pub privacy_pass_object_ref_id: Option<Uuid>,
    /// `sha256:` of the pass output (plaintext).
    pub privacy_pass_content_hash: Option<String>,
    /// `sha256:` of the source bytes the pass read.
    pub privacy_pass_source_hash: Option<String>,
    /// The merged residual-risk labels, as `safe_residual_risk_basis_labels`
    /// writes them.
    pub privacy_pass_residual_risk_basis: Option<Vec<String>>,
    pub privacy_pass_outcome: Option<PrivacyPassOutcome>,
    pub privacy_pass_recorded_at: Option<DateTime<Utc>>,
    /// A human approval of an escalated run: the approving assessment's
    /// `evidence_hash` and the hold reasons it resolved. Set together, and
    /// only when `privacy_pass_outcome` is `Escalated`.
    pub privacy_pass_approval_assessment_hash: Option<String>,
    pub privacy_pass_approval_resolved_reasons: Option<Vec<String>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// What the Review-start privacy pass decided (V117
/// `pipeline_runs.privacy_pass_outcome`): the run goes on to the Review
/// policy, or it is held for a human.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyPassOutcome {
    Cleared,
    Escalated,
}

impl PrivacyPassOutcome {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Cleared => "cleared",
            Self::Escalated => "escalated",
        }
    }

    pub fn from_db(value: &str) -> Result<Self, DatabaseError> {
        match value {
            "cleared" => Ok(Self::Cleared),
            "escalated" => Ok(Self::Escalated),
            _ => Err(DatabaseError::Serialization(
                "unknown privacy pass outcome".to_string(),
            )),
        }
    }
}

/// A human reviewer's exclusive, time-boxed claim on a quarantined run at
/// Review (`pipeline_review_claims`), separate from the run's own
/// worker-phase lease (`pipeline_runs.lease_token`): a reviewer can hold this
/// while the run itself sits idle in `pending`, `retry`, or
/// `awaiting_review`, waiting for `record_review_assessment` to resolve it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineReviewClaim {
    #[serde(skip_serializing, default)]
    pub tenant_id: String,
    pub run_id: Uuid,
    pub reviewer_principal_ref: String,
    pub lease_token: Uuid,
    pub lease_expires_at: DateTime<Utc>,
}

/// What a reviewer's claim on a run came to (`PgPipelineStore::claim_review`;
/// Zaki review 1, round 2, simplification): decided under the run's lock,
/// so a caller never has to read the run back to tell the cases apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineReviewClaimOutcome {
    /// The reviewer holds the claim.
    Claimed(PipelineReviewClaim),
    /// The run is not waiting for review: there is no such run, it is not
    /// at Review, it is not `pending`, `retry` or `awaiting_review`, it is
    /// not a quarantine, or it already has an assessment.
    Ineligible,
    /// The run waits for review, and another reviewer holds a live claim.
    HeldByAnotherReviewer,
}

impl PipelineReviewClaimOutcome {
    /// The claim, when the reviewer holds it.
    pub fn claimed(self) -> Option<PipelineReviewClaim> {
        match self {
            Self::Claimed(claim) => Some(claim),
            Self::Ineligible | Self::HeldByAnotherReviewer => None,
        }
    }
}

/// A worker's exclusive, time-boxed claim on one queued index invalidation
/// (`PgPipelineStore::claim_due_index_invalidations`). The claim moves the
/// row's `next_attempt_at` to `lease_expires_at`, so no other claim gets the
/// row before then. `attempt_count` is the number of attempts charged
/// before this one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineIndexInvalidationClaim {
    pub tenant_id: String,
    pub run_id: Uuid,
    pub registry_revision_id: Uuid,
    pub attempt_count: u32,
    pub lease_expires_at: DateTime<Utc>,
}

/// The follow-up steps the worker runs for a tenant after draining its runs:
/// the index invalidation pass (`PipelineService::process_index_invalidations`),
/// the payout pass (`PipelineService::process_payouts`), and the credit
/// audit step (`PgPipelineStore::list_unaudited_credit_events`; Zaki review
/// 1, round 2, N-5). As `PipelineService::take_follow_ups` returns it, the
/// steps this service queued work for since the worker last took them (Zaki
/// review 1, round 2, item 4).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PipelineFollowUps {
    pub index_invalidations: bool,
    pub payouts: bool,
    pub credit_audits: bool,
}

/// Where one follow-up of a withdrawal stands.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PipelineWithdrawalFollowUpState {
    NotRequired,
    Pending,
    Complete,
    Failed,
}

/// What `PgPipelineStore::withdraw_submission` did. It holds the raw
/// tenant id (in `withdrawal`), so it is never serialized: a route builds
/// its own response from it (Ruling F-M4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineWithdrawalOutcome {
    /// The requested submission's `trace_withdrawals` row.
    pub withdrawal: crate::trace_corpus_storage::TraceWithdrawalRecord,
    pub affected_submission_ids: Vec<Uuid>,
    pub index_invalidation: PipelineWithdrawalFollowUpState,
    pub revocation_propagation: PipelineWithdrawalFollowUpState,
    pub trace_credit_forfeited: bool,
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

/// Why a bound policy is or is not runnable (V111). `runnable` (V93) stays
/// the flag every reader uses; the database keeps the two equal
/// (`runnable = (operational_status = 'runnable')`). `Terminated` is a state
/// the schema holds and no code writes: `terminate` is refused
/// (`PIPELINE_POLICY_INTERVENTION_NOT_SUPPORTED_LABEL`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyOperationalStatus {
    Runnable,
    Suspended,
    Terminated,
}

impl PolicyOperationalStatus {
    pub(super) fn as_db(self) -> &'static str {
        match self {
            Self::Runnable => "runnable",
            Self::Suspended => "suspended",
            Self::Terminated => "terminated",
        }
    }

    pub(super) fn from_db(value: &str) -> Result<Self, DatabaseError> {
        match value {
            "runnable" => Ok(Self::Runnable),
            "suspended" => Ok(Self::Suspended),
            "terminated" => Ok(Self::Terminated),
            _ => Err(DatabaseError::Serialization(
                "unknown policy operational status".to_string(),
            )),
        }
    }
}

/// One operator intervention on a bound policy, as recorded: hash-only and
/// label-only. The actor is the credential's `principal_ref`, the reason a
/// safe label, and `evidence_hash` the SHA-256 of the intervention's fields.
/// The tenant is the caller's, never part of the record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelinePolicyInterventionRecord {
    pub intervention_id: Uuid,
    pub bundle_id: String,
    pub phase: Phase,
    pub action: String,
    pub actor_principal_ref: String,
    pub reason_code: String,
    pub previous_status: PolicyOperationalStatus,
    pub resulting_status: PolicyOperationalStatus,
    pub evidence_hash: String,
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
    /// The adapter's external receipt hash, recorded when the leg completed
    /// with a receipt from an external system. `None` for an internal
    /// effect and for every leg that is not `complete`.
    pub external_receipt_hash: Option<String>,
    pub operation_state: String,
    pub credit_event_id: Option<Uuid>,
    pub settlement_batch_id: Option<Uuid>,
    pub payout_rail: String,
    pub payout_state: String,
    /// Whether the payout may pay this leg: Score seeded it `pending` for a
    /// batch Settle writes under the account's settlement key and hold
    /// (Zaki review 1, round 2, finding 16). A leg the earlier V94 code
    /// seeded has `false` and is never paid.
    #[serde(default)]
    pub payout_eligible: bool,
    /// Diagnostic count of dispatches that ended `retry` or `failed`; the
    /// run's own attempt budget governs retries.
    pub attempt_count: u32,
    pub last_error_label: Option<String>,
    /// When the leg was first leased for an adapter call; never
    /// cleared. A failed run reconciles a dispatched external leg rather
    /// than forfeiting it.
    pub dispatched_at: Option<DateTime<Utc>>,
}

/// A credit event a Trace Credit leg wrote to `main`'s ledger with no
/// `CreditMutate` audit event yet (`PgPipelineStore::list_unaudited_credit_events`):
/// the ledger row's own fields, which the audit event records hash-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineCreditAuditItem {
    pub run_id: Uuid,
    pub instrument_id: String,
    pub credit_event_id: Uuid,
    pub submission_id: Uuid,
    pub event_type: TraceCreditEventType,
    /// The ledger's `points_delta`, as PostgreSQL prints the decimal.
    pub points_delta: String,
    pub reason: Option<String>,
    pub external_ref: Option<String>,
    pub actor_principal_ref: String,
    pub actor_role: String,
}

/// A Review decision of a run's automatic Review, as the review audit pass
/// reads it from `phase_outcomes`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineReviewAuditOutcome {
    pub outcome_id: Uuid,
    pub approved: bool,
}

/// A human review assessment of a run, as the review audit pass reads it
/// from `pipeline_review_assessments`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineReviewAuditAssessment {
    pub assessment_id: Uuid,
    pub approved: bool,
    pub reason_code: String,
    pub reviewer_principal_ref: String,
    /// The assessment resolved a Score policy's hold (V130 `phase =
    /// 'score'`) rather than a Review hold. It changes no corpus status.
    pub at_score: bool,
}

/// A run with a review audit marker (`review_audit_pending_at`,
/// `PgPipelineStore::list_pending_review_audits`): the Review outcome and
/// the assessment whose audit events the worker may still have to append,
/// and whether the privacy pass escalated the run or the run failed at its
/// privacy classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineReviewAuditItem {
    pub run_id: Uuid,
    pub submission_id: Uuid,
    pub pending_at: DateTime<Utc>,
    pub outcome: Option<PipelineReviewAuditOutcome>,
    /// Every assessment of the run, oldest first: one per phase (V130).
    pub assessments: Vec<PipelineReviewAuditAssessment>,
    /// The privacy pass escalated the run (`privacy_pass_outcome = 'escalated'`):
    /// the run is, or was, held for a human under `privacy_pass_review_required`.
    pub privacy_pass_escalated: bool,
    /// The run ended `failed` under `privacy_classification_failed`.
    pub privacy_classification_failed: bool,
    /// The run ended `failed` while its submission was `received`, and the
    /// same transaction moved the submission to `rejected` under
    /// `pipeline_processing_failed` (#1346, V123 `submission_rejected_at`).
    pub submission_rejected: bool,
    /// The time the run was last held for an operator (V128
    /// `operator_held_at`, #1313): a leg paid and its Settle commit was
    /// refused for good, or a released run's attempt failed. Read from the
    /// column, not the run's state, so the hold's audit event is appended
    /// even after an operator has ended the hold. `None` for a run never
    /// held.
    pub settle_held_for_operator_at: Option<DateTime<Utc>>,
}

/// Also the shape `settle_selection` persists: the Settle
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
    /// `canonical_summary_for_embedding` of the approved envelope and its
    /// hash, which `main`'s duplicate controls read
    /// ([`PipelineDuplicateControls`]): the Review commit records them on
    /// its derived record. `None` when the approved content is not an
    /// envelope.
    pub canonical_summary: Option<String>,
    pub canonical_summary_hash: Option<String>,
}

/// What the Review-start privacy pass records with `record_privacy_pass`:
/// its output object (staged under the claim's lease as `privacy-pass`), the
/// hashes of its input and output, its merged residual-risk labels, its
/// outcome, and the post-classifier privacy values written back to the
/// submission row (decision P5). Hashes, ids, labels and counts only.
#[derive(Debug, Clone, Copy)]
pub struct PrivacyPassRecord<'a> {
    /// `privacy_pass_object_ref`'s write for the published pass object.
    pub object_ref: &'a TraceObjectRefWrite,
    /// The pass object's ciphertext hash, as the staged row holds it (bare
    /// hex, `EncryptedTraceArtifactReceipt::ciphertext_sha256`).
    pub ciphertext_sha256: &'a str,
    /// `sha256:` of the pass output (plaintext).
    pub content_hash: &'a str,
    /// `sha256:` of the source bytes the pass read.
    pub source_hash: &'a str,
    /// The merged basis, as `safe_residual_risk_basis_labels` writes it.
    pub basis_labels: &'a [String],
    pub outcome: PrivacyPassOutcome,
    /// The pass envelope's raw residual risk, written to
    /// `trace_submissions.privacy_risk` as the receipt writes it (never the
    /// mapped `PrivacyRisk`).
    pub residual_pii_risk: ResidualPiiRisk,
    pub redaction_counts: &'a BTreeMap<String, u32>,
    pub redaction_pipeline_version: &'a str,
}

/// The receipt-time privacy values of a submission, raw as
/// `trace_submissions` stores them. The privacy pass compares against
/// `pipeline_privacy_risk(&residual_pii_risk, &parsed basis)`, never the raw
/// value (decision P3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmissionReceiptPrivacy {
    pub residual_pii_risk: ResidualPiiRisk,
    /// The stored basis labels; empty when the column is NULL.
    pub residual_risk_basis: Vec<String>,
}

/// A stored human assessment and when it was recorded
/// (`pipeline_review_assessments.recorded_at`). The Review arm ignores an
/// assessment recorded before the run's privacy pass when the pass
/// escalated (critique 1 (2), decision P9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredReviewAssessment {
    pub assessment: HumanReviewAssessment,
    pub recorded_at: DateTime<Utc>,
}

/// One row of the review queue: the run, the reason it is held, and whether
/// the assessment it has was recorded before a privacy pass that escalated
/// it (decision P9: such a run is listed, but cannot be claimed or assessed
/// again).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineReviewQueueEntry {
    pub run: PipelineRunRecord,
    /// The Score policy's label for a run its Score policy held for operator
    /// review (`run.next_phase` is Score,
    /// `PgPipelineStore::hold_score_for_review_on_tx`);
    /// `privacy_pass_review_required` when the privacy pass escalated the
    /// run; else the run's `admission_reason`.
    pub hold_reason: Option<String>,
    pub assessment_superseded: bool,
}

/// Per-instrument settlement caps the service enforces in Settle. An
/// instrument with no entry is a configuration gap: the run waits,
/// uncharged, as `settlement_cap_missing`, before the leg's adapter is
/// called. An amount above its entry is a spend limit: the leg fails as
/// `credit_cap_exceeded` and the retry is charged.
#[derive(Debug, Clone)]
pub struct PipelineCaps {
    pub per_instrument_atomic_units: BTreeMap<String, AtomicUnits>,
}

/// NEAR payout of settled Trace Credit (P3-D11). On exactly when a service
/// is built `with_payout`: that is the one way to turn it on, and leaving it
/// out the one way to leave it off. A payout always requires NEAR
/// confirmation evidence before a leg reads `confirmed`; nothing turns that
/// off.
///
/// `near_contract_id` is the NEAR credit contract every payout call names
/// (and so part of each call's idempotency key). It is the contract `main`'s
/// legacy NEAR path is configured with
/// (`TRACE_COMMONS_CREDIT_SETTLEMENT_NEAR_CONTRACT_ID`, Ruling T10-4); ingest
/// hands it to the assembly and refuses a runtime whose enabled payout
/// names another. An enabled payout requires one.
///
/// `confirmation_interval` is how often a `submitted` payout is polled for
/// its NEAR confirmation: `main`'s NEAR outbox scheduler cadence
/// (`TRACE_COMMONS_NEAR_CREDIT_OUTBOX_SCHEDULER_INTERVAL_SECONDS`, 60
/// seconds by default), which ingest hands to the assembly (Ruling T10-10).
///
/// `controls` are `main`'s NEAR payout controls, which ingest hands to the
/// assembly and refuses a runtime that does not hold.
#[derive(Debug, Clone)]
pub struct PipelinePayoutConfig {
    pub near_contract_id: Option<String>,
    pub confirmation_interval: std::time::Duration,
    pub controls: PipelineNearPayoutControls,
}

/// `main`'s NEAR settlement mode, `TRACE_COMMONS_NEAR_SETTLEMENT_MODE`, as
/// ingest resolves it (unset or unknown is `Disabled`). An enabled payout
/// follows it as `main`'s outbox worker does (Zaki review 1, round 2,
/// finding 2):
///
/// - `Disabled`: nothing advances. The payout writes no outbox row and
///   submits and confirms nothing; each leg stays `pending`.
/// - `DryRun`: the full outbox state machine runs through the in-process
///   `DryRunNearPayoutAdapter`, with no network and no funds, in place of
///   the injected adapter.
/// - `Http`: the injected adapter pays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineNearSettlementMode {
    Disabled,
    DryRun,
    Http,
}

impl PipelineNearSettlementMode {
    pub fn as_label(self) -> &'static str {
        match self {
            PipelineNearSettlementMode::Disabled => "disabled",
            PipelineNearSettlementMode::DryRun => "dry_run",
            PipelineNearSettlementMode::Http => "http",
        }
    }
}

/// `main`'s NEAR payout controls an enabled payout applies (Zaki review 1,
/// round 2, finding 2): its settlement mode, and
/// `TRACE_COMMONS_NEAR_CREDIT_REQUIRE_ADAPTER_AUTH`, under which the payout
/// adapter must present a credential (`NearPayoutAdapter::authenticated`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineNearPayoutControls {
    pub settlement_mode: PipelineNearSettlementMode,
    pub require_adapter_auth: bool,
}

/// Ruling T15-6: the configuration `main`'s `NoveltyUtility` credit checks
/// read, which a compatibility run's Trace Credit leg applies before it
/// writes its ledger row, and (Zaki review 1, item 2) `main`'s settlement
/// controls, which the paid Trace Credit leg applies before its NEAR payout.
/// Ingest hands it to the assembly (`IngestPipelineRuntimeContext`) and
/// refuses a runtime that does not hold the same value.
///
/// - `central_issuer_principal_refs` is `main`'s
///   `TRACE_COMMONS_CREDIT_SETTLEMENT_CENTRAL_ISSUER_PRINCIPAL_REFS`, as
///   `main` parses it. Empty allows, as on `main`.
/// - `issuer_principal_ref` is the principal the pipeline issues credit as
///   (Ruling T15-10), checked against that list in place of `main`'s calling
///   gate worker, and in place of the principal that runs `main`'s live
///   settlement for a paid leg. With no issuer, every `NoveltyUtility` leg
///   is withheld (Zaki review 3, Z3-2); with a non-empty list and no issuer,
///   no payout is made either.
/// - `require_production_gate` is `main`'s
///   `TRACE_COMMONS_NOVELTY_UTILITY_REQUIRE_PRODUCTION_GATE` (Ruling T15-11).
/// - `settlement_allowed_policy_versions` is `main`'s
///   `TRACE_COMMONS_CREDIT_SETTLEMENT_ALLOWED_POLICY_VERSIONS`. Empty allows,
///   as on `main`.
/// - `settlement_require_issuer_approval` is `main`'s
///   `TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_ISSUER_APPROVAL`.
/// - `settlement_require_rollout_smoke_ready` is `main`'s
///   `TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_ROLLOUT_SMOKE_READY`, and
///   `settlement_max_micros_per_account` its
///   `TRACE_COMMONS_CREDIT_SETTLEMENT_MAX_POINTS_PER_ACCOUNT` in
///   microcredits. Either refuses an enabled payout.
/// - `duplicate_controls` is `main`'s perplexity-driver duplicate cost
///   controls ([`PipelineDuplicateControls`]), which a compatibility run's
///   gate decision row and Trace Credit leg apply. `None` applies neither
///   rule; ingest always passes `Some` for an assembled runtime.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PipelineNoveltyUtilityChecks {
    pub central_issuer_principal_refs: std::collections::BTreeSet<String>,
    pub issuer_principal_ref: Option<String>,
    pub require_production_gate: bool,
    pub settlement_allowed_policy_versions: std::collections::BTreeSet<String>,
    pub settlement_require_issuer_approval: bool,
    pub settlement_require_rollout_smoke_ready: bool,
    pub settlement_max_micros_per_account: Option<i64>,
    pub duplicate_controls: Option<PipelineDuplicateControls>,
}

/// `main`'s two duplicate short-circuits in the perplexity-scoring driver
/// (`score_one_submission` in `trace-commons-ingest.rs`), which record a
/// gate decision with credit withheld instead of scoring:
///
/// - `skipped_duplicate`, when `skip_duplicates` and the submission's
///   duplicate score, in micros, is at or above
///   `skip_duplicate_threshold_micros`
///   (`TRACE_COMMONS_PERPLEXITY_DRIVER_SKIP_DUPLICATES`, default `true`, and
///   `TRACE_COMMONS_PERPLEXITY_DRIVER_SKIP_DUPLICATE_THRESHOLD_MICROS`,
///   default `900000`);
/// - `cached`, unconditionally, when another submission with the same
///   canonical summary hash already has a gate decision.
///
/// The contributor status reads either label as "duplicates an earlier
/// submission" and shows no pending credit. A compatibility run, and a run
/// under any Score policy a registry supplies, applies both, in that order,
/// to its gate decision row
/// (`PgPipelineStore::duplicate_withheld_reason_on_tx`): the row keeps its
/// Score values, records the label, and holds no credit quality, and the
/// run's Trace Credit leg is withheld under the same label whichever event
/// it settles under, as `main` never emits that event for a submission it
/// did not score, and the run is
/// kept out of the index, as `main` never indexes a submission it did not
/// score. Settle decides the verdict once, when it persists its selection
/// (`PgPipelineStore::duplicate_verdict_for_selection`). The Score evidence
/// and the bundle are unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineDuplicateControls {
    pub skip_duplicates: bool,
    pub skip_duplicate_threshold_micros: i64,
}

impl PipelineDuplicateControls {
    /// `main`'s defaults, with neither variable set.
    pub const MAIN_DEFAULT: Self = Self {
        skip_duplicates: true,
        skip_duplicate_threshold_micros: 900_000,
    };
}

/// The submission-operability check Settle runs before deciding index
/// membership and again immediately before it dispatches to the index
/// (`PipelineService::submission_guard`).
#[derive(Debug, Clone, Copy)]
pub struct SubmissionGuard {
    pub operable: bool,
}

/// Whether each held dependency is production-qualified, across *every*
/// dependency the service holds -- regardless of whether any bundle actually
/// uses it. `scorer` and `embedder` are true only when every scorer/embedder
/// the service holds is (decision P4 as first shipped); a bundle can name
/// any one of them by content hash, so a single unqualified reference
/// dependency disqualifies the whole set. `authority` and `privacy` follow
/// the same shape (Ruling T2-2): true only when the held object is
/// `production_qualified()`, false when the service holds none at all
/// (`submit` already fails closed on that case before any dependency check
/// runs). `payout` is the same for the NEAR payout adapter: true only when
/// the service holds one and it is `production_qualified()`.
///
/// Startup does not read this field-by-field check: see
/// [`PipelineService::bundle_qualification`], which scopes qualification to
/// the one bundle a runtime actually starts (decision P4-D7), and whose
/// `configuration_qualifiable` term carries the bundle's configuration
/// (wave 2; the `bundle` field this struct had is gone). The
/// `NoveltyUtility` production-gate check in `novelty_utility_withheld_reason`
/// still reads this method directly -- that check is about every dependency
/// the service could ever route a compatibility receipt to, not one bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineDependencyQualification {
    pub scorer: bool,
    pub embedder: bool,
    pub index_reader: bool,
    pub index_writer: bool,
    pub settlement_adapters: BTreeMap<String, bool>,
    pub authority: bool,
    pub privacy: bool,
    pub payout: bool,
}

/// One dependency's identity and production-qualification state, as
/// [`PipelineService::bundle_qualification`] reports it for a single bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PipelineDependencyCheck {
    pub identity: String,
    pub production_qualified: bool,
}

/// Whether every dependency one bundle package actually uses is
/// production-qualified (decision P4-D7). Unlike
/// [`PipelineDependencyQualification`], which reports every dependency the
/// service holds, this reports only: the scorer and embedder the package
/// names among the held dependencies (resolved the same way
/// [`PipelineService::construct`] resolves them); the held index reader and
/// writer (every bundle uses the one pair the service holds -- a bundle does
/// not name these independently); one settlement-adapter check per
/// instrument the package pins (an instrument with no registered adapter
/// reports identity `settlement_adapter_missing`, unqualified); `authority`
/// and `privacy` (unconditional -- every bundle needs both); and `payout`,
/// which is `None` while payout is disabled and otherwise `Some` of the held
/// NEAR adapter's own qualification, for every package: payout pays the
/// complete runs of every bundle, so it is service-wide like the index
/// writer (final review M5). `configuration_qualifiable` is the package's
/// own configuration term: a compatibility package whose configuration is
/// not qualifiable (`CompatibilityBundleConfig::is_qualifiable`: the local
/// reference's all-zero floors, say) is never production-qualified, so
/// neither startup nor `qualify_bundle` accepts it (wave 2; rebase 9
/// review, M2). A package of another family carries no such configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PipelineBundleQualification {
    pub bundle_id: String,
    pub scorer: PipelineDependencyCheck,
    pub embedder: PipelineDependencyCheck,
    pub index_reader: PipelineDependencyCheck,
    pub index_writer: PipelineDependencyCheck,
    pub settlement_adapters: BTreeMap<String, PipelineDependencyCheck>,
    pub authority: bool,
    pub privacy: bool,
    pub payout: Option<bool>,
    pub configuration_qualifiable: bool,
    /// The Score policy a registry supplies for this bundle
    /// (`PipelineServiceBuilder::with_score_policies`): its factory's
    /// identity and `production_qualified()`. `None` for a built-in Score
    /// policy, whose qualification the scorer, embedder, and configuration
    /// terms already carry; omitted from the serialized form then, so a
    /// built-in bundle's qualification serializes as it always has. A Score
    /// implementation the service holds no factory for reports identity
    /// `score_policy_missing`, unqualified.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score_policy: Option<PipelineDependencyCheck>,
    pub dependency_digest: String,
}

impl PipelineBundleQualification {
    /// The safe labels naming every blocked dependency this qualification
    /// found; empty exactly when [`Self::is_production_qualified`] is true.
    pub fn blockers(&self) -> Vec<&'static str> {
        let mut blockers = Vec::new();
        for (check, label) in [
            (&self.scorer, "runtime_scorer_not_production"),
            (&self.embedder, "runtime_embedder_not_production"),
            (&self.index_reader, "runtime_index_reader_not_production"),
            (&self.index_writer, "runtime_index_writer_not_production"),
        ] {
            if !check.production_qualified {
                blockers.push(label);
            }
        }
        if self
            .settlement_adapters
            .values()
            .any(|check| !check.production_qualified)
        {
            blockers.push("runtime_settlement_not_production");
        }
        if !self.authority {
            blockers.push("runtime_authority_not_production");
        }
        if !self.privacy {
            blockers.push("runtime_privacy_not_production");
        }
        if self.payout == Some(false) {
            blockers.push("runtime_payout_not_production");
        }
        if self
            .score_policy
            .as_ref()
            .is_some_and(|check| !check.production_qualified)
        {
            blockers.push(PIPELINE_SCORE_POLICY_NOT_PRODUCTION_LABEL);
        }
        if !self.configuration_qualifiable {
            blockers.push(PIPELINE_BUNDLE_CONFIGURATION_NOT_QUALIFIABLE_LABEL);
        }
        blockers
    }

    pub fn is_production_qualified(&self) -> bool {
        self.blockers().is_empty()
    }
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
    /// The operator's `TRACE_COMMONS_ACCEPT_MEDIUM_RISK_SUBMISSIONS`, as
    /// ingest holds it (`AppState::accept_medium_risk_submissions`). When
    /// `true`, a Medium receipt whose basis records only what redaction
    /// found and removed (`admission_privacy_risk`) is admitted, the narrow
    /// pipeline reading of the setting `main`'s legacy path applies to every
    /// Medium. When `false`, Admission sees exactly
    /// `pipeline_privacy_risk`. The bundle's Admission policy never reads
    /// it.
    pub accept_medium_risk: bool,
    /// The source session `main`'s admission claimed for this upload, as
    /// `(account_id, session_digest)`: the final receipt transaction locks
    /// it and refuses the receipt when it is withdrawn (Zaki review 1,
    /// item 5). `None` when admission claimed none.
    pub source_session: Option<(Uuid, [u8; 32])>,
}

/// The outcome of a receipt submission.
#[derive(Debug)]
pub enum PipelineReceiptResult {
    Created(PipelineRunRecord),
    Replayed(PipelineRunRecord),
    ContentConflict,
    Tombstoned,
    QuotaExceeded(PipelineQuotaScope),
    /// The upload's source session was withdrawn before the receipt
    /// committed: nothing is recorded, as `main`'s receipt refuses it
    /// (`source_session_withdrawn`).
    SourceSessionWithdrawn,
    /// The legacy path owns the submission id: its ownership row says so, or
    /// a legacy submission row holds the id. The pipeline never gives such an
    /// id a run, and the receipt stored nothing
    /// (`PIPELINE_LEGACY_RECEIPT_OWNED_LABEL`). A contained tenant is
    /// answered `NotRouted(Contained)` before this, whoever owns the id.
    LegacyOwned,
    /// The tenant's routing does not send new receipts to the pipeline: its
    /// committed routing row is `legacy` or `contained`, or it has no row and
    /// the service does not allow unqualified routing
    /// (`PipelineServiceBuilder::with_unqualified_routing`), which reads as
    /// `Legacy`. The receipt stored nothing. A retry of a receipt that
    /// already has a run is never answered this way: it replays in every
    /// routing state.
    NotRouted(RoutingState),
}

/// `PipelineService::replay_receipt`'s result: the outcome for the retried
/// key, plus the principal the run was originally created under, so a
/// caller can check ownership before acting on either outcome.
#[derive(Debug)]
pub struct PipelineReplayReceipt {
    pub result: PipelineReceiptResult,
    pub auth_principal_ref: String,
}

/// A run's full state: the run row, its recorded phase outcomes, and its
/// settlements.
pub struct PipelineInspection {
    pub run: PipelineRunRecord,
    pub outcomes: Vec<PhaseOutcomeRecord>,
    pub settlements: Vec<PipelineSettlementRecord>,
}

/// What one pass of [`PipelineService::rebuild_index_from_authoritative_commands`]
/// did: how many sealed commands it replayed, how many entries it upserted
/// across all of them, how many of those the writer already held
/// (`IndexUpsertResult::Unchanged`), how many listed runs it skipped
/// because, re-checked under their own lock, they were no longer operable
/// (a withdrawal or an invalidation that committed after the listing), and
/// a hash of the exact set of command hashes it rebuilt from -- so a caller
/// can tell two rebuilds apart, or confirm a repeat rebuilt from the same
/// authoritative set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineIndexRebuildReport {
    pub command_count: usize,
    pub entry_count: usize,
    pub unchanged_entry_count: usize,
    pub skipped_run_count: usize,
    pub command_set_hash: String,
}

/// What `PipelineService::rebuild_index_run` did for one listed run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineIndexRebuildRun {
    /// The run's sealed command was replayed: its hash, the entries upserted,
    /// and how many of them the writer already held.
    Rebuilt {
        command_hash: String,
        entry_count: usize,
        unchanged_entry_count: usize,
    },
    /// Re-checked under its lock, the run was no longer operable; nothing was
    /// written for it.
    Skipped,
}

/// The outbox lines a payout pass changed, by what changed them. A statement
/// that changed no row counts nothing: two replicas can poll one line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PipelinePayoutTally {
    /// Lines a submit moved to `submitted`.
    pub submitted: u64,
    /// Lines a refused submit moved to `failed`.
    pub submit_failed: u64,
    /// Lines the confirmation moved to `confirmed`.
    pub confirmed: u64,
    /// Lines the chain reported as failed, moved to `failed`.
    pub chain_failed: u64,
}

/// One of the (at most three) objects a pipeline phase attempt writes under
/// its own lease-token key (`pipeline_attempt_object_id`): Review's
/// `approved` content, and Score's `index-command` and `score-neighbors`
/// (PR 4). `as_str` is the exact string the `artifact` column of
/// `pipeline_attempt_artifacts` stores and `pipeline_attempt_object_id`
/// embeds in the object id; `from_db_str` is its inverse for a reader that
/// only has the column's value, and `store_kind` is the `TraceArtifactKind`
/// its write site stores it under (`ContributionEnvelope` for `approved`
/// and `privacy-pass`, `VectorPayload` for the two Score artifacts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineAttemptArtifact {
    Approved,
    IndexCommand,
    ScoreNeighbors,
    /// The Review-start privacy pass's output (`record_privacy_pass`).
    PrivacyPass,
}

impl PipelineAttemptArtifact {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::IndexCommand => "index-command",
            Self::ScoreNeighbors => "score-neighbors",
            Self::PrivacyPass => "privacy-pass",
        }
    }

    pub(super) fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "approved" => Some(Self::Approved),
            "index-command" => Some(Self::IndexCommand),
            "score-neighbors" => Some(Self::ScoreNeighbors),
            "privacy-pass" => Some(Self::PrivacyPass),
            _ => None,
        }
    }

    pub(super) fn store_kind(&self) -> TraceArtifactKind {
        match self {
            Self::Approved | Self::PrivacyPass => TraceArtifactKind::ContributionEnvelope,
            Self::IndexCommand | Self::ScoreNeighbors => TraceArtifactKind::VectorPayload,
        }
    }
}

/// What one attempt sweep pass did
/// (`PipelineService::sweep_attempt_artifacts_from`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptSweepPass {
    /// How many rows it removed.
    pub removed: usize,
    /// Where the next pass resumes; `None` to start at the oldest due row.
    pub resume_after: Option<AttemptSweepCursor>,
}
