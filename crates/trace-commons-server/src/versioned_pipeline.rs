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
    PolicyError, PolicyRef, PrivacyRisk, ReasonCode, ReviewDecision, ReviewEvaluation,
    ReviewEvidence, ReviewInput, ReviewOutput, ReviewRecommendation, SchemaRef, ScoreDecision,
    ScoreEvaluation, ScoreEvidence, ScoreHoldReview, ScoreInput, ScoreOutput, SealedIndexCommand,
    SettleDecision, SettleEvaluation, SettleEvidence, SettleInput, TenantStorageRef,
    UnverifiedScoreDecision,
};
use trace_commons_gate_api::{
    IdentifiedEmbedder, IdentifiedIndexReader, IdentifiedIndexWriter, IdentifiedPerplexityScorer,
    IndexUpsertResult, IndexWriteError, SettlementError, SettlementReceipt, SettlementRequest,
};
use trace_commons_protocol::trace_contribution::{
    ConsentScope, ResidualPiiRisk, ResidualRiskCondition, TraceAllowedUse,
    TraceContributionEnvelope, retention_policy_for_trace,
};
use uuid::Uuid;

use crate::db::postgres::PgBackend;
use crate::db::{
    insert_credit_settlement_batch_on_tx, record_source_submission_withdrawal_on_tx,
    withdraw_source_session_on_tx,
};
use crate::error::DatabaseError;
use crate::trace_artifact_store::{
    EncryptedTraceArtifactReceipt, TraceArtifactKind, TraceArtifactStore,
};
use crate::trace_corpus_storage::{
    PIPELINE_PROCESSING_FAILED_STATUS_REASON, TraceCorpusStatus, TraceCorpusStore,
    TraceCreditAccountSettlementLineItem, TraceCreditEventType, TraceCreditSettlementBatchStatus,
    TraceCreditSettlementBatchWrite, TraceCreditSettlementNearStatus, TraceObjectArtifactKind,
    TraceObjectRefWrite, TraceSubmissionWrite, TraceWitnessProvenanceClass,
    safe_residual_risk_basis_labels,
};
use crate::versioned_pipeline_activation::RoutingState;
use crate::versioned_pipeline_authority::{
    PIPELINE_AUTHORITY_CONTROL_MISSING_LABEL, PIPELINE_AUTHORITY_READ_FAILED_LABEL,
    PIPELINE_PRIVACY_CLASSIFICATION_FAILED_LABEL, PIPELINE_PRIVACY_CONTROL_MISSING_LABEL,
    PIPELINE_PRIVACY_RESCRUB_FAILED_LABEL, PipelineAuthorityProvider, PipelinePrivacyBoundary,
};
use crate::versioned_pipeline_bundle::{
    MinimalPolicyBundle, PIPELINE_BUNDLE_INVALID_LABEL, PIPELINE_DEPENDENCY_MISSING_LABEL,
    PIPELINE_REVIEW_ASSESSMENT_REQUIRED_LABEL, PipelineTraceCreditEvent, dependency_content_hash,
    pipeline_operation_ref, pipeline_result_ref, registered_score_factory,
};
use crate::versioned_pipeline_compat::UnappliedIndexCommandsReader;
use crate::versioned_pipeline_credit::{
    DryRunNearPayoutAdapter, NearPayoutAdapter, NearPayoutConfirmation, PIPELINE_CREDIT_ACTOR_ROLE,
    PIPELINE_CREDIT_REASON, PIPELINE_NOVELTY_UTILITY_ACTOR_ROLE,
    PIPELINE_SETTLEMENT_POLICY_VERSION, SettlementAdapterRegistry, credit_account_hash,
    disabled_near_call, microcredits_to_settled_i64, payout_state_label, pipeline_credit_event_id,
    pipeline_ledger_source_key, pipeline_near_outbox_line_id, pipeline_novelty_utility_reason,
    pipeline_settlement_batch_id, source_list_hash,
};
use crate::versioned_pipeline_qualification::package_digests;
use crate::versioned_pipeline_score_registry::{
    SCORE_POLICY_MISSING_LABEL, ScorePolicyRegistry, is_built_in_score_implementation,
};

/// The lifecycle half of every pipeline operability check, over the
/// `trace_submissions` row aliased `s` (Zaki review 1, round 2,
/// simplification: one definition, where there were five that disagreed on
/// `withdrawn_at`): not revoked, purged, or withdrawn (`withdrawn_at`, and no
/// `trace_withdrawals` row), and not expired on the database clock. Each
/// check adds the status it needs (`accepted_submission_sql!`,
/// `reviewable_submission_sql!`). A macro, so every query stays one
/// `&'static str`.
macro_rules! live_submission_sql {
    () => {
        "s.revoked_at IS NULL AND s.purged_at IS NULL AND s.withdrawn_at IS NULL
         AND (s.expires_at IS NULL OR s.expires_at > NOW())
         AND NOT EXISTS (
             SELECT 1 FROM trace_withdrawals w
              WHERE w.tenant_id = s.tenant_id AND w.submission_id = s.submission_id
         )"
    };
}
pub(crate) use live_submission_sql;

/// A live submission (`live_submission_sql!`) Review approved: what Score,
/// Settle, and an export need (the submission guard).
macro_rules! accepted_submission_sql {
    () => {
        concat!("s.status = 'accepted' AND ", live_submission_sql!())
    };
}
pub(crate) use accepted_submission_sql;

/// A live submission (`live_submission_sql!`) Review has not decided yet:
/// what a review claim, an assessment, and the review queue need.
macro_rules! reviewable_submission_sql {
    () => {
        concat!(
            "s.status IN ('received', 'quarantined') AND ",
            live_submission_sql!()
        )
    };
}

/// The submission, aliased `s`, of a parked run, aliased `p`, is still one
/// the hold applies to: a submission Review has not decided yet
/// (`reviewable_submission_sql!`) for a run parked at Review, and an
/// accepted one (`accepted_submission_sql!`, what Score needs) for a run its
/// Score policy held for operator review at Score. A parked run whose
/// submission fails this is released (`release_inoperable_parked_runs`) and
/// left out of the review queue.
macro_rules! parked_run_submission_sql {
    () => {
        concat!(
            "((p.next_phase = 'score' AND ",
            accepted_submission_sql!(),
            ") OR (p.next_phase IS DISTINCT FROM 'score' AND ",
            reviewable_submission_sql!(),
            "))"
        )
    };
}

/// A run, aliased `p`, that waits for a human review: at Review, quarantined
/// by Admission or escalated by the Review-start privacy pass, and idle
/// (`pending`, `retry`, or `awaiting_review`). A run that needs the privacy
/// pass (`privacy_pass_required`, every run received from V117 on) waits
/// for review only once its pass is recorded (critique 1 (1)), so no human
/// assesses content the classifier has not seen; a run received before V117
/// keeps the earlier rule. The review queue also leaves out a run that has
/// an assessment (unless the pass escalated it after that assessment,
/// decision P9); a claim checks that in a statement of its own, after it
/// holds the run's lock (Zaki review 1, round 2, finding 8).
macro_rules! run_waiting_for_review_sql {
    () => {
        "p.next_phase = 'review'
         AND (p.admission_decision = 'quarantine' OR p.privacy_pass_outcome = 'escalated')
         AND (NOT p.privacy_pass_required OR p.privacy_pass_object_ref_id IS NOT NULL)
         AND p.state IN ('pending', 'retry', 'awaiting_review')"
    };
}

/// A run, aliased `p`, that its Score policy held for operator review
/// (`hold_score_for_review_on_tx`): parked in `awaiting_review` at Score.
/// `claim_review` and `record_review_assessment` act on it as on a run
/// waiting at Review, with its own assessment (`phase` `score`, V130).
macro_rules! run_held_at_score_sql {
    () => {
        "p.next_phase = 'score' AND p.state = 'awaiting_review'"
    };
}

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
/// Settle refuses to write a compatibility run's `trace_gate_decisions` row
/// (spec 2026-10-08, Slice C) when the run has no committed Score outcome or
/// its evidence lacks a field the row needs: the commit writes nothing.
pub const PIPELINE_GATE_DECISION_EVIDENCE_INCOMPLETE_LABEL: &str =
    "pipeline_gate_decision_evidence_incomplete";
/// A Score policy asked for a hold again on an input that carries the
/// operator's approval of its earlier hold
/// (`ScoreInput::operator_review`). The run fails closed under this label
/// rather than going back to the review queue, so a hold cannot loop.
pub const PIPELINE_SCORE_HOLD_REPEATED_LABEL: &str = "pipeline_score_hold_repeated";
/// A compatibility Score's embedder refused the trace's own text (for
/// example a chunk with no text in it, only whitespace). The same input
/// fails the same way on every attempt, so the run fails terminally under
/// this label on its first Score instead of retrying. An embedder outage
/// stays the transient `embedder_unavailable`.
pub const PIPELINE_SCORE_INPUT_UNEMBEDDABLE_LABEL: &str = "pipeline_score_input_unembeddable";
/// Settle refuses to commit when the submission already has a pipeline gate
/// decision row written by another run (V116 allows one per submission).
pub const PIPELINE_GATE_DECISION_CONFLICT_LABEL: &str = "pipeline_gate_decision_conflict";

/// #1313 (b): the label a failure path records when it gives a finishable
/// Settle run (`finishable_settle_run_sql!`) its one extra attempt instead
/// of failing it.
pub const PIPELINE_SETTLE_FINISH_GRANTED_LABEL: &str = "settle_finish_granted";

/// #1313: the reason label of the audit event for a run held for an
/// operator (`PipelineRunState::AwaitingOperator`).
pub const PIPELINE_SETTLE_HELD_FOR_OPERATOR_LABEL: &str = "pipeline_settle_held_for_operator";

/// #1313: the label an operator's `Fail` of a held run records.
pub const PIPELINE_OPERATOR_FAILED_HELD_RUN_LABEL: &str = "operator_failed_held_run";

/// #1313: the label an operator's `Release` of a held run records.
pub const PIPELINE_OPERATOR_RELEASED_HELD_RUN_LABEL: &str = "operator_released_held_run";

/// #1313 (b): a run, aliased `p`, whose Settle can finish with no further
/// effect: its selection is persisted, its index write is done (not
/// `pending`, and not `failed`, whose partly written entries must leave the
/// index under N-7), and every settlement leg is `complete` or `forfeited`
/// (a `failed` leg is resolved only by the failure path). A run with no leg
/// qualifies. Its one extra attempt reuses the selection, skips the index
/// write and every terminal leg, and only commits.
macro_rules! settle_run_is_finishable_sql {
    () => {
        "p.next_phase = 'settle'
         AND p.settle_selection IS NOT NULL
         AND p.index_write_state IN ('none', 'complete', 'cancelled')
         AND NOT EXISTS (
             SELECT 1 FROM pipeline_run_settlements fs
              WHERE fs.tenant_id = p.tenant_id AND fs.run_id = p.run_id
                AND fs.operation_state NOT IN ('complete', 'forfeited'))"
    };
}

/// #1313: a run, aliased `p`, with a settlement leg that paid. A leg
/// completes only with a receipt (`result_ref_hash`), a Trace Credit ledger
/// event, or an external receipt; any of them means the instrument was
/// paid, so N-7 removing the run's revision would leave a payment for a
/// revision that is no longer indexed.
macro_rules! settle_run_has_paid_leg_sql {
    () => {
        "EXISTS (
             SELECT 1 FROM pipeline_run_settlements paid
              WHERE paid.tenant_id = p.tenant_id AND paid.run_id = p.run_id
                AND paid.operation_state = 'complete'
                AND (paid.credit_event_id IS NOT NULL
                     OR paid.result_ref_hash IS NOT NULL
                     OR paid.external_receipt_hash IS NOT NULL))"
    };
}

/// `settle_run_is_finishable_sql!` for a run that has not had its extra
/// attempt yet (V128 `settle_finish_granted_at`): the attempt is given once.
macro_rules! finishable_settle_run_sql {
    () => {
        concat!(
            settle_run_is_finishable_sql!(),
            " AND p.settle_finish_granted_at IS NULL"
        )
    };
}

/// What a pipeline gate decision row's `embedding_evidence_hash` hashes when
/// the Score sealed no index command (spec C-D3).
pub const PIPELINE_NO_INDEX_COMMAND: &str = "pipeline_no_index_command";
/// The schema string `pipeline_score_outcome_hash` binds.
pub const PIPELINE_SCORE_OUTCOME_HASH_SCHEMA: &str = "trace_commons.pipeline_score_outcome_hash.v1";

/// The decision id of a run's `trace_gate_decisions` row (spec C-D2): a
/// UUIDv5 over the tenant and the run, so the row is idempotent per run.
pub fn pipeline_gate_decision_id(tenant_id: &str, run_id: Uuid) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("trace_commons.pipeline_gate_decision.v1\n{tenant_id}\n{run_id}").as_bytes(),
    )
}

/// The content hash of a committed Score outcome, the pipeline row's
/// `attestation_chain_hash` (spec C-D3): SHA-256 over the canonical JSON of
/// its decision, evidence and evaluation under a fixed schema string.
pub fn pipeline_score_outcome_hash(
    decision: &serde_json::Value,
    evidence: &serde_json::Value,
    evaluation: &serde_json::Value,
) -> Result<String, serde_json::Error> {
    let bytes = trace_commons_protocol::canonical_json::to_canonical_vec(&serde_json::json!({
        "schema": PIPELINE_SCORE_OUTCOME_HASH_SCHEMA,
        "decision": decision,
        "evidence": evidence,
        "evaluation": evaluation,
    }))?;
    Ok(sha256_prefixed(&bytes))
}
/// `intervene_policy` accepts `suspend` and `resume`. `terminate` has a
/// database state (V111) but no behavior: no specification says what happens
/// to a run bound to a terminated policy (CMP-003), so it is refused and
/// writes nothing.
pub const PIPELINE_POLICY_INTERVENTION_NOT_SUPPORTED_LABEL: &str =
    "policy_intervention_not_supported";
/// An unknown action, or an actor or reason that fails the operator-action
/// rule (`validate_actor`).
pub const PIPELINE_POLICY_INTERVENTION_INVALID_LABEL: &str = "policy_intervention_invalid";
/// An intervention that waited `PIPELINE_ADMIN_LOCK_TIMEOUT` for its policy
/// row and did not get it (a phase commit, a payout, or another intervention
/// holds it): nothing is written, and the operator repeats the request.
pub const PIPELINE_POLICY_INTERVENTION_BUSY_LABEL: &str = "policy_intervention_busy";
/// How long an operator's action waits for a lock (final fix wave G15): a
/// routing change (`PipelineActivationStore`) for the tenant's routing lock
/// and the rows it locks after it, and a policy intervention for its policy
/// row. Set with `SET LOCAL lock_timeout` in the action's own transaction,
/// so it bounds no other transaction: a receipt and a phase commit keep no
/// lock timeout, and never fail because an operator's action waits.
pub(crate) const PIPELINE_ADMIN_LOCK_TIMEOUT_SQL: &str = "SET LOCAL lock_timeout = '5s'";
/// A `suspend` of a policy that is not runnable, or a `resume` of one that is.
pub const PIPELINE_POLICY_INTERVENTION_NO_TRANSITION_LABEL: &str =
    "policy_intervention_no_transition";
pub const PIPELINE_INDEX_UNAVAILABLE_LABEL: &str = "index_unavailable";
/// Settle's read of the stored index command failed in the artifact store
/// call, an integrity failure the store reports included (a missing object,
/// a wrong key). The attempt is charged and the next one waits an hour, as
/// for `artifact_integrity_failed`. A command that the store read and that
/// is wrong stays `index_command_invalid`.
pub const PIPELINE_INDEX_COMMAND_UNREADABLE_LABEL: &str = "index_command_unreadable";
/// An index rebuild whose committed fence (V113,
/// `PgPipelineStore::set_index_rebuild_fence`) could not be written before a
/// run's writes: the rebuild stops before that run's first write
/// (`PipelineService::rebuild_index_run`).
pub const PIPELINE_INDEX_REBUILD_FENCE_UNAVAILABLE_LABEL: &str = "index_rebuild_fence_unavailable";
/// A phase commit, or a receipt's final transaction, found no `staged`
/// attempt row naming the object it is about to record (or the row names
/// another object or hash). Only an out-of-band change or a defect gets
/// there: the commit is refused, and a phase records it as a charged retry
/// under this label (wave 2, fix round 1; review I2, I3).
pub const PIPELINE_ATTEMPT_ARTIFACT_MISSING_LABEL: &str = "pipeline_attempt_artifact_missing";
/// Safe label of a `record_privacy_pass` refused because the run already
/// has a pass recorded (`privacy_pass_object_ref_id IS NOT NULL`): a pass is
/// recorded once per run and never re-run (decision P7). The refused call
/// writes nothing.
pub const PIPELINE_PRIVACY_PASS_ALREADY_RECORDED_LABEL: &str = "privacy_pass_already_recorded";
/// Safe label of a Review approval refused because the run needs a privacy
/// pass (`privacy_pass_required`, every run received from V117 on) and has
/// none recorded. `commit_review` returns it instead of the raw CHECK
/// violation (`pipeline_runs_privacy_pass_before_approval`), which stays as
/// the backstop for a binary without the predicate. The refused commit
/// writes nothing.
pub const PIPELINE_PRIVACY_PASS_MISSING_LABEL: &str = "privacy_pass_missing";
/// Safe label of a run the Review-start privacy pass escalated (its risk is
/// High and above the receipt-time risk, decision P3 as amended 2026-10-10,
/// `privacy_pass_outcome`), held for a human whatever
/// Admission decided: the run's parking label, the review queue's hold
/// reason, and a reason an approving assessment must resolve. Distinct from
/// Admission's `privacy_review_required` (Medium) and
/// `privacy_risk_high_review_required` (High).
pub const PIPELINE_PRIVACY_PASS_REVIEW_REQUIRED_LABEL: &str = "privacy_pass_review_required";
/// Rule id of a Review rejection the server commits itself: a reviewer's
/// Reject of an Admission-admitted run the privacy pass escalated (Q2).
/// `MinimalReviewPolicy` ignores a human assessment when Admission
/// admitted, so the policy would approve such a run; the server never calls
/// it and records the rejection under this rule id instead.
pub const PIPELINE_PRIVACY_PASS_REJECTED_RULE_ID: &str = "privacy_pass_human_review_rejected_v1";
/// The default ceiling on one privacy pass classifier call. A call that has
/// not returned by then is a classifier failure
/// (`privacy_classification_failed`). An operator replaces it with
/// `PipelineLeaseConfig::with_privacy_pass_max_timeout`
/// (`TRACE_COMMONS_PIPELINE_PRIVACY_PASS_MAX_TIMEOUT_SECONDS` in ingest).
const PIPELINE_PRIVACY_PASS_MAX_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(900);
/// The bounds an operator-set privacy pass ceiling must fall in: at least the
/// commit margin's worth of classifier time, at most the two hours a lease
/// may last.
const PIPELINE_PRIVACY_PASS_MAX_TIMEOUT_MIN: std::time::Duration =
    std::time::Duration::from_secs(60);
const PIPELINE_PRIVACY_PASS_MAX_TIMEOUT_MAX: std::time::Duration =
    std::time::Duration::from_secs(7200);
/// Safe label for a privacy-pass ceiling outside
/// `PIPELINE_PRIVACY_PASS_MAX_TIMEOUT_MIN..=PIPELINE_PRIVACY_PASS_MAX_TIMEOUT_MAX`.
pub const PIPELINE_PRIVACY_PASS_TIMEOUT_INVALID_LABEL: &str =
    "pipeline_privacy_pass_timeout_invalid";
/// What a privacy pass keeps back from the Review lease cap for the work
/// around its classifier call: loading the source, storing the output,
/// the record transaction and the Review commit (decision P8).
const PIPELINE_PRIVACY_PASS_COMMIT_MARGIN: std::time::Duration = std::time::Duration::from_secs(60);
/// The base of the charged retry backoff under
/// `privacy_classification_failed`: `30 s x 2^(attempt_count - 1)`,
/// legacy's PII backstop shape (`TRACE_PII_BACKSTOP_DEFAULT_BACKOFF_BASE_SECONDS`).
const PIPELINE_PRIVACY_RETRY_BASE_SECONDS: i64 = 30;

/// The bound on one privacy pass classifier call (decision P8). For a
/// boundary that runs a prose-PII classifier it is
/// `min(ceiling, review_lease x PIPELINE_LEASE_RENEWAL_CAP_FACTOR -
/// PIPELINE_PRIVACY_PASS_COMMIT_MARGIN)`, so the pass ends inside the
/// Review lease cap with time left to commit; a Review lease that leaves
/// the classifier less than the margin (under 30 s) is refused with
/// `PIPELINE_LEASE_CONFIG_INVALID_LABEL`. A boundary that classifies
/// nothing (a pass-through or a test double, whose call is local) is
/// bounded by `ceiling` alone, so a short test lease never times it out.
/// `ceiling` is `PIPELINE_PRIVACY_PASS_MAX_TIMEOUT` outside tests.
fn privacy_pass_timeout_for(
    review_lease: Duration,
    classifies_prose_pii: bool,
    ceiling: std::time::Duration,
) -> anyhow::Result<std::time::Duration> {
    if !classifies_prose_pii {
        return Ok(ceiling);
    }
    let cap = (review_lease * PIPELINE_LEASE_RENEWAL_CAP_FACTOR)
        .to_std()
        .unwrap_or_default();
    let available = cap.saturating_sub(PIPELINE_PRIVACY_PASS_COMMIT_MARGIN);
    anyhow::ensure!(
        available >= PIPELINE_PRIVACY_PASS_COMMIT_MARGIN,
        PIPELINE_LEASE_CONFIG_INVALID_LABEL
    );
    Ok(available.min(ceiling))
}
/// Safe label of a receipt for a submission id that the legacy path owns
/// (`PipelineReceiptResult::LegacyOwned`): the ownership row names the legacy
/// path, or a legacy submission row holds the id. A conflict with a legacy
/// owner at the receipt's own inserts (`insert_receipt_records`) carries this
/// label to `commit_receipt_attempt`, which turns it into the result. A
/// conflict with a pipeline owner does not: it is
/// `PIPELINE_SUBMISSION_BOUND_MESSAGE`, an error.
pub const PIPELINE_LEGACY_RECEIPT_OWNED_LABEL: &str = "legacy_receipt_owned";
/// `main`'s error for a receipt whose submission id is already bound to
/// another receipt. `insert_receipt_records` returns it when a pipeline run
/// owns the id (a second idempotency key for the same submission id): the id
/// is not the legacy path's, so the receipt is an error, not `LegacyOwned`.
const PIPELINE_SUBMISSION_BOUND_MESSAGE: &str =
    "submission identity is already bound to another receipt";
/// A bundle whose own configuration is not qualifiable
/// (`PipelineBundleQualification::configuration_qualifiable`).
pub const PIPELINE_BUNDLE_CONFIGURATION_NOT_QUALIFIABLE_LABEL: &str =
    "bundle_configuration_not_qualifiable";
/// A bundle whose Score policy a registry supplies, from a factory that is
/// not production-qualified, or that names a Score policy the service holds
/// no factory for (`PipelineBundleQualification::score_policy`).
pub const PIPELINE_SCORE_POLICY_NOT_PRODUCTION_LABEL: &str = "runtime_score_policy_not_production";
/// Safe label of an object-store call in the run path that failed (an
/// outage, not a decode or hash mismatch): the uncharged suspension of
/// ruling FR3 (multi-lens review L2-2).
pub const PIPELINE_ARTIFACT_STORE_UNAVAILABLE_LABEL: &str = "artifact_store_unavailable";
/// Safe label of an object-store call in the run path that reached the
/// object and found it missing or not what its receipt names (a
/// `TraceArtifactIntegrityError`: not found, a hash or ref mismatch, a decode
/// or decrypt failure). Retrying cannot fix it, so it is charged and the
/// phase's attempt budget ends the run with it (Zaki's approval of #1143,
/// ZA-2).
pub const PIPELINE_ARTIFACT_INTEGRITY_FAILED_LABEL: &str = "artifact_integrity_failed";
/// Safe label of a compatibility Score released, uncharged, because its
/// tenant's Score lock was held by another Score (multi-lens review L3-2).
/// The run is due again after `PIPELINE_SCORE_LOCK_BUSY_RETRY_MILLISECONDS`,
/// and the worker ends that tenant's batch for the pass.
pub const PIPELINE_SCORE_LOCK_BUSY_LABEL: &str = "score_lock_busy";
/// The fixed delay before a `score_lock_busy` run is due again: about one
/// Score, so the lock holder has likely committed, and short, since the run
/// waited for nothing of its own.
pub const PIPELINE_SCORE_LOCK_BUSY_RETRY_MILLISECONDS: i64 = 2_000;
/// The fence margin of Settle's index writes (multi-lens review L4-3). A
/// write stops issuing calls at its lease's end, so a withdrawal that finds
/// a `pending` write a lease may still be making queues the revision's
/// invalidation no earlier than that lease's end plus this margin, which
/// also covers clock skew between replicas, and a dispatch or a rebuilt run
/// past its deadline waits this long for the call in flight before it lets
/// its run go.
///
/// Since issue #1233 the margin is a release bound, not the safety bound:
/// an index call may take any time. A call still running once the margin
/// has passed is awaited by a task of its own, which reopens the revision's
/// invalidation when the call returns
/// (`PgPipelineStore::reopen_index_invalidation_after_late_write`), so an
/// entry that lands after a withdrawal's removal is removed again.
pub const PIPELINE_INDEX_WRITE_FENCE_MARGIN_SECONDS: i64 = 60;
/// The longest Settle's index dispatch holds the run and submission rows
/// (multi-lens review L4-1): past the earlier of its lease's end and this
/// budget, it stops writing, rolls back, and the run waits uncharged as
/// `index_unavailable`.
pub const PIPELINE_INDEX_DISPATCH_BUDGET_SECONDS: i64 = 30;
/// The key of a pipeline NEAR outbox line's stored call that records the
/// settlement mode that submitted it (`dry_run` or `http`); only that mode
/// confirms the line (Zaki review 3, Z3-L2). A line with no key reads as
/// `PIPELINE_NEAR_SUBMISSION_MODE_DEFAULT` (multi-lens review C3).
pub const PIPELINE_NEAR_SUBMISSION_MODE_KEY: &str = "pipeline_submission_mode";
/// The mode a pipeline NEAR outbox line with no
/// `PIPELINE_NEAR_SUBMISSION_MODE_KEY` reads as: code from before the key
/// submitted it. The one definition, for the read of the line and for the
/// confirming `UPDATE`.
const PIPELINE_NEAR_SUBMISSION_MODE_DEFAULT: PipelineNearSettlementMode =
    PipelineNearSettlementMode::Http;
/// Startup refusal labels of `PipelineService::check_tenant_bundles` that
/// the default package's checks do not share: a tenant bundle whose scorer
/// or embedder the service does not hold, one no policy family runs, and
/// one that cannot be read (multi-lens review L5-2).
pub const PIPELINE_TENANT_BUNDLE_DEPENDENCY_MISSING_LABEL: &str =
    "pipeline_tenant_bundle_dependency_missing";
pub const PIPELINE_TENANT_BUNDLE_NOT_RUNNABLE_LABEL: &str = "pipeline_tenant_bundle_not_runnable";
pub const PIPELINE_TENANT_BUNDLE_UNREADABLE_LABEL: &str = "pipeline_tenant_bundle_unreadable";
pub const PIPELINE_INDEX_CONFLICT_LABEL: &str = "index_key_conflict";
pub const PIPELINE_CREDIT_HELD_LABEL: &str = "credit_held";
pub const PIPELINE_CREDIT_CAP_LABEL: &str = "credit_cap_exceeded";
pub const PIPELINE_SUBMISSION_INOPERABLE_LABEL: &str = "submission_inoperable";
/// A Trace Credit leg's payout label when the NEAR adapter refused or
/// failed its submit; the payout is then `failed`.
pub const PIPELINE_NEAR_SUBMIT_FAILED_LABEL: &str = "near_submit_failed";
/// A Trace Credit leg's payout label when the adapter reports the
/// transaction of a submitted line as failed on chain; the payout is then
/// `failed`.
pub const PIPELINE_NEAR_TRANSACTION_FAILED_LABEL: &str = "near_transaction_failed";
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
/// An enabled payout is refused while `main` requires rollout-smoke readiness
/// for a live settlement (`main`'s own label for that gap), or while `main`
/// caps an account's settlement (a pipeline label: `main` has no refusal
/// label for a configured cap).
pub const PIPELINE_PAYOUT_ROLLOUT_SMOKE_NOT_READY_LABEL: &str =
    "credit_settlement_rollout_smoke_not_ready";
pub const PIPELINE_PAYOUT_ACCOUNT_CAP_UNSUPPORTED_LABEL: &str =
    "credit_settlement_account_cap_unsupported";
/// `PipelineServiceBuilder::build`'s refusal of an enabled payout on an
/// adapter that presents no credential while `main` requires one
/// (`TRACE_COMMONS_NEAR_CREDIT_REQUIRE_ADAPTER_AUTH`; Zaki review 1, round 2,
/// finding 2).
pub const PIPELINE_PAYOUT_ADAPTER_AUTH_MISSING_LABEL: &str = "near_payout_adapter_auth_missing";
pub const PIPELINE_TOMBSTONE_LABEL: &str = "content_tombstoned";
/// The reason code of an index invalidation a withdrawal queues.
const PIPELINE_WITHDRAWAL_INVALIDATION_REASON: &str = "withdrawn";
/// The reason code of an index invalidation `main`'s revocation queues (Zaki
/// review 1, round 2, finding 5).
const PIPELINE_REVOCATION_INVALIDATION_REASON: &str = "revoked";
/// The reason code of the index invalidation a run failed for good queues
/// (owner ruling on Zaki review 1, round 2, N-7).
const PIPELINE_FAILED_RUN_INVALIDATION_REASON: &str = "run_failed";

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
/// The longest wait of an uncharged suspension
/// (`PgPipelineStore::mark_transient_retry`), and the wait between two
/// charged `artifact_integrity_failed` attempts
/// (`PgPipelineStore::mark_retry`), as a SQL interval. One definition, so
/// the two stay equal. A macro, so each query stays one `&'static str`.
macro_rules! suspension_max_delay_sql {
    () => {
        "INTERVAL '1 hour'"
    };
}
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
/// The blocking thread a phase's synchronous dependency call (an object
/// store, the index) was moved onto panicked or was cancelled before it
/// returned (Zaki review 1, round 2, N-6). Fails closed.
const PIPELINE_BLOCKING_CALL_FAILED_LABEL: &str = "blocking_call_failed";

/// The artifact store cannot derive an object key before the content
/// exists (`TraceArtifactStore::serialized_json_object_key`'s default
/// refusal), which a compatibility Score needs before its tenant lock
/// (rebase 10, option D). A failed store call like any other
/// (`artifact_store_call`), so the uncharged suspension of ruling FR3
/// (rebase 10 review, M2); it keeps its own label, since it names a store
/// that lacks the capability rather than one that is down.
const PIPELINE_OBJECT_KEY_UNAVAILABLE_LABEL: &str = "serialized_json_object_key_unavailable";
/// The key the store prepared a compatibility Score's object under differs
/// from the key it derived, and the attempt staged, before the tenant lock;
/// publishing it would leave an object no row names, so nothing is
/// published. The store's own inconsistency, not a check of the content it
/// returned, so it is suspended uncharged as a failed store call is
/// (`artifact_store_call`; rebase 10 review, M2).
const PIPELINE_ATTEMPT_OBJECT_KEY_MISMATCH_LABEL: &str = "pipeline_attempt_object_key_mismatch";
/// Store refusals `artifact_store_call` reports under their own label
/// rather than `artifact_store_unavailable`: each names a capability the
/// store lacks, which an operator fixes by configuration, not by waiting.
const ARTIFACT_STORE_CAPABILITY_LABELS: &[&str] = &[PIPELINE_OBJECT_KEY_UNAVAILABLE_LABEL];

/// Runs `call`, a synchronous dependency call (an object store or an index:
/// file or network I/O), on the blocking pool, so it never parks a runtime
/// worker thread (Zaki review 1, round 2, N-6). A caller may hold a
/// transaction across it: the await does not block the worker.
async fn on_blocking_pool<T, F>(call: F) -> anyhow::Result<T>
where
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(call)
        .await
        .map_err(|_| anyhow::anyhow!(PIPELINE_BLOCKING_CALL_FAILED_LABEL))?
}

/// Runs `call`, an object-store call of the run path (Review's source read,
/// and its approved object's prepare and publish; Score's approved read,
/// its object keys' derivation before the tenant lock, and each object's
/// prepare and publish), on the blocking pool, and reports the store's own
/// error by its class.
///
/// This is the pipeline's one rule for its object store (wave 2, merging
/// PR 3's c8d65fcb with PR 4's 6dace131, and Zaki's approval of #1143,
/// ZA-2). A failure of the store itself -- a failed call, a capability it
/// lacks, or a key it does not keep
/// (`PIPELINE_ATTEMPT_OBJECT_KEY_MISMATCH_LABEL`, raised by the caller as
/// the same transient `PolicyError`) -- is the uncharged suspension
/// `artifact_store_unavailable` (multi-lens review L2-2, ruling FR3), or
/// the store's own label for a refusal in
/// `ARTIFACT_STORE_CAPABILITY_LABELS`: an outage, not the trace's fault. An
/// integrity failure the store reports
/// (`is_trace_artifact_integrity_error`: the object missing, or not what
/// its receipt names) is the charged `artifact_integrity_failed`, which the
/// phase's attempt budget ends, with its attempts one hour apart
/// (`PgPipelineStore::mark_retry`). The caller's own checks of the content the
/// store returned (a decode, a hash or a revision mismatch) stay charged,
/// and a lost blocking task keeps `blocking_call_failed`. The attempt sweep
/// has no run to charge: a store call of the sweep that fails keeps its row
/// for the next pass (`PipelineService::sweep_attempt_artifacts`).
async fn artifact_store_call<T, F>(call: F) -> anyhow::Result<T>
where
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
    T: Send + 'static,
{
    on_blocking_pool(move || {
        call().map_err(|error| {
            // ZA-2: an integrity failure the store reports is charged; every
            // other failure of the store is the uncharged suspension.
            if crate::trace_artifact_store::is_trace_artifact_integrity_error(&error) {
                anyhow::anyhow!(PIPELINE_ARTIFACT_INTEGRITY_FAILED_LABEL)
            } else {
                artifact_store_failure(&error).into()
            }
        })
    })
    .await
}

/// The uncharged suspension `artifact_store_call` reports for a store
/// call's `error` that is not an integrity failure: the store's own refusal
/// when it is one of `ARTIFACT_STORE_CAPABILITY_LABELS`, else
/// `artifact_store_unavailable`. Nothing else of the store's message
/// reaches the run.
fn artifact_store_failure(error: &anyhow::Error) -> PolicyError {
    let message = error.to_string();
    let label = ARTIFACT_STORE_CAPABILITY_LABELS
        .iter()
        .copied()
        .find(|label| *label == message)
        .unwrap_or(PIPELINE_ARTIFACT_STORE_UNAVAILABLE_LABEL);
    PolicyError::transient(label).expect("static label")
}

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
/// neither as `lease_expired` nor as a charge reversed.
///
/// `PipelineLeaseRenewal` closes only the second gap, and only for a live
/// worker: it extends the lease before it expires, so no other worker can
/// reclaim a phase that is merely slow. The first gap stays: a crashed
/// worker still records nothing, and its run is reclaimed as a charged
/// attempt once its last lease expires. The second gap reopens when a live
/// worker's lease expires anyway: its phase runs past
/// `PIPELINE_LEASE_RENEWAL_CAP_FACTOR` phase leases (renewal stops at the
/// cap), or its renewal does not get to run before the lease expires.
pub const PIPELINE_LEASE_EXPIRED_LABEL: &str = "lease_expired";
/// Safe label `PipelineLeaseConfig::new` refuses with when a phase's
/// configured lease falls outside [1 second, 2 hours].
pub const PIPELINE_LEASE_CONFIG_INVALID_LABEL: &str = "pipeline_lease_config_invalid";
const PIPELINE_LEASE_MIN_SECONDS: i64 = 1;
const PIPELINE_LEASE_MAX_SECONDS: i64 = 2 * 60 * 60;
const PIPELINE_LEASE_DEFAULT_REVIEW_SECONDS: i64 = 5 * 60;
const PIPELINE_LEASE_DEFAULT_SCORE_SECONDS: i64 = 30 * 60;
const PIPELINE_LEASE_DEFAULT_SETTLE_SECONDS: i64 = 5 * 60;
/// A lease is renewed at most until this many phase leases after its claim
/// (`PipelineLeaseRenewal::start`'s `lease_cap`): an honest phase that is
/// merely slow keeps being renewed, but a phase that never comes back at all
/// still surrenders its claim within a bounded multiple of its own
/// configured lease, rather than being renewed forever.
pub const PIPELINE_LEASE_RENEWAL_CAP_FACTOR: i32 = 4;
/// How long past the latest moment a phase attempt's lease can still be
/// live (`PipelineService::attempt_artifact_lease_bound`) its staged
/// objects wait before the sweep may remove them: time for the commit
/// itself to land once the phase work is done.
const PIPELINE_ATTEMPT_ARTIFACT_COMMIT_MARGIN: Duration = Duration::hours(1);

/// Each pipeline phase gets
/// its own claim-lease length, sized for how long that phase can actually
/// run rather than one fixed lease every phase shared. Score, in
/// particular, runs the injected scorer and the embedder inside the lease
/// -- a chunked NEAR AI perplexity scorer or a CPU-bound embedder routinely
/// exceeds a short fixed lease, which used to make `commit_score` and the
/// following `mark_retry` both fail on a stale lease and burn an attempt
/// every time. The defaults below give Score six times the headroom Review
/// and Settle get. `PipelineLeaseRenewal` extends a lease the phase still
/// holds, up to `PIPELINE_LEASE_RENEWAL_CAP_FACTOR` times the length set
/// here -- this struct only sizes the one lease a phase gets when it is
/// claimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineLeaseConfig {
    review: Duration,
    score: Duration,
    settle: Duration,
    privacy_pass_max_timeout: std::time::Duration,
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
            privacy_pass_max_timeout: PIPELINE_PRIVACY_PASS_MAX_TIMEOUT,
        })
    }

    /// Replaces the privacy pass's 900 s classifier ceiling (decision P8).
    /// Refuses a value outside 60 s..=2 h with the safe label
    /// `PIPELINE_PRIVACY_PASS_TIMEOUT_INVALID_LABEL`.
    ///
    /// It replaces the ceiling only: the pass is still bounded by
    /// `review x PIPELINE_LEASE_RENEWAL_CAP_FACTOR - 60 s`
    /// (`privacy_pass_timeout_for`), so a ceiling above that needs a longer
    /// Review lease to take effect. A longer pass holds its Review lane
    /// longer.
    pub fn with_privacy_pass_max_timeout(
        mut self,
        ceiling: std::time::Duration,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            (PIPELINE_PRIVACY_PASS_MAX_TIMEOUT_MIN..=PIPELINE_PRIVACY_PASS_MAX_TIMEOUT_MAX)
                .contains(&ceiling),
            PIPELINE_PRIVACY_PASS_TIMEOUT_INVALID_LABEL
        );
        self.privacy_pass_max_timeout = ceiling;
        Ok(self)
    }

    /// The privacy pass classifier ceiling: 900 s unless configured.
    pub fn privacy_pass_max_timeout(&self) -> std::time::Duration {
        self.privacy_pass_max_timeout
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

    /// The lease a claim of `phase` gets; Admission never runs as a claim and
    /// falls back to Review, as the claim SQL's `ELSE` arm does.
    pub fn for_phase(&self, phase: Phase) -> Duration {
        match phase {
            Phase::Score => self.score,
            Phase::Settle => self.settle,
            Phase::Admission | Phase::Review => self.review,
        }
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
            privacy_pass_max_timeout: PIPELINE_PRIVACY_PASS_MAX_TIMEOUT,
        }
    }
}

/// `pub(crate)`: `versioned_pipeline_product.rs` reuses this instead of
/// holding a second copy.
pub(crate) fn sha256_prefixed(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// Whether `value` has the shape of a bundle id: `sha256:` and 64 lowercase
/// hex digits, the shape the tables check. An operator action checks it
/// before the id reaches a query or an error string (`intervene_policy`,
/// ingest's admin routes).
pub fn is_bundle_id(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}

/// The rule for who an operator action is recorded under and why, shared by
/// every operator action of the pipeline (a routing change in
/// `versioned_pipeline_activation.rs`, a policy intervention here). The actor
/// is the credential's `principal_ref`, already a hash-derived reference on
/// `main`: 1 to 160 of `A-Za-z0-9_:.-`, the shape the tables check. The
/// reason is a safe label. Either failing is a `Constraint` carrying
/// `refusal_label`, which each caller chooses, so a refusal names the action
/// that was refused. Nothing of the actor or the reason is in the error.
pub(crate) fn validate_actor(
    actor_principal_ref: &str,
    reason_code: &str,
    refusal_label: &str,
) -> Result<(), DatabaseError> {
    let actor_ok = !actor_principal_ref.is_empty()
        && actor_principal_ref.len() <= 160
        && actor_principal_ref
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b':' | b'.' | b'-'));
    if !actor_ok || !crate::versioned_pipeline_qualification::is_safe_label(reason_code) {
        return Err(DatabaseError::Constraint(refusal_label.to_string()));
    }
    Ok(())
}

/// Maps an envelope's raw residual PII risk onto Admission's scale. A
/// Medium whose basis is exactly `[ConsentContentFlag]` is a consent fact,
/// not a PII finding, so it maps to Low; every other Medium stays Medium.
/// The receipt maps its Admission input through this, and the Review-start
/// privacy pass maps both the stored receipt-time risk and its own result
/// through it, so the two sides of its comparison share one scale.
pub fn pipeline_privacy_risk(
    risk: &ResidualPiiRisk,
    basis: &[ResidualRiskCondition],
) -> PrivacyRisk {
    match risk {
        ResidualPiiRisk::Low => PrivacyRisk::Low,
        ResidualPiiRisk::Medium if matches!(basis, [ResidualRiskCondition::ConsentContentFlag]) => {
            PrivacyRisk::Low
        }
        ResidualPiiRisk::Medium => PrivacyRisk::Medium,
        ResidualPiiRisk::High => PrivacyRisk::High,
    }
}

/// The Admission rule id a Medium receipt is admitted under when the
/// operator accepts Medium risk (`admission_privacy_risk`). The receipt
/// records it in place of the bundle's own `minimal_admission_v1`, so an
/// operator can tell those admissions from a Low's. A label, nothing else.
pub const PIPELINE_ADMISSION_ACCEPT_FOUND_AND_REMOVED_RULE_ID: &str =
    "minimal_admission_v1_accept_found_and_removed";

/// The privacy risk the receipt hands Admission, and whether the operator's
/// accept-Medium setting lowered it. `accept_medium_risk` is ingest's
/// `TRACE_COMMONS_ACCEPT_MEDIUM_RISK_SUBMISSIONS`.
///
/// Owner decision 2026-10-12 ("once PII is removed, accept", applied to
/// Admission): with the setting on, a Medium whose basis records only what
/// redaction found and removed -- `FoundAndRemoved`, with or without
/// `ConsentContentFlag` -- is handed to Admission as Low, so the bundle's
/// Admission policy admits it, unchanged. Everything else is exactly
/// `pipeline_privacy_risk`: with the setting off, a Medium of any other
/// basis (an empty one, a declared Medium nothing was found for), and every
/// High (a key finding, a coverage gap, a survivor, a residual scan that
/// could not run) keep their hold. The Review-start privacy pass never
/// calls this: it compares on `pipeline_privacy_risk`, so the receipt it
/// compares against stays Medium and a pass High still escalates.
pub fn admission_privacy_risk(
    risk: &ResidualPiiRisk,
    basis: &[ResidualRiskCondition],
    accept_medium_risk: bool,
) -> (PrivacyRisk, bool) {
    let mapped = pipeline_privacy_risk(risk, basis);
    let found_and_removed_only = basis.contains(&ResidualRiskCondition::FoundAndRemoved)
        && basis.iter().all(|condition| {
            matches!(
                condition,
                ResidualRiskCondition::FoundAndRemoved | ResidualRiskCondition::ConsentContentFlag
            )
        });
    if accept_medium_risk && mapped == PrivacyRisk::Medium && found_and_removed_only {
        (PrivacyRisk::Low, true)
    } else {
        (mapped, false)
    }
}

/// The Review-start privacy pass's outcome (decision P3, as amended by the
/// owner decision of 2026-10-10, "once PII removed we should go to
/// accepted"). `receipt_risk` and `pass_risk` are both on Admission's scale
/// (`pipeline_privacy_risk`).
///
/// The pass escalates only when its risk is High and strictly above the
/// receipt's. A post-pass Medium is the found-and-removed (or consent-flag)
/// floor: the classifier found prose PII and the envelope Review reads is
/// the redacted one, so the pass clears and the run goes on to the Review
/// policy on the scrubbed bytes, with the merged basis
/// (`found_and_removed`) and the redaction counts recorded for audit. High
/// means something redaction did not resolve (a key finding, a coverage gap,
/// a survivor, a residual scan that could not run) and is held for a human,
/// never rejected (D1). A receipt already at High is held by Admission
/// (`privacy_risk_high_review_required`, owner decision PC-D27,
/// 2026-10-11), so the pass cannot escalate it (its risk is not above the
/// receipt's) and returns `Cleared`; that run, like a Medium receipt
/// Admission quarantined, keeps Admission's hold whatever the pass clears.
/// A found-and-removed Medium Admission admitted under the operator's
/// accept-Medium setting (`admission_privacy_risk`) is still Medium here,
/// so a pass Medium clears it like any admitted run and only a pass High
/// escalates it.
pub fn privacy_pass_outcome(
    receipt_risk: PrivacyRisk,
    pass_risk: PrivacyRisk,
) -> PrivacyPassOutcome {
    if pass_risk == PrivacyRisk::High && pass_risk > receipt_risk {
        PrivacyPassOutcome::Escalated
    } else {
        PrivacyPassOutcome::Cleared
    }
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

/// A JSONB array of labels, as V117's pass columns hold them; NULL is `None`.
fn json_label_array(row: &Row, column: &str) -> Result<Option<Vec<String>>, DatabaseError> {
    row.get::<_, Option<serde_json::Value>>(column)
        .map(|value| {
            serde_json::from_value::<Vec<String>>(value).map_err(|_| {
                DatabaseError::Serialization(format!(
                    "pipeline run {column} is not an array of labels"
                ))
            })
        })
        .transpose()
}

/// The result of `PipelineService::settle_internal_credit`: the Trace Credit
/// leg settled (its ledger row, the finalized batch that carries it, and
/// the completed settlement row committed together), or a hold on the
/// account, found under the account lock, rolled the credit transaction
/// back -- the caller then records the row as `held` and no ledger row is
/// written -- or the submission stopped being operable under that same
/// lock, which also rolls the transaction back with nothing written -- or
/// the Settle policy was no longer runnable under its status row's lock, which
/// rolls the transaction back with nothing written as well.
enum InternalCreditResult {
    Complete,
    Held,
    /// The Settle policy of the run's bundle was suspended (GRD-004), found
    /// under `lock_runnable_policy` inside this same transaction, after the
    /// run, account, and submission locks and before the first write. The
    /// transaction rolled back with nothing written -- no ledger row, no
    /// batch. The leg's adapter call, if it was made, is not retracted.
    PolicyNotRunnable,
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

/// The approved content's canonical summary and its hash
/// (`ApprovedRevision::canonical_summary`).
pub fn approved_canonical_summary(content: &[u8]) -> (Option<String>, Option<String>) {
    match serde_json::from_slice::<TraceContributionEnvelope>(content) {
        Ok(envelope) => {
            let summary =
                trace_commons_protocol::trace_contribution::canonical_summary_for_embedding(
                    &envelope,
                );
            let hash = sha256_prefixed(summary.as_bytes());
            (Some(summary), Some(hash))
        }
        Err(_) => (None, None),
    }
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
    /// created, a legacy owner, a routing that no longer serves the tenant,
    /// or a tombstone). This attempt's object is not referenced.
    Refused(PipelineReceiptResult),
    /// This attempt's row is no longer `staged` (the sweeper removed it) or
    /// is due (a sweep that failed may have deleted its object), so it
    /// cannot commit.
    StagingMissing,
    /// The Admission policy of the run's bound bundle was suspended after the
    /// staging transaction read it (GRD-004). Nothing was written, so the
    /// attempt's object and row are discarded, and the receipt is the error
    /// `bundle_policy_not_runnable`, as when the staging transaction finds it.
    PolicyNotRunnable,
}

/// The run half of index-rebuild eligibility, over `pipeline_runs p`:
/// complete, included in the index, with a completed index write that no
/// invalidation has touched since, and a stored command reference and hash
/// to rebuild from. `PgPipelineStore::list_rebuildable_index_runs` lists by
/// it and `PgPipelineStore::lock_rebuildable_index_run_on_tx` re-checks it
/// under the run's lock, so the two cannot drift. The submission half is
/// `PgPipelineStore::submission_guard_on_tx` itself, read per run under
/// that lock (final review I1 and M8).
///
/// A run's invalidation state is its `pipeline_index_invalidations` row's
/// state, `none` without a row (PR 3, 62cc2768: `pipeline_runs` keeps no
/// copy), so "no invalidation has touched it" is "it has no row", whatever
/// that row's state.
macro_rules! rebuildable_index_run_predicate {
    () => {
        "p.state = 'complete'
         AND p.index_membership = 'included'
         AND p.index_write_state = 'complete'
         AND NOT EXISTS (
             SELECT 1 FROM pipeline_index_invalidations i
              WHERE i.tenant_id = p.tenant_id AND i.run_id = p.run_id
         )
         AND p.index_command_ref IS NOT NULL
         AND p.index_command_hash IS NOT NULL"
    };
}

/// Cheap to clone: the pool it wraps is already an `Arc`.
/// `PipelineLeaseRenewal::start` clones one into its own background task,
/// which takes its own short connection checkout rather than sharing the
/// caller's.
#[derive(Clone)]
pub struct PgPipelineStore {
    backend: Arc<PgBackend>,
    /// Per tenant, the last submission id the lost follow-up recovery read
    /// (`recover_lost_inoperable_follow_ups`); the next pass resumes after
    /// it. Shared by the store's clones; lost on restart, when the next
    /// pass starts from the lowest id again.
    lost_follow_up_cursors: Arc<std::sync::Mutex<std::collections::HashMap<String, Uuid>>>,
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

/// Why a new receipt may not start a run for `tenant_id` now, read inside
/// `tx`: the tenant is contained, the legacy path owns the submission id, or
/// the tenant's routing does not send new receipts to the pipeline, in that
/// order. `None` when the receipt may go on.
/// With `lock`, the tenant's routing lock is held shared for the rest of
/// `tx`, so a routing change waits for this transaction and no later one
/// misses the change. The lock is the one `PipelineActivationStore` takes
/// exclusively: the same key (`pipeline_routing_lock`) and the same seed
/// (`PIPELINE_ROUTING_LOCK_SEED`).
///
/// A caller asks only after it found no run for the receipt's key, so a retry
/// of an existing receipt is answered from its run in every routing state.
/// One statement of single-row reads by primary key (the owner and the
/// routing state together), and one lock request with `lock`.
async fn new_receipt_refusal_in(
    tx: &Transaction<'_>,
    tenant_id: &str,
    submission_id: Uuid,
    unqualified_routing: bool,
    lock: bool,
) -> Result<Option<PipelineReceiptResult>, DatabaseError> {
    if lock {
        tx.execute(
            "SELECT pg_advisory_xact_lock_shared(hashtextextended($1, $2))",
            &[
                &pipeline_routing_lock(tenant_id),
                &PIPELINE_ROUTING_LOCK_SEED,
            ],
        )
        .await?;
    }
    // The legacy path owns the id when its ownership row says so, or when a
    // submission row holds the id and no pipeline run does (a legacy receipt
    // from before ownership rows). A pipeline-owned id has a run. The owner
    // and the routing state are one statement (final fix wave G26): the
    // staging transaction runs it inside the tenant's quota lock, and the
    // statement is after the routing lock, so it reads what a routing change
    // committed before the lock was granted.
    let row = tx
        .query_one(
            "SELECT EXISTS (
                 SELECT 1 FROM pipeline_receipt_ownership
                  WHERE tenant_id = $1 AND submission_id = $2 AND owner = 'legacy'
             ) OR EXISTS (
                 SELECT 1 FROM trace_submissions s
                  WHERE s.tenant_id = $1 AND s.submission_id = $2
                    AND NOT EXISTS (
                        SELECT 1 FROM pipeline_runs r
                         WHERE r.tenant_id = s.tenant_id
                           AND r.submission_id = s.submission_id
                    )
             ) AS legacy_owned,
             (SELECT routing_state FROM pipeline_tenant_routing
               WHERE tenant_id = $1) AS routing_state",
            &[&tenant_id, &submission_id],
        )
        .await?;
    let state = row
        .get::<_, Option<String>>("routing_state")
        .map(|state| RoutingState::from_db(state.as_str()))
        .transpose()?;
    // Containment is answered before the owner (review round 1, point 7): a
    // legacy-owned id with no legacy record (a claim whose write failed)
    // would else go back to the handler as `LegacyOwned`, and the handler's
    // legacy path would write for a tenant whose containment committed after
    // the handler's own routing read.
    if state == Some(RoutingState::Contained) {
        return Ok(Some(PipelineReceiptResult::NotRouted(
            RoutingState::Contained,
        )));
    }
    if row.get::<_, bool>("legacy_owned") {
        return Ok(Some(PipelineReceiptResult::LegacyOwned));
    }
    Ok(match state {
        Some(RoutingState::Pipeline) => None,
        Some(other) => Some(PipelineReceiptResult::NotRouted(other)),
        None if unqualified_routing => None,
        None => Some(PipelineReceiptResult::NotRouted(RoutingState::Legacy)),
    })
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

/// #1313, the gap: holds `run` for an operator, on the caller's
/// transaction, which just failed it under `error_label` and holds its row.
/// A run is held when its Settle can finish (`settle_run_is_finishable_sql!`)
/// and one of its legs paid (`settle_run_has_paid_leg_sql!`), and either the
/// failure is a refusal of the Settle commit that a retry cannot change
/// (`PIPELINE_GATE_DECISION_CONFLICT_LABEL`,
/// `PIPELINE_GATE_DECISION_EVIDENCE_INCOMPLETE_LABEL`) or the run was held
/// before (V128 `operator_held_at`): an operator released it, and its
/// attempt failed. The run moves to `awaiting_operator` with its label kept,
/// records the hold's time and sets the review audit marker, so the
/// worker's review audit pass appends a label-only `lifecycle_status_change`
/// event for the hold. Returns the held run, or `None` when it held nothing.
async fn hold_paid_settle_run_for_operator_on_tx(
    tx: &Transaction<'_>,
    run: &PipelineRunRecord,
    error_label: &str,
) -> Result<Option<PipelineRunRecord>, DatabaseError> {
    let refused_for_good = error_label == PIPELINE_GATE_DECISION_CONFLICT_LABEL
        || error_label == PIPELINE_GATE_DECISION_EVIDENCE_INCOMPLETE_LABEL;
    tx.query_opt(
        concat!(
            "UPDATE pipeline_runs p
                SET state = 'awaiting_operator',
                    operator_held_at = clock_timestamp(),
                    review_audit_pending_at = clock_timestamp(),
                    updated_at = NOW()
              WHERE p.tenant_id = $1 AND p.run_id = $2 AND p.state = 'failed'
                AND ($3::boolean OR p.operator_held_at IS NOT NULL)
                AND ",
            settle_run_is_finishable_sql!(),
            " AND ",
            settle_run_has_paid_leg_sql!(),
            " RETURNING p.*"
        ),
        &[&run.tenant_id, &run.run_id, &refused_for_good],
    )
    .await?
    .as_ref()
    .map(pipeline_run_from_row)
    .transpose()
}

/// #1346: for each run of `run_ids` that this transaction ended `failed`
/// while its submission is still `received` -- Review never decided it, so
/// nothing else would ever tell the contributor -- moves the submission to
/// `rejected`, with the status reason `pipeline_processing_failed` and no
/// pending credit, records the move on the run (V123,
/// `submission_rejected_at`) and sets the run's review audit marker, so the
/// worker's review audit pass appends `main`'s `lifecycle_status_change`
/// event for it. Runs in the caller's transaction, after the run UPDATE.
///
/// Only a live `received` row moves (`live_submission_sql!`): a submission
/// that is revoked, purged, withdrawn or expired, or that is stored
/// `accepted`, `rejected` or `quarantined`, is left as it is, and its run
/// gets no marker. `quarantined` includes a submission Admission quarantined
/// (stored `quarantined` at receipt, before Review ran) as well as one the
/// Review-start privacy pass escalated; neither moves (PR #1351, owner
/// decision 3). A run failed under `privacy_classification_failed`
/// does not move either: its content was never classified, which owner
/// decision Q1 reports as held content (`quarantined`), not as a processing
/// failure. Idempotent.
async fn reject_received_submissions_of_failed_runs_on_tx(
    tx: &Transaction<'_>,
    tenant_id: &str,
    run_ids: &[Uuid],
) -> Result<u64, DatabaseError> {
    if run_ids.is_empty() {
        return Ok(0);
    }
    let marked = tx
        .execute(
            concat!(
                "WITH failed AS MATERIALIZED (
                    SELECT p.run_id, p.submission_id FROM pipeline_runs p
                     WHERE p.tenant_id = $1 AND p.run_id = ANY($2)
                       AND p.state = 'failed'
                       AND p.last_error_label IS DISTINCT FROM $3
                 ),
                 moved AS (
                    UPDATE trace_submissions s
                       SET status = 'rejected', last_status_reason = $4,
                           credit_points_pending = 0, updated_at = NOW()
                      FROM failed f
                     WHERE s.tenant_id = $1 AND s.submission_id = f.submission_id
                       AND s.status = 'received' AND ",
                live_submission_sql!(),
                "
                    RETURNING s.submission_id
                 )
                 UPDATE pipeline_runs r
                    SET submission_rejected_at = clock_timestamp(),
                        review_audit_pending_at = clock_timestamp()
                   FROM failed f JOIN moved m ON m.submission_id = f.submission_id
                  WHERE r.tenant_id = $1 AND r.run_id = f.run_id"
            ),
            &[
                &tenant_id,
                &run_ids,
                &PIPELINE_PRIVACY_CLASSIFICATION_FAILED_LABEL,
                &PIPELINE_PROCESSING_FAILED_STATUS_REASON,
            ],
        )
        .await?;
    Ok(marked)
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
/// Owner ruling on Zaki review 1, round 2, N-7: the index holds only
/// revisions of runs that completed. For each run of `run_ids` that this
/// transaction failed for good and whose index write may have written
/// entries (`pending`, possibly partly written; `complete`; `failed`, a
/// content conflict after earlier entries; `cancelled`), it queues the
/// invalidation of the run's revision through the queue a withdrawal uses
/// (`end_runs_of_inoperable_submission_on_tx`, reason `run_failed`): a
/// `pending` write is cancelled and the run excluded from the index. Every
/// failure path that resolves a Settle run's open legs calls it in the same
/// transaction: the claim's sweep, `mark_failed`, and `mark_retry` when it
/// exhausts the run. Lock order: the run rows the caller's update holds,
/// then the invalidation rows. Idempotent.
async fn invalidate_index_writes_of_failed_runs_on_tx(
    tx: &Transaction<'_>,
    tenant_id: &str,
    run_ids: &[Uuid],
) -> Result<(), DatabaseError> {
    if run_ids.is_empty() {
        return Ok(());
    }
    let runs = tx
        .query(
            "SELECT * FROM pipeline_runs
              WHERE tenant_id = $1 AND run_id = ANY($2)
                AND state = 'failed' AND index_write_state <> 'none'
              ORDER BY run_id",
            &[&tenant_id, &run_ids],
        )
        .await?
        .iter()
        .map(pipeline_run_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    PgPipelineStore::end_runs_of_inoperable_submission_on_tx(
        tx,
        tenant_id,
        &runs,
        PIPELINE_FAILED_RUN_INVALIDATION_REASON,
    )
    .await
}

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
/// transaction: the pipeline's ownership row for the submission id, the
/// submission, its source object ref, the run itself, the Admission outcome,
/// and the attempt's staging row's transition to `committed` -- which
/// requires the row to be `staged` and to name exactly the object the object
/// ref records.
///
/// The ownership row is the first statement. Its primary key is the one place
/// where the legacy path's claim (`PipelineActivationStore::claim_legacy_receipt`)
/// and this insert meet, so exactly one of them succeeds for a submission id:
/// a concurrent claim makes this insert wait for its outcome. The row names a
/// run that does not exist yet; its foreign key is deferred, so it is checked
/// when the transaction commits.
///
/// A conflict is answered by who owns the id, and the caller rolls the
/// transaction back either way. At the ownership row: a `legacy` owner ends in
/// `DatabaseError::Constraint(PIPELINE_LEGACY_RECEIPT_OWNED_LABEL)`, which
/// `commit_receipt_attempt` turns into `LegacyOwned`; a `pipeline` owner (a
/// second key for an id a run owns) ends in `main`'s error
/// (`PIPELINE_SUBMISSION_BOUND_MESSAGE`), which stays an error. At the
/// submission row, where no ownership row existed: a pipeline run for the id
/// (a run from before ownership rows) is the same error, and no run means a
/// legacy submission from before the tenant came into scope, which is the
/// legacy label.
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
    let owned = tx
        .execute(
            "INSERT INTO pipeline_receipt_ownership (tenant_id, submission_id, owner, run_id)
             VALUES ($1, $2, 'pipeline', $3)
             ON CONFLICT (tenant_id, submission_id) DO NOTHING",
            &[&run.tenant_id, &run.submission_id, &run.run_id],
        )
        .await?;
    if owned != 1 {
        // The id has an owner. Its row, committed by the time this insert
        // returned, says whose: only a legacy owner makes the receipt
        // `LegacyOwned`.
        let owner: String = tx
            .query_one(
                "SELECT owner FROM pipeline_receipt_ownership
                  WHERE tenant_id = $1 AND submission_id = $2",
                &[&run.tenant_id, &run.submission_id],
            )
            .await?
            .get(0);
        return Err(DatabaseError::Constraint(
            if owner == "legacy" {
                PIPELINE_LEGACY_RECEIPT_OWNED_LABEL
            } else {
                PIPELINE_SUBMISSION_BOUND_MESSAGE
            }
            .to_string(),
        ));
    }
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
        // No ownership row existed, so the submission row is a pipeline run's
        // from before ownership rows, or a legacy receipt's.
        let pipeline_run_exists: bool = tx
            .query_one(
                "SELECT EXISTS (
                     SELECT 1 FROM pipeline_runs
                      WHERE tenant_id = $1 AND submission_id = $2
                 )",
                &[&run.tenant_id, &run.submission_id],
            )
            .await?
            .get(0);
        return Err(DatabaseError::Constraint(
            if pipeline_run_exists {
                PIPELINE_SUBMISSION_BOUND_MESSAGE
            } else {
                PIPELINE_LEGACY_RECEIPT_OWNED_LABEL
            }
            .to_string(),
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
    // The same rule as a phase commit's attempt rows (wave 2, fix round 1;
    // review I2): no `staged` row naming this object is a refusal under
    // `pipeline_attempt_artifact_missing`.
    if committed != 1 {
        return Err(DatabaseError::Constraint(
            PIPELINE_ATTEMPT_ARTIFACT_MISSING_LABEL.to_string(),
        ));
    }
    Ok(())
}

pub(crate) async fn load_bundle_from_transaction(
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

/// Whether the policy of `phase` in `bundle_id` is runnable, with its status
/// row locked `FOR SHARE` for the rest of `tx`: an intervention waits for
/// this commit, and a commit that starts after a suspension is refused
/// (GRD-004). A bundle with no status row is not runnable, as at the start of
/// a phase (`policy_is_runnable`).
///
/// Every phase commit calls this once, after its own lease, submission, and
/// routing guards and before its first write, so the policy row is the last
/// lock before the writes. An intervention takes only this row, so no
/// transaction that holds it waits for a lock an intervention holds.
pub(crate) async fn lock_runnable_policy(
    tx: &Transaction<'_>,
    tenant_id: &str,
    bundle_id: &str,
    phase: Phase,
) -> Result<bool, DatabaseError> {
    Ok(tx
        .query_opt(
            "SELECT runnable FROM pipeline_bundle_policy_status
              WHERE tenant_id = $1 AND bundle_id = $2 AND phase = $3
              FOR SHARE",
            &[&tenant_id, &bundle_id, &phase_as_db(Some(phase))],
        )
        .await?
        .is_some_and(|row| row.get::<_, bool>("runnable")))
}

/// `error` as the refusal `label` when it is a lock wait that ran out its
/// `lock_timeout` (`lock_not_available`, SQLSTATE 55P03); any other error as
/// it is. For an operator's action that set `PIPELINE_ADMIN_LOCK_TIMEOUT_SQL`.
pub(crate) fn lock_timeout_as(error: DatabaseError, label: &str) -> DatabaseError {
    match error {
        DatabaseError::Postgres(error)
            if error.code() == Some(&tokio_postgres::error::SqlState::LOCK_NOT_AVAILABLE) =>
        {
            DatabaseError::Constraint(label.to_string())
        }
        other => other,
    }
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
    // Expiry is judged on the database clock (`NOW()`), as the review
    // queue, the submission guard and `settle_internal_credit` judge it, so
    // the claim and the assessment never disagree with them about a
    // submission that expires near now (Zaki review 1, round 2, finding 20).
    // Two statements, lock first, read second, as the submission guard
    // does: the read takes its snapshot after the lock is held, so it sees a
    // withdrawal that committed while the lock was awaited.
    tx.query_opt(
        "SELECT 1 FROM trace_submissions
          WHERE tenant_id = $1 AND submission_id = $2
          FOR UPDATE",
        &[&tenant_id, &submission_id],
    )
    .await?;
    Ok(tx
        .query_opt(
            concat!(
                "SELECT ",
                reviewable_submission_sql!(),
                " FROM trace_submissions s
                  WHERE s.tenant_id = $1 AND s.submission_id = $2"
            ),
            &[&tenant_id, &submission_id],
        )
        .await?
        .is_some_and(|row| row.get::<_, bool>(0)))
}

/// The `phase` column value of an assessment (V130).
fn assessment_phase(at_score: bool) -> &'static str {
    if at_score { "score" } else { "review" }
}

/// Whether the submission of a held run is still one its hold applies to,
/// under the submission row's lock: at Review, `review_submission_is_operable`;
/// at Score, the accepted live submission Score needs
/// (`accepted_submission_sql!`, as `parked_run_submission_sql!` reads it).
async fn hold_submission_is_operable(
    tx: &Transaction<'_>,
    tenant_id: &str,
    submission_id: Uuid,
    at_score: bool,
) -> Result<bool, DatabaseError> {
    if !at_score {
        return review_submission_is_operable(tx, tenant_id, submission_id).await;
    }
    tx.query_opt(
        "SELECT 1 FROM trace_submissions
          WHERE tenant_id = $1 AND submission_id = $2
          FOR UPDATE",
        &[&tenant_id, &submission_id],
    )
    .await?;
    Ok(tx
        .query_opt(
            concat!(
                "SELECT ",
                accepted_submission_sql!(),
                " FROM trace_submissions s
                  WHERE s.tenant_id = $1 AND s.submission_id = $2"
            ),
            &[&tenant_id, &submission_id],
        )
        .await?
        .is_some_and(|row| row.get::<_, bool>(0)))
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

/// The pipeline follow-up of a withdrawal, for the submissions that have a
/// pipeline run (each with its row), on the withdrawal's transaction (which
/// already holds those submission rows): a tombstone each; their object refs
/// invalidated and each live one queued for payload deletion
/// (`delete_object_payload`, done by the revocation-propagation worker);
/// their derived records revoked; their vector entries, legacy export
/// manifests and items, and pipeline export snapshots and items
/// invalidated. Every statement runs once for all of them (Zaki review 1,
/// round 2, item 5), and every write is idempotent. Nothing at all for no
/// submission. `reason` is the caller's (`withdrawn` or `revoked`): the
/// tombstone's reason and the export items' invalidation reason (Zaki
/// review 3, Z3-L3).
async fn withdraw_pipeline_content_on_tx(
    tx: &Transaction<'_>,
    tenant_id: &str,
    submissions: &[(Uuid, &Row)],
    actor_principal_ref: &str,
    reason: &str,
) -> Result<(), DatabaseError> {
    if submissions.is_empty() {
        return Ok(());
    }
    let submission_ids = submissions
        .iter()
        .map(|(submission_id, _)| *submission_id)
        .collect::<Vec<_>>();
    let trace_ids = submissions
        .iter()
        .map(|(_, row)| row.get::<_, Uuid>("trace_id"))
        .collect::<Vec<_>>();
    let object_rows = tx
        .query(
            "SELECT submission_id, object_ref_id
               FROM trace_object_refs
              WHERE tenant_id = $1 AND submission_id = ANY($2)
                AND deleted_at IS NULL
              ORDER BY submission_id, object_ref_id",
            &[&tenant_id, &submission_ids],
        )
        .await?;
    let tombstone_ids = submission_ids
        .iter()
        .map(|submission_id| {
            Uuid::new_v5(
                &Uuid::NAMESPACE_URL,
                format!("pipeline-withdrawal:{tenant_id}:{submission_id}").as_bytes(),
            )
        })
        .collect::<Vec<_>>();
    let redaction_hashes = submissions
        .iter()
        .map(|(_, row)| row.get::<_, String>("redaction_hash"))
        .collect::<Vec<_>>();
    let canonical_summary_hashes = submissions
        .iter()
        .map(|(_, row)| row.get::<_, Option<String>>("canonical_summary_hash"))
        .collect::<Vec<_>>();
    tx.execute(
        "INSERT INTO trace_tombstones (
            tenant_id, tombstone_id, submission_id, trace_id, redaction_hash,
            canonical_summary_hash, reason, effective_at, created_by_principal_ref
         )
         SELECT $1, tombstone.tombstone_id, tombstone.submission_id, tombstone.trace_id,
                tombstone.redaction_hash, tombstone.canonical_summary_hash, $8,
                NOW(), $7
           FROM unnest($2::UUID[], $3::UUID[], $4::UUID[], $5::TEXT[], $6::TEXT[])
                AS tombstone(
                    tombstone_id, submission_id, trace_id, redaction_hash,
                    canonical_summary_hash
                )
         ON CONFLICT (tenant_id, submission_id) DO NOTHING",
        &[
            &tenant_id,
            &tombstone_ids,
            &submission_ids,
            &trace_ids,
            &redaction_hashes,
            &canonical_summary_hashes,
            &actor_principal_ref,
            &reason,
        ],
    )
    .await?;
    tx.execute(
        "UPDATE trace_object_refs
            SET invalidated_at = COALESCE(invalidated_at, NOW()), updated_at = NOW()
          WHERE tenant_id = $1 AND submission_id = ANY($2) AND invalidated_at IS NULL",
        &[&tenant_id, &submission_ids],
    )
    .await?;
    tx.execute(
        "UPDATE trace_derived_records
            SET status = 'revoked', updated_at = NOW()
          WHERE tenant_id = $1 AND submission_id = ANY($2) AND status <> 'revoked'",
        &[&tenant_id, &submission_ids],
    )
    .await?;
    tx.execute(
        "UPDATE trace_vector_entries
            SET status = 'invalidated',
                invalidated_at = COALESCE(invalidated_at, NOW()), updated_at = NOW()
          WHERE tenant_id = $1 AND submission_id = ANY($2)
            AND status <> 'invalidated' AND deleted_at IS NULL",
        &[&tenant_id, &submission_ids],
    )
    .await?;
    tx.execute(
        "UPDATE trace_export_manifest_items
            SET source_invalidated_at = COALESCE(source_invalidated_at, NOW()),
                source_invalidation_reason = 'revoked', updated_at = NOW()
          WHERE tenant_id = $1 AND submission_id = ANY($2)
            AND source_invalidated_at IS NULL",
        &[&tenant_id, &submission_ids],
    )
    .await?;
    tx.execute(
        "UPDATE trace_export_manifests
            SET invalidated_at = COALESCE(invalidated_at, NOW()), updated_at = NOW()
          WHERE tenant_id = $1 AND source_submission_ids && $2::UUID[]
            AND invalidated_at IS NULL AND deleted_at IS NULL",
        &[&tenant_id, &submission_ids],
    )
    .await?;
    invalidate_pipeline_exports_on_tx(tx, tenant_id, &submission_ids, reason).await?;
    let trace_id_of = submission_ids
        .iter()
        .copied()
        .zip(trace_ids.iter().copied())
        .collect::<BTreeMap<_, _>>();
    queue_payload_deletions_of_rows_on_tx(
        tx,
        tenant_id,
        &object_rows,
        &trace_id_of,
        PipelinePayloadDeletion::Withdrawal,
    )
    .await
}

/// Invalidates the pipeline export snapshot items of `submission_ids`, under
/// `reason` (a `pipeline_export_snapshot_items.invalidation_reason`), and
/// every snapshot that holds one. Idempotent.
async fn invalidate_pipeline_exports_on_tx(
    tx: &Transaction<'_>,
    tenant_id: &str,
    submission_ids: &[Uuid],
    reason: &str,
) -> Result<(), DatabaseError> {
    tx.execute(
        "UPDATE pipeline_export_snapshot_items
            SET invalidated_at = COALESCE(invalidated_at, NOW()),
                invalidation_reason = $3
          WHERE tenant_id = $1 AND submission_id = ANY($2)
            AND invalidated_at IS NULL",
        &[&tenant_id, &submission_ids, &reason],
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
                   AND item.submission_id = ANY($2)
            )",
        &[&tenant_id, &submission_ids],
    )
    .await?;
    clear_gate_decision_dedup_on_tx(tx, tenant_id, submission_ids).await?;
    Ok(())
}

/// Clears the dedup columns of the submissions' `trace_gate_decisions` rows
/// on the caller's transaction (spec 2026-10-08, Slice C, C-D6): the same
/// UPDATE as `main`'s `clear_trace_dedup_cluster_for_submission`, and
/// nothing else. The row stays (account trust keeps the gate evaluation as
/// history), and no withdrawal value is written into
/// `credit_withheld_reason`, whose only `main` writer is the gate path.
/// Idempotent.
async fn clear_gate_decision_dedup_on_tx(
    tx: &Transaction<'_>,
    tenant_id: &str,
    submission_ids: &[Uuid],
) -> Result<(), DatabaseError> {
    if submission_ids.is_empty() {
        return Ok(());
    }
    // The version stamp goes with the value it names (V57).
    tx.execute(
        "UPDATE trace_gate_decisions
            SET dedup_simhash = NULL,
                dedup_cluster_id = NULL,
                dedup_cluster_size = NULL,
                dedup_signal_version = NULL
          WHERE tenant_id = $1 AND submission_id = ANY($2)
            AND (dedup_simhash IS NOT NULL OR dedup_cluster_id IS NOT NULL
                 OR dedup_cluster_size IS NOT NULL OR dedup_signal_version IS NOT NULL)",
        &[&tenant_id, &submission_ids],
    )
    .await?;
    Ok(())
}

/// Why the pipeline queues a submission's payload deletions: each names its
/// own idempotency key and propagation reason, so a withdrawal and a purge
/// of one object never collapse into one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PipelinePayloadDeletion {
    Withdrawal,
    RetentionPurge,
}

impl PipelinePayloadDeletion {
    fn idempotency_label(self) -> &'static str {
        match self {
            Self::Withdrawal => "pipeline-withdrawal-object-delete:v1",
            Self::RetentionPurge => "pipeline-retention-purge-object-delete:v1",
        }
    }

    fn reason(self) -> &'static str {
        match self {
            Self::Withdrawal => "pipeline_withdrawal",
            Self::RetentionPurge => "pipeline_retention_purge",
        }
    }
}

/// Invalidates the object refs of the submissions in `trace_id_of` (each
/// mapped to its trace id) and queues one payload deletion per live one
/// (`queue_payload_deletions_of_rows_on_tx`).
async fn queue_pipeline_payload_deletions_on_tx(
    tx: &Transaction<'_>,
    tenant_id: &str,
    trace_id_of: &BTreeMap<Uuid, Uuid>,
    deletion: PipelinePayloadDeletion,
) -> Result<(), DatabaseError> {
    let submission_ids = trace_id_of.keys().copied().collect::<Vec<_>>();
    let object_rows = tx
        .query(
            "SELECT submission_id, object_ref_id
               FROM trace_object_refs
              WHERE tenant_id = $1 AND submission_id = ANY($2)
                AND deleted_at IS NULL
              ORDER BY submission_id, object_ref_id",
            &[&tenant_id, &submission_ids],
        )
        .await?;
    tx.execute(
        "UPDATE trace_object_refs
            SET invalidated_at = COALESCE(invalidated_at, NOW()), updated_at = NOW()
          WHERE tenant_id = $1 AND submission_id = ANY($2) AND invalidated_at IS NULL",
        &[&tenant_id, &submission_ids],
    )
    .await?;
    queue_payload_deletions_of_rows_on_tx(tx, tenant_id, &object_rows, trace_id_of, deletion).await
}

/// Queues one `delete_object_payload` propagation item per row of
/// `object_rows` (`submission_id`, `object_ref_id`), done by `main`'s
/// revocation-propagation worker. Idempotent: each item's key is derived
/// from the deletion's kind, the submission and the object ref.
async fn queue_payload_deletions_of_rows_on_tx(
    tx: &Transaction<'_>,
    tenant_id: &str,
    object_rows: &[Row],
    trace_id_of: &BTreeMap<Uuid, Uuid>,
    deletion: PipelinePayloadDeletion,
) -> Result<(), DatabaseError> {
    if object_rows.is_empty() {
        return Ok(());
    }
    let mut propagation_item_ids = Vec::with_capacity(object_rows.len());
    let mut source_submission_ids = Vec::with_capacity(object_rows.len());
    let mut item_trace_ids = Vec::with_capacity(object_rows.len());
    let mut target_jsons = Vec::with_capacity(object_rows.len());
    let mut idempotency_keys = Vec::with_capacity(object_rows.len());
    for row in object_rows {
        let submission_id: Uuid = row.get("submission_id");
        let object_ref_id: Uuid = row.get("object_ref_id");
        let idempotency_key = sha256_prefixed(
            format!(
                "{}:{tenant_id}:{submission_id}:{object_ref_id}",
                deletion.idempotency_label()
            )
            .as_bytes(),
        );
        propagation_item_ids.push(Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            idempotency_key.as_bytes(),
        ));
        source_submission_ids.push(submission_id);
        item_trace_ids.push(trace_id_of[&submission_id]);
        target_jsons.push(serde_json::json!({
            "kind": "object_ref",
            "object_ref_id": object_ref_id,
        }));
        idempotency_keys.push(idempotency_key);
    }
    let metadata_json = serde_json::json!({"source": "versioned_pipeline"});
    tx.execute(
        "INSERT INTO trace_revocation_propagation_items (
            tenant_id, propagation_item_id, source_submission_id, trace_id,
            target_kind, target_json, action, status, idempotency_key, reason,
            attempt_count, metadata_json
         )
         SELECT $1, item.propagation_item_id, item.source_submission_id, item.trace_id,
                'object_ref', item.target_json, 'delete_object_payload', 'pending',
                item.idempotency_key, $8, 0, $7
           FROM unnest($2::UUID[], $3::UUID[], $4::UUID[], $5::JSONB[], $6::TEXT[])
                AS item(
                    propagation_item_id, source_submission_id, trace_id, target_json,
                    idempotency_key
                )
         ON CONFLICT (tenant_id, idempotency_key) DO NOTHING",
        &[
            &tenant_id,
            &propagation_item_ids,
            &source_submission_ids,
            &item_trace_ids,
            &target_jsons,
            &idempotency_keys,
            &metadata_json,
            &deletion.reason(),
        ],
    )
    .await?;
    Ok(())
}

/// Moves `tenant_id`'s `failed` index invalidations -- only `run_id`'s, when
/// given -- back to `pending`: no attempt charged, no label, due now, not
/// complete. Returns how many invalidations it moved. It writes only the
/// invalidation rows, so it takes no run lock (Zaki review 1, round 2,
/// simplification: the run's copy of the state is gone); a caller that holds
/// run rows, as the enqueue path does, took them before these.
async fn reset_failed_index_invalidations_on_tx(
    tx: &Transaction<'_>,
    tenant_id: &str,
    run_id: Option<Uuid>,
    not_before: Option<DateTime<Utc>>,
) -> Result<u64, DatabaseError> {
    let reset = tx
        .execute(
            "UPDATE pipeline_index_invalidations
                SET state = 'pending', attempt_count = 0, last_error_label = NULL,
                    completed_at = NULL,
                    next_attempt_at = GREATEST(NOW(), COALESCE($3::timestamptz, NOW()))
              WHERE tenant_id = $1 AND state = 'failed'
                AND ($2::UUID IS NULL OR run_id = $2)",
            &[&tenant_id, &run_id, &not_before],
        )
        .await?;
    Ok(reset)
}

/// A refused Review commit as the dispatch re-raises it. The store's
/// `Display` prefixes every `Constraint` error ("Constraint violation:
/// ..."), which would not match the safe-label allowlist verbatim (the same
/// reason `commit_score_phase` re-raises `settlement_adapter_missing` bare);
/// the safe refusals are re-raised as their bare labels.
fn review_commit_refusal(error: DatabaseError) -> anyhow::Error {
    match error {
        DatabaseError::Constraint(label)
            if label == PIPELINE_SUBMISSION_INOPERABLE_LABEL
                || label == PIPELINE_ATTEMPT_ARTIFACT_MISSING_LABEL
                || label == PIPELINE_POLICY_NOT_RUNNABLE_LABEL =>
        {
            anyhow::anyhow!(label)
        }
        error => error.into(),
    }
}

/// Whether a failed phase commit certainly committed nothing: the store
/// refused it (a `Constraint` label -- a stale lease,
/// `settlement_adapter_missing`, `submission_inoperable` -- or another
/// refusal before the commit), no connection was had, or the database
/// answered with an error, which aborts the transaction. A connection lost
/// with no answer may have lost it after the `COMMIT` landed, so the
/// objects the attempt wrote are kept then: a committed row may name them.
fn phase_commit_refused(error: &DatabaseError) -> bool {
    match error {
        DatabaseError::Postgres(error) => error.as_db_error().is_some(),
        _ => true,
    }
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
        privacy_pass_required: row.get("privacy_pass_required"),
        privacy_pass_object_ref_id: row.get("privacy_pass_object_ref_id"),
        privacy_pass_content_hash: row.get("privacy_pass_content_hash"),
        privacy_pass_source_hash: row.get("privacy_pass_source_hash"),
        privacy_pass_residual_risk_basis: json_label_array(
            row,
            "privacy_pass_residual_risk_basis",
        )?,
        privacy_pass_outcome: row
            .get::<_, Option<&str>>("privacy_pass_outcome")
            .map(PrivacyPassOutcome::from_db)
            .transpose()?,
        privacy_pass_recorded_at: row.get("privacy_pass_recorded_at"),
        privacy_pass_approval_assessment_hash: row.get("privacy_pass_approval_assessment_hash"),
        privacy_pass_approval_resolved_reasons: json_label_array(
            row,
            "privacy_pass_approval_resolved_reasons",
        )?,
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
        payout_eligible: row.get("payout_eligible"),
        attempt_count,
        last_error_label: row.get("last_error_label"),
        dispatched_at: row.get("dispatched_at"),
    })
}

fn policy_intervention_from_row(
    row: &Row,
) -> Result<PipelinePolicyInterventionRecord, DatabaseError> {
    Ok(PipelinePolicyInterventionRecord {
        intervention_id: row.get("intervention_id"),
        bundle_id: row.get("bundle_id"),
        phase: phase_from_db(row.get("phase"))?.ok_or_else(|| {
            DatabaseError::Serialization("intervention phase cannot be none".to_string())
        })?,
        action: row.get("action"),
        actor_principal_ref: row.get("actor_principal_ref"),
        reason_code: row.get("reason_code"),
        previous_status: PolicyOperationalStatus::from_db(
            row.get::<_, String>("previous_status").as_str(),
        )?,
        resulting_status: PolicyOperationalStatus::from_db(
            row.get::<_, String>("resulting_status").as_str(),
        )?,
        evidence_hash: row.get("evidence_hash"),
        recorded_at: row.get("recorded_at"),
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

/// `main`'s `credit_withheld_reason` for its skip-duplicate short-circuit.
pub const PIPELINE_SKIPPED_DUPLICATE_LABEL: &str = "skipped_duplicate";
/// `main`'s `credit_withheld_reason` for its canonical-hash cache.
pub const PIPELINE_CACHED_DUPLICATE_LABEL: &str = "cached";

fn is_duplicate_withheld_label(label: &str) -> bool {
    label == PIPELINE_SKIPPED_DUPLICATE_LABEL || label == PIPELINE_CACHED_DUPLICATE_LABEL
}

/// The duplicate verdict a run's persisted Settle selection carries: the
/// label of an `Exclude` that Step 3 of `complete_settle_phase` wrote for a
/// duplicate (`PgPipelineStore::duplicate_verdict_for_selection`), `None`
/// for any other membership. A selection persisted before the verdict moved
/// to Step 3 carries none, and reads as no duplicate.
fn selection_duplicate_verdict(
    selection: &StoredPhaseResult,
) -> anyhow::Result<Option<&'static str>> {
    let decision = serde_json::from_value::<SettleDecision>(selection.decision.clone())
        .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
    Ok(match &decision.index_membership {
        IndexMembershipDecision::Exclude { reason }
            if reason.as_str() == PIPELINE_SKIPPED_DUPLICATE_LABEL =>
        {
            Some(PIPELINE_SKIPPED_DUPLICATE_LABEL)
        }
        IndexMembershipDecision::Exclude { reason }
            if reason.as_str() == PIPELINE_CACHED_DUPLICATE_LABEL =>
        {
            Some(PIPELINE_CACHED_DUPLICATE_LABEL)
        }
        _ => None,
    })
}

/// `main`'s withheld-reason labels for a `NoveltyUtility` credit its checks
/// refuse (`attempt_emit_novelty_utility_credit` in `trace-commons-ingest.rs`,
/// and `CREDIT_CHECK_ERROR_LABEL` there), which a withheld compatibility leg
/// records as its `last_error_label` (Ruling T15-12).
pub const PIPELINE_NOVELTY_UTILITY_NON_PRODUCTION_GATE_LABEL: &str = "non_production_gate";
pub const PIPELINE_NOVELTY_UTILITY_CENTRAL_ISSUER_DENIED_LABEL: &str = "central_issuer_denied";
pub const PIPELINE_NOVELTY_UTILITY_CREDIT_CHECK_ERROR_LABEL: &str = "credit_check_error";
pub const PIPELINE_NOVELTY_UTILITY_POLICY_MISMATCH_LABEL: &str = "policy_mismatch";

/// The error of a rebuilt run whose deadline passed with an index call
/// still running once the write fence margin had passed too
/// (`PipelineService::rebuild_index_run`). Its text is `index_unavailable`,
/// the same as any run past its deadline, so callers and the route see no
/// difference; `rebuild_index_from_authoritative_commands` reads the type
/// to leave the rebuild's fence to expire instead of clearing it.
#[derive(Debug)]
struct IndexRebuildWriteInFlight;

impl std::fmt::Display for IndexRebuildWriteInFlight {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(PIPELINE_INDEX_UNAVAILABLE_LABEL)
    }
}

impl std::error::Error for IndexRebuildWriteInFlight {}

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
    unqualified_routing: bool,
    index_rebuild_fence_margin: std::time::Duration,
    index_write_fence_margin: std::time::Duration,
    /// A test's override of the lease config's privacy pass ceiling.
    privacy_pass_timeout_ceiling: Option<std::time::Duration>,
    /// Score policies an embedding binary registered
    /// (`with_score_policies`); empty by default.
    score_policies: ScorePolicyRegistry,
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
            unqualified_routing: false,
            index_rebuild_fence_margin: std::time::Duration::from_secs(
                PIPELINE_INDEX_WRITE_FENCE_MARGIN_SECONDS.unsigned_abs(),
            ),
            index_write_fence_margin: std::time::Duration::from_secs(
                PIPELINE_INDEX_WRITE_FENCE_MARGIN_SECONDS.unsigned_abs(),
            ),
            privacy_pass_timeout_ceiling: None,
            score_policies: ScorePolicyRegistry::new(),
        }
    }

    /// The Score policies an embedding binary supplies for a substituted
    /// backend, keyed by the Score implementation id a bundle manifest
    /// names (`versioned_pipeline_score_registry`). Defaults to an empty
    /// registry, which is what the ingest binary runs: only the two
    /// built-in families bind, and any other Score id is refused as
    /// `bundle_policy_not_runnable`. A registered factory is a dependency
    /// of every bundle that names it, so `bundle_qualification` reports its
    /// own `production_qualified()` (`runtime_score_policy_not_production`).
    pub fn with_score_policies(mut self, score_policies: ScorePolicyRegistry) -> Self {
        self.score_policies = score_policies;
        self
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

    /// The privacy boundary. `submit` runs its deterministic half
    /// (`rescrub_deterministic`) over the server envelope before it stages
    /// or stores anything, and never its classifier half: the prose-PII
    /// classifier (`rescrub_classifier`) runs in the Review-start privacy
    /// pass. A service built without this fails every receipt closed with
    /// `privacy_control_missing`.
    pub fn with_privacy(mut self, privacy: Arc<dyn PipelinePrivacyBoundary>) -> Self {
        self.privacy = Some(privacy);
        self
    }

    /// The NEAR adapter and configuration the payout pass
    /// (`PipelineService::process_payouts`) uses. A service built without
    /// this pays nothing out, and Score seeds every leg's payout as
    /// `disabled`.
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

    /// Whether a new receipt of a tenant that has no committed routing row
    /// may start a run. Defaults to `false`: a tenant with no row is a legacy
    /// tenant until an operator activates it, and its receipt is
    /// `PipelineReceiptResult::NotRouted`. A committed row always decides
    /// first, whatever this says. This is for tests (decision P5-D5): test
    /// bundles cannot be qualified, so the harnesses set it to route a tenant
    /// they invent without a row.
    pub fn with_unqualified_routing(mut self, allowed: bool) -> Self {
        self.unqualified_routing = allowed;
        self
    }

    /// The write fence margin of the index rebuild
    /// (`PipelineService::rebuild_index_run`): how long a run past its
    /// deadline waits for the index call in flight, and how far past the
    /// run's deadline its committed fence reaches. Defaults to
    /// `PIPELINE_INDEX_WRITE_FENCE_MARGIN_SECONDS`. The margin bounds how
    /// long a run is held, not how long an index call runs: a call that
    /// outlasts it is awaited by `reopen_invalidation_after_index_writes`,
    /// which reopens the run's invalidation when the call returns (issue
    /// #1233). For tests only: a shorter margin lets
    /// a test see a lost rebuild's fence expire. Settle's own margin is
    /// `with_index_write_fence_margin`.
    #[doc(hidden)]
    pub fn with_index_rebuild_fence_margin(mut self, margin: std::time::Duration) -> Self {
        self.index_rebuild_fence_margin = margin;
        self
    }

    /// The write fence margin of Settle's index dispatch
    /// (`dispatch_index_write`): how long a dispatch past its deadline waits
    /// for the index call in flight before it releases the run. Defaults to
    /// `PIPELINE_INDEX_WRITE_FENCE_MARGIN_SECONDS`. For tests only: a
    /// shorter margin lets a test hold an index call past it without
    /// waiting a minute.
    #[doc(hidden)]
    pub fn with_index_write_fence_margin(mut self, margin: std::time::Duration) -> Self {
        self.index_write_fence_margin = margin;
        self
    }

    /// Replaces the privacy pass's ceiling, unbounded, over the lease
    /// config's (`PipelineLeaseConfig::privacy_pass_max_timeout`, decision
    /// P8). For tests only: a short ceiling lets a test see a classifier call
    /// time out. Operators set the lease config's instead.
    #[doc(hidden)]
    pub fn with_privacy_pass_timeout_ceiling(mut self, ceiling: std::time::Duration) -> Self {
        self.privacy_pass_timeout_ceiling = Some(ceiling);
        self
    }

    /// Resolves the default package once, so a service that cannot run its
    /// own default bundle fails at construction rather than on the first
    /// receipt.
    pub fn build(self) -> anyhow::Result<PipelineService> {
        if let Some((adapter, config)) = self.payout.as_ref() {
            // Zaki review 1, round 2, finding 2: `main` refuses to start its
            // NEAR adapters without their credentials under
            // `TRACE_COMMONS_NEAR_CREDIT_REQUIRE_ADAPTER_AUTH`, whatever the
            // settlement mode; so is a payout adapter that presents none.
            anyhow::ensure!(
                !config.controls.require_adapter_auth || adapter.authenticated(),
                PIPELINE_PAYOUT_ADAPTER_AUTH_MISSING_LABEL
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
            // Zaki review 1, fix round, item 1: two more of `main`'s live
            // settlement controls have no equivalent for a pipeline batch.
            // `main` gates each settlement run on recorded rollout-smoke
            // evidence, which Settle has no run to check against; and it caps
            // an account's line per settlement run by leaving events for a
            // later run, which the pipeline's one-batch-per-leg settlement
            // cannot do. Each refuses an enabled payout while it is set.
            anyhow::ensure!(
                !self
                    .novelty_utility_checks
                    .settlement_require_rollout_smoke_ready,
                PIPELINE_PAYOUT_ROLLOUT_SMOKE_NOT_READY_LABEL
            );
            anyhow::ensure!(
                self.novelty_utility_checks
                    .settlement_max_micros_per_account
                    .is_none(),
                PIPELINE_PAYOUT_ACCOUNT_CAP_UNSUPPORTED_LABEL
            );
            // Zaki review 1, round 2, item 2: the two allowlists are process
            // constants, every pipeline batch carries
            // `PIPELINE_SETTLEMENT_POLICY_VERSION`, and the pipeline settles
            // as its one configured issuer. So they are decided here, once:
            // an allowlist that leaves the pipeline out refuses an enabled
            // payout, under `main`'s label for that refusal.
            let checks = &self.novelty_utility_checks;
            anyhow::ensure!(
                checks.settlement_allowed_policy_versions.is_empty()
                    || checks
                        .settlement_allowed_policy_versions
                        .contains(PIPELINE_SETTLEMENT_POLICY_VERSION),
                PIPELINE_PAYOUT_POLICY_VERSION_NOT_ALLOWED_LABEL
            );
            anyhow::ensure!(
                checks.central_issuer_principal_refs.is_empty()
                    || checks
                        .issuer_principal_ref
                        .as_ref()
                        .is_some_and(|issuer| checks
                            .central_issuer_principal_refs
                            .contains(issuer)),
                PIPELINE_NOVELTY_UTILITY_CENTRAL_ISSUER_DENIED_LABEL
            );
        }
        let privacy_pass_timeout = privacy_pass_timeout_for(
            self.lease_config.review(),
            self.privacy
                .as_ref()
                .is_some_and(|privacy| privacy.classifies_prose_pii()),
            self.privacy_pass_timeout_ceiling
                .unwrap_or(self.lease_config.privacy_pass_max_timeout()),
        )?;
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
            crash_pending: Arc::new(AtomicBool::new(self.crash_point.is_some())),
            score_evaluations: AtomicUsize::new(0),
            settle_evaluations: AtomicUsize::new(0),
            authority: self.authority,
            privacy: self.privacy,
            payout: self.payout,
            novelty_utility_checks: self.novelty_utility_checks,
            unqualified_routing: self.unqualified_routing,
            index_rebuild_fence_margin: self.index_rebuild_fence_margin,
            index_write_fence_margin: self.index_write_fence_margin,
            privacy_pass_timeout,
            follow_ups: std::sync::Mutex::new(BTreeMap::new()),
            score_policies: self.score_policies,
            payout_notifier: std::sync::OnceLock::new(),
        };
        service
            .construct(service.default_package.clone())
            .map_err(|label| anyhow::anyhow!(label))?;
        Ok(service)
    }
}

/// Truncates to microsecond precision, the resolution a PostgreSQL
/// `TIMESTAMPTZ` column actually stores (and what `postgres-types` round-trips
/// on the wire). An in-memory `DateTime<Utc>` built from `Utc::now()` can
/// carry nanoseconds (notably on Linux); comparing that untruncated value
/// against one read back from the database can then never observe equality,
/// even when the two represent the database's own idea of "the same instant."
/// `PipelineLeaseRenewal::start` truncates `lease_cap` here so a capped
/// `renew_lease` result compares equal to it and the early exit actually
/// fires, instead of paying for a few extra no-op renewals until the lease
/// naturally expires.
fn truncate_to_microseconds(instant: DateTime<Utc>) -> DateTime<Utc> {
    DateTime::from_timestamp_micros(instant.timestamp_micros()).unwrap_or(instant)
}

/// Renews a claim's lease while its phase runs. Stopped when the phase ends,
/// at the cap, or when the lease is lost. The commit fences
/// (`ensure_current_lease`, `ensure_live_lease`, every lease-checked `UPDATE`)
/// stay the only authority: renewal only keeps an honest slow phase from
/// being reclaimed out from under it before it finishes, never a substitute
/// for those checks.
struct PipelineLeaseRenewal {
    stop: tokio::sync::watch::Sender<bool>,
    join: tokio::task::JoinHandle<()>,
}

impl PipelineLeaseRenewal {
    /// `None` when `run` carries no phase to renew for or no lease token --
    /// both should be impossible for a just-claimed run, but this is a
    /// best-effort background helper, not a correctness gate, so it declines
    /// quietly rather than panicking or failing the claim.
    fn start(
        store: PgPipelineStore,
        run: &PipelineRunRecord,
        config: PipelineLeaseConfig,
    ) -> Option<Self> {
        let phase = run.next_phase?;
        let lease_token = run.lease_token?;
        let lease = config.for_phase(phase);
        let lease_cap =
            truncate_to_microseconds(Utc::now() + lease * PIPELINE_LEASE_RENEWAL_CAP_FACTOR);
        let interval = std::cmp::max(lease / 3, Duration::milliseconds(100))
            .to_std()
            .ok()?;
        let tenant_id = run.tenant_id.clone();
        let run_id = run.run_id;
        let (stop, mut stopped) = tokio::sync::watch::channel(false);
        let join = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {}
                    _ = stopped.changed() => return,
                }
                match store
                    .renew_lease(&tenant_id, run_id, lease_token, lease, lease_cap)
                    .await
                {
                    Ok(Some(expires_at)) if expires_at < lease_cap => {}
                    // Either the cap was reached (no point renewing further:
                    // the phase gives back its claim on its own next
                    // lease-checked write, same as if this task never ran) or
                    // the lease is already gone (lost to a reclaim, or the
                    // phase itself already finished and cleared it) -- either
                    // way, nothing left for this task to do.
                    Ok(_) => return,
                    Err(_) => tracing::warn!(
                        label = "pipeline_lease_renewal_failed",
                        "a lease renewal failed; the next interval tries again"
                    ),
                }
            }
        });
        Some(Self { stop, join })
    }

    /// Signals the background task to stop and waits for it to actually
    /// exit, so no renewal ever races the caller's own next claim of this
    /// run.
    async fn stop(self) {
        let _ = self.stop.send(true);
        if self.join.await.is_err() {
            // Label-only: never log the panic payload (or any other
            // `JoinError` detail) -- a panic message can carry whatever the
            // panicking code formatted into it.
            tracing::warn!(
                label = "pipeline_lease_renewal_task_failed",
                "the lease renewal background task did not exit cleanly"
            );
        }
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
    crash_pending: Arc<AtomicBool>,
    score_evaluations: AtomicUsize,
    settle_evaluations: AtomicUsize,
    authority: Option<Arc<dyn PipelineAuthorityProvider>>,
    privacy: Option<Arc<dyn PipelinePrivacyBoundary>>,
    payout: Option<(Arc<dyn NearPayoutAdapter>, PipelinePayoutConfig)>,
    novelty_utility_checks: PipelineNoveltyUtilityChecks,
    /// Whether a tenant with no committed routing row may start new runs
    /// (`PipelineServiceBuilder::with_unqualified_routing`).
    unqualified_routing: bool,
    /// The index rebuild's write fence margin
    /// (`PipelineServiceBuilder::with_index_rebuild_fence_margin`).
    index_rebuild_fence_margin: std::time::Duration,
    /// Settle's index dispatch write fence margin
    /// (`PipelineServiceBuilder::with_index_write_fence_margin`).
    index_write_fence_margin: std::time::Duration,
    /// The bound on one privacy pass classifier call, computed at build
    /// (`privacy_pass_timeout_for`, decision P8).
    privacy_pass_timeout: std::time::Duration,
    /// The follow-up steps this service queued work for, per tenant, since
    /// the worker last took them (`take_follow_ups`).
    follow_ups: std::sync::Mutex<BTreeMap<String, PipelineFollowUps>>,
    /// Score policies an embedding binary registered
    /// (`PipelineServiceBuilder::with_score_policies`).
    score_policies: ScorePolicyRegistry,
    /// Wakes the payout rails after a credit batch is finalized
    /// (`attach_payout_notifier`; payout spec section 7). Unset, nothing is
    /// woken; the rails' retry tick still runs.
    payout_notifier: std::sync::OnceLock<crate::payout_worker::PayoutNotifier>,
}

/// What one run's payout attempt came to (`process_payout_on`).
enum PayoutAttempt {
    /// The run does not exist.
    NoRun,
    /// The attempt ran to its end: a confirmation lookup, a submit, or
    /// nothing that was due.
    Done(PipelineRunRecord),
    /// A submit was held back because the Settle policy of the run's bundle
    /// is not runnable (GRD-004). The run's payout is as it was; the pass
    /// does not count it.
    Held(PipelineRunRecord),
}

/// The payout's policy guard (GRD-004: the NEAR worker repeats its policy
/// guard before dispatch): whether the Settle policy of the run's own bundle
/// is runnable, so that a line may be sent to the adapter. No status row is
/// not runnable, as at the start of a phase (`policy_is_runnable`). It is
/// called immediately before a submit, by the one function both payout
/// callers reach the submit through (`dispatch_near_settlements`), and
/// never before a confirmation lookup.
///
/// The read is a short tenant transaction on `client`, the connection the
/// payout already holds, so it takes no second connection (the pool-size-one
/// rule) and no lock: the tenant's NEAR submit lock, which the caller holds
/// across every submit, is unchanged, and nothing is held over the adapter
/// call. A suspension that lands after the read takes effect at the next
/// dispatch; a guard does not claim to retract an operation the adapter
/// already accepted.
async fn settle_policy_allows_dispatch(
    client: &mut deadpool_postgres::Client,
    run: &PipelineRunRecord,
) -> anyhow::Result<bool> {
    let tx = PgPipelineStore::tenant_transaction(client, &run.tenant_id).await?;
    let runnable = tx
        .query_opt(
            "SELECT runnable FROM pipeline_bundle_policy_status
              WHERE tenant_id = $1 AND bundle_id = $2 AND phase = 'settle'",
            &[&run.tenant_id, &run.bundle_id],
        )
        .await?
        .is_some_and(|row| row.get::<_, bool>("runnable"));
    tx.commit().await?;
    Ok(runnable)
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
) -> anyhow::Result<(String, Vec<TraceCreditAccountSettlementLineItem>)> {
    let tx = PgPipelineStore::tenant_transaction(client, &run.tenant_id).await?;
    let row = tx
        .query_opt(
            "SELECT source_list_hash, line_items_json
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
    Ok((row.get("source_list_hash"), lines))
}

/// The hash an outbox line stores in `last_error_hash` when its transaction
/// failed on chain (`PIPELINE_NEAR_TRANSACTION_FAILED_LABEL`).
fn near_transaction_failed_hash() -> String {
    sha256_prefixed(PIPELINE_NEAR_TRANSACTION_FAILED_LABEL.as_bytes())
}

/// The status, `last_error_hash` and stored call of one outbox line, or
/// `None` when it has no row yet.
async fn near_outbox_line(
    client: &mut deadpool_postgres::Client,
    run: &PipelineRunRecord,
    outbox_id: Uuid,
) -> anyhow::Result<Option<(String, Option<String>, serde_json::Value)>> {
    let tx = PgPipelineStore::tenant_transaction(client, &run.tenant_id).await?;
    let line = tx
        .query_opt(
            "SELECT status, last_error_hash, near_call_json
               FROM trace_near_credit_outbox
              WHERE tenant_id = $1 AND near_outbox_id = $2",
            &[&run.tenant_id, &outbox_id],
        )
        .await?
        .map(|row| {
            (
                row.get("status"),
                row.get("last_error_hash"),
                row.get("near_call_json"),
            )
        });
    tx.commit().await?;
    Ok(line)
}

/// Writes one outbox line `pending`, for `instrument_id = 'trace_credit'`;
/// a line that already exists is left as it is.
#[allow(clippy::too_many_arguments)]
async fn insert_near_outbox_line(
    client: &mut deadpool_postgres::Client,
    run: &PipelineRunRecord,
    settlement: &PipelineSettlementRecord,
    batch_id: Uuid,
    outbox_id: Uuid,
    credit_account_hash: &str,
    payout_near_account_id: Option<&str>,
    call: &crate::near_credit::NearCreditReceiptCall,
) -> anyhow::Result<()> {
    let call_json = serde_json::to_value(call)
        .map_err(|_| anyhow::anyhow!(PIPELINE_NEAR_CALL_INVALID_LABEL))?;
    let tx = PgPipelineStore::tenant_transaction(client, &run.tenant_id).await?;
    tx.execute(
        "INSERT INTO trace_near_credit_outbox (
            tenant_id, near_outbox_id, settlement_batch_id, credit_account_hash,
            near_call_json, status, payout_near_account_id, instrument_id
         ) VALUES ($1,$2,$3,$4,$5,'pending',$6,$7)
         ON CONFLICT (tenant_id, near_outbox_id) DO NOTHING",
        &[
            &run.tenant_id,
            &outbox_id,
            &batch_id,
            &credit_account_hash,
            &call_json,
            &payout_near_account_id,
            &settlement.instrument_id,
        ],
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// The NEAR account a batch line is paid to, as `main`'s credit settlement
/// resolves it: `Ok(None)` for a principal's line (`main` pays it with no
/// target), the account's designated or sole active NEAR account for an
/// account's line, and `Err` with `main`'s hold label when the account has
/// none or several with none designated. Resolved on `client`, the
/// connection the payout already holds.
async fn payout_target_for_line(
    client: &mut deadpool_postgres::Client,
    run: &PipelineRunRecord,
    line: &TraceCreditAccountSettlementLineItem,
) -> anyhow::Result<Result<Option<String>, &'static str>> {
    let Some(account_id) = crate::db::settlement_key_account_id(&line.credit_account_ref) else {
        return Ok(Ok(None));
    };
    let tx = PgPipelineStore::tenant_transaction(client, &run.tenant_id).await?;
    let resolution =
        crate::db::postgres::resolve_payout_near_account_id_on_tx(&tx, account_id).await?;
    tx.commit().await?;
    Ok(match resolution {
        crate::db::PayoutResolution::Designated(near)
        | crate::db::PayoutResolution::SoleActive(near) => Ok(Some(near)),
        crate::db::PayoutResolution::Hold(reason) => Err(reason.label()),
    })
}

/// Records a line's submit: `submitted`, with the hash of the adapter's
/// transaction reference. Only a `pending` or `failed` line moves; the
/// columns change as `update_trace_near_credit_outbox_status` changes them.
async fn mark_near_outbox_line_submitted(
    client: &mut deadpool_postgres::Client,
    run: &PipelineRunRecord,
    outbox_id: Uuid,
    near_transaction_hash: &str,
    submission_mode: &str,
) -> anyhow::Result<()> {
    let tx = PgPipelineStore::tenant_transaction(client, &run.tenant_id).await?;
    // The mode that submitted the line rides in its stored call
    // (`PIPELINE_NEAR_SUBMISSION_MODE_KEY`), which `main`'s call type
    // ignores; only that mode confirms it (Zaki review 3, Z3-L2).
    tx.execute(
        "UPDATE trace_near_credit_outbox
            SET status = 'submitted',
                near_transaction_hash = $3,
                near_call_json = jsonb_set(
                    near_call_json, '{pipeline_submission_mode}', to_jsonb($4::TEXT), TRUE
                ),
                submitted_at = COALESCE(submitted_at, NOW()),
                confirmed_at = NULL,
                last_error_hash = NULL
          WHERE tenant_id = $1 AND near_outbox_id = $2
            AND status IN ('pending', 'failed')",
        &[
            &run.tenant_id,
            &outbox_id,
            &near_transaction_hash,
            &submission_mode,
        ],
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
/// `near_submit_failed` (`near_transaction_failed` when each `failed` line's
/// stored hash is that label's), every line `submitted` or `confirmed` is
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
            "SELECT status, last_error_hash
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
        .map(|row| {
            (
                row.get::<_, String>("status"),
                row.get::<_, Option<String>>("last_error_hash"),
            )
        })
        .collect::<Vec<_>>();
    tx.commit().await?;
    let transaction_failed_hash = near_transaction_failed_hash();
    let (payout, label) =
        if !statuses.is_empty() && statuses.iter().all(|(status, _)| status == "confirmed") {
            (TraceCreditSettlementNearStatus::Confirmed, None)
        } else if statuses.iter().any(|(status, _)| status == "failed") {
            // A pipeline batch has one line now, so a leg cannot hold both
            // failure hashes. If it ever does, the leg reads `near_submit_failed`:
            // the runbook has the operator check that line against NEAR by hand.
            let transaction_failed = statuses
                .iter()
                .filter(|(status, _)| status == "failed")
                .all(|(_, hash)| hash.as_deref() == Some(transaction_failed_hash.as_str()));
            (
                TraceCreditSettlementNearStatus::Failed,
                Some(if transaction_failed {
                    PIPELINE_NEAR_TRANSACTION_FAILED_LABEL
                } else {
                    PIPELINE_NEAR_SUBMIT_FAILED_LABEL
                }),
            )
        } else if !statuses.is_empty()
            && statuses
                .iter()
                .all(|(status, _)| status == "submitted" || status == "confirmed")
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

/// The bytes inside a P1 wrapper, or `None` when `plaintext` is not one.
/// For a reader that loads a submission's latest active envelope object,
/// which is a bare envelope for a gate-path submission and this wrapper for
/// one a pipeline run has stored. A wrapper that names the schema but does
/// not decode is refused with the same safe label.
pub fn pipeline_artifact_inner_bytes(plaintext: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(plaintext) else {
        return Ok(None);
    };
    if value.get("schema").and_then(serde_json::Value::as_str)
        != Some("trace_commons.pipeline_artifact_bytes.v1")
    {
        return Ok(None);
    }
    decode_pipeline_artifact_bytes(&value).map(Some)
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

/// How many due rows one attempt sweep pass examines for each row it may
/// remove (`PipelineService::sweep_attempt_artifacts`'s `limit`): enough to
/// go on past rows it keeps, and a bound on one pass's work.
pub const PIPELINE_ATTEMPT_SWEEP_EXAMINED_PER_REMOVAL: usize = 4;

/// The key of the last due attempt row a sweep pass examined, in the
/// sweep's order: the keyset cursor its next page, or the next pass
/// (`PipelineService::sweep_attempt_artifacts_from`), reads after. Opaque
/// outside this module; it holds no tenant id and is never logged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptSweepCursor {
    cleanup_after: DateTime<Utc>,
    run_id: Uuid,
    lease_token: Uuid,
    artifact: String,
}

/// The sweep could not delete a due attempt row's object (or could not
/// tell whether it is gone); the row stays for the next pass.
const PIPELINE_ATTEMPT_SWEEP_DELETE_FAILED: &str = "pipeline_attempt_sweep_delete_failed";
/// A due attempt row with no hash names an object key other than the one
/// the store derives from the row's artifact, run and lease token (rebase
/// 10 review, M1); the row and that object stay.
const PIPELINE_ATTEMPT_SWEEP_KEY_MISMATCH: &str = "pipeline_attempt_sweep_key_mismatch";

/// What `PipelineService::sweep_attempt_artifacts` does with one due row
/// once it has handled the row's object.
enum AttemptSweepRow {
    /// The object is gone; the row is deleted.
    Remove,
    /// The row stays, and the sweep logs this label, with the store's own
    /// refusal label when it gave one.
    Keep(&'static str, Option<String>),
}

/// The store's own refusal, when its text is a label (`^[a-z0-9_]{1,64}$`):
/// the sweep logs it beside `pipeline_attempt_sweep_delete_failed`, so an
/// operator can tell `artifact_delete_at_object_key_unavailable`,
/// `remote_trace_artifact_delete_at_key_unavailable`, and a store's other
/// refusals apart (review of the follow-up wave, m2). Any other error text
/// is left out: operational output is label-only.
fn store_refusal_label(error: &anyhow::Error) -> Option<String> {
    let text = error.to_string();
    crate::versioned_pipeline_qualification::is_safe_label(&text).then_some(text)
}

/// The attempt rows a compatibility Score staged before it took its
/// tenant's Score lock (rebase 10, option D;
/// `PipelineService::stage_score_artifacts_before_lock`): each artifact's
/// object key, and the moment after which the attempt publishes nothing
/// (`PipelineService::attempt_artifact_lease_bound`).
struct PrestagedScoreArtifacts {
    object_keys: Vec<(PipelineAttemptArtifact, String)>,
    publish_deadline: DateTime<Utc>,
}

impl PrestagedScoreArtifacts {
    fn object_key(&self, artifact: PipelineAttemptArtifact) -> Option<&str> {
        self.object_keys
            .iter()
            .find(|(staged, _)| *staged == artifact)
            .map(|(_, object_key)| object_key.as_str())
    }
}

/// The object id a claim stores one of its run's phase artifacts under
/// (`artifact` is `approved`, `index-command`, `score-neighbors`, or
/// `privacy-pass`).
///
/// Ruling FR1: the id carries the claim's lease token, not the run id
/// alone. Every write encrypts with a fresh salt and nonce, so two writes of
/// equal plaintext under one key still differ in ciphertext; a worker whose
/// lease expired during policy work must therefore never write the key a
/// later claim committed, or the committed `content_sha256` would stop
/// matching the stored object. The database refs a commit records stay
/// deterministic (the approved object ref id is derived from the run id
/// alone); only the object key moves per claim. A phase attempt that
/// crashes before its commit, loses its lease, or has its commit refused
/// leaves its objects named by a `pipeline_attempt_artifacts` `staged` row
/// (PR 4); `PipelineService::sweep_attempt_artifacts` deletes both once the
/// row's `cleanup_after` passes. The receipt's source object is tracked
/// separately: see `pipeline_receipt_object_id`.
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

/// The transaction-scoped advisory lock that serializes a tenant's
/// compatibility Score from its neighbour read through its commit (Zaki
/// review 1, round 2, finding 12).
/// Fails as an injected crash when `point` is the service's crash point and
/// the crash has not happened yet (`PipelineServiceBuilder::with_crash_point`).
fn inject_crash_at(
    crash_point: Option<PipelineCrashPoint>,
    crash_pending: &AtomicBool,
    point: PipelineCrashPoint,
) -> anyhow::Result<()> {
    if crash_point == Some(point) && crash_pending.swap(false, Ordering::SeqCst) {
        anyhow::bail!(INJECTED_PIPELINE_CRASH);
    }
    Ok(())
}

/// What Settle's index dispatch task needs, owned, so the task outlives a
/// dropped Settle future (`dispatch_index_write`).
struct IndexDispatch {
    backend: Arc<PgBackend>,
    writer: Arc<dyn IdentifiedIndexWriter>,
    run: PipelineRunRecord,
    command: anyhow::Result<Option<SealedIndexCommand>>,
    crash_point: Option<PipelineCrashPoint>,
    crash_pending: Arc<AtomicBool>,
    /// How long a dispatch past its deadline waits for the call in flight
    /// before it releases the run
    /// (`PipelineServiceBuilder::with_index_write_fence_margin`).
    margin: std::time::Duration,
}

/// How Settle's index dispatch ended. `WriteFailed`'s transaction was rolled
/// back, so `pending` stays.
enum IndexDispatchOutcome {
    Cancelled(PipelineRunRecord),
    Complete(PipelineRunRecord),
    WriteFailed(IndexWriteError),
    /// The Settle policy of the run's bundle was not runnable under its
    /// status row's lock (GRD-004): no upsert started, the transaction wrote
    /// nothing, and `pending` stays.
    PolicyNotRunnable,
}

/// The safe label logged when a late index write reopened its run's
/// invalidation (`reopen_invalidation_after_index_writes`).
const PIPELINE_INDEX_WRITE_LANDED_LATE_LABEL: &str = "pipeline_index_write_landed_late";
/// The safe label logged when that reopen could not be written and is
/// tried again.
const PIPELINE_INDEX_WRITE_REOPEN_RETRY_LABEL: &str = "pipeline_index_write_reopen_retry";
/// The longest wait between two tries of a reopen that failed.
const PIPELINE_INDEX_WRITE_REOPEN_MAX_BACKOFF: std::time::Duration =
    std::time::Duration::from_secs(60);

/// Awaits index writes that outlasted their deadline and their fence margin
/// (a Settle dispatch's, or a rebuilt run's), however long they take, and
/// then reopens the run's index invalidation
/// (`PgPipelineStore::reopen_index_invalidation_after_late_write`).
///
/// The invariant (issue #1233): a withdrawal or revocation is never undone
/// by an index write that started before it, however long that write
/// takes. A write holds the run's row locks while it is inside its
/// deadline, so a withdrawal waits for it (or commits first, and the write
/// is cancelled). Past the deadline the locks go while the call may still
/// be running, and no clock bounds that call: an index call has no time
/// limit of its own. So the guarantee does not rest on a margin or on any
/// clock. Each such write is awaited here until it returns, and only then
/// is the run's invalidation reopened, so a removal that ran while the call
/// was in flight runs again after the entry landed. An invalidation queued
/// after the reopen's statement was queued after the write returned, and
/// runs after it. The window in which a late entry is searchable is from
/// its landing to the next invalidation pass.
///
/// The reopen is tried until it is written, with a doubling backoff capped
/// at `PIPELINE_INDEX_WRITE_REOPEN_MAX_BACKOFF`. What this does not cover:
/// a process that exits between the late write's landing and the reopen's
/// commit. An in-process index call ends with the process; a remote one
/// that lands after the process is gone is outside what the pipeline can
/// observe.
async fn reopen_invalidation_after_index_writes<T>(
    store: PgPipelineStore,
    tenant_id: String,
    run_id: Uuid,
    writes: tokio::task::JoinHandle<T>,
) {
    // The writes' own result does not matter: an error or a panic may
    // follow an entry that was written.
    let _ = writes.await;
    let mut backoff = std::time::Duration::from_secs(1);
    loop {
        match store
            .reopen_index_invalidation_after_late_write(&tenant_id, run_id)
            .await
        {
            Ok(reopened) => {
                if reopened {
                    tracing::warn!(
                        label = PIPELINE_INDEX_WRITE_LANDED_LATE_LABEL,
                        "an index write returned after its fence margin; its run's index \
                         invalidation runs again"
                    );
                }
                return;
            }
            Err(_) => {
                tracing::warn!(
                    label = PIPELINE_INDEX_WRITE_REOPEN_RETRY_LABEL,
                    "could not reopen a late index write's invalidation; retrying"
                );
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(PIPELINE_INDEX_WRITE_REOPEN_MAX_BACKOFF);
            }
        }
    }
}

/// The time left before `lease_expires_at`, read on the database's clock
/// (`clock_timestamp()`), the clock that set it, and never negative: a lease
/// that has ended has none left. Settle's index dispatch builds its
/// deadline from this (`index_dispatch_deadline`), so a step of the
/// process's wall clock cannot move it (issue #1233). Public for the
/// runtime tests only.
#[doc(hidden)]
pub async fn index_lease_remaining_on_db_clock<C: deadpool_postgres::GenericClient>(
    client: &C,
    lease_expires_at: DateTime<Utc>,
) -> Result<std::time::Duration, tokio_postgres::Error> {
    let milliseconds: i64 = client
        .query_one(
            "SELECT GREATEST(
                        0,
                        (EXTRACT(EPOCH FROM ($1::timestamptz - clock_timestamp())) * 1000)::bigint
                    )",
            &[&lease_expires_at],
        )
        .await?
        .get(0);
    Ok(std::time::Duration::from_millis(
        milliseconds.unsigned_abs(),
    ))
}

/// Settle's index dispatch deadline on the monotonic clock: `now` plus the
/// earlier of the lease's remaining time
/// (`index_lease_remaining_on_db_clock`) and the dispatch budget
/// (`PIPELINE_INDEX_DISPATCH_BUDGET_SECONDS`, multi-lens review L4-1). No
/// upsert starts at or past it (issue #1233).
fn index_dispatch_deadline(
    now: std::time::Instant,
    lease_remaining: std::time::Duration,
    budget: std::time::Duration,
) -> std::time::Instant {
    now + lease_remaining.min(budget)
}

/// Settle's index dispatch (Step 5), run in a task of its own (multi-lens
/// review L4-3). One transaction locks the run row (`ensure_current_lease`),
/// then the submission row (the guard), then the Settle policy's status row
/// `FOR SHARE` (GRD-004: a policy that is not runnable starts no upsert, and a
/// suspension waits for this transaction), and holds all three through every
/// upsert to the commit of the `index_write_state` it records. A withdrawal
/// locks the submission row `FOR UPDATE`, so it either commits before the
/// guard read (the dispatch then sees it, cancels, and queues an
/// invalidation for whatever an earlier attempt wrote) or waits until the
/// write and its `complete` have committed.
///
/// The writes are synchronous index calls on the blocking pool (N-6), and
/// the writer does not see the row locks. The task owns the transaction and
/// awaits the writes, so a dropped Settle future does not end it early. The
/// locks can still go while the writes run -- a lost database session, or a
/// process exit, which drops this task with the runtime while the blocking
/// writes go on -- so the writes also stop at the deadline: no upsert starts
/// once the earlier of `lease_expires_at` and the dispatch budget
/// (`PIPELINE_INDEX_DISPATCH_BUDGET_SECONDS`, multi-lens review L4-1) has
/// passed. The deadline is on the monotonic clock (`std::time::Instant`),
/// and the lease's remaining time is read from the database's clock, the
/// one that set `lease_expires_at`, so a step of the process's wall clock
/// cannot move it (issue #1233). At the deadline the transaction rolls back
/// at once, so the rows are free, and the run is released once the upsert
/// in flight has returned, or once the fence margin has passed. A task
/// that did not return may have written part of the command, as an
/// `Uncertain` answer may.
///
/// An upsert has no time limit of its own, so one still running once the
/// margin has passed is not fenced by any clock: it is awaited by a task of
/// its own, which reopens the run's invalidation when it returns
/// (`reopen_invalidation_after_index_writes`, which states the invariant).
/// A withdrawal that commits while it runs therefore cannot leave its entry
/// in the index.
async fn dispatch_index_write(dispatch: IndexDispatch) -> anyhow::Result<IndexDispatchOutcome> {
    let IndexDispatch {
        backend,
        writer,
        run,
        command,
        crash_point,
        crash_pending,
        margin,
    } = dispatch;
    let mut client = backend.trace_pool().get().await?;
    let tx = PgPipelineStore::tenant_transaction(&mut client, &run.tenant_id).await?;
    // Lock order: the run row (`ensure_current_lease`), then the submission
    // row (the guard), then the Settle policy row (below).
    ensure_current_lease(&tx, &run, required_lease_token(&run)?).await?;
    let guard = PgPipelineStore::submission_guard_on_tx(&tx, &run).await?;
    if !guard.operable {
        // `pending` may be partly written: an earlier attempt can have
        // applied entries and then rolled back. So the cancel queues an
        // invalidation of the revision in the same transaction (a no-op for
        // the index if nothing was written). This task holds the lease and
        // writes nothing, so the invalidation is due at once.
        PgPipelineStore::enqueue_index_invalidation_on_tx(
            &tx,
            &run,
            PIPELINE_SUBMISSION_INOPERABLE_LABEL,
            None,
        )
        .await?;
        let run = PgPipelineStore::set_index_write_state_on_tx(&tx, &run, "cancelled").await?;
        tx.commit().await?;
        return Ok(IndexDispatchOutcome::Cancelled(run));
    }
    // GRD-004: the Settle policy's status row, `FOR SHARE`, after the run row
    // and the submission row, held through every upsert to the commit of the
    // `index_write_state` -- the same span as those two rows, so a suspension
    // waits for a dispatch in flight (at most the earlier of the lease's end
    // and the dispatch budget) and a dispatch that starts after one starts
    // no upsert. An inoperable submission cancels above whatever the policy
    // says: that path ends the write and queues its invalidation, and must run
    // in every policy state.
    if !lock_runnable_policy(&tx, &run.tenant_id, &run.bundle_id, Phase::Settle).await? {
        tx.commit().await?;
        return Ok(IndexDispatchOutcome::PolicyNotRunnable);
    }
    let command = command?.ok_or_else(|| anyhow::anyhow!("index_command_invalid"))?;
    // Multi-lens review L4-1: the rows are held no longer than the earlier
    // of the lease's end and the dispatch budget. Issue #1233: the lease's
    // remaining time is read from the database's clock, which set
    // `lease_expires_at`, and the deadline is on the monotonic clock, so the
    // process's wall clock plays no part.
    let lease_expires_at = run.lease_expires_at.ok_or_else(stale_lease_error)?;
    let lease_remaining = index_lease_remaining_on_db_clock(&tx, lease_expires_at).await?;
    let deadline = index_dispatch_deadline(
        std::time::Instant::now(),
        lease_remaining,
        std::time::Duration::from_secs(PIPELINE_INDEX_DISPATCH_BUDGET_SECONDS.unsigned_abs()),
    );
    let tenant = pipeline_tenant_storage_ref(&run.tenant_id);
    // Each entry is written under its own key: the command pairs them, so
    // one entry is never stored under another's key.
    let entries = command
        .keyed_entries(&tenant)
        .map(|(key, entry)| (key, entry.embedding.clone(), entry.content_hash.clone()))
        .collect::<Vec<_>>();
    let mut writes = tokio::task::spawn_blocking(move || {
        for (key, embedding, content_hash) in &entries {
            if std::time::Instant::now() >= deadline {
                return Err(IndexWriteError::Uncertain);
            }
            writer.upsert(key, embedding, content_hash)?;
        }
        Ok(())
    });
    let budget = deadline.saturating_duration_since(std::time::Instant::now());
    let written = match tokio::time::timeout(budget, &mut writes).await {
        Ok(joined) => joined.unwrap_or(Err(IndexWriteError::Uncertain)),
        Err(_) => {
            // Past the deadline: roll back at once, so the rows are free.
            // A task of its own awaits the call in flight, however long it
            // runs, and then reopens the run's invalidation (issue #1233).
            // The run is released once that task ended or the fence margin
            // passed; dropping the handle leaves the task running.
            drop(tx);
            drop(client);
            let reopen = tokio::spawn(reopen_invalidation_after_index_writes(
                PgPipelineStore::new(backend.clone()),
                run.tenant_id.clone(),
                run.run_id,
                writes,
            ));
            let _ = tokio::time::timeout(margin, reopen).await;
            return Ok(IndexDispatchOutcome::WriteFailed(
                IndexWriteError::Uncertain,
            ));
        }
    };
    if let Err(error) = written {
        // Rolled back: `pending` stays.
        drop(tx);
        return Ok(IndexDispatchOutcome::WriteFailed(error));
    }
    // A crash here drops the transaction, so `pending` stays; the retry
    // applies the same entries again, which the index reports as unchanged.
    inject_crash_at(
        crash_point,
        &crash_pending,
        PipelineCrashPoint::AfterIndexApply,
    )?;
    let run = PgPipelineStore::set_index_write_state_on_tx(&tx, &run, "complete").await?;
    tx.commit().await?;
    Ok(IndexDispatchOutcome::Complete(run))
}

fn pipeline_compatibility_score_lock(tenant_id: &str) -> String {
    format!("pipeline-compatibility-score:{tenant_id}")
}

/// The transaction-scoped advisory lock that serializes a receipt key's
/// staging and final transactions.
fn pipeline_receipt_lock(tenant_id: &str, request_idempotency_key: &str) -> String {
    format!("pipeline-receipt:{tenant_id}:{request_idempotency_key}")
}

/// The `hashtextextended` seed of the routing lock. A seed names a lock
/// class: the receipt lock uses 0, the quota and bundle registry locks use 1,
/// so a tenant's routing lock key can never collide with one of those.
/// Exclusive holders (`PipelineActivationStore`) and shared holders (a
/// receipt's transactions) both bind this constant, so they cannot disagree.
pub(crate) const PIPELINE_ROUTING_LOCK_SEED: i64 = 2;

/// The transaction-scoped advisory lock key of a tenant's routing. A routing
/// change takes it exclusively; a receipt's staging and commit transactions
/// take it shared, so a routing change waits for the receipts in those
/// transactions and no later receipt misses it.
pub(crate) fn pipeline_routing_lock(tenant_id: &str) -> String {
    format!("pipeline-routing:{tenant_id}")
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

/// Builds the `trace_object_refs` write for the Review-start privacy
/// pass's output object. Like `approved_object_ref`, the object ref id is
/// derived from the run id alone, so every attempt of the run records the
/// identical id and only the object key moves per claim (Ruling FR1; spec
/// correction 1). The kind is `review_snapshot`, which no legacy by-name
/// selector picks up and which the withdrawal's deletion maps to a
/// contribution envelope (decision P2); `created_by_job_id` is the run,
/// which tells it apart from the approved ref (`None`).
pub fn privacy_pass_object_ref(
    run: &PipelineRunRecord,
    receipt: &EncryptedTraceArtifactReceipt,
    size_bytes: usize,
    object_store: &str,
) -> TraceObjectRefWrite {
    TraceObjectRefWrite {
        object_ref_id: Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("tracecommons:pipeline-privacy-pass-object:{}", run.run_id).as_bytes(),
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
        created_by_job_id: Some(run.run_id),
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

// Declared after every `macro_rules!` above, so the child modules see them.
mod pg_store;
mod service;
mod types;

pub use self::types::*;

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #1233, defect 2: Settle's index dispatch deadline is a
    /// monotonic `Instant`, the earlier of the lease's remaining time (read
    /// on the database's clock) and the dispatch budget, from `now`.
    #[test]
    fn the_index_dispatch_deadline_is_the_earlier_of_the_lease_and_the_budget() {
        let now = std::time::Instant::now();
        let s = std::time::Duration::from_secs;
        assert_eq!(index_dispatch_deadline(now, s(5), s(30)), now + s(5));
        assert_eq!(index_dispatch_deadline(now, s(300), s(30)), now + s(30));
        assert_eq!(
            index_dispatch_deadline(now, std::time::Duration::ZERO, s(30)),
            now,
            "a lease with no time left starts no upsert"
        );
    }

    #[test]
    fn the_sweep_logs_a_store_refusal_only_when_it_is_a_label() {
        assert_eq!(
            store_refusal_label(&anyhow::anyhow!(
                "artifact_delete_at_object_key_unavailable"
            )),
            Some("artifact_delete_at_object_key_unavailable".to_string())
        );
        assert_eq!(
            store_refusal_label(&anyhow::anyhow!(
                "remote_trace_artifact_delete_at_key_unavailable"
            )),
            Some("remote_trace_artifact_delete_at_key_unavailable".to_string())
        );
        for unsafe_text in [
            "failed to delete gs://bucket/tenants/abc/object",
            "Permission denied (os error 13)",
            "",
            &"a".repeat(65),
        ] {
            assert_eq!(
                store_refusal_label(&anyhow::anyhow!(unsafe_text.to_string())),
                None,
                "{unsafe_text}"
            );
        }
        // A wrapped refusal's outer text is what is tested, never its cause.
        let wrapped = anyhow::anyhow!("artifact_delete_at_object_key_unavailable")
            .context("deleting tenant-a's object at a key");
        assert_eq!(store_refusal_label(&wrapped), None);
    }

    /// Wave 2 (PR 3's c8d65fcb with PR 4's 6dace131): every failed store
    /// call of the run path that is not an integrity failure (ZA-2) is one
    /// transient `PolicyError`, the uncharged suspension. A capability the store lacks keeps its own label; any
    /// other failure, a safe-looking label included, is
    /// `artifact_store_unavailable`, and no other store text reaches the run.
    #[test]
    fn a_failed_store_call_is_one_uncharged_suspension() {
        for (message, label) in [
            (
                PIPELINE_OBJECT_KEY_UNAVAILABLE_LABEL,
                PIPELINE_OBJECT_KEY_UNAVAILABLE_LABEL,
            ),
            (
                "object store unavailable",
                PIPELINE_ARTIFACT_STORE_UNAVAILABLE_LABEL,
            ),
            (
                "artifact_delete_at_object_key_unavailable",
                PIPELINE_ARTIFACT_STORE_UNAVAILABLE_LABEL,
            ),
            (
                "failed to write gs://bucket/tenants/abc/object",
                PIPELINE_ARTIFACT_STORE_UNAVAILABLE_LABEL,
            ),
        ] {
            let failure = artifact_store_failure(&anyhow::anyhow!(message.to_string()));
            assert!(failure.is_transient(), "{message}");
            assert_eq!(failure.label(), label, "{message}");
        }
        let wrapped = anyhow::anyhow!(PIPELINE_OBJECT_KEY_UNAVAILABLE_LABEL)
            .context("deriving tenant-a's object key");
        assert_eq!(
            artifact_store_failure(&wrapped).label(),
            PIPELINE_ARTIFACT_STORE_UNAVAILABLE_LABEL
        );
    }

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

    /// `truncate_to_microseconds` drops any nanosecond remainder (what a
    /// Linux clock can hand `Utc::now()`, and what a `TIMESTAMPTZ` column
    /// cannot store) while leaving a value already at microsecond precision
    /// -- the shape a value read back from PostgreSQL always has -- alone,
    /// so the two can compare equal.
    #[test]
    fn truncate_to_microseconds_drops_the_sub_microsecond_remainder() {
        let with_nanos = DateTime::from_timestamp(1_700_000_000, 123_456_789).unwrap();
        let truncated = truncate_to_microseconds(with_nanos);
        assert_eq!(truncated.timestamp_micros(), 1_700_000_000_123_456);
        assert_eq!(truncated.timestamp_subsec_nanos(), 123_456_000);
        assert!(truncated <= with_nanos);

        // Already at microsecond precision (as anything read back from
        // PostgreSQL is): truncating again changes nothing.
        assert_eq!(truncate_to_microseconds(truncated), truncated);
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
                payout_eligible: false,
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

    /// The operator-action rule is one function for every caller, and a
    /// refusal carries the label its caller passed, never text of the actor
    /// or the reason.
    #[test]
    fn the_operator_action_rule_refuses_with_the_callers_label() {
        let actor = format!("operator_sha256:{}", "a".repeat(64));
        assert!(validate_actor(&actor, "suspend_policy", "any_label").is_ok());
        let too_long = "a".repeat(161);
        for (actor, reason) in [
            (actor.as_str(), "Bad Reason"),
            (actor.as_str(), ""),
            (actor.as_str(), "has space"),
            ("", "suspend_policy"),
            ("an actor with spaces", "suspend_policy"),
            ("actor/with/slashes", "suspend_policy"),
            (too_long.as_str(), "suspend_policy"),
        ] {
            for label in [
                PIPELINE_POLICY_INTERVENTION_INVALID_LABEL,
                "activation_actor_invalid",
            ] {
                let refused = validate_actor(actor, reason, label)
                    .expect_err("an invalid actor or reason is refused");
                assert!(
                    matches!(&refused, DatabaseError::Constraint(text) if text == label),
                    "actor {actor:?} with reason {reason:?} is refused under {label}: {refused:?}"
                );
            }
        }
    }

    /// Task 2: the receipt's residual-risk mapping, factored so the
    /// Review-start privacy pass maps the same way. A Medium whose only
    /// basis is the consent content flag is not a PII finding (Low); any
    /// other Medium stays Medium.
    #[test]
    fn pipeline_privacy_risk_maps_like_the_receipt() {
        assert_eq!(
            pipeline_privacy_risk(&ResidualPiiRisk::Low, &[]),
            PrivacyRisk::Low
        );
        assert_eq!(
            pipeline_privacy_risk(
                &ResidualPiiRisk::Medium,
                &[ResidualRiskCondition::ConsentContentFlag]
            ),
            PrivacyRisk::Low
        );
        assert_eq!(
            pipeline_privacy_risk(
                &ResidualPiiRisk::Medium,
                &[
                    ResidualRiskCondition::ConsentContentFlag,
                    ResidualRiskCondition::FoundAndRemoved,
                ]
            ),
            PrivacyRisk::Medium
        );
        assert_eq!(
            pipeline_privacy_risk(&ResidualPiiRisk::Medium, &[]),
            PrivacyRisk::Medium
        );
        assert_eq!(
            pipeline_privacy_risk(&ResidualPiiRisk::High, &[]),
            PrivacyRisk::High
        );
    }

    /// Owner decision 2026-10-12: with the operator's accept-Medium setting
    /// on, a Medium whose basis is only what redaction found and removed
    /// (with or without the consent flag) reaches Admission as Low and is
    /// marked as lowered. A declared Medium with no basis, any other basis
    /// condition, every High, and every case with the setting off map
    /// exactly as `pipeline_privacy_risk`.
    #[test]
    fn admission_privacy_risk_accepts_only_a_found_and_removed_medium() {
        use ResidualRiskCondition::{
            ConsentContentFlag, CoverageIncomplete, FoundAndRemoved, KeyFinding, ResidualSurvivor,
        };
        let medium = ResidualPiiRisk::Medium;
        for basis in [
            &[FoundAndRemoved][..],
            &[FoundAndRemoved, ConsentContentFlag][..],
            &[ConsentContentFlag, FoundAndRemoved][..],
        ] {
            assert_eq!(
                admission_privacy_risk(&medium, basis, true),
                (PrivacyRisk::Low, true),
                "{basis:?}"
            );
            assert_eq!(
                admission_privacy_risk(&medium, basis, false),
                (PrivacyRisk::Medium, false),
                "{basis:?}"
            );
        }
        // Already Low on Admission's scale: not lowered by the setting.
        assert_eq!(
            admission_privacy_risk(&medium, &[ConsentContentFlag], true),
            (PrivacyRisk::Low, false)
        );
        // A declared Medium nothing was found for keeps its hold.
        assert_eq!(
            admission_privacy_risk(&medium, &[], true),
            (PrivacyRisk::Medium, false)
        );
        for basis in [
            &[FoundAndRemoved, KeyFinding][..],
            &[FoundAndRemoved, CoverageIncomplete][..],
            &[FoundAndRemoved, ResidualSurvivor][..],
            &[
                FoundAndRemoved,
                ResidualRiskCondition::ResidualScanUnavailable,
            ][..],
        ] {
            assert_eq!(
                admission_privacy_risk(&medium, basis, true),
                (PrivacyRisk::Medium, false),
                "{basis:?}"
            );
        }
        for basis in [
            &[][..],
            &[FoundAndRemoved][..],
            &[FoundAndRemoved, ConsentContentFlag][..],
            &[ResidualRiskCondition::ResidualScanUnavailable][..],
        ] {
            assert_eq!(
                admission_privacy_risk(&ResidualPiiRisk::High, basis, true),
                (PrivacyRisk::High, false),
                "{basis:?}"
            );
        }
        assert_eq!(
            admission_privacy_risk(&ResidualPiiRisk::Low, &[FoundAndRemoved], true),
            (PrivacyRisk::Low, false)
        );
    }

    /// Decision P3 as amended 2026-10-10: only a High pass risk above the
    /// receipt's escalates; a Medium pass risk (PII found and removed, or a
    /// consent flag) clears, whatever the receipt's risk was.
    #[test]
    fn privacy_pass_escalates_only_at_high() {
        use PrivacyRisk::{High, Low, Medium};
        for (receipt, pass, expected) in [
            (Low, Low, PrivacyPassOutcome::Cleared),
            (Low, Medium, PrivacyPassOutcome::Cleared),
            (Medium, Medium, PrivacyPassOutcome::Cleared),
            (Medium, Low, PrivacyPassOutcome::Cleared),
            (Low, High, PrivacyPassOutcome::Escalated),
            (Medium, High, PrivacyPassOutcome::Escalated),
            (High, High, PrivacyPassOutcome::Cleared),
        ] {
            assert_eq!(
                privacy_pass_outcome(receipt, pass),
                expected,
                "{receipt:?} -> {pass:?}"
            );
        }
        // The pilot's prose-PII probe: a Low receipt whose classifier found
        // and removed names, a date of birth and an address.
        let pass = pipeline_privacy_risk(
            &ResidualPiiRisk::Medium,
            &[ResidualRiskCondition::FoundAndRemoved],
        );
        assert_eq!(
            privacy_pass_outcome(PrivacyRisk::Low, pass),
            PrivacyPassOutcome::Cleared
        );
    }

    /// Decision P8: the pass timeout is `min(900 s, review lease x
    /// PIPELINE_LEASE_RENEWAL_CAP_FACTOR - 60 s)` for a boundary that runs a
    /// prose-PII classifier, so a pass always ends inside the Review lease
    /// cap with time left to commit. A Review lease too short to leave the
    /// classifier 60 s refuses the build. A boundary that classifies nothing
    /// is bounded by the ceiling only, never by a short test lease, and
    /// never by a negative value.
    /// The ceiling is an operator setting carried on the lease config
    /// (`TRACE_COMMONS_PIPELINE_PRIVACY_PASS_MAX_TIMEOUT_SECONDS`), bounded
    /// to 60..=7200 s, and it only replaces the 900 s ceiling: the Review
    /// lease bound still applies, so raising it alone does not lengthen a
    /// pass under the default 5-minute Review lease.
    #[test]
    fn privacy_pass_ceiling_is_configurable_and_still_lease_bound() {
        let seconds = std::time::Duration::from_secs;
        let defaults = PipelineLeaseConfig::default();
        assert_eq!(defaults.privacy_pass_max_timeout(), seconds(900));

        let raised = defaults
            .with_privacy_pass_max_timeout(seconds(3600))
            .expect("3600 s is in range");
        assert_eq!(raised.privacy_pass_max_timeout(), seconds(3600));
        // Default Review lease 300 s: 300 x 4 - 60 = 1140 s, not 3600.
        assert_eq!(
            privacy_pass_timeout_for(raised.review(), true, raised.privacy_pass_max_timeout())
                .unwrap(),
            seconds(1140)
        );
        // A 20-minute Review lease leaves 4740 s, so the 3600 s ceiling binds.
        let long_review =
            PipelineLeaseConfig::new(Duration::seconds(1200), defaults.score(), defaults.settle())
                .unwrap()
                .with_privacy_pass_max_timeout(seconds(3600))
                .unwrap();
        assert_eq!(
            privacy_pass_timeout_for(
                long_review.review(),
                true,
                long_review.privacy_pass_max_timeout()
            )
            .unwrap(),
            seconds(3600)
        );
        // Lowering it binds under the default lease too.
        let lowered = defaults.with_privacy_pass_max_timeout(seconds(60)).unwrap();
        assert_eq!(
            privacy_pass_timeout_for(lowered.review(), true, lowered.privacy_pass_max_timeout())
                .unwrap(),
            seconds(60)
        );
        for bad in [0, 59, 7201, 86_400] {
            let error = defaults
                .with_privacy_pass_max_timeout(seconds(bad))
                .expect_err("out of range is refused");
            assert_eq!(
                error.to_string(),
                PIPELINE_PRIVACY_PASS_TIMEOUT_INVALID_LABEL
            );
        }
    }

    #[test]
    fn privacy_pass_timeout_fits_inside_the_review_lease_cap() {
        let ceiling = PIPELINE_PRIVACY_PASS_MAX_TIMEOUT;
        let seconds = std::time::Duration::from_secs;
        assert_eq!(
            privacy_pass_timeout_for(Duration::seconds(300), true, ceiling).unwrap(),
            seconds(900)
        );
        assert_eq!(
            privacy_pass_timeout_for(Duration::seconds(120), true, ceiling).unwrap(),
            seconds(420)
        );
        assert_eq!(
            privacy_pass_timeout_for(Duration::seconds(30), true, ceiling).unwrap(),
            seconds(60)
        );
        let error = privacy_pass_timeout_for(Duration::seconds(20), true, ceiling)
            .expect_err("a classifier left under 60 s refuses the build");
        assert_eq!(error.to_string(), PIPELINE_LEASE_CONFIG_INVALID_LABEL);
        assert_eq!(
            privacy_pass_timeout_for(Duration::seconds(20), false, ceiling).unwrap(),
            seconds(900)
        );
        assert_eq!(
            privacy_pass_timeout_for(Duration::seconds(5), false, ceiling).unwrap(),
            seconds(900)
        );
        assert_eq!(
            privacy_pass_timeout_for(Duration::seconds(1), false, ceiling).unwrap(),
            seconds(900)
        );
        // The test-only ceiling knob bounds both kinds of boundary.
        let short = std::time::Duration::from_millis(200);
        assert_eq!(
            privacy_pass_timeout_for(Duration::seconds(5), false, short).unwrap(),
            short
        );
        assert_eq!(
            privacy_pass_timeout_for(Duration::seconds(300), true, short).unwrap(),
            short
        );
    }

    /// A run record for the object-ref unit tests below: only the ids
    /// matter to them.
    fn privacy_pass_test_run(run_id: Uuid) -> PipelineRunRecord {
        let now = Utc::now();
        PipelineRunRecord {
            tenant_id: "tenant-privacy-pass-unit".to_string(),
            run_id,
            submission_id: Uuid::new_v4(),
            trace_id: Uuid::new_v4(),
            bundle_id: "bundle".to_string(),
            request_idempotency_key: "key".to_string(),
            request_content_hash: "sha256:00".to_string(),
            source_object_ref_id: Uuid::new_v4(),
            approved_revision_id: None,
            next_phase: Some(Phase::Review),
            state: PipelineRunState::Leased,
            lease_token: Some(Uuid::new_v4()),
            lease_expires_at: Some(now),
            attempt_count: 1,
            max_attempts: 5,
            next_attempt_at: now,
            phase_started_at: now,
            last_error_label: None,
            index_membership: "none".to_string(),
            index_command_ref: None,
            index_command_hash: None,
            index_write_state: "none".to_string(),
            admission_decision: "admit".to_string(),
            admission_reason: None,
            approved_object_ref_id: None,
            approved_content_hash: None,
            score_neighbor_ref: None,
            score_neighbor_hash: None,
            settle_selection_hash: None,
            privacy_pass_required: true,
            privacy_pass_object_ref_id: None,
            privacy_pass_content_hash: None,
            privacy_pass_source_hash: None,
            privacy_pass_residual_risk_basis: None,
            privacy_pass_outcome: None,
            privacy_pass_recorded_at: None,
            privacy_pass_approval_assessment_hash: None,
            privacy_pass_approval_resolved_reasons: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn privacy_pass_test_receipt(object_key: &str) -> EncryptedTraceArtifactReceipt {
        EncryptedTraceArtifactReceipt {
            tenant_storage_ref: "tenant-ref".to_string(),
            artifact_kind: TraceArtifactKind::ContributionEnvelope,
            object_key: object_key.to_string(),
            ciphertext_sha256: "ab".repeat(32),
            encrypted_at: Utc::now(),
        }
    }

    /// Task 4: the pass object's staging name round-trips through the
    /// column value, and it is stored as a contribution envelope, as the
    /// approved content is.
    #[test]
    fn privacy_pass_artifact_round_trips() {
        let artifact = PipelineAttemptArtifact::PrivacyPass;
        assert_eq!(artifact.as_str(), "privacy-pass");
        assert_eq!(
            PipelineAttemptArtifact::from_db_str(artifact.as_str()),
            Some(PipelineAttemptArtifact::PrivacyPass)
        );
        assert_eq!(
            artifact.store_kind(),
            TraceArtifactKind::ContributionEnvelope
        );
    }

    /// Task 4 (spec correction 1): the pass object ref id is derived from
    /// the run id alone, so every attempt of one run records the same id,
    /// and it is not the approved ref's id. It is a `review_snapshot` ref
    /// whose `created_by_job_id` is the run, which tells it apart from the
    /// approved ref (`None`).
    #[test]
    fn privacy_pass_object_ref_is_derived_from_the_run() {
        let run_id = Uuid::new_v4();
        let run = privacy_pass_test_run(run_id);
        let first = privacy_pass_object_ref(
            &run,
            &privacy_pass_test_receipt("pipeline-privacy-pass-a"),
            10,
            "store",
        );
        let second = privacy_pass_object_ref(
            &run,
            &privacy_pass_test_receipt("pipeline-privacy-pass-b"),
            20,
            "store",
        );
        assert_eq!(first.object_ref_id, second.object_ref_id);
        assert_ne!(first.object_key, second.object_key);
        let approved = approved_object_ref(
            &run,
            &privacy_pass_test_receipt("pipeline-approved-a"),
            10,
            "store",
        );
        assert_ne!(first.object_ref_id, approved.object_ref_id);
        assert_eq!(first.artifact_kind, TraceObjectArtifactKind::ReviewSnapshot);
        assert_eq!(first.created_by_job_id, Some(run_id));
        assert_eq!(first.submission_id, run.submission_id);
        assert_eq!(first.content_sha256, format!("sha256:{}", "ab".repeat(32)));
        let other = privacy_pass_object_ref(
            &privacy_pass_test_run(Uuid::new_v4()),
            &privacy_pass_test_receipt("pipeline-privacy-pass-a"),
            10,
            "store",
        );
        assert_ne!(first.object_ref_id, other.object_ref_id);
    }

    /// Task 4: the revocation-propagation worker must take the
    /// `ContributionEnvelope` arm for the pass ref, not treat it as a
    /// pipeline Score object.
    #[test]
    fn privacy_pass_ref_is_not_a_score_object() {
        let run_id = Uuid::new_v4();
        let pass = privacy_pass_object_ref(
            &privacy_pass_test_run(run_id),
            &privacy_pass_test_receipt("pipeline-privacy-pass-a"),
            10,
            "store",
        );
        assert!(!is_pipeline_score_object_ref(
            pass.object_ref_id,
            Some(run_id)
        ));
    }
}
