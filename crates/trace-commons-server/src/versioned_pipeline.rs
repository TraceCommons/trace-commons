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
use trace_commons_gate_api::pipeline::{
    AdmissionDecision, AdmissionEvaluation, AdmissionEvidence, AdmissionInput, AtomicUnits,
    BundlePackage, HumanReviewAssessment, IndexMembershipDecision, InstrumentAward,
    InstrumentAwards, InstrumentId, InstrumentSettlement, InstrumentSettlementProgress,
    Microcredits, PIPELINE_OUTCOME_SCHEMA_ID, PIPELINE_OUTCOME_SCHEMA_VERSION, Phase, PhaseResult,
    PolicyError, PrivacyRisk, ReasonCode, ReviewDecision, ReviewEvaluation, ReviewEvidence,
    ReviewInput, ReviewRecommendation, SchemaRef, ScoreDecision, ScoreEvaluation, ScoreEvidence,
    ScoreInput, SealedIndexCommand, SettleDecision, SettleEvaluation, SettleEvidence, SettleInput,
    TenantStorageRef, UnverifiedScoreDecision,
};
use trace_commons_gate_api::{
    IdentifiedEmbedder, IdentifiedIndexReader, IdentifiedIndexWriter, IdentifiedPerplexityScorer,
    IndexWriteError, SettlementError, SettlementReceipt, SettlementRequest,
};
use trace_commons_protocol::trace_contribution::{
    ConsentScope, ResidualPiiRisk, ResidualRiskCondition, TraceAllowedUse,
    TraceContributionEnvelope, retention_policy_for_trace,
};
use uuid::Uuid;

use crate::db::postgres::PgBackend;
use crate::db::{
    insert_credit_settlement_batch_on_tx, list_trace_credit_holds_on_tx,
    record_source_submission_withdrawal_on_tx, withdraw_source_session_on_tx,
};
use crate::error::DatabaseError;
use crate::trace_artifact_store::{
    EncryptedTraceArtifactReceipt, TraceArtifactKind, TraceArtifactStore,
};
use crate::trace_corpus_storage::{
    TraceCorpusStatus, TraceCorpusStore, TraceCreditAccountSettlementLineItem,
    TraceCreditEventType, TraceCreditSettlementBatchStatus, TraceCreditSettlementBatchWrite,
    TraceCreditSettlementNearStatus, TraceObjectArtifactKind, TraceObjectRefWrite,
    TraceSubmissionWrite, safe_residual_risk_basis_labels,
};
use crate::versioned_pipeline_authority::{
    PIPELINE_AUTHORITY_CONTROL_MISSING_LABEL, PIPELINE_PRIVACY_CLASSIFICATION_FAILED_LABEL,
    PIPELINE_PRIVACY_CONTROL_MISSING_LABEL, PipelineAuthorityProvider, PipelinePrivacyBoundary,
};
use crate::versioned_pipeline_bundle::{
    MinimalPolicyBundle, PIPELINE_BUNDLE_INVALID_LABEL, PIPELINE_DEPENDENCY_MISSING_LABEL,
    PIPELINE_REVIEW_ASSESSMENT_REQUIRED_LABEL, PipelineTraceCreditEvent, dependency_content_hash,
    pipeline_operation_ref, pipeline_result_ref,
};
use crate::versioned_pipeline_credit::{
    NearPayoutAdapter, PIPELINE_CREDIT_ACTOR_ROLE, PIPELINE_CREDIT_REASON,
    PIPELINE_NOVELTY_UTILITY_ACTOR_ROLE, PIPELINE_SETTLEMENT_POLICY_VERSION,
    SettlementAdapterRegistry, credit_account_hash, disabled_near_call,
    microcredits_to_settled_i64, payout_state_label, pipeline_credit_event_id,
    pipeline_ledger_source_key, pipeline_near_outbox_line_id, pipeline_novelty_utility_reason,
    pipeline_settlement_batch_id, source_list_hash,
};

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
/// A Trace Credit leg's payout label when the NEAR adapter refused or
/// failed its submit; the payout is then `failed`.
pub const PIPELINE_NEAR_SUBMIT_FAILED_LABEL: &str = "near_submit_failed";
/// Payout labels for an error in one run's payout (Ruling T10-5): the
/// payout pass records it on that run's leg, which ends `failed`, and goes
/// on with the next run. `payout_operation_failed` covers any error without
/// a label of its own.
pub const PIPELINE_PAYOUT_BATCH_MISSING_LABEL: &str = "payout_batch_missing";
pub const PIPELINE_NEAR_CALL_INVALID_LABEL: &str = "near_call_invalid";
pub const PIPELINE_NEAR_CONFIRMATION_INVALID_LABEL: &str = "near_confirmation_invalid";
pub const PIPELINE_PAYOUT_OPERATION_FAILED_LABEL: &str = "payout_operation_failed";
/// A payout line whose stored call names a NEAR contract other than the
/// configured one: it is never submitted again (Finding I2).
pub const PIPELINE_NEAR_CONTRACT_CHANGED_LABEL: &str = "near_contract_changed";
/// `PipelineService::process_payout`'s refusal while another payout pass, or
/// `main`'s NEAR outbox submitter, holds the tenant's NEAR submit lock
/// (Finding I1).
pub const PIPELINE_PAYOUT_LOCK_HELD_LABEL: &str = "payout_lock_held";
/// `PipelineServiceBuilder::build`'s refusals of an enabled payout without
/// a usable NEAR contract (Ruling T10-4).
pub const PIPELINE_PAYOUT_NEAR_CONTRACT_MISSING_LABEL: &str = "payout_near_contract_missing";
pub const PIPELINE_PAYOUT_NEAR_CONTRACT_INVALID_LABEL: &str = "payout_near_contract_invalid";
/// `main`'s labels for its settlement controls (Zaki review 1, item 2), as
/// its credit settlement drill reports them: an enabled payout is refused
/// while `main` requires issuer approval, and a payout line is not sent when
/// its batch's policy version is not on `main`'s allowed list. A paid leg
/// whose issuer is not on `main`'s central-issuer allowlist records
/// `PIPELINE_NOVELTY_UTILITY_CENTRAL_ISSUER_DENIED_LABEL`, as the
/// `NoveltyUtility` leg does.
pub const PIPELINE_PAYOUT_ISSUER_APPROVAL_MISSING_LABEL: &str =
    "issuer_approval_evidence_hash_missing";
pub const PIPELINE_PAYOUT_POLICY_VERSION_NOT_ALLOWED_LABEL: &str =
    "credit_settlement_policy_version_not_allowed";
pub const PIPELINE_TOMBSTONE_LABEL: &str = "content_tombstoned";
/// The reason code of an index invalidation a withdrawal queues.
const PIPELINE_WITHDRAWAL_INVALIDATION_REASON: &str = "withdrawn";
/// An index invalidation attempt that did not remove the revision, while
/// the invalidation stays `pending` and is retried: an index outage (the
/// index answered `Failed` or `Uncertain`; uncharged), or a charged failure
/// with an attempt left.
pub const PIPELINE_INDEX_INVALIDATION_UNAVAILABLE_LABEL: &str = "index_invalidation_unavailable";
/// An index invalidation whose attempts ran out on failures that waiting
/// cannot heal (`PgPipelineStore::fail_index_invalidation`): it is `failed`
/// for good, and the revision's entries may still be in the index.
pub const PIPELINE_INDEX_INVALIDATION_FAILED_LABEL: &str = "index_invalidation_failed";
pub const PIPELINE_BUNDLE_STORE_UNAVAILABLE_LABEL: &str = "bundle_store_unavailable";
/// The `trace_object_refs.object_store` label a service records when its
/// builder is not told the configured store's name
/// (`PipelineServiceBuilder::with_object_store_name`). Ingest's assembly
/// refuses a service that still carries it (M11).
pub const PIPELINE_DEFAULT_OBJECT_STORE_NAME: &str = "pipeline_local_encrypted";
pub const INJECTED_PIPELINE_CRASH: &str = "injected_pipeline_crash";
const DEFAULT_RETRY_MILLISECONDS: i64 = 50;
/// Label for an adapter call error or a charged settlement blocker; the leg
/// waits and the run retries (`complete_settle_phase`).
const PIPELINE_SETTLEMENT_RETRY_LABEL: &str = "settlement_operation_retry";
/// Label for a settlement adapter the service does not hold.
const PIPELINE_SETTLEMENT_ADAPTER_MISSING_LABEL: &str = "settlement_adapter_missing";
/// A leg whose instrument has no configured per-instrument cap is a
/// configuration gap, not the trace's fault: an uncharged suspension,
/// raised before the leg's adapter is called. An amount over a configured
/// cap stays `PIPELINE_CREDIT_CAP_LABEL`.
pub const PIPELINE_SETTLEMENT_CAP_MISSING_LABEL: &str = "settlement_cap_missing";
/// A transient database failure (`is_transient_database_error`) in any
/// phase. An uncharged suspension.
pub const PIPELINE_DATABASE_UNAVAILABLE_LABEL: &str = "database_unavailable";
/// The label on a leg a failed run forfeits -- one that was
/// never dispatched, or a Trace Credit leg (whose only effect is the ledger
/// row that commits with its completion, so an incomplete one paid nothing).
pub const PIPELINE_SETTLEMENT_RUN_FAILED_LABEL: &str = "run_failed";
/// The label on a dispatched external leg of a failed run whose
/// outcome was not reconciled -- its reconciling adapter call returned a
/// different result, errored, or was not made (a missing adapter or cap, an
/// inoperable submission, or the claim sweep, which has no worker). The
/// external effect may have happened; an operator reconciles it against the
/// adapter by `operation_ref_hash`.
pub const PIPELINE_SETTLEMENT_UNRECONCILED_LABEL: &str = "settlement_unreconciled";
/// The label on a leg whose adapter returned a receipt that does not answer
/// the leg's request: a result reference other than the one the persisted
/// selection expects, or an external receipt that another leg already
/// recorded (one external receipt answers one leg). The leg is never
/// dispatched again. When its run fails, a leg of an external instrument
/// stays `failed` with this label, since its effect is unknown, for an
/// operator to reconcile by `operation_ref_hash`. A Trace Credit leg is
/// forfeited as `run_failed` instead: it pays only through the ledger row
/// that commits with its completion, and a mismatch writes no ledger row.
pub const PIPELINE_SETTLEMENT_RESULT_MISMATCH_LABEL: &str = "settlement_result_mismatch";
/// The label on a leg whose settlement request cannot be formed:
/// `SettlementRequest::new` refuses its references or its amount. No adapter
/// is called, and the run's retry is charged.
pub const PIPELINE_SETTLEMENT_REQUEST_INVALID_LABEL: &str = "settlement_request_invalid";
/// The labels of a `failed` leg that is never dispatched again: Step 6 skips
/// it, and the reconciling call a worker makes before it fails the run never
/// calls the adapter for it. `settlement_result_mismatch` is a receipt that
/// did not answer the request. The other two are the adapter's own
/// `SettlementError::Conflict` and `SettlementError::Rejected` labels: no
/// effect happened, and the operation must not be retried.
const TERMINAL_SETTLEMENT_FAILURE_LABELS: [&str; 3] = [
    PIPELINE_SETTLEMENT_RESULT_MISMATCH_LABEL,
    "settlement_request_conflict",
    "settlement_request_rejected",
];
/// The refusal a leg write (`mark_settlement_leased`,
/// `record_settlement_reconciliation`) returns under a lease that is still
/// live when the leg is not in a state that write may change, or does not
/// exist. Deliberately not the stale-lease error: `process_claimed_run`
/// records a stale lease as the uncharged `lease_expired`, and a leg-state
/// refusal is not a lease problem.
const PIPELINE_SETTLEMENT_LEG_NOT_OPEN_LABEL: &str = "settlement_leg_not_open";
/// A receipt attempt that reached its final transaction after
/// its staging row's `cleanup_after`: the sweeper removed the row, or may
/// have deleted its object. The attempt deletes its own object and fails; a
/// retry is a new attempt.
const PIPELINE_RECEIPT_STAGING_MISSING_LABEL: &str = "receipt_staging_missing";
/// The artifact store wrote a receipt object other than the
/// one it prepared, so the staging row does not name it. Fails closed.
const PIPELINE_RECEIPT_OBJECT_MISMATCH_LABEL: &str = "receipt_object_mismatch";
/// The blocking thread a receipt's object-store call was moved onto
/// (`tokio::task::spawn_blocking`) panicked or was cancelled before it
/// returned. Fails closed rather than treating a lost result as success or
/// as an ordinary store error.
const PIPELINE_RECEIPT_OBJECT_TASK_FAILED_LABEL: &str = "receipt_object_task_failed";

/// The SQL form of `settlement_leg_is_unresolved`, over a row aliased `s`.
/// `COALESCE` makes a `failed` leg with no label unresolved, as in Rust: a
/// comparison with a `NULL` label is `NULL`, which would otherwise leave the
/// leg matched by neither this predicate nor its negation.
const UNRESOLVED_SETTLEMENT_LEG_SQL: &str = "(s.operation_state NOT IN ('complete', 'forfeited')
      AND NOT (
          s.operation_state = 'failed'
          AND COALESCE(
              s.last_error_label = 'settlement_unreconciled'
              OR (
                  s.last_error_label = 'settlement_result_mismatch'
                  AND s.instrument_id <> 'trace_credit'
              ),
              FALSE
          )
      ))";

/// Whether a failure path still has to resolve a leg. `complete` and
/// `forfeited` never change. A leg `failed` as `settlement_unreconciled` is
/// resolved: its adapter may have taken effect, and only its records can
/// say. So is a leg of an external instrument `failed` as
/// `settlement_result_mismatch`. A Trace Credit leg `failed` as
/// `settlement_result_mismatch` is unresolved: no ledger row was written, so
/// nothing was paid, and the failure path forfeits it as `run_failed`. A leg
/// `failed` as `settlement_request_conflict` or `settlement_request_rejected`
/// is unresolved: no effect happened, and the failure path forfeits it under
/// its own label. A leg `failed` for any other reason (an amount over the
/// cap, a request that cannot be formed, or no label at all) is one Step 6
/// retries, so it is unresolved. `UNRESOLVED_SETTLEMENT_LEG_SQL` is the same
/// rule in SQL.
fn settlement_leg_is_unresolved(
    instrument_id: &str,
    operation_state: &str,
    last_error_label: Option<&str>,
) -> bool {
    match operation_state {
        "complete" | "forfeited" => false,
        "failed" => match last_error_label {
            Some(PIPELINE_SETTLEMENT_UNRECONCILED_LABEL) => false,
            Some(PIPELINE_SETTLEMENT_RESULT_MISMATCH_LABEL) => {
                instrument_id == InstrumentId::trace_credit().as_str()
            }
            _ => true,
        },
        _ => true,
    }
}

/// The SQL form of `settlement_leg_is_open_to_dispatch`, over a row aliased
/// `s`. `COALESCE` makes a `failed` leg with no label open to dispatch, as in
/// Rust.
const SETTLEMENT_LEG_OPEN_TO_DISPATCH_SQL: &str =
    "(s.operation_state NOT IN ('complete', 'forfeited')
      AND NOT (
          s.operation_state = 'failed'
          AND COALESCE(
              s.last_error_label IN (
                  'settlement_result_mismatch',
                  'settlement_request_conflict',
                  'settlement_request_rejected'
              ),
              FALSE
          )
      ))";

/// Whether an adapter call may still be made for a leg. `complete` and
/// `forfeited` legs are done, and a `failed` leg with a label in
/// `TERMINAL_SETTLEMENT_FAILURE_LABELS` is never dispatched again. Every
/// other leg is dispatched by Step 6, including a `settlement_unreconciled`
/// leg on a run that is still live (a failure path interrupted by a stale
/// lease or a database error before it failed the run): the run did not
/// fail, so the leg is tried again. `SETTLEMENT_LEG_OPEN_TO_DISPATCH_SQL` is
/// the same rule in SQL.
fn settlement_leg_is_open_to_dispatch(
    operation_state: &str,
    last_error_label: Option<&str>,
) -> bool {
    match operation_state {
        "complete" | "forfeited" => false,
        "failed" => !last_error_label
            .is_some_and(|label| TERMINAL_SETTLEMENT_FAILURE_LABELS.contains(&label)),
        _ => true,
    }
}

/// The unique index that holds one external receipt to one leg per tenant.
const EXTERNAL_RECEIPT_UNIQUE_INDEX: &str = "pipeline_run_settlements_external_receipt_unique";

/// Whether `error` is the database refusing an external receipt hash that
/// another leg of the tenant already recorded.
fn is_external_receipt_reuse(error: &tokio_postgres::Error) -> bool {
    error.as_db_error().is_some_and(|db| {
        db.code() == &tokio_postgres::error::SqlState::UNIQUE_VIOLATION
            && db.constraint() == Some(EXTERNAL_RECEIPT_UNIQUE_INDEX)
    })
}

/// `is_external_receipt_reuse` for an error from a leg write, however the
/// caller wrapped it.
fn is_external_receipt_reuse_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<tokio_postgres::Error>()
            .is_some_and(is_external_receipt_reuse)
            || matches!(
                cause.downcast_ref::<DatabaseError>(),
                Some(DatabaseError::Postgres(inner)) if is_external_receipt_reuse(inner)
            )
    })
}

/// An attempt whose own phase lease has already gone stale, discovered
/// when *that same attempt* -- the worker still holding its own claim's
/// lease token -- writes again (the phase's own commit, or a follow-up
/// `mark_retry`/`mark_transient_retry`/`mark_failed` after the phase failed
/// for another reason) and finds its lease has already expired. Recorded by
/// `PgPipelineStore::record_lease_expired` and never charged as
/// `PIPELINE_OPERATIONAL_ERROR_LABEL` or any other P2 label.
///
/// This is *not* what happens to a worker that crashes mid-phase: a crashed
/// worker never comes back to write anything, so
/// nothing is recorded for it. Its run stays `leased` until
/// `lease_expires_at` passes, then the next claim -- by any worker --
/// reclaims it as an ordinary charged attempt (`attempt_count += 1`,
/// `last_error_label` cleared); enough crashes still exhaust `max_attempts`
/// and the sweep in `claim_next` marks the run `failed`/`attempts_exhausted`
/// with nothing recording why. The same gap applies with more than one
/// worker racing the same run: if a second worker reclaims the lease before
/// the first worker's own stale write runs, `record_lease_expired`'s
/// token-only fence finds no row under the first worker's now-superseded
/// token and changes nothing -- that attempt is silently lost, recorded
/// neither as `lease_expired` nor as a charge reversed. Lease renewal (PR 4)
/// is what closes both gaps, by extending a live lease before it expires
/// rather than discovering the expiry after the fact.
pub const PIPELINE_LEASE_EXPIRED_LABEL: &str = "lease_expired";
/// Safe label `PipelineLeaseConfig::new` refuses with when a phase's
/// configured lease falls outside [1 second, 2 hours].
pub const PIPELINE_LEASE_CONFIG_INVALID_LABEL: &str = "pipeline_lease_config_invalid";
const PIPELINE_LEASE_MIN_SECONDS: i64 = 1;
const PIPELINE_LEASE_MAX_SECONDS: i64 = 2 * 60 * 60;
const PIPELINE_LEASE_DEFAULT_REVIEW_SECONDS: i64 = 5 * 60;
const PIPELINE_LEASE_DEFAULT_SCORE_SECONDS: i64 = 30 * 60;
const PIPELINE_LEASE_DEFAULT_SETTLE_SECONDS: i64 = 5 * 60;

/// Each pipeline phase gets
/// its own claim-lease length, sized for how long that phase can actually
/// run rather than one fixed lease every phase shared. Score, in
/// particular, runs the injected scorer and the embedder inside the lease
/// -- a chunked NEAR AI perplexity scorer or a CPU-bound embedder routinely
/// exceeds a short fixed lease, which used to make `commit_score` and the
/// following `mark_retry` both fail on a stale lease and burn an attempt
/// every time. The defaults below give Score six times the headroom Review
/// and Settle get. Lease renewal mid-phase (extending a lease the phase
/// still holds) is PR 4 and out of scope here -- this only sizes the one
/// lease a phase gets when it is claimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineLeaseConfig {
    review: Duration,
    score: Duration,
    settle: Duration,
}

impl PipelineLeaseConfig {
    /// Refuses any of the three durations outside [1 second, 2 hours]
    /// inclusive, with the safe label `PIPELINE_LEASE_CONFIG_INVALID_LABEL`
    /// (fail closed: an operator typo never silently becomes an unbounded or
    /// zero-length lease).
    pub fn new(review: Duration, score: Duration, settle: Duration) -> anyhow::Result<Self> {
        let min = Duration::seconds(PIPELINE_LEASE_MIN_SECONDS);
        let max = Duration::seconds(PIPELINE_LEASE_MAX_SECONDS);
        for candidate in [review, score, settle] {
            anyhow::ensure!(
                candidate >= min && candidate <= max,
                PIPELINE_LEASE_CONFIG_INVALID_LABEL
            );
        }
        Ok(Self {
            review,
            score,
            settle,
        })
    }

    pub fn review(&self) -> Duration {
        self.review
    }

    pub fn score(&self) -> Duration {
        self.score
    }

    pub fn settle(&self) -> Duration {
        self.settle
    }
}

impl Default for PipelineLeaseConfig {
    /// Review 5 minutes, Score 30 minutes, Settle 5 minutes: defaults that
    /// fit a slow Score without operator configuration.
    fn default() -> Self {
        Self {
            review: Duration::seconds(PIPELINE_LEASE_DEFAULT_REVIEW_SECONDS),
            score: Duration::seconds(PIPELINE_LEASE_DEFAULT_SCORE_SECONDS),
            settle: Duration::seconds(PIPELINE_LEASE_DEFAULT_SETTLE_SECONDS),
        }
    }
}

/// `pub(crate)`: `versioned_pipeline_product.rs` reuses this instead of
/// holding a second copy.
pub(crate) fn sha256_prefixed(bytes: &[u8]) -> String {
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PipelineRunState {
    Pending,
    Leased,
    Retry,
    /// A run Review quarantined with no human assessment yet
    /// (`PgPipelineStore::mark_awaiting_review`, label
    /// `review_assessment_required`). No claim query selects this state, so
    /// the run does not retry hourly forever while it waits.
    /// `PgPipelineStore::record_review_assessment` moves it back to
    /// `Pending`, due at once, once an assessment lands (Ruling T3-3); a
    /// parked run whose submission has since become inoperable is released
    /// the same way by `claim_review` or `record_review_assessment`
    /// themselves (Ruling T3-6).
    AwaitingReview,
    Complete,
    Failed,
}

impl PipelineRunState {
    fn as_db(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Leased => "leased",
            Self::Retry => "retry",
            Self::AwaitingReview => "awaiting_review",
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
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
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

/// A worker's exclusive, time-boxed claim on one queued index invalidation
/// (`PgPipelineStore::claim_index_invalidation`). The claim moves the row's
/// `next_attempt_at` to `lease_expires_at`, so no other claim gets the row
/// before then. `attempt_count` is the number of attempts charged before
/// this one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineIndexInvalidationClaim {
    pub tenant_id: String,
    pub run_id: Uuid,
    pub registry_revision_id: Uuid,
    pub attempt_count: u32,
    pub lease_expires_at: DateTime<Utc>,
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
    /// Diagnostic count of dispatches that ended `retry` or `failed`; the
    /// run's own attempt budget governs retries.
    pub attempt_count: u32,
    pub last_error_label: Option<String>,
    /// When the leg was first leased for an adapter call; never
    /// cleared. A failed run reconciles a dispatched external leg rather
    /// than forfeiting it.
    pub dispatched_at: Option<DateTime<Utc>>,
}

/// The result of `PipelineService::settle_internal_credit`: the Trace Credit
/// leg settled (its ledger row, the finalized batch that carries it, and
/// the completed settlement row committed together), or a hold on the
/// account, found under the account lock, rolled the credit transaction
/// back -- the caller then records the row as `held` and no ledger row is
/// written -- or the submission stopped being operable under that same
/// lock, which also rolls the transaction back with nothing written.
enum InternalCreditResult {
    Complete,
    Held,
    /// The submission-operability re-check taken under the
    /// submission row's own lock, inside this same transaction, found the
    /// submission no longer operable (withdrawn, revoked, purged, expired,
    /// or no longer `accepted`). The transaction rolled back with nothing
    /// written -- no ledger row, no batch.
    Inoperable,
}

/// The settlement-row update for a leg an active credit hold keeps waiting.
fn held_settlement_update() -> SettlementUpdate<'static> {
    SettlementUpdate {
        operation_state: "held",
        result_ref_hash: None,
        external_receipt_hash: None,
        credit_event_id: None,
        settlement_batch_id: None,
        payout_state: None,
        error_label: Some(PIPELINE_CREDIT_HELD_LABEL),
    }
}

/// The settlement-row update for a leg that fails closed with `label`.
fn failed_settlement_update(label: &str) -> SettlementUpdate<'_> {
    SettlementUpdate {
        operation_state: "failed",
        result_ref_hash: None,
        external_receipt_hash: None,
        credit_event_id: None,
        settlement_batch_id: None,
        payout_state: None,
        error_label: Some(label),
    }
}

/// The settlement-row update for a leg forfeited with `label`.
fn forfeited_settlement_update(label: &str) -> SettlementUpdate<'_> {
    SettlementUpdate {
        operation_state: "forfeited",
        result_ref_hash: None,
        external_receipt_hash: None,
        credit_event_id: None,
        settlement_batch_id: None,
        payout_state: None,
        error_label: Some(label),
    }
}

/// The label a leg gets when the submission it belongs to stops being
/// operable during Settle (withdrawn, revoked, purged, or expired) and the
/// leg is forfeited without a further adapter call. A dispatched leg of an
/// external instrument (anything but `trace_credit`) may have taken effect
/// -- its adapter was called at least once -- so it is flagged
/// `settlement_unreconciled` instead of `submission_inoperable`: an operator
/// reconciles it by hand against the adapter's own records by
/// `operation_ref_hash`; nothing in this release reconciles it
/// automatically. The exception is a leg whose own label already says no
/// effect happened (the adapter's own `Conflict`/`Rejected` answer): it
/// keeps `submission_inoperable`. An undispatched leg had no effect, and a
/// Trace Credit leg pays only through the ledger row that commits with its
/// completion, so both of those keep `submission_inoperable` too.
fn withdrawal_forfeit_label(settlement: &PipelineSettlementRecord) -> &'static str {
    let already_no_effect = matches!(
        settlement.last_error_label.as_deref(),
        Some(label)
            if label == SettlementError::Conflict.label()
                || label == SettlementError::Rejected.label()
    );
    if settlement.instrument_id != InstrumentId::trace_credit().as_str()
        && settlement.dispatched_at.is_some()
        && !already_no_effect
    {
        PIPELINE_SETTLEMENT_UNRECONCILED_LABEL
    } else {
        PIPELINE_SUBMISSION_INOPERABLE_LABEL
    }
}

/// The settlement-row update for a leg forfeited because its submission
/// stopped being operable during Settle. See `withdrawal_forfeit_label` for
/// the label rule; both of Step 6's withdrawal-forfeit branches in
/// `complete_settle_phase` share this one function rather than each
/// picking a label on its own.
fn withdrawal_forfeited_settlement_update(
    settlement: &PipelineSettlementRecord,
) -> SettlementUpdate<'static> {
    forfeited_settlement_update(withdrawal_forfeit_label(settlement))
}

/// `PgPipelineStore::update_settlement`'s per-call update. `credit_event_id`
/// and `settlement_batch_id` only ever move from `NULL` to `Some` (a `None`
/// here leaves whatever is already stored); `payout_state` similarly leaves
/// the column unchanged when `None`. `result_ref_hash` and
/// `external_receipt_hash` are the exception -- see `update_settlement`'s doc
/// comment for which states force them to `NULL` regardless of what this
/// carries.
struct SettlementUpdate<'a> {
    operation_state: &'a str,
    result_ref_hash: Option<&'a str>,
    external_receipt_hash: Option<&'a str>,
    credit_event_id: Option<Uuid>,
    settlement_batch_id: Option<Uuid>,
    payout_state: Option<&'a str>,
    error_label: Option<&'a str>,
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
}

/// A run identity not yet persisted: computed deterministically from the
/// tenant and idempotency key before the receipt's transactions open, and
/// created by the receipt's final transaction.
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

/// One receipt attempt: its own random attempt id, and the
/// object it writes, named -- key and ciphertext hash -- by the prepared
/// receipt before the write. Its `pipeline_receipt_artifacts` row carries
/// exactly these fields.
#[derive(Debug, Clone)]
struct ReceiptAttempt {
    tenant_id: String,
    run_id: Uuid,
    attempt_id: Uuid,
    request_idempotency_key: String,
    request_content_hash: String,
    receipt: EncryptedTraceArtifactReceipt,
}

/// The receipt's staging transaction's outcome
/// (`PipelineService::stage_receipt_attempt`).
enum ReceiptStage {
    /// A refusal: nothing was staged and nothing will be stored.
    Refused(PipelineReceiptResult),
    /// The attempt's row is committed; its object may now be written.
    Staged {
        bundle_id: String,
        bundle: MinimalPolicyBundle,
    },
}

/// The receipt's final transaction's outcome
/// (`PipelineService::commit_receipt_attempt`).
enum ReceiptCommit {
    Created(PipelineRunRecord),
    /// A refusal found by the final re-checks (a run another attempt
    /// created, or a tombstone). This attempt's object is not referenced.
    Refused(PipelineReceiptResult),
    /// This attempt's row is no longer `staged` (the sweeper removed it) or
    /// is due (a sweep that failed may have deleted its object), so it
    /// cannot commit.
    StagingMissing,
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

    /// Switches a tenant to a different, already-registered bundle. This is
    /// an operator action: no ingest route calls it, only
    /// `activate_bundle_if_none` (below) does, and the ingest runtime login
    /// holds no `UPDATE` on `pipeline_active_bundles`, so this call fails
    /// with a database permission error unless it runs through a role that
    /// has been separately granted on the table.
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

    /// Loads the single committed outcome for `phase`, or `None` if the run
    /// has not reached it yet -- one targeted query instead of listing every
    /// phase outcome and filtering in memory.
    pub async fn outcome_for_phase(
        &self,
        tenant_id: &str,
        run_id: Uuid,
        phase: Phase,
    ) -> Result<Option<PhaseOutcomeRecord>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT * FROM phase_outcomes
                 WHERE tenant_id = $1 AND run_id = $2 AND phase = $3",
                &[&tenant_id, &run_id, &phase_as_db(Some(phase))],
            )
            .await?;
        tx.commit().await?;
        row.as_ref().map(phase_outcome_from_row).transpose()
    }

    /// Claims the next due run for `tenant_id` across every phase, sizing
    /// the lease this claim grants by the claimed row's own `next_phase`:
    /// the CASE below picks Review's, Score's, or Settle's configured lease
    /// for the one row the candidate CTE selects, in the same statement
    /// that claims it, so a Score run never gets Review's shorter lease or
    /// vice versa.
    ///
    /// First it sweeps: a run still `leased` after its lease expired with
    /// its attempts exhausted (its worker is gone) is failed as
    /// `attempts_exhausted`. In the same transaction, every open
    /// leg of a swept Settle run is resolved without an adapter call
    /// (`resolve_open_settlement_legs_on_tx`) -- there is no worker to make
    /// one.
    pub async fn claim_next(
        &self,
        tenant_id: &str,
        lease_config: PipelineLeaseConfig,
    ) -> Result<Option<PipelineRunRecord>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let swept = tx
            .query(
                "UPDATE pipeline_runs
                 SET state = 'failed', lease_token = NULL, lease_expires_at = NULL,
                     last_error_label = $2, updated_at = NOW()
                 WHERE tenant_id = $1
                   AND state = 'leased'
                   AND lease_expires_at <= NOW()
                   AND attempt_count >= max_attempts
                 RETURNING run_id, next_phase",
                &[&tenant_id, &PIPELINE_ATTEMPTS_EXHAUSTED_LABEL],
            )
            .await?;
        let swept_settle_runs = swept
            .iter()
            .filter(|row| row.get::<_, &str>("next_phase") == phase_as_db(Some(Phase::Settle)))
            .map(|row| row.get::<_, Uuid>("run_id"))
            .collect::<Vec<_>>();
        resolve_open_settlement_legs_on_tx(&tx, tenant_id, &swept_settle_runs).await?;
        let lease_token = Uuid::new_v4();
        let review_milliseconds = lease_config.review().num_milliseconds();
        let score_milliseconds = lease_config.score().num_milliseconds();
        let settle_milliseconds = lease_config.settle().num_milliseconds();
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
                     lease_expires_at = NOW() + (
                         CASE p.next_phase
                             WHEN 'review' THEN $3::bigint
                             WHEN 'score' THEN $4::bigint
                             WHEN 'settle' THEN $5::bigint
                             ELSE $3::bigint
                         END * INTERVAL '1 millisecond'
                     ),
                     attempt_count = p.attempt_count + 1,
                     last_error_label = NULL,
                     updated_at = NOW()
                 FROM candidate
                 WHERE p.tenant_id = $1 AND p.run_id = candidate.run_id
                 RETURNING p.*",
                &[
                    &tenant_id,
                    &lease_token,
                    &review_milliseconds,
                    &score_milliseconds,
                    &settle_milliseconds,
                ],
            )
            .await?;
        tx.commit().await?;
        row.as_ref().map(pipeline_run_from_row).transpose()
    }

    /// Claims a specific run with an explicit lease length, kept for tests
    /// that need precise control over a claim's own lease (many call this
    /// directly with `chrono::Duration::seconds(30)`).
    /// `PipelineService::process_run` uses `claim_run_with_lease_config`
    /// instead, so it never has to read the run's phase before claiming it.
    /// The bound was 5 minutes; it is now 2 hours so a caller
    /// can hand this the same lease `PipelineLeaseConfig` allows.
    pub async fn claim_run(
        &self,
        tenant_id: &str,
        run_id: Uuid,
        lease_duration: Duration,
    ) -> Result<Option<PipelineRunRecord>, DatabaseError> {
        if lease_duration <= Duration::zero() || lease_duration > Duration::hours(2) {
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

    /// Claims a specific run, picking the granted lease by its own
    /// `next_phase` in the claiming SQL itself -- the same `CASE`
    /// `claim_next` uses, scoped to one `run_id` instead
    /// of scanning the whole tenant queue. `PipelineService::process_run`
    /// uses this rather than reading the run's phase first and handing
    /// `claim_run` an explicit duration: a phase commit landing between an
    /// earlier read and a later claim could otherwise hand out a lease sized
    /// for the phase the row was in at read time, not the one it is
    /// actually claimed for.
    pub async fn claim_run_with_lease_config(
        &self,
        tenant_id: &str,
        run_id: Uuid,
        lease_config: PipelineLeaseConfig,
    ) -> Result<Option<PipelineRunRecord>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let lease_token = Uuid::new_v4();
        let review_milliseconds = lease_config.review().num_milliseconds();
        let score_milliseconds = lease_config.score().num_milliseconds();
        let settle_milliseconds = lease_config.settle().num_milliseconds();
        let row = tx
            .query_opt(
                "UPDATE pipeline_runs p
                 SET state = 'leased',
                     lease_token = $3,
                     lease_expires_at = NOW() + (
                         CASE p.next_phase
                             WHEN 'review' THEN $4::bigint
                             WHEN 'score' THEN $5::bigint
                             WHEN 'settle' THEN $6::bigint
                             ELSE $4::bigint
                         END * INTERVAL '1 millisecond'
                     ),
                     attempt_count = p.attempt_count + 1,
                     last_error_label = NULL,
                     updated_at = NOW()
                 WHERE p.tenant_id = $1 AND p.run_id = $2
                   AND next_phase <> 'none'
                   AND attempt_count < max_attempts
                   AND (
                       (state IN ('pending', 'retry') AND next_attempt_at <= NOW())
                       OR (state = 'leased' AND lease_expires_at <= NOW())
                   )
                 RETURNING p.*",
                &[
                    &tenant_id,
                    &run_id,
                    &lease_token,
                    &review_milliseconds,
                    &score_milliseconds,
                    &settle_milliseconds,
                ],
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

    /// Commits the Review outcome together with its approved-content
    /// provenance and the phase transition, in one transaction (decision
    /// D7). On approval: the approved object ref, its `trace_derived_records`
    /// row, the submission's `accepted` status, the outcome row, and the
    /// run's `next_phase = score` / `approved_*` columns all commit or none
    /// do. On rejection: only the submission's `rejected` status, the
    /// outcome row, and the run's terminal transition commit -- the run's
    /// `approved_*` columns stay NULL, satisfying
    /// `pipeline_runs_approved_content_shape`.
    ///
    /// The submission-operability
    /// guard that gated the source read at the start of Review
    /// (`load_object_bytes`) was checked in an earlier, already-committed
    /// transaction. A withdrawal (or an expiry/purge) landing after that
    /// read and before this commit -- for example while this attempt was
    /// writing the approved object -- must never be reversed by an approval
    /// or rejection racing in behind it. This re-checks operability here,
    /// under the same lock this commit already takes, and refuses the whole
    /// commit (`PIPELINE_SUBMISSION_INOPERABLE_LABEL`) rather than resurrect
    /// the trace: no object ref, no derived record, no status change, no
    /// outcome, no run update.
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
        // Lock order: the run row (`ensure_current_lease`), then the
        // submission row (below) -- held the same way everywhere both are
        // touched.
        ensure_current_lease(&tx, run, lease_token).await?;

        // Fix round 1, finding I2 / Ruling T3-8: this re-check used to
        // inline its own copy of the operability predicate; it now calls
        // the same `review_submission_is_operable` helper `claim_review`
        // and `record_review_assessment` use, so the predicate exists once.
        // Behaviour is unchanged (`!operable || withdrawn`): the helper
        // returns `Ok(false)` for exactly the same two conditions, just
        // short-circuiting the `trace_withdrawals` lookup when the
        // `trace_submissions` row alone already refuses.
        if !review_submission_is_operable(&tx, &run.tenant_id, run.submission_id).await? {
            return Err(DatabaseError::Constraint(
                PIPELINE_SUBMISSION_INOPERABLE_LABEL.to_string(),
            ));
        }

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
                // Defense in depth: the status update stays
                // conditional on the same pre-review states the gate above
                // just re-checked, and must touch exactly one row. The
                // whole transaction has not committed yet, so refusing here
                // still rolls back the two inserts just above.
                let updated_submission = tx
                    .execute(
                        "UPDATE trace_submissions
                         SET status = 'accepted', reviewed_at = NOW(), updated_at = NOW()
                         WHERE tenant_id = $1 AND submission_id = $2
                           AND status IN ('received', 'quarantined')",
                        &[&run.tenant_id, &run.submission_id],
                    )
                    .await?;
                if updated_submission != 1 {
                    return Err(DatabaseError::Constraint(
                        PIPELINE_SUBMISSION_INOPERABLE_LABEL.to_string(),
                    ));
                }
                (
                    Some(approved.revision_id),
                    Some(approved.object_ref.object_ref_id),
                    Some(approved.content_hash.clone()),
                )
            } else {
                let updated_submission = tx
                    .execute(
                        "UPDATE trace_submissions
                         SET status = 'rejected', reviewed_at = NOW(), updated_at = NOW()
                         WHERE tenant_id = $1 AND submission_id = $2
                           AND status IN ('received', 'quarantined')",
                        &[&run.tenant_id, &run.submission_id],
                    )
                    .await?;
                if updated_submission != 1 {
                    return Err(DatabaseError::Constraint(
                        PIPELINE_SUBMISSION_INOPERABLE_LABEL.to_string(),
                    ));
                }
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

    /// A reviewer's exclusive claim on a run quarantined at Review (port
    /// `ef97a459` lines 832 to 892), adapted for Ruling T3-6: before
    /// attempting the claim, this locks the run row and refuses -- releasing
    /// an `awaiting_review` run back to `pending` in the same transaction --
    /// when the run's submission is no longer operable, the same predicate
    /// `commit_review` re-checks (`review_submission_is_operable`). The port
    /// folded its own narrower `trace_withdrawals` check into the claiming
    /// `INSERT ... SELECT`; that check is now this broader one, done first.
    ///
    /// Fix round 1, finding I1: the locking `SELECT` also requires `state IN
    /// ('pending', 'retry', 'awaiting_review')`, `admission_decision =
    /// 'quarantine'`, and no assessment already recorded. Without the state
    /// predicate, a reviewer could claim a run a worker currently holds
    /// `leased` (and then, worse, a run that has already failed -- `failed`
    /// never clears `next_phase`); without the decision predicate, a
    /// reviewer could claim an `Admit` run that is simply waiting its own
    /// automatic Review turn; without the assessment predicate, a reviewer
    /// could re-claim a run that already has one, and a later
    /// `record_review_assessment` would then hit
    /// `pipeline_review_assessments`'s `UNIQUE (tenant_id, run_id)` instead
    /// of refusing cleanly. `FOR UPDATE` on this row also now does the
    /// concurrency work: a worker's `claim_run` (its own `UPDATE ...
    /// RETURNING *`) and this `SELECT` take the same row lock, so whichever
    /// transaction commits first is the one the other sees -- a worker that
    /// already claimed the run makes this `SELECT` see `state = 'leased'`
    /// and refuse; a reviewer's claim or assessment that commits first
    /// makes the worker's `claim_run` (which re-reads state in its own
    /// `WHERE`) see the post-assessment row once it acquires the lock.
    ///
    /// Beyond that, `Ok(None)` still covers two cases a caller cannot tell
    /// apart from the return value alone: the run is not currently eligible
    /// (does not exist, is not at Review, is not `pending`/`retry`/
    /// `awaiting_review`, is not a quarantine, or already has an
    /// assessment); or another reviewer already holds a live claim on an
    /// otherwise-eligible run. Ingest's `pipeline_review_claim_conflict`
    /// resolves the ambiguity by reading the run back, the same shape
    /// `claim_review_lease_handler`'s `review_lease_claim_conflict` uses for
    /// the legacy lease (Ruling T3-9(b): the first case maps to 404, only
    /// the second to 409).
    pub async fn claim_review(
        &self,
        tenant_id: &str,
        run_id: Uuid,
        reviewer_principal_ref: &str,
        lease_duration: Duration,
    ) -> Result<Option<PipelineReviewClaim>, DatabaseError> {
        if !reviewer_principal_ref.starts_with("reviewer_sha256:")
            || lease_duration <= Duration::zero()
            || lease_duration > Duration::minutes(30)
        {
            return Err(DatabaseError::Constraint(
                "invalid review claim".to_string(),
            ));
        }
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let run_row = tx
            .query_opt(
                "SELECT state, submission_id FROM pipeline_runs p
                  WHERE p.tenant_id = $1 AND p.run_id = $2 AND p.next_phase = 'review'
                    AND p.state IN ('pending', 'retry', 'awaiting_review')
                    AND p.admission_decision = 'quarantine'
                    AND NOT EXISTS (SELECT 1 FROM pipeline_review_assessments a
                                     WHERE a.tenant_id = p.tenant_id AND a.run_id = p.run_id)
                  FOR UPDATE",
                &[&tenant_id, &run_id],
            )
            .await?;
        let Some(run_row) = run_row else {
            tx.commit().await?;
            return Ok(None);
        };
        let run_state: String = run_row.get("state");
        let submission_id: Uuid = run_row.get("submission_id");
        if !review_submission_is_operable(&tx, tenant_id, submission_id).await? {
            if run_state == "awaiting_review" {
                release_awaiting_review(&tx, tenant_id, run_id).await?;
            }
            tx.commit().await?;
            return Err(DatabaseError::Constraint(
                "review claim is stale or inoperable".to_string(),
            ));
        }
        let lease_token = Uuid::new_v4();
        let lease_milliseconds = lease_duration.num_milliseconds();
        let row = tx
            .query_opt(
                "INSERT INTO pipeline_review_claims (
                    tenant_id, run_id, reviewer_principal_ref, lease_token, lease_expires_at
                 ) VALUES ($1, $2, $3, $4, NOW() + ($5::bigint * INTERVAL '1 millisecond'))
                 ON CONFLICT (tenant_id, run_id) DO UPDATE
                 SET reviewer_principal_ref = EXCLUDED.reviewer_principal_ref,
                     lease_token = EXCLUDED.lease_token,
                     lease_expires_at = EXCLUDED.lease_expires_at,
                     claimed_at = NOW()
                 WHERE pipeline_review_claims.lease_expires_at <= NOW()
                    OR pipeline_review_claims.reviewer_principal_ref = $3
                 RETURNING *",
                &[
                    &tenant_id,
                    &run_id,
                    &reviewer_principal_ref,
                    &lease_token,
                    &lease_milliseconds,
                ],
            )
            .await?;
        tx.commit().await?;
        Ok(row.map(|row| PipelineReviewClaim {
            tenant_id: row.get("tenant_id"),
            run_id: row.get("run_id"),
            reviewer_principal_ref: row.get("reviewer_principal_ref"),
            lease_token: row.get("lease_token"),
            lease_expires_at: row.get("lease_expires_at"),
        }))
    }

    /// Records a reviewer's Approve/Reject assessment of a quarantined run
    /// (port `ef97a459` lines 893 to 1006): the assessment row and the
    /// spent claim's deletion commit together, in the same transaction as
    /// (Ruling T3-3) releasing an `awaiting_review` run back to `pending`,
    /// due at once with its parking label cleared, so the next claim
    /// reaches it without waiting out any backoff. A `pending` or `retry`
    /// run needs no such release; the update is a harmless no-op for it.
    ///
    /// Ruling T3-6 adds the same submission-operability re-check
    /// `claim_review` takes, under the same row locks this already holds
    /// (`FOR UPDATE OF c, p`): an inoperable submission refuses with the
    /// port's stale-claim label and releases an `awaiting_review` run back
    /// to `pending` in this same transaction, so the run still ends under
    /// `submission_inoperable` rather than staying parked with a claim that
    /// can never be assessed.
    ///
    /// Fix round 1, finding I1: the initial lookup also requires `p.state IN
    /// ('pending', 'retry', 'awaiting_review')` and `p.admission_decision =
    /// 'quarantine'` (not the assessment-exists check `claim_review` adds --
    /// a claim already excludes an assessed run, so this is defense against
    /// a stale claim object, not a first line of defense). Without the
    /// state predicate, a worker that leased the run between the claim and
    /// the assessment (or that failed it, which leaves `next_phase =
    /// 'review'`) would never block this call, and the assessment could
    /// land after -- or racing -- the worker's own Review attempt. `FOR
    /// UPDATE OF ... p` makes this the same lock-and-recheck pattern
    /// `claim_review` uses: a worker's `claim_run` and this lookup take the
    /// same row lock, so the loser re-reads the winner's committed state.
    ///
    /// An `Approve` recommendation still refuses
    /// (`quarantine reason is unresolved`) unless `resolved_quarantine_reasons`
    /// names the run's own `admission_reason`, or the run was quarantined
    /// under no logged reason at all and at least one reason is resolved --
    /// unchanged from the port.
    pub async fn record_review_assessment(
        &self,
        claim: &PipelineReviewClaim,
        recommendation: ReviewRecommendation,
        reason: ReasonCode,
        resolved_quarantine_reasons: Vec<ReasonCode>,
    ) -> Result<HumanReviewAssessment, DatabaseError> {
        let mut resolved = resolved_quarantine_reasons;
        resolved.sort();
        resolved.dedup();
        let recommendation_label = match recommendation {
            ReviewRecommendation::Approve => "approve",
            ReviewRecommendation::Reject => "reject",
        };
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &claim.tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT p.admission_reason, p.state, p.submission_id
                 FROM pipeline_review_claims c
                 JOIN pipeline_runs p
                   ON p.tenant_id = c.tenant_id AND p.run_id = c.run_id
                 WHERE c.tenant_id = $1 AND c.run_id = $2
                   AND c.lease_token = $3
                   AND c.reviewer_principal_ref = $4
                   AND c.lease_expires_at > NOW()
                   AND p.next_phase = 'review'
                   AND p.state IN ('pending', 'retry', 'awaiting_review')
                   AND p.admission_decision = 'quarantine'
                 FOR UPDATE OF c, p",
                &[
                    &claim.tenant_id,
                    &claim.run_id,
                    &claim.lease_token,
                    &claim.reviewer_principal_ref,
                ],
            )
            .await?;
        let Some(row) = row else {
            return Err(DatabaseError::Constraint(
                "review claim is stale or inoperable".to_string(),
            ));
        };
        let admission_reason: Option<String> = row.get("admission_reason");
        let run_state: String = row.get("state");
        let submission_id: Uuid = row.get("submission_id");
        if !review_submission_is_operable(&tx, &claim.tenant_id, submission_id).await? {
            if run_state == "awaiting_review" {
                release_awaiting_review(&tx, &claim.tenant_id, claim.run_id).await?;
            }
            tx.commit().await?;
            return Err(DatabaseError::Constraint(
                "review claim is stale or inoperable".to_string(),
            ));
        }
        if recommendation == ReviewRecommendation::Approve {
            let resolved_admission_reason = admission_reason
                .as_ref()
                .is_some_and(|reason| resolved.iter().any(|item| item.as_str() == reason));
            if !resolved_admission_reason && !(admission_reason.is_none() && !resolved.is_empty()) {
                return Err(DatabaseError::Constraint(
                    "quarantine reason is unresolved".to_string(),
                ));
            }
        }
        let resolved_json = serde_json::to_value(&resolved).map_err(|_| {
            DatabaseError::Serialization("review assessment encode failed".to_string())
        })?;
        let evidence_hash = sha256_prefixed(
            serde_json::to_vec(&serde_json::json!({
                "schema": "trace_commons.pipeline_review_assessment.v1",
                "run_id": claim.run_id,
                "recommendation": recommendation_label,
                "reason": reason.as_str(),
                "resolved_quarantine_reasons": resolved,
            }))
            .map_err(|_| {
                DatabaseError::Serialization("review assessment encode failed".to_string())
            })?
            .as_slice(),
        );
        let assessment_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!(
                "tracecommons:pipeline-review-assessment:{}:{evidence_hash}",
                claim.run_id
            )
            .as_bytes(),
        );
        tx.execute(
            "INSERT INTO pipeline_review_assessments (
                tenant_id, assessment_id, run_id, reviewer_principal_ref,
                recommendation, reason_code, resolved_quarantine_reasons, evidence_hash
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
            &[
                &claim.tenant_id,
                &assessment_id,
                &claim.run_id,
                &claim.reviewer_principal_ref,
                &recommendation_label,
                &reason.as_str(),
                &resolved_json,
                &evidence_hash,
            ],
        )
        .await?;
        tx.execute(
            "DELETE FROM pipeline_review_claims
             WHERE tenant_id = $1 AND run_id = $2 AND lease_token = $3",
            &[&claim.tenant_id, &claim.run_id, &claim.lease_token],
        )
        .await?;
        // Ruling T3-3: unparks the run so the worker's ordinary claim query
        // reaches it right away, whether this assessment approved (Review
        // now resolves) or rejected (Review now ends the run) it.
        release_awaiting_review(&tx, &claim.tenant_id, claim.run_id).await?;
        tx.commit().await?;
        Ok(HumanReviewAssessment {
            assessment_id,
            recommendation,
            reason,
            resolved_quarantine_reasons: resolved,
            evidence_hash,
        })
    }

    /// The stored assessment for `run_id`, if a reviewer has recorded one
    /// (port `ef97a459` lines 1007 to 1052, unchanged). Read into
    /// `ReviewInput.human_assessment` before every Review attempt.
    pub async fn load_review_assessment(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> Result<Option<HumanReviewAssessment>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT assessment_id, recommendation, reason_code,
                        resolved_quarantine_reasons, evidence_hash
                 FROM pipeline_review_assessments
                 WHERE tenant_id = $1 AND run_id = $2",
                &[&tenant_id, &run_id],
            )
            .await?;
        tx.commit().await?;
        row.map(|row| {
            let recommendation = match row.get::<_, String>("recommendation").as_str() {
                "approve" => Ok(ReviewRecommendation::Approve),
                "reject" => Ok(ReviewRecommendation::Reject),
                _ => Err(DatabaseError::Serialization(
                    "unknown review recommendation".to_string(),
                )),
            }?;
            let reason = ReasonCode::new(row.get::<_, String>("reason_code")).map_err(|_| {
                DatabaseError::Serialization("invalid review assessment reason".to_string())
            })?;
            let resolved_quarantine_reasons = serde_json::from_value(
                row.get::<_, serde_json::Value>("resolved_quarantine_reasons"),
            )
            .map_err(|_| {
                DatabaseError::Serialization("invalid resolved quarantine reasons".to_string())
            })?;
            Ok(HumanReviewAssessment {
                assessment_id: row.get("assessment_id"),
                recommendation,
                reason,
                resolved_quarantine_reasons,
                evidence_hash: row.get("evidence_hash"),
            })
        })
        .transpose()
    }

    /// Runs waiting for a human assessment, oldest first, label-only fields
    /// (brief Step 3). Ruling T3-2 adds `awaiting_review` to the states
    /// selected -- the port's `list_policy_interventions`-adjacent draft
    /// predates that state. Ruling T3-6 excludes a run whose submission is
    /// no longer operable, the same predicate `review_submission_is_operable`
    /// checks (Ruling T3-8 names it as the one rule both this query and that
    /// helper mirror), applied here as a join and an anti-join rather than a
    /// per-row lock (this is a plain read, not a claim, so it must filter in
    /// SQL rather than call the helper row by row).
    pub async fn list_review_queue(
        &self,
        tenant_id: &str,
        limit: usize,
    ) -> Result<Vec<PipelineRunRecord>, DatabaseError> {
        let limit = i64::try_from(limit.clamp(1, 500)).unwrap_or(500);
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT p.* FROM pipeline_runs p
                  JOIN trace_submissions s
                    ON s.tenant_id = p.tenant_id AND s.submission_id = p.submission_id
                  WHERE p.tenant_id = $1 AND p.next_phase = 'review'
                    AND p.admission_decision = 'quarantine'
                    AND p.state IN ('pending', 'retry', 'awaiting_review')
                    AND NOT EXISTS (SELECT 1 FROM pipeline_review_assessments a
                                     WHERE a.tenant_id = p.tenant_id AND a.run_id = p.run_id)
                    AND s.status IN ('received', 'quarantined')
                    AND s.revoked_at IS NULL AND s.purged_at IS NULL
                    AND s.withdrawn_at IS NULL
                    AND (s.expires_at IS NULL OR s.expires_at > NOW())
                    AND NOT EXISTS (SELECT 1 FROM trace_withdrawals w
                                     WHERE w.tenant_id = p.tenant_id
                                       AND w.submission_id = p.submission_id)
                  ORDER BY p.created_at, p.run_id
                  LIMIT $2",
                &[&tenant_id, &limit],
            )
            .await?;
        tx.commit().await?;
        rows.iter().map(pipeline_run_from_row).collect()
    }

    /// Commits the Score outcome together with the exact index command it
    /// proposed (if any) and one pending settlement row per award, in one
    /// transaction (decision D5). `command`/`neighbor` are each `(stored
    /// ref, hash, object ref)`, already stored by the caller before this call
    /// opens its transaction. Each object is also recorded as an object ref
    /// of the submission (`score_object_ref`), as Review records its
    /// approved object, so everything that deletes a submission's payloads
    /// -- a withdrawal's queued follow-up above all -- deletes these too.
    /// Every award's settlement row starts `operation_state =
    /// 'pending'` (the column default) with `result_ref_hash` NULL --
    /// Settle records a result only once a leg completes. An award naming
    /// an instrument that `payout_rails` does not cover fails the whole
    /// commit with the safe label `settlement_adapter_missing`.
    ///
    /// A leg's payout starts `pending` only on the `near` rail of a service
    /// whose NEAR payout is enabled (`payout_enabled`); every other leg
    /// starts `disabled` (Ruling T10-2), so a payout that nothing will ever
    /// make never reads as pending. A leg seeded `disabled` is not paid
    /// later, even if payout is enabled afterwards. The Trace Credit leg of a
    /// bundle whose Trace Credit event is `NoveltyUtility` (the compatibility
    /// family, `trace_credit_event`) is never batched or paid, so it starts
    /// `disabled` on any rail (Ruling T10-9).
    ///
    /// Score read its input (`load_approved_bytes`) in an earlier
    /// transaction that has committed, so a withdrawal can land between
    /// that read and this commit. This re-checks the submission under this
    /// transaction (`submission_guard_on_tx`, after the run row's lock) and
    /// refuses the whole commit with `PIPELINE_SUBMISSION_INOPERABLE_LABEL`:
    /// no outcome, no settlement rows, no run update.
    #[allow(clippy::too_many_arguments)]
    pub async fn commit_score(
        &self,
        run: &PipelineRunRecord,
        outcome: StoredPhaseResult,
        awards: &InstrumentAwards,
        command: Option<(&str, &str, &TraceObjectRefWrite)>,
        neighbor: Option<(&str, &str, &TraceObjectRefWrite)>,
        payout_rails: &BTreeMap<String, String>,
        payout_enabled: bool,
        trace_credit_event: PipelineTraceCreditEvent,
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
        if !Self::submission_guard_on_tx(&tx, run).await?.operable {
            // Dropping the transaction rolls it back: nothing was written.
            return Err(DatabaseError::Constraint(
                PIPELINE_SUBMISSION_INOPERABLE_LABEL.to_string(),
            ));
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

        for award in awards.iter() {
            let instrument_id = award.instrument_id().as_str();
            let payout_rail = payout_rails.get(instrument_id).ok_or_else(|| {
                DatabaseError::Constraint(PIPELINE_SETTLEMENT_ADAPTER_MISSING_LABEL.to_string())
            })?;
            let never_paid = instrument_id == InstrumentId::trace_credit().as_str()
                && trace_credit_event == PipelineTraceCreditEvent::NoveltyUtility;
            let payout_state = if payout_rail == "near" && payout_enabled && !never_paid {
                "pending"
            } else {
                "disabled"
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

        for (_, _, object_ref) in command.iter().chain(neighbor.iter()) {
            insert_score_object_ref_on_tx(&tx, object_ref).await?;
        }
        let (command_ref, command_hash) = match command {
            Some((stored_ref, hash, _)) => (Some(stored_ref), Some(hash)),
            None => (None, None),
        };
        let (neighbor_ref, neighbor_hash) = match neighbor {
            Some((stored_ref, hash, _)) => (Some(stored_ref), Some(hash)),
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

    /// Persists the Settle policy's raw result before any external effect:
    /// `settle_selection`, its hash, the decided
    /// `index_membership`, and -- only for an include -- `index_write_state
    /// = 'pending'`, all in one transaction, and only the first time (the
    /// `settle_selection IS NULL` guard). A later call for the same run
    /// finds the guard already tripped, matches no row, and returns the
    /// stale-lease error; the caller checks `load_settle_selection` first
    /// and skips this call entirely on a retry, so a retry never reaches
    /// that error.
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

    /// Records progress on the run's index dispatch (port 2258 to 2285), in
    /// a transaction of its own: `pending` after `persist_settle_selection`
    /// decides an include, `complete` once every entry has been applied,
    /// `failed` on a content conflict, `cancelled` when the submission
    /// stopped being operable before dispatch could run. The dispatch
    /// records `complete` and `cancelled` on the transaction that holds the
    /// submission guard (`set_index_write_state_on_tx`); it calls this for
    /// `failed`, after it has dropped that transaction.
    pub async fn mark_index_write_state(
        &self,
        run: &PipelineRunRecord,
        index_write_state: &str,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let updated = Self::set_index_write_state_on_tx(&tx, run, index_write_state).await?;
        tx.commit().await?;
        Ok(updated)
    }

    /// `mark_index_write_state`'s update, with the same lease check, on the
    /// caller's transaction: Settle's index dispatch records `complete` or
    /// `cancelled` in the transaction that holds the submission guard
    /// (`submission_guard_on_tx`), so the record commits together with the
    /// release of that lock.
    pub async fn set_index_write_state_on_tx(
        tx: &Transaction<'_>,
        run: &PipelineRunRecord,
        index_write_state: &str,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        let lease_token = required_lease_token(run)?;
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
        pipeline_run_from_row(&row)
    }

    /// Queues the removal of `run`'s revision from the vector index, on the
    /// caller's transaction: one `pipeline_index_invalidations` row, due at
    /// once, and `pipeline_runs.index_invalidation_state = 'pending'`.
    /// `reason_code` must match `^[a-z0-9_]{1,64}$` (the table's CHECK).
    ///
    /// The rule: an `index_write_state` of `pending` may be partly written.
    /// A dispatch attempt can apply some or all of its entries and then roll
    /// its transaction back -- an `Uncertain` or `Failed` answer after
    /// earlier entries were applied, a crash after the last upsert, a failed
    /// commit -- so `pending` stays while entries are live. Whoever stops a
    /// `pending` write because the submission is no longer operable must
    /// therefore queue an invalidation, as for a `complete` one. Removing a
    /// revision that has no entries changes nothing
    /// (`VectorIndexWriter::invalidate_revision` returns `Ok(false)`).
    ///
    /// Idempotent: a run that already has an invalidation row keeps that
    /// row as it is, and `index_invalidation_state` moves only from `none`.
    pub async fn enqueue_index_invalidation_on_tx(
        tx: &Transaction<'_>,
        run: &PipelineRunRecord,
        reason_code: &str,
    ) -> Result<(), DatabaseError> {
        let revision_id = run.approved_revision_id.ok_or_else(|| {
            DatabaseError::Constraint("index_invalidation_revision_missing".to_string())
        })?;
        tx.execute(
            "INSERT INTO pipeline_index_invalidations (
                tenant_id, run_id, submission_id, registry_revision_id, reason_code
             ) VALUES ($1,$2,$3,$4,$5)
             ON CONFLICT (tenant_id, run_id) DO NOTHING",
            &[
                &run.tenant_id,
                &run.run_id,
                &run.submission_id,
                &revision_id,
                &reason_code,
            ],
        )
        .await?;
        tx.execute(
            "UPDATE pipeline_runs
                SET index_invalidation_state = 'pending', updated_at = NOW()
              WHERE tenant_id = $1 AND run_id = $2
                AND index_invalidation_state = 'none'",
            &[&run.tenant_id, &run.run_id],
        )
        .await?;
        Ok(())
    }

    /// Up to `limit` (clamped to 1..=500) of `tenant_id`'s index
    /// invalidations that are due: `pending`, not held by a claim's lease
    /// or waiting out a retry's backoff, and with an attempt left. Oldest
    /// due first.
    pub async fn list_due_index_invalidations(
        &self,
        tenant_id: &str,
        limit: usize,
    ) -> Result<Vec<Uuid>, DatabaseError> {
        let limit = i64::try_from(limit.clamp(1, 500)).unwrap_or(500);
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT run_id FROM pipeline_index_invalidations
                  WHERE tenant_id = $1 AND state = 'pending'
                    AND next_attempt_at <= NOW() AND attempt_count < max_attempts
                  ORDER BY next_attempt_at, run_id
                  LIMIT $2",
                &[&tenant_id, &limit],
            )
            .await?;
        tx.commit().await?;
        Ok(rows.iter().map(|row| row.get("run_id")).collect())
    }

    /// Claims `run_id`'s index invalidation for one attempt, if it is due
    /// (as `list_due_index_invalidations` defines it), and holds it for
    /// `lease`: the claim moves `next_attempt_at` to the lease's end in the
    /// same statement that checks the row is due. Two claims at once
    /// serialize on the row lock, and the second then finds the row no
    /// longer due, so only one gets it. `None` when the row is not due,
    /// not `pending`, out of attempts, or not `tenant_id`'s.
    ///
    /// The claim charges no attempt: only `fail_index_invalidation` charges
    /// one, for a failure that waiting cannot heal. An index outage
    /// (`retry_index_invalidation`) is not charged, and neither is an
    /// attempt that never reports back (the worker died): the row is due
    /// again once the lease has passed, the way a run's expired lease is.
    pub async fn claim_index_invalidation(
        &self,
        tenant_id: &str,
        run_id: Uuid,
        lease: Duration,
    ) -> Result<Option<PipelineIndexInvalidationClaim>, DatabaseError> {
        let lease_milliseconds = lease.num_milliseconds().max(1);
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                "UPDATE pipeline_index_invalidations
                    SET next_attempt_at = NOW() + ($3::bigint * INTERVAL '1 millisecond')
                  WHERE tenant_id = $1 AND run_id = $2
                    AND state = 'pending'
                    AND next_attempt_at <= NOW()
                    AND attempt_count < max_attempts
                  RETURNING registry_revision_id, attempt_count, next_attempt_at",
                &[&tenant_id, &run_id, &lease_milliseconds],
            )
            .await?;
        tx.commit().await?;
        row.map(|row| {
            Ok(PipelineIndexInvalidationClaim {
                tenant_id: tenant_id.to_string(),
                run_id,
                registry_revision_id: row.get("registry_revision_id"),
                attempt_count: u32::try_from(row.get::<_, i32>("attempt_count")).map_err(|_| {
                    DatabaseError::Serialization(
                        "invalid index invalidation attempt count".to_string(),
                    )
                })?,
                lease_expires_at: row.get("next_attempt_at"),
            })
        })
        .transpose()
    }

    /// Locks the claimed invalidation's run row, on the caller's
    /// transaction, before its invalidation row: the order a withdrawal
    /// and Settle's cancelled dispatch use (the run row, then the `INSERT`
    /// into this queue), and the order a run's deletion cascades in, so
    /// recording a result does not deadlock with them. `false` when the run
    /// is gone (its invalidation row went with it).
    async fn lock_invalidation_run_on_tx(
        tx: &Transaction<'_>,
        claim: &PipelineIndexInvalidationClaim,
    ) -> Result<bool, DatabaseError> {
        Ok(tx
            .query_opt(
                "SELECT 1 FROM pipeline_runs
                  WHERE tenant_id = $1 AND run_id = $2
                  FOR NO KEY UPDATE",
                &[&claim.tenant_id, &claim.run_id],
            )
            .await?
            .is_some())
    }

    /// Records that `claim`'s attempt removed the revision from the index:
    /// the invalidation and the run's `index_invalidation_state` become
    /// `complete`, in one transaction. The invalidation must still be
    /// `pending`; the claim's lease need not be current, since a removal
    /// that succeeded is a fact about the index whoever holds the row now
    /// (`invalidate_revision` is idempotent, and nothing writes the
    /// revision again once its invalidation is queued). `None`, with
    /// nothing written, when the invalidation is no longer `pending`.
    pub async fn complete_index_invalidation(
        &self,
        claim: &PipelineIndexInvalidationClaim,
    ) -> Result<Option<PipelineRunRecord>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &claim.tenant_id).await?;
        if !Self::lock_invalidation_run_on_tx(&tx, claim).await? {
            return Ok(None);
        }
        let completed = tx
            .execute(
                "UPDATE pipeline_index_invalidations
                    SET state = 'complete', completed_at = NOW(), last_error_label = NULL
                  WHERE tenant_id = $1 AND run_id = $2 AND state = 'pending'",
                &[&claim.tenant_id, &claim.run_id],
            )
            .await?;
        if completed == 0 {
            return Ok(None);
        }
        let row = tx
            .query_one(
                "UPDATE pipeline_runs
                    SET index_invalidation_state = 'complete', updated_at = NOW()
                  WHERE tenant_id = $1 AND run_id = $2
                  RETURNING *",
                &[&claim.tenant_id, &claim.run_id],
            )
            .await?;
        let run = pipeline_run_from_row(&row)?;
        tx.commit().await?;
        Ok(Some(run))
    }

    /// Records that the index was unavailable for `claim`'s attempt (it
    /// answered `Failed` or `Uncertain`): an uncharged retry (Ruling T8-2).
    /// The invalidation stays `pending` with its attempt count unchanged,
    /// under `index_invalidation_unavailable`, and is due again after the
    /// FR3 backoff `mark_transient_retry` uses, measured from the row's own
    /// `requested_at` instead of a run's `phase_started_at`: the wait is the
    /// invalidation's age, at least one second and at most one hour, so each
    /// retry lands when the invalidation is about twice as old as at the one
    /// before -- the delay doubles -- until, once it is an hour old, it is
    /// retried once an hour. There is no terminal bound: an outage never
    /// ends a queued invalidation, so the revision is removed once the index
    /// answers again (Review Focus 1). The run's `index_invalidation_state`
    /// stays `pending`.
    ///
    /// Fenced as `fail_index_invalidation` is: `None`, with nothing written,
    /// unless `claim` still holds the row.
    pub async fn retry_index_invalidation(
        &self,
        claim: &PipelineIndexInvalidationClaim,
    ) -> Result<Option<PipelineRunRecord>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &claim.tenant_id).await?;
        if !Self::lock_invalidation_run_on_tx(&tx, claim).await? {
            return Ok(None);
        }
        let retried = tx
            .execute(
                "UPDATE pipeline_index_invalidations
                    SET last_error_label = $3,
                        next_attempt_at = NOW() + LEAST(
                            GREATEST(NOW() - requested_at, INTERVAL '1 second'),
                            INTERVAL '1 hour'
                        )
                  WHERE tenant_id = $1 AND run_id = $2
                    AND state = 'pending' AND next_attempt_at = $4",
                &[
                    &claim.tenant_id,
                    &claim.run_id,
                    &PIPELINE_INDEX_INVALIDATION_UNAVAILABLE_LABEL,
                    &claim.lease_expires_at,
                ],
            )
            .await?;
        if retried == 0 {
            return Ok(None);
        }
        let row = tx
            .query_one(
                "SELECT * FROM pipeline_runs WHERE tenant_id = $1 AND run_id = $2",
                &[&claim.tenant_id, &claim.run_id],
            )
            .await?;
        let run = pipeline_run_from_row(&row)?;
        tx.commit().await?;
        Ok(Some(run))
    }

    /// Records that `claim`'s attempt failed in a way waiting cannot heal
    /// (the run's committed Score evidence names no index, or the index
    /// broke its contract and answered `ContentConflict`), charging it one
    /// attempt. With an attempt left, the invalidation stays `pending`
    /// under `index_invalidation_unavailable` and is due again after the
    /// charged-retry backoff `mark_retry` uses (50 ms, doubling with each
    /// charged attempt, capped at 50 ms x 2^9); the run's
    /// `index_invalidation_state` stays `pending`. The attempt that uses the
    /// last one leaves the invalidation and the run's state `failed` under
    /// `index_invalidation_failed`. An index outage is never recorded here
    /// (`retry_index_invalidation`).
    ///
    /// Only the current claim records a failure: the invalidation must be
    /// `pending` with `next_attempt_at` still at `claim`'s lease end. A
    /// claim whose lease passed and whose row another worker claimed, or a
    /// claim that already recorded its result, gets `None` and writes
    /// nothing.
    pub async fn fail_index_invalidation(
        &self,
        claim: &PipelineIndexInvalidationClaim,
    ) -> Result<Option<PipelineRunRecord>, DatabaseError> {
        let exponent = claim.attempt_count.min(9);
        let retry_milliseconds = DEFAULT_RETRY_MILLISECONDS.saturating_mul(1_i64 << exponent);
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &claim.tenant_id).await?;
        if !Self::lock_invalidation_run_on_tx(&tx, claim).await? {
            return Ok(None);
        }
        let Some(row) = tx
            .query_opt(
                "UPDATE pipeline_index_invalidations
                    SET attempt_count = attempt_count + 1,
                        state = CASE
                            WHEN attempt_count + 1 >= max_attempts THEN 'failed'
                            ELSE 'pending'
                        END,
                        last_error_label = CASE
                            WHEN attempt_count + 1 >= max_attempts THEN $4
                            ELSE $3
                        END,
                        next_attempt_at = CASE
                            WHEN attempt_count + 1 >= max_attempts THEN next_attempt_at
                            ELSE NOW() + ($5::bigint * INTERVAL '1 millisecond')
                        END
                  WHERE tenant_id = $1 AND run_id = $2
                    AND state = 'pending' AND next_attempt_at = $6
                  RETURNING state",
                &[
                    &claim.tenant_id,
                    &claim.run_id,
                    &PIPELINE_INDEX_INVALIDATION_UNAVAILABLE_LABEL,
                    &PIPELINE_INDEX_INVALIDATION_FAILED_LABEL,
                    &retry_milliseconds,
                    &claim.lease_expires_at,
                ],
            )
            .await?
        else {
            return Ok(None);
        };
        let state: String = row.get("state");
        let row = tx
            .query_one(
                "UPDATE pipeline_runs
                    SET index_invalidation_state = $3, updated_at = NOW()
                  WHERE tenant_id = $1 AND run_id = $2
                  RETURNING *",
                &[&claim.tenant_id, &claim.run_id, &state],
            )
            .await?;
        let run = pipeline_run_from_row(&row)?;
        tx.commit().await?;
        Ok(Some(run))
    }

    /// Whether the submission behind `run` is operable for Score and Settle:
    /// accepted, not revoked, not purged, not expired, and with no
    /// `trace_withdrawals` row. Runs on the caller's transaction and holds
    /// the submission row `FOR SHARE` until that transaction ends, so a
    /// withdrawal (which locks the row `FOR UPDATE`) waits for whatever the
    /// caller commits under the guard. Callers that also lock the run row
    /// (`ensure_current_lease`) take it first: run row, then submission row,
    /// the order `commit_review` documents.
    ///
    /// Two statements: lock first, read second, as `settle_internal_credit`
    /// does. A single locking read waits for a withdrawal that holds the row
    /// and then still evaluates the `trace_withdrawals` subquery against the
    /// snapshot it took before the wait, so it misses a withdrawal row that
    /// transaction inserted without changing the submission row. Under READ
    /// COMMITTED the second statement takes a new snapshot after the lock
    /// is held, so it sees that withdrawal.
    pub async fn submission_guard_on_tx(
        tx: &Transaction<'_>,
        run: &PipelineRunRecord,
    ) -> Result<SubmissionGuard, DatabaseError> {
        tx.query_opt(
            "SELECT 1 FROM trace_submissions
              WHERE tenant_id = $1 AND submission_id = $2
              FOR SHARE",
            &[&run.tenant_id, &run.submission_id],
        )
        .await?;
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
        let operable = row.map(|row| row.get::<_, bool>(0)).unwrap_or(false);
        Ok(SubmissionGuard { operable })
    }

    /// Withdraws a submission for its owner, with every side effect in one
    /// transaction (port `ef97a459` lines 1083 to 1471, written in the shape
    /// `main`'s account withdrawal has now).
    ///
    /// Source session (`main`'s #1021 binding): when `account_id` maps the
    /// submission to one of that account's source sessions, the session is
    /// marked withdrawn, so a new upload of it is refused, and every
    /// submission mapped to the session is withdrawn with this one, each
    /// first checked to belong to the account. This is the session half
    /// `main`'s `withdraw_trace_source_session` also calls
    /// (`withdraw_source_session_on_tx`). Otherwise only the requested
    /// submission is withdrawn.
    ///
    /// Lock order: the session row, then the `pipeline_runs` rows of every
    /// affected submission (`FOR UPDATE`, by `run_id`), then the submission
    /// rows (`FOR UPDATE`, by `submission_id`). Settle's index dispatch holds
    /// a run row and then its submission row `FOR SHARE` for its whole write,
    /// so a withdrawal waits for that write at the run row and never holds a
    /// submission row the dispatch waits for.
    ///
    /// Every affected submission gets `main`'s rows, through the same
    /// per-submission half `main` calls
    /// (`record_source_submission_withdrawal_on_tx`): a `trace_withdrawals`
    /// row (first writer wins, so a retry reports the first tier and time)
    /// and the status `revoked` with `withdrawn_at`, `revoked_at` and
    /// `purged_at` set; a delivered pipeline export snapshot counts as an
    /// export for the tier. One with a pipeline run also gets the pipeline
    /// follow-up (`withdraw_pipeline_content_on_tx`), and each of its runs:
    ///
    /// - an index write that is `pending` may be partly written, so it is
    ///   cancelled, the run is excluded from the index, and an invalidation
    ///   of the revision is queued; a write that is `complete`, `failed` or
    ///   `cancelled` wrote or may have written entries, so an invalidation is
    ///   queued (idempotent); `none` needs nothing;
    /// - a run parked for review goes back to `pending`, due at once, so the
    ///   runner ends it under `submission_inoperable`.
    ///
    /// Nothing here fails a run or changes a settlement row: the runner does
    /// both under the run's lease, and Settle forfeits every leg that is not
    /// complete on its next pass. No audit row is written here either: the
    /// caller appends the event to the mirrored audit log, as `main`'s
    /// withdrawal does.
    ///
    /// Not the owner (the requested submission's `auth_principal_ref` is not
    /// `actor_principal_ref`), or no such submission: `NotFound`, and
    /// nothing is written.
    pub async fn withdraw_submission(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        actor_principal_ref: &str,
        account_id: Option<Uuid>,
    ) -> Result<PipelineWithdrawalOutcome, DatabaseError> {
        let not_found = || DatabaseError::NotFound {
            entity: "trace_submission".to_string(),
            id: submission_id.to_string(),
        };
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        // Ownership before any lock is taken; checked again under the lock.
        let owner = tx
            .query_opt(
                "SELECT auth_principal_ref FROM trace_submissions
                  WHERE tenant_id = $1 AND submission_id = $2",
                &[&tenant_id, &submission_id],
            )
            .await?
            .map(|row| row.get::<_, String>(0));
        if owner.as_deref() != Some(actor_principal_ref) {
            return Err(not_found());
        }

        // The session half of `main`'s source-session withdrawal: it marks
        // the session withdrawn (its row is the first lock taken here) and
        // returns every submission mapped to it, each checked to belong to
        // the account. Without a mapped session, only this submission.
        let now: DateTime<Utc> = tx.query_one("SELECT NOW()", &[]).await?.get(0);
        let session = match account_id {
            Some(account_id) => {
                withdraw_source_session_on_tx(&tx, tenant_id, account_id, submission_id, now)
                    .await?
            }
            None => None,
        };
        let (withdrawn_at, affected) = session.unwrap_or((now, vec![submission_id]));

        let runs = tx
            .query(
                "SELECT * FROM pipeline_runs
                  WHERE tenant_id = $1 AND submission_id = ANY($2)
                  ORDER BY run_id
                  FOR UPDATE",
                &[&tenant_id, &affected],
            )
            .await?
            .iter()
            .map(pipeline_run_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let submissions = tx
            .query(
                "SELECT submission_id, status, auth_principal_ref, trace_id, redaction_hash,
                        canonical_summary_hash
                   FROM trace_submissions
                  WHERE tenant_id = $1 AND submission_id = ANY($2)
                  ORDER BY submission_id
                  FOR UPDATE",
                &[&tenant_id, &affected],
            )
            .await?
            .into_iter()
            .map(|row| (row.get::<_, Uuid>("submission_id"), row))
            .collect::<BTreeMap<_, _>>();
        let owned = submissions
            .get(&submission_id)
            .is_some_and(|row| row.get::<_, String>("auth_principal_ref") == actor_principal_ref);
        if !owned {
            return Err(not_found());
        }

        let mut with_runs = Vec::new();
        for affected_id in &affected {
            let submission = submissions.get(affected_id);
            // `main`'s per-submission rows, with its tier rule; a delivered
            // pipeline export snapshot also put copies out.
            let exported_by_a_pipeline_snapshot: bool = tx
                .query_one(
                    "SELECT EXISTS (
                        SELECT 1
                          FROM pipeline_export_snapshot_items item
                          JOIN pipeline_export_snapshots snapshot
                            ON snapshot.tenant_id = item.tenant_id
                           AND snapshot.snapshot_id = item.snapshot_id
                         WHERE item.tenant_id = $1 AND item.submission_id = $2
                           AND snapshot.completed_at IS NOT NULL
                     )",
                    &[&tenant_id, affected_id],
                )
                .await?
                .get(0);
            let content_status = submission.map(|row| row.get::<_, String>("status"));
            record_source_submission_withdrawal_on_tx(
                &tx,
                tenant_id,
                *affected_id,
                content_status.as_deref(),
                withdrawn_at,
                exported_by_a_pipeline_snapshot,
            )
            .await?;
            if let Some(submission) = submission
                && runs.iter().any(|run| run.submission_id == *affected_id)
            {
                withdraw_pipeline_content_on_tx(
                    &tx,
                    tenant_id,
                    *affected_id,
                    submission,
                    actor_principal_ref,
                )
                .await?;
                with_runs.push(*affected_id);
            }
        }

        for run in &runs {
            match run.index_write_state.as_str() {
                "pending" => {
                    tx.execute(
                        "UPDATE pipeline_runs
                            SET index_membership = 'excluded',
                                index_write_state = 'cancelled',
                                updated_at = NOW()
                          WHERE tenant_id = $1 AND run_id = $2",
                        &[&tenant_id, &run.run_id],
                    )
                    .await?;
                    Self::enqueue_index_invalidation_on_tx(
                        &tx,
                        run,
                        PIPELINE_WITHDRAWAL_INVALIDATION_REASON,
                    )
                    .await?;
                }
                "complete" | "failed" | "cancelled" => {
                    Self::enqueue_index_invalidation_on_tx(
                        &tx,
                        run,
                        PIPELINE_WITHDRAWAL_INVALIDATION_REASON,
                    )
                    .await?;
                }
                _ => {}
            }
            release_awaiting_review(&tx, tenant_id, run.run_id).await?;
        }

        let withdrawal_row = tx
            .query_one(
                "SELECT tenant_id, submission_id, withdrawn_at, prior_status, distribution_reach
                   FROM trace_withdrawals
                  WHERE tenant_id = $1 AND submission_id = $2",
                &[&tenant_id, &submission_id],
            )
            .await?;
        let follow_up = tx
            .query_one(
                "SELECT
                    (SELECT COUNT(*) FROM trace_revocation_propagation_items
                      WHERE tenant_id = $1 AND source_submission_id = ANY($2)
                        AND status IN ('pending', 'in_progress', 'failed')),
                    (SELECT COUNT(*) FROM trace_revocation_propagation_items
                      WHERE tenant_id = $1 AND source_submission_id = ANY($2)),
                    (SELECT COALESCE(array_agg(DISTINCT index_invalidation_state), '{}')
                       FROM pipeline_runs
                      WHERE tenant_id = $1 AND submission_id = ANY($2)),
                    EXISTS (
                        SELECT 1
                          FROM pipeline_run_settlements settlement
                          JOIN pipeline_runs run
                            ON run.tenant_id = settlement.tenant_id
                           AND run.run_id = settlement.run_id
                         WHERE settlement.tenant_id = $1 AND run.submission_id = ANY($2)
                           AND settlement.instrument_id = $3
                           AND (settlement.operation_state NOT IN ('complete', 'forfeited')
                                OR (settlement.operation_state = 'forfeited'
                                    AND settlement.last_error_label = $4))
                    )",
                &[
                    &tenant_id,
                    &with_runs,
                    &InstrumentId::trace_credit().as_str(),
                    &PIPELINE_SUBMISSION_INOPERABLE_LABEL,
                ],
            )
            .await?;
        let unfinished_propagation: i64 = follow_up.get(0);
        let queued_propagation: i64 = follow_up.get(1);
        let invalidation_states: Vec<String> = follow_up.get(2);
        // A leg Settle has not settled yet is forfeited on its next pass; a
        // Trace Credit leg Settle already forfeited for this reason stays in
        // the count, so a retry after that pass reports the same.
        let trace_credit_forfeited: bool = follow_up.get(3);
        // `pending` while any queued payload deletion is unfinished, `complete` once all
        // finished, `not_required` if none was queued; the port said `not_required` once
        // no live object ref remained, which misreports deletions that finished.
        let revocation_propagation = if unfinished_propagation > 0 {
            PipelineWithdrawalFollowUpState::Pending
        } else if queued_propagation > 0 {
            PipelineWithdrawalFollowUpState::Complete
        } else {
            PipelineWithdrawalFollowUpState::NotRequired
        };
        let has_state = |state: &str| invalidation_states.iter().any(|value| value == state);
        let index_invalidation = if has_state("failed") {
            PipelineWithdrawalFollowUpState::Failed
        } else if has_state("pending") {
            PipelineWithdrawalFollowUpState::Pending
        } else if has_state("complete") {
            PipelineWithdrawalFollowUpState::Complete
        } else {
            PipelineWithdrawalFollowUpState::NotRequired
        };
        let withdrawal = crate::trace_corpus_storage::TraceWithdrawalRecord {
            tenant_id: withdrawal_row.get("tenant_id"),
            submission_id: withdrawal_row.get("submission_id"),
            withdrawn_at: withdrawal_row.get("withdrawn_at"),
            prior_status: withdrawal_row.get("prior_status"),
            distribution_reach: withdrawal_row.get("distribution_reach"),
        };
        tx.commit().await?;
        Ok(PipelineWithdrawalOutcome {
            withdrawal,
            affected_submission_ids: affected,
            index_invalidation,
            revocation_propagation,
            trace_credit_forfeited,
        })
    }

    /// Whether a withdrawal of `submission_id` by `account_id` reaches a
    /// pipeline run: the submission itself has one, or a submission mapped
    /// to the same source session of that account does. `main`'s account
    /// withdrawal route uses the pipeline withdrawal exactly when this is
    /// true.
    pub async fn withdrawal_reaches_a_pipeline_run(
        &self,
        tenant_id: &str,
        account_id: Uuid,
        submission_id: Uuid,
    ) -> Result<bool, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let reaches: bool = tx
            .query_one(
                "SELECT EXISTS (
                    SELECT 1 FROM pipeline_runs run
                     WHERE run.tenant_id = $1
                       AND (run.submission_id = $3 OR run.submission_id IN (
                            SELECT sibling.submission_id
                              FROM trace_submission_sessions mapping
                              JOIN trace_submission_sessions sibling
                                ON sibling.tenant_id = mapping.tenant_id
                               AND sibling.account_id = mapping.account_id
                               AND sibling.session_digest = mapping.session_digest
                             WHERE mapping.tenant_id = $1 AND mapping.account_id = $2
                               AND mapping.submission_id = $3))
                 )",
                &[&tenant_id, &account_id, &submission_id],
            )
            .await?
            .get(0);
        tx.commit().await?;
        Ok(reaches)
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

    /// Fails the run terminally under its live lease. When the
    /// run is in Settle, every open leg is resolved in this same
    /// transaction (`resolve_open_settlement_legs_on_tx`). A worker calls
    /// this through `PipelineService::fail_run`, which reconciles the
    /// dispatched external legs first, so here they are already resolved.
    pub async fn mark_failed(
        &self,
        run: &PipelineRunRecord,
        error_label: &str,
    ) -> Result<(), DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let row = tx
            .query_opt(
                "UPDATE pipeline_runs
             SET state = 'failed', lease_token = NULL, lease_expires_at = NULL,
                 last_error_label = $3, updated_at = NOW()
             WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'
               AND lease_token = $4 AND lease_expires_at > NOW()
             RETURNING next_phase",
                &[&run.tenant_id, &run.run_id, &error_label, &lease_token],
            )
            .await?
            .ok_or_else(stale_lease_error)?;
        if phase_from_db(row.get("next_phase"))? == Some(Phase::Settle) {
            resolve_open_settlement_legs_on_tx(&tx, &run.tenant_id, &[run.run_id]).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// A charged retry: the attempt the claim took stays charged. When the
    /// run has no attempt left, the run is failed as `attempts_exhausted`
    /// instead, and a Settle run's open legs are resolved
    /// in this same transaction (`resolve_open_settlement_legs_on_tx`). A
    /// worker calls this through `PipelineService::charged_retry`, which
    /// first reconciles the dispatched external legs of a run this retry
    /// will exhaust.
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
        if updated.state == PipelineRunState::Failed && updated.next_phase == Some(Phase::Settle) {
            resolve_open_settlement_legs_on_tx(&tx, &run.tenant_id, &[run.run_id]).await?;
        }
        tx.commit().await?;
        Ok(updated)
    }

    /// The one suspension path (ruling FR3): a failure that is not the
    /// trace's fault -- a dependency outage, a missing or not-runnable bound
    /// dependency, a credit hold -- releases the claim and schedules a
    /// retry without charging the attempt the claim took. A quarantined run
    /// awaiting human review is parked instead (`mark_awaiting_review`),
    /// not retried here.
    ///
    /// The delay is `clamp(NOW() - phase_started_at, 1 second, 1 hour)`:
    /// the time the run has spent in its current phase. `phase_started_at`
    /// is set at receipt and reset by every phase commit, and nothing else
    /// moves it, so without a retry counter each retry lands when the phase
    /// is twice as old as at the previous one -- the delay doubles -- and it
    /// caps at one retry per hour. There is no terminal bound: a hold must
    /// stay retryable ("suspension leaves work retryable"), so the bound is
    /// on work per hour, not on attempts.
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
                        next_attempt_at = NOW() + LEAST(
                            GREATEST(NOW() - phase_started_at, INTERVAL '1 second'),
                            INTERVAL '1 hour'
                        ),
                        last_error_label = $3, updated_at = NOW()
                  WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'
                    AND lease_token = $4 AND lease_expires_at > NOW()
                  RETURNING *",
                &[&run.tenant_id, &run.run_id, &error_label, &lease_token],
            )
            .await?
            .ok_or_else(stale_lease_error)?;
        let updated = pipeline_run_from_row(&row)?;
        tx.commit().await?;
        Ok(updated)
    }

    /// Parks a run Review quarantined with no
    /// human assessment yet (`PipelineRunState::AwaitingReview`, always the
    /// label `review_assessment_required`) instead of retrying it under
    /// `mark_transient_retry`'s hourly backoff forever. No claim query
    /// selects `awaiting_review`, so a parked run does no further work on
    /// its own; `record_review_assessment` moves it back to `pending`, due
    /// at once, once an assessment lands (Ruling T3-3). A parked run whose
    /// submission is later withdrawn, expired, or purged stays parked here
    /// -- nothing in this call moves it -- but `claim_review` and
    /// `record_review_assessment` each check the submission's operability
    /// themselves and release such a run back to `pending` when they find
    /// it inoperable (Ruling T3-6), so the run still ends under
    /// `submission_inoperable` rather than staying parked forever.
    ///
    /// Fenced by the lease exactly like `mark_transient_retry`: the lease
    /// is cleared and the claim's attempt is given back
    /// (`attempt_count - 1`, floor 0), since parking is not a charged
    /// failure. Unlike `mark_transient_retry`, `next_attempt_at` is left
    /// alone -- nothing reads it while the run is parked, and the review
    /// route that unparks it will set its own value.
    pub async fn mark_awaiting_review(
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
                    SET state = 'awaiting_review', lease_token = NULL, lease_expires_at = NULL,
                        attempt_count = GREATEST(attempt_count - 1, 0),
                        last_error_label = $3, updated_at = NOW()
                  WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'
                    AND lease_token = $4 AND lease_expires_at > NOW()
                  RETURNING *",
                &[&run.tenant_id, &run.run_id, &error_label, &lease_token],
            )
            .await?
            .ok_or_else(stale_lease_error)?;
        let updated = pipeline_run_from_row(&row)?;
        tx.commit().await?;
        Ok(updated)
    }

    /// Records that `run`'s own phase lease went stale before the phase
    /// finished, as its own uncharged reason (`PIPELINE_LEASE_EXPIRED_LABEL`)
    /// with the same FR3 backoff `mark_transient_retry` uses -- never a
    /// charged policy failure, even though the lease has already gone stale
    /// by the time this is called.
    ///
    /// Fenced by the lease TOKEN alone, with no expiry predicate: every
    /// other lease-checked write in this store also requires
    /// `lease_expires_at > NOW()`, which is exactly the condition this call
    /// exists to act past. `state = 'leased' AND lease_token = $token` still
    /// guards it -- once a phase transition, a terminal write, or another
    /// worker's claim has moved the row off this token, this changes
    /// nothing and reports that as `Ok(None)` (a token-only fence) rather
    /// than disturbing whatever holds the run now.
    pub async fn record_lease_expired(
        &self,
        run: &PipelineRunRecord,
    ) -> Result<Option<PipelineRunRecord>, DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let row = tx
            .query_opt(
                "UPDATE pipeline_runs
                    SET state = 'retry', lease_token = NULL, lease_expires_at = NULL,
                        attempt_count = GREATEST(attempt_count - 1, 0),
                        next_attempt_at = NOW() + LEAST(
                            GREATEST(NOW() - phase_started_at, INTERVAL '1 second'),
                            INTERVAL '1 hour'
                        ),
                        last_error_label = $3, updated_at = NOW()
                  WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'
                    AND lease_token = $4
                  RETURNING *",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &PIPELINE_LEASE_EXPIRED_LABEL,
                    &lease_token,
                ],
            )
            .await?;
        let updated = row.as_ref().map(pipeline_run_from_row).transpose()?;
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

    /// The payout pass's work list (P3-D11): up to `limit` complete runs of
    /// `tenant_id` whose completed `trace_credit` leg is on the `near` rail,
    /// carries its settlement batch, and has a payout still to make
    /// (`pending`) or to confirm (`submitted`), least recently updated
    /// first. A leg without a batch -- a compatibility run's
    /// `NoveltyUtility` event, which `main` never pays -- is never listed
    /// (Ruling S8), nor is a `disabled`, `confirmed`, or `failed` payout.
    pub async fn list_runs_with_pending_payout(
        &self,
        tenant_id: &str,
        limit: usize,
    ) -> Result<Vec<Uuid>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        Ok(
            Self::list_payout_work_on(&mut client, tenant_id, limit, std::time::Duration::ZERO)
                .await?
                .into_iter()
                .map(|(run_id, _)| run_id)
                .collect(),
        )
    }

    /// The payout pass's work list, with each leg's payout state:
    /// `list_runs_with_pending_payout`, except that a `submitted` leg is
    /// listed only once `confirmation_interval` has passed since its
    /// `updated_at`. Every payout write sets `updated_at`, confirmation polls
    /// included, so a `submitted` leg is polled at most once per interval
    /// (Ruling T10-10). A `pending` leg is always listed.
    async fn list_payout_work_on(
        client: &mut deadpool_postgres::Client,
        tenant_id: &str,
        limit: usize,
        confirmation_interval: std::time::Duration,
    ) -> Result<Vec<(Uuid, String)>, DatabaseError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let confirmation_interval_seconds = confirmation_interval.as_secs_f64();
        let tx = Self::tenant_transaction(client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT s.run_id, s.payout_state
                   FROM pipeline_run_settlements s
                   JOIN pipeline_runs r
                     ON r.tenant_id = s.tenant_id AND r.run_id = s.run_id
                  WHERE s.tenant_id = $1
                    AND r.state = 'complete'
                    AND s.instrument_id = $2
                    AND s.operation_state = 'complete'
                    AND s.payout_rail = 'near'
                    AND s.settlement_batch_id IS NOT NULL
                    AND (
                        s.payout_state = 'pending'
                        OR (
                            s.payout_state = 'submitted'
                            AND s.updated_at <= NOW() - make_interval(secs => $3)
                        )
                    )
                  ORDER BY s.updated_at, s.run_id
                  LIMIT $4",
                &[
                    &tenant_id,
                    &InstrumentId::trace_credit().as_str(),
                    &confirmation_interval_seconds,
                    &limit,
                ],
            )
            .await?;
        tx.commit().await?;
        Ok(rows
            .iter()
            .map(|row| (row.get("run_id"), row.get("payout_state")))
            .collect())
    }

    /// Advances one instrument leg (port 2160 to 2211). Gated on the run's
    /// own lease, same as every other commit in this phase -- settlement
    /// rows have no lease of their own that this call checks.
    ///
    /// `result_ref_hash` only ever moves from `NULL` to a value on the
    /// transition to `'complete'`; the schema's own
    /// `pipeline_run_settlements_result_shape` check already forbids a
    /// non-`NULL` result on any other state, but `'forfeited'` and
    /// `'retry'` write it as `NULL` outright rather than `COALESCE`ing
    /// through whatever `update.result_ref_hash` carries, so a caller can
    /// never accidentally persist a result alongside a non-`'complete'`
    /// state. `external_receipt_hash` is written only on the transition to
    /// `'complete'` and is `NULL` for every other state
    /// (`pipeline_run_settlements_external_receipt_shape`); a hash another
    /// leg of the tenant already recorded is refused by
    /// `pipeline_run_settlements_external_receipt_unique`.
    /// `credit_event_id`/`settlement_batch_id` only move from `NULL`
    /// to a value (never overwritten or cleared); `payout_state` is written
    /// only when the caller supplies one. `attempt_count` increments on
    /// `'retry'`/`'failed'` (not `'forfeited'` -- forfeiture is not a
    /// dispatch failure) and saturates rather than overflow: it is a
    /// diagnostic count, and an adapter outage is an uncharged run retry
    /// with no terminal bound, so it must never refuse the
    /// update. Every update here moves the leg out of `leased` (none sets
    /// it; `mark_settlement_leased` does), so it clears the lease columns
    /// (`pipeline_run_settlements_lease_shape`); `dispatched_at` is never
    /// touched.
    async fn update_settlement(
        &self,
        run: &PipelineRunRecord,
        instrument_id: &str,
        update: SettlementUpdate<'_>,
    ) -> Result<PipelineSettlementRecord, DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let settlement =
            update_settlement_on_tx(&tx, run, lease_token, instrument_id, update).await?;
        tx.commit().await?;
        Ok(settlement)
    }

    /// Moves one leg to `leased` under the run's own lease token
    /// and expiry immediately before its adapter call, and records the first
    /// dispatch (`dispatched_at`, set once and never cleared). It accepts
    /// exactly the legs Step 6 dispatches (`SETTLEMENT_LEG_OPEN_TO_DISPATCH_SQL`),
    /// so a `leased` leg an earlier, crashed attempt left behind is leased
    /// again under this attempt's lease and dispatched again (the adapter is
    /// idempotent by `operation_ref_hash`), and a leg that is never
    /// dispatched again is refused.
    ///
    /// Fenced by the run lease: `ensure_current_lease` locks the run row and
    /// returns the stale-lease error only for a lease that is really stale.
    /// A leg closed to dispatch (or no leg) under a live lease is
    /// `settlement_leg_not_open_error`, never the stale-lease error.
    async fn mark_settlement_leased(
        &self,
        run: &PipelineRunRecord,
        instrument_id: &str,
    ) -> Result<(), DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        ensure_current_lease(&tx, run, lease_token).await?;
        let updated = tx
            .execute(
                &format!(
                    "UPDATE pipeline_run_settlements s
                        SET operation_state = 'leased',
                            lease_token = p.lease_token,
                            lease_expires_at = p.lease_expires_at,
                            dispatched_at = COALESCE(s.dispatched_at, NOW()),
                            last_error_label = NULL,
                            updated_at = NOW()
                       FROM pipeline_runs p
                      WHERE s.tenant_id = $1 AND s.run_id = $2 AND s.instrument_id = $3
                        AND p.tenant_id = s.tenant_id AND p.run_id = s.run_id
                        AND p.lease_token = $4
                        AND {SETTLEMENT_LEG_OPEN_TO_DISPATCH_SQL}"
                ),
                &[&run.tenant_id, &run.run_id, &instrument_id, &lease_token],
            )
            .await?;
        if updated != 1 {
            return Err(settlement_leg_not_open_error());
        }
        tx.commit().await?;
        Ok(())
    }

    /// Records the reconciling adapter call a worker makes for
    /// an open, dispatched, external leg before it fails the run
    /// (`PipelineService::reconcile_dispatched_settlement_legs`).
    /// `Some(receipt)` -- a receipt that answers the leg's request --
    /// completes the leg with its result reference and its external receipt
    /// hash; `None` records `failed` / `settlement_unreconciled`. Fenced like
    /// `mark_settlement_leased`, with the same distinct refusal for any other
    /// leg, including a leg that is never dispatched again.
    async fn record_settlement_reconciliation(
        &self,
        run: &PipelineRunRecord,
        instrument_id: &str,
        receipt: Option<&SettlementReceipt>,
    ) -> Result<(), DatabaseError> {
        let (operation_state, error_label) = match receipt {
            Some(_) => ("complete", None),
            None => ("failed", Some(PIPELINE_SETTLEMENT_UNRECONCILED_LABEL)),
        };
        let result_ref_hash = receipt.map(SettlementReceipt::result_ref_hash);
        let external_receipt_hash = receipt.and_then(SettlementReceipt::external_receipt_hash);
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        ensure_current_lease(&tx, run, lease_token).await?;
        let updated = tx
            .execute(
                &format!(
                    "UPDATE pipeline_run_settlements s
                        SET operation_state = $4,
                            result_ref_hash = $5,
                            external_receipt_hash = $8,
                            last_error_label = $6,
                            lease_token = NULL,
                            lease_expires_at = NULL,
                            updated_at = NOW()
                      WHERE s.tenant_id = $1 AND s.run_id = $2 AND s.instrument_id = $3
                        AND s.instrument_id <> $7
                        AND s.dispatched_at IS NOT NULL
                        AND {UNRESOLVED_SETTLEMENT_LEG_SQL}
                        AND {SETTLEMENT_LEG_OPEN_TO_DISPATCH_SQL}"
                ),
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &instrument_id,
                    &operation_state,
                    &result_ref_hash,
                    &error_label,
                    &InstrumentId::trace_credit().as_str(),
                    &external_receipt_hash,
                ],
            )
            .await?;
        if updated != 1 {
            return Err(settlement_leg_not_open_error());
        }
        tx.commit().await?;
        Ok(())
    }

    /// Inserts a receipt attempt's `staged` row inside the receipt's staging
    /// transaction, which commits before the attempt writes its object. The
    /// row names that object -- its key and ciphertext hash, fixed when the
    /// object was prepared -- so whatever happens after the write (an
    /// Admission error, a failed records insert, a process crash), a
    /// `staged` row names the object, and `PipelineService::sweep_staged_receipts`
    /// deletes both once `cleanup_after` passes. The receipt's final
    /// transaction moves the row to `committed` together with the object ref
    /// (`insert_receipt_records`).
    async fn stage_receipt_artifact(
        tx: &Transaction<'_>,
        attempt: &ReceiptAttempt,
    ) -> Result<(), DatabaseError> {
        tx.execute(
            "INSERT INTO pipeline_receipt_artifacts (
                tenant_id, run_id, attempt_id, request_idempotency_key,
                request_content_hash, object_key, ciphertext_sha256
             ) VALUES ($1,$2,$3,$4,$5,$6,$7)",
            &[
                &attempt.tenant_id,
                &attempt.run_id,
                &attempt.attempt_id,
                &attempt.request_idempotency_key,
                &attempt.request_content_hash,
                &attempt.receipt.object_key,
                &attempt.receipt.ciphertext_sha256,
            ],
        )
        .await?;
        Ok(())
    }

    /// Locks a receipt attempt's row inside the receipt's final transaction
    /// and reports whether the attempt may still commit: the row is
    /// `staged` and not yet due. The sweeper deletes a due row (and its
    /// object) while holding the row's lock, and skips a row locked here, so
    /// an attempt whose row it removed cannot commit, and a row this
    /// transaction commits is never swept.
    ///
    /// A sweep that deleted a row's object and then failed
    /// before its commit (a lost connection, a later row's `DELETE`) leaves
    /// the row `staged` and unlocked with its object gone. Every row a sweep
    /// locks was due when that sweep read it, so refusing a due row here
    /// refuses a superset of the rows a failed sweep may have emptied -- it
    /// also refuses a live attempt whose own commit is simply slower than
    /// `cleanup_after`. The due check is its own statement after the lock
    /// is held, against `clock_timestamp()`: a condition in the `FOR
    /// UPDATE` query is evaluated before the lock wait, and `NOW()` is this
    /// transaction's start.
    async fn lock_committable_receipt_artifact(
        tx: &Transaction<'_>,
        attempt: &ReceiptAttempt,
    ) -> Result<bool, DatabaseError> {
        let Some(row) = tx
            .query_opt(
                "SELECT cleanup_after FROM pipeline_receipt_artifacts
                  WHERE tenant_id = $1 AND run_id = $2 AND attempt_id = $3
                    AND state = 'staged'
                  FOR UPDATE",
                &[&attempt.tenant_id, &attempt.run_id, &attempt.attempt_id],
            )
            .await?
        else {
            return Ok(false);
        };
        let cleanup_after: DateTime<Utc> = row.get("cleanup_after");
        let due: bool = tx
            .query_one("SELECT $1 <= clock_timestamp()", &[&cleanup_after])
            .await?
            .get(0);
        Ok(!due)
    }

    /// Deletes a refused receipt attempt's `staged` row, once its object is
    /// deleted (`PipelineService::discard_receipt_attempt`).
    async fn remove_staged_receipt_artifact(
        &self,
        attempt: &ReceiptAttempt,
    ) -> Result<(), DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &attempt.tenant_id).await?;
        tx.execute(
            "DELETE FROM pipeline_receipt_artifacts
              WHERE tenant_id = $1 AND run_id = $2 AND attempt_id = $3
                AND state = 'staged'",
            &[&attempt.tenant_id, &attempt.run_id, &attempt.attempt_id],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// After a refused receipt attempt's object delete failed: makes its row
    /// `staged` and due now, inserting it again if the sweeper already
    /// removed it, so the sweeper retries the delete on its next pass.
    async fn restage_receipt_artifact_due_now(
        &self,
        attempt: &ReceiptAttempt,
    ) -> Result<(), DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &attempt.tenant_id).await?;
        tx.execute(
            "INSERT INTO pipeline_receipt_artifacts (
                tenant_id, run_id, attempt_id, request_idempotency_key,
                request_content_hash, object_key, ciphertext_sha256, cleanup_after
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,NOW())
             ON CONFLICT (tenant_id, run_id, attempt_id) DO UPDATE
                SET cleanup_after = NOW()
              WHERE pipeline_receipt_artifacts.state = 'staged'",
            &[
                &attempt.tenant_id,
                &attempt.run_id,
                &attempt.attempt_id,
                &attempt.request_idempotency_key,
                &attempt.request_content_hash,
                &attempt.receipt.object_key,
                &attempt.receipt.ciphertext_sha256,
            ],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

/// The run a receipt's idempotency key already created, if any.
async fn existing_receipt_run(
    tx: &Transaction<'_>,
    tenant_id: &str,
    request_idempotency_key: &str,
) -> Result<Option<PipelineRunRecord>, DatabaseError> {
    tx.query_opt(
        "SELECT * FROM pipeline_runs
          WHERE tenant_id = $1 AND request_idempotency_key = $2",
        &[&tenant_id, &request_idempotency_key],
    )
    .await?
    .map(|row| pipeline_run_from_row(&row))
    .transpose()
}

/// A receipt whose key already has a run: `Replayed` for the same content,
/// `ContentConflict` for other content.
fn replay_result(existing: PipelineRunRecord, request_content_hash: &str) -> PipelineReceiptResult {
    if existing.request_content_hash == request_content_hash {
        PipelineReceiptResult::Replayed(existing)
    } else {
        PipelineReceiptResult::ContentConflict
    }
}

/// The refusal a receipt's key alone decides: `Replayed` or
/// `ContentConflict` when the key already has a run, and `ContentConflict`
/// when an attempt for the key is staged with other content.
async fn receipt_key_refusal(
    tx: &Transaction<'_>,
    tenant_id: &str,
    request_idempotency_key: &str,
    request_content_hash: &str,
) -> Result<Option<PipelineReceiptResult>, DatabaseError> {
    if let Some(existing) = existing_receipt_run(tx, tenant_id, request_idempotency_key).await? {
        return Ok(Some(replay_result(existing, request_content_hash)));
    }
    let staged_with_other_content: bool = tx
        .query_one(
            "SELECT EXISTS (
                SELECT 1 FROM pipeline_receipt_artifacts
                 WHERE tenant_id = $1 AND request_idempotency_key = $2
                   AND request_content_hash <> $3
             )",
            &[&tenant_id, &request_idempotency_key, &request_content_hash],
        )
        .await?
        .get(0);
    Ok(staged_with_other_content.then_some(PipelineReceiptResult::ContentConflict))
}

/// The quota a receipt that counts would exceed, if any. A key that already
/// has a usage row was counted by an earlier attempt, so it is never refused
/// for its own count.
///
/// Quota counts pipeline receipts only (pipeline_admission_usage). Legacy
/// submission records are not counted, so in the first hour after a tenant
/// moves to the pipeline it can receive up to one extra hourly quota. The
/// owner accepted this on 2026-09-23; see the activation runbook,
/// "Submission quota at switch-over".
async fn receipt_quota_refusal(
    tx: &Transaction<'_>,
    request: &PipelineReceiptRequest<'_>,
    request_idempotency_key: &str,
    principal_ref_hash: &str,
) -> Result<Option<PipelineQuotaScope>, DatabaseError> {
    let counts = tx
        .query_one(
            "SELECT COUNT(*) FILTER (WHERE counted_at > NOW() - INTERVAL '1 hour') AS tenant_count,
                    COUNT(*) FILTER (WHERE counted_at > NOW() - INTERVAL '1 hour' AND principal_ref_hash = $2) AS principal_count,
                    COUNT(*) FILTER (WHERE request_idempotency_key = $3) AS key_count
               FROM pipeline_admission_usage WHERE tenant_id = $1",
            &[&request.tenant_id, &principal_ref_hash, &request_idempotency_key],
        )
        .await?;
    if counts.get::<_, i64>("key_count") > 0 {
        return Ok(None);
    }
    let tenant_count: i64 = counts.get("tenant_count");
    let principal_count: i64 = counts.get("principal_count");
    if request.limits.max_per_tenant_per_hour != 0
        && tenant_count >= request.limits.max_per_tenant_per_hour as i64
    {
        return Ok(Some(PipelineQuotaScope::Tenant));
    }
    if request.limits.max_per_principal_per_hour != 0
        && principal_count >= request.limits.max_per_principal_per_hour as i64
    {
        return Ok(Some(PipelineQuotaScope::Principal));
    }
    Ok(None)
}

/// Whether a receipt's content is refused: its submission is withdrawn, or
/// a tombstone matches its submission, trace, redaction hash, or request
/// content hash. Both receipt transactions read it, the second because a
/// tombstone can arrive while the object is written.
async fn receipt_is_tombstoned(
    tx: &Transaction<'_>,
    tenant_id: &str,
    envelope: &TraceContributionEnvelope,
    request_content_hash: &str,
) -> Result<bool, DatabaseError> {
    Ok(tx
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
        .get(0))
}

/// Resolves every open leg of the
/// failed Settle runs `run_ids`, inside the caller's transaction -- the one
/// that fails them (`mark_failed`, a `mark_retry` that exhausts the
/// attempts, or the claim sweep in `claim_next`). No adapter is called
/// here. A leg `failed` as `settlement_request_conflict` or
/// `settlement_request_rejected` is `forfeited` under that same label: its
/// adapter said no effect happened. A dispatched external leg still open is
/// `failed` / `settlement_unreconciled`: its effect may have happened, and
/// only its adapter can say. Every other open leg is `forfeited` /
/// `run_failed`: one never dispatched paid nothing, and a Trace Credit leg
/// pays only through the ledger row that commits with its completion, so an
/// incomplete one paid nothing either -- a Trace Credit leg `failed` as
/// `settlement_result_mismatch` included. Every leg it touches loses its lease
/// columns (`pipeline_run_settlements_lease_shape`); a resolved leg (see
/// `settlement_leg_is_unresolved`) is never touched. With a worker present,
/// `PipelineService::reconcile_dispatched_settlement_legs` has already
/// resolved the dispatched external legs it may call, so this only forfeits.
async fn resolve_open_settlement_legs_on_tx(
    tx: &Transaction<'_>,
    tenant_id: &str,
    run_ids: &[Uuid],
) -> Result<(), DatabaseError> {
    if run_ids.is_empty() {
        return Ok(());
    }
    let no_effect_labels = [
        SettlementError::Conflict.label(),
        SettlementError::Rejected.label(),
    ];
    tx.execute(
        &format!(
            "UPDATE pipeline_run_settlements s
                SET operation_state = CASE
                        WHEN s.operation_state = 'failed' AND s.last_error_label = ANY($6)
                            THEN 'forfeited'
                        WHEN s.dispatched_at IS NOT NULL AND s.instrument_id <> $3
                            THEN 'failed'
                        ELSE 'forfeited'
                    END,
                    last_error_label = CASE
                        WHEN s.operation_state = 'failed' AND s.last_error_label = ANY($6)
                            THEN s.last_error_label
                        WHEN s.dispatched_at IS NOT NULL AND s.instrument_id <> $3
                            THEN $4
                        ELSE $5
                    END,
                    lease_token = NULL,
                    lease_expires_at = NULL,
                    updated_at = NOW()
              WHERE s.tenant_id = $1 AND s.run_id = ANY($2)
                AND {UNRESOLVED_SETTLEMENT_LEG_SQL}"
        ),
        &[
            &tenant_id,
            &run_ids,
            &InstrumentId::trace_credit().as_str(),
            &PIPELINE_SETTLEMENT_UNRECONCILED_LABEL,
            &PIPELINE_SETTLEMENT_RUN_FAILED_LABEL,
            &no_effect_labels.as_slice(),
        ],
    )
    .await?;
    Ok(())
}

/// `PgPipelineStore::update_settlement` inside the caller's tenant
/// transaction, fenced by the same lease check (the run row must still be
/// `leased` under `lease_token` and unexpired), so the Trace Credit leg can
/// complete its settlement row in the same transaction as its ledger work.
async fn update_settlement_on_tx(
    tx: &Transaction<'_>,
    run: &PipelineRunRecord,
    lease_token: Uuid,
    instrument_id: &str,
    update: SettlementUpdate<'_>,
) -> Result<PipelineSettlementRecord, DatabaseError> {
    let row = tx
        .query_opt(
            "UPDATE pipeline_run_settlements s
                    SET operation_state = $4,
                        result_ref_hash = CASE
                            WHEN $4 IN ('forfeited', 'retry') THEN NULL
                            ELSE COALESCE(s.result_ref_hash, $5)
                        END,
                        external_receipt_hash = CASE
                            WHEN $4 = 'complete' THEN COALESCE(s.external_receipt_hash, $11)
                            ELSE NULL
                        END,
                        credit_event_id = COALESCE(s.credit_event_id, $6),
                        settlement_batch_id = COALESCE(s.settlement_batch_id, $7),
                        payout_state = COALESCE($8, s.payout_state),
                        last_error_label = $9,
                        attempt_count = CASE
                            WHEN $4 IN ('retry', 'failed')
                                THEN LEAST(s.attempt_count::BIGINT + 1, 2147483647)::INTEGER
                            ELSE s.attempt_count
                        END,
                        lease_token = NULL,
                        lease_expires_at = NULL,
                        updated_at = NOW()
                   FROM pipeline_runs p
                  WHERE s.tenant_id = $1 AND s.run_id = $2 AND s.instrument_id = $3
                    AND p.tenant_id = s.tenant_id AND p.run_id = s.run_id
                    AND p.state = 'leased' AND p.lease_token = $10
                    AND p.lease_expires_at > NOW()
                  RETURNING s.*, s.atomic_units::TEXT AS atomic_units_text",
            &[
                &run.tenant_id,
                &run.run_id,
                &instrument_id,
                &update.operation_state,
                &update.result_ref_hash,
                &update.credit_event_id,
                &update.settlement_batch_id,
                &update.payout_state,
                &update.error_label,
                &lease_token,
                &update.external_receipt_hash,
            ],
        )
        .await?
        .ok_or_else(stale_lease_error)?;
    pipeline_settlement_from_row(&row)
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
/// Admission outcome, and the attempt's staging row's transition to
/// `committed` -- which requires the row to be `staged` and to name exactly
/// the object the object ref records. Unlike the port, this does not insert
/// a `pipeline_receipt_ownership` row (PR 5 / cross-pipeline ownership is
/// deferred).
async fn insert_receipt_records(
    tx: &Transaction<'_>,
    run: &NewPipelineRun,
    attempt: &ReceiptAttempt,
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
             WHERE tenant_id = $1 AND run_id = $2 AND attempt_id = $3
               AND request_idempotency_key = $4
               AND request_content_hash = $5
               AND object_key = $6
               AND ciphertext_sha256 = $7
               AND state = 'staged'",
            &[
                &run.tenant_id,
                &run.run_id,
                &attempt.attempt_id,
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

/// The safe label for a store error while loading a run's bound bundle.
/// Only a stored package that fails its own validation -- the one
/// error `load_bundle_from_transaction` raises as
/// `bundle_package_invalid` -- is permanent; any other database error means
/// the store could not answer, `bundle_store_unavailable`, which
/// `process_claimed_run` treats as an uncharged suspension (ruling FR3).
fn bound_bundle_load_error_label(error: &DatabaseError) -> &'static str {
    match error {
        DatabaseError::Serialization(label) if label == PIPELINE_BUNDLE_INVALID_LABEL => {
            PIPELINE_BUNDLE_INVALID_LABEL
        }
        _ => PIPELINE_BUNDLE_STORE_UNAVAILABLE_LABEL,
    }
}

fn required_lease_token(run: &PipelineRunRecord) -> Result<Uuid, DatabaseError> {
    run.lease_token.ok_or_else(stale_lease_error)
}

/// Re-reads the run row `FOR UPDATE` and confirms the caller's lease is
/// still the current one before it writes anything durable under it. It
/// does not re-validate `next_phase`: a phase transition always clears
/// `lease_token`, so the token match alone already proves the lease has not
/// moved on.
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

/// The one Review-phase submission-operability predicate: neither pre-review
/// status, nor revoked, purged, withdrawn, or expired, and no separate
/// `trace_withdrawals` tombstone. `commit_review` calls this directly
/// (Ruling T3-8 -- it used to inline its own copy); `claim_review` and
/// `record_review_assessment` call it too (Ruling T3-6). Locks the
/// submission row `FOR UPDATE`, after the run row a caller may already hold
/// locked -- the same order `commit_review` documents (run row, then
/// submission row).
async fn review_submission_is_operable(
    tx: &Transaction<'_>,
    tenant_id: &str,
    submission_id: Uuid,
) -> Result<bool, DatabaseError> {
    let submission_row = tx
        .query_opt(
            "SELECT status, revoked_at, purged_at, withdrawn_at, expires_at
               FROM trace_submissions
              WHERE tenant_id = $1 AND submission_id = $2
              FOR UPDATE",
            &[&tenant_id, &submission_id],
        )
        .await?;
    let operable = match &submission_row {
        None => false,
        Some(row) => {
            let status: String = row.get("status");
            let revoked_at: Option<DateTime<Utc>> = row.get("revoked_at");
            let purged_at: Option<DateTime<Utc>> = row.get("purged_at");
            let withdrawn_at: Option<DateTime<Utc>> = row.get("withdrawn_at");
            let expires_at: Option<DateTime<Utc>> = row.get("expires_at");
            matches!(status.as_str(), "received" | "quarantined")
                && revoked_at.is_none()
                && purged_at.is_none()
                && withdrawn_at.is_none()
                && expires_at.is_none_or(|expires_at| expires_at > Utc::now())
        }
    };
    if !operable {
        return Ok(false);
    }
    let withdrawn = tx
        .query_opt(
            "SELECT 1 FROM trace_withdrawals WHERE tenant_id = $1 AND submission_id = $2",
            &[&tenant_id, &submission_id],
        )
        .await?
        .is_some();
    Ok(!withdrawn)
}

/// Moves an `awaiting_review` run back to `pending`, due at once, with its
/// parking label cleared (Ruling T3-3): the shape both an assessment that
/// resolves the quarantine and the inoperable-submission release (Ruling
/// T3-6) need, so the run leaves `awaiting_review` and reaches a worker's
/// ordinary claim query again -- whether that worker then runs Review to
/// completion or ends the run under `submission_inoperable`. A no-op when
/// the run is not currently `awaiting_review` (already `pending` or
/// `retry`, or gone).
async fn release_awaiting_review(
    tx: &Transaction<'_>,
    tenant_id: &str,
    run_id: Uuid,
) -> Result<(), DatabaseError> {
    tx.execute(
        "UPDATE pipeline_runs
            SET state = 'pending', next_attempt_at = NOW(), last_error_label = NULL,
                updated_at = NOW()
          WHERE tenant_id = $1 AND run_id = $2 AND state = 'awaiting_review'",
        &[&tenant_id, &run_id],
    )
    .await?;
    Ok(())
}

/// The pipeline follow-up of a withdrawal, for one submission that has a
/// pipeline run, on the withdrawal's transaction (which already holds the
/// submission row): a tombstone; its object refs invalidated and each live
/// one queued for payload deletion (`delete_object_payload`, done by the
/// revocation-propagation worker); its derived records revoked; its vector
/// entries, legacy export manifests and items, and pipeline export
/// snapshots and items invalidated. Every write is idempotent.
async fn withdraw_pipeline_content_on_tx(
    tx: &Transaction<'_>,
    tenant_id: &str,
    submission_id: Uuid,
    submission: &Row,
    actor_principal_ref: &str,
) -> Result<(), DatabaseError> {
    let trace_id: Uuid = submission.get("trace_id");
    let redaction_hash: String = submission.get("redaction_hash");
    let canonical_summary_hash: Option<String> = submission.get("canonical_summary_hash");
    let object_rows = tx
        .query(
            "SELECT object_ref_id
               FROM trace_object_refs
              WHERE tenant_id = $1 AND submission_id = $2
                AND deleted_at IS NULL
              ORDER BY object_ref_id",
            &[&tenant_id, &submission_id],
        )
        .await?;
    let tombstone_id = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("pipeline-withdrawal:{tenant_id}:{submission_id}").as_bytes(),
    );
    tx.execute(
        "INSERT INTO trace_tombstones (
            tenant_id, tombstone_id, submission_id, trace_id, redaction_hash,
            canonical_summary_hash, reason, effective_at, created_by_principal_ref
         ) VALUES ($1,$2,$3,$4,$5,$6,'withdrawn',NOW(),$7)
         ON CONFLICT (tenant_id, submission_id) DO NOTHING",
        &[
            &tenant_id,
            &tombstone_id,
            &submission_id,
            &trace_id,
            &redaction_hash,
            &canonical_summary_hash,
            &actor_principal_ref,
        ],
    )
    .await?;
    tx.execute(
        "UPDATE trace_object_refs
            SET invalidated_at = COALESCE(invalidated_at, NOW()), updated_at = NOW()
          WHERE tenant_id = $1 AND submission_id = $2 AND invalidated_at IS NULL",
        &[&tenant_id, &submission_id],
    )
    .await?;
    tx.execute(
        "UPDATE trace_derived_records
            SET status = 'revoked', updated_at = NOW()
          WHERE tenant_id = $1 AND submission_id = $2 AND status <> 'revoked'",
        &[&tenant_id, &submission_id],
    )
    .await?;
    tx.execute(
        "UPDATE trace_vector_entries
            SET status = 'invalidated',
                invalidated_at = COALESCE(invalidated_at, NOW()), updated_at = NOW()
          WHERE tenant_id = $1 AND submission_id = $2
            AND status <> 'invalidated' AND deleted_at IS NULL",
        &[&tenant_id, &submission_id],
    )
    .await?;
    tx.execute(
        "UPDATE trace_export_manifest_items
            SET source_invalidated_at = COALESCE(source_invalidated_at, NOW()),
                source_invalidation_reason = 'revoked', updated_at = NOW()
          WHERE tenant_id = $1 AND submission_id = $2
            AND source_invalidated_at IS NULL",
        &[&tenant_id, &submission_id],
    )
    .await?;
    tx.execute(
        "UPDATE trace_export_manifests
            SET invalidated_at = COALESCE(invalidated_at, NOW()), updated_at = NOW()
          WHERE tenant_id = $1 AND $2 = ANY(source_submission_ids)
            AND invalidated_at IS NULL AND deleted_at IS NULL",
        &[&tenant_id, &submission_id],
    )
    .await?;
    tx.execute(
        "UPDATE pipeline_export_snapshot_items
            SET invalidated_at = COALESCE(invalidated_at, NOW()),
                invalidation_reason = 'withdrawn'
          WHERE tenant_id = $1 AND submission_id = $2
            AND invalidated_at IS NULL",
        &[&tenant_id, &submission_id],
    )
    .await?;
    tx.execute(
        "UPDATE pipeline_export_snapshots snapshot
            SET state = 'invalidated',
                invalidated_at = COALESCE(snapshot.invalidated_at, NOW())
          WHERE snapshot.tenant_id = $1
            AND snapshot.state <> 'invalidated'
            AND EXISTS (
                SELECT 1
                  FROM pipeline_export_snapshot_items item
                 WHERE item.tenant_id = snapshot.tenant_id
                   AND item.snapshot_id = snapshot.snapshot_id
                   AND item.submission_id = $2
            )",
        &[&tenant_id, &submission_id],
    )
    .await?;
    let metadata_json = serde_json::json!({"source": "versioned_pipeline"});
    for row in &object_rows {
        let object_ref_id: Uuid = row.get("object_ref_id");
        let idempotency_key = sha256_prefixed(
            format!(
                "pipeline-withdrawal-object-delete:v1:{tenant_id}:{submission_id}:{object_ref_id}"
            )
            .as_bytes(),
        );
        let propagation_item_id = Uuid::new_v5(&Uuid::NAMESPACE_URL, idempotency_key.as_bytes());
        let target_json = serde_json::json!({
            "kind": "object_ref",
            "object_ref_id": object_ref_id,
        });
        tx.execute(
            "INSERT INTO trace_revocation_propagation_items (
                tenant_id, propagation_item_id, source_submission_id, trace_id,
                target_kind, target_json, action, status, idempotency_key, reason,
                attempt_count, metadata_json
             ) VALUES (
                $1,$2,$3,$4,'object_ref',$5,'delete_object_payload','pending',$6,
                'pipeline_withdrawal',0,$7
             )
             ON CONFLICT (tenant_id, idempotency_key) DO NOTHING",
            &[
                &tenant_id,
                &propagation_item_id,
                &submission_id,
                &trace_id,
                &target_json,
                &idempotency_key,
                &metadata_json,
            ],
        )
        .await?;
    }
    Ok(())
}

fn stale_lease_error() -> DatabaseError {
    DatabaseError::Constraint("pipeline lease is stale".to_string())
}

/// True for exactly the `DatabaseError` `stale_lease_error()` builds --
/// every mark_*/commit_*/update_settlement call in this store raises this
/// one when its lease-checked `WHERE` finds no row, and `ensure_current_lease`
/// raises it directly. Matches by variant *and* message, not variant alone:
/// `DatabaseError::Constraint` also carries unrelated refusals (a phase
/// mismatch, an invalid lease duration), so the variant alone is not proof of
/// a stale lease.
fn is_stale_lease_db_error(error: &DatabaseError) -> bool {
    matches!(error, DatabaseError::Constraint(message) if message == "pipeline lease is stale")
}

/// One helper for both shapes a stale lease reaches `process_claimed_run`
/// in -- a `DatabaseError::Constraint` from any mark_*/commit_* call (case
/// A, when the phase's own commit already found the lease gone, and case B,
/// when the phase failed for another reason and the follow-up mark_* call
/// finds it gone), or `ensure_live_lease`'s bare `anyhow!("pipeline lease is
/// stale")` (case A's service-level check before an external effect, which
/// has no open transaction and so no `DatabaseError` to carry). Checked by
/// type first: a `DatabaseError`'s `Display` prefixes every message with
/// "Constraint violation: ", so comparing the *outer* anyhow message against
/// the bare "pipeline lease is stale" text would silently miss every case
/// that reached here as a `DatabaseError`.
fn is_stale_lease_error(error: &anyhow::Error) -> bool {
    match error.downcast_ref::<DatabaseError>() {
        Some(db_error) => is_stale_lease_db_error(db_error),
        None => error.to_string() == "pipeline lease is stale",
    }
}

/// A leg write under a live lease that found the leg in a
/// state it may not change (or missing). Its own error, so it is never
/// recorded as `lease_expired`.
fn settlement_leg_not_open_error() -> DatabaseError {
    DatabaseError::Constraint(PIPELINE_SETTLEMENT_LEG_NOT_OPEN_LABEL.to_string())
}

/// The SQLSTATEs that mean the database could not
/// serve the statement right now, not that the statement or its data was
/// wrong: class 08 (connection exception), 40001 (serialization failure),
/// 40P01 (deadlock detected), 57P01 to 57P03 (shutdown, crash shutdown,
/// cannot connect now), and 53300 (too many connections).
fn is_transient_sqlstate(code: &tokio_postgres::error::SqlState) -> bool {
    let code = code.code();
    code.starts_with("08")
        || matches!(
            code,
            "40001" | "40P01" | "57P01" | "57P02" | "57P03" | "53300"
        )
}

/// These are the transient cases for one driver error: a SQLSTATE in
/// `is_transient_sqlstate`, or no SQLSTATE because the server never
/// answered -- the connection is closed, or the cause is an I/O error
/// (sending, receiving, connecting). A driver error with no SQLSTATE and
/// no I/O cause (a row-count mismatch, a type conversion, a parameter
/// count) is a local error in this code, not the database being
/// unavailable, and keeps P2.
fn is_transient_postgres_error(error: &tokio_postgres::Error) -> bool {
    match error.code() {
        Some(code) => is_transient_sqlstate(code),
        None => {
            error.is_closed()
                || std::error::Error::source(error)
                    .is_some_and(|cause| cause.is::<std::io::Error>())
        }
    }
}

/// Whether `error` is a transient database failure,
/// which is never the trace's fault in any phase. Classified by downcast,
/// along the whole error chain, because store code returns `DatabaseError`
/// while service code wraps raw driver and pool errors with `?`: the pool
/// could not give a connection (`DatabaseError::Pool`,
/// `DatabaseError::PoolRuntime`, or a raw `deadpool_postgres::PoolError`),
/// or a `tokio_postgres::Error` is transient by `is_transient_postgres_error`.
/// Every other error keeps P2.
fn is_transient_database_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        if let Some(database_error) = cause.downcast_ref::<DatabaseError>() {
            return matches!(
                database_error,
                DatabaseError::Pool(_) | DatabaseError::PoolRuntime(_)
            );
        }
        if cause.is::<deadpool_postgres::PoolError>() {
            return true;
        }
        cause
            .downcast_ref::<tokio_postgres::Error>()
            .is_some_and(is_transient_postgres_error)
    })
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
        admission_reason: row.get("admission_reason"),
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
    Ok(PipelineSettlementRecord {
        tenant_id: row.get("tenant_id"),
        run_id: row.get("run_id"),
        instrument_id: row.get("instrument_id"),
        atomic_units,
        operation_ref_hash: row.get("operation_ref_hash"),
        result_ref_hash: row.get("result_ref_hash"),
        external_receipt_hash: row.get("external_receipt_hash"),
        operation_state: row.get("operation_state"),
        credit_event_id: row.get("credit_event_id"),
        settlement_batch_id: row.get("settlement_batch_id"),
        payout_rail: row.get("payout_rail"),
        payout_state: row.get("payout_state"),
        attempt_count,
        last_error_label: row.get("last_error_label"),
        dispatched_at: row.get("dispatched_at"),
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
            // A stored Score decision loads without its manifest, so it
            // loads unverified; Settle verifies it against the run's bound
            // manifest before it uses it.
            let _: UnverifiedScoreDecision = decode(decision)?;
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

/// Per-instrument settlement caps the service enforces in Settle. An
/// instrument with no entry is a configuration gap: the run waits,
/// uncharged, as `settlement_cap_missing`, before the leg's adapter is
/// called. An amount above its entry is a spend limit: the leg fails as
/// `credit_cap_exceeded` and the retry is charged.
#[derive(Debug, Clone)]
pub struct PipelineCaps {
    pub per_instrument_atomic_units: BTreeMap<String, AtomicUnits>,
}

/// NEAR payout of settled Trace Credit (P3-D11). Disabled unless a service
/// is built `with_payout` and `enabled`. Confirmation evidence cannot be
/// turned off while payout is enabled (`PipelineServiceBuilder::build`
/// refuses that combination).
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
#[derive(Debug, Clone)]
pub struct PipelinePayoutConfig {
    pub enabled: bool,
    pub require_confirmation_evidence: bool,
    pub near_contract_id: Option<String>,
    pub confirmation_interval: std::time::Duration,
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
///   settlement for a paid leg. With a non-empty list and no issuer, every
///   positive award is withheld and no payout is made.
/// - `require_production_gate` is `main`'s
///   `TRACE_COMMONS_NOVELTY_UTILITY_REQUIRE_PRODUCTION_GATE` (Ruling T15-11).
/// - `settlement_allowed_policy_versions` is `main`'s
///   `TRACE_COMMONS_CREDIT_SETTLEMENT_ALLOWED_POLICY_VERSIONS`. Empty allows,
///   as on `main`.
/// - `settlement_require_issuer_approval` is `main`'s
///   `TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_ISSUER_APPROVAL`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PipelineNoveltyUtilityChecks {
    pub central_issuer_principal_refs: std::collections::BTreeSet<String>,
    pub issuer_principal_ref: Option<String>,
    pub require_production_gate: bool,
    pub settlement_allowed_policy_versions: std::collections::BTreeSet<String>,
    pub settlement_require_issuer_approval: bool,
}

/// `main`'s withheld-reason labels for a `NoveltyUtility` credit its checks
/// refuse (`attempt_emit_novelty_utility_credit` in `trace-commons-ingest.rs`,
/// and `CREDIT_CHECK_ERROR_LABEL` there), which a withheld compatibility leg
/// records as its `last_error_label` (Ruling T15-12).
pub const PIPELINE_NOVELTY_UTILITY_NON_PRODUCTION_GATE_LABEL: &str = "non_production_gate";
pub const PIPELINE_NOVELTY_UTILITY_CENTRAL_ISSUER_DENIED_LABEL: &str = "central_issuer_denied";
pub const PIPELINE_NOVELTY_UTILITY_CREDIT_CHECK_ERROR_LABEL: &str = "credit_check_error";
pub const PIPELINE_NOVELTY_UTILITY_POLICY_MISMATCH_LABEL: &str = "policy_mismatch";

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
/// `authority` and `privacy` follow the same shape (Ruling T2-2): true only
/// when the held object is `production_qualified()`, false when the service
/// holds none at all (`submit` already fails closed on that case before any
/// dependency check runs). `payout` is the same for the NEAR payout adapter:
/// true only when the service holds one and it is `production_qualified()`.
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
    object_store_name: String,
    lease_config: PipelineLeaseConfig,
    crash_point: Option<PipelineCrashPoint>,
    authority: Option<Arc<dyn PipelineAuthorityProvider>>,
    privacy: Option<Arc<dyn PipelinePrivacyBoundary>>,
    payout: Option<(Arc<dyn NearPayoutAdapter>, PipelinePayoutConfig)>,
    novelty_utility_checks: PipelineNoveltyUtilityChecks,
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
            object_store_name: PIPELINE_DEFAULT_OBJECT_STORE_NAME.to_string(),
            lease_config: PipelineLeaseConfig::default(),
            crash_point: None,
            authority: None,
            privacy: None,
            payout: None,
            novelty_utility_checks: PipelineNoveltyUtilityChecks::default(),
        }
    }

    /// The name of the object store `artifact_store` writes to, recorded as
    /// `trace_object_refs.object_store` on every object ref the pipeline
    /// commits -- the same label the legacy receipt records from the
    /// configured store (`ConfiguredTraceArtifactStore::object_store_name`),
    /// which the legacy readers match before they read an object.
    pub fn with_object_store_name(mut self, object_store_name: impl Into<String>) -> Self {
        self.object_store_name = object_store_name.into();
        self
    }

    /// The per-phase claim lease `process_one`/`process_run` use.
    /// Defaults to `PipelineLeaseConfig::default()` when not called.
    pub fn with_lease_config(mut self, lease_config: PipelineLeaseConfig) -> Self {
        self.lease_config = lease_config;
        self
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

    /// The tenant authority provider `submit` consults before it does any
    /// database work. A service built without this fails every receipt
    /// closed with `authority_control_missing`.
    pub fn with_authority(mut self, authority: Arc<dyn PipelineAuthorityProvider>) -> Self {
        self.authority = Some(authority);
        self
    }

    /// The privacy boundary `submit` rescrubs the server envelope through
    /// before it stages or stores anything. A service built without this
    /// fails every receipt closed with `privacy_control_missing`.
    pub fn with_privacy(mut self, privacy: Arc<dyn PipelinePrivacyBoundary>) -> Self {
        self.privacy = Some(privacy);
        self
    }

    /// The NEAR adapter and configuration the payout pass
    /// (`PipelineService::process_payouts`) uses. A service built without
    /// this, or with `enabled: false`, pays nothing out, and Score seeds
    /// every leg's payout as `disabled`.
    pub fn with_payout(
        mut self,
        adapter: Arc<dyn NearPayoutAdapter>,
        config: PipelinePayoutConfig,
    ) -> Self {
        self.payout = Some((adapter, config));
        self
    }

    /// The configuration of `main`'s `NoveltyUtility` credit checks, which a
    /// compatibility run's Trace Credit leg applies (Ruling T15-6). Defaults
    /// to no issuer allowlist, no pipeline issuer, and no production-gate
    /// requirement when not called.
    pub fn with_novelty_utility_checks(mut self, checks: PipelineNoveltyUtilityChecks) -> Self {
        self.novelty_utility_checks = checks;
        self
    }

    /// Resolves the default package once, so a service that cannot run its
    /// own default bundle fails at construction rather than on the first
    /// receipt.
    pub fn build(self) -> anyhow::Result<PipelineService> {
        if let Some((_, config)) = self.payout.as_ref().filter(|(_, config)| config.enabled) {
            anyhow::ensure!(
                config.require_confirmation_evidence,
                "payout confirmation evidence cannot be disabled"
            );
            // Ruling T10-4: an enabled payout names a NEAR contract, and one
            // a call can be built for.
            let near_contract_id = config
                .near_contract_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!(PIPELINE_PAYOUT_NEAR_CONTRACT_MISSING_LABEL))?;
            let probe = sha256_prefixed(b"pipeline_payout_near_contract_probe");
            disabled_near_call(near_contract_id, Uuid::nil(), &probe, &probe, 1)
                .map_err(|_| anyhow::anyhow!(PIPELINE_PAYOUT_NEAR_CONTRACT_INVALID_LABEL))?;
            // Zaki review 1, item 2: `main` finalizes a live settlement under
            // `TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_ISSUER_APPROVAL` only
            // with approval evidence an operator recorded for that batch's
            // source list and named in the request. Settle finalizes a
            // pipeline batch with no request to name it in, so there is no
            // equivalent check to make: an enabled payout is refused, as
            // `main` refuses its own automated live settlement under the
            // flag, under `main`'s label for missing approval evidence.
            anyhow::ensure!(
                !self
                    .novelty_utility_checks
                    .settlement_require_issuer_approval,
                PIPELINE_PAYOUT_ISSUER_APPROVAL_MISSING_LABEL
            );
        }
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
            object_store_name: self.object_store_name,
            lease_config: self.lease_config,
            crash_point: self.crash_point,
            crash_pending: AtomicBool::new(self.crash_point.is_some()),
            score_evaluations: AtomicUsize::new(0),
            settle_evaluations: AtomicUsize::new(0),
            authority: self.authority,
            privacy: self.privacy,
            payout: self.payout,
            novelty_utility_checks: self.novelty_utility_checks,
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
    caps: PipelineCaps,
    object_store_name: String,
    lease_config: PipelineLeaseConfig,
    crash_point: Option<PipelineCrashPoint>,
    crash_pending: AtomicBool,
    score_evaluations: AtomicUsize,
    settle_evaluations: AtomicUsize,
    authority: Option<Arc<dyn PipelineAuthorityProvider>>,
    privacy: Option<Arc<dyn PipelinePrivacyBoundary>>,
    payout: Option<(Arc<dyn NearPayoutAdapter>, PipelinePayoutConfig)>,
    novelty_utility_checks: PipelineNoveltyUtilityChecks,
}

impl PipelineService {
    pub fn bundle_id(&self) -> &str {
        &self.default_package.bundle_id
    }

    /// The `trace_object_refs.object_store` label this service records.
    pub fn object_store_name(&self) -> &str {
        &self.object_store_name
    }

    /// The per-phase claim lease this service's `process_one`/`process_run`
    /// use.
    pub fn lease_config(&self) -> PipelineLeaseConfig {
        self.lease_config
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
            authority: self
                .authority
                .as_ref()
                .is_some_and(|authority| authority.production_qualified()),
            privacy: self
                .privacy
                .as_ref()
                .is_some_and(|privacy| privacy.production_qualified()),
            payout: self
                .payout
                .as_ref()
                .is_some_and(|(adapter, _)| adapter.production_qualified()),
        }
    }

    /// Whether this service pays settled Trace Credit out through NEAR: it
    /// holds a payout adapter (`with_payout`) whose configuration is
    /// enabled.
    pub fn payout_enabled(&self) -> bool {
        self.payout
            .as_ref()
            .is_some_and(|(_, config)| config.enabled)
    }

    /// How often an enabled payout polls a `submitted` payout for its
    /// confirmation (`PipelinePayoutConfig::confirmation_interval`); `None`
    /// while payout is disabled.
    pub fn payout_confirmation_interval(&self) -> Option<std::time::Duration> {
        self.payout
            .as_ref()
            .filter(|(_, config)| config.enabled)
            .map(|(_, config)| config.confirmation_interval)
    }

    /// The configuration of `main`'s `NoveltyUtility` credit checks this
    /// service applies (`PipelineServiceBuilder::with_novelty_utility_checks`).
    pub fn novelty_utility_checks(&self) -> &PipelineNoveltyUtilityChecks {
        &self.novelty_utility_checks
    }

    /// The NEAR credit contract an enabled payout names
    /// (`PipelinePayoutConfig::near_contract_id`); `None` while payout is
    /// disabled.
    pub fn payout_near_contract_id(&self) -> Option<&str> {
        self.payout
            .as_ref()
            .filter(|(_, config)| config.enabled)
            .and_then(|(_, config)| config.near_contract_id.as_deref())
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

    /// An operator action; see `PgPipelineStore::activate_bundle`. A service
    /// built with the ingest runtime login cannot call this successfully.
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

    pub async fn withdraw_submission(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        actor_principal_ref: &str,
        account_id: Option<Uuid>,
    ) -> anyhow::Result<PipelineWithdrawalOutcome> {
        Ok(self
            .store
            .withdraw_submission(tenant_id, submission_id, actor_principal_ref, account_id)
            .await?)
    }

    /// Processes up to `limit` of `tenant_id`'s due index invalidations
    /// (`process_index_invalidation` each) and returns how many it recorded
    /// a result for. Only `tenant_id`'s own queue is read. A database error
    /// ends the pass; the invalidation it was on waits out its claim's
    /// lease, uncharged.
    pub async fn process_index_invalidations(
        &self,
        tenant_id: &str,
        limit: usize,
    ) -> anyhow::Result<usize> {
        let mut processed = 0;
        for run_id in self
            .store
            .list_due_index_invalidations(tenant_id, limit)
            .await?
        {
            if self
                .process_index_invalidation(tenant_id, run_id)
                .await?
                .is_some()
            {
                processed += 1;
            }
        }
        Ok(processed)
    }

    /// One attempt at `run_id`'s queued index invalidation: claims it
    /// (`PgPipelineStore::claim_index_invalidation`, leased for the Settle
    /// phase's lease, since Settle is the phase that writes the index),
    /// removes the queued revision from the index under the tenant's
    /// storage reference and the index id of the run's committed Score
    /// evidence, and records the result. `Ok(true)` or `Ok(false)` from the
    /// index completes the invalidation. An index outage (`Failed` or
    /// `Uncertain`) is an uncharged retry with backoff
    /// (`retry_index_invalidation`, Ruling T8-2), so an outage never ends
    /// the invalidation. Only a failure waiting cannot heal is charged
    /// (`fail_index_invalidation`), and ends `failed` once the attempts run
    /// out: Score evidence that names no index, or a `ContentConflict`,
    /// which the `invalidate_revision` contract rules out.
    ///
    /// Returns the run as the recorded result left it, or `None` when this
    /// call recorded nothing: the invalidation was not due or not claimed,
    /// or another claim recorded a result first. Each step checks out its
    /// own pooled connection and returns it before the next, and none is
    /// held across the index call.
    pub async fn process_index_invalidation(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> anyhow::Result<Option<PipelineRunRecord>> {
        let Some(claim) = self
            .store
            .claim_index_invalidation(tenant_id, run_id, self.lease_config.settle())
            .await?
        else {
            return Ok(None);
        };
        let Some(index_id) = self.committed_index_id(tenant_id, run_id).await? else {
            return Ok(self.store.fail_index_invalidation(&claim).await?);
        };
        let result = self.index_writer.invalidate_revision(
            &pipeline_tenant_storage_ref(tenant_id),
            &index_id,
            claim.registry_revision_id,
        );
        Ok(match result {
            Ok(_) => self.store.complete_index_invalidation(&claim).await?,
            Err(IndexWriteError::Failed | IndexWriteError::Uncertain) => {
                self.store.retry_index_invalidation(&claim).await?
            }
            Err(IndexWriteError::ContentConflict) => {
                self.store.fail_index_invalidation(&claim).await?
            }
        })
    }

    /// The index id the run's committed Score evidence names: the index its
    /// Settle wrote to. `None` when there is no Score outcome, or its
    /// evidence does not decode or names no index.
    async fn committed_index_id(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> Result<Option<String>, DatabaseError> {
        let Some(outcome) = self
            .store
            .outcome_for_phase(tenant_id, run_id, Phase::Score)
            .await?
        else {
            return Ok(None);
        };
        Ok(serde_json::from_value::<ScoreEvidence>(outcome.evidence)
            .ok()
            .and_then(|evidence| evidence.index_id))
    }

    /// Registers this service's default bundle for `tenant_id` and, if the
    /// tenant has no active bundle yet, activates it. Idempotent: calling it
    /// again for a tenant that already has an active bundle changes
    /// nothing.
    ///
    /// Ingest startup calls this once for every rollout tenant before it
    /// starts taking receipts, so a receipt never has to. A test that calls
    /// `submit` directly, on a tenant startup never touched, must call this
    /// first for the same reason.
    pub async fn register_default_bundle(&self, tenant_id: &str) -> anyhow::Result<()> {
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

    /// Whether the submission behind a run is still operable right now
    /// (`PgPipelineStore::submission_guard_on_tx`), read in a transaction of
    /// its own that commits at once. Settle reads this fresh before it
    /// decides index membership and again before its settlement legs,
    /// since those checks can be far apart in wall-clock time across a
    /// crash and retry. The index dispatch does not use it: it reads the
    /// guard on its own transaction and holds it until the write commits.
    async fn submission_guard(&self, run: &PipelineRunRecord) -> anyhow::Result<SubmissionGuard> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = PgPipelineStore::tenant_transaction(&mut client, &run.tenant_id).await?;
        let guard = PgPipelineStore::submission_guard_on_tx(&tx, run).await?;
        tx.commit().await?;
        Ok(guard)
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
            .map_err(|error| bound_bundle_load_error_label(&error))?
            .ok_or(PIPELINE_BUNDLE_MISSING_LABEL)?;
        if !self
            .store
            .policy_is_runnable(&run.tenant_id, &run.bundle_id, phase)
            .await
            .map_err(|error| bound_bundle_load_error_label(&error))?
        {
            return Err(PIPELINE_POLICY_NOT_RUNNABLE_LABEL);
        }
        self.construct(package)
    }

    /// Accepts a receipt as one attempt. The attempt's object
    /// is recorded before it is written, and no lock is held while it is
    /// written or while Admission runs:
    ///
    /// 1. The tenant authority lookup and the privacy-boundary presence
    ///    check run before any database work (Ruling T2-1): a tenant with no
    ///    configured authority fails closed with
    ///    `authority_control_missing`, and a service built without a privacy
    ///    boundary fails closed with `privacy_control_missing`. Neither
    ///    creates a run or a staging row.
    /// 2. A read-only check with no advisory lock (`precheck_receipt`)
    ///    refuses the common cases -- replay, content conflict, tombstone,
    ///    quota -- before any encryption. The staging transaction repeats
    ///    every check, so this one only saves work. A replayed key returns
    ///    here, so a replay never calls the privacy boundary again.
    /// 3. The rescrub: the privacy boundary transforms a clone of the
    ///    server envelope and returns any residual-risk conditions it found,
    ///    merged into the caller's own basis. From here on the transformed
    ///    envelope replaces `request.server_envelope` everywhere -- the
    ///    staged and stored source bytes, the retention derivation, and the
    ///    Admission input -- while `request_content_hash` stays the hash of
    ///    the raw `request.request_bytes` (replay identity). This runs with
    ///    no pooled connection held, since a rescrub can call an external
    ///    classifier.
    /// 4. The attempt's object -- the (now transformed) server envelope,
    ///    wrapped per decision P1 -- is encrypted under the attempt's own
    ///    object id (`pipeline_receipt_object_id`, a fresh random attempt
    ///    id), which fixes its object key and ciphertext hash. Nothing is
    ///    stored.
    /// 5. The staging transaction (`stage_receipt_attempt`) checks replay, an
    ///    attempt for the key staged with other content, the bound bundle,
    ///    tombstones, and the quota -- in that order -- counts the quota,
    ///    and inserts the attempt's `staged` row naming the object. It
    ///    commits before anything is stored, so every refusal there stores
    ///    nothing.
    /// 6. The object is written and Admission runs, with no transaction
    ///    open. A failure from here on (an error or a crash) leaves the
    ///    `staged` row naming the object; `sweep_staged_receipts` deletes
    ///    both once the row's `cleanup_after` passes.
    /// 7. The final transaction (`commit_receipt_attempt`) re-checks an
    ///    existing run for the key and the tombstones, requires the
    ///    attempt's row to be still `staged` and not yet due, and commits
    ///    the records and the row's move to `committed` together. On a
    ///    refusal there, the attempt deletes its own object and row
    ///    (`discard_receipt_attempt`).
    ///
    /// Each attempt writes its own object, so two concurrent receipts for
    /// one key never overwrite each other's object: the one that commits
    /// second finds the first one's run and returns `Replayed`. A tenant
    /// with no active bundle is not registered here -- that happens once,
    /// at startup (`register_default_bundle`), and a tenant this call finds
    /// without one fails closed with `PIPELINE_BUNDLE_MISSING_LABEL`. Every
    /// synchronous object-store call (the encrypt-and-prepare in step 2, the
    /// write in step 4, and a discarded attempt's delete) runs on a blocking
    /// thread rather than this task, and each transaction returns its
    /// connection to the pool before the next step checks one out, which is
    /// what keeps a pool of size one safe.
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
        let request_content_hash = sha256_prefixed(request.request_bytes);
        let request_idempotency_key_hash =
            sha256_prefixed(request.request_idempotency_key.as_bytes());

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

        // 1. The authority lookup and the privacy-boundary presence check
        // run before any database work, so a tenant with a missing control
        // fails closed with no run and no staging row (Ruling T2-1).
        let authority = self
            .authority
            .as_ref()
            .and_then(|provider| provider.authority_for_tenant(tenant_id))
            .ok_or_else(|| anyhow::anyhow!(PIPELINE_AUTHORITY_CONTROL_MISSING_LABEL))?;
        let privacy = self
            .privacy
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!(PIPELINE_PRIVACY_CONTROL_MISSING_LABEL))?;

        // 2. The early, lock-free refusal check.
        if let Some(refused) = self
            .precheck_receipt(
                &request,
                &request_idempotency_key_hash,
                &request_content_hash,
            )
            .await?
        {
            return Ok(refused);
        }

        // 3. The rescrub: after the lock-free precheck (a replay or a
        // refusal returns before this, so a replay never calls the
        // classifier again) and before the staging row, with no pooled
        // connection held. From here on, the transformed envelope replaces
        // `request.server_envelope` everywhere PR 2 used it: the staged and
        // stored source bytes, the retention derivation, and the Admission
        // input. `request_content_hash` (above) stays the hash of the raw
        // `request.request_bytes` -- the replay identity never moves.
        let mut envelope = request.server_envelope.clone();
        let mut consent_scopes = envelope.consent.scopes.clone();
        if !consent_scopes.contains(&envelope.trace_card.consent_scope) {
            consent_scopes.push(envelope.trace_card.consent_scope);
        }
        let grant_valid = authority.permits(&consent_scopes, &envelope.trace_card.allowed_uses);
        let mut residual_risk_basis = request.residual_risk_basis.to_vec();
        for condition in privacy
            .rescrub(&mut envelope)
            .await
            .map_err(|_| anyhow::anyhow!(PIPELINE_PRIVACY_CLASSIFICATION_FAILED_LABEL))?
        {
            if !residual_risk_basis.contains(&condition) {
                residual_risk_basis.push(condition);
            }
        }
        let request = PipelineReceiptRequest {
            tenant_id: request.tenant_id,
            actor_principal_ref: request.actor_principal_ref,
            counts_toward_quota: request.counts_toward_quota,
            request_idempotency_key: request.request_idempotency_key,
            request_bytes: request.request_bytes,
            server_envelope: &envelope,
            residual_risk_basis: &residual_risk_basis,
            limits: request.limits,
        };
        let envelope = request.server_envelope;

        // 4. Prepare this attempt's object. Nothing is stored yet. The
        // store's encrypt call is synchronous (it may reach a remote
        // key-wrap service), so it runs on a blocking thread rather than
        // this task.
        let tenant_storage_ref = pipeline_tenant_storage_ref(tenant_id);
        let server_envelope_bytes = serde_json::to_vec(envelope)?;
        let wrapper = encode_pipeline_artifact_bytes(&server_envelope_bytes)?;
        let attempt_id = Uuid::new_v4();
        let object_id = pipeline_receipt_object_id(run_id, attempt_id);
        let prepared = {
            let store = self.artifact_store.clone();
            let tenant_storage_ref = tenant_storage_ref.clone();
            tokio::task::spawn_blocking(move || {
                store.prepare_serialized_json(
                    tenant_storage_ref.as_str(),
                    TraceArtifactKind::ContributionEnvelope,
                    &object_id,
                    &wrapper,
                )
            })
            .await
            .map_err(|_| anyhow::anyhow!(PIPELINE_RECEIPT_OBJECT_TASK_FAILED_LABEL))??
        };
        let attempt = ReceiptAttempt {
            tenant_id: tenant_id.to_string(),
            run_id,
            attempt_id,
            request_idempotency_key: request_idempotency_key_hash.clone(),
            request_content_hash: request_content_hash.clone(),
            receipt: prepared.receipt().clone(),
        };

        // 5. The staging transaction: every refusal, then the attempt's row.
        let (bundle_id, bundle) = match self.stage_receipt_attempt(&request, &attempt).await? {
            ReceiptStage::Refused(result) => return Ok(result),
            ReceiptStage::Staged { bundle_id, bundle } => (bundle_id, bundle),
        };

        // 6. Write the object the row names, then run Admission. The write
        // is synchronous file or network I/O, so it also runs on a blocking
        // thread -- with no transaction open and no advisory lock held (the
        // staging transaction above already committed and released both
        // locks it took).
        let written = {
            let store = self.artifact_store.clone();
            tokio::task::spawn_blocking(move || store.publish_serialized_json(&prepared))
                .await
                .map_err(|_| anyhow::anyhow!(PIPELINE_RECEIPT_OBJECT_TASK_FAILED_LABEL))??
        };
        if !written.matches_identity(&attempt.receipt) {
            // A store that wrote some other object than the one it
            // prepared: that object is not the one the row names, so delete
            // it here (best effort) and fail closed. The row's own object,
            // if the store wrote it too, is the sweeper's.
            let store = self.artifact_store.clone();
            let delete_tenant_storage_ref = tenant_storage_ref.clone();
            let delete_failed = match tokio::task::spawn_blocking(move || {
                store.delete_artifact(delete_tenant_storage_ref.as_str(), &written)
            })
            .await
            {
                Ok(result) => result.is_err(),
                Err(_) => true,
            };
            if delete_failed {
                tracing::warn!(
                    label = "pipeline_receipt_object_delete_failed",
                    "best-effort delete of a receipt object the store wrote \
                     under an unexpected key failed"
                );
            }
            anyhow::bail!(PIPELINE_RECEIPT_OBJECT_MISMATCH_LABEL);
        }
        self.inject_crash(PipelineCrashPoint::AfterArtifactStorage)?;

        // `authenticated` stays fixed `true` (decision D15): the legacy
        // handler already authenticated the request before ever calling
        // `submit`. Ruling T2-3: a missing authority source stops the
        // receipt outright (`PIPELINE_AUTHORITY_CONTROL_MISSING_LABEL`,
        // above, before any database work); reaching here means a source
        // was resolved and validated the request, so `authority_valid` is
        // `true`. Whether that source permits this submission's consent
        // scopes and allowed uses (`grant_valid`, computed above) is an
        // ordinary business decision, not a structural failure -- a denial
        // is an Admission Reject with reason `grant_invalid`, the same as
        // any other Reject, never `MinimalAdmissionPolicy`'s permanent
        // `authority_missing` error (SYS-003, reserved for the missing-
        // source case this receipt already handled).
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
            request_content_hash: request_content_hash.clone(),
            schema_version: envelope.schema_version.clone(),
            authenticated: true,
            authority_valid: true,
            contribution_path_valid: !envelope.ironclaw.version.trim().is_empty()
                && !envelope
                    .privacy
                    .redaction_pipeline_version
                    .trim()
                    .is_empty(),
            grant_valid,
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

        // 7. The final transaction.
        let run = NewPipelineRun {
            tenant_id: tenant_id.to_string(),
            run_id,
            submission_id: envelope.submission_id,
            trace_id: envelope.trace_id,
            bundle_id,
            request_idempotency_key: request_idempotency_key_hash,
            request_content_hash,
            source_object_ref_id: object_ref_id,
        };
        match self
            .commit_receipt_attempt(
                &request,
                &run,
                &attempt,
                server_envelope_bytes.len(),
                stored,
                &admission.decision,
            )
            .await?
        {
            ReceiptCommit::Created(created) => Ok(PipelineReceiptResult::Created(created)),
            ReceiptCommit::Refused(result) => {
                self.discard_receipt_attempt(&attempt).await;
                Ok(result)
            }
            ReceiptCommit::StagingMissing => {
                self.discard_receipt_attempt(&attempt).await;
                Err(anyhow::anyhow!(PIPELINE_RECEIPT_STAGING_MISSING_LABEL))
            }
        }
    }

    /// A read-only replay lookup for a caller that already knows a run may
    /// exist for this key and only needs to know what to hand back. Two
    /// callers use it: `trace-commons-ingest`'s `submit_trace_handler`, for
    /// a retried upload whose legacy admission ledger already marked it
    /// `completed` (that path never wrote a legacy file record for a
    /// pipeline-routed tenant, so it cannot replay from there); and
    /// `route_pipeline_receipt`, when the pipeline's own `submit` call
    /// itself returned `Replayed` or `ContentConflict`. Neither caller may
    /// hand back what this returns without checking ownership first -- see
    /// below.
    ///
    /// One short tenant transaction, read-only (`existing_receipt_run`, the
    /// same lookup `receipt_key_refusal` uses): `Replayed` for a run whose
    /// content hash matches `request_bytes`, `ContentConflict` for a run
    /// whose content differs, `None` when no run exists for the key yet.
    /// Unlike `precheck_receipt`, this never checks a staged attempt, a
    /// tombstone, or the quota, and it creates, stages, stores, or counts
    /// nothing -- `submit` keeps its own replay check
    /// (`receipt_key_refusal`) for the write path; this is a narrower read
    /// for a caller that isn't submitting anything new. It also does not
    /// need the re-scrubbed envelope, only the raw request bytes whose hash
    /// the key was recorded against.
    ///
    /// Also reads the run's `trace_submissions.auth_principal_ref` (the
    /// principal `submit` recorded when it first created the run --
    /// `PipelineRunRecord` itself carries only `submission_id`, not who
    /// submitted it) in the same transaction. The caller must check
    /// ownership against it before handing back either outcome: nothing
    /// upstream of this call proves the retrying principal is the one that
    /// made the original submission -- an admission anchor, for one, can
    /// cover more than one principal (one per device on the same account),
    /// so even a completed reservation does not prove it on its own.
    pub async fn replay_receipt(
        &self,
        tenant_id: &str,
        request_idempotency_key: &str,
        request_bytes: &[u8],
    ) -> anyhow::Result<Option<PipelineReplayReceipt>> {
        let request_content_hash = sha256_prefixed(request_bytes);
        let request_idempotency_key_hash = sha256_prefixed(request_idempotency_key.as_bytes());
        let mut client = self.backend.trace_pool().get().await?;
        let tx = PgPipelineStore::tenant_transaction(&mut client, tenant_id).await?;
        let Some(existing) =
            existing_receipt_run(&tx, tenant_id, &request_idempotency_key_hash).await?
        else {
            tx.commit().await?;
            return Ok(None);
        };
        let auth_principal_ref: String = tx
            .query_one(
                "SELECT auth_principal_ref FROM trace_submissions
                  WHERE tenant_id = $1 AND submission_id = $2",
                &[&tenant_id, &existing.submission_id],
            )
            .await?
            .get(0);
        let result = replay_result(existing, &request_content_hash);
        tx.commit().await?;
        Ok(Some(PipelineReplayReceipt {
            result,
            auth_principal_ref,
        }))
    }

    /// The receipt's early refusal check (`submit` step 1): one short
    /// tenant transaction that only reads and takes no
    /// advisory lock. It refuses the common cases -- an existing run for
    /// the key, an attempt staged with other content, a tombstone, and the
    /// quota (for a key not yet counted) -- before the attempt's object is
    /// encrypted, so a refused receipt costs no encryption and, on a remote
    /// store, no key-wrap call. It decides nothing on its own: the staging
    /// transaction repeats every check under its locks, since the state can
    /// change in between. Its connection returns to the pool before the
    /// next step checks one out.
    async fn precheck_receipt(
        &self,
        request: &PipelineReceiptRequest<'_>,
        request_idempotency_key: &str,
        request_content_hash: &str,
    ) -> anyhow::Result<Option<PipelineReceiptResult>> {
        let tenant_id = request.tenant_id;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = PgPipelineStore::tenant_transaction(&mut client, tenant_id).await?;
        let refused = if let Some(refused) = receipt_key_refusal(
            &tx,
            tenant_id,
            request_idempotency_key,
            request_content_hash,
        )
        .await?
        {
            Some(refused)
        } else if receipt_is_tombstoned(
            &tx,
            tenant_id,
            request.server_envelope,
            request_content_hash,
        )
        .await?
        {
            Some(PipelineReceiptResult::Tombstoned)
        } else if request.counts_toward_quota {
            let principal_ref_hash = sha256_prefixed(request.actor_principal_ref.as_bytes());
            receipt_quota_refusal(&tx, request, request_idempotency_key, &principal_ref_hash)
                .await?
                .map(PipelineReceiptResult::QuotaExceeded)
        } else {
            None
        };
        tx.commit().await?;
        Ok(refused)
    }

    /// The receipt's staging transaction (`submit` step 3). It takes the
    /// receipt lock and then the tenant quota lock -- transaction-scoped
    /// advisory locks that serialize concurrent receipts for one key (or
    /// one tenant's quota) without a row lock that would block other
    /// tenants -- and releases both when it commits, before the object is
    /// written. In order: an existing run for the key (`Replayed` or
    /// `ContentConflict`); an attempt for the key staged with other content
    /// (`ContentConflict`); the bound bundle; tombstones (`Tombstoned`); the
    /// quota (`QuotaExceeded`). The quota is counted here
    /// (`pipeline_admission_usage`, once per key), so a receipt that fails
    /// after its write stays counted, and a retry of that key is neither
    /// counted again nor refused for its own earlier count. Last, the
    /// attempt's `staged` row.
    async fn stage_receipt_attempt(
        &self,
        request: &PipelineReceiptRequest<'_>,
        attempt: &ReceiptAttempt,
    ) -> anyhow::Result<ReceiptStage> {
        let tenant_id = request.tenant_id;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = PgPipelineStore::tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
            &[&pipeline_receipt_lock(
                tenant_id,
                &attempt.request_idempotency_key,
            )],
        )
        .await?;
        let quota_lock = format!("pipeline-quota:{tenant_id}");
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 1))",
            &[&quota_lock],
        )
        .await?;

        if let Some(refused) = receipt_key_refusal(
            &tx,
            tenant_id,
            &attempt.request_idempotency_key,
            &attempt.request_content_hash,
        )
        .await?
        {
            tx.commit().await?;
            return Ok(ReceiptStage::Refused(refused));
        }

        // Bound bundle: the active bundle id (from pipeline_active_bundles
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

        // A tombstone hit is a normal outcome, not an error: nothing has
        // been staged or stored.
        if receipt_is_tombstoned(
            &tx,
            tenant_id,
            request.server_envelope,
            &attempt.request_content_hash,
        )
        .await?
        {
            tx.commit().await?;
            return Ok(ReceiptStage::Refused(PipelineReceiptResult::Tombstoned));
        }

        // Quota, only when the caller wants this receipt counted.
        if request.counts_toward_quota {
            let principal_ref_hash = sha256_prefixed(request.actor_principal_ref.as_bytes());
            if let Some(scope) = receipt_quota_refusal(
                &tx,
                request,
                &attempt.request_idempotency_key,
                &principal_ref_hash,
            )
            .await?
            {
                tx.commit().await?;
                return Ok(ReceiptStage::Refused(PipelineReceiptResult::QuotaExceeded(
                    scope,
                )));
            }
            tx.execute(
                "INSERT INTO pipeline_admission_usage (
                    tenant_id, request_idempotency_key, principal_ref_hash
                 ) VALUES ($1, $2, $3)
                 ON CONFLICT (tenant_id, request_idempotency_key) DO NOTHING",
                &[
                    &tenant_id,
                    &attempt.request_idempotency_key,
                    &principal_ref_hash,
                ],
            )
            .await?;
        }

        PgPipelineStore::stage_receipt_artifact(&tx, attempt).await?;
        tx.commit().await?;
        Ok(ReceiptStage::Staged { bundle_id, bundle })
    }

    /// The receipt's final transaction (`submit` step 5). It takes the
    /// receipt lock again -- its only advisory lock -- so it commits one
    /// attempt for the key at a time: the attempt that comes second finds
    /// the first one's run and returns `Replayed` rather than failing on
    /// the run's unique key or the submission id. Then it re-checks the
    /// tombstones (one can arrive while the object is written), locks the
    /// attempt's row, which must still be `staged` and not due
    /// (`lock_committable_receipt_artifact`), and inserts the records and
    /// the row's move to `committed` together.
    ///
    /// Retention is derived on the server, as the legacy receipt derives it
    /// (`retention_policy_for_trace` over the envelope's allowed uses and
    /// consent; expiry `received_at + max_age_days`), never taken from the
    /// envelope's own `trace_card.retention_policy`. `received_at` is the
    /// column default, this transaction's `NOW()`, read here so the expiry
    /// is anchored to the exact stored receipt time. No
    /// `pipeline_receipt_ownership` row is inserted (PR 5).
    async fn commit_receipt_attempt(
        &self,
        request: &PipelineReceiptRequest<'_>,
        run: &NewPipelineRun,
        attempt: &ReceiptAttempt,
        size_bytes: usize,
        stored: StoredPhaseResult,
        admission: &AdmissionDecision,
    ) -> anyhow::Result<ReceiptCommit> {
        let tenant_id = request.tenant_id;
        let envelope = request.server_envelope;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = PgPipelineStore::tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
            &[&pipeline_receipt_lock(
                tenant_id,
                &attempt.request_idempotency_key,
            )],
        )
        .await?;
        if let Some(existing) =
            existing_receipt_run(&tx, tenant_id, &attempt.request_idempotency_key).await?
        {
            tx.commit().await?;
            return Ok(ReceiptCommit::Refused(replay_result(
                existing,
                &attempt.request_content_hash,
            )));
        }
        if receipt_is_tombstoned(&tx, tenant_id, envelope, &attempt.request_content_hash).await? {
            tx.commit().await?;
            return Ok(ReceiptCommit::Refused(PipelineReceiptResult::Tombstoned));
        }
        if !PgPipelineStore::lock_committable_receipt_artifact(&tx, attempt).await? {
            tx.commit().await?;
            return Ok(ReceiptCommit::StagingMissing);
        }

        let retention_policy = retention_policy_for_trace(envelope);
        let received_at: DateTime<Utc> = tx.query_one("SELECT NOW()", &[]).await?.get(0);
        let expires_at = retention_policy
            .max_age_days
            .map(|days| received_at + Duration::days(i64::from(days)));
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
            retention_policy_id: retention_policy.name,
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
            expires_at,
        };
        let tenant_storage_ref = pipeline_tenant_storage_ref(tenant_id);
        let object_ref = TraceObjectRefWrite {
            object_ref_id: run.source_object_ref_id,
            tenant_id: tenant_id.to_string(),
            submission_id: envelope.submission_id,
            artifact_kind: TraceObjectArtifactKind::SubmittedEnvelope,
            object_store: self.object_store_name.clone(),
            object_key: attempt.receipt.object_key.clone(),
            content_sha256: format!("sha256:{}", attempt.receipt.ciphertext_sha256),
            encryption_key_ref: format!("tenant:{}", tenant_storage_ref.as_str()),
            size_bytes: i64::try_from(size_bytes).unwrap_or(i64::MAX),
            compression: None,
            created_by_job_id: None,
        };
        insert_receipt_records(
            &tx,
            run,
            attempt,
            &submission,
            &object_ref,
            stored,
            admission,
        )
        .await?;
        let row = tx
            .query_one(
                "SELECT * FROM pipeline_runs WHERE tenant_id = $1 AND run_id = $2",
                &[&tenant_id, &run.run_id],
            )
            .await?;
        let created = pipeline_run_from_row(&row)?;
        tx.commit().await?;
        Ok(ReceiptCommit::Created(created))
    }

    /// Removes an attempt the final transaction refused: its object, then
    /// its `staged` row. Best effort, with the sweeper as the backstop: when
    /// the delete fails, the row stays (or is inserted again, if the
    /// sweeper already removed it) and is made due now, so the next sweep
    /// retries it. Logs labels only. The delete is synchronous I/O, so it
    /// runs on a blocking thread rather than this task.
    async fn discard_receipt_attempt(&self, attempt: &ReceiptAttempt) {
        let tenant_storage_ref = pipeline_tenant_storage_ref(&attempt.tenant_id);
        let store = self.artifact_store.clone();
        let receipt = attempt.receipt.clone();
        let deleted = tokio::task::spawn_blocking(move || {
            store.delete_artifact(tenant_storage_ref.as_str(), &receipt)
        })
        .await;
        let cleanup = match deleted {
            Ok(Ok(_)) => self.store.remove_staged_receipt_artifact(attempt).await,
            Ok(Err(_)) | Err(_) => {
                tracing::warn!(
                    label = "pipeline_receipt_object_delete_failed",
                    "best-effort delete of a refused receipt attempt's object failed; \
                     the sweeper retries it"
                );
                self.store.restage_receipt_artifact_due_now(attempt).await
            }
        };
        if cleanup.is_err() {
            tracing::warn!(
                label = "pipeline_receipt_staging_cleanup_failed",
                "could not update a refused receipt attempt's staging row"
            );
        }
    }

    /// Deletes the objects of receipt attempts that never committed: in one
    /// tenant transaction, up to `limit` `staged` rows whose
    /// `cleanup_after` has passed, oldest first, each locked
    /// (`FOR UPDATE SKIP LOCKED`). For each, it deletes the object the row
    /// names and then the row. A delete that fails keeps its row for the
    /// next pass and logs a label only. Returns how many rows it removed.
    ///
    /// It never deletes an object a run references: an attempt's row moves
    /// to `committed` in the same transaction that inserts its object ref,
    /// under the row lock the sweeper skips, and a row this sweep holds is
    /// deleted before that transaction can lock it -- which then refuses
    /// (`ReceiptCommit::StagingMissing`). If this sweep fails after it
    /// deleted an object but before it commits, the row stays `staged` with
    /// its object gone; it is due, and the final transaction refuses a due
    /// row too (`lock_committable_receipt_artifact`). The object keys are
    /// unique per attempt (`UNIQUE (tenant_id, object_key)`).
    pub async fn sweep_staged_receipts(
        &self,
        tenant_id: &str,
        limit: usize,
    ) -> anyhow::Result<usize> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let tenant_storage_ref = pipeline_tenant_storage_ref(tenant_id);
        let mut client = self.backend.trace_pool().get().await?;
        let tx = PgPipelineStore::tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT run_id, attempt_id, object_key, ciphertext_sha256, staged_at
                   FROM pipeline_receipt_artifacts
                  WHERE tenant_id = $1 AND state = 'staged' AND cleanup_after <= NOW()
                  ORDER BY cleanup_after, run_id, attempt_id
                  LIMIT $2
                  FOR UPDATE SKIP LOCKED",
                &[&tenant_id, &limit],
            )
            .await?;
        let mut removed = 0;
        for row in &rows {
            let run_id: Uuid = row.get("run_id");
            let attempt_id: Uuid = row.get("attempt_id");
            let receipt = EncryptedTraceArtifactReceipt {
                tenant_storage_ref: tenant_storage_ref.as_str().to_string(),
                artifact_kind: TraceArtifactKind::ContributionEnvelope,
                object_key: row.get("object_key"),
                ciphertext_sha256: row.get("ciphertext_sha256"),
                encrypted_at: row.get("staged_at"),
            };
            if self
                .artifact_store
                .delete_artifact(tenant_storage_ref.as_str(), &receipt)
                .is_err()
            {
                tracing::warn!(
                    label = "pipeline_receipt_sweep_delete_failed",
                    "a staged receipt object could not be deleted; its row is kept \
                     for the next pass"
                );
                continue;
            }
            tx.execute(
                "DELETE FROM pipeline_receipt_artifacts
                  WHERE tenant_id = $1 AND run_id = $2 AND attempt_id = $3
                    AND state = 'staged'",
                &[&tenant_id, &run_id, &attempt_id],
            )
            .await?;
            removed += 1;
        }
        tx.commit().await?;
        Ok(removed)
    }

    /// Loads a run's committed outcome for `phase` and decodes its decision.
    async fn committed_decision<T: serde::de::DeserializeOwned>(
        &self,
        run: &PipelineRunRecord,
        phase: Phase,
    ) -> anyhow::Result<T> {
        let outcome = self
            .store
            .outcome_for_phase(&run.tenant_id, run.run_id, phase)
            .await?
            .ok_or_else(|| anyhow::anyhow!("{phase:?} outcome is missing"))?;
        serde_json::from_value(outcome.decision)
            .map_err(|_| anyhow::anyhow!("{phase:?} outcome is malformed"))
    }

    /// Reads and decrypts the bytes stored at `object_ref_id` for `run`,
    /// under the port's operability predicate (the submission must not be
    /// revoked/expired/purged or have a `trace_withdrawals` row, the object
    /// ref must not be invalidated or deleted), inside one tenant-scoped
    /// `FOR SHARE` transaction, then
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
                    AND NOT EXISTS (
                        SELECT 1 FROM trace_withdrawals w
                         WHERE w.tenant_id = submission.tenant_id
                           AND w.submission_id = submission.submission_id
                    )
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
    pub async fn load_source_bytes(&self, run: &PipelineRunRecord) -> anyhow::Result<Vec<u8>> {
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
    /// The claim itself picks the lease by the claimed row's own
    /// `next_phase`; see `PgPipelineStore::claim_next`.
    pub async fn process_one(&self, tenant_id: &str) -> anyhow::Result<Option<PipelineRunRecord>> {
        let Some(run) = self.store.claim_next(tenant_id, self.lease_config).await? else {
            return Ok(None);
        };
        self.process_claimed_run(run).await
    }

    /// Claims a specific run and advances it one phase. The claim itself
    /// picks the lease by the row's own `next_phase` in SQL -- there is no
    /// separate read of the run before the claim, so a phase commit that
    /// lands concurrently can
    /// never hand out a lease sized for a phase the row is no longer in.
    pub async fn process_run(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> anyhow::Result<Option<PipelineRunRecord>> {
        let Some(run) = self
            .store
            .claim_run_with_lease_config(tenant_id, run_id, self.lease_config)
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
            // D9 / FR3: a dependency the service does not hold, a bundle
            // whose operator-controlled runnable flag is off, or a store that
            // could not load the bound bundle is not this run's fault --
            // it waits in retry without the claim's attempt being charged,
            // with `mark_transient_retry`'s backoff.
            Err(label)
                if label == PIPELINE_DEPENDENCY_MISSING_LABEL
                    || label == PIPELINE_POLICY_NOT_RUNNABLE_LABEL
                    || label == PIPELINE_BUNDLE_STORE_UNAVAILABLE_LABEL =>
            {
                return self
                    .mark_transient_retry_or_record_lease_expired(&run, label)
                    .await;
            }
            Err(label) => {
                return self.mark_failed_or_record_lease_expired(&run, label).await;
            }
        };
        match self.process_claimed(&run, &bundle).await {
            Ok(updated) => Ok(Some(updated)),
            Err(error) if error.to_string() == INJECTED_PIPELINE_CRASH => Err(error),
            // The phase itself found its
            // own lease already gone -- from a commit's `ensure_current_lease`
            // re-check, from a commit's plain `WHERE ... lease_token = $t AND
            // lease_expires_at > NOW()` finding no row, or from
            // `ensure_live_lease`'s service-level check before an
            // uncommittable external effect (the index write, a settlement
            // adapter dispatch). Recorded as its own uncharged reason ahead
            // of every other P2 branch below -- a Score or Settle phase slow
            // enough to outrun its lease (a chunked NEAR AI scorer, a
            // CPU-bound embedder) must never be mistaken for
            // `index_key_conflict`, a Review-only inoperable submission, a
            // typed `PolicyError`, a missing settlement adapter, or the
            // generic `minimal_policy_failed` retry.
            Err(error) if is_stale_lease_error(&error) => {
                Ok(self.store.record_lease_expired(&run).await?)
            }
            // A transient database failure (the pool
            // could not give a connection, the connection failed, or a
            // serialization failure or deadlock) is not the trace's fault in
            // any phase -- the uncharged suspension, ahead of every charged
            // P2 branch below. In Settle this covers a failure after
            // `adapter.settle` returned: the leg stays `leased`, and the next
            // attempt repeats the idempotent adapter call. If the database is
            // still down, recording the suspension fails too and its error
            // is returned: nothing can be recorded, the lease expires, and
            // the next claim (or its sweep) handles the run.
            Err(error) if is_transient_database_error(&error) => {
                self.mark_transient_retry_or_record_lease_expired(
                    &run,
                    PIPELINE_DATABASE_UNAVAILABLE_LABEL,
                )
                .await
            }
            Err(error) => {
                let label = error.to_string();
                if label == PIPELINE_INDEX_CONFLICT_LABEL {
                    return self
                        .mark_failed_or_record_lease_expired(&run, PIPELINE_INDEX_CONFLICT_LABEL)
                        .await;
                }
                // In Review, an inoperable
                // submission (withdrawn, expired, or purged) is permanent --
                // the condition that caused it can never reverse -- so the
                // run ends terminally here rather than waiting out P2's
                // ordinary charged retry for this label. This covers both
                // `commit_review`'s own refusal and the existing
                // `load_object_bytes` refusal `load_source_bytes` hits
                // before Review even runs its policy. Every other phase
                // keeps the charged retry below (Score's read of the
                // already-validated approved bytes, for one).
                if label == PIPELINE_SUBMISSION_INOPERABLE_LABEL
                    && run.next_phase == Some(Phase::Review)
                {
                    return self
                        .mark_failed_or_record_lease_expired(
                            &run,
                            PIPELINE_SUBMISSION_INOPERABLE_LABEL,
                        )
                        .await;
                }
                // D9 / FR3: a typed `PolicyError` raised while a
                // phase runs is budgeted by kind, ahead of the P2 string
                // allowlist below -- a transient failure (an outage or a
                // timeout) is the uncharged suspension with backoff, exactly
                // like a missing bound dependency; a Review quarantine
                // awaiting human assessment instead parks the run rather
                // than retrying it (below); a permanent policy failure is
                // charged like any other labeled retry.
                if let Some(policy) = error.downcast_ref::<PolicyError>() {
                    return if policy.is_transient() {
                        // A Review quarantine
                        // waiting on a human assessment parks the run
                        // instead of joining `mark_transient_retry`'s
                        // hourly backoff -- the condition never resolves on
                        // its own, so retrying it costs a worker pass every
                        // hour for nothing. Every other transient
                        // `PolicyError`, in Review or any other phase,
                        // keeps the ordinary uncharged retry.
                        if policy.label() == PIPELINE_REVIEW_ASSESSMENT_REQUIRED_LABEL
                            && run.next_phase == Some(Phase::Review)
                        {
                            self.mark_awaiting_review_or_record_lease_expired(&run, policy.label())
                                .await
                        } else {
                            self.mark_transient_retry_or_record_lease_expired(&run, policy.label())
                                .await
                        }
                    } else {
                        self.mark_retry_or_record_lease_expired(&run, policy.label())
                            .await
                    };
                }
                // Ruling FR3: a settlement adapter the service does not hold
                // is a deployment gap, not the trace's fault -- the same
                // uncharged suspension as a missing bound dependency. So is
                // a missing per-instrument cap.
                if let Some(gap) = [
                    PIPELINE_SETTLEMENT_ADAPTER_MISSING_LABEL,
                    PIPELINE_SETTLEMENT_CAP_MISSING_LABEL,
                ]
                .into_iter()
                .find(|gap| *gap == label)
                {
                    return self
                        .mark_transient_retry_or_record_lease_expired(&run, gap)
                        .await;
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
                    | "submission_inoperable" => label.as_str(),
                    _ => PIPELINE_OPERATIONAL_ERROR_LABEL,
                };
                self.mark_retry_or_record_lease_expired(&run, retry_label)
                    .await
            }
        }
    }

    /// The phase failed for `error_label`, a reason
    /// `process_claimed_run` charges as a terminal `mark_failed` -- unless
    /// the lease itself has gone stale between the phase's own failure and
    /// this call (the phase ran right up to its lease's edge), in which case
    /// the expiry is recorded instead and `error_label` is never charged.
    /// A Settle run resolves its open legs first (`fail_run`);
    /// a stale lease anywhere in that resolution is recorded the same way.
    async fn mark_failed_or_record_lease_expired(
        &self,
        run: &PipelineRunRecord,
        error_label: &str,
    ) -> anyhow::Result<Option<PipelineRunRecord>> {
        match self.fail_run(run, error_label).await {
            Ok(()) => self
                .store
                .get_run(&run.tenant_id, run.run_id)
                .await
                .map_err(Into::into),
            Err(error) if is_stale_lease_error(&error) => {
                Ok(self.store.record_lease_expired(run).await?)
            }
            Err(error) => Err(error),
        }
    }

    /// The `mark_retry` shape of the same stale-lease-vs-charged-failure
    /// case -- see `mark_failed_or_record_lease_expired` and
    /// `charged_retry`.
    async fn mark_retry_or_record_lease_expired(
        &self,
        run: &PipelineRunRecord,
        error_label: &str,
    ) -> anyhow::Result<Option<PipelineRunRecord>> {
        match self.charged_retry(run, error_label).await {
            Ok(updated) => Ok(Some(updated)),
            Err(error) if is_stale_lease_error(&error) => {
                Ok(self.store.record_lease_expired(run).await?)
            }
            Err(error) => Err(error),
        }
    }

    /// The terminal `mark_failed` failure taken with a worker present. A
    /// Settle run first reconciles its dispatched external legs
    /// (`reconcile_dispatched_settlement_legs`); `mark_failed` then forfeits
    /// every other open leg in the transaction that fails the run.
    async fn fail_run(&self, run: &PipelineRunRecord, error_label: &str) -> anyhow::Result<()> {
        self.reconcile_dispatched_settlement_legs(run).await?;
        self.store.mark_failed(run, error_label).await?;
        Ok(())
    }

    /// P2's charged retry with a worker present. When this retry exhausts
    /// the run's attempts -- `mark_retry` then fails the run as
    /// `attempts_exhausted` -- a Settle run first reconciles its dispatched
    /// external legs, and `mark_retry` forfeits every other
    /// open leg in the transaction that fails the run. The claim holds the
    /// lease, so no other writer moves `attempt_count` under it: the claimed
    /// record predicts the outcome `mark_retry`'s SQL decides.
    async fn charged_retry(
        &self,
        run: &PipelineRunRecord,
        error_label: &str,
    ) -> anyhow::Result<PipelineRunRecord> {
        if run.attempt_count >= run.max_attempts {
            self.reconcile_dispatched_settlement_legs(run).await?;
        }
        Ok(self.store.mark_retry(run, error_label).await?)
    }

    /// Before a worker fails a
    /// Settle run, each open, dispatched, external leg gets one more call to
    /// its idempotent adapter with the request Step 6 made. A receipt that
    /// answers the request completes the leg with its result reference and
    /// external receipt hash. Anything else records `failed` /
    /// `settlement_unreconciled`: a receipt that does not answer the request
    /// or that another leg already recorded, an adapter error, or no call at
    /// all -- for a missing adapter, a missing cap or an amount over it (the
    /// reconciling call never pays what Step 6 would refuse), a selection
    /// without the leg's result, a request that cannot be formed, or a
    /// submission that is no longer operable.
    ///
    /// A leg that is never dispatched again (`settlement_leg_is_open_to_dispatch`)
    /// gets no call: an external leg `failed` as `settlement_result_mismatch`
    /// is already resolved and stays `failed`, and a
    /// `settlement_request_conflict` or `settlement_request_rejected` leg is
    /// forfeited by the transaction that fails the run. (A Trace Credit leg
    /// gets no call either; that transaction forfeits it, a
    /// `settlement_result_mismatch` one included.)
    ///
    /// Each call is fenced by `ensure_live_lease` and each record by the run
    /// lease, so a stale lease here ends as `lease_expired`, never as a
    /// charge. A Trace Credit leg and an undispatched leg are left for the
    /// transaction that fails the run (`resolve_open_settlement_legs_on_tx`),
    /// which forfeits them. Not a Settle run: nothing to do.
    async fn reconcile_dispatched_settlement_legs(
        &self,
        run: &PipelineRunRecord,
    ) -> anyhow::Result<()> {
        if run.next_phase != Some(Phase::Settle) {
            return Ok(());
        }
        let dispatched = self
            .store
            .list_settlements(&run.tenant_id, run.run_id)
            .await?
            .into_iter()
            .filter(|settlement| {
                settlement.dispatched_at.is_some()
                    && settlement.instrument_id != InstrumentId::trace_credit().as_str()
                    && settlement_leg_is_unresolved(
                        &settlement.instrument_id,
                        &settlement.operation_state,
                        settlement.last_error_label.as_deref(),
                    )
                    && settlement_leg_is_open_to_dispatch(
                        &settlement.operation_state,
                        settlement.last_error_label.as_deref(),
                    )
            })
            .collect::<Vec<_>>();
        if dispatched.is_empty() {
            return Ok(());
        }
        let operable = self.submission_guard(run).await?.operable;
        let expected_results = match self.store.load_settle_selection(run).await {
            Ok(selection) => selection
                .and_then(|selection| {
                    serde_json::from_value::<SettleDecision>(selection.decision).ok()
                })
                .map(|decision| {
                    decision
                        .settlement_operations()
                        .iter()
                        .filter_map(|operation| {
                            operation.result_ref_hash().map(|result_ref_hash| {
                                (
                                    operation.instrument_id().as_str().to_string(),
                                    result_ref_hash.to_string(),
                                )
                            })
                        })
                        .collect::<BTreeMap<_, _>>()
                })
                .unwrap_or_default(),
            // A selection that no longer decodes cannot form the request:
            // no reconciling call, so the leg is `settlement_unreconciled`.
            Err(DatabaseError::Serialization(_)) => BTreeMap::new(),
            Err(error) => return Err(error.into()),
        };
        for settlement in dispatched {
            let reconciled = if operable {
                self.reconciling_settle_call(run, &settlement, &expected_results)
                    .await?
            } else {
                None
            };
            match self
                .store
                .record_settlement_reconciliation(
                    run,
                    &settlement.instrument_id,
                    reconciled.as_ref(),
                )
                .await
            {
                Ok(()) => {}
                // One external receipt answers one leg: a receipt another
                // leg already recorded does not reconcile this one.
                Err(DatabaseError::Postgres(error))
                    if reconciled.is_some() && is_external_receipt_reuse(&error) =>
                {
                    self.store
                        .record_settlement_reconciliation(run, &settlement.instrument_id, None)
                        .await?;
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    /// The one reconciling adapter call for `settlement`, or `None` when it
    /// is not made or its receipt does not answer the request (see
    /// `reconcile_dispatched_settlement_legs`).
    async fn reconciling_settle_call(
        &self,
        run: &PipelineRunRecord,
        settlement: &PipelineSettlementRecord,
        expected_results: &BTreeMap<String, String>,
    ) -> anyhow::Result<Option<SettlementReceipt>> {
        let Ok(instrument_id) = InstrumentId::new(settlement.instrument_id.clone()) else {
            return Ok(None);
        };
        let Some(adapter) = self.settlement_adapters.get(&instrument_id) else {
            return Ok(None);
        };
        let Some(expected_result_ref_hash) = expected_results.get(instrument_id.as_str()) else {
            return Ok(None);
        };
        let within_cap = self
            .caps
            .per_instrument_atomic_units
            .get(instrument_id.as_str())
            .is_some_and(|cap| settlement.atomic_units <= *cap);
        if !within_cap {
            return Ok(None);
        }
        let Ok(request) = SettlementRequest::new(
            pipeline_tenant_storage_ref(&run.tenant_id),
            run.run_id,
            instrument_id,
            settlement.atomic_units,
            settlement.operation_ref_hash.clone(),
            expected_result_ref_hash.clone(),
        ) else {
            return Ok(None);
        };
        self.ensure_live_lease(run).await?;
        Ok(match adapter.settle(&request).await {
            Ok(receipt) if receipt.answers(&request) => Some(receipt),
            Ok(_) | Err(_) => None,
        })
    }

    /// The `mark_transient_retry` shape of the same
    /// stale-lease-vs-charged-failure case -- see
    /// `mark_failed_or_record_lease_expired`.
    async fn mark_transient_retry_or_record_lease_expired(
        &self,
        run: &PipelineRunRecord,
        error_label: &str,
    ) -> anyhow::Result<Option<PipelineRunRecord>> {
        match self.store.mark_transient_retry(run, error_label).await {
            Ok(updated) => Ok(Some(updated)),
            Err(error) if is_stale_lease_db_error(&error) => {
                Ok(self.store.record_lease_expired(run).await?)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// The `mark_awaiting_review` shape of the same
    /// stale-lease-vs-charged-failure case -- see
    /// `mark_failed_or_record_lease_expired`.
    async fn mark_awaiting_review_or_record_lease_expired(
        &self,
        run: &PipelineRunRecord,
        error_label: &str,
    ) -> anyhow::Result<Option<PipelineRunRecord>> {
        match self.store.mark_awaiting_review(run, error_label).await {
            Ok(updated) => Ok(Some(updated)),
            Err(error) if is_stale_lease_db_error(&error) => {
                Ok(self.store.record_lease_expired(run).await?)
            }
            Err(error) => Err(error.into()),
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
                // Ruling T3-5: loads whatever assessment a reviewer has
                // already recorded for this run, so `MinimalReviewPolicy`
                // (and any other Review policy) can resolve a quarantine
                // once `record_review_assessment` has moved it back to
                // `pending`. `None` for a run that has never been
                // quarantined, or one still waiting on a human assessment.
                let human_assessment = self
                    .store
                    .load_review_assessment(&run.tenant_id, run.run_id)
                    .await?;
                let output = bundle
                    .review
                    .execute(&ReviewInput {
                        run_id: run.run_id,
                        tenant_storage_ref: pipeline_tenant_storage_ref(&run.tenant_id),
                        trace_id: run.trace_id,
                        source_content_hash: source_content_hash.clone(),
                        source_artifact,
                        admission,
                        human_assessment,
                    })
                    .await?;
                let (result, content) = output.into_parts();
                let mut written_receipt = None;
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
                        let object_id = pipeline_attempt_object_id(
                            "approved",
                            run.run_id,
                            required_lease_token(run)?,
                        );
                        let wrapper = encode_pipeline_artifact_bytes(content.bytes())?;
                        let receipt = self.artifact_store.put_serialized_json(
                            pipeline_tenant_storage_ref(&run.tenant_id).as_str(),
                            TraceArtifactKind::ContributionEnvelope,
                            &object_id,
                            &wrapper,
                        )?;
                        self.inject_crash(PipelineCrashPoint::AfterReviewArtifactStorage)?;
                        written_receipt = Some(receipt.clone());
                        Some(ApprovedRevision {
                            revision_id: *registry_revision_id,
                            object_ref: approved_object_ref(
                                run,
                                &receipt,
                                content.bytes().len(),
                                &self.object_store_name,
                            ),
                            content_hash: content.content_hash().to_string(),
                            source_content_hash,
                            worker_identity: content.worker_identity().to_string(),
                        })
                    }
                    (ReviewDecision::Rejected { .. }, None) => None,
                    _ => anyhow::bail!("review_output_invalid"),
                };
                let commit_result = self
                    .store
                    .commit_review(
                        run,
                        StoredPhaseResult::from_result(Phase::Review, &result)?,
                        approved,
                    )
                    .await;
                let updated = match commit_result {
                    Ok(updated) => updated,
                    // The submission became inoperable (a
                    // withdrawal, expiry, or purge) in the window between
                    // this attempt's artifact write and the commit above.
                    // The store's `Display` prefixes every `Constraint`
                    // error ("Constraint violation: ..."), which would not
                    // match the safe-label allowlist verbatim (the same
                    // reason `commit_score_phase` re-raises
                    // `settlement_adapter_missing` bare); re-raise this one
                    // the same way. Delete the approved object this attempt
                    // wrote -- best effort, and never let a failed delete
                    // mask the real refusal.
                    Err(DatabaseError::Constraint(ref label))
                        if label == PIPELINE_SUBMISSION_INOPERABLE_LABEL =>
                    {
                        if let Some(receipt) = written_receipt.as_ref() {
                            if self
                                .artifact_store
                                .delete_artifact(
                                    pipeline_tenant_storage_ref(&run.tenant_id).as_str(),
                                    receipt,
                                )
                                .is_err()
                            {
                                tracing::warn!(
                                    label = "review_approved_object_delete_failed",
                                    "best-effort delete of an approved object failed after \
                                     commit_review refused an inoperable submission"
                                );
                            }
                        }
                        return Err(anyhow::anyhow!(PIPELINE_SUBMISSION_INOPERABLE_LABEL));
                    }
                    Err(error) => return Err(error.into()),
                };
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
        // Before any artifact is stored, the decision must be bound to the
        // run's bundle: it records the bound manifest's bundle identifier,
        // and every award names an instrument that manifest pins. A policy
        // can build a decision under a manifest of its own making, so the
        // pin check alone does not bind the decision to the run.
        result
            .decision
            .require_bound(&bundle.package.manifest)
            .map_err(|_| anyhow::anyhow!("score_outcome_invalid"))?;
        // M4: a Trace Credit award must fit the ledger's signed 64-bit
        // microcredit column; refuse one that does not here, before any
        // artifact is stored, rather than at the database CHECK.
        ensure_trace_credit_awards_fit_the_ledger(result.decision.awards())?;
        let lease_token = required_lease_token(run)?;
        let command_bytes = match &command {
            None => None,
            Some(command) => {
                anyhow::ensure!(
                    command.revision_id() == revision_id,
                    "index_command_invalid"
                );
                Some((serde_json::to_vec(command)?, command.content_hash()?))
            }
        };
        // Each stored object is `(artifact, stored ref, hash, object ref,
        // receipt)`. A write that fails leaves the objects stored before it;
        // those are not tracked, like the objects of an attempt that crashes
        // before its commit.
        let mut written = Vec::new();
        for (artifact, bytes, hash) in [
            command_bytes
                .as_ref()
                .map(|(bytes, hash)| ("index-command", bytes, hash.clone())),
            neighbor
                .as_ref()
                .map(|bytes| ("score-neighbors", bytes, sha256_prefixed(bytes))),
        ]
        .into_iter()
        .flatten()
        {
            let wrapper = encode_pipeline_artifact_bytes(bytes)?;
            let receipt = self.artifact_store.put_serialized_json(
                tenant.as_str(),
                TraceArtifactKind::VectorPayload,
                &pipeline_attempt_object_id(artifact, run.run_id, lease_token),
                &wrapper,
            )?;
            let object_ref = score_object_ref(
                run,
                artifact,
                &receipt,
                bytes.len(),
                &self.object_store_name,
            );
            written.push((
                artifact,
                format!("{}#{}", receipt.object_key, receipt.ciphertext_sha256),
                hash,
                object_ref,
                receipt,
            ));
        }
        let stored = |name: &str| {
            written.iter().find(|(artifact, ..)| *artifact == name).map(
                |(_, stored_ref, hash, object_ref, _)| {
                    (stored_ref.as_str(), hash.as_str(), object_ref)
                },
            )
        };
        self.inject_crash(PipelineCrashPoint::AfterScoreArtifactStorage)?;
        let commit_result = self
            .store
            .commit_score(
                run,
                StoredPhaseResult::from_result(Phase::Score, &result)?,
                result.decision.awards(),
                stored("index-command"),
                stored("score-neighbors"),
                &self.settlement_adapters.payout_rails(),
                self.payout_enabled(),
                bundle.trace_credit_event,
            )
            .await;
        let updated = match commit_result {
            Ok(updated) => updated,
            // The store's Display prefixes every Constraint error
            // ("Constraint violation: ..."), which would not match decision
            // P2's fixed allowlist verbatim; re-raise the labels the
            // allowlist expects as bare anyhow errors.
            Err(DatabaseError::Constraint(ref label))
                if label == PIPELINE_SETTLEMENT_ADAPTER_MISSING_LABEL =>
            {
                return Err(anyhow::anyhow!(PIPELINE_SETTLEMENT_ADAPTER_MISSING_LABEL));
            }
            // The submission became inoperable (a withdrawal, expiry, or
            // purge) between this attempt's read and its commit, so nothing
            // committed and no object ref names these objects. Delete every
            // object this attempt wrote, as Review deletes its approved
            // object: best effort, and a failed delete never masks the
            // refusal.
            Err(DatabaseError::Constraint(ref label))
                if label == PIPELINE_SUBMISSION_INOPERABLE_LABEL =>
            {
                for (.., receipt) in &written {
                    if self
                        .artifact_store
                        .delete_artifact(tenant.as_str(), receipt)
                        .is_err()
                    {
                        tracing::warn!(
                            label = "score_object_delete_failed",
                            "best-effort delete of a Score object failed after commit_score \
                             refused an inoperable submission"
                        );
                    }
                }
                return Err(anyhow::anyhow!(PIPELINE_SUBMISSION_INOPERABLE_LABEL));
            }
            Err(error) => return Err(error.into()),
        };
        self.inject_crash(PipelineCrashPoint::AfterScoreCommit)?;
        Ok(updated)
    }

    /// Settle's first half (brief 3B/3C, port 4547 to 4723 under the #971
    /// settlement shape): validates the committed Score outcome and its
    /// seeded settlement operations, persists the Settle selection before
    /// any external effect, and applies the stored index
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
        // One query for the Score phase's own row, not a list of every
        // outcome the run has: the other phases' outcomes are never read
        // here.
        let score_outcome = self
            .store
            .outcome_for_phase(&run.tenant_id, run.run_id, Phase::Score)
            .await?
            .ok_or_else(|| anyhow::anyhow!("score_outcome_invalid"))?;
        // The stored decision loads unverified and becomes a
        // `ScoreDecision` only against the run's bound manifest: a decision
        // under another bundle, or an award for an instrument that manifest
        // does not pin, fails closed before anything settles.
        let score_decision =
            serde_json::from_value::<UnverifiedScoreDecision>(score_outcome.decision)
                .map_err(|_| anyhow::anyhow!("score_outcome_invalid"))?
                .verify(&bundle.package.manifest)
                .map_err(|_| anyhow::anyhow!("score_outcome_invalid"))?;
        let score_evidence = serde_json::from_value::<ScoreEvidence>(score_outcome.evidence)
            .map_err(|_| anyhow::anyhow!("score_outcome_invalid"))?;
        let score_evaluation = serde_json::from_value::<ScoreEvaluation>(score_outcome.evaluation)
            .map_err(|_| anyhow::anyhow!("score_outcome_invalid"))?;
        anyhow::ensure!(
            *score_decision.awards() == score_evidence.fixed_awards
                && *score_decision.awards() == score_evaluation.awards,
            "score_outcome_invalid"
        );

        // Step 2 (brief 3C, `ensure_operations_match_committed_awards`):
        // the settlement rows Score seeded (decision D5) must still be
        // exactly one per award. Listed once per pass: only the worker
        // holding the run's lease changes these rows, and nothing between
        // here and Step 6 below writes to them (Step 4 persists the
        // selection on `pipeline_runs`, not the settlement rows; Step 5
        // touches only the index and `index_write_state`), so the list read
        // here is still current when Step 6 uses it.
        let settlements = self
            .store
            .list_settlements(&run.tenant_id, run.run_id)
            .await?;
        ensure_operations_match_committed_awards(
            run.run_id,
            score_decision.awards(),
            &settlements,
        )?;

        // Step 4: reuse a persisted selection on retry; otherwise run the
        // Settle policy once and persist its selection before any external
        // effect.
        let selection = match self.store.load_settle_selection(&run).await? {
            Some(selection) => selection,
            None => {
                // Step 3: the submission-operability guard, read fresh,
                // directly before the index-membership decision below that
                // this branch persists. A retry that already
                // has a persisted selection never reaches this branch, so
                // it never pays for this read only to see it replaced
                // before use at Step 5 or Step 6 below.
                //
                // The stored command is read first. A withdrawal's queued
                // follow-up deletes the objects Score stored (they are the
                // submission's object refs), and it can do so before this
                // runs; the deletion is queued only once the submission is
                // no longer operable, so a guard read after a failed load
                // sees that. An inoperable submission is excluded from the
                // index whatever the policy decides, so it settles without
                // the command.
                let index_command = self.load_index_command(&run, &score_evidence).await;
                let guard = self.submission_guard(&run).await?;
                let index_command = match index_command {
                    Ok(command) => command,
                    Err(_) if !guard.operable => None,
                    Err(error) => return Err(error),
                };
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
                        .awards()
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
        // pending (an include whose entries are not yet all applied). The
        // guard is read here too, rather than trusting whatever Step 3 saw,
        // since a withdrawal can land in the gap between them. It is also
        // held (brief section 2): one transaction reads the guard, keeps the
        // submission row locked `FOR SHARE` through every upsert, and
        // commits the `index_write_state` it records. A withdrawal locks
        // that row `FOR UPDATE`, so it either commits before the guard read
        // (the dispatch then sees it, cancels, and queues an invalidation
        // for whatever an earlier attempt wrote) or waits until the write
        // and its `complete` have committed.
        if run.index_write_state == "pending" {
            // Both reads below come before the dispatch transaction opens:
            // `ensure_live_lease` checks out a pooled connection of its own,
            // and the dispatch never holds two at once (with a pool of one it
            // would wait for itself). The stored command's error is raised
            // only once the guard lets the write go ahead: a cancelled
            // dispatch does not need the command.
            self.ensure_live_lease(&run).await?;
            let command = self.load_index_command(&run, &score_evidence).await;
            let mut client = self.backend.trace_pool().get().await?;
            let tx = PgPipelineStore::tenant_transaction(&mut client, &run.tenant_id).await?;
            // Lock order: the run row (`ensure_current_lease`), then the
            // submission row (the guard).
            ensure_current_lease(&tx, &run, required_lease_token(&run)?).await?;
            let guard = PgPipelineStore::submission_guard_on_tx(&tx, &run).await?;
            if !guard.operable {
                // `pending` may be partly written: an earlier attempt can
                // have applied entries and then rolled back. So the cancel
                // queues an invalidation of the revision in the same
                // transaction (a no-op for the index if nothing was written).
                PgPipelineStore::enqueue_index_invalidation_on_tx(
                    &tx,
                    &run,
                    PIPELINE_SUBMISSION_INOPERABLE_LABEL,
                )
                .await?;
                run = PgPipelineStore::set_index_write_state_on_tx(&tx, &run, "cancelled").await?;
            } else {
                let command = command?.ok_or_else(|| anyhow::anyhow!("index_command_invalid"))?;
                let tenant = pipeline_tenant_storage_ref(&run.tenant_id);
                // Each entry is written under its own key: the command pairs
                // them, so one entry is never stored under another's key.
                for (key, entry) in command.keyed_entries(&tenant) {
                    match self
                        .index_writer
                        .upsert(&key, &entry.embedding, &entry.content_hash)
                    {
                        Ok(_) => {}
                        Err(IndexWriteError::Uncertain) | Err(IndexWriteError::Failed) => {
                            // An index outage is a
                            // dependency failure, like Score's own
                            // `index_unavailable` -- an uncharged
                            // suspension, which the Settle code records
                            // itself rather than returning an `Err`. Ruling
                            // S5: the transaction (rolled back, `pending`
                            // stays) and its pooled client go before the
                            // store call, which checks out its own.
                            drop(tx);
                            drop(client);
                            return Ok(self
                                .store
                                .mark_transient_retry(&run, PIPELINE_INDEX_UNAVAILABLE_LABEL)
                                .await?);
                        }
                        Err(IndexWriteError::ContentConflict) => {
                            // Ruling S5, as above.
                            drop(tx);
                            drop(client);
                            self.store.mark_index_write_state(&run, "failed").await?;
                            return Err(anyhow::anyhow!(PIPELINE_INDEX_CONFLICT_LABEL));
                        }
                    }
                }
                // A crash here drops the transaction, so `pending` stays; the
                // retry applies the same entries again, which the index
                // reports as unchanged.
                self.inject_crash(PipelineCrashPoint::AfterIndexApply)?;
                run = PgPipelineStore::set_index_write_state_on_tx(&tx, &run, "complete").await?;
            }
            tx.commit().await?;
        }

        // Step 6: each settlement row Score seeded settles as an independent
        // leg, with no atomicity across instruments: no leg waits for or
        // reverses another leg, and a retry never repeats a leg that already
        // reached `complete`. The guard is read again immediately before any
        // adapter call, for the same reason Step 5 reads it before its own
        // external effect: a withdrawal can land after Step 3's read (when
        // this pass took it -- it does not run on the persisted-selection
        // branch) or Step 5's, when that ran. The settlement rows themselves
        // are not listed again: Step 2's list is still current, per the note
        // there.
        let mut guard = self.submission_guard(&run).await?;
        if !guard.operable {
            // The submission stopped being operable: every leg that has not
            // already completed is forfeited without calling its adapter.
            // Credit that already settled stays settled (skipped below);
            // credit that did not is forfeited, the same as every other
            // instrument -- there is no special case for Trace Credit here.
            //
            // Any leg of an external instrument whose `dispatched_at` is set
            // may have taken effect when it is forfeited here, whatever its
            // state or label: its adapter was called at least once. That
            // includes a `settlement_unreconciled` leg of a live run and a leg
            // dispatched earlier and later refused by a lowered cap. Such a
            // leg is flagged `settlement_unreconciled` here instead of
            // `submission_inoperable`, for an operator to reconcile by hand
            // against the adapter's own records -- this forfeit does not make
            // that reconciling call itself, and the leg keeps its
            // `dispatched_at`. The exception is a leg whose own label already
            // says no effect happened (`Conflict`/`Rejected`): it keeps
            // `submission_inoperable`. An undispatched leg had no effect, and
            // a Trace Credit leg that is not `complete` paid nothing: it pays
            // only through the ledger row that commits with its completion.
            // `withdrawal_forfeit_label` is the one rule both this branch and
            // the in-pass branch below share.
            for settlement in &settlements {
                if settlement.operation_state != "complete" {
                    self.store
                        .update_settlement(
                            &run,
                            &settlement.instrument_id,
                            withdrawal_forfeited_settlement_update(settlement),
                        )
                        .await?;
                }
            }
        } else {
            // The expected result comes from the persisted selection,
            // not from the row -- the row's own `result_ref_hash` column
            // holds `NULL` until the leg actually completes.
            let selection_decision =
                serde_json::from_value::<SettleDecision>(selection.decision.clone())
                    .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
            let tenant_storage_ref = pipeline_tenant_storage_ref(&run.tenant_id);
            // Why legs this attempt leaves open are waiting: a hold or an
            // `Unavailable` answer (both uncharged suspensions), or a charged
            // blocker -- a leg that fails closed in this pass (a receipt that
            // does not answer its request, a `Conflict` or `Rejected` answer,
            // an amount over the cap, a request that cannot be formed) or a
            // leg an earlier pass failed closed for good.
            let mut held = false;
            let mut adapter_unavailable = false;
            let mut settlement_blocked = false;
            for settlement in settlements.clone() {
                if matches!(
                    settlement.operation_state.as_str(),
                    "complete" | "forfeited"
                ) {
                    continue;
                }
                let instrument_id = InstrumentId::new(settlement.instrument_id.clone())
                    .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
                // The Trace Credit leg's own ledger transaction
                // can discover, partway through this pass, that the
                // submission stopped being operable (a withdrawal landed
                // after this pass's guard read but before that transaction).
                // From that point on this pass treats every remaining
                // non-terminal leg the way Step 6's top-level inoperable
                // branch treats a submission already known inoperable:
                // forfeited without calling its adapter, including a leg that
                // was dispatched before and whose effect may have happened
                // (see that branch, and `withdrawal_forfeit_label`, the same
                // rule this branch shares with it). A leg that already
                // completed earlier in this pass is untouched: no leg
                // reverses another.
                if !guard.operable {
                    self.store
                        .update_settlement(
                            &run,
                            instrument_id.as_str(),
                            withdrawal_forfeited_settlement_update(&settlement),
                        )
                        .await?;
                    continue;
                }
                // A leg an earlier pass failed closed for good is never
                // dispatched again. While one exists, the run's retry stays
                // charged, so the other legs still progress, the attempts run
                // out, and the failure path resolves it.
                if !settlement_leg_is_open_to_dispatch(
                    &settlement.operation_state,
                    settlement.last_error_label.as_deref(),
                ) {
                    settlement_blocked = true;
                    continue;
                }
                let adapter = self
                    .settlement_adapters
                    .get(&instrument_id)
                    .ok_or_else(|| anyhow::anyhow!(PIPELINE_SETTLEMENT_ADAPTER_MISSING_LABEL))?;
                // No configured cap for the
                // instrument is a configuration gap, an uncharged suspension
                // (`process_claimed_run`), raised before the leg's adapter
                // is called. An amount over a configured cap is a spend
                // limit that refuses the payment: the leg fails below and
                // the retry stays charged.
                let cap = self
                    .caps
                    .per_instrument_atomic_units
                    .get(instrument_id.as_str())
                    .copied()
                    .ok_or_else(|| anyhow::anyhow!(PIPELINE_SETTLEMENT_CAP_MISSING_LABEL))?;
                if settlement.atomic_units > cap {
                    self.store
                        .update_settlement(
                            &run,
                            instrument_id.as_str(),
                            failed_settlement_update(PIPELINE_CREDIT_CAP_LABEL),
                        )
                        .await?;
                    settlement_blocked = true;
                    continue;
                }
                let expected_result_ref_hash = selection_decision
                    .settlement_operations()
                    .iter()
                    .find(|operation| operation.instrument_id() == &instrument_id)
                    .and_then(|operation| operation.result_ref_hash())
                    .map(str::to_string)
                    .ok_or_else(|| anyhow::anyhow!("settlement result reference is missing"))?;
                // A request that `SettlementRequest::new` refuses cannot be
                // sent: the leg fails closed before any adapter call, and the
                // retry is charged.
                let request = match SettlementRequest::new(
                    tenant_storage_ref.clone(),
                    run.run_id,
                    instrument_id.clone(),
                    settlement.atomic_units,
                    settlement.operation_ref_hash.clone(),
                    expected_result_ref_hash,
                ) {
                    Ok(request) => request,
                    Err(_) => {
                        self.store
                            .update_settlement(
                                &run,
                                instrument_id.as_str(),
                                failed_settlement_update(PIPELINE_SETTLEMENT_REQUEST_INVALID_LABEL),
                            )
                            .await?;
                        settlement_blocked = true;
                        continue;
                    }
                };
                // A held Trace Credit account is checked before any external
                // effect, so a held account never reaches its adapter.
                let credit_account_ref = if instrument_id == InstrumentId::trace_credit() {
                    let account_ref = self.credit_account_ref(&run).await?;
                    if self
                        .credit_account_is_held(&run.tenant_id, &account_ref)
                        .await?
                    {
                        self.store
                            .update_settlement(
                                &run,
                                instrument_id.as_str(),
                                held_settlement_update(),
                            )
                            .await?;
                        held = true;
                        continue;
                    }
                    Some(account_ref)
                } else {
                    None
                };
                // Every adapter call is fenced by the lease this attempt
                // still holds, the same check Step 5 runs before the index
                // write.
                self.ensure_live_lease(&run).await?;
                // The leg is `leased` under this attempt's lease
                // while its adapter call is in flight, and `dispatched_at`
                // records that it was dispatched -- a failed run reconciles
                // such a leg against its adapter instead of forfeiting it.
                self.store
                    .mark_settlement_leased(&run, instrument_id.as_str())
                    .await?;
                let receipt = match adapter.settle(&request).await {
                    // The receipt must answer the request: its result
                    // reference is the one the persisted selection recorded.
                    Ok(receipt) if receipt.answers(&request) => receipt,
                    // Any other receipt fails the leg closed, and the leg is
                    // never dispatched again.
                    Ok(_) => {
                        self.store
                            .update_settlement(
                                &run,
                                instrument_id.as_str(),
                                failed_settlement_update(PIPELINE_SETTLEMENT_RESULT_MISMATCH_LABEL),
                            )
                            .await?;
                        settlement_blocked = true;
                        continue;
                    }
                    // The effect may or may not have happened. The leg waits
                    // in `retry`, uncharged, and a later attempt sends the
                    // same request; the adapter's idempotency contract makes
                    // that safe.
                    Err(SettlementError::Unavailable) => {
                        self.store
                            .update_settlement(
                                &run,
                                instrument_id.as_str(),
                                SettlementUpdate {
                                    operation_state: "retry",
                                    result_ref_hash: None,
                                    external_receipt_hash: None,
                                    credit_event_id: None,
                                    settlement_batch_id: None,
                                    payout_state: None,
                                    error_label: Some(SettlementError::Unavailable.label()),
                                },
                            )
                            .await?;
                        adapter_unavailable = true;
                        continue;
                    }
                    // No effect happened, and the operation must not be sent
                    // again: the leg fails under the adapter's own label and
                    // is never dispatched again.
                    Err(error @ (SettlementError::Conflict | SettlementError::Rejected)) => {
                        self.store
                            .update_settlement(
                                &run,
                                instrument_id.as_str(),
                                failed_settlement_update(error.label()),
                            )
                            .await?;
                        settlement_blocked = true;
                        continue;
                    }
                };
                match credit_account_ref {
                    // The ledger row, the finalized batch, and the completed
                    // settlement row commit in one transaction.
                    Some(account_ref) => match self
                        .settle_internal_credit(
                            &run,
                            &settlement,
                            score_outcome.outcome_id,
                            &account_ref,
                            &receipt,
                            bundle.trace_credit_event,
                        )
                        .await
                    {
                        Ok(InternalCreditResult::Complete) => {}
                        Ok(InternalCreditResult::Held) => {
                            self.store
                                .update_settlement(
                                    &run,
                                    instrument_id.as_str(),
                                    held_settlement_update(),
                                )
                                .await?;
                            held = true;
                            continue;
                        }
                        // The ledger transaction's own
                        // submission re-check found the submission
                        // inoperable (a withdrawal landed during this leg's
                        // adapter call, between Step 6's guard read and this
                        // transaction). Forfeit this leg the same way the
                        // top-level inoperable branch does -- no result, no
                        // event, no batch -- and treat the submission as
                        // inoperable for the rest of this pass, so every
                        // remaining non-terminal leg is forfeited too. When
                        // no earlier leg in this same pass blocked, the
                        // committed Settle outcome records
                        // `submission_inoperable`; when one did, this
                        // attempt retries instead, and the next pass's guard
                        // read forfeits every leg before any adapter call.
                        Ok(InternalCreditResult::Inoperable) => {
                            self.store
                                .update_settlement(
                                    &run,
                                    instrument_id.as_str(),
                                    forfeited_settlement_update(
                                        PIPELINE_SUBMISSION_INOPERABLE_LABEL,
                                    ),
                                )
                                .await?;
                            guard.operable = false;
                            continue;
                        }
                        // One external receipt answers one leg: a receipt
                        // another leg already recorded does not answer this
                        // one. The credit transaction rolled back, so no
                        // ledger row was written.
                        Err(error) if is_external_receipt_reuse_error(&error) => {
                            self.store
                                .update_settlement(
                                    &run,
                                    instrument_id.as_str(),
                                    failed_settlement_update(
                                        PIPELINE_SETTLEMENT_RESULT_MISMATCH_LABEL,
                                    ),
                                )
                                .await?;
                            settlement_blocked = true;
                            continue;
                        }
                        Err(error) => return Err(error),
                    },
                    None => {
                        let completed = self
                            .store
                            .update_settlement(
                                &run,
                                instrument_id.as_str(),
                                SettlementUpdate {
                                    operation_state: "complete",
                                    result_ref_hash: Some(receipt.result_ref_hash()),
                                    external_receipt_hash: receipt.external_receipt_hash(),
                                    credit_event_id: None,
                                    settlement_batch_id: None,
                                    payout_state: None,
                                    error_label: None,
                                },
                            )
                            .await;
                        match completed {
                            Ok(_) => {}
                            // One external receipt answers one leg: a receipt
                            // another leg already recorded does not answer
                            // this one.
                            Err(DatabaseError::Postgres(error))
                                if is_external_receipt_reuse(&error) =>
                            {
                                self.store
                                    .update_settlement(
                                        &run,
                                        instrument_id.as_str(),
                                        failed_settlement_update(
                                            PIPELINE_SETTLEMENT_RESULT_MISMATCH_LABEL,
                                        ),
                                    )
                                    .await?;
                                settlement_blocked = true;
                                continue;
                            }
                            Err(error) => return Err(error.into()),
                        }
                    }
                }
                self.inject_crash(PipelineCrashPoint::AfterInstrumentOperation)?;
            }
            // Every non-terminal row this attempt leaves behind is retried
            // together; the Settle code records the retry itself and never
            // returns `Err` for this case. A charged blocker fails closed and
            // is charged, whatever else happened; otherwise a hold or an
            // `Unavailable` answer is an uncharged suspension, labeled by the
            // hold when there is one. The charged retry that exhausts the
            // attempts resolves every open leg before the run fails
            // (`charged_retry`).
            if settlement_blocked {
                return self
                    .charged_retry(&run, PIPELINE_SETTLEMENT_RETRY_LABEL)
                    .await;
            }
            if held || adapter_unavailable {
                let label = if held {
                    PIPELINE_CREDIT_HELD_LABEL
                } else {
                    PIPELINE_SETTLEMENT_RETRY_LABEL
                };
                return Ok(self.store.mark_transient_retry(&run, label).await?);
            }
        }

        self.commit_settle_from_progress(&run, &selection, guard, &score_decision)
            .await
    }

    /// Commits the Settle outcome once every settlement row is terminal
    /// (`complete` or `forfeited`) -- Step 6 above guarantees that by the
    /// time this runs, either through completion or through the guard's
    /// forfeiture pass. Builds the final decision from what actually
    /// happened to each leg (amendments-971 A9), never from the selection's
    /// own proposed operations. `score_decision` is the same value Step 1 of
    /// `complete_settle_phase` already decoded; this does not re-read it.
    async fn commit_settle_from_progress(
        &self,
        run: &PipelineRunRecord,
        selection: &StoredPhaseResult,
        guard: SubmissionGuard,
        score_decision: &ScoreDecision,
    ) -> anyhow::Result<PipelineRunRecord> {
        let stored_decision = serde_json::from_value::<SettleDecision>(selection.decision.clone())
            .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
        // The committed decision records what actually happened, not what
        // the policy raw-selected: the selection persists `Include` even
        // when the guard was already inoperable at persist time (only the
        // DB `index_membership` column was corrected then), and the guard
        // can newly fail between persist and dispatch, leaving
        // `index_write_state = 'cancelled'` with `index_membership` still
        // `'included'`. Task 13's rule (applies here per the controller
        // ruling): the selection's `Include` only if `run.index_membership
        // == "included"` and `run.index_write_state == "complete"`; else
        // `Exclude { reason: submission_inoperable }` when the guard is
        // inoperable now; else the selection's own `Exclude` (a plain
        // policy exclusion, unrelated to the guard).
        let observed_include =
            run.index_membership == "included" && run.index_write_state == "complete";
        let final_index_membership = if observed_include {
            stored_decision.index_membership.clone()
        } else if !guard.operable {
            IndexMembershipDecision::Exclude {
                reason: ReasonCode::new(PIPELINE_SUBMISSION_INOPERABLE_LABEL)?,
            }
        } else {
            match &stored_decision.index_membership {
                IndexMembershipDecision::Exclude { reason } => IndexMembershipDecision::Exclude {
                    reason: reason.clone(),
                },
                // Unreachable given the state machine above (an operable
                // guard with `observed_include` false and a selection that
                // still says `Include` would mean dispatch neither
                // completed, retried, nor cancelled) -- fail closed rather
                // than commit an unproven `Include`.
                IndexMembershipDecision::Include { .. } => IndexMembershipDecision::Exclude {
                    reason: ReasonCode::new(PIPELINE_SUBMISSION_INOPERABLE_LABEL)?,
                },
            }
        };
        // Every row is terminal here (Step 6's contract): `complete` becomes
        // a `Completed` operation carrying the adapter's actual result;
        // `forfeited` becomes a `Forfeited` operation under the same safe
        // label the row itself was written with. `list_settlements` orders
        // by `instrument_id`, so this is already instrument order;
        // `SettleDecision::from_parts` re-sorts regardless.
        //
        // Ruling F-I1: a `trace_credit` row `complete` with no credit event
        // is a leg one of `main`'s NoveltyUtility checks withheld (the
        // withheld branch of `settle_internal_credit`, the only writer of
        // such a row). No credit was issued, so the outcome records it as
        // `Forfeited` under the row's own withheld label, not as a completed
        // settlement of the award. A row of that shape with no label cannot
        // be told apart from a lost event, and fails closed.
        let settlements = self
            .store
            .list_settlements(&run.tenant_id, run.run_id)
            .await?;
        let operations = settlements
            .iter()
            .map(|settlement| -> anyhow::Result<InstrumentSettlement> {
                let instrument_id = InstrumentId::new(settlement.instrument_id.clone())
                    .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
                match settlement.operation_state.as_str() {
                    "complete"
                        if settlement.instrument_id == InstrumentId::trace_credit().as_str()
                            && settlement.credit_event_id.is_none() =>
                    {
                        let withheld_label = settlement
                            .last_error_label
                            .clone()
                            .ok_or_else(|| anyhow::anyhow!("settlement_operation_mismatch"))?;
                        InstrumentSettlement::forfeited(
                            instrument_id,
                            settlement.atomic_units,
                            settlement.operation_ref_hash.clone(),
                            ReasonCode::new(withheld_label)
                                .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?,
                        )
                        .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))
                    }
                    "complete" => {
                        let result_ref_hash = settlement
                            .result_ref_hash
                            .clone()
                            .ok_or_else(|| anyhow::anyhow!("settlement_operation_mismatch"))?;
                        // The committed operation carries the receipt the leg
                        // recorded, its external receipt hash included.
                        let receipt = match settlement.external_receipt_hash.clone() {
                            Some(external_receipt_hash) => {
                                SettlementReceipt::external(result_ref_hash, external_receipt_hash)
                            }
                            None => SettlementReceipt::internal(result_ref_hash),
                        }
                        .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
                        InstrumentSettlement::completed(
                            instrument_id,
                            settlement.atomic_units,
                            settlement.operation_ref_hash.clone(),
                            &receipt,
                        )
                        .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))
                    }
                    "forfeited" => InstrumentSettlement::forfeited(
                        instrument_id,
                        settlement.atomic_units,
                        settlement.operation_ref_hash.clone(),
                        ReasonCode::new(PIPELINE_SUBMISSION_INOPERABLE_LABEL)
                            .expect("static safe label"),
                    )
                    .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch")),
                    // Step 6 never returns here with a row left `pending`,
                    // `retry`, `held`, or `failed` -- it retries the run
                    // itself first. Fail closed rather than commit a
                    // decision that misrepresents a leg still in flight.
                    _ => Err(anyhow::anyhow!("settlement_operation_mismatch")),
                }
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let final_decision =
            SettleDecision::new(final_index_membership, score_decision, operations)?;
        let mut evidence = serde_json::from_value::<SettleEvidence>(selection.evidence.clone())
            .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
        evidence.index_command_hash = run.index_command_hash.clone();
        evidence.settlement_progress = settlements
            .iter()
            .map(|settlement| {
                Ok(InstrumentSettlementProgress {
                    instrument_id: InstrumentId::new(settlement.instrument_id.clone())?,
                    operation_ref_hash: settlement.operation_ref_hash.clone(),
                    result_ref_hash: settlement.result_ref_hash.clone(),
                    external_receipt_hash: settlement.external_receipt_hash.clone(),
                })
            })
            .collect::<Result<Vec<_>, trace_commons_gate_api::pipeline::ContractError>>()
            .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
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

    /// The Trace Credit account a run's credit settles to: the submission's
    /// authenticated principal reference.
    async fn credit_account_ref(&self, run: &PipelineRunRecord) -> anyhow::Result<String> {
        Ok(self
            .backend
            .get_trace_submission(&run.tenant_id, run.submission_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("submission is missing"))?
            .auth_principal_ref)
    }

    /// Whether an unreleased hold names `account_ref` (ruling FR2, step 1:
    /// read before any external effect).
    async fn credit_account_is_held(
        &self,
        tenant_id: &str,
        account_ref: &str,
    ) -> anyhow::Result<bool> {
        Ok(self
            .backend
            .list_trace_credit_holds(tenant_id)
            .await?
            .iter()
            .any(|hold| hold.credit_account_ref == account_ref && hold.released_at.is_none()))
    }

    /// Settles the Trace Credit leg into the internal credit ledger (port
    /// 4937 to 5112) in ONE tenant transaction, after the caller has checked
    /// the hold, fenced the lease, and received the adapter's receipt
    /// (`receipt`, which already answers the leg's request). The
    /// transaction:
    ///
    /// 1. locks the run row and confirms this attempt's lease is still the
    ///    current one (`ensure_current_lease`, as the commit transactions do);
    /// 2. takes the per-account advisory lock, so two runs for one account
    ///    never select the same pending event into two batches;
    /// 3. re-checks the hold under that lock -- a hold returns `Held` and
    ///    rolls the transaction back with nothing written;
    /// 4. locks the submission row (`FOR SHARE`) and, in a later statement,
    ///    re-checks it is still operable -- an inoperable submission returns
    ///    `Inoperable` and rolls the transaction back with nothing written
    ///    (the guard Step 6 read before this leg's adapter call
    ///    is only a snapshot; a withdrawal can land in the gap between that
    ///    read and this transaction, and must forfeit the pending award
    ///    rather than let it be paid);
    /// 5. inserts the ledger row idempotently (the event id is derived from
    ///    the run and its Score outcome);
    /// 6. if that event is already final, reuses the one finalized batch that
    ///    carries it; otherwise composes the batch from the account's pending
    ///    events, writes it finalized, sets its `instrument_id`, and marks
    ///    those events final;
    /// 7. completes the settlement row with the receipt's result reference
    ///    and external receipt hash, the event, and the batch. A receipt hash
    ///    another leg already recorded is refused here, and the whole
    ///    transaction rolls back.
    ///
    /// Everything commits together or nothing does, so a crash or a stale
    /// lease anywhere in it leaves the leg exactly as it was before the
    /// attempt, and the retry repeats the idempotent adapter call and this
    /// transaction.
    ///
    /// Adapted for the #971 settlement shape and PR 2's scope:
    ///
    /// - Ruling A8: the microcredit amount comes from
    ///   `InstrumentAward::trace_credit_microcredits`, not the removed
    ///   `Microcredits::from_atomic_units` (private as of #971).
    /// - `external_ref` carries `pipeline_ledger_source_key` (a hash of the
    ///   tenant and request idempotency key); PR 2's schema has no dedicated
    ///   `ledger_source_key` column on `trace_credit_ledger`.
    /// - The batch never dispatches a NEAR payout in PR 2: `near_status` is
    ///   always `Disabled` and `near_contract_id` always `None`.
    async fn settle_internal_credit(
        &self,
        run: &PipelineRunRecord,
        settlement: &PipelineSettlementRecord,
        score_outcome_id: Uuid,
        account_ref: &str,
        receipt: &SettlementReceipt,
        trace_credit_event: PipelineTraceCreditEvent,
    ) -> anyhow::Result<InternalCreditResult> {
        let lease_token = required_lease_token(run)?;
        let account_hash = credit_account_hash(account_ref);
        let event_id = pipeline_credit_event_id(&run.tenant_id, run.run_id, score_outcome_id);
        let amount = InstrumentAward::new(InstrumentId::trace_credit(), settlement.atomic_units)
            .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?
            .trace_credit_microcredits()
            .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
        let external_ref = pipeline_ledger_source_key(&run.tenant_id, &run.request_idempotency_key);
        // Ruling S10: only the stored event type varies by bundle family.
        // `PipelineScore` keeps PR 2's ledger event (`Accepted`); a
        // compatibility run's `NoveltyUtility` leg is main's gate-emitted,
        // never-settled event, which `trace_credit_event_type_is_settlement_eligible`
        // in `trace-commons-ingest.rs` excludes. Rulings T15-1 and T15-2:
        // the rest of the row goes with the event type. A `NoveltyUtility`
        // row is written as `main`'s gate path writes that event: the gate
        // caller's actor role (which `main`'s readers can parse), `final`
        // (it never settles, and the batch composition's event-type filter,
        // Ruling T5-1, keeps it out of every batch), and `main`'s reason
        // shape.
        let (ledger_event_type, actor_role, settlement_state, reason) = match trace_credit_event {
            PipelineTraceCreditEvent::PipelineScore => (
                TraceCreditEventType::Accepted,
                PIPELINE_CREDIT_ACTOR_ROLE,
                "pending",
                PIPELINE_CREDIT_REASON.to_string(),
            ),
            PipelineTraceCreditEvent::NoveltyUtility => (
                TraceCreditEventType::NoveltyUtility,
                PIPELINE_NOVELTY_UTILITY_ACTOR_ROLE,
                "final",
                pipeline_novelty_utility_reason(),
            ),
        };
        let ledger_event_type_label = enum_string(&ledger_event_type)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = PgPipelineStore::tenant_transaction(&mut client, &run.tenant_id).await?;
        ensure_current_lease(&tx, run, lease_token).await?;
        let account_lock = format!("pipeline-credit-account:{}:{account_hash}", run.tenant_id);
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 1))",
            &[&account_lock],
        )
        .await?;
        if list_trace_credit_holds_on_tx(&tx, &run.tenant_id)
            .await?
            .iter()
            .any(|hold| hold.credit_account_ref == account_ref && hold.released_at.is_none())
        {
            // Dropping the transaction rolls it back: nothing was written.
            return Ok(InternalCreditResult::Held);
        }
        // The submission-operability
        // guard Step 6 read before this leg's adapter call is only a
        // snapshot -- `submission_guard` commits and releases its lock
        // immediately. A withdrawal can land in the gap between that read
        // and this transaction, so re-check under this transaction's own
        // lock, before the ledger row is inserted and the batch finalized,
        // and roll back rather than pay a forfeited award.
        //
        // Two statements: lock first, read second. Under READ COMMITTED a
        // later statement in the same transaction is guaranteed to see a
        // withdrawal that committed while the first statement waited for
        // the row lock; relying on a single combined lock-and-read statement
        // for that is not the same guarantee. The `FOR SHARE` lock also
        // makes a withdrawal that has not yet reached its own row update
        // wait until this transaction commits -- the payment then happens
        // before the withdrawal, and withdrawal is not a clawback.
        tx.query_opt(
            "SELECT 1 FROM trace_submissions
              WHERE tenant_id = $1 AND submission_id = $2
              FOR SHARE",
            &[&run.tenant_id, &run.submission_id],
        )
        .await?;
        let submission_operable = tx
            .query_opt(
                "SELECT s.status = 'accepted' AND s.revoked_at IS NULL AND s.purged_at IS NULL
                        AND s.withdrawn_at IS NULL
                        AND (s.expires_at IS NULL OR s.expires_at > NOW())
                        AND NOT EXISTS (
                            SELECT 1 FROM trace_withdrawals w
                             WHERE w.tenant_id = s.tenant_id AND w.submission_id = s.submission_id
                        )
                   FROM trace_submissions s
                  WHERE s.tenant_id = $1 AND s.submission_id = $2",
                &[&run.tenant_id, &run.submission_id],
            )
            .await?
            .map(|row| row.get::<_, bool>(0))
            .unwrap_or(false);
        if !submission_operable {
            // Dropping the transaction rolls it back: nothing was written.
            return Ok(InternalCreditResult::Inoperable);
        }
        // Rulings T15-6 and T15-12: `main` appends a `NoveltyUtility` event
        // only when its credit checks pass. A compatibility leg one of them
        // refuses completes here with no ledger row, under `main`'s label;
        // the Score decision stays as it was.
        if trace_credit_event == PipelineTraceCreditEvent::NoveltyUtility {
            if let Some(label) = self
                .novelty_utility_withheld_reason(&tx, run, amount.get())
                .await?
            {
                update_settlement_on_tx(
                    &tx,
                    run,
                    lease_token,
                    &settlement.instrument_id,
                    SettlementUpdate {
                        operation_state: "complete",
                        result_ref_hash: Some(receipt.result_ref_hash()),
                        external_receipt_hash: receipt.external_receipt_hash(),
                        credit_event_id: None,
                        settlement_batch_id: None,
                        payout_state: None,
                        error_label: Some(label),
                    },
                )
                .await?;
                tx.commit().await?;
                return Ok(InternalCreditResult::Complete);
            }
        }
        tx.execute(
            "INSERT INTO trace_credit_ledger (
                tenant_id, credit_event_id, submission_id, trace_id, credit_account_ref,
                event_type, points_delta, reason, external_ref, actor_principal_ref,
                actor_role, settlement_state, pipeline_run_id, score_outcome_id, instrument_id
             ) VALUES (
                $1,$2,$3,$4,$5,$12,$6,$7,$8,$5,$13,$14,$9,$10,$11
             )
             ON CONFLICT (tenant_id, credit_event_id) DO NOTHING",
            &[
                &run.tenant_id,
                &event_id,
                &run.submission_id,
                &run.trace_id,
                &account_ref,
                &amount.to_credit_decimal(),
                &reason,
                &external_ref,
                &run.run_id,
                &score_outcome_id,
                &settlement.instrument_id,
                &ledger_event_type_label,
                &actor_role,
                &settlement_state,
            ],
        )
        .await?;
        self.inject_crash(PipelineCrashPoint::AfterCreditLedgerInsert)?;
        let event_state: String = tx
            .query_opt(
                "SELECT settlement_state FROM trace_credit_ledger
                  WHERE tenant_id = $1 AND credit_event_id = $2
                    AND pipeline_run_id = $3 AND instrument_id = $4",
                &[
                    &run.tenant_id,
                    &event_id,
                    &run.run_id,
                    &settlement.instrument_id,
                ],
            )
            .await?
            .ok_or_else(|| anyhow::anyhow!("settlement_operation_mismatch"))?
            .get("settlement_state");
        // Ruling S10: a `NoveltyUtility` leg is never composed or finalized
        // into a batch -- it completes with no `settlement_batch_id`, same
        // as `main` never pays that event. `PipelineScore` keeps PR 2's
        // batch-or-reuse behavior unchanged.
        let batch_id = match trace_credit_event {
            PipelineTraceCreditEvent::NoveltyUtility => None,
            PipelineTraceCreditEvent::PipelineScore => Some(if event_state == "final" {
                // The event already settled in an earlier batch: reuse that
                // batch rather than composing a second one that carries it.
                let carrying = tx
                    .query(
                        "SELECT settlement_batch_id FROM trace_credit_settlement_batches
                          WHERE tenant_id = $1 AND instrument_id = $2
                            AND status = 'finalized'
                            AND $3 = ANY(source_credit_event_ids)",
                        &[&run.tenant_id, &settlement.instrument_id, &event_id],
                    )
                    .await?;
                anyhow::ensure!(carrying.len() == 1, "settlement_operation_mismatch");
                carrying[0].get::<_, Uuid>("settlement_batch_id")
            } else {
                self.finalize_pending_credit_batch(
                    &tx,
                    run,
                    &settlement.instrument_id,
                    account_ref,
                    &account_hash,
                    event_id,
                    &ledger_event_type_label,
                )
                .await?
            }),
        };
        update_settlement_on_tx(
            &tx,
            run,
            lease_token,
            &settlement.instrument_id,
            SettlementUpdate {
                operation_state: "complete",
                result_ref_hash: Some(receipt.result_ref_hash()),
                external_receipt_hash: receipt.external_receipt_hash(),
                credit_event_id: Some(event_id),
                settlement_batch_id: batch_id,
                payout_state: None,
                error_label: None,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(InternalCreditResult::Complete)
    }

    /// Rulings T15-6 and T15-8 to T15-11: `main`'s `NoveltyUtility` credit
    /// checks (`attempt_emit_novelty_utility_credit` in
    /// `trace-commons-ingest.rs`, and what it calls), in `main`'s order, for
    /// a compatibility leg about to write its ledger row, on the leg's own
    /// credit transaction. `Some` is the label the leg is withheld under,
    /// the label `main` records on its gate decision for the same refusal.
    ///
    /// - The production gate: with `require_production_gate`, the service's
    ///   scorer and embedder must be production-qualified, where `main`
    ///   requires a production gate service (`non_production_gate`).
    /// - The central issuer: with a non-empty issuer allowlist, a positive
    ///   award needs the pipeline's configured issuer on it, where `main`
    ///   needs its calling gate worker there (`central_issuer_denied`).
    /// - `main` also applies its calling token's scoped allowlists. Settle
    ///   has no calling token, so that check does not apply here (Ruling
    ///   T15-9); the tenant authority's own allowlists were applied to this
    ///   submission at the receipt (`SubmissionAuthority::permits`).
    /// - The tenant policy comes from the authority provider, the source the
    ///   receipt uses. No authority for the tenant, no policy while one is
    ///   required, or a submission row whose scopes or uses do not decode is
    ///   `credit_check_error`, where `main` fails the call; a withheld leg is
    ///   never a charged Settle error.
    /// - The submission's allowed uses must include model training, and its
    ///   consent scopes and model training must be inside the policy's
    ///   non-empty allowlists (`policy_mismatch`). `main` also requires the
    ///   submission to be `accepted`; the leg's operability re-check, which
    ///   runs first, already does (Ruling T15-8).
    ///
    /// This is a copy of `main`'s checks in `attempt_emit_novelty_utility_credit`
    /// (`bin/trace-commons-ingest.rs`), which the library cannot call; a
    /// change to either must be made to both.
    async fn novelty_utility_withheld_reason(
        &self,
        tx: &Transaction<'_>,
        run: &PipelineRunRecord,
        amount_microcredits: u64,
    ) -> anyhow::Result<Option<&'static str>> {
        let checks = &self.novelty_utility_checks;
        if checks.require_production_gate {
            let qualification = self.dependency_qualification();
            if !(qualification.scorer && qualification.embedder) {
                return Ok(Some(PIPELINE_NOVELTY_UTILITY_NON_PRODUCTION_GATE_LABEL));
            }
        }
        if amount_microcredits > 0
            && !checks.central_issuer_principal_refs.is_empty()
            && !checks
                .issuer_principal_ref
                .as_ref()
                .is_some_and(|issuer| checks.central_issuer_principal_refs.contains(issuer))
        {
            return Ok(Some(PIPELINE_NOVELTY_UTILITY_CENTRAL_ISSUER_DENIED_LABEL));
        }
        let Some(authority) = self
            .authority
            .as_ref()
            .and_then(|provider| provider.authority_for_tenant(&run.tenant_id))
        else {
            return Ok(Some(PIPELINE_NOVELTY_UTILITY_CREDIT_CHECK_ERROR_LABEL));
        };
        if authority.policy.is_none() && authority.require_policy {
            return Ok(Some(PIPELINE_NOVELTY_UTILITY_CREDIT_CHECK_ERROR_LABEL));
        }
        let row = tx
            .query_one(
                "SELECT consent_scopes, allowed_uses FROM trace_submissions
                  WHERE tenant_id = $1 AND submission_id = $2",
                &[&run.tenant_id, &run.submission_id],
            )
            .await?;
        let (Ok(consent_scopes), Ok(allowed_uses)) = (
            serde_json::from_value::<Vec<ConsentScope>>(row.get("consent_scopes")),
            serde_json::from_value::<Vec<TraceAllowedUse>>(row.get("allowed_uses")),
        ) else {
            return Ok(Some(PIPELINE_NOVELTY_UTILITY_CREDIT_CHECK_ERROR_LABEL));
        };
        let required = TraceAllowedUse::ModelTraining;
        if !allowed_uses.contains(&required) {
            return Ok(Some(PIPELINE_NOVELTY_UTILITY_POLICY_MISMATCH_LABEL));
        }
        if let Some(policy) = authority.policy.as_ref() {
            let scope_outside = !policy.allowed_consent_scopes.is_empty()
                && !consent_scopes
                    .iter()
                    .any(|scope| policy.allowed_consent_scopes.contains(scope));
            let use_outside =
                !policy.allowed_uses.is_empty() && !policy.allowed_uses.contains(&required);
            if scope_outside || use_outside {
                return Ok(Some(PIPELINE_NOVELTY_UTILITY_POLICY_MISMATCH_LABEL));
            }
        }
        Ok(None)
    }

    /// Composes the account's pending pipeline credit events for
    /// `instrument_id` into one batch, writes it finalized, sets its
    /// `instrument_id`, and marks those events final -- all inside the
    /// caller's credit transaction, under the per-account advisory lock the
    /// caller holds. `event_id` (the run's own event) must be among them.
    ///
    /// Ruling T5-1: only pending events whose `event_type` is `event_type`
    /// (the caller's own leg's ledger event type -- `PipelineScore`'s
    /// `Accepted`, computed once by the caller with the same enum-to-string
    /// path the ledger insert used) are eligible, so a `NoveltyUtility` row
    /// for the same account and instrument is never swept into this batch,
    /// since `main` never pays that event (Review Focus 3). Ruling T15-2
    /// writes that row `final`, so the state predicate leaves it out as
    /// well; the event-type predicate does not depend on that.
    #[allow(clippy::too_many_arguments)]
    async fn finalize_pending_credit_batch(
        &self,
        tx: &Transaction<'_>,
        run: &PipelineRunRecord,
        instrument_id: &str,
        account_ref: &str,
        account_hash: &str,
        event_id: Uuid,
        event_type: &str,
    ) -> anyhow::Result<Uuid> {
        let pending_events = tx
            .query(
                "SELECT credit_event_id, submission_id, points_delta
                   FROM trace_credit_ledger
                  WHERE tenant_id = $1
                    AND pipeline_run_id IS NOT NULL
                    AND instrument_id = $2
                    AND credit_account_ref = $3
                    AND settlement_state = 'pending'
                    AND event_type = $4
                  ORDER BY credit_event_id",
                &[&run.tenant_id, &instrument_id, &account_ref, &event_type],
            )
            .await?;
        anyhow::ensure!(
            pending_events
                .iter()
                .any(|event| event.get::<_, Uuid>("credit_event_id") == event_id),
            "eligible credit event is missing"
        );
        let event_ids = pending_events
            .iter()
            .map(|event| event.get::<_, Uuid>("credit_event_id"))
            .collect::<Vec<_>>();
        let submission_ids = pending_events
            .iter()
            .map(|event| event.get::<_, Uuid>("submission_id"))
            .collect::<Vec<_>>();
        let list_hash = source_list_hash(&event_ids);
        let settled_micros = pending_events.iter().try_fold(0_i64, |total, event| {
            let points_delta: String = event.get("points_delta");
            let amount = Microcredits::from_credit_decimal(&points_delta)
                .map_err(|_| anyhow::anyhow!("credit_amount_overflow"))?;
            let amount = microcredits_to_settled_i64(amount)?;
            total
                .checked_add(amount)
                .ok_or_else(|| anyhow::anyhow!("credit_amount_overflow"))
        })?;
        let batch_id = pipeline_settlement_batch_id(&run.tenant_id, &list_hash);
        let line_item = TraceCreditAccountSettlementLineItem {
            credit_account_ref: account_ref.to_string(),
            credit_account_hash: account_hash.to_string(),
            settled_credit_delta_micros: settled_micros,
            source_credit_event_ids: event_ids.clone(),
            source_submission_ids: submission_ids.clone(),
            source_list_hash: list_hash.clone(),
            near_status: TraceCreditSettlementNearStatus::Disabled,
            near_outbox_id: None,
            near_payout_hold_reason: None,
        };
        let batch = TraceCreditSettlementBatchWrite {
            tenant_id: run.tenant_id.clone(),
            settlement_batch_id: batch_id,
            policy_version: PIPELINE_SETTLEMENT_POLICY_VERSION.to_string(),
            status: TraceCreditSettlementBatchStatus::Finalized,
            reason_hash: list_hash.clone(),
            // `main` records the issuer approval evidence its settlement
            // request named, and none when it named none. Settle names none,
            // and a payout is refused while `main` requires one
            // (`PipelineServiceBuilder::build`).
            issuer_approval_evidence_hash: None,
            source_credit_event_ids: event_ids.clone(),
            source_submission_ids: submission_ids,
            source_list_hash: list_hash,
            settled_credit_points: Microcredits::from_raw(
                u64::try_from(settled_micros).unwrap_or(0),
            )
            .to_credit_decimal(),
            settled_credit_micros: settled_micros,
            line_items: vec![line_item],
            near_contract_id: None,
            ranking_model_version: None,
            ranking_target_use: None,
            ranking_calibration_run_id: None,
            ranking_calibration_report_hash: None,
            ranking_calibration_joined_evidence_hash: None,
            ranking_credit_events_excluded_count: 0,
            ranking_credit_events_excluded_reason_counts: BTreeMap::new(),
            actor_principal_ref: account_ref.to_string(),
        };
        insert_credit_settlement_batch_on_tx(tx, &batch).await?;
        let tagged = tx
            .execute(
                "UPDATE trace_credit_settlement_batches
                    SET instrument_id = $3
                  WHERE tenant_id = $1 AND settlement_batch_id = $2
                    AND (instrument_id IS NULL OR instrument_id = $3)",
                &[&run.tenant_id, &batch_id, &instrument_id],
            )
            .await?;
        anyhow::ensure!(tagged == 1, "settlement_operation_mismatch");
        let finalized = tx
            .execute(
                "UPDATE trace_credit_ledger
                    SET settlement_state = 'final'
                  WHERE tenant_id = $1 AND credit_event_id = ANY($2)
                    AND pipeline_run_id IS NOT NULL
                    AND instrument_id = $3
                    AND settlement_state = 'pending'",
                &[&run.tenant_id, &event_ids, &instrument_id],
            )
            .await?;
        anyhow::ensure!(
            usize::try_from(finalized).ok() == Some(event_ids.len()),
            "settlement_operation_mismatch"
        );
        self.inject_crash(PipelineCrashPoint::AfterCreditBatchFinalize)?;
        Ok(batch_id)
    }

    /// The payout pass (P3-D11): pays out the settled Trace Credit of up
    /// to `limit` of `tenant_id`'s complete runs and returns how many runs
    /// it processed. Nothing at all unless payout is enabled. Payout runs
    /// after Settle, outside any run lease, and never writes a phase
    /// outcome.
    ///
    /// The work list (`PgPipelineStore::list_payout_work_on`) is split in
    /// two:
    ///
    /// - A `submitted` leg only needs its confirmation looked up, which can
    ///   submit nothing, so it is polled without the tenant's NEAR submit
    ///   lock (Ruling T10-10), on one pooled connection, and at most once per
    ///   `PipelinePayoutConfig::confirmation_interval` -- `main`'s NEAR
    ///   outbox scheduler cadence.
    /// - A `pending` leg has a line to submit. Those legs are paid under the
    ///   tenant's NEAR submit lock -- the session advisory lock `main`'s NEAR
    ///   outbox submitter takes (`Database::try_acquire_near_credit_submit_lock`,
    ///   same key) -- held across every external submit, so no two payout
    ///   passes (two ingest replicas, or a pass and a direct
    ///   `process_payout`), and no payout pass and `main`'s submitter, ever
    ///   submit for one tenant at once (Finding I1). When the lock is held,
    ///   those legs wait for the next pass. The lock is taken only when there
    ///   is something to submit, and the locked work runs on the lock's own
    ///   connection, never on a second one (the pool-size-one rule). Every
    ///   path to `NearPayoutAdapter::submit` runs under this lock.
    ///
    /// Ruling T10-5: an error in one run's payout -- a missing batch, a
    /// call that cannot be built, confirmation evidence that is not
    /// hash-only -- is recorded on that run's leg (payout `failed`, under the
    /// safe label `payout_error_label` picks) and the pass goes on with the
    /// next run, so one bad run cannot hold up the tenant's other payouts.
    /// Only a database error ends the pass, as does an injected crash (a
    /// test's stand-in for the process dying).
    pub async fn process_payouts(&self, tenant_id: &str, limit: usize) -> anyhow::Result<usize> {
        let Some((_, config)) = self.payout.as_ref().filter(|(_, config)| config.enabled) else {
            return Ok(0);
        };
        let mut client = self.backend.trace_pool().get().await?;
        let work = PgPipelineStore::list_payout_work_on(
            &mut client,
            tenant_id,
            limit,
            config.confirmation_interval,
        )
        .await?;
        let (to_confirm, to_submit): (Vec<_>, Vec<_>) = work
            .into_iter()
            .partition(|(_, payout_state)| payout_state == "submitted");
        let to_confirm = to_confirm
            .into_iter()
            .map(|(run_id, _)| run_id)
            .collect::<Vec<_>>();
        let to_submit = to_submit
            .into_iter()
            .map(|(run_id, _)| run_id)
            .collect::<Vec<_>>();
        let mut processed = self
            .pay_out_runs_on(&mut client, tenant_id, &to_confirm, false)
            .await?;
        drop(client);
        if to_submit.is_empty() {
            return Ok(processed);
        }
        let Some(mut lock) = self.try_lock_payouts(tenant_id).await? else {
            return Ok(processed);
        };
        let result = match lock.client_mut() {
            Some(client) => {
                self.pay_out_runs_on(client, tenant_id, &to_submit, true)
                    .await
            }
            None => Err(anyhow::anyhow!(PIPELINE_PAYOUT_LOCK_HELD_LABEL)),
        };
        let released = lock.release().await;
        processed += payout_result_after_release(result, released)?;
        Ok(processed)
    }

    /// Pays out one run's settled Trace Credit (`dispatch_near_settlements`)
    /// once the run is complete, under the same tenant lock as the pass's
    /// submits (Finding I1): while a pass or `main`'s submitter holds it,
    /// this is refused with `payout_lock_held`. `None` when the run does not
    /// exist; a run that is not complete is returned untouched. Unlike the
    /// pass, this also takes up a `failed` payout again, does not wait for
    /// the confirmation interval, and returns a per-run error to its caller
    /// instead of recording it.
    pub async fn process_payout(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> anyhow::Result<Option<PipelineRunRecord>> {
        if !self.payout_enabled() {
            return Ok(self.store.get_run(tenant_id, run_id).await?);
        }
        let Some(mut lock) = self.try_lock_payouts(tenant_id).await? else {
            anyhow::bail!(PIPELINE_PAYOUT_LOCK_HELD_LABEL);
        };
        let result = match lock.client_mut() {
            Some(client) => {
                self.process_payout_on(client, tenant_id, run_id, true, true)
                    .await
            }
            None => Err(anyhow::anyhow!(PIPELINE_PAYOUT_LOCK_HELD_LABEL)),
        };
        let released = lock.release().await;
        payout_result_after_release(result, released)
    }

    /// The tenant's NEAR submit lock (`main`'s key), or `None` when another
    /// pass or `main`'s submitter holds it.
    async fn try_lock_payouts(
        &self,
        tenant_id: &str,
    ) -> anyhow::Result<Option<crate::db::NearCreditSubmitAdvisoryLock>> {
        Ok(crate::db::Database::try_acquire_near_credit_submit_lock(
            self.backend.as_ref(),
            tenant_id,
        )
        .await?)
    }

    /// Pays out `run_ids` on `client`, recording a per-run error on its leg
    /// and going on (Ruling T10-5). `may_submit` is true only on the
    /// connection that holds the tenant's NEAR submit lock. The pass never
    /// takes up a `failed` payout again (Ruling F-I3), including one that
    /// failed after the pass listed it: only `process_payout` retries one.
    async fn pay_out_runs_on(
        &self,
        client: &mut deadpool_postgres::Client,
        tenant_id: &str,
        run_ids: &[Uuid],
        may_submit: bool,
    ) -> anyhow::Result<usize> {
        let mut processed = 0;
        for &run_id in run_ids {
            match self
                .process_payout_on(client, tenant_id, run_id, may_submit, false)
                .await
            {
                Ok(run) => {
                    if run.is_some() {
                        processed += 1;
                    }
                }
                Err(error)
                    if error.to_string() == INJECTED_PIPELINE_CRASH
                        || is_database_error(&error) =>
                {
                    return Err(error);
                }
                Err(error) => {
                    set_payout_state_on(
                        client,
                        tenant_id,
                        run_id,
                        TraceCreditSettlementNearStatus::Failed,
                        Some(payout_error_label(&error)),
                    )
                    .await?;
                    processed += 1;
                }
            }
        }
        Ok(processed)
    }

    /// `process_payout`'s body, on `client`. `may_submit` as in
    /// `pay_out_runs_on`; `retry_failed` as in `dispatch_near_settlements`.
    async fn process_payout_on(
        &self,
        client: &mut deadpool_postgres::Client,
        tenant_id: &str,
        run_id: Uuid,
        may_submit: bool,
        retry_failed: bool,
    ) -> anyhow::Result<Option<PipelineRunRecord>> {
        let tx = PgPipelineStore::tenant_transaction(client, tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT * FROM pipeline_runs WHERE tenant_id = $1 AND run_id = $2",
                &[&tenant_id, &run_id],
            )
            .await?;
        tx.commit().await?;
        let Some(run) = row.as_ref().map(pipeline_run_from_row).transpose()? else {
            return Ok(None);
        };
        if run.state == PipelineRunState::Complete {
            self.dispatch_near_settlements(client, &run, may_submit, retry_failed)
                .await?;
        }
        Ok(Some(run))
    }

    /// Port lines 5114 to 5309, for PR 3. For each completed `trace_credit`
    /// leg of `run` on the `near` rail whose payout is `pending` or
    /// `submitted` -- or `failed`, only when `retry_failed` -- pays each line
    /// of the leg's finalized batch
    /// through one `trace_near_credit_outbox` row, keyed by
    /// `pipeline_near_outbox_line_id` and tagged `instrument_id =
    /// 'trace_credit'`, then records the leg's payout state. It submits only
    /// when `may_submit` -- only on the connection that holds the tenant's
    /// NEAR submit lock (Finding I1); without it, a line that still needs a
    /// submit is left for a locked pass, and only confirmations are looked
    /// up (Ruling T10-10). Every outbox read and write is tenant-scoped,
    /// and every write is conditional on the line's current status, so a
    /// repeat is a no-op.
    ///
    /// - A leg without a batch (a compatibility `NoveltyUtility` event) is
    ///   never paid (Ruling S8).
    /// - A line with no outbox row gets a new call on the configured NEAR
    ///   contract (Ruling T10-4). A line with a row replays the call stored
    ///   in it, never one rebuilt from the current configuration (Finding
    ///   I2): its idempotency key names the contract it was first built for.
    ///   A stored call whose contract is no longer the configured one is
    ///   never submitted again -- that could pay twice, on two contracts --
    ///   and the payout ends `failed` under `near_contract_changed`; a
    ///   `submitted` line is still confirmed through its stored key.
    /// - The submission guard is not read: a leg reaches `complete` only
    ///   through Settle, while the submission was operable, and a later
    ///   withdrawal forfeits only the legs Settle has not completed. Its
    ///   settled credit is paid and confirmed after a withdrawal, as
    ///   `main`'s submitter pays finalized credit (owner decision, Zaki
    ///   review 1, item S).
    /// - A line already `submitted` is never submitted again: only its
    ///   confirmation is looked up. The submit is recorded after the
    ///   adapter returns, so a crash between the two repeats the call on
    ///   the next pass, under the same idempotency key
    ///   (`NearPayoutAdapter::submit`).
    /// - A failed submit marks the line and the payout `failed` under
    ///   `near_submit_failed`; the pass does not list it again.
    /// - `retry_failed` is true only from a direct `process_payout`
    ///   (Ruling F-I3). Without it, a `failed` payout, or a `failed` line of
    ///   a payout still `pending`, is never submitted again: another replica
    ///   can fail a payout after this pass listed it as `pending`, and the
    ///   re-read here sees that.
    /// - Confirmation evidence is hash-only (else `near_confirmation_invalid`);
    ///   the evidence and the `confirmed` status commit together.
    async fn dispatch_near_settlements(
        &self,
        client: &mut deadpool_postgres::Client,
        run: &PipelineRunRecord,
        may_submit: bool,
        retry_failed: bool,
    ) -> anyhow::Result<()> {
        let Some((adapter, config)) = self.payout.as_ref() else {
            return Ok(());
        };
        if !config.enabled {
            return Ok(());
        }
        // `PipelineServiceBuilder::build` refuses an enabled payout without one.
        let near_contract_id = config
            .near_contract_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!(PIPELINE_PAYOUT_NEAR_CONTRACT_MISSING_LABEL))?;
        let tx = PgPipelineStore::tenant_transaction(client, &run.tenant_id).await?;
        let settlements = tx
            .query(
                "SELECT *, atomic_units::TEXT AS atomic_units_text
                   FROM pipeline_run_settlements
                  WHERE tenant_id = $1 AND run_id = $2
                  ORDER BY instrument_id",
                &[&run.tenant_id, &run.run_id],
            )
            .await?;
        tx.commit().await?;
        let settlements = settlements
            .iter()
            .map(pipeline_settlement_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        for settlement in settlements.into_iter().filter(|settlement| {
            settlement.instrument_id == InstrumentId::trace_credit().as_str()
                && settlement.operation_state == "complete"
                && settlement.payout_rail == "near"
                && match settlement.payout_state.as_str() {
                    "pending" | "submitted" => true,
                    "failed" => retry_failed,
                    _ => false,
                }
        }) {
            let Some(batch_id) = settlement.settlement_batch_id else {
                continue;
            };
            let (batch_source_list_hash, batch_policy_version, lines) =
                load_payout_batch(client, run, &settlement, batch_id).await?;
            // Zaki review 1, item 2: `main`'s settlement controls, applied
            // before anything is sent. `None` lets the line go.
            let control_refusal = self.payout_control_refusal(&batch_policy_version);
            let mut work = Vec::new();
            for line in lines
                .iter()
                .filter(|line| line.settled_credit_delta_micros > 0)
            {
                let outbox_id = pipeline_near_outbox_line_id(
                    &run.tenant_id,
                    batch_id,
                    &line.credit_account_hash,
                );
                let (status, call) = match near_outbox_line(client, run, outbox_id).await? {
                    Some((status, stored_call)) => {
                        let call: crate::near_credit::NearCreditReceiptCall =
                            serde_json::from_value(stored_call)
                                .map_err(|_| anyhow::anyhow!(PIPELINE_NEAR_CALL_INVALID_LABEL))?;
                        call.validate()
                            .map_err(|_| anyhow::anyhow!(PIPELINE_NEAR_CALL_INVALID_LABEL))?;
                        (Some(status), call)
                    }
                    None => (
                        None,
                        disabled_near_call(
                            near_contract_id,
                            batch_id,
                            &line.credit_account_hash,
                            &batch_source_list_hash,
                            line.settled_credit_delta_micros,
                        )
                        .map_err(|_| anyhow::anyhow!(PIPELINE_NEAR_CALL_INVALID_LABEL))?,
                    ),
                };
                work.push((line, call, outbox_id, status));
            }

            let mut contract_changed = false;
            let mut refused_by_control = None;
            for (line, call, outbox_id, status) in &work {
                let outbox_id = *outbox_id;
                match status.as_deref() {
                    Some("confirmed") | Some("disabled") => continue,
                    Some("failed") if !retry_failed => continue,
                    Some("submitted") => {}
                    _ => {
                        if !may_submit {
                            continue;
                        }
                        if call.contract_id != near_contract_id {
                            contract_changed = true;
                            continue;
                        }
                        if let Some(label) = control_refusal {
                            refused_by_control = Some(label);
                            continue;
                        }
                        if status.is_none() {
                            insert_near_outbox_line(
                                client,
                                run,
                                &settlement,
                                batch_id,
                                outbox_id,
                                &line.credit_account_hash,
                                call,
                            )
                            .await?;
                        }
                        match adapter.submit(call).await {
                            Ok(transaction_ref) => {
                                mark_near_outbox_line_submitted(
                                    client,
                                    run,
                                    outbox_id,
                                    &sha256_prefixed(transaction_ref.as_bytes()),
                                )
                                .await?;
                                self.inject_crash(PipelineCrashPoint::AfterNearSubmit)?;
                            }
                            Err(_) => {
                                mark_near_outbox_line_failed(
                                    client,
                                    run,
                                    outbox_id,
                                    &sha256_prefixed(PIPELINE_NEAR_SUBMIT_FAILED_LABEL.as_bytes()),
                                )
                                .await?;
                                continue;
                            }
                        }
                    }
                }
                let Some(evidence) = adapter.confirmation(&call.idempotency_key).await else {
                    continue;
                };
                if !(evidence.transaction_hash_hash.starts_with("sha256:")
                    && evidence.receipt_hash.starts_with("sha256:"))
                {
                    anyhow::bail!(PIPELINE_NEAR_CONFIRMATION_INVALID_LABEL);
                }
                let tx = PgPipelineStore::tenant_transaction(client, &run.tenant_id).await?;
                tx.execute(
                    "UPDATE trace_near_credit_outbox
                        SET near_call_json = jsonb_set(
                                near_call_json,
                                '{confirmation_evidence}',
                                jsonb_build_object(
                                    'transaction_hash_hash', $3::TEXT,
                                    'receipt_hash', $4::TEXT
                                ),
                                TRUE
                            ),
                            status = 'confirmed',
                            near_transaction_hash = $3,
                            confirmed_at = NOW(),
                            last_error_hash = NULL
                      WHERE tenant_id = $1 AND near_outbox_id = $2
                        AND status = 'submitted'",
                    &[
                        &run.tenant_id,
                        &outbox_id,
                        &evidence.transaction_hash_hash,
                        &evidence.receipt_hash,
                    ],
                )
                .await?;
                tx.commit().await?;
                self.inject_crash(PipelineCrashPoint::AfterNearConfirm)?;
            }
            if contract_changed {
                set_payout_state_on(
                    client,
                    &run.tenant_id,
                    run.run_id,
                    TraceCreditSettlementNearStatus::Failed,
                    Some(PIPELINE_NEAR_CONTRACT_CHANGED_LABEL),
                )
                .await?;
            } else if let Some(label) = refused_by_control {
                set_payout_state_on(
                    client,
                    &run.tenant_id,
                    run.run_id,
                    TraceCreditSettlementNearStatus::Failed,
                    Some(label),
                )
                .await?;
            } else {
                record_payout_state_on(client, run, batch_id).await?;
            }
        }
        Ok(())
    }

    /// `main`'s settlement controls a payout line must pass before it is
    /// sent (Zaki review 1, item 2), from `main`'s configuration as ingest
    /// hands it over (`PipelineNoveltyUtilityChecks`): the batch's policy
    /// version on `main`'s allowed list (an empty list allows any), and,
    /// with `main`'s central-issuer allowlist set, the pipeline's issuer on
    /// it, where `main` needs the principal that runs its live settlement.
    /// `Some` is the label of the first control that refuses. `main`'s
    /// issuer-approval requirement has no per-line check: an enabled payout
    /// is refused at build while it is set.
    fn payout_control_refusal(&self, batch_policy_version: &str) -> Option<&'static str> {
        let checks = &self.novelty_utility_checks;
        if !checks.settlement_allowed_policy_versions.is_empty()
            && !checks
                .settlement_allowed_policy_versions
                .contains(batch_policy_version)
        {
            return Some(PIPELINE_PAYOUT_POLICY_VERSION_NOT_ALLOWED_LABEL);
        }
        if !checks.central_issuer_principal_refs.is_empty()
            && !checks
                .issuer_principal_ref
                .as_ref()
                .is_some_and(|issuer| checks.central_issuer_principal_refs.contains(issuer))
        {
            return Some(PIPELINE_NOVELTY_UTILITY_CENTRAL_ISSUER_DENIED_LABEL);
        }
        None
    }
}

/// The payout's result once the tenant's NEAR submit lock is released
/// (Ruling F-M5): the locked work's own error comes first, so a failed
/// release cannot hide it (an injected crash included), and that release
/// error is logged hash-only. After locked work that succeeded, a failed
/// release is the error.
fn payout_result_after_release<T>(
    result: anyhow::Result<T>,
    released: Result<(), DatabaseError>,
) -> anyhow::Result<T> {
    match result {
        Ok(value) => {
            released?;
            Ok(value)
        }
        Err(error) => {
            if let Err(release_error) = released {
                tracing::warn!(
                    label = "pipeline_payout_lock_release_failed",
                    error_hash = %sha256_prefixed(release_error.to_string().as_bytes()),
                    "the tenant's NEAR submit lock was not released after a failed payout pass"
                );
            }
            Err(error)
        }
    }
}

/// The source list hash and line items of the finalized batch that carries
/// a completed Trace Credit leg, read on the payout's connection.
async fn load_payout_batch(
    client: &mut deadpool_postgres::Client,
    run: &PipelineRunRecord,
    settlement: &PipelineSettlementRecord,
    batch_id: Uuid,
) -> anyhow::Result<(String, String, Vec<TraceCreditAccountSettlementLineItem>)> {
    let tx = PgPipelineStore::tenant_transaction(client, &run.tenant_id).await?;
    let row = tx
        .query_opt(
            "SELECT source_list_hash, policy_version, line_items_json
               FROM trace_credit_settlement_batches
              WHERE tenant_id = $1 AND settlement_batch_id = $2
                AND instrument_id = $3 AND status = 'finalized'",
            &[&run.tenant_id, &batch_id, &settlement.instrument_id],
        )
        .await?
        .ok_or_else(|| anyhow::anyhow!(PIPELINE_PAYOUT_BATCH_MISSING_LABEL))?;
    tx.commit().await?;
    let lines = serde_json::from_value(row.get("line_items_json"))
        .map_err(|_| anyhow::anyhow!(PIPELINE_PAYOUT_BATCH_MISSING_LABEL))?;
    Ok((
        row.get("source_list_hash"),
        row.get("policy_version"),
        lines,
    ))
}

/// The status and stored call of one outbox line, or `None` when it has no
/// row yet.
async fn near_outbox_line(
    client: &mut deadpool_postgres::Client,
    run: &PipelineRunRecord,
    outbox_id: Uuid,
) -> anyhow::Result<Option<(String, serde_json::Value)>> {
    let tx = PgPipelineStore::tenant_transaction(client, &run.tenant_id).await?;
    let line = tx
        .query_opt(
            "SELECT status, near_call_json FROM trace_near_credit_outbox
              WHERE tenant_id = $1 AND near_outbox_id = $2",
            &[&run.tenant_id, &outbox_id],
        )
        .await?
        .map(|row| (row.get("status"), row.get("near_call_json")));
    tx.commit().await?;
    Ok(line)
}

/// Writes one outbox line `pending`, for `instrument_id = 'trace_credit'`;
/// a line that already exists is left as it is.
async fn insert_near_outbox_line(
    client: &mut deadpool_postgres::Client,
    run: &PipelineRunRecord,
    settlement: &PipelineSettlementRecord,
    batch_id: Uuid,
    outbox_id: Uuid,
    credit_account_hash: &str,
    call: &crate::near_credit::NearCreditReceiptCall,
) -> anyhow::Result<()> {
    let call_json = serde_json::to_value(call)
        .map_err(|_| anyhow::anyhow!(PIPELINE_NEAR_CALL_INVALID_LABEL))?;
    let tx = PgPipelineStore::tenant_transaction(client, &run.tenant_id).await?;
    tx.execute(
        "INSERT INTO trace_near_credit_outbox (
            tenant_id, near_outbox_id, settlement_batch_id, credit_account_hash,
            near_call_json, status, payout_near_account_id, instrument_id
         ) VALUES ($1,$2,$3,$4,$5,'pending',NULL,$6)
         ON CONFLICT (tenant_id, near_outbox_id) DO NOTHING",
        &[
            &run.tenant_id,
            &outbox_id,
            &batch_id,
            &credit_account_hash,
            &call_json,
            &settlement.instrument_id,
        ],
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Records a line's submit: `submitted`, with the hash of the adapter's
/// transaction reference. Only a `pending` or `failed` line moves; the
/// columns change as `update_trace_near_credit_outbox_status` changes them.
async fn mark_near_outbox_line_submitted(
    client: &mut deadpool_postgres::Client,
    run: &PipelineRunRecord,
    outbox_id: Uuid,
    near_transaction_hash: &str,
) -> anyhow::Result<()> {
    let tx = PgPipelineStore::tenant_transaction(client, &run.tenant_id).await?;
    tx.execute(
        "UPDATE trace_near_credit_outbox
            SET status = 'submitted',
                near_transaction_hash = $3,
                submitted_at = COALESCE(submitted_at, NOW()),
                confirmed_at = NULL,
                last_error_hash = NULL
          WHERE tenant_id = $1 AND near_outbox_id = $2
            AND status IN ('pending', 'failed')",
        &[&run.tenant_id, &outbox_id, &near_transaction_hash],
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Records a line's failed submit: `failed`, with a hash-only error. Only a
/// `pending` or `failed` line moves.
async fn mark_near_outbox_line_failed(
    client: &mut deadpool_postgres::Client,
    run: &PipelineRunRecord,
    outbox_id: Uuid,
    last_error_hash: &str,
) -> anyhow::Result<()> {
    let tx = PgPipelineStore::tenant_transaction(client, &run.tenant_id).await?;
    tx.execute(
        "UPDATE trace_near_credit_outbox
            SET status = 'failed',
                confirmed_at = NULL,
                last_error_hash = $3
          WHERE tenant_id = $1 AND near_outbox_id = $2
            AND status IN ('pending', 'failed')",
        &[&run.tenant_id, &outbox_id, &last_error_hash],
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Records a leg's payout state from its batch's outbox lines: every line
/// `confirmed` is `confirmed`, any `failed` line is `failed` under
/// `near_submit_failed`, every line `submitted` or `confirmed` is
/// `submitted`, and otherwise (no line yet, or one still `pending`)
/// `pending`.
async fn record_payout_state_on(
    client: &mut deadpool_postgres::Client,
    run: &PipelineRunRecord,
    batch_id: Uuid,
) -> anyhow::Result<()> {
    let tx = PgPipelineStore::tenant_transaction(client, &run.tenant_id).await?;
    let statuses = tx
        .query(
            "SELECT status
               FROM trace_near_credit_outbox
              WHERE tenant_id = $1 AND settlement_batch_id = $2
                AND instrument_id = $3",
            &[
                &run.tenant_id,
                &batch_id,
                &InstrumentId::trace_credit().as_str(),
            ],
        )
        .await?
        .iter()
        .map(|row| row.get::<_, String>("status"))
        .collect::<Vec<_>>();
    tx.commit().await?;
    let (payout, label) =
        if !statuses.is_empty() && statuses.iter().all(|status| status == "confirmed") {
            (TraceCreditSettlementNearStatus::Confirmed, None)
        } else if statuses.iter().any(|status| status == "failed") {
            (
                TraceCreditSettlementNearStatus::Failed,
                Some(PIPELINE_NEAR_SUBMIT_FAILED_LABEL),
            )
        } else if !statuses.is_empty()
            && statuses
                .iter()
                .all(|status| status == "submitted" || status == "confirmed")
        {
            (TraceCreditSettlementNearStatus::Submitted, None)
        } else {
            (TraceCreditSettlementNearStatus::Pending, None)
        };
    set_payout_state_on(client, &run.tenant_id, run.run_id, payout, label).await
}

/// Sets the payout state and label of a run's `trace_credit` leg. A
/// `disabled` or `confirmed` payout is never changed.
async fn set_payout_state_on(
    client: &mut deadpool_postgres::Client,
    tenant_id: &str,
    run_id: Uuid,
    payout: TraceCreditSettlementNearStatus,
    label: Option<&str>,
) -> anyhow::Result<()> {
    let tx = PgPipelineStore::tenant_transaction(client, tenant_id).await?;
    tx.execute(
        "UPDATE pipeline_run_settlements
            SET payout_state = $4, last_error_label = $5, updated_at = NOW()
          WHERE tenant_id = $1 AND run_id = $2 AND instrument_id = $3
            AND payout_state IN ('pending', 'submitted', 'failed')",
        &[
            &tenant_id,
            &run_id,
            &InstrumentId::trace_credit().as_str(),
            &payout_state_label(payout),
            &label,
        ],
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Whether `error` came from the database or its connection pool, anywhere
/// in its chain. The payout pass stops on one (Ruling T10-5); every other
/// payout error is the run's own and is recorded on its leg.
fn is_database_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.is::<DatabaseError>()
            || cause.is::<deadpool_postgres::PoolError>()
            || cause.is::<tokio_postgres::Error>()
    })
}

/// The safe label a per-run payout error is recorded under (Ruling T10-5):
/// the dispatch raises its own errors as these labels, and anything else is
/// `payout_operation_failed`, never the error's own text.
fn payout_error_label(error: &anyhow::Error) -> &'static str {
    match error.to_string().as_str() {
        PIPELINE_PAYOUT_BATCH_MISSING_LABEL => PIPELINE_PAYOUT_BATCH_MISSING_LABEL,
        PIPELINE_NEAR_CALL_INVALID_LABEL => PIPELINE_NEAR_CALL_INVALID_LABEL,
        PIPELINE_NEAR_CONFIRMATION_INVALID_LABEL => PIPELINE_NEAR_CONFIRMATION_INVALID_LABEL,
        _ => PIPELINE_PAYOUT_OPERATION_FAILED_LABEL,
    }
}

/// M4: every Trace Credit award in `awards` fits the credit ledger's
/// signed 64-bit microcredit column (`microcredits_to_settled_i64`), else
/// the safe label `score_outcome_invalid`. `InstrumentAward::new` and its
/// serde form already refuse a Trace Credit amount above
/// `MAX_TRACE_CREDIT_MICROCREDITS` (`i64::MAX`), so no Score output can
/// carry one today; this check keeps the runner's refusal ahead of any
/// artifact write should that contract bound ever move, and the
/// `pipeline_run_settlements_trace_credit_bound` CHECK stays the backstop.
fn ensure_trace_credit_awards_fit_the_ledger(awards: &InstrumentAwards) -> anyhow::Result<()> {
    for award in awards
        .iter()
        .filter(|award| award.instrument_id() == &InstrumentId::trace_credit())
    {
        let microcredits = award
            .trace_credit_microcredits()
            .map_err(|_| anyhow::anyhow!("score_outcome_invalid"))?;
        microcredits_to_settled_i64(microcredits)
            .map_err(|_| anyhow::anyhow!("score_outcome_invalid"))?;
    }
    Ok(())
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

/// The object id a claim stores one of its run's phase artifacts under
/// (`artifact` is `approved`, `index-command`, or `score-neighbors`).
///
/// Ruling FR1: the id carries the claim's lease token, not the run id
/// alone. Every write encrypts with a fresh salt and nonce, so two writes of
/// equal plaintext under one key still differ in ciphertext; a worker whose
/// lease expired during policy work must therefore never write the key a
/// later claim committed, or the committed `content_sha256` would stop
/// matching the stored object. The database refs a commit records stay
/// deterministic (the approved object ref id is derived from the run id
/// alone); only the object key moves per claim. A phase attempt that
/// crashes before its commit leaves its objects unreferenced (not tracked
/// in PR 2). The receipt's source object is tracked: see
/// `pipeline_receipt_object_id`.
pub fn pipeline_attempt_object_id(artifact: &str, run_id: Uuid, lease_token: Uuid) -> String {
    format!("pipeline-{artifact}-{run_id}-{lease_token}")
}

/// The object id one receipt attempt stores its source envelope under.
/// Like `pipeline_attempt_object_id`, the id
/// carries a per-attempt value -- a random attempt id, since a receipt holds
/// no lease -- so two attempts for one key never write the same object, and
/// a committed object ref's hash always matches its object. The attempt's
/// staging row names the object before it is written.
fn pipeline_receipt_object_id(run_id: Uuid, attempt_id: Uuid) -> String {
    pipeline_attempt_object_id("source", run_id, attempt_id)
}

/// The transaction-scoped advisory lock that serializes a receipt key's
/// staging and final transactions.
fn pipeline_receipt_lock(tenant_id: &str, request_idempotency_key: &str) -> String {
    format!("pipeline-receipt:{tenant_id}:{request_idempotency_key}")
}

/// Builds the `trace_object_refs` write for the approved content a Review
/// approval stores. The object ref id is derived from the run id alone, so a
/// retry after a crash between the artifact write and the commit records the
/// identical id; the object key it points at is the retry's own
/// (`pipeline_attempt_object_id`), and `commit_review` records exactly one
/// ref for the run.
fn approved_object_ref(
    run: &PipelineRunRecord,
    receipt: &EncryptedTraceArtifactReceipt,
    size_bytes: usize,
    object_store: &str,
) -> TraceObjectRefWrite {
    TraceObjectRefWrite {
        object_ref_id: Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("tracecommons:pipeline-approved-object:{}", run.run_id).as_bytes(),
        ),
        tenant_id: run.tenant_id.clone(),
        submission_id: run.submission_id,
        artifact_kind: TraceObjectArtifactKind::ReviewSnapshot,
        object_store: object_store.to_string(),
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

/// The objects a Score attempt stores for its run, by the `artifact` name
/// `pipeline_attempt_object_id` gives each: the index command and the
/// neighbour set.
const PIPELINE_SCORE_OBJECT_ARTIFACTS: [&str; 2] = ["index-command", "score-neighbors"];

/// The object ref id of the Score object `artifact` of `run_id`: derived
/// from the run id alone, as the approved object ref id is, so a retry
/// records the identical id and only the object key moves per claim.
fn pipeline_score_object_ref_id(run_id: Uuid, artifact: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("tracecommons:pipeline-{artifact}-object:{run_id}").as_bytes(),
    )
}

/// Builds the `trace_object_refs` write for one object a Score attempt
/// stored (`artifact` is `index-command` or `score-neighbors`), under the
/// `worker_intermediate` kind, the kind of every object stored as a
/// `VectorPayload` artifact. `created_by_job_id` is the run id, and with
/// the object ref id it is how `is_pipeline_score_object_ref` tells this
/// ref from a vector payload of `main`'s.
fn score_object_ref(
    run: &PipelineRunRecord,
    artifact: &str,
    receipt: &EncryptedTraceArtifactReceipt,
    size_bytes: usize,
    object_store: &str,
) -> TraceObjectRefWrite {
    TraceObjectRefWrite {
        object_ref_id: pipeline_score_object_ref_id(run.run_id, artifact),
        tenant_id: run.tenant_id.clone(),
        submission_id: run.submission_id,
        artifact_kind: TraceObjectArtifactKind::WorkerIntermediate,
        object_store: object_store.to_string(),
        object_key: receipt.object_key.clone(),
        content_sha256: format!("sha256:{}", receipt.ciphertext_sha256),
        encryption_key_ref: format!(
            "tenant:{}",
            pipeline_tenant_storage_ref(&run.tenant_id).as_str()
        ),
        size_bytes: i64::try_from(size_bytes).unwrap_or(i64::MAX),
        compression: None,
        created_by_job_id: Some(run.run_id),
    }
}

/// Records one Score object (`score_object_ref`) on `commit_score`'s
/// transaction. A plain insert: the commit happens once per run, so an
/// existing ref at this id is an error, never something to keep.
async fn insert_score_object_ref_on_tx(
    tx: &Transaction<'_>,
    object_ref: &TraceObjectRefWrite,
) -> Result<(), DatabaseError> {
    tx.execute(
        "INSERT INTO trace_object_refs (
            tenant_id, submission_id, object_ref_id, artifact_kind, object_store,
            object_key, content_sha256, encryption_key_ref, size_bytes, compression,
            created_by_job_id
         ) VALUES ($1,$2,$3,'worker_intermediate',$4,$5,$6,$7,$8,$9,$10)",
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
    Ok(())
}

/// Whether an object ref names an object a pipeline Score attempt stored
/// (`score_object_ref`): its id is the one `pipeline_score_object_ref_id`
/// derives from the run id it records as `created_by_job_id`. `main`'s
/// revocation-propagation worker uses this before it deletes a
/// `worker_intermediate` payload, to check the object as a pipeline object
/// (`is_pipeline_artifact_wrapper`) instead of as one of its own vector
/// payloads.
pub fn is_pipeline_score_object_ref(object_ref_id: Uuid, created_by_job_id: Option<Uuid>) -> bool {
    created_by_job_id.is_some_and(|run_id| {
        PIPELINE_SCORE_OBJECT_ARTIFACTS
            .iter()
            .any(|artifact| pipeline_score_object_ref_id(run_id, artifact) == object_ref_id)
    })
}

/// Whether `wrapper` is the byte wrapper (decision P1) every object the
/// pipeline stores is written as.
pub fn is_pipeline_artifact_wrapper(wrapper: &serde_json::Value) -> bool {
    decode_pipeline_artifact_bytes(wrapper).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Zaki review 1, item 1: `main`'s revocation-propagation worker checks a
    /// `worker_intermediate` object as a pipeline object only when its ref
    /// is one `score_object_ref` builds. A vector payload ref of `main`'s,
    /// whose `created_by_job_id` is its vector entry, is not one; nor is a
    /// Score object ref id recorded against another run, or with no run.
    #[test]
    fn only_a_score_object_ref_is_a_pipeline_score_object_ref() {
        let run_id = Uuid::new_v4();
        for artifact in PIPELINE_SCORE_OBJECT_ARTIFACTS {
            let object_ref_id = pipeline_score_object_ref_id(run_id, artifact);
            assert!(is_pipeline_score_object_ref(object_ref_id, Some(run_id)));
            assert!(!is_pipeline_score_object_ref(
                object_ref_id,
                Some(Uuid::new_v4())
            ));
            assert!(!is_pipeline_score_object_ref(object_ref_id, None));
        }
        let vector_entry_id = Uuid::new_v4();
        assert!(!is_pipeline_score_object_ref(
            Uuid::new_v4(),
            Some(vector_entry_id)
        ));
        assert!(!is_pipeline_score_object_ref(
            pipeline_score_object_ref_id(run_id, "approved"),
            Some(run_id)
        ));
    }

    /// Ruling F-M5: the payout's own result comes first. A lock release that
    /// fails after a failed pass does not hide the pass's error (an injected
    /// crash included); after a pass that succeeded, the release error is
    /// the result.
    #[test]
    fn the_payout_error_comes_before_the_lock_release_error() {
        let release_error = || Err(DatabaseError::Pool("release failed".to_string()));
        let error = payout_result_after_release::<usize>(
            Err(anyhow::anyhow!(INJECTED_PIPELINE_CRASH)),
            release_error(),
        )
        .expect_err("the failed pass is an error");
        assert_eq!(error.to_string(), INJECTED_PIPELINE_CRASH);
        let error = payout_result_after_release(Ok(2_usize), release_error())
            .expect_err("a failed release after a good pass is an error");
        assert!(error.to_string().contains("release failed"), "{error}");
        assert_eq!(payout_result_after_release(Ok(2_usize), Ok(())).unwrap(), 2);
        let error = payout_result_after_release::<usize>(
            Err(anyhow::anyhow!(PIPELINE_PAYOUT_LOCK_HELD_LABEL)),
            Ok(()),
        )
        .expect_err("the failed pass is an error");
        assert_eq!(error.to_string(), PIPELINE_PAYOUT_LOCK_HELD_LABEL);
    }

    /// M3: a stored package that fails its own validation is permanent
    /// (`bundle_package_invalid`, the run fails); any other database error
    /// while loading the bound bundle is the store being unavailable -- an
    /// uncharged suspension, never `bundle_package_invalid`.
    #[test]
    fn a_database_error_loading_the_bound_bundle_is_not_an_invalid_package() {
        assert_eq!(
            bound_bundle_load_error_label(&DatabaseError::Serialization(
                PIPELINE_BUNDLE_INVALID_LABEL.to_string()
            )),
            PIPELINE_BUNDLE_INVALID_LABEL
        );
        for error in [
            DatabaseError::Pool("pool exhausted".to_string()),
            DatabaseError::Query("statement timeout".to_string()),
            DatabaseError::Serialization("row decode failed".to_string()),
        ] {
            assert_eq!(
                bound_bundle_load_error_label(&error),
                PIPELINE_BUNDLE_STORE_UNAVAILABLE_LABEL
            );
        }
    }

    /// M4: a Trace Credit award up to the ledger's `i64::MAX` microcredits
    /// passes the runner's check; one unit more cannot even be built as an
    /// award, which is why the runner's refusal is a backstop.
    #[test]
    fn trace_credit_awards_are_bounded_by_the_ledger_before_storage() {
        let at_limit = InstrumentAwards::new(vec![
            InstrumentAward::new(
                InstrumentId::trace_credit(),
                AtomicUnits::from_raw(i64::MAX as u128),
            )
            .unwrap(),
        ])
        .unwrap();
        assert!(ensure_trace_credit_awards_fit_the_ledger(&at_limit).is_ok());
        assert!(
            InstrumentAward::new(
                InstrumentId::trace_credit(),
                AtomicUnits::from_raw(i64::MAX as u128 + 1),
            )
            .is_err()
        );
    }

    /// Expected value computed outside Rust: SHA-256("tenant-a"), first 16 bytes.
    #[test]
    fn tenant_storage_ref_uses_the_ingest_derivation() {
        assert_eq!(
            pipeline_tenant_storage_ref("tenant-a").as_str(),
            "tenant_sha256:80a707af7dc77ee1228f9127180f3964"
        );
    }

    /// The default configuration is Review 5 minutes, Score 30 minutes,
    /// Settle 5 minutes, and Score's default is longer than the old fixed
    /// 30-second lease every phase used to share.
    #[test]
    fn default_lease_config_fits_a_slow_score() {
        let config = PipelineLeaseConfig::default();
        assert_eq!(config.review(), Duration::minutes(5));
        assert_eq!(config.score(), Duration::minutes(30));
        assert_eq!(config.settle(), Duration::minutes(5));
        assert!(config.score() > Duration::seconds(30));
    }

    /// Each phase's lease must fall in [1 second, 2 hours] inclusive.
    #[test]
    fn lease_config_refuses_a_value_outside_one_second_to_two_hours() {
        let one_second = Duration::seconds(1);
        let two_hours = Duration::hours(2);

        assert!(PipelineLeaseConfig::new(one_second, one_second, one_second).is_ok());
        assert!(PipelineLeaseConfig::new(two_hours, two_hours, two_hours).is_ok());

        let zero = Duration::zero();
        let error = PipelineLeaseConfig::new(zero, one_second, one_second)
            .expect_err("zero seconds is refused");
        assert_eq!(error.to_string(), PIPELINE_LEASE_CONFIG_INVALID_LABEL);

        let too_long = two_hours + Duration::seconds(1);
        let error = PipelineLeaseConfig::new(one_second, too_long, one_second)
            .expect_err("two hours plus one second is refused");
        assert_eq!(error.to_string(), PIPELINE_LEASE_CONFIG_INVALID_LABEL);

        // Every one of the three fields is checked, not only the first.
        let error = PipelineLeaseConfig::new(one_second, one_second, zero)
            .expect_err("a zero settle lease is refused too");
        assert_eq!(error.to_string(), PIPELINE_LEASE_CONFIG_INVALID_LABEL);
    }

    /// `is_stale_lease_error` must recognize the stale-lease condition
    /// both as a `DatabaseError::Constraint` (every mark_*/commit_* method's
    /// `stale_lease_error()`) and as `ensure_live_lease`'s bare `anyhow!` --
    /// and must not mistake an unrelated `DatabaseError::Constraint` (a
    /// phase mismatch, an invalid lease duration) or an unrelated bare
    /// anyhow error for one.
    #[test]
    fn stale_lease_detection_matches_by_type_or_by_the_exact_message() {
        assert!(is_stale_lease_error(&stale_lease_error().into()));
        assert!(is_stale_lease_error(&anyhow::anyhow!(
            "pipeline lease is stale"
        )));
        assert!(!is_stale_lease_error(&anyhow::anyhow!(
            "pipeline lease is STALE"
        )));
        assert!(!is_stale_lease_error(
            &DatabaseError::Constraint("phase does not match run transition".to_string()).into()
        ));
        assert!(!is_stale_lease_error(&anyhow::anyhow!(
            PIPELINE_INDEX_CONFLICT_LABEL
        )));
        // A leg write refused because the leg is not open has
        // its own error, never the stale-lease one -- recorded as
        // `lease_expired` it would be uncharged with no terminal bound.
        assert!(!is_stale_lease_db_error(&settlement_leg_not_open_error()));
        assert!(!is_stale_lease_error(
            &settlement_leg_not_open_error().into()
        ));
    }

    /// A failure path still resolves a leg unless it is `complete`,
    /// `forfeited`, or already `failed` as `settlement_unreconciled`, or an
    /// external leg `failed` as `settlement_result_mismatch` (its effect is
    /// unknown). A Trace Credit leg `failed` as `settlement_result_mismatch`
    /// is unresolved (no ledger row was written, and the failure path
    /// forfeits it as `run_failed`). So is a leg `failed` with a `Conflict`
    /// or `Rejected` label (the failure path forfeits it), and one `failed`
    /// for a reason Step 6 retries (an amount over the cap, a request that
    /// cannot be formed, or no label at all). The SQL form names the same
    /// labels, the Trace Credit exception, and the `COALESCE` that makes a
    /// missing label unresolved.
    #[test]
    fn a_leg_is_unresolved_until_it_is_resolved() {
        let trace_credit = InstrumentId::trace_credit();
        for instrument in ["storage_rebate", trace_credit.as_str()] {
            for state in ["pending", "leased", "retry", "held"] {
                assert!(
                    settlement_leg_is_unresolved(instrument, state, None),
                    "{instrument} {state}"
                );
            }
            for label in [
                None,
                Some(SettlementError::Conflict.label()),
                Some(SettlementError::Rejected.label()),
                Some(PIPELINE_CREDIT_CAP_LABEL),
                Some(PIPELINE_SETTLEMENT_REQUEST_INVALID_LABEL),
            ] {
                assert!(
                    settlement_leg_is_unresolved(instrument, "failed", label),
                    "{instrument} {label:?}"
                );
            }
            assert!(!settlement_leg_is_unresolved(
                instrument,
                "failed",
                Some(PIPELINE_SETTLEMENT_UNRECONCILED_LABEL)
            ));
            assert!(!settlement_leg_is_unresolved(instrument, "complete", None));
            for label in [
                PIPELINE_SETTLEMENT_RUN_FAILED_LABEL,
                SettlementError::Conflict.label(),
            ] {
                assert!(!settlement_leg_is_unresolved(
                    instrument,
                    "forfeited",
                    Some(label)
                ));
            }
        }
        assert!(!settlement_leg_is_unresolved(
            "storage_rebate",
            "failed",
            Some(PIPELINE_SETTLEMENT_RESULT_MISMATCH_LABEL)
        ));
        assert!(settlement_leg_is_unresolved(
            trace_credit.as_str(),
            "failed",
            Some(PIPELINE_SETTLEMENT_RESULT_MISMATCH_LABEL)
        ));
        for fragment in [
            format!("'{PIPELINE_SETTLEMENT_UNRECONCILED_LABEL}'"),
            format!("'{PIPELINE_SETTLEMENT_RESULT_MISMATCH_LABEL}'"),
            format!("s.instrument_id <> '{}'", trace_credit.as_str()),
            "COALESCE(".to_string(),
            "FALSE".to_string(),
        ] {
            assert!(
                UNRESOLVED_SETTLEMENT_LEG_SQL.contains(&fragment),
                "{fragment}"
            );
        }
    }

    /// A leg failed as `settlement_result_mismatch`,
    /// `settlement_request_conflict`, or `settlement_request_rejected` is
    /// never dispatched again, and neither is a `complete` or `forfeited`
    /// leg. Every other leg -- a `settlement_unreconciled` leg of a live run
    /// and a `failed` leg with no label included -- is open to dispatch. The
    /// SQL form names the same labels, and its `COALESCE` keeps a missing
    /// label open.
    #[test]
    fn a_leg_failed_closed_is_never_dispatched_again() {
        assert_eq!(
            TERMINAL_SETTLEMENT_FAILURE_LABELS,
            [
                PIPELINE_SETTLEMENT_RESULT_MISMATCH_LABEL,
                SettlementError::Conflict.label(),
                SettlementError::Rejected.label(),
            ]
        );
        for label in TERMINAL_SETTLEMENT_FAILURE_LABELS {
            assert!(
                !settlement_leg_is_open_to_dispatch("failed", Some(label)),
                "{label}"
            );
            assert!(
                SETTLEMENT_LEG_OPEN_TO_DISPATCH_SQL.contains(&format!("'{label}'")),
                "{label}"
            );
        }
        assert!(!settlement_leg_is_open_to_dispatch("complete", None));
        assert!(!settlement_leg_is_open_to_dispatch("forfeited", None));
        for state in ["pending", "leased", "retry", "held"] {
            assert!(settlement_leg_is_open_to_dispatch(state, None), "{state}");
        }
        for label in [
            PIPELINE_SETTLEMENT_UNRECONCILED_LABEL,
            PIPELINE_CREDIT_CAP_LABEL,
            PIPELINE_SETTLEMENT_REQUEST_INVALID_LABEL,
            SettlementError::Unavailable.label(),
        ] {
            assert!(
                settlement_leg_is_open_to_dispatch("failed", Some(label)),
                "{label}"
            );
        }
        assert!(settlement_leg_is_open_to_dispatch("failed", None));
        assert!(SETTLEMENT_LEG_OPEN_TO_DISPATCH_SQL.contains("COALESCE("));
        assert!(SETTLEMENT_LEG_OPEN_TO_DISPATCH_SQL.contains("FALSE"));
    }

    /// The label rule both withdrawal-forfeit branches of
    /// `complete_settle_phase` share (Step 6's top-level inoperable branch,
    /// and the in-pass branch after `InternalCreditResult::Inoperable`).
    /// Only a dispatched leg of an external instrument gets flagged
    /// `settlement_unreconciled` for an operator to reconcile by hand; an
    /// undispatched leg, a Trace Credit leg, and a leg whose own label
    /// already says no effect happened all keep `submission_inoperable`.
    #[test]
    fn withdrawal_forfeit_flags_a_dispatched_external_leg_for_reconciliation() {
        fn settlement(
            instrument_id: &str,
            dispatched_at: Option<DateTime<Utc>>,
            last_error_label: Option<&str>,
        ) -> PipelineSettlementRecord {
            PipelineSettlementRecord {
                tenant_id: "tenant-a".to_string(),
                run_id: Uuid::new_v4(),
                instrument_id: instrument_id.to_string(),
                atomic_units: AtomicUnits::from_raw(1),
                operation_ref_hash: "sha256:test".to_string(),
                result_ref_hash: None,
                external_receipt_hash: None,
                operation_state: "retry".to_string(),
                credit_event_id: None,
                settlement_batch_id: None,
                payout_rail: "none".to_string(),
                payout_state: "none".to_string(),
                attempt_count: 1,
                last_error_label: last_error_label.map(str::to_string),
                dispatched_at,
            }
        }

        let now = Utc::now();
        // A dispatched external leg is flagged for reconciliation, whether
        // or not it already carries a label.
        assert_eq!(
            withdrawal_forfeit_label(&settlement("storage_rebate", Some(now), None)),
            PIPELINE_SETTLEMENT_UNRECONCILED_LABEL
        );
        assert_eq!(
            withdrawal_forfeit_label(&settlement(
                "storage_rebate",
                Some(now),
                Some(PIPELINE_SETTLEMENT_UNRECONCILED_LABEL)
            )),
            PIPELINE_SETTLEMENT_UNRECONCILED_LABEL
        );
        // `settlement_result_mismatch` is ambiguous, not "no effect
        // happened", so a dispatched leg with it still gets flagged.
        assert_eq!(
            withdrawal_forfeit_label(&settlement(
                "storage_rebate",
                Some(now),
                Some(PIPELINE_SETTLEMENT_RESULT_MISMATCH_LABEL)
            )),
            PIPELINE_SETTLEMENT_UNRECONCILED_LABEL
        );
        // An undispatched external leg had no effect.
        assert_eq!(
            withdrawal_forfeit_label(&settlement("storage_rebate", None, None)),
            PIPELINE_SUBMISSION_INOPERABLE_LABEL
        );
        // A Trace Credit leg pays only through its ledger row, dispatched or
        // not.
        assert_eq!(
            withdrawal_forfeit_label(&settlement(
                InstrumentId::trace_credit().as_str(),
                Some(now),
                None
            )),
            PIPELINE_SUBMISSION_INOPERABLE_LABEL
        );
        // A leg whose own label already says no effect happened keeps
        // `submission_inoperable` rather than `settlement_unreconciled`,
        // even dispatched.
        for label in [
            SettlementError::Conflict.label(),
            SettlementError::Rejected.label(),
        ] {
            assert_eq!(
                withdrawal_forfeit_label(&settlement("storage_rebate", Some(now), Some(label))),
                PIPELINE_SUBMISSION_INOPERABLE_LABEL,
                "{label}"
            );
        }

        assert_eq!(
            withdrawal_forfeited_settlement_update(&settlement("storage_rebate", Some(now), None))
                .operation_state,
            "forfeited"
        );
    }

    /// A transient database failure -- the pool
    /// could not give a connection, or a SQLSTATE in class 08, 40001,
    /// 40P01, 57P01 to 57P03, or 53300 -- is `database_unavailable`, found by
    /// downcast however service code wrapped it. Any other database error
    /// (a constraint, a raise, a query cancel in class 57, a full disk in
    /// class 53) is not.
    #[test]
    fn transient_database_errors_are_classified_by_sqlstate_and_pool() {
        use tokio_postgres::error::SqlState;

        for code in [
            SqlState::CONNECTION_EXCEPTION,
            SqlState::CONNECTION_DOES_NOT_EXIST,
            SqlState::CONNECTION_FAILURE,
            SqlState::SQLCLIENT_UNABLE_TO_ESTABLISH_SQLCONNECTION,
            SqlState::SQLSERVER_REJECTED_ESTABLISHMENT_OF_SQLCONNECTION,
            SqlState::TRANSACTION_RESOLUTION_UNKNOWN,
            SqlState::PROTOCOL_VIOLATION,
            SqlState::from_code("08ZZZ"),
            SqlState::T_R_SERIALIZATION_FAILURE,
            SqlState::T_R_DEADLOCK_DETECTED,
            SqlState::ADMIN_SHUTDOWN,
            SqlState::CRASH_SHUTDOWN,
            SqlState::CANNOT_CONNECT_NOW,
            SqlState::TOO_MANY_CONNECTIONS,
        ] {
            assert!(is_transient_sqlstate(&code), "{} is transient", code.code());
        }
        for code in [
            SqlState::UNIQUE_VIOLATION,
            SqlState::CHECK_VIOLATION,
            SqlState::FOREIGN_KEY_VIOLATION,
            SqlState::RAISE_EXCEPTION,
            SqlState::INSUFFICIENT_PRIVILEGE,
            SqlState::T_R_INTEGRITY_CONSTRAINT_VIOLATION,
            SqlState::QUERY_CANCELED,
            SqlState::DISK_FULL,
        ] {
            assert!(
                !is_transient_sqlstate(&code),
                "{} is not transient",
                code.code()
            );
        }

        let transient: Vec<anyhow::Error> = vec![
            DatabaseError::Pool("pool exhausted".to_string()).into(),
            DatabaseError::PoolRuntime(deadpool_postgres::PoolError::Closed).into(),
            deadpool_postgres::PoolError::Closed.into(),
            anyhow::Error::from(DatabaseError::Pool("pool exhausted".to_string()))
                .context("settle leg update"),
        ];
        for error in &transient {
            assert!(is_transient_database_error(error), "{error:#}");
        }
        let permanent: Vec<anyhow::Error> = vec![
            stale_lease_error().into(),
            settlement_leg_not_open_error().into(),
            DatabaseError::Constraint("phase does not match run transition".to_string()).into(),
            DatabaseError::Serialization("row decode failed".to_string()).into(),
            DatabaseError::Query("statement timeout".to_string()).into(),
            DatabaseError::NotFound {
                entity: "pipeline_run".to_string(),
                id: "run".to_string(),
            }
            .into(),
            anyhow::anyhow!(PIPELINE_INDEX_CONFLICT_LABEL),
            anyhow::anyhow!(PIPELINE_DATABASE_UNAVAILABLE_LABEL),
        ];
        for error in &permanent {
            assert!(!is_transient_database_error(error), "{error:#}");
        }
    }

    /// A driver error with no SQLSTATE whose cause
    /// is I/O is transient, whether service code raised it bare or the
    /// store wrapped it as `DatabaseError::Postgres`. Nothing listens on
    /// port 1, so the connection is refused locally.
    #[tokio::test]
    async fn a_refused_connection_is_a_transient_database_error() {
        async fn refused() -> tokio_postgres::Error {
            tokio_postgres::connect(
                "host=127.0.0.1 port=1 user=probe connect_timeout=5",
                tokio_postgres::NoTls,
            )
            .await
            .err()
            .expect("nothing listens on port 1")
        }
        let bare = refused().await;
        assert!(bare.code().is_none());
        assert!(is_transient_database_error(&bare.into()));
        let wrapped: anyhow::Error = DatabaseError::Postgres(refused().await).into();
        assert!(is_transient_database_error(&wrapped));
    }
}
