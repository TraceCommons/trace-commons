use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::skill_loop::{EvaluationArm, SkillTrialResult};

/// Isolation scope for an attempt; account scope stores only a bare SHA-256.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MissionAttemptScope {
    /// Local evaluation that is not bound to an account.
    Practice,
    /// Account-bound evaluation keyed by a bare lowercase owner SHA-256.
    Account { owner_sha256: String },
}

/// One expected fixture and evaluator arm in the fixed request set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct MissionTrialKey {
    /// Bounded public fixture identifier.
    pub task_id: String,
    /// Comparison arm required for this fixture.
    pub arm: EvaluationArm,
}

/// Immutable bindings captured before any model request starts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MissionAttemptStart {
    /// Caller-stable idempotency key.
    pub request_id: Uuid,
    /// Local spend-approval identifier.
    pub approval_id: Uuid,
    /// Published mission identifier.
    pub mission_id: Uuid,
    /// Reward program identifier.
    pub program_id: Uuid,
    /// Bare lowercase SHA-256 of canonical package bytes.
    pub package_sha256: String,
    /// `sha256:`-prefixed immutable reward-offer version hash.
    pub offer_version_hash: String,
    /// Bare lowercase SHA-256 of rendered skill bytes.
    pub skill_sha256: String,
    /// Bare lowercase exact-production evaluator contract SHA-256.
    pub evaluation_contract_hash: String,
    /// Exact model identifier every trial must report as served.
    pub requested_model: String,
    /// Complete fixed trial set required for completion.
    pub expected_trials: Vec<MissionTrialKey>,
}

/// Persisted lifecycle state for one local evaluation attempt.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MissionAttemptStatus {
    /// Evaluation may start or record trials.
    Running,
    /// Cancellation is durable while already-started trials drain.
    CancelRequested,
    /// Every expected trial was recorded.
    Completed,
    /// Evaluation stopped with a bounded stable failure reason.
    Failed,
    /// Requested cancellation finished draining.
    Cancelled,
    /// Startup recovery closed an unfinished attempt.
    Interrupted,
}

impl MissionAttemptStatus {
    pub(super) fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }
}

/// Durable attempt receipt and, until revocation, its bounded trial content.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MissionAttempt {
    /// Locally minted unique attempt identifier.
    pub attempt_id: Uuid,
    /// Immutable pre-inference bindings.
    pub start: MissionAttemptStart,
    /// Current lifecycle state.
    pub status: MissionAttemptStatus,
    /// Creation timestamp in Unix seconds.
    pub created_at_unix_seconds: u64,
    /// Monotonic non-decreasing update timestamp in Unix seconds.
    pub updated_at_unix_seconds: u64,
    /// Bounded model results, removed by terminal content revocation.
    pub trials: Vec<SkillTrialResult>,
    /// Durable count retained after trial content revocation.
    pub trial_count: usize,
    /// Stable terminal label, absent while non-terminal.
    pub terminal_reason: Option<String>,
    /// Whether trial content was removed while the receipt was retained.
    pub content_revoked: bool,
}

/// Idempotent begin result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MissionAttemptBegin {
    /// True only when this call created the attempt.
    pub inserted: bool,
    /// Created or replayed attempt.
    pub attempt: MissionAttempt,
}

/// Content-free list projection for one attempt receipt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MissionAttemptSummary {
    /// Local attempt identifier.
    pub attempt_id: Uuid,
    /// Caller idempotency key.
    pub request_id: Uuid,
    /// Published mission identifier.
    pub mission_id: Uuid,
    /// Reward program identifier.
    pub program_id: Uuid,
    /// Bare canonical package SHA-256.
    pub package_sha256: String,
    /// `sha256:`-prefixed offer version.
    pub offer_version_hash: String,
    /// Bare rendered-skill SHA-256.
    pub skill_sha256: String,
    /// Bare evaluator contract SHA-256.
    pub evaluation_contract_hash: String,
    /// Current lifecycle state.
    pub status: MissionAttemptStatus,
    /// Recorded count retained after content revocation.
    pub trial_count: usize,
    /// Fixed total required by the package contract.
    pub expected_trial_count: usize,
    /// Creation timestamp in Unix seconds.
    pub created_at_unix_seconds: u64,
    /// Latest update timestamp in Unix seconds.
    pub updated_at_unix_seconds: u64,
    /// Whether raw trial content has been revoked.
    pub content_revoked: bool,
}
