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
    BundlePackage, IndexMembershipDecision, InstrumentAward, InstrumentAwards, InstrumentId,
    InstrumentSettlement, InstrumentSettlementProgress, Microcredits, PIPELINE_OUTCOME_SCHEMA_ID,
    PIPELINE_OUTCOME_SCHEMA_VERSION, Phase, PhaseResult, PolicyError, PrivacyRisk, ReasonCode,
    ReviewDecision, ReviewEvaluation, ReviewEvidence, ReviewInput, SchemaRef, ScoreDecision,
    ScoreEvaluation, ScoreEvidence, ScoreInput, SealedIndexCommand, SettleDecision,
    SettleEvaluation, SettleEvidence, SettleInput, TenantStorageRef, UnverifiedScoreDecision,
};
use trace_commons_gate_api::{
    IdentifiedEmbedder, IdentifiedIndexReader, IdentifiedIndexWriter, IdentifiedPerplexityScorer,
    IndexWriteError, SettlementError, SettlementReceipt, SettlementRequest,
};
use trace_commons_protocol::trace_contribution::{
    ResidualPiiRisk, ResidualRiskCondition, TraceContributionEnvelope, retention_policy_for_trace,
};
use uuid::Uuid;

use crate::db::postgres::PgBackend;
use crate::db::{insert_credit_settlement_batch_on_tx, list_trace_credit_holds_on_tx};
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
    PIPELINE_CREDIT_REASON, PIPELINE_SETTLEMENT_POLICY_VERSION, SettlementAdapterRegistry,
    credit_account_hash, issuer_approval_hash, microcredits_to_settled_i64,
    pipeline_credit_event_id, pipeline_ledger_source_key, pipeline_settlement_batch_id,
    source_list_hash,
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
pub const PIPELINE_TOMBSTONE_LABEL: &str = "content_tombstoned";
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
    /// the run does not retry hourly forever while it waits. The route that
    /// moves it back to `Pending` once an assessment lands is not in this
    /// code.
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

        let submission_row = tx
            .query_opt(
                "SELECT status, revoked_at, purged_at, withdrawn_at, expires_at
                   FROM trace_submissions
                  WHERE tenant_id = $1 AND submission_id = $2
                  FOR UPDATE",
                &[&run.tenant_id, &run.submission_id],
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
        let withdrawn = tx
            .query_opt(
                "SELECT 1 FROM trace_withdrawals WHERE tenant_id = $1 AND submission_id = $2",
                &[&run.tenant_id, &run.submission_id],
            )
            .await?
            .is_some();
        if !operable || withdrawn {
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
                DatabaseError::Constraint(PIPELINE_SETTLEMENT_ADAPTER_MISSING_LABEL.to_string())
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
    /// its own; the route that moves it back to `pending` once an
    /// assessment lands is not in this code. A parked run whose submission
    /// is later withdrawn, expired, or purged stays parked -- nothing here
    /// moves it -- so that route must itself handle an inoperable
    /// submission when it runs.
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
/// dependency check runs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineDependencyQualification {
    pub scorer: bool,
    pub embedder: bool,
    pub index_reader: bool,
    pub index_writer: bool,
    pub settlement_adapters: BTreeMap<String, bool>,
    pub authority: bool,
    pub privacy: bool,
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
            object_store_name: self.object_store_name,
            lease_config: self.lease_config,
            crash_point: self.crash_point,
            crash_pending: AtomicBool::new(self.crash_point.is_some()),
            score_evaluations: AtomicUsize::new(0),
            settle_evaluations: AtomicUsize::new(0),
            authority: self.authority,
            privacy: self.privacy,
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
                    &pipeline_attempt_object_id("index-command", run.run_id, lease_token),
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
                    &pipeline_attempt_object_id("score-neighbors", run.run_id, lease_token),
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
                result.decision.awards(),
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
                DatabaseError::Constraint(label)
                    if label == PIPELINE_SETTLEMENT_ADAPTER_MISSING_LABEL =>
                {
                    anyhow::anyhow!(PIPELINE_SETTLEMENT_ADAPTER_MISSING_LABEL)
                }
                _ => anyhow::Error::from(error),
            })?;
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
                let guard = self.submission_guard(&run).await?;
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
        // guard is read fresh here too, directly before this dispatch --
        // its own external effect -- rather than trusting whatever Step 3
        // saw, since a withdrawal can land in the gap between them.
        if run.index_write_state == "pending" {
            let guard = self.submission_guard(&run).await?;
            if !guard.operable {
                run = self.store.mark_index_write_state(&run, "cancelled").await?;
            } else {
                self.ensure_live_lease(&run).await?;
                let command = self
                    .load_index_command(&run, &score_evidence)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("index_command_invalid"))?;
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
                            // itself rather than returning an `Err`.
                            return Ok(self
                                .store
                                .mark_transient_retry(&run, PIPELINE_INDEX_UNAVAILABLE_LABEL)
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
        // in `trace-commons-ingest.rs` excludes.
        let ledger_event_type = match trace_credit_event {
            PipelineTraceCreditEvent::PipelineScore => TraceCreditEventType::Accepted,
            PipelineTraceCreditEvent::NoveltyUtility => TraceCreditEventType::NoveltyUtility,
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
        tx.execute(
            "INSERT INTO trace_credit_ledger (
                tenant_id, credit_event_id, submission_id, trace_id, credit_account_ref,
                event_type, points_delta, reason, external_ref, actor_principal_ref,
                actor_role, settlement_state, pipeline_run_id, score_outcome_id, instrument_id
             ) VALUES (
                $1,$2,$3,$4,$5,$12,$6,$7,$8,$5,'pipeline_worker','pending',$9,$10,$11
             )
             ON CONFLICT (tenant_id, credit_event_id) DO NOTHING",
            &[
                &run.tenant_id,
                &event_id,
                &run.submission_id,
                &run.trace_id,
                &account_ref,
                &amount.to_credit_decimal(),
                &PIPELINE_CREDIT_REASON,
                &external_ref,
                &run.run_id,
                &score_outcome_id,
                &settlement.instrument_id,
                &ledger_event_type_label,
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

    /// Composes the account's pending pipeline credit events for
    /// `instrument_id` into one batch, writes it finalized, sets its
    /// `instrument_id`, and marks those events final -- all inside the
    /// caller's credit transaction, under the per-account advisory lock the
    /// caller holds. `event_id` (the run's own event) must be among them.
    ///
    /// Ruling T5-1: only pending events whose `event_type` is `event_type`
    /// (the caller's own leg's ledger event type -- `PipelineScore`'s
    /// `Accepted`, computed once by the caller with the same enum-to-string
    /// path the ledger insert used) are eligible. Without this predicate, a
    /// still-`pending` `NoveltyUtility` row for the same account and
    /// instrument would be swept into this batch too, even though `main`
    /// never pays that event (Review Focus 3).
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
            issuer_approval_evidence_hash: Some(issuer_approval_hash(&list_hash)),
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

#[cfg(test)]
mod tests {
    use super::*;

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
