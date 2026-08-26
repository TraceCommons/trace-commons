//! Privacy-preserving trace contribution envelopes.
//!
//! This module is intentionally separate from replay traces. Replay fixtures
//! capture enough behavior to drive tests; contribution envelopes capture the
//! consent, privacy, replayability, scoring, and revocation metadata needed
//! before a trace can leave a user's machine.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use regex::Regex;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::llm::recording::{TraceFile, TraceResponse};
use crate::redaction::redact_sensitive_json;

pub const TRACE_CONTRIBUTION_SCHEMA_VERSION: &str = "ironclaw.trace_contribution.v1";
pub const TRACE_CONTRIBUTION_POLICY_VERSION: &str = "2026-04-24";
/// Bumped to v3 when the contextual-entropy pass began re-anchoring after `=`
/// so a cue glued to its value (`api_key=<secret>`) is seen. Stored envelopes
/// carry this string, so a v2 stamp means the glued-assignment shape was not
/// covered when that envelope was redacted.
pub const DETERMINISTIC_REDACTION_PIPELINE_VERSION: &str = "ironclaw-deterministic-secret-path-v3";
pub const PRIVACY_FILTER_SIDECAR_PIPELINE_SUFFIX: &str = "privacy-filter-sidecar-v1";
pub const PRIVACY_FILTER_NEAR_AI_PIPELINE_SUFFIX: &str = "privacy-filter-near-ai-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivacyFilterBackendTag {
    None,
    Sidecar,
    NearAi,
}

impl PrivacyFilterBackendTag {
    /// Stable operational label. Reaches boot logs and stored envelope
    /// summaries, so it is a contract rather than a debug rendering.
    pub fn label(self) -> &'static str {
        match self {
            PrivacyFilterBackendTag::None => "none",
            PrivacyFilterBackendTag::Sidecar => "sidecar",
            PrivacyFilterBackendTag::NearAi => "near_ai",
        }
    }
}
/// v2 alongside the deterministic pipeline bump above: the server re-scrub
/// runs the same detector, so its stamp has to move with it.
pub const SERVER_RESCRUB_PIPELINE_SUFFIX: &str = "server-rescrub-v2";
#[cfg(feature = "near-ai-privacy-filter")]
pub const NEAR_AI_PII_BACKSTOP_PIPELINE_SUFFIX: &str = "near-ai-pii-backstop-v1";
pub const PRIVACY_FILTER_CANARY_VERSION: &str = "trace-privacy-filter-canary-v1";
/// Largest serialized contribution envelope the platform accepts, and the
/// single source of truth for that number.
///
/// It lives here rather than in the client because both ends have to agree:
/// the contributor refuses an oversized envelope before upload, ingest caps
/// the request body, and account read-back caps what it will return. When
/// those were three independent constants they drifted -- the client refused
/// at 1.5 MB while ingest accepted 2 MiB -- so a client raise alone silently
/// bought nothing. Everything downstream is now derived from this value with
/// explicit headroom, and tests assert the ordering holds.
///
/// Sized for whole agent coding sessions: a 42 MB raw session redacts to
/// roughly 2.8 MB of envelope, so this clears the observed worst case by a
/// wide margin. Scoring cost does not scale with it -- the gate chunk cap
/// bounds how much of a trace is ever scored.
pub const MAX_TRACE_ENVELOPE_BYTES: usize = 16_000_000;
/// Largest single field text an external privacy filter will accept.
///
/// Tied to the envelope cap, because any one event could in principle carry
/// most of an envelope -- an agent pasting a whole file into one message is
/// the ordinary case. A smaller per-field cap does not bound the work: the
/// NEAR AI adapter classifies in fixed `CLASSIFY_CHUNK_BYTES` windows, so a
/// given quantity of text costs the same whether it arrives as one field or
/// a hundred, and the envelope cap already bounds the total. All a lower
/// per-field cap bought was a failure mode -- the external-filter pass
/// propagates with `?`, so one oversized event failed the whole submission.
pub const PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES: usize = MAX_TRACE_ENVELOPE_BYTES;
/// Sidecar stdout carries the REDACTED TEXT back, so it has to scale with
/// the input cap or the refusal simply moves from the input guard to the
/// output guard. Doubled because replacement placeholders can be longer
/// than what they replace and JSON escaping expands control characters.
pub const PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDOUT_BYTES: usize =
    2 * PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES;
const _: () = assert!(PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES >= MAX_TRACE_ENVELOPE_BYTES);
const _: () = assert!(
    PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDOUT_BYTES
        >= PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES
);
pub const PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDERR_BYTES: usize = 64 * 1024;
pub const TRACE_CREDIT_NOTICE_MAX_SNOOZE_HOURS: u32 = 24 * 365;
pub const TRACE_UPLOAD_CLAIM_DEFAULT_TIMEOUT_MS: u64 = 5_000;
pub const TRACE_UPLOAD_CLAIM_MAX_RESPONSE_BYTES: usize = 64 * 1024;
const TRACE_UPLOAD_CLAIM_REFRESH_SKEW_SECONDS: i64 = 60;
const TRACE_CREDIT_NOTICE_OUTBOX_MAX_ATTEMPTS_STORED: usize = 10;

fn is_zero_f32(value: &f32) -> bool {
    *value == 0.0
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

fn default_submission_status() -> String {
    "accepted".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceContributionEnvelope {
    pub schema_version: String,
    pub trace_id: Uuid,
    pub submission_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub ironclaw: IronclawTraceMetadata,
    pub consent: ConsentMetadata,
    pub contributor: ContributorMetadata,
    pub privacy: PrivacyMetadata,
    pub events: Vec<TraceContributionEvent>,
    pub outcome: OutcomeMetadata,
    pub replay: ReplayMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_analysis: Option<EmbeddingAnalysisMetadata>,
    pub value: ValueMetadata,
    #[serde(default)]
    pub trace_card: TraceCard,
    #[serde(default)]
    pub value_card: TraceValueCard,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hindsight: Option<HindsightRelabelingCandidate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub training_dynamics: Option<TrainingDynamicsSignals>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_evaluation: Option<ProcessEvaluationLabels>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IronclawTraceMetadata {
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_version: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub feature_flags: BTreeMap<String, String>,
    pub channel: TraceChannel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TraceChannel {
    Web,
    Cli,
    Telegram,
    Slack,
    Routine,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsentMetadata {
    pub policy_version: String,
    pub scopes: Vec<ConsentScope>,
    pub message_text_included: bool,
    pub tool_payloads_included: bool,
    pub revocable: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ConsentScope {
    DebuggingEvaluation,
    BenchmarkOnly,
    RankingTraining,
    ModelTraining,
    /// Contributor has explicitly consented to map their pseudonymous
    /// principal_ref to a publicly-visible handle via the community
    /// surface. Does NOT grant any trace-content allowed-uses on its
    /// own — it gates the /v1/community/profile endpoints. A claim
    /// scoped to ONLY public_attribution cannot submit traces.
    PublicAttribution,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContributorMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pseudonymous_contributor_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_scope_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_account_ref: Option<String>,
    pub revocation_handle: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivacyMetadata {
    pub redaction_pipeline_version: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub redaction_counts: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy_filter_summary: Option<SafePrivacyFilterSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pii_labels_present: Vec<String>,
    pub residual_pii_risk: ResidualPiiRisk,
    pub redaction_hash: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ResidualPiiRisk {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceContributionEvent {
    pub event_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_event_id: Option<Uuid>,
    pub event_type: TraceContributionEventType,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redacted_content: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub structured_payload: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_counts: Option<TokenCounts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<Decimal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failure_modes: Vec<TraceFailureMode>,
    #[serde(default)]
    pub side_effect: SideEffectLevel,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceContributionEventType {
    UserMessage,
    AssistantMessage,
    Reasoning,
    ToolCall,
    ToolResult,
    RoutingDecision,
    Feedback,
    HttpExchange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TraceFailureMode {
    ToolSelectionError,
    ToolArgumentError,
    ToolOrderingError,
    MissingVerification,
    PrematureTermination,
    LoopingOrRepetition,
    ContextLoss,
    PrivacyPolicyViolation,
    SecretExposureAttempt,
    UserIntentMisread,
    UnrecoverableToolFailure,
    BadMemoryRetrieval,
    BadRoutingDecision,
    UnsafeSideEffect,
    SpecificationAmbiguity,
    EnvironmentOrAuthFailure,
    Other(String),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectLevel {
    #[default]
    None,
    ReadOnly,
    LocalWrite,
    ExternalWrite,
    CredentialUse,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenCounts {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutcomeMetadata {
    pub user_feedback: UserFeedback,
    pub task_success: TaskSuccess,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub error_taxonomy: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failure_modes: Vec<TraceFailureMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub human_correction: Option<String>,
}

impl Default for OutcomeMetadata {
    fn default() -> Self {
        Self {
            user_feedback: UserFeedback::None,
            task_success: TaskSuccess::Unknown,
            error_taxonomy: Vec::new(),
            failure_modes: Vec::new(),
            human_correction: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UserFeedback {
    ThumbsUp,
    ThumbsDown,
    Correction,
    None,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskSuccess {
    Success,
    Partial,
    Failure,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplayMetadata {
    pub replayable: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tool_manifest_hashes: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expected_assertions: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replay_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmbeddingAnalysisMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub canonical_summary_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_vector_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nearest_trace_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nearest_cluster_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub novelty_score: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_score: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coverage_tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SafePrivacyFilterSummary {
    pub schema_version: u16,
    pub output_mode: String,
    pub span_count: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub by_label: BTreeMap<String, u32>,
    pub decoded_mismatch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SafePrivacyFilterRedaction {
    pub redacted_text: String,
    pub summary: SafePrivacyFilterSummary,
    pub report: RedactionReport,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivacyFilterSidecarRequest {
    pub text: String,
}

impl PrivacyFilterSidecarRequest {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivacyFilterCanaryReport {
    pub canary_version: String,
    pub healthy: bool,
    pub canary_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redacted_output_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<SafePrivacyFilterSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TraceAllowedUse {
    Debugging,
    Evaluation,
    BenchmarkGeneration,
    RankingModelTraining,
    ModelTraining,
    AggregateAnalytics,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TraceRetentionClass {
    LocalQueue,
    PrivateCorpusRevocable,
    BenchmarkRevocable,
    TrainingRevocable,
    AggregateOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRetentionPolicy {
    pub name: String,
    pub class: TraceRetentionClass,
    pub revocable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age_days: Option<u32>,
    pub allows_derived_artifacts: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DerivedArtifactInvalidationMarker {
    pub schema_version: String,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub revocation_handle_hash: String,
    pub redaction_hash: String,
    pub artifact_prefixes: Vec<String>,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceDatasetEligibility {
    pub eligible: bool,
    pub requested_use: TraceAllowedUse,
    pub retention_policy: TraceRetentionPolicy,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceCard {
    pub consent_scope: ConsentScope,
    pub redaction_pipeline_version: String,
    pub source_channel: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_categories: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_uses: Vec<TraceAllowedUse>,
    pub retention_policy: String,
    pub revocation_handle: String,
}

impl Default for TraceCard {
    fn default() -> Self {
        Self {
            consent_scope: ConsentScope::DebuggingEvaluation,
            redaction_pipeline_version: DETERMINISTIC_REDACTION_PIPELINE_VERSION.to_string(),
            source_channel: "unknown".to_string(),
            tool_categories: Vec::new(),
            allowed_uses: default_allowed_uses_for_scope(ConsentScope::DebuggingEvaluation),
            retention_policy: "private_corpus_revocable".to_string(),
            revocation_handle: Uuid::nil().to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceValueCard {
    pub score_version: String,
    pub scorecard: TraceValueScorecard,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limitations: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub user_visible_explanation: Vec<String>,
}

impl Default for TraceValueCard {
    fn default() -> Self {
        Self {
            score_version: "trace-value-scorecard-v1".to_string(),
            scorecard: TraceValueScorecard::default(),
            limitations: vec![
                "Initial score uses local heuristics only; delayed utility credit is assigned by downstream evaluation, benchmark, and training jobs.".to_string(),
            ],
            user_visible_explanation: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TraceValueScorecard {
    pub schema_validity: f32,
    pub privacy_risk: f32,
    pub quality: f32,
    pub replayability: f32,
    pub novelty: f32,
    pub duplicate_penalty: f32,
    pub coverage_bonus: f32,
    pub difficulty: f32,
    pub dependability: f32,
    pub user_correction_value: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_eval_value: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downstream_utility: Option<f32>,
    pub online_score: f32,
    pub credit_points_estimate: f32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub explanation: Vec<String>,
}

/// Aggregate confidence signals for a trace, reduced from per-token
/// log-probabilities on the contributor's machine.
///
/// # These are not dataset cartography, and must not be described as it
///
/// The field names come from Swayamdipta et al., *Dataset Cartography* (2020),
/// which defines confidence, variability and correctness **across training
/// epochs, against a gold label**. Single-pass generation has neither epochs
/// nor a gold label, so what is recorded here is a deliberate analogue:
///
/// | Field | Cartography | Here |
/// |---|---|---|
/// | `mean_confidence` | mean p(gold) across epochs | mean p(chosen token) across the trace |
/// | `variability` | s.d. of p(gold) **across epochs** | s.d. of p(chosen) **across tokens** |
/// | `correctness` | fraction of epochs predicted right | an outcome signal, supplied separately |
///
/// `variability` is the one that differs in kind rather than degree: within-
/// sequence dispersion is not the across-epoch instability the paper measures,
/// and the two should not be compared. The name is kept because the field
/// predates this reduction and renaming it would break the wire format; this
/// paragraph exists so nobody reads the name and infers the paper's semantics.
///
/// # Why aggregates rather than raw distributions
///
/// Raw per-token distributions do not fit — `MAX_INGEST_BODY_BYTES` is 2 MiB
/// and top-5 logprobs for a typical trace is several times that — and they are
/// conditioned on the entire context, which makes them more sensitive than the
/// text they describe. Four numbers give an attacker essentially nothing to
/// invert. The reduction therefore happens where the raw data already lives,
/// and only the result crosses the ingest boundary.
///
/// # Trust
///
/// Every field here is contributor-supplied. Nothing may be treated as
/// attested, and anything that consumes these values must assume they were
/// chosen rather than measured. [`Self::validation_error`] enforces only that
/// they are well-formed, which is not the same as true.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TrainingDynamicsSignals {
    /// Mean probability of the tokens the model actually emitted, in `0.0..=1.0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_confidence: Option<f32>,
    /// Population standard deviation of those probabilities across tokens, in
    /// `0.0..=1.0`. See the type docs: this is not the paper's variability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variability: Option<f32>,
    /// Whether the work was right, in `0.0..=1.0`. Never derivable from
    /// log-probabilities; it needs an outcome signal such as a failing test or
    /// a task-failed flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correctness: Option<f32>,
    /// Coarse bucket derived from the two figures above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cartography_bucket: Option<CartographyBucket>,
}

/// At or above this mean confidence, a steady trace is [`CartographyBucket::Easy`].
///
/// Provisional. Chosen to be defensible rather than calibrated — no corpus has
/// been measured against it yet — so treat a shift in the distribution of
/// buckets after any change here as expected, not as a finding.
pub const CARTOGRAPHY_EASY_MEAN_CONFIDENCE: f32 = 0.75;

/// At or above this dispersion, a trace is [`CartographyBucket::Ambiguous`]
/// regardless of its mean. Provisional, as above.
pub const CARTOGRAPHY_AMBIGUOUS_VARIABILITY: f32 = 0.25;

impl TrainingDynamicsSignals {
    /// Name of the first field that is not well-formed, or `None`.
    ///
    /// Well-formed means finite and within `0.0..=1.0`. These values arrive
    /// inside a submitted envelope, so they are attacker-controlled: a
    /// consumer that assumes a probability and receives `1e30` is the bug this
    /// prevents. Rejecting rather than clamping is deliberate — silently
    /// repairing contributor data hides a broken producer, and a producer that
    /// emits out-of-range confidence is wrong in ways clamping will not fix.
    #[must_use]
    pub fn validation_error(&self) -> Option<&'static str> {
        for (name, value) in [
            ("mean_confidence", self.mean_confidence),
            ("variability", self.variability),
            ("correctness", self.correctness),
        ] {
            if let Some(value) = value
                && !(value.is_finite() && (0.0..=1.0).contains(&value))
            {
                return Some(name);
            }
        }
        None
    }
}

/// Reduce per-token probabilities to the aggregate signals the envelope carries.
///
/// `probabilities` are p(chosen token) for each generated token — that is,
/// `exp(logprob)`, not the logprob itself. An empty slice measures nothing and
/// yields an empty set of signals rather than a confident-looking zero.
///
/// `correctness` is always left unset: no arrangement of log-probabilities
/// answers whether the work was right, and substituting confidence for
/// correctness would be the kind of quiet redefinition this envelope's
/// vocabulary cannot afford. Callers with a real outcome signal should set it
/// themselves.
#[must_use]
pub fn reduce_token_confidences(probabilities: &[f32]) -> TrainingDynamicsSignals {
    if probabilities.is_empty() {
        return TrainingDynamicsSignals::default();
    }

    let count = probabilities.len() as f32;
    // Clamp on the way in: a caller that hands us a malformed probability
    // should not be able to produce signals the server would then reject.
    let clamped = || {
        probabilities.iter().map(|p| {
            if p.is_finite() {
                p.clamp(0.0, 1.0)
            } else {
                0.0
            }
        })
    };

    let mean = clamped().sum::<f32>() / count;
    let variance = clamped().map(|p| (p - mean).powi(2)).sum::<f32>() / count;
    let variability = variance.sqrt().clamp(0.0, 1.0);
    let mean = mean.clamp(0.0, 1.0);

    let bucket = if variability >= CARTOGRAPHY_AMBIGUOUS_VARIABILITY {
        // Dispersion first: a run that swings between certainty and doubt is
        // the interesting case, and its mean hides exactly that.
        CartographyBucket::Ambiguous
    } else if mean >= CARTOGRAPHY_EASY_MEAN_CONFIDENCE {
        CartographyBucket::Easy
    } else {
        CartographyBucket::Hard
    };

    TrainingDynamicsSignals {
        mean_confidence: Some(mean),
        variability: Some(variability),
        correctness: None,
        cartography_bucket: Some(bucket),
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CartographyBucket {
    Easy,
    Ambiguous,
    Hard,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ProcessEvaluationLabels {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluator_name: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub evaluator_version: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<ProcessEvaluatorLabel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_selection: Option<ProcessEvalRating>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_argument_quality: Option<ProcessEvalRating>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_ordering: Option<ProcessEvalRating>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<ProcessEvalRating>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side_effect_safety: Option<ProcessEvalRating>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overall_score: Option<f32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ProcessEvalRating {
    Pass,
    Partial,
    Fail,
    NotApplicable,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ProcessEvaluatorLabel {
    CorrectToolSelection,
    IncorrectToolSelection,
    CorrectToolArguments,
    IncorrectToolArguments,
    CorrectToolOrdering,
    ToolOrderingIssue,
    RetryLoop,
    MissingVerification,
    ProperVerification,
    SafeSideEffects,
    UnsafeSideEffectAttempt,
    UserCorrectionHandled,
    RecoverableFailure,
    BenchmarkCandidate,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalRepresentationKind {
    WholeTrace,
    Turn,
    ToolSequence,
    ErrorOutcome,
    Correction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalTraceRepresentation {
    pub kind: CanonicalRepresentationKind,
    pub vector_key: String,
    pub canonical_hash: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct HindsightRelabelingCandidate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_goal_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub achieved_subgoals: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_type: Option<TraceFailureMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recoverability_score: Option<f32>,
    #[serde(default)]
    pub benchmark_candidate: bool,
    #[serde(default)]
    pub relabeled_training_candidate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TraceCreditEventKind {
    Accepted,
    RejectedPrivacy,
    RejectedDuplicate,
    CreditSynced,
    Replayable,
    NovelCluster,
    UnderrepresentedCoverage,
    UserCorrectionIncluded,
    ConvertedToBenchmark,
    CaughtRegression,
    UsedForTrainingOrRanking,
    ReviewerBonus,
    AbusePenalty,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceCreditEvent {
    pub event_id: Uuid,
    pub submission_id: Uuid,
    pub contributor_pseudonym: String,
    pub kind: TraceCreditEventKind,
    pub points_delta: f32,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValueMetadata {
    pub submission_score: f32,
    pub credit_points_pending: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_points_final: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub explanation: Vec<String>,
}

impl Default for ValueMetadata {
    fn default() -> Self {
        Self {
            submission_score: 0.0,
            credit_points_pending: 0.0,
            credit_points_final: None,
            explanation: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StandingTraceContributionPolicy {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingestion_endpoint: Option<String>,
    pub bearer_token_env: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_token_issuer_url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub upload_token_issuer_allowed_hosts: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_token_audience: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_token_tenant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_token_workload_token_env: Option<String>,
    #[serde(default = "default_trace_upload_claim_issuer_timeout_ms")]
    pub upload_token_issuer_timeout_ms: u64,
    pub include_message_text: bool,
    pub include_tool_payloads: bool,
    pub auto_submit_failed_traces: bool,
    pub auto_submit_high_value_traces: bool,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub selected_tools: BTreeSet<String>,
    pub require_manual_approval_when_pii_detected: bool,
    pub min_submission_score: f32,
    pub credit_notice_interval_hours: u32,
    pub default_scope: ConsentScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceContributionAcceptance {
    PreviewOnly,
    QueueFromPreview,
    ManualSubmit,
    AutonomousSubmit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceContributionPolicyRejection {
    OptInDisabled,
    EndpointMissing,
}

impl std::fmt::Display for TraceContributionPolicyRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OptInDisabled => write!(f, "trace contribution opt-in is disabled"),
            Self::EndpointMissing => write!(f, "trace contribution endpoint is not configured"),
        }
    }
}

impl std::error::Error for TraceContributionPolicyRejection {}

pub fn preflight_trace_contribution_policy(
    policy: &StandingTraceContributionPolicy,
    intent: TraceContributionAcceptance,
) -> Result<(), TraceContributionPolicyRejection> {
    if intent == TraceContributionAcceptance::PreviewOnly {
        return Ok(());
    }
    if !policy.enabled {
        return Err(TraceContributionPolicyRejection::OptInDisabled);
    }
    if policy.ingestion_endpoint.is_none() {
        return Err(TraceContributionPolicyRejection::EndpointMissing);
    }
    Ok(())
}

impl Default for StandingTraceContributionPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            ingestion_endpoint: None,
            bearer_token_env: "IRONCLAW_TRACE_SUBMIT_TOKEN".to_string(),
            upload_token_issuer_url: None,
            upload_token_issuer_allowed_hosts: BTreeSet::new(),
            upload_token_audience: None,
            upload_token_tenant_id: None,
            upload_token_workload_token_env: None,
            upload_token_issuer_timeout_ms: TRACE_UPLOAD_CLAIM_DEFAULT_TIMEOUT_MS,
            include_message_text: false,
            include_tool_payloads: false,
            auto_submit_failed_traces: true,
            auto_submit_high_value_traces: true,
            selected_tools: BTreeSet::new(),
            require_manual_approval_when_pii_detected: true,
            min_submission_score: 0.35,
            credit_notice_interval_hours: 168,
            default_scope: ConsentScope::DebuggingEvaluation,
        }
    }
}

fn default_trace_upload_claim_issuer_timeout_ms() -> u64 {
    TRACE_UPLOAD_CLAIM_DEFAULT_TIMEOUT_MS
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CreditEstimate {
    pub submission_score: f32,
    pub credit_points_pending: f32,
    pub scorecard: TraceValueScorecard,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub explanation: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CreditSummary {
    pub submissions_total: u32,
    pub submissions_submitted: u32,
    pub submissions_revoked: u32,
    #[serde(default)]
    pub submissions_expired: u32,
    pub pending_credit: f32,
    pub final_credit: f32,
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub delayed_credit_delta: f32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub credit_events_total: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_explanations: Vec<String>,
}

pub fn trace_credit_notice_message(summary: &CreditSummary) -> String {
    let mut message = format!(
        "Trace contribution credit update: {} submitted, {} expired ({} total), pending +{:.2}, final confirmed +{:.2}, delayed ledger {:+.2}. Delayed credit can change after privacy review, replay/eval, duplicate checks, and downstream utility scoring.",
        summary.submissions_submitted,
        summary.submissions_expired,
        summary.submissions_total,
        summary.pending_credit,
        summary.final_credit,
        summary.delayed_credit_delta
    );
    if summary.credit_events_total > 0 {
        message.push_str(&format!(
            " {} credit event(s) recorded.",
            summary.credit_events_total
        ));
    }
    if !summary.recent_explanations.is_empty() {
        let explanations = summary
            .recent_explanations
            .iter()
            .take(2)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" ");
        message.push_str(" Recent factors: ");
        message.push_str(&explanations);
    }
    message
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceCreditReport {
    pub submissions_total: u32,
    pub submissions_submitted: u32,
    pub submissions_revoked: u32,
    #[serde(default)]
    pub submissions_expired: u32,
    #[serde(default)]
    pub submissions_accepted: u32,
    #[serde(default)]
    pub submissions_quarantined: u32,
    #[serde(default)]
    pub submissions_rejected: u32,
    pub pending_credit: f32,
    pub final_credit: f32,
    #[serde(default)]
    pub credit_events_total: u32,
    #[serde(default)]
    pub delayed_credit_delta: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_submission_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_credit_sync_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub explanation_lines: Vec<String>,
}

pub fn estimate_initial_credit(envelope: &TraceContributionEnvelope) -> CreditEstimate {
    let scorecard = compute_value_scorecard(envelope);
    let submission_score = scorecard.online_score;
    let credit_points_pending = scorecard.credit_points_estimate;
    let explanation = scorecard.explanation.clone();

    CreditEstimate {
        submission_score,
        credit_points_pending,
        scorecard,
        explanation,
    }
}

/// Field names an emitter may use to carry the arguments a tool call was
/// issued with. `arguments` is what this crate's own envelope builder writes;
/// the rest are the spellings the upstream source adapters encounter.
const REPLAY_ARGUMENT_KEYS: &[&str] = &["arguments", "args", "parameters", "params", "input"];

/// Field names an emitter may use to carry what a tool returned.
const REPLAY_RESULT_KEYS: &[&str] = &["result", "results", "output", "response", "content"];

/// A marker flag is not a payload. `{"has_result": true}` says a result
/// existed somewhere upstream; it does not carry the result, and a consumer
/// cannot grade against it. Booleans and nulls therefore never count as
/// populated, which is what keeps this measure from being satisfied by the
/// same metadata that satisfied the boolean it replaces.
fn replay_value_is_populated(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(_) => false,
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
        Value::Number(_) => true,
    }
}

fn payload_carries_any(payload: &Value, keys: &[&str]) -> bool {
    let Some(object) = payload.as_object() else {
        return false;
    };
    keys.iter()
        .any(|key| object.get(*key).is_some_and(replay_value_is_populated))
}

fn has_text(content: Option<&String>) -> bool {
    content.is_some_and(|text| !text.trim().is_empty())
}

/// Arguments are read from the payload ONLY.
///
/// `redacted_content` used to count here as well, which sounded generous and
/// was actively wrong: on the web-history path a tool call's `content` was the
/// tool's RESULT, so every call that came back with anything scored as though
/// it carried the arguments to re-issue it. The arguments third of the replay
/// measure could be satisfied by never recording an argument at all. Every
/// emitter in the tree now puts arguments in the payload under one of
/// [`REPLAY_ARGUMENT_KEYS`], and none of them puts arguments in `content`.
fn event_carries_arguments(event: &TraceContributionEvent) -> bool {
    payload_carries_any(&event.structured_payload, REPLAY_ARGUMENT_KEYS)
}

fn event_carries_result(event: &TraceContributionEvent) -> bool {
    has_text(event.redacted_content.as_ref())
        || payload_carries_any(&event.structured_payload, REPLAY_RESULT_KEYS)
}

/// Whether an event carries anything a consumer could read, as opposed to
/// metadata about an event that once carried something.
fn event_carries_content(event: &TraceContributionEvent) -> bool {
    has_text(event.redacted_content.as_ref())
        || payload_carries_any(&event.structured_payload, REPLAY_ARGUMENT_KEYS)
        || payload_carries_any(&event.structured_payload, REPLAY_RESULT_KEYS)
}

fn replay_call_id(event: &TraceContributionEvent) -> Option<&str> {
    event.tool_call_id.as_deref().or_else(|| {
        event
            .structured_payload
            .as_object()
            .and_then(|object| object.get("tool_call_id"))
            .and_then(Value::as_str)
    })
}

/// What a downstream consumer needs to rebuild a trace as a runnable task:
/// a prompt to issue, arguments to issue each tool call with, and a result
/// per call to grade against.
///
/// This exists because `replayability` used to be `replay.replayable`
/// restated. That field is set by the emitter and was `true` on every
/// envelope in the pilot corpus, including the ones carrying nothing but
/// tool names, so the score could not fail and the corpus reported itself
/// healthy indefinitely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReplaySufficiency {
    pub has_initial_prompt: bool,
    pub tool_calls: usize,
    pub tool_calls_with_arguments: usize,
    pub tool_calls_with_results: usize,
}

impl ReplaySufficiency {
    /// Equal thirds, because a replay needs all three and any one of them
    /// missing leaves a consumer unable to build a benchmark item. Partial
    /// tool coverage earns partial credit: a corpus where half the calls are
    /// seedable is genuinely worth more than one where none are.
    pub fn score(&self) -> f32 {
        let prompt = if self.has_initial_prompt { 1.0 } else { 0.0 };
        if self.tool_calls == 0 {
            // Nothing to seed beyond the prompt, so the prompt is the whole
            // of what replay needs. Guards the division below.
            return prompt;
        }
        let calls = self.tool_calls as f32;
        let arguments = self.tool_calls_with_arguments as f32 / calls;
        let results = self.tool_calls_with_results as f32 / calls;
        ((prompt + arguments + results) / 3.0).clamp(0.0, 1.0)
    }
}

/// Measure what of a replay actually survived into the envelope.
pub fn replay_sufficiency(envelope: &TraceContributionEnvelope) -> ReplaySufficiency {
    let has_initial_prompt = envelope
        .events
        .iter()
        .find(|event| event.event_type == TraceContributionEventType::UserMessage)
        .is_some_and(event_carries_content);

    // Results are matched to calls by `tool_call_id` where both sides carry
    // one, and otherwise drawn in order from a pool of unkeyed results. The
    // fallback matters because no emitter on the pilot path sets the id.
    let mut keyed_results: BTreeMap<&str, usize> = BTreeMap::new();
    let mut unkeyed_results = 0usize;
    for event in &envelope.events {
        if event.event_type != TraceContributionEventType::ToolResult {
            continue;
        }
        if !event_carries_result(event) {
            continue;
        }
        match replay_call_id(event) {
            Some(call_id) => *keyed_results.entry(call_id).or_default() += 1,
            None => unkeyed_results += 1,
        }
    }

    let mut sufficiency = ReplaySufficiency {
        has_initial_prompt,
        ..ReplaySufficiency::default()
    };
    for event in &envelope.events {
        if event.event_type != TraceContributionEventType::ToolCall {
            continue;
        }
        sufficiency.tool_calls += 1;
        if event_carries_arguments(event) {
            sufficiency.tool_calls_with_arguments += 1;
        }
        let matched_by_id = replay_call_id(event)
            .and_then(|call_id| keyed_results.get_mut(call_id))
            .is_some_and(|remaining| {
                if *remaining > 0 {
                    *remaining -= 1;
                    true
                } else {
                    false
                }
            });
        if matched_by_id {
            sufficiency.tool_calls_with_results += 1;
        } else if unkeyed_results > 0 {
            unkeyed_results -= 1;
            sufficiency.tool_calls_with_results += 1;
        }
    }
    sufficiency
}

pub fn compute_value_scorecard(envelope: &TraceContributionEnvelope) -> TraceValueScorecard {
    let schema_validity = if envelope.schema_version == TRACE_CONTRIBUTION_SCHEMA_VERSION {
        1.0
    } else {
        0.0
    };
    let privacy_risk = privacy_risk_score(envelope.privacy.residual_pii_risk);
    let gate = privacy_gate(envelope.privacy.residual_pii_risk);
    let event_count = envelope.events.len() as f32;
    // Length alone used to be the whole of `quality`, which meant redaction
    // raised a trace's score: stripping content leaves the event count
    // untouched. Weight length by the share of events that actually carry
    // something, so padding an envelope with contentless events cannot pay.
    let substantive_events = envelope
        .events
        .iter()
        .filter(|event| event_carries_content(event))
        .count() as f32;
    let content_share = if event_count == 0.0 {
        0.0
    } else {
        substantive_events / event_count
    };
    let quality = ((event_count / 8.0).clamp(0.0, 1.0) * content_share).clamp(0.15, 1.0);
    let sufficiency = replay_sufficiency(envelope);
    // Sufficiency can only lower the score: an emitter that declares a trace
    // unreplayable keeps the last word, but one that declares it replayable
    // now has to have shipped the inputs that claim requires.
    let replayability = if envelope.replay.replayable {
        sufficiency.score()
    } else {
        0.0
    };
    let novelty = envelope
        .embedding_analysis
        .as_ref()
        .and_then(|analysis| analysis.novelty_score)
        .unwrap_or_else(|| (event_count / 12.0).clamp(0.15, 0.6))
        .min(0.85);
    let duplicate_penalty = envelope
        .embedding_analysis
        .as_ref()
        .and_then(|analysis| analysis.duplicate_score)
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let coverage_bonus = (envelope.replay.required_tools.len() as f32 / 5.0).clamp(0.0, 1.0);
    let failed_or_partial = matches!(
        envelope.outcome.task_success,
        TaskSuccess::Failure | TaskSuccess::Partial
    );
    let difficulty = if failed_or_partial { 0.65 } else { 0.35 };
    let dependability = if envelope.events.is_empty() {
        0.0
    } else if envelope.privacy.redaction_hash.starts_with("sha256:") {
        1.0
    } else {
        0.5
    };
    let user_correction_value = if envelope.outcome.human_correction.is_some()
        || envelope.outcome.user_feedback == UserFeedback::Correction
    {
        1.0
    } else {
        0.0
    };
    let process_eval_value = envelope
        .process_evaluation
        .as_ref()
        .and_then(|labels| labels.overall_score)
        .map(|score| score.clamp(0.0, 1.0));

    // Residual privacy risk is charged ONCE, as the multiplicative `gate`.
    // There used to be a `- 0.60 * privacy_risk` term here as well, which
    // charged it twice: `privacy_gate` and `privacy_risk_score` are
    // complementary functions of the same enum (they sum to 1.0 for every
    // band — pinned by a test), so the subtraction re-applied the gate. The
    // effect fell entirely on the medium band, where a 0.5 gate plus a flat
    // -0.30 put every realistic score at or below zero: accepted work that
    // could never earn anything. Low risk was unaffected either way, since
    // `privacy_risk_score(Low)` is 0.0, and high risk is zeroed by the gate
    // and again by the explicit `High` check below.
    let raw = gate
        * schema_validity
        * (0.25 * quality
            + 0.20 * replayability
            + 0.20 * novelty
            + 0.15 * coverage_bonus
            + 0.10 * difficulty
            + 0.10 * user_correction_value)
        - 0.40 * duplicate_penalty;
    let online_score = raw.clamp(0.0, 1.0);
    let credit_points_estimate =
        if matches!(envelope.privacy.residual_pii_risk, ResidualPiiRisk::High) {
            0.0
        } else {
            (10.0 * online_score * 100.0).round() / 100.0
        };

    let mut explanation = Vec::new();
    if gate > 0.0 {
        explanation.push(format!(
            "Privacy gate passed with {:?} residual risk.",
            envelope.privacy.residual_pii_risk
        ));
    } else {
        explanation.push("Residual privacy risk is high; credit is held for review.".to_string());
    }
    if envelope.replay.replayable {
        // Name what is missing rather than asserting the block exists. A
        // consumer who cannot replay a trace needs to know which of the three
        // inputs did not survive, and "Replay metadata is present." told them
        // nothing at all.
        if sufficiency.tool_calls == 0 {
            explanation.push(if sufficiency.has_initial_prompt {
                "Replay inputs present: an initial prompt, and no tool calls to seed.".to_string()
            } else {
                "Replay is blocked: no initial prompt to re-issue.".to_string()
            });
        } else {
            explanation.push(format!(
                "Replay inputs: initial prompt {}, arguments on {} of {} tool call(s), results on {}.",
                if sufficiency.has_initial_prompt {
                    "present"
                } else {
                    "missing"
                },
                sufficiency.tool_calls_with_arguments,
                sufficiency.tool_calls,
                sufficiency.tool_calls_with_results,
            ));
        }
    }
    if !envelope.replay.required_tools.is_empty() {
        explanation.push(format!(
            "Covers {} tool(s).",
            envelope.replay.required_tools.len()
        ));
    }
    if user_correction_value > 0.0 {
        explanation.push("Includes a redacted user correction signal.".to_string());
    }
    if duplicate_penalty > 0.0 {
        explanation.push(format!(
            "Duplicate penalty applied at {:.2}.",
            duplicate_penalty
        ));
    }
    if !envelope.privacy.redaction_counts.is_empty() {
        explanation.push("Deterministic redactions were applied before submission.".to_string());
    }

    TraceValueScorecard {
        schema_validity,
        privacy_risk,
        quality,
        replayability,
        novelty,
        duplicate_penalty,
        coverage_bonus,
        difficulty,
        dependability,
        user_correction_value,
        process_eval_value,
        downstream_utility: None,
        online_score,
        credit_points_estimate,
        explanation,
    }
}

fn privacy_gate(risk: ResidualPiiRisk) -> f32 {
    match risk {
        ResidualPiiRisk::Low => 1.0,
        ResidualPiiRisk::Medium => 0.5,
        ResidualPiiRisk::High => 0.0,
    }
}

fn privacy_risk_score(risk: ResidualPiiRisk) -> f32 {
    match risk {
        ResidualPiiRisk::Low => 0.0,
        ResidualPiiRisk::Medium => 0.5,
        ResidualPiiRisk::High => 1.0,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawTraceContribution {
    pub trace_id: Uuid,
    pub submission_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub ironclaw: IronclawTraceMetadata,
    pub consent: ConsentMetadata,
    pub contributor: ContributorMetadata,
    pub events: Vec<RawTraceContributionEvent>,
    pub outcome: OutcomeMetadata,
    pub replay: ReplayMetadata,
    pub embedding_analysis: Option<EmbeddingAnalysisMetadata>,
    pub value: ValueMetadata,
}

/// The pre-redaction event an emitter builds.
///
/// This deliberately mirrors [`TraceContributionEvent`] field for field,
/// minus the ones redaction derives (`tool_category`, `side_effect`) and
/// minus the redacted text itself. It did not, and the four fields it was
/// missing -- `parent_event_id`, `tool_call_id`, `success` and
/// `failure_modes` -- were consequently unreachable for every emitter in the
/// tree: the envelope modelled them, the conversion below hardcoded them
/// empty, and no caller had anywhere to put them. That is what made the 86
/// failed traces in the pilot corpus undiagnosable (issue #298). None of the
/// four is user content, so none of them depends on a consent decision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawTraceContributionEvent {
    pub event_id: Uuid,
    /// The event this one answers or follows from -- a result naming its
    /// call, a reasoning step naming what it explains. Makes causal order
    /// explicit where array order is the only other signal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_event_id: Option<Uuid>,
    pub event_type: TraceContributionEventType,
    pub timestamp: DateTime<Utc>,
    pub content: Option<String>,
    pub structured_payload: Value,
    pub tool_name: Option<String>,
    /// The harness's own id for the call, carried so a result can be paired
    /// with its call without relying on array order. Redaction still accepts
    /// the older spelling inside `structured_payload` as a fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    pub latency_ms: Option<u64>,
    pub token_counts: Option<TokenCounts>,
    pub cost_usd: Option<Decimal>,
    /// Whether this step did what it was asked. `None` means the emitter does
    /// not know, which is not the same as failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failure_modes: Vec<TraceFailureMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawTraceCaptureTurn {
    pub user_input: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<RawTraceCaptureToolCall>,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawTraceCaptureToolCall {
    pub name: String,
    /// The harness's own id for this call, when it recorded one. Carried so
    /// the result event below can name the call it answers; absent it, a
    /// consumer is back to array order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The arguments the call was issued with.
    ///
    /// A consumer rebuilding a trace as a runnable task needs these: the tool
    /// name says which service was touched, the arguments say what was asked
    /// of it. Without them a captured call can be counted but not replayed.
    /// Gated by `include_tool_payloads` like any other payload; absent that
    /// consent the envelope reports only `has_arguments`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RecordedTraceContributionOptions {
    pub include_message_text: bool,
    pub include_tool_payloads: bool,
    pub consent_scopes: Vec<ConsentScope>,
    pub channel: TraceChannel,
    pub engine_version: Option<String>,
    /// The model the captured session ran on. `from_recorded_trace` reads it
    /// off the recording; the capture-turn path has no equivalent, so it has
    /// to be supplied here -- it was hardcoded `None`, which is why
    /// `model_name` was present on 3 of 330 envelopes in the pilot corpus.
    pub model_name: Option<String>,
    pub feature_flags: BTreeMap<String, String>,
    pub pseudonymous_contributor_id: Option<String>,
    pub tenant_scope_ref: Option<String>,
    pub credit_account_ref: Option<String>,
}

impl Default for RecordedTraceContributionOptions {
    fn default() -> Self {
        Self {
            include_message_text: false,
            include_tool_payloads: false,
            consent_scopes: vec![ConsentScope::DebuggingEvaluation],
            channel: TraceChannel::Cli,
            engine_version: None,
            model_name: None,
            feature_flags: BTreeMap::new(),
            pseudonymous_contributor_id: None,
            tenant_scope_ref: None,
            credit_account_ref: None,
        }
    }
}

impl RawTraceContribution {
    pub fn from_recorded_trace(
        trace: &TraceFile,
        options: RecordedTraceContributionOptions,
    ) -> Self {
        let created_at = Utc::now();
        let mut events = Vec::new();
        let mut required_tools = BTreeSet::new();
        // Which event carried each call, so the result that answers it can
        // name its parent instead of relying on array order.
        let mut call_event_ids: BTreeMap<&str, Uuid> = BTreeMap::new();

        for step in &trace.steps {
            match &step.response {
                TraceResponse::UserInput { content } => {
                    events.push(RawTraceContributionEvent {
                        event_id: Uuid::new_v4(),
                        parent_event_id: None,
                        event_type: TraceContributionEventType::UserMessage,
                        timestamp: created_at,
                        content: options.include_message_text.then(|| content.clone()),
                        structured_payload: Value::Null,
                        tool_name: None,
                        tool_call_id: None,
                        latency_ms: None,
                        token_counts: None,
                        cost_usd: None,
                        success: None,
                        failure_modes: Vec::new(),
                    });
                }
                TraceResponse::Text {
                    content,
                    input_tokens,
                    output_tokens,
                } => {
                    events.push(RawTraceContributionEvent {
                        event_id: Uuid::new_v4(),
                        parent_event_id: None,
                        event_type: TraceContributionEventType::AssistantMessage,
                        timestamp: created_at,
                        content: options.include_message_text.then(|| content.clone()),
                        structured_payload: Value::Null,
                        tool_name: None,
                        tool_call_id: None,
                        latency_ms: None,
                        token_counts: Some(TokenCounts {
                            input_tokens: *input_tokens,
                            output_tokens: *output_tokens,
                        }),
                        cost_usd: None,
                        success: None,
                        failure_modes: Vec::new(),
                    });
                }
                TraceResponse::ToolCalls {
                    tool_calls,
                    input_tokens,
                    output_tokens,
                } => {
                    for tool_call in tool_calls {
                        required_tools.insert(tool_call.name.clone());
                        let structured_payload = if options.include_tool_payloads {
                            serde_json::json!({
                                "tool_call_id": tool_call.id,
                                "arguments": tool_call.arguments,
                            })
                        } else {
                            serde_json::json!({
                                "tool_call_id": tool_call.id,
                            })
                        };

                        let event_id = Uuid::new_v4();
                        call_event_ids.insert(tool_call.id.as_str(), event_id);
                        events.push(RawTraceContributionEvent {
                            event_id,
                            parent_event_id: None,
                            event_type: TraceContributionEventType::ToolCall,
                            timestamp: created_at,
                            content: None,
                            structured_payload,
                            tool_name: Some(tool_call.name.clone()),
                            tool_call_id: Some(tool_call.id.clone()),
                            latency_ms: None,
                            token_counts: Some(TokenCounts {
                                input_tokens: *input_tokens,
                                output_tokens: *output_tokens,
                            }),
                            cost_usd: None,
                            success: None,
                            failure_modes: Vec::new(),
                        });
                    }
                }
            }

            for expected in &step.expected_tool_results {
                required_tools.insert(expected.name.clone());
                events.push(RawTraceContributionEvent {
                    event_id: Uuid::new_v4(),
                    parent_event_id: call_event_ids.get(expected.tool_call_id.as_str()).copied(),
                    event_type: TraceContributionEventType::ToolResult,
                    timestamp: created_at,
                    content: options
                        .include_tool_payloads
                        .then(|| expected.content.clone()),
                    structured_payload: serde_json::json!({
                        "tool_call_id": expected.tool_call_id,
                    }),
                    tool_name: Some(expected.name.clone()),
                    tool_call_id: Some(expected.tool_call_id.clone()),
                    latency_ms: None,
                    token_counts: None,
                    cost_usd: None,
                    success: None,
                    failure_modes: Vec::new(),
                });
            }
        }

        for exchange in &trace.http_exchanges {
            let structured_payload = if options.include_tool_payloads {
                serde_json::json!({
                    "request": {
                        "method": exchange.request.method,
                        "url": exchange.request.url,
                        "headers": exchange.request.headers,
                        "body": exchange.request.body,
                    },
                    "response": {
                        "status": exchange.response.status,
                        "headers": exchange.response.headers,
                    },
                })
            } else {
                serde_json::json!({
                    "request": {
                        "method": exchange.request.method,
                    },
                    "response": {
                        "status": exchange.response.status,
                    },
                })
            };

            events.push(RawTraceContributionEvent {
                event_id: Uuid::new_v4(),
                parent_event_id: None,
                event_type: TraceContributionEventType::HttpExchange,
                timestamp: created_at,
                content: options
                    .include_tool_payloads
                    .then(|| exchange.response.body.clone()),
                structured_payload,
                tool_name: Some("http".to_string()),
                tool_call_id: None,
                latency_ms: None,
                token_counts: None,
                cost_usd: None,
                // An HTTP status is an outcome the recording already knows,
                // and it is metadata rather than body content, so it is
                // reported regardless of the payload consent above.
                success: Some((200..400).contains(&exchange.response.status)),
                failure_modes: Vec::new(),
            });
        }

        let required_tools: Vec<String> = required_tools.into_iter().collect();

        Self {
            trace_id: Uuid::new_v4(),
            submission_id: Uuid::new_v4(),
            created_at,
            ironclaw: IronclawTraceMetadata {
                version: env!("CARGO_PKG_VERSION").to_string(),
                engine_version: options.engine_version,
                feature_flags: options.feature_flags,
                channel: options.channel,
                model_name: Some(trace.model_name.clone()),
            },
            consent: ConsentMetadata {
                policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
                scopes: options.consent_scopes,
                message_text_included: options.include_message_text,
                tool_payloads_included: options.include_tool_payloads,
                revocable: true,
            },
            contributor: ContributorMetadata {
                pseudonymous_contributor_id: options.pseudonymous_contributor_id,
                tenant_scope_ref: options.tenant_scope_ref,
                credit_account_ref: options.credit_account_ref,
                revocation_handle: Uuid::new_v4(),
            },
            events,
            outcome: OutcomeMetadata::default(),
            replay: ReplayMetadata {
                replayable: !trace.steps.is_empty(),
                required_tools,
                tool_manifest_hashes: BTreeMap::new(),
                expected_assertions: Vec::new(),
                replay_notes: Vec::new(),
            },
            embedding_analysis: None,
            value: ValueMetadata::default(),
        }
    }

    pub fn from_capture_turns(
        turns: &[RawTraceCaptureTurn],
        options: RecordedTraceContributionOptions,
    ) -> Self {
        let created_at = Utc::now();
        let mut events = Vec::new();
        let mut required_tools = BTreeSet::new();
        let mut task_success = TaskSuccess::Unknown;

        for turn in turns {
            if !turn.user_input.is_empty() {
                events.push(RawTraceContributionEvent {
                    event_id: Uuid::new_v4(),
                    parent_event_id: None,
                    event_type: TraceContributionEventType::UserMessage,
                    timestamp: turn.started_at,
                    content: options
                        .include_message_text
                        .then(|| turn.user_input.clone()),
                    structured_payload: serde_json::json!({
                        "state": turn.state,
                    }),
                    tool_name: None,
                    tool_call_id: None,
                    latency_ms: None,
                    token_counts: None,
                    cost_usd: None,
                    success: None,
                    failure_modes: Vec::new(),
                });
            }

            for tool_call in &turn.tool_calls {
                required_tools.insert(tool_call.name.clone());

                // `Reasoning` is defined in the schema and was never emitted;
                // the rationale sat on the tool call, where a consumer could
                // not see that a reasoning step had occurred or where in the
                // sequence it sat. Emit it as its own event, ahead of the call
                // it explains. Reasoning is prose, so its text is governed by
                // message-text consent rather than tool payloads — but the
                // event itself is emitted regardless, because knowing a
                // reasoning step happened is shape, not content.
                let call_event_id = Uuid::new_v4();
                let timestamp = turn.completed_at.unwrap_or(turn.started_at);
                // A call that recorded an error failed; one that came back
                // with a result did not. Either way this is the only place the
                // corpus says WHICH tool failed -- the turn state said
                // "Failed" and named nothing -- and a boolean is not content,
                // so it is set regardless of the payload consent below.
                let success = match (&tool_call.error, &tool_call.result_preview) {
                    (Some(_), _) => Some(false),
                    (None, Some(_)) => Some(true),
                    (None, None) => None,
                };

                if tool_call.rationale.is_some() {
                    events.push(RawTraceContributionEvent {
                        event_id: Uuid::new_v4(),
                        // The call this reasoning explains, which is also the
                        // event that follows it.
                        parent_event_id: Some(call_event_id),
                        event_type: TraceContributionEventType::Reasoning,
                        timestamp,
                        content: options
                            .include_message_text
                            .then(|| tool_call.rationale.clone())
                            .flatten(),
                        structured_payload: Value::Null,
                        tool_name: Some(tool_call.name.clone()),
                        tool_call_id: tool_call.id.clone(),
                        latency_ms: None,
                        token_counts: None,
                        cost_usd: None,
                        success: None,
                        failure_modes: Vec::new(),
                    });
                }

                let structured_payload = if options.include_tool_payloads {
                    // Omit absent fields rather than writing nulls: a null
                    // `arguments` key would claim the payload exists and is
                    // empty, and the replay-sufficiency measure reads these
                    // keys to decide whether a call could be re-issued.
                    //
                    // The result and the error used to be written here too,
                    // on the call. They belong to the result event below: a
                    // call carries what was asked, a result carries what came
                    // back, and folding both into the call left a consumer
                    // unable to tell which of the two it was holding.
                    let mut payload = serde_json::Map::new();
                    if let Some(arguments) = &tool_call.arguments {
                        payload.insert("arguments".to_string(), arguments.clone());
                    }
                    if let Some(rationale) = &tool_call.rationale {
                        payload.insert("rationale".to_string(), Value::String(rationale.clone()));
                    }
                    Value::Object(payload)
                } else {
                    serde_json::json!({
                        "has_arguments": tool_call.arguments.is_some(),
                        "has_result": tool_call.result_preview.is_some(),
                        "has_error": tool_call.error.is_some(),
                    })
                };

                events.push(RawTraceContributionEvent {
                    event_id: call_event_id,
                    parent_event_id: None,
                    event_type: TraceContributionEventType::ToolCall,
                    timestamp,
                    // Arguments travel in the payload under their own key, not
                    // in `content`. Putting the RESULT here (which is what
                    // this did) made a call look like it carried arguments to
                    // anything measuring replay sufficiency.
                    content: None,
                    structured_payload,
                    tool_name: Some(tool_call.name.clone()),
                    tool_call_id: tool_call.id.clone(),
                    latency_ms: None,
                    token_counts: None,
                    cost_usd: None,
                    success,
                    failure_modes: Vec::new(),
                });

                // `ToolResult` is defined in the schema and was never emitted
                // on this path: the observation the agent acted on had nowhere
                // to live, so a consumer had a call with no answer to grade
                // against. Emit it whenever the capture recorded either a
                // result or an error. The event itself is shape, so it is
                // emitted regardless of consent; only its text is gated.
                if tool_call.result_preview.is_some() || tool_call.error.is_some() {
                    let result_text = tool_call
                        .result_preview
                        .as_deref()
                        .or(tool_call.error.as_deref())
                        .unwrap_or_default()
                        .to_string();
                    events.push(RawTraceContributionEvent {
                        event_id: Uuid::new_v4(),
                        parent_event_id: Some(call_event_id),
                        event_type: TraceContributionEventType::ToolResult,
                        timestamp,
                        content: options
                            .include_tool_payloads
                            .then_some(result_text)
                            .filter(|text| !text.is_empty()),
                        structured_payload: if options.include_tool_payloads {
                            Value::Null
                        } else {
                            serde_json::json!({
                                "has_result": tool_call.result_preview.is_some(),
                                "has_error": tool_call.error.is_some(),
                            })
                        },
                        tool_name: Some(tool_call.name.clone()),
                        tool_call_id: tool_call.id.clone(),
                        latency_ms: None,
                        token_counts: None,
                        cost_usd: None,
                        success,
                        failure_modes: Vec::new(),
                    });
                }
            }

            if let Some(response) = &turn.response {
                // Both ends of the turn were already captured and neither was
                // ever turned into a duration, so the corpus carried no timing
                // at all. The response event completes the turn, so it is where
                // the elapsed time belongs. A turn that never recorded a
                // completion has no measurable duration: leave it unset rather
                // than fabricating a zero.
                let latency_ms = turn.completed_at.and_then(|completed| {
                    (completed - turn.started_at)
                        .num_milliseconds()
                        .try_into()
                        .ok()
                });
                events.push(RawTraceContributionEvent {
                    event_id: Uuid::new_v4(),
                    parent_event_id: None,
                    event_type: TraceContributionEventType::AssistantMessage,
                    timestamp: turn.completed_at.unwrap_or(turn.started_at),
                    content: options.include_message_text.then(|| response.clone()),
                    structured_payload: Value::Null,
                    tool_name: None,
                    tool_call_id: None,
                    latency_ms,
                    token_counts: None,
                    cost_usd: None,
                    success: None,
                    failure_modes: Vec::new(),
                });
            }

            if matches!(turn.state.as_deref(), Some("Failed" | "failed")) {
                task_success = TaskSuccess::Failure;
            } else if task_success == TaskSuccess::Unknown && turn.response.is_some() {
                task_success = TaskSuccess::Success;
            }
        }

        let required_tools: Vec<String> = required_tools.into_iter().collect();

        Self {
            trace_id: Uuid::new_v4(),
            submission_id: Uuid::new_v4(),
            created_at,
            ironclaw: IronclawTraceMetadata {
                version: env!("CARGO_PKG_VERSION").to_string(),
                engine_version: options.engine_version,
                feature_flags: options.feature_flags,
                channel: options.channel,
                model_name: options.model_name,
            },
            consent: ConsentMetadata {
                policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
                scopes: options.consent_scopes,
                message_text_included: options.include_message_text,
                tool_payloads_included: options.include_tool_payloads,
                revocable: true,
            },
            contributor: ContributorMetadata {
                pseudonymous_contributor_id: options.pseudonymous_contributor_id,
                tenant_scope_ref: options.tenant_scope_ref,
                credit_account_ref: options.credit_account_ref,
                revocation_handle: Uuid::new_v4(),
            },
            events,
            outcome: OutcomeMetadata {
                task_success,
                ..OutcomeMetadata::default()
            },
            replay: ReplayMetadata {
                replayable: !turns.is_empty(),
                required_tools,
                tool_manifest_hashes: BTreeMap::new(),
                expected_assertions: Vec::new(),
                replay_notes: vec![
                    "Captured from web conversation history; exact tool arguments may be omitted by consent policy.".to_string(),
                ],
            },
            embedding_analysis: None,
            value: ValueMetadata::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RedactionReport {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub counts: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pii_labels_present: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// Set when this pass DETECTED secret-shaped content. It says the
    /// scrubber had work to do, not that anything survived: every span that
    /// sets this flag is redacted by the same pass that set it. Treating it
    /// as evidence of danger is what made every real coding session High
    /// (issue #373). Survivors are reported by the post-redaction residual
    /// scan instead - see [`residual_risk`].
    pub blocked_secret_detected: bool,
    /// Set when a classifier flagged an *object key* (not just a value) as
    /// PII-bearing. Keys are not rewritten in place (rewriting risks
    /// collisions with sibling keys), so a key finding cannot be resolved by
    /// redaction the way a value finding can. It forces High rather than
    /// being silently dropped or merely counted.
    #[serde(default)]
    pub key_finding_detected: bool,
    /// Set when a configured privacy-filter backend was unavailable, errored,
    /// or otherwise left content unexamined, so this pass cannot speak for
    /// the text it was supposed to cover. Absence of findings under a broken
    /// filter is not evidence of cleanliness, so this forces High.
    #[serde(default)]
    pub coverage_incomplete: bool,
}

impl RedactionReport {
    pub(crate) fn increment(&mut self, label: impl Into<String>) {
        *self.counts.entry(label.into()).or_insert(0) += 1;
    }

    pub(crate) fn add_pii_label(&mut self, label: impl Into<String>) {
        let label = label.into();
        if !self.pii_labels_present.contains(&label) {
            self.pii_labels_present.push(label);
        }
    }

    pub(crate) fn add_warning(&mut self, warning: impl Into<String>) {
        let warning = warning.into();
        if !self.warnings.contains(&warning) {
            self.warnings.push(warning);
        }
    }

    fn merge(&mut self, other: RedactionReport) {
        for (key, value) in other.counts {
            *self.counts.entry(key).or_insert(0) += value;
        }
        for label in other.pii_labels_present {
            if !self.pii_labels_present.contains(&label) {
                self.pii_labels_present.push(label);
            }
        }
        for warning in other.warnings {
            self.add_warning(warning);
        }
        self.blocked_secret_detected |= other.blocked_secret_detected;
        self.key_finding_detected |= other.key_finding_detected;
        self.coverage_incomplete |= other.coverage_incomplete;
    }
}

pub fn safe_privacy_filter_redaction_from_output(
    output: &Value,
) -> Result<SafePrivacyFilterRedaction, TraceContributionError> {
    let redacted_text = output
        .get("redacted_text")
        .and_then(Value::as_str)
        .ok_or_else(|| TraceContributionError::RedactionFailed {
            reason: "privacy filter output is missing redacted_text".to_string(),
        })?
        .to_string();
    let detected_spans = output
        .get("detected_spans")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut by_label = BTreeMap::new();
    let mut report = RedactionReport::default();
    for span in &detected_spans {
        let raw_label = span
            .get("label")
            .or_else(|| span.get("type"))
            .or_else(|| span.get("entity_type"))
            .and_then(Value::as_str);
        let label = safe_privacy_filter_label(raw_label, &mut report);
        *by_label.entry(label.clone()).or_insert(0) += 1;
        report.increment(format!("privacy_filter:{label}"));
        if label.eq_ignore_ascii_case("secret") {
            report.blocked_secret_detected = true;
        }
        if !report.pii_labels_present.contains(&label) {
            report.pii_labels_present.push(label);
        }
    }

    let schema_version = output
        .get("schema_version")
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .unwrap_or(1);
    let decoded_mismatch = output
        .get("decoded_mismatch")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Ok(SafePrivacyFilterRedaction {
        redacted_text,
        summary: SafePrivacyFilterSummary {
            schema_version,
            output_mode: "redacted_text_only".to_string(),
            span_count: detected_spans.len() as u32,
            by_label,
            decoded_mismatch,
        },
        report,
    })
}

pub(crate) fn safe_privacy_filter_label(
    raw_label: Option<&str>,
    report: &mut RedactionReport,
) -> String {
    let Some(raw_label) = raw_label else {
        return "unknown".to_string();
    };
    let normalized = raw_label
        .trim()
        .chars()
        .map(|character| {
            if character == '-' {
                '_'
            } else {
                character.to_ascii_lowercase()
            }
        })
        .collect::<String>();
    let allowed = matches!(
        normalized.as_str(),
        "account_number"
            | "credit_card"
            | "ip_address"
            | "private_address"
            | "private_date"
            | "private_email"
            | "private_location"
            | "private_name"
            | "private_person"
            | "private_phone"
            | "private_url"
            | "secret"
            | "secret_like"
            | "ssn"
    );
    if allowed {
        return normalized;
    }

    report.add_warning("Privacy Filter sidecar emitted unsupported span label; mapped to unknown.");
    "unknown".to_string()
}

pub fn synthetic_privacy_filter_canary_text() -> String {
    synthetic_privacy_filter_canary_values().join(" ")
}

pub fn synthetic_privacy_filter_canary_values() -> Vec<String> {
    vec![
        "trace-canary.person@example.invalid".to_string(),
        "tc_canary_secret_0123456789abcdef".to_string(),
        "/tmp/trace_canary_private/path.txt".to_string(),
    ]
}

pub async fn run_configured_privacy_filter_canary_from_env()
-> Result<Option<PrivacyFilterCanaryReport>, TraceContributionError> {
    let Some((adapter, _backend)) = privacy_filter_adapter_from_env().map_err(|err| {
        TraceContributionError::RedactionFailed {
            reason: err.to_string(),
        }
    })?
    else {
        return Ok(None);
    };
    run_privacy_filter_canary(adapter.as_ref()).await.map(Some)
}

pub async fn run_privacy_filter_canary(
    adapter: &dyn PrivacyFilterAdapter,
) -> Result<PrivacyFilterCanaryReport, TraceContributionError> {
    let raw_values = synthetic_privacy_filter_canary_values();
    let canary_text = raw_values.join(" ");
    let canary_hash = canonical_hash(&canary_text);
    let redaction = adapter.redact_text(&canary_text).await?;

    let Some(redaction) = redaction else {
        return Ok(PrivacyFilterCanaryReport {
            canary_version: PRIVACY_FILTER_CANARY_VERSION.to_string(),
            healthy: false,
            canary_hash,
            redacted_output_hash: None,
            summary: None,
            failures: vec!["privacy filter returned no redaction for synthetic canary".to_string()],
        });
    };

    let mut failures = Vec::new();
    let summary_json = serde_json::to_string(&redaction.summary).unwrap_or_default();
    let report_json = serde_json::to_string(&redaction.report).unwrap_or_default();
    for raw_value in &raw_values {
        if redaction.redacted_text.contains(raw_value) {
            failures.push(format!(
                "privacy filter redacted_text retained canary value hash {}",
                canonical_hash(raw_value)
            ));
        }
        if summary_json.contains(raw_value) || report_json.contains(raw_value) {
            failures.push(format!(
                "privacy filter safe summary retained canary value hash {}",
                canonical_hash(raw_value)
            ));
        }
    }

    Ok(PrivacyFilterCanaryReport {
        canary_version: PRIVACY_FILTER_CANARY_VERSION.to_string(),
        healthy: failures.is_empty(),
        canary_hash,
        redacted_output_hash: Some(canonical_hash(&redaction.redacted_text)),
        summary: Some(redaction.summary),
        failures,
    })
}

fn merge_privacy_filter_summary(
    target: &mut Option<SafePrivacyFilterSummary>,
    next: &SafePrivacyFilterSummary,
) {
    let target = target.get_or_insert_with(|| SafePrivacyFilterSummary {
        schema_version: next.schema_version,
        output_mode: "redacted_text_only".to_string(),
        span_count: 0,
        by_label: BTreeMap::new(),
        decoded_mismatch: false,
    });
    target.schema_version = target.schema_version.max(next.schema_version);
    target.span_count = target.span_count.saturating_add(next.span_count);
    target.decoded_mismatch |= next.decoded_mismatch;
    for (label, count) in &next.by_label {
        *target.by_label.entry(label.clone()).or_insert(0) += count;
    }
}

fn redaction_pipeline_version(backend: PrivacyFilterBackendTag) -> String {
    match backend {
        PrivacyFilterBackendTag::None => DETERMINISTIC_REDACTION_PIPELINE_VERSION.to_string(),
        PrivacyFilterBackendTag::Sidecar => format!(
            "{DETERMINISTIC_REDACTION_PIPELINE_VERSION}+{PRIVACY_FILTER_SIDECAR_PIPELINE_SUFFIX}"
        ),
        PrivacyFilterBackendTag::NearAi => format!(
            "{DETERMINISTIC_REDACTION_PIPELINE_VERSION}+{PRIVACY_FILTER_NEAR_AI_PIPELINE_SUFFIX}"
        ),
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum TraceContributionError {
    #[error("trace contribution redaction failed: {reason}")]
    RedactionFailed { reason: String },
}

#[derive(Debug, thiserror::Error)]
pub enum PrivacyFilterConfigError {
    #[error("unknown TRACE_PRIVACY_FILTER_BACKEND value: {value}")]
    UnknownBackend { value: String },
    #[error("missing required env var for backend {backend}: {var}")]
    MissingEnv {
        backend: &'static str,
        var: &'static str,
    },
    #[error("invalid env var {var}: {reason}")]
    InvalidEnv { var: &'static str, reason: String },
    #[error("backend {backend} requires the {feature} cargo feature")]
    FeatureDisabled {
        backend: &'static str,
        feature: &'static str,
    },
}

#[async_trait]
pub trait TraceRedactor: Send + Sync {
    async fn redact_trace(
        &self,
        trace: RawTraceContribution,
    ) -> Result<TraceContributionEnvelope, TraceContributionError>;
}

#[async_trait]
pub trait PrivacyFilterAdapter: Send + Sync {
    async fn redact_text(
        &self,
        text: &str,
    ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError>;
}

pub struct NoopPrivacyFilterAdapter;

#[async_trait]
impl PrivacyFilterAdapter for NoopPrivacyFilterAdapter {
    async fn redact_text(
        &self,
        _text: &str,
    ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
        Ok(None)
    }
}

#[derive(Debug, Clone)]
pub struct CommandPrivacyFilterAdapter {
    command: PathBuf,
    args: Vec<String>,
    timeout: Duration,
    max_input_bytes: usize,
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
}

impl CommandPrivacyFilterAdapter {
    pub fn new(command: impl Into<PathBuf>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            timeout: Duration::from_secs(10),
            max_input_bytes: PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES,
            max_stdout_bytes: PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDOUT_BYTES,
            max_stderr_bytes: PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDERR_BYTES,
        }
    }

    pub fn with_args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_input_limit(mut self, max_input_bytes: usize) -> Self {
        self.max_input_bytes = max_input_bytes;
        self
    }

    pub fn with_output_limits(mut self, max_stdout_bytes: usize, max_stderr_bytes: usize) -> Self {
        self.max_stdout_bytes = max_stdout_bytes;
        self.max_stderr_bytes = max_stderr_bytes;
        self
    }
}

#[async_trait]
impl PrivacyFilterAdapter for CommandPrivacyFilterAdapter {
    async fn redact_text(
        &self,
        text: &str,
    ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
        if text.trim().is_empty() {
            return Ok(None);
        }
        if text.len() > self.max_input_bytes {
            return Err(TraceContributionError::RedactionFailed {
                reason: format!(
                    "privacy filter sidecar input exceeded limit: input_len={} max_input_bytes={}",
                    text.len(),
                    self.max_input_bytes
                ),
            });
        }

        let mut command = tokio::process::Command::new(&self.command);
        command.env_clear();
        for name in ["PATH", "LANG", "LC_ALL"] {
            if let Ok(value) = std::env::var(name) {
                command.env(name, value);
            }
        }
        command
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child =
            command
                .spawn()
                .map_err(|error| TraceContributionError::RedactionFailed {
                    reason: format!(
                        "failed to spawn privacy filter sidecar {}: {}",
                        self.command.display(),
                        error
                    ),
                })?;

        let mut stdin =
            child
                .stdin
                .take()
                .ok_or_else(|| TraceContributionError::RedactionFailed {
                    reason: "privacy filter sidecar stdin was not available".to_string(),
                })?;
        let request = PrivacyFilterSidecarRequest::new(text);
        let request_body = serde_json::to_vec(&request).map_err(|error| {
            TraceContributionError::RedactionFailed {
                reason: format!("failed to serialize privacy filter request: {error}"),
            }
        })?;
        stdin.write_all(&request_body).await.map_err(|error| {
            TraceContributionError::RedactionFailed {
                reason: format!("failed to write privacy filter request: {error}"),
            }
        })?;
        drop(stdin);

        let output = tokio::time::timeout(self.timeout, child.wait_with_output())
            .await
            .map_err(|_| TraceContributionError::RedactionFailed {
                reason: format!(
                    "privacy filter sidecar timed out after {}ms",
                    self.timeout.as_millis()
                ),
            })?
            .map_err(|error| TraceContributionError::RedactionFailed {
                reason: format!("privacy filter sidecar failed: {error}"),
            })?;

        if output.stdout.len() > self.max_stdout_bytes {
            return Err(TraceContributionError::RedactionFailed {
                reason: format!(
                    "stdout exceeded privacy filter sidecar limit: stdout_len={} max_stdout_bytes={}",
                    output.stdout.len(),
                    self.max_stdout_bytes
                ),
            });
        }
        if output.stderr.len() > self.max_stderr_bytes {
            return Err(TraceContributionError::RedactionFailed {
                reason: format!(
                    "stderr exceeded privacy filter sidecar limit: stderr_len={} stderr_hash={} max_stderr_bytes={}",
                    output.stderr.len(),
                    privacy_filter_bytes_hash(&output.stderr),
                    self.max_stderr_bytes
                ),
            });
        }

        if !output.status.success() {
            return Err(TraceContributionError::RedactionFailed {
                reason: format!(
                    "privacy filter sidecar exited with {}; stderr_len={} stderr_hash={}",
                    output.status,
                    output.stderr.len(),
                    privacy_filter_bytes_hash(&output.stderr)
                ),
            });
        }

        let value: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
            TraceContributionError::RedactionFailed {
                reason: format!("failed to parse privacy filter output: {error}"),
            }
        })?;
        safe_privacy_filter_redaction_from_output(&value).map(Some)
    }
}

fn privacy_filter_bytes_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{}", hex::encode(digest))
}

pub fn privacy_filter_adapter_from_env() -> Result<
    Option<(Arc<dyn PrivacyFilterAdapter>, PrivacyFilterBackendTag)>,
    PrivacyFilterConfigError,
> {
    let backend = match std::env::var("TRACE_PRIVACY_FILTER_BACKEND") {
        Ok(value) => value.trim().to_string(),
        Err(_) => String::new(),
    };
    if backend.is_empty() {
        return Ok(None);
    }
    match backend.as_str() {
        "sidecar" => {
            build_sidecar_adapter().map(|adapter| Some((adapter, PrivacyFilterBackendTag::Sidecar)))
        }
        "near-ai" => {
            build_near_ai_adapter().map(|adapter| Some((adapter, PrivacyFilterBackendTag::NearAi)))
        }
        other => Err(PrivacyFilterConfigError::UnknownBackend {
            value: other.to_string(),
        }),
    }
}

/// Resolve which privacy-filter backend the environment configures, without
/// keeping the adapter.
///
/// Callers use this at boot to report the live backend and to refuse to start
/// when a deployment requires one. It exists because the absence of a filter
/// is otherwise invisible: [`privacy_filter_adapter_from_env`] returns
/// `Ok(None)` for an unset backend, the redactor built from it performs no
/// prose-PII filtering, and a runtime filter failure falls back to the
/// unfiltered text. None of those paths distinguish "filtered and found
/// nothing" from "never filtered".
///
/// A backend named without its credentials is an error here, so that
/// misconfiguration surfaces once at startup rather than on every submission.
pub fn privacy_filter_backend_from_env() -> Result<PrivacyFilterBackendTag, PrivacyFilterConfigError>
{
    Ok(privacy_filter_adapter_from_env()?
        .map(|(_adapter, tag)| tag)
        .unwrap_or(PrivacyFilterBackendTag::None))
}

fn build_sidecar_adapter() -> Result<Arc<dyn PrivacyFilterAdapter>, PrivacyFilterConfigError> {
    let command = read_privacy_env(
        "TRACE_PRIVACY_FILTER_COMMAND",
        "IRONCLAW_TRACE_PRIVACY_FILTER_COMMAND",
    )
    .ok_or(PrivacyFilterConfigError::MissingEnv {
        backend: "sidecar",
        var: "TRACE_PRIVACY_FILTER_COMMAND",
    })?;

    let args = read_privacy_env(
        "TRACE_PRIVACY_FILTER_ARGS",
        "IRONCLAW_TRACE_PRIVACY_FILTER_ARGS",
    )
    .map(|raw| {
        raw.split_whitespace()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();

    let mut adapter = CommandPrivacyFilterAdapter::new(command).with_args(args);
    if let Some(value) = read_privacy_env(
        "TRACE_PRIVACY_FILTER_TIMEOUT_MS",
        "IRONCLAW_TRACE_PRIVACY_FILTER_TIMEOUT_MS",
    ) {
        let ms = value
            .parse::<u64>()
            .map_err(|err| PrivacyFilterConfigError::InvalidEnv {
                var: "TRACE_PRIVACY_FILTER_TIMEOUT_MS",
                reason: err.to_string(),
            })?;
        adapter = adapter.with_timeout(Duration::from_millis(ms));
    }
    if let Some(value) = read_privacy_env(
        "TRACE_PRIVACY_FILTER_MAX_INPUT_BYTES",
        "IRONCLAW_TRACE_PRIVACY_FILTER_MAX_INPUT_BYTES",
    ) {
        let bytes = value
            .parse::<usize>()
            .map_err(|err| PrivacyFilterConfigError::InvalidEnv {
                var: "TRACE_PRIVACY_FILTER_MAX_INPUT_BYTES",
                reason: err.to_string(),
            })?;
        adapter = adapter.with_input_limit(bytes);
    }
    let max_stdout = read_privacy_env(
        "TRACE_PRIVACY_FILTER_MAX_STDOUT_BYTES",
        "IRONCLAW_TRACE_PRIVACY_FILTER_MAX_STDOUT_BYTES",
    )
    .map(|v| {
        v.parse::<usize>()
            .map_err(|err| PrivacyFilterConfigError::InvalidEnv {
                var: "TRACE_PRIVACY_FILTER_MAX_STDOUT_BYTES",
                reason: err.to_string(),
            })
    })
    .transpose()?;
    let max_stderr = read_privacy_env(
        "TRACE_PRIVACY_FILTER_MAX_STDERR_BYTES",
        "IRONCLAW_TRACE_PRIVACY_FILTER_MAX_STDERR_BYTES",
    )
    .map(|v| {
        v.parse::<usize>()
            .map_err(|err| PrivacyFilterConfigError::InvalidEnv {
                var: "TRACE_PRIVACY_FILTER_MAX_STDERR_BYTES",
                reason: err.to_string(),
            })
    })
    .transpose()?;
    if max_stdout.is_some() || max_stderr.is_some() {
        adapter = adapter.with_output_limits(
            max_stdout.unwrap_or(PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDOUT_BYTES),
            max_stderr.unwrap_or(PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDERR_BYTES),
        );
    }
    Ok(Arc::new(adapter))
}

#[cfg(not(feature = "near-ai-privacy-filter"))]
fn build_near_ai_adapter() -> Result<Arc<dyn PrivacyFilterAdapter>, PrivacyFilterConfigError> {
    Err(PrivacyFilterConfigError::FeatureDisabled {
        backend: "near-ai",
        feature: "near-ai-privacy-filter",
    })
}

#[cfg(feature = "near-ai-privacy-filter")]
fn build_near_ai_adapter() -> Result<Arc<dyn PrivacyFilterAdapter>, PrivacyFilterConfigError> {
    crate::privacy_filter_near_ai::build_from_env()
}

pub(crate) fn read_privacy_env(canonical: &str, legacy: &str) -> Option<String> {
    let canonical_present = std::env::var(canonical)
        .ok()
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    if let Ok(value) = std::env::var(canonical) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    if let Ok(value) = std::env::var(legacy) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            if !canonical_present {
                emit_legacy_privacy_env_deprecation_warning_once();
            }
            return Some(trimmed.to_string());
        }
    }
    None
}

static LEGACY_PRIVACY_ENV_WARNED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn emit_legacy_privacy_env_deprecation_warning_once() {
    if LEGACY_PRIVACY_ENV_WARNED
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        )
        .is_ok()
    {
        eprintln!(
            "warning: legacy IRONCLAW_TRACE_PRIVACY_FILTER_* environment variables are deprecated; \
             rename to TRACE_PRIVACY_FILTER_* (legacy names will be removed in a future release)"
        );
    }
}

#[cfg(test)]
pub(crate) fn reset_legacy_privacy_env_warning_for_tests() {
    LEGACY_PRIVACY_ENV_WARNED.store(false, std::sync::atomic::Ordering::SeqCst);
}

fn privacy_filter_backend_label(backend: PrivacyFilterBackendTag) -> &'static str {
    backend.label()
}

#[derive(Debug, Clone, Copy)]
enum SecretLeakSeverity {
    High,
    Critical,
}

#[derive(Debug, Clone)]
struct SecretLeakMatch {
    pattern_name: &'static str,
    severity: SecretLeakSeverity,
    location: std::ops::Range<usize>,
}

#[derive(Debug)]
struct SecretLeakScan {
    matches: Vec<SecretLeakMatch>,
}

impl SecretLeakScan {
    fn is_clean(&self) -> bool {
        self.matches.is_empty()
    }
}

#[derive(Debug, Default)]
struct SecretLeakDetector;

impl SecretLeakDetector {
    fn new() -> Self {
        Self
    }

    fn scan(&self, content: &str) -> SecretLeakScan {
        let mut matches = Vec::new();
        for pattern in secret_leak_patterns() {
            for matched in pattern.regex.find_iter(content) {
                matches.push(SecretLeakMatch {
                    pattern_name: pattern.name,
                    severity: pattern.severity,
                    location: matched.start()..matched.end(),
                });
            }
        }
        matches.sort_by_key(|matched| matched.location.start);
        SecretLeakScan { matches }
    }
}

struct SecretLeakPattern {
    name: &'static str,
    severity: SecretLeakSeverity,
    regex: Regex,
}

/// Chars scanned before a candidate high-entropy token to look for a
/// secret-shaped cue (`api_key:`, `Bearer `, `password=`, ...).
const CUE_WINDOW: usize = 48;
/// Minimum candidate token length considered for contextual-entropy
/// detection. Shorter tokens are too noisy to gate reliably.
const ENTROPY_MIN_LEN: usize = 16;
/// Minimum Shannon entropy (bits/char) for a candidate token to be treated
/// as opaque high-entropy secret material.
const ENTROPY_BITS_MIN: f64 = 3.2;
/// Size of each window used when measuring entropy over a bounded (`=`-split)
/// reading of a candidate.
///
/// A bounded reading is only taken once the cheap gates in [`is_cued_secret`]
/// (length, cue, allowlist) have passed, so this is not on the hot path an
/// attacker controls by stuffing a candidate with `=`; see the ordering note
/// there. [`entropy_sample_bits`] covers the whole candidate in windows of
/// this size, so a candidate of any length is measured completely rather than
/// only at fixed anchors.
///
/// Kept close to [`ENTROPY_MIN_LEN`] rather than large: entropy is measured
/// as an aggregate over the whole window, so a short opaque secret sitting in
/// a window otherwise full of low-entropy filler is diluted by that filler --
/// a big window can miss a real secret even when a window does cover it. A
/// smaller window bounds how much filler can dilute the one window that
/// contains the secret.
const ENTROPY_SAMPLE_BYTES: usize = 64;

/// Entropy of `token`, measured as the maximum over a set of windows that
/// together cover the whole token, on char boundaries.
///
/// This only runs once [`is_cued_secret`]'s cheap gates (length, cue,
/// allowlist) have already passed, so it is on the rare path, not the hot
/// one -- see the ordering note on [`is_cued_secret`].
///
/// A fixed number of windows spread evenly across the token (the previous
/// approach here) leaves a blind spot on any token long enough that the
/// spacing between windows exceeds a window: opaque material placed between
/// two windows, surrounded by low-entropy filler, is never sampled. Instead,
/// windows advance by half a window each step. Any run of opaque material no
/// longer than half a window cannot fall entirely in the gap between two
/// windows -- it always lands wholly inside at least one of them -- so the
/// whole token is covered with no length-dependent gap. Cost stays linear in
/// the token's length: the window count grows with the token, but each
/// window is a fixed [`ENTROPY_SAMPLE_BYTES`]-byte scan.
fn entropy_sample_bits(token: &str) -> f64 {
    if token.len() <= ENTROPY_SAMPLE_BYTES {
        return token_shannon_entropy(token);
    }
    let step = (ENTROPY_SAMPLE_BYTES / 2).max(1);
    let mut best = 0.0f64;
    let mut start = 0usize;
    loop {
        while start < token.len() && !token.is_char_boundary(start) {
            start += 1;
        }
        let mut end = (start + ENTROPY_SAMPLE_BYTES).min(token.len());
        while end > start && !token.is_char_boundary(end) {
            end -= 1;
        }
        best = best.max(token_shannon_entropy(&token[start..end]));
        if end >= token.len() {
            break;
        }
        start += step;
    }
    best
}

/// Precomputed windowed-entropy measurements over a whole candidate, built
/// once and reused across every `=`-split reading attempted on it.
///
/// A candidate can contain many `=`, and each one immediately preceded by
/// what looks like a cue is trivial for an attacker to arrange (just repeat
/// the cue text), so [`contextual_entropy_secret_ranges`] may need an answer
/// to "what is the bounded entropy of the reading starting here?" many times
/// for the SAME candidate. Recomputing [`entropy_sample_bits`] from scratch
/// on every attempt is linear in the remaining candidate length each time --
/// summed over many attempts on one long candidate, that is quadratic again,
/// exactly what the cheap-gate ordering fix on [`is_cued_secret`] was meant
/// to remove (that fix only removes the cost when there is no cue at all;
/// this removes it when there is a cue at many positions). Building the
/// window profile once, in one linear pass over the candidate, and answering
/// every reading with an O(log windows) suffix-max lookup keeps total
/// entropy work linear in the candidate's length however many `=` it
/// contains or how many of them have a cue.
struct EntropyWindowProfile<'a> {
    candidate: &'a str,
    /// Offset of `candidate`'s start within the larger `content` string that
    /// [`is_cued_secret`] is called against, so callers can pass it an
    /// absolute byte offset.
    candidate_start: usize,
    /// Window start offsets, relative to `candidate`, in ascending order.
    starts: Vec<usize>,
    /// `suffix_max[i]` is the largest entropy among `starts[i..]`'s windows.
    suffix_max: Vec<f64>,
}

impl<'a> EntropyWindowProfile<'a> {
    fn build(candidate: &'a str, candidate_start: usize) -> Self {
        if candidate.len() <= ENTROPY_SAMPLE_BYTES {
            return Self {
                candidate,
                candidate_start,
                starts: vec![0],
                suffix_max: vec![token_shannon_entropy(candidate)],
            };
        }
        let step = (ENTROPY_SAMPLE_BYTES / 2).max(1);
        let mut starts = Vec::new();
        let mut bits = Vec::new();
        let mut start = 0usize;
        loop {
            while start < candidate.len() && !candidate.is_char_boundary(start) {
                start += 1;
            }
            let mut end = (start + ENTROPY_SAMPLE_BYTES).min(candidate.len());
            while end > start && !candidate.is_char_boundary(end) {
                end -= 1;
            }
            starts.push(start);
            bits.push(token_shannon_entropy(&candidate[start..end]));
            if end >= candidate.len() {
                break;
            }
            start += step;
        }
        let mut suffix_max = vec![0.0f64; bits.len()];
        let mut running = 0.0f64;
        for i in (0..bits.len()).rev() {
            running = running.max(bits[i]);
            suffix_max[i] = running;
        }
        Self {
            candidate,
            candidate_start,
            starts,
            suffix_max,
        }
    }

    /// Largest window entropy among windows that start at or after
    /// `absolute_start` (a byte offset into the original `content`, not into
    /// `candidate`), matching what a from-scratch [`entropy_sample_bits`]
    /// call over `content[absolute_start..]` would report. Falls back to
    /// measuring the exact remaining slice directly when `absolute_start`
    /// falls after the last precomputed window start; that only happens
    /// within the last window's span, so the fallback slice is short.
    fn bits_from(&self, absolute_start: usize) -> f64 {
        let from = absolute_start.saturating_sub(self.candidate_start);
        let idx = self.starts.partition_point(|&s| s < from);
        self.suffix_max.get(idx).copied().unwrap_or_else(|| {
            token_shannon_entropy(&self.candidate[from.min(self.candidate.len())..])
        })
    }
}

/// Shannon entropy in bits/char over the token's byte distribution.
fn token_shannon_entropy(s: &str) -> f64 {
    if s.is_empty() {
        return 0.0;
    }
    let mut counts: std::collections::HashMap<u8, usize> = std::collections::HashMap::new();
    for byte in s.bytes() {
        *counts.entry(byte).or_insert(0) += 1;
    }
    let len = s.len() as f64;
    counts
        .values()
        .map(|&count| {
            let p = count as f64 / len;
            -p * p.log2()
        })
        .sum()
}

fn secret_cue_regex() -> &'static Regex {
    static SECRET_CUE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        // safety: hardcoded regex is covered by unit tests and should always compile.
        Regex::new(
            r"(?i)(authorization|bearer|api[_-]?key|secret|password|passwd|access[_-]?token|client[_-]?secret|private[_-]?key|token|apikey)[\x22'`:=\s]{1,6}$",
        )
        .expect("hardcoded secret cue regex must compile")
    });
    &SECRET_CUE_REGEX
}

fn entropy_candidate_regex() -> &'static Regex {
    static ENTROPY_CANDIDATE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        // safety: hardcoded regex is covered by unit tests and should always compile.
        Regex::new(r"[A-Za-z0-9+/=_.\-]{16,}")
            .expect("hardcoded entropy candidate regex must compile")
    });
    &ENTROPY_CANDIDATE_REGEX
}

fn uuid_regex() -> &'static Regex {
    static UUID_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        // safety: hardcoded regex is covered by unit tests and should always compile.
        Regex::new(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
            .expect("hardcoded UUID regex must compile")
    });
    &UUID_REGEX
}

/// Structural ID prefixes observed across real transcripts (message ids,
/// request ids, tool-call ids, ...). These are opaque and high-entropy by
/// construction but are not secrets; a prototype scan against real
/// transcripts found ~105k such tokens against ~20 real secrets, which is
/// why this allowlist exists rather than relying on entropy alone.
const ALLOWLISTED_ID_PREFIXES: &[&str] = &[
    "msg_", "req_", "mcp_", "toolu_", "chatcmpl", "run_", "file_", "asst_", "resp_", "call_",
];

/// Exact `RedactionReport` metric-key label fragments this module emits via
/// `report.increment("secret:<label>")` / `report.increment("secret:contextual_entropy")`
/// (see the `secret:` increments below and in `apply_pem_block_redaction`).
/// These diagnostic counter names are embedded in the finished envelope's
/// `PrivacyMetadata::redaction_counts` and therefore appear in the very JSON
/// that `envelope_has_residual_secret`-style fail-closed guards re-scan.
/// Without this allowlist, the contextual-entropy pass would flag its own
/// bookkeeping key (e.g. `"secret:contextual_entropy"`, an 18-char
/// underscore-joined identifier immediately preceded by the cue word
/// `secret:`) as a surviving secret, wrongly refusing any session whose
/// redaction pipeline legitimately found and redacted something -- exactly
/// the sessions the guard exists to let through. Exact-match only: a real
/// secret value is astronomically unlikely to equal one of these literal
/// label strings verbatim.
const REPORT_METRIC_LABELS: &[&str] = &[
    "contextual_entropy",
    "openai_api_key",
    "github_token",
    "aws_access_key",
    "provider_token",
    "npm_token",
    "google_api_key",
    "pem_header_orphan",
    "pem_private_key",
];

fn is_pure_hex(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// True when the candidate token is a structural identifier (UUID, known ID
/// prefix, hex hash/sha, or this module's own report-metric label) rather
/// than an opaque secret.
fn is_allowlisted_entropy_candidate(token: &str) -> bool {
    if uuid_regex().is_match(token) {
        return true;
    }
    if ALLOWLISTED_ID_PREFIXES
        .iter()
        .any(|prefix| token.starts_with(prefix))
    {
        return true;
    }
    if REPORT_METRIC_LABELS.contains(&token) {
        return true;
    }
    if is_pure_hex(token) && matches!(token.len(), 7 | 8 | 40 | 64) {
        return true;
    }
    // All-lowercase-hex of length >= 32 with no uppercase and no non-hex
    // chars reads as a content hash (e.g. sha256/git blob), not a secret.
    if token.len() >= 32
        && token
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return true;
    }
    false
}

/// Start of the cue window preceding `start`, snapped to a char boundary.
fn cue_window_start(content: &str, start: usize) -> usize {
    let mut window_start = start.saturating_sub(CUE_WINDOW);
    while window_start > 0 && !content.is_char_boundary(window_start) {
        window_start -= 1;
    }
    window_start
}

/// True when a secret cue sits within [`CUE_WINDOW`] chars before `start`.
///
/// Cheap and highly selective, so it runs before the length-proportional
/// entropy scan: an ungated entropy pass over real transcripts flags on the
/// order of 105k structural tokens against ~20 real secrets, and computing
/// entropy for all of them is both wasted work and, on `=`-dense input where
/// every suffix is retried, quadratic.
fn has_secret_cue(content: &str, start: usize) -> bool {
    secret_cue_regex().is_match(&content[cue_window_start(content, start)..start])
}

/// True when `content[start..end]` is preceded by a secret cue, long enough,
/// opaque enough, and not a structural identifier. Fail-closed: when unsure
/// whether a token is a structural identifier, it is redacted.
///
/// This exists to catch secrets in formats not covered by
/// [`secret_leak_patterns`] (unknown provider key shapes, ad hoc tokens, etc).
/// `bounded` measures entropy via [`entropy_sample_bits`]'s windowed scan
/// instead of a single whole-token pass. It is set only for the readings this
/// pass adds, never for the whole-token reading, so the pre-existing decision
/// is reproduced exactly and no input that was redacted can stop being
/// redacted.
///
/// The cheap gates (length, cue, allowlist) run FIRST and return early; entropy
/// is measured only once all three have passed. Every `=` in the input starts
/// another candidate reading, so on `=`-dense input the entropy scan -- a
/// windowed pass over [`ENTROPY_SAMPLE_BYTES`]-sized chunks of the whole
/// candidate -- would otherwise run once per `=` even though
/// [`has_secret_cue`] rejects nearly all of them. Ordering the cheap checks
/// first keeps the expensive path off that hot loop.
///
/// `profile`, when given, answers a bounded reading from a precomputed
/// [`EntropyWindowProfile`] instead of rescanning the candidate from
/// scratch; see that type for why. Pass `None` to measure directly, which
/// [`contextual_entropy_secret_ranges`] only does before a profile exists to
/// build.
fn is_cued_secret(
    content: &str,
    start: usize,
    end: usize,
    bounded: bool,
    profile: Option<&EntropyWindowProfile<'_>>,
) -> bool {
    let token = &content[start..end];
    if token.len() < ENTROPY_MIN_LEN {
        return false;
    }
    if !has_secret_cue(content, start) {
        return false;
    }
    if is_allowlisted_entropy_candidate(token) {
        return false;
    }
    let measured_bits = if bounded {
        match profile {
            Some(profile) => profile.bits_from(start),
            None => entropy_sample_bits(token),
        }
    } else {
        token_shannon_entropy(token)
    };
    measured_bits >= ENTROPY_BITS_MIN
}

/// Byte ranges of high-entropy tokens that a secret cue points at.
///
/// Scope: only unspaced `=` assignments (`api_key=<value>`) are re-anchored
/// here. A literal zero-separator glue with no `=` at all (`api_keySECRET`,
/// `BearerSECRET`) is NOT covered -- there is no `=` to split on, so the cue
/// word and the value are one token and [`has_secret_cue`]'s window still
/// never sees a cue word immediately before `start`. That is a separate,
/// unaddressed gap, not a variant of the one this function fixes.
fn contextual_entropy_secret_ranges(content: &str) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    for candidate in entropy_candidate_regex().find_iter(content) {
        // The candidate class contains `=`, so an unspaced assignment such as
        // `api_key=<secret>` arrives as ONE token with the cue glued on. The
        // cue then sits inside the token being judged instead of in the window
        // before it, and [`secret_cue_regex`] — anchored to the end of that
        // window — never sees it, so the value survives. The same secret with a
        // space is redacted.
        //
        // Re-anchoring after each `=` puts the cue back into the window the
        // existing regex already inspects, which also covers compound names
        // (`OPENAI_API_KEY=`, `x-api-key=`) because that regex is unanchored on
        // the left. The whole-token reading is tried FIRST, so this can only
        // ever add coverage: every range the previous logic produced is still
        // produced.
        let candidate_str = candidate.as_str();
        // Built lazily and at most once per candidate, the first time a
        // bounded reading needs one -- see `EntropyWindowProfile`. Gating the
        // build on `has_secret_cue` (not just `bounded`) keeps a candidate
        // with many `=` but no cue anywhere (the DoS case the ordering fix on
        // `is_cued_secret` targets) from paying even this one-time cost.
        let mut profile: Option<EntropyWindowProfile<'_>> = None;
        let cued_start = std::iter::once((candidate.start(), false))
            .chain(
                candidate_str
                    .match_indices('=')
                    .map(|(index, _)| (candidate.start() + index + 1, true)),
            )
            .find(|&(start, bounded)| {
                if bounded && profile.is_none() && has_secret_cue(content, start) {
                    profile = Some(EntropyWindowProfile::build(
                        candidate_str,
                        candidate.start(),
                    ));
                }
                is_cued_secret(content, start, candidate.end(), bounded, profile.as_ref())
            })
            .map(|(start, _)| start);
        if let Some(start) = cued_start {
            ranges.push(start..candidate.end());
        }
    }
    ranges
}

fn secret_leak_patterns() -> &'static [SecretLeakPattern] {
    static SECRET_LEAK_PATTERNS: LazyLock<Vec<SecretLeakPattern>> = LazyLock::new(|| {
        vec![
            SecretLeakPattern {
                name: "openai_api_key",
                severity: SecretLeakSeverity::Critical,
                regex: Regex::new(r"\bsk-[A-Za-z0-9_-]{20,}\b")
                    .expect("hardcoded OpenAI key regex must compile"),
            },
            SecretLeakPattern {
                name: "github_token",
                severity: SecretLeakSeverity::Critical,
                regex: Regex::new(r"\bgh[pousr]_[A-Za-z0-9_]{10,}\b")
                    .expect("hardcoded GitHub token regex must compile"),
            },
            SecretLeakPattern {
                name: "aws_access_key",
                severity: SecretLeakSeverity::High,
                regex: Regex::new(r"\bAKIA[0-9A-Z]{16}\b")
                    .expect("hardcoded AWS access key regex must compile"),
            },
            SecretLeakPattern {
                name: "provider_token",
                severity: SecretLeakSeverity::Critical,
                regex: Regex::new(r"(?i)\b(?:rk|pk|glpat|xox[baprs])[-_a-z0-9]{8,}\b")
                    .expect("hardcoded provider token regex must compile"),
            },
            SecretLeakPattern {
                name: "jwt",
                severity: SecretLeakSeverity::Critical,
                regex: Regex::new(
                    r"\beyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b",
                )
                .expect("hardcoded JWT regex must compile"),
            },
            SecretLeakPattern {
                name: "npm_token",
                severity: SecretLeakSeverity::Critical,
                regex: Regex::new(r"\bnpm_[A-Za-z0-9]{36}\b")
                    .expect("hardcoded npm token regex must compile"),
            },
            SecretLeakPattern {
                name: "google_api_key",
                severity: SecretLeakSeverity::High,
                regex: Regex::new(r"\bAIza[0-9A-Za-z_-]{35,}\b")
                    .expect("hardcoded Google API key regex must compile"),
            },
            SecretLeakPattern {
                name: "pem_header_orphan",
                severity: SecretLeakSeverity::Critical,
                regex: Regex::new(r"-----BEGIN [A-Z ]*PRIVATE KEY-----[A-Za-z0-9+/=\s]*")
                    .expect("hardcoded orphan private key header regex must compile"),
            },
        ]
    });
    SECRET_LEAK_PATTERNS.as_slice()
}

/// The names of every named secret detector, in table order.
///
/// Exists so a client can TELL a contributor what is scrubbed without
/// transcribing the list. A hand-written copy of this in a shell is a
/// sentence that silently stops being true the day a detector is added, and
/// it would be making a privacy claim while doing it -- which is the one
/// category of drift this codebase can least afford.
///
/// Names only. The regexes are not published: a contributor deciding whether
/// to trust the scrubbing needs to know what it looks for, and printing the
/// patterns would tell someone trying to slip a secret past it exactly what
/// to avoid.
pub fn secret_leak_pattern_names() -> Vec<&'static str> {
    secret_leak_patterns().iter().map(|p| p.name).collect()
}

pub struct DeterministicTraceRedactor {
    leak_detector: SecretLeakDetector,
    known_path_prefixes: Vec<String>,
    privacy_filter: Option<Arc<dyn PrivacyFilterAdapter>>,
    privacy_filter_backend: PrivacyFilterBackendTag,
}

impl Default for DeterministicTraceRedactor {
    fn default() -> Self {
        Self::try_default().expect(
            "DeterministicTraceRedactor::default(): privacy filter config invalid; use try_default()",
        )
    }
}

impl DeterministicTraceRedactor {
    pub fn deterministic_only(known_path_prefixes: Vec<String>) -> Self {
        let mut known_path_prefixes: Vec<String> = known_path_prefixes
            .into_iter()
            .filter(|prefix| !prefix.trim().is_empty())
            .collect();
        known_path_prefixes.sort_by_key(|prefix| std::cmp::Reverse(prefix.len()));
        known_path_prefixes.dedup();

        Self {
            leak_detector: SecretLeakDetector::new(),
            known_path_prefixes,
            privacy_filter: None,
            privacy_filter_backend: PrivacyFilterBackendTag::None,
        }
    }

    /// A redactor with no attached privacy-filter adapter and no known path
    /// prefixes, for detection-only work that never touches
    /// `attached_privacy_filter`. Unlike `new`/`try_default`, this never
    /// reads `TRACE_PRIVACY_FILTER_BACKEND` or its adapter-specific env
    /// vars, so it cannot race concurrent env mutation elsewhere in the
    /// process and cannot fail from missing/invalid privacy-filter config -
    /// exactly what the residual scan needs, since it only calls the plain
    /// `redact_text`, which never consults the attached adapter.
    fn bare() -> Self {
        Self {
            leak_detector: SecretLeakDetector::new(),
            known_path_prefixes: Vec::new(),
            privacy_filter: None,
            privacy_filter_backend: PrivacyFilterBackendTag::None,
        }
    }

    pub fn new(known_path_prefixes: Vec<String>) -> Result<Self, PrivacyFilterConfigError> {
        let mut known_path_prefixes: Vec<String> = known_path_prefixes
            .into_iter()
            .filter(|prefix| !prefix.trim().is_empty())
            .collect();
        known_path_prefixes.sort_by_key(|prefix| std::cmp::Reverse(prefix.len()));
        known_path_prefixes.dedup();

        let (privacy_filter, privacy_filter_backend) = match privacy_filter_adapter_from_env()? {
            Some((adapter, tag)) => (Some(adapter), tag),
            None => (None, PrivacyFilterBackendTag::None),
        };

        Ok(Self {
            leak_detector: SecretLeakDetector::new(),
            known_path_prefixes,
            privacy_filter,
            privacy_filter_backend,
        })
    }

    pub fn try_default() -> Result<Self, PrivacyFilterConfigError> {
        let mut known_path_prefixes = Vec::new();
        if let Some(home) = dirs::home_dir() {
            known_path_prefixes.push(path_to_string(home));
        }
        if let Ok(current_dir) = std::env::current_dir() {
            known_path_prefixes.push(path_to_string(current_dir));
        }
        Self::new(known_path_prefixes)
    }

    pub fn with_privacy_filter(
        mut self,
        adapter: Arc<dyn PrivacyFilterAdapter>,
        backend: PrivacyFilterBackendTag,
    ) -> Self {
        self.privacy_filter = Some(adapter);
        self.privacy_filter_backend = backend;
        self
    }

    /// The optional privacy-filter adapter attached to this redactor (via
    /// `with_privacy_filter` or an env-configured backend picked up by
    /// `new`). Callers use this to run [`run_privacy_filter_canary`] against
    /// whatever backend is actually wired in before trusting it with real
    /// traffic.
    pub fn attached_privacy_filter(&self) -> Option<&Arc<dyn PrivacyFilterAdapter>> {
        self.privacy_filter.as_ref()
    }

    async fn apply_privacy_filter_to_text(
        &self,
        text: String,
        report: &mut RedactionReport,
        privacy_filter_summary: &mut Option<SafePrivacyFilterSummary>,
    ) -> Result<String, TraceContributionError> {
        let Some(adapter) = self.privacy_filter.as_ref() else {
            return Ok(text);
        };
        let redaction = match adapter.redact_text(&text).await {
            Ok(Some(redaction)) => redaction,
            Ok(None) => return Ok(text),
            Err(error) => {
                match self.privacy_filter_backend {
                    PrivacyFilterBackendTag::NearAi => {
                        // Spec fail-closed: surface as RedactionFailed.
                        return Err(error);
                    }
                    PrivacyFilterBackendTag::Sidecar => {
                        let error_text = error.to_string();
                        let backend_label =
                            privacy_filter_backend_label(self.privacy_filter_backend);
                        report.increment(format!("privacy_filter:{backend_label}_failure"));
                        // The configured filter did not examine this text, so
                        // this pass cannot claim coverage of it. Fail closed.
                        report.coverage_incomplete = true;
                        report.add_warning(format!(
                            "Privacy Filter {backend_label} backend failed; deterministic redaction fallback was used. error_hash={}",
                            canonical_hash(&error_text)
                        ));
                        return Ok(text);
                    }
                    PrivacyFilterBackendTag::None => {
                        // Unreachable: when backend tag is None, no adapter is
                        // installed and we returned early above. Be defensive
                        // and surface the error rather than silently swallow.
                        return Err(error);
                    }
                }
            }
        };

        merge_privacy_filter_summary(privacy_filter_summary, &redaction.summary);
        report.merge(redaction.report);
        Ok(redaction.redacted_text)
    }

    pub fn with_known_path_prefixes(
        prefixes: impl IntoIterator<Item = PathBuf>,
    ) -> Result<Self, PrivacyFilterConfigError> {
        Self::new(prefixes.into_iter().map(path_to_string).collect())
    }

    pub fn redact_text(&self, input: &str) -> (String, RedactionReport) {
        let mut state = RedactionState::default();
        self.redact_text_with_state(input, &mut state)
    }

    fn redact_text_with_state(
        &self,
        input: &str,
        state: &mut RedactionState,
    ) -> (String, RedactionReport) {
        let mut report = RedactionReport::default();
        let mut redacted = self.redact_private_emails(input, state, &mut report);
        redacted = self.redact_generic_paths(&redacted, state, &mut report);
        redacted = self.redact_known_paths(&redacted, state, &mut report);
        redacted = apply_pem_block_redaction(&redacted, &mut report);

        let scan = self.leak_detector.scan(&redacted);
        let mut ranges = scan
            .matches
            .iter()
            .map(|m| {
                report.increment("secret");
                report.increment(format!("secret:{}", m.pattern_name));
                if matches!(
                    m.severity,
                    SecretLeakSeverity::High | SecretLeakSeverity::Critical
                ) {
                    report.blocked_secret_detected = true;
                }
                m.location.clone()
            })
            .collect::<Vec<_>>();

        // Contextual-entropy pass: mop up unknown key formats missed by the
        // named patterns above. Runs over the already-pattern-redacted text
        // so known secrets stay attributed to their named rule; dedupe
        // against ranges already flagged so a token isn't double-counted.
        // Only the named-pattern ranges need the overlap check, and a single
        // forward cursor suffices: both sequences are ordered by start, and two
        // entropy ranges can never overlap each other because each comes from a
        // distinct non-overlapping candidate. Comparing every new range against
        // every range accumulated so far was quadratic, which a contribution
        // with thousands of glued assignments could reach.
        let named_range_count = ranges.len();
        let mut named_cursor = 0usize;
        for entropy_range in contextual_entropy_secret_ranges(&redacted) {
            while named_cursor < named_range_count
                && ranges[named_cursor].end <= entropy_range.start
            {
                named_cursor += 1;
            }
            let overlaps = ranges[named_cursor..named_range_count]
                .iter()
                .take_while(|existing| existing.start < entropy_range.end)
                .any(|existing| entropy_range.start < existing.end);
            if overlaps {
                continue;
            }
            report.increment("secret");
            report.increment("secret:contextual_entropy");
            report.blocked_secret_detected = true;
            ranges.push(entropy_range);
        }

        if ranges.is_empty() {
            return (redacted, report);
        }

        (apply_redaction_ranges(&redacted, &ranges), report)
    }

    fn redact_json_value(
        &self,
        tool_name: Option<&str>,
        value: &Value,
        state: &mut RedactionState,
    ) -> (Value, RedactionReport) {
        let mut report = RedactionReport::default();
        let tool_redacted = redact_tool_specific_payload(tool_name, value, &mut report);
        let keyed_redaction = redact_sensitive_json(&tool_redacted);
        count_sensitive_field_redactions(&tool_redacted, &keyed_redaction, &mut report);
        let redacted = self.redact_json_strings(keyed_redaction, state, &mut report);
        (redacted, report)
    }

    fn redact_json_strings(
        &self,
        value: Value,
        state: &mut RedactionState,
        report: &mut RedactionReport,
    ) -> Value {
        match value {
            Value::String(s) => {
                let (redacted, child_report) = self.redact_text_with_state(&s, state);
                report.merge(child_report);
                Value::String(redacted)
            }
            Value::Array(items) => Value::Array(
                items
                    .into_iter()
                    .map(|item| self.redact_json_strings(item, state, report))
                    .collect(),
            ),
            Value::Object(map) => Value::Object(
                map.into_iter()
                    .map(|(key, value)| (key, self.redact_json_strings(value, state, report)))
                    .collect(),
            ),
            other => other,
        }
    }

    fn redact_private_emails(
        &self,
        input: &str,
        state: &mut RedactionState,
        report: &mut RedactionReport,
    ) -> String {
        apply_placeholder_regex(input, private_email_regex(), "private_email", state, report)
    }

    fn redact_known_paths(
        &self,
        input: &str,
        state: &mut RedactionState,
        report: &mut RedactionReport,
    ) -> String {
        let mut output = input.to_string();
        for prefix in &self.known_path_prefixes {
            let count = output.matches(prefix).count();
            if count == 0 {
                continue;
            }
            let placeholder = state.placeholders.placeholder_for("local_path", prefix);
            output = output.replace(prefix, &placeholder);
            for _ in 0..count {
                report.increment("local_path");
                report.add_pii_label("local_path");
            }
        }
        output
    }

    fn redact_generic_paths(
        &self,
        input: &str,
        state: &mut RedactionState,
        report: &mut RedactionReport,
    ) -> String {
        apply_placeholder_regex(input, local_path_regex(), "local_path", state, report)
    }
}

#[derive(Debug, Default)]
struct RedactionState {
    placeholders: PlaceholderMap,
}

#[derive(Debug, Default)]
struct PlaceholderMap {
    by_label_and_value: BTreeMap<(String, String), String>,
    next_by_label: BTreeMap<String, u32>,
}

impl PlaceholderMap {
    fn placeholder_for(&mut self, label: &str, value: &str) -> String {
        let key = (label.to_string(), value.to_string());
        if let Some(existing) = self.by_label_and_value.get(&key) {
            return existing.clone();
        }

        let next = self.next_by_label.entry(label.to_string()).or_insert(0);
        *next += 1;
        let token = format!("<PRIVATE_{}_{}>", placeholder_label_fragment(label), *next);
        self.by_label_and_value.insert(key, token.clone());
        token
    }
}

#[async_trait]
impl TraceRedactor for DeterministicTraceRedactor {
    async fn redact_trace(
        &self,
        trace: RawTraceContribution,
    ) -> Result<TraceContributionEnvelope, TraceContributionError> {
        let mut report = RedactionReport::default();
        let mut state = RedactionState::default();
        let mut privacy_filter_summary = None;
        let mut events = Vec::with_capacity(trace.events.len());
        let trace_card_scopes = trace.consent.scopes.clone();
        let trace_card_channel = trace.ironclaw.channel;
        let trace_card_revocation_handle = trace.contributor.revocation_handle;

        for raw_event in trace.events {
            let redacted_content = match raw_event.content {
                Some(content) => {
                    let (mut redacted, child_report) =
                        self.redact_text_with_state(&content, &mut state);
                    report.merge(child_report);
                    redacted = self
                        .apply_privacy_filter_to_text(
                            redacted,
                            &mut report,
                            &mut privacy_filter_summary,
                        )
                        .await?;
                    Some(redacted)
                }
                None => None,
            };

            let (structured_payload, payload_report) = self.redact_json_value(
                raw_event.tool_name.as_deref(),
                &raw_event.structured_payload,
                &mut state,
            );
            report.merge(payload_report);

            // The explicit field wins; the payload key stays a fallback so an
            // emitter that already wrote `{"tool_call_id": ...}` keeps working.
            let tool_call_id = raw_event.tool_call_id.clone().or_else(|| {
                raw_event
                    .structured_payload
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            });
            let tool_category = raw_event.tool_name.as_deref().map(tool_category_for);
            let side_effect = side_effect_for(raw_event.event_type, raw_event.tool_name.as_deref());

            events.push(TraceContributionEvent {
                event_id: raw_event.event_id,
                parent_event_id: raw_event.parent_event_id,
                event_type: raw_event.event_type,
                timestamp: raw_event.timestamp,
                redacted_content,
                structured_payload,
                tool_name: raw_event.tool_name,
                tool_category,
                tool_call_id,
                latency_ms: raw_event.latency_ms,
                token_counts: raw_event.token_counts,
                cost_usd: raw_event.cost_usd,
                success: raw_event.success,
                failure_modes: raw_event.failure_modes,
                side_effect,
            });
        }

        let mut outcome = trace.outcome;
        if let Some(correction) = outcome.human_correction.take() {
            let (mut redacted, child_report) = self.redact_text_with_state(&correction, &mut state);
            report.merge(child_report);
            redacted = self
                .apply_privacy_filter_to_text(redacted, &mut report, &mut privacy_filter_summary)
                .await?;
            outcome.human_correction = Some(redacted);
        }

        let residual_pii_risk = residual_risk(&trace.consent, &report);
        let redaction_hash = redaction_hash(&events, &report.counts);
        let mut warnings = privacy_warnings(residual_pii_risk);
        warnings.extend(report.warnings.clone());
        let privacy = PrivacyMetadata {
            redaction_pipeline_version: redaction_pipeline_version(self.privacy_filter_backend),
            redaction_counts: report.counts,
            privacy_filter_summary,
            pii_labels_present: report.pii_labels_present,
            residual_pii_risk,
            redaction_hash,
            warnings,
        };

        let trace_card = build_trace_card(
            &trace_card_scopes,
            trace_card_channel,
            trace_card_revocation_handle,
            &events,
        );
        let value_card = TraceValueCard::default();
        Ok(TraceContributionEnvelope {
            schema_version: TRACE_CONTRIBUTION_SCHEMA_VERSION.to_string(),
            trace_id: trace.trace_id,
            submission_id: trace.submission_id,
            created_at: trace.created_at,
            ironclaw: trace.ironclaw,
            consent: trace.consent,
            contributor: trace.contributor,
            privacy,
            events,
            outcome,
            replay: trace.replay,
            embedding_analysis: trace.embedding_analysis,
            value: trace.value,
            trace_card,
            value_card,
            hindsight: None,
            training_dynamics: None,
            process_evaluation: None,
        })
    }
}

pub fn rescrub_trace_envelope(
    envelope: &mut TraceContributionEnvelope,
) -> Result<(), PrivacyFilterConfigError> {
    let redactor = DeterministicTraceRedactor::try_default()?;
    rescrub_trace_envelope_with(&redactor, envelope);
    Ok(())
}

pub fn rescrub_trace_envelope_with(
    redactor: &DeterministicTraceRedactor,
    envelope: &mut TraceContributionEnvelope,
) {
    // Consent flags are a factual declaration of what the envelope carries.
    // Correct under-reported flags before risk derivation so residual_risk
    // and the PII-backstop hold cannot be skipped by a false declaration.
    reconcile_consent_declarations(envelope);

    let mut report = RedactionReport::default();
    let mut state = RedactionState::default();

    for event in &mut envelope.events {
        if let Some(content) = event.redacted_content.take() {
            let (redacted, child_report) = redactor.redact_text_with_state(&content, &mut state);
            report.merge(child_report);
            event.redacted_content = Some(redacted);
        }

        if !event.structured_payload.is_null() {
            let (redacted_payload, child_report) = redactor.redact_json_value(
                event.tool_name.as_deref(),
                &event.structured_payload,
                &mut state,
            );
            report.merge(child_report);
            event.structured_payload = redacted_payload;
        }
    }

    if let Some(correction) = envelope.outcome.human_correction.take() {
        let (redacted, child_report) = redactor.redact_text_with_state(&correction, &mut state);
        report.merge(child_report);
        envelope.outcome.human_correction = Some(redacted);
    }

    redact_envelope_side_channels(redactor, envelope, &mut report, &mut state);

    // Detection-only backstop, run after every mutation above. The
    // typed traversal can only cover fields it knows about, and the
    // schema keeps growing; this catches whatever the traversal missed.
    // It never mutates - anything it finds has already survived
    // redaction, which is what makes it *residual* and why it forces
    // High rather than Medium.
    let residual = residual_envelope_scan(redactor, envelope);

    // Derive the server-pass risk from what the pass actually found,
    // before `report` is drained into the envelope below. Previously
    // this was computed from an empty report, so only a blocked secret
    // could raise the classification.
    let server_pass_risk = residual_risk(&envelope.consent, &report);

    let prior_risk = envelope.privacy.residual_pii_risk;
    // No classifier runs on this path, so there is no classifier evidence and
    // this assessment can never lower the prior risk.
    //
    // An earlier version set this `true` on the reasoning that the
    // deterministic pass is a pure function and therefore self-evidencing.
    // That is wrong, and it was a fail-open: being a pure function
    // establishes *availability*, not detection completeness or the absence
    // of PII. The prior risk is High because something already found cause
    // for concern; the deterministic patterns failing to match is a proxy for
    // cleanliness, not evidence of it. A High trace missed by the regex suite
    // would have been published without NEAR AI ever examining it.
    //
    // The pass still *raises* risk freely -- `resolve_post_scrub_risk` falls
    // back to `max_residual_risk`, and a residual finding still forces High.
    // Only the downgrade direction requires classifier evidence.
    let assessment = PostScrubAssessment {
        complete_coverage: residual.is_ok(),
        useful_classifier_result: false,
        findings: report.clone(),
        residual_findings: residual.clone().unwrap_or_default(),
    };
    envelope.privacy.residual_pii_risk = match residual {
        Ok(_) => resolve_post_scrub_risk(prior_risk, server_pass_risk, &assessment),
        Err(_) => {
            // Fail closed: the residual scan could not run, so this pass
            // cannot prove the envelope is clean. Never trust an empty
            // report produced by a failed scan; force the worst case.
            ResidualPiiRisk::High
        }
    };

    for (label, count) in report.counts {
        *envelope.privacy.redaction_counts.entry(label).or_insert(0) += count;
    }
    for label in report.pii_labels_present {
        if !envelope.privacy.pii_labels_present.contains(&label) {
            envelope.privacy.pii_labels_present.push(label);
        }
    }

    if !envelope
        .privacy
        .redaction_pipeline_version
        .contains(SERVER_RESCRUB_PIPELINE_SUFFIX)
    {
        envelope.privacy.redaction_pipeline_version.push('+');
        envelope
            .privacy
            .redaction_pipeline_version
            .push_str(SERVER_RESCRUB_PIPELINE_SUFFIX);
    }
    envelope.trace_card.redaction_pipeline_version =
        envelope.privacy.redaction_pipeline_version.clone();
    merge_privacy_warnings(
        &mut envelope.privacy.warnings,
        privacy_warnings(envelope.privacy.residual_pii_risk),
    );
    merge_privacy_warnings(
        &mut envelope.privacy.warnings,
        vec!["Server-side trace re-scrub was applied before corpus storage.".to_string()],
    );
    envelope.privacy.redaction_hash =
        redaction_hash(&envelope.events, &envelope.privacy.redaction_counts);
}

/// Byte/node/depth budgets for classifying `event.structured_payload` trees
/// through the async classifier (Task 3). These bound the async classifier
/// traffic and CPU work per envelope; hitting any of them makes coverage
/// incomplete rather than silently skipping the rest of the tree.
const STRUCTURED_PAYLOAD_MAX_AGGREGATE_BYTES: usize = 400_000;
const STRUCTURED_PAYLOAD_MAX_FIELD_BYTES: usize = 32_000;
const STRUCTURED_PAYLOAD_MAX_NODES: usize = 4_000;
const STRUCTURED_PAYLOAD_MAX_DEPTH: usize = 24;

#[derive(Default)]
struct StructuredPayloadBudget {
    aggregate_bytes: usize,
    nodes: usize,
}

/// Recursively classify every string leaf and object key inside a
/// structured tool payload through the async classifier.
///
/// String *values* are replaced with the classifier's redacted text (same
/// as the prose fields). Object *keys* are classified for detection only -
/// they are never rewritten, because two distinct keys can classify to
/// colliding redacted text and a key rewrite has no analogue to the
/// collision guard `insert_without_collision` gives map values. A finding
/// on a key therefore forces High via `RedactionReport::key_finding_detected`
/// rather than being "resolved" by a rewrite that could silently drop data.
///
/// Returns `Ok(true)` when the whole subtree was covered within budget, or
/// `Ok(false)` the moment any budget is exceeded (aggregate bytes, a single
/// field's bytes, node count, or recursion depth) - the caller treats that
/// as incomplete coverage, never as a silent skip.
fn classify_structured_payload_node<'a>(
    adapter: &'a dyn PrivacyFilterAdapter,
    value: &'a mut Value,
    depth: usize,
    budget: &'a mut StructuredPayloadBudget,
    report: &'a mut RedactionReport,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<bool, TraceContributionError>> + Send + 'a>,
> {
    Box::pin(async move {
        if depth > STRUCTURED_PAYLOAD_MAX_DEPTH {
            return Ok(false);
        }
        match value {
            Value::String(text) => {
                budget.nodes += 1;
                if budget.nodes > STRUCTURED_PAYLOAD_MAX_NODES {
                    return Ok(false);
                }
                if text.len() > STRUCTURED_PAYLOAD_MAX_FIELD_BYTES {
                    return Ok(false);
                }
                budget.aggregate_bytes += text.len();
                if budget.aggregate_bytes > STRUCTURED_PAYLOAD_MAX_AGGREGATE_BYTES {
                    return Ok(false);
                }
                if let Some(redaction) = adapter.redact_text(text).await? {
                    report.merge(redaction.report);
                    *text = redaction.redacted_text;
                }
                Ok(true)
            }
            Value::Array(items) => {
                for item in items.iter_mut() {
                    if !classify_structured_payload_node(adapter, item, depth + 1, budget, report)
                        .await?
                    {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Value::Object(entries) => {
                for (key, item) in entries.iter_mut() {
                    budget.nodes += 1;
                    if budget.nodes > STRUCTURED_PAYLOAD_MAX_NODES {
                        return Ok(false);
                    }
                    if key.len() > STRUCTURED_PAYLOAD_MAX_FIELD_BYTES {
                        return Ok(false);
                    }
                    budget.aggregate_bytes += key.len();
                    if budget.aggregate_bytes > STRUCTURED_PAYLOAD_MAX_AGGREGATE_BYTES {
                        return Ok(false);
                    }
                    if let Some(redaction) = adapter.redact_text(key).await? {
                        let has_finding = !redaction.report.counts.is_empty()
                            || !redaction.report.pii_labels_present.is_empty()
                            || redaction.report.blocked_secret_detected;
                        if has_finding {
                            report.key_finding_detected = true;
                        }
                    }
                    if !classify_structured_payload_node(adapter, item, depth + 1, budget, report)
                        .await?
                    {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => Ok(true),
        }
    })
}

/// Runs an async prose-PII filter (e.g. the NEAR AI backstop) over an
/// already-produced envelope's content-bearing fields:
/// `events[*].redacted_content`, `events[*].structured_payload` (every
/// string leaf and object key, within the budgets above), and
/// `outcome.human_correction`.
///
/// Two-pass and atomic: every `adapter.redact_text` call is awaited and
/// collected in the first pass, so any adapter error is returned before any
/// field of `envelope` is mutated. The second pass applies the collected
/// text replacements and metadata updates without further awaits.
///
/// A HIGH prior risk can be downgraded here ONLY when the reassessment is
/// complete and convincing: every field was covered within budget, the
/// classifier's result was evidence-backed (see the zero-finding canary
/// check below), and a fresh residual scan came back clean. Any gap in that
/// chain preserves the prior risk instead - see [`resolve_post_scrub_risk`].
/// Free-text fields that carry contributor- or model-authored prose but that
/// [`rescrub_envelope_prose_pii_with`] never submits to the classifier. The
/// scrub pass covers only `events[*].redacted_content`,
/// `events[*].structured_payload`, and `outcome.human_correction`; everything
/// listed here is untouched by it, and the deterministic residual scan that
/// follows only matches patterned secrets, not prose PII such as names or
/// addresses.
///
/// So when any of these carries text, the reassessment did NOT see the whole
/// envelope and must not claim complete coverage - a High prior risk stays
/// High. Envelopes that leave these empty (the common case) are unaffected.
///
/// Only user-derived prose counts. Identifier-shaped strings are excluded
/// (versions, enum tags, retention policies, revocation handles, tool
/// categories, pseudonymous contributor refs), and so is system-authored
/// text: `value.explanation`, `value_card.limitations`, and
/// `value_card.user_visible_explanation` are written by the scorer, not by
/// the contributor. `TraceValueCard::default()` in particular ships a fixed
/// boilerplate `limitations` entry on every envelope, so counting it would
/// block every downgrade and make the reassessment dead code. If the scorer
/// ever begins quoting trace content into those fields, they belong here.
///
/// This list is explicit rather than derived, so a newly added prose field
/// will not be accounted for until it is added here or covered by the scrub.
#[cfg(feature = "near-ai-privacy-filter")]
fn uncovered_prose_present(envelope: &TraceContributionEnvelope) -> bool {
    fn has_text(values: &[String]) -> bool {
        values.iter().any(|v| !v.trim().is_empty())
    }

    if has_text(&envelope.replay.replay_notes) {
        return true;
    }

    if let Some(hindsight) = envelope.hindsight.as_ref() {
        if hindsight
            .original_goal_summary
            .as_deref()
            .is_some_and(|v| !v.trim().is_empty())
            || has_text(&hindsight.achieved_subgoals)
        {
            return true;
        }
    }

    false
}

#[cfg(feature = "near-ai-privacy-filter")]
pub async fn rescrub_envelope_prose_pii_with(
    adapter: &dyn PrivacyFilterAdapter,
    envelope: &mut TraceContributionEnvelope,
) -> Result<(), TraceContributionError> {
    // Same concordance floor as the sync server re-scrub: under-reported
    // consent must not survive into residual_risk / status decisions.
    reconcile_consent_declarations(envelope);

    let mut event_updates: Vec<(usize, String)> = Vec::new();
    let mut correction_update: Option<String> = None;
    let mut structured_updates: Vec<(usize, Value)> = Vec::new();
    let mut report = RedactionReport::default();
    let mut summary: Option<SafePrivacyFilterSummary> = None;
    let mut structured_complete = true;

    for (index, event) in envelope.events.iter().enumerate() {
        let Some(content) = event.redacted_content.as_deref() else {
            continue;
        };
        if let Some(redaction) = adapter.redact_text(content).await? {
            merge_privacy_filter_summary(&mut summary, &redaction.summary);
            report.merge(redaction.report);
            event_updates.push((index, redaction.redacted_text));
        }
    }

    if let Some(correction) = envelope.outcome.human_correction.as_deref() {
        if let Some(redaction) = adapter.redact_text(correction).await? {
            merge_privacy_filter_summary(&mut summary, &redaction.summary);
            report.merge(redaction.report);
            correction_update = Some(redaction.redacted_text);
        }
    }

    // Structured tool payloads (Task 3): classify every string leaf and
    // object key of each event's `structured_payload`, working on a clone
    // so a mid-traversal adapter error (propagated via `?`) leaves the real
    // envelope untouched, matching the atomicity of the prose passes above.
    {
        let mut budget = StructuredPayloadBudget::default();
        for (index, event) in envelope.events.iter().enumerate() {
            if event.structured_payload.is_null() {
                continue;
            }
            let mut clone = event.structured_payload.clone();
            let covered =
                classify_structured_payload_node(adapter, &mut clone, 0, &mut budget, &mut report)
                    .await?;
            structured_updates.push((index, clone));
            if !covered {
                structured_complete = false;
                break;
            }
        }
    }

    // Second pass: no awaits below this line, so the updates collected
    // above are applied atomically.
    for (index, redacted_text) in event_updates {
        envelope.events[index].redacted_content = Some(redacted_text);
    }
    if let Some(redacted_text) = correction_update {
        envelope.outcome.human_correction = Some(redacted_text);
    }
    for (index, redacted_payload) in structured_updates {
        envelope.events[index].structured_payload = redacted_payload;
    }

    for (label, count) in &report.counts {
        *envelope
            .privacy
            .redaction_counts
            .entry(label.clone())
            .or_insert(0) += count;
    }
    for label in &report.pii_labels_present {
        if !envelope.privacy.pii_labels_present.contains(label) {
            envelope.privacy.pii_labels_present.push(label.clone());
        }
    }
    if let Some(summary) = &summary {
        merge_privacy_filter_summary(&mut envelope.privacy.privacy_filter_summary, summary);
    }

    // Zero-finding responses are not automatically trustworthy (Task 2): a
    // 200 with an empty span list is indistinguishable, on its own, from an
    // unavailable, misconfigured, or systematically false-negative
    // classifier. When this pass found *nothing at all* across every field
    // it covered, demand a fresh, live canary round-trip as explicit
    // evidence the classifier is actually working before trusting that
    // emptiness. Any real finding is itself sufficient evidence the
    // classifier ran for real, so the canary is skipped in that case.
    let aggregate_empty = report.counts.is_empty()
        && report.pii_labels_present.is_empty()
        && !report.blocked_secret_detected
        && !report.key_finding_detected;
    // A healthy canary is a liveness signal, not evidence about arbitrary
    // content: the canary is three static, public constants, so a classifier
    // can recognise exactly those and miss everything real. This file's own
    // `CanaryHealthyButFindsNoRealPii` fixture is that classifier, which is
    // how we know the bypass is constructible. Findings are the only
    // evidence: a classifier that found and removed PII demonstrably ran,
    // which permits High -> Medium; one that returned nothing cannot be
    // told apart from a broken one, so High stays High.
    let useful_classifier_result = !aggregate_empty;

    // Residual scan (Task 1/4): re-run the deterministic detection-only
    // scan after the classifier's mutations, exactly as the sync server
    // rescrub does. `bare()` is infallible and reads no env, since this
    // scan only ever calls the plain `redact_text` and never touches an
    // attached privacy-filter adapter.
    let redactor = DeterministicTraceRedactor::bare();
    let residual = residual_envelope_scan(&redactor, envelope).map_err(|_| {
        TraceContributionError::RedactionFailed {
            reason: "residual envelope scan failed after PII backstop pass".to_string(),
        }
    });

    let prior_risk = envelope.privacy.residual_pii_risk;
    envelope.privacy.residual_pii_risk = match residual {
        Ok(residual_findings) => {
            let backstop_pass_risk = residual_risk(&envelope.consent, &report);
            let assessment = PostScrubAssessment {
                complete_coverage: structured_complete && !uncovered_prose_present(envelope),
                useful_classifier_result,
                findings: report.clone(),
                residual_findings,
            };
            resolve_post_scrub_risk(prior_risk, backstop_pass_risk, &assessment)
        }
        Err(_) => {
            // Fail closed: could not verify the envelope is clean after
            // this pass, so never trust it enough to downgrade or even
            // hold steady on an empty report. Force the worst case.
            ResidualPiiRisk::High
        }
    };
    if !envelope
        .privacy
        .redaction_pipeline_version
        .contains(NEAR_AI_PII_BACKSTOP_PIPELINE_SUFFIX)
    {
        envelope.privacy.redaction_pipeline_version.push('+');
        envelope
            .privacy
            .redaction_pipeline_version
            .push_str(NEAR_AI_PII_BACKSTOP_PIPELINE_SUFFIX);
    }
    envelope.trace_card.redaction_pipeline_version =
        envelope.privacy.redaction_pipeline_version.clone();
    merge_privacy_warnings(
        &mut envelope.privacy.warnings,
        privacy_warnings(envelope.privacy.residual_pii_risk),
    );
    envelope.privacy.redaction_hash =
        redaction_hash(&envelope.events, &envelope.privacy.redaction_counts);

    Ok(())
}

/// Redact one owned string in place, folding its report into `report`.
fn redact_string_in_place(
    redactor: &DeterministicTraceRedactor,
    value: &mut String,
    report: &mut RedactionReport,
    state: &mut RedactionState,
) {
    let (redacted, child_report) = redactor.redact_text_with_state(value, state);
    report.merge(child_report);
    *value = redacted;
}

fn redact_strings_in_place(
    redactor: &DeterministicTraceRedactor,
    values: &mut [String],
    report: &mut RedactionReport,
    state: &mut RedactionState,
) {
    for value in values {
        redact_string_in_place(redactor, value, report, state);
    }
}

/// Redact the content-bearing fields outside the three surfaces the
/// original pass covered (`event.redacted_content`,
/// `event.structured_payload`, `outcome.human_correction`).
///
/// Everything here is attacker-controlled free text that reached
/// accepted storage unscrubbed. Map *keys* are rewritten as well as
/// values: a key is just as free-form as the string beside it, and no
/// typed traversal reaches keys by default.
///
/// Structural fields are deliberately left alone - ids, hashes,
/// versions, enum discriminants and revocation handles are server- or
/// schema-controlled, and rewriting them would break lookups.
fn redact_envelope_side_channels(
    redactor: &DeterministicTraceRedactor,
    envelope: &mut TraceContributionEnvelope,
    report: &mut RedactionReport,
    state: &mut RedactionState,
) {
    if !envelope.ironclaw.feature_flags.is_empty() {
        let flags = std::mem::take(&mut envelope.ironclaw.feature_flags);
        for (key, mut value) in flags {
            let mut key = key;
            redact_string_in_place(redactor, &mut key, report, state);
            redact_string_in_place(redactor, &mut value, report, state);
            insert_without_collision(&mut envelope.ironclaw.feature_flags, key, value);
        }
    }
    if let Some(model_name) = envelope.ironclaw.model_name.as_mut() {
        redact_string_in_place(redactor, model_name, report, state);
    }

    for event in &mut envelope.events {
        if let Some(tool_name) = event.tool_name.as_mut() {
            redact_string_in_place(redactor, tool_name, report, state);
        }
    }

    redact_strings_in_place(
        redactor,
        &mut envelope.outcome.error_taxonomy,
        report,
        state,
    );
    for failure_mode in &mut envelope.outcome.failure_modes {
        if let TraceFailureMode::Other(detail) = failure_mode {
            redact_string_in_place(redactor, detail, report, state);
        }
    }

    redact_strings_in_place(redactor, &mut envelope.replay.required_tools, report, state);
    redact_strings_in_place(redactor, &mut envelope.replay.replay_notes, report, state);
    if !envelope.replay.tool_manifest_hashes.is_empty() {
        let manifest = std::mem::take(&mut envelope.replay.tool_manifest_hashes);
        for (key, value) in manifest {
            let mut key = key;
            redact_string_in_place(redactor, &mut key, report, state);
            insert_without_collision(&mut envelope.replay.tool_manifest_hashes, key, value);
        }
    }
    for assertion in &mut envelope.replay.expected_assertions {
        let (redacted, child_report) = redactor.redact_json_value(None, assertion, state);
        report.merge(child_report);
        *assertion = redacted;
    }

    if let Some(embedding) = envelope.embedding_analysis.as_mut() {
        redact_strings_in_place(redactor, &mut embedding.coverage_tags, report, state);
    }

    redact_strings_in_place(
        redactor,
        &mut envelope.trace_card.tool_categories,
        report,
        state,
    );
    redact_strings_in_place(
        redactor,
        &mut envelope.value_card.limitations,
        report,
        state,
    );
    redact_strings_in_place(
        redactor,
        &mut envelope.value_card.user_visible_explanation,
        report,
        state,
    );

    if let Some(hindsight) = envelope.hindsight.as_mut() {
        if let Some(summary) = hindsight.original_goal_summary.as_mut() {
            redact_string_in_place(redactor, summary, report, state);
        }
        redact_strings_in_place(redactor, &mut hindsight.achieved_subgoals, report, state);
        if let Some(TraceFailureMode::Other(detail)) = hindsight.failure_type.as_mut() {
            redact_string_in_place(redactor, detail, report, state);
        }
    }

    if let Some(process_evaluation) = envelope.process_evaluation.as_mut() {
        if let Some(evaluator_name) = process_evaluation.evaluator_name.as_mut() {
            redact_string_in_place(redactor, evaluator_name, report, state);
        }
    }
}

/// Object keys never collide as long as `map` did not already contain the
/// candidate key: rewriting a key (e.g. through deterministic redaction)
/// can produce the same placeholder for two originally-distinct keys, and a
/// plain `insert` would silently let the second overwrite the first. This
/// disambiguates instead of losing data.
fn insert_without_collision(map: &mut BTreeMap<String, String>, key: String, value: String) {
    if let std::collections::btree_map::Entry::Vacant(entry) = map.entry(key.clone()) {
        entry.insert(value);
        return;
    }
    let mut suffix = 1u32;
    loop {
        let candidate = format!("{key}~dup{suffix}");
        if let std::collections::btree_map::Entry::Vacant(entry) = map.entry(candidate) {
            entry.insert(value);
            return;
        }
        suffix += 1;
    }
}

/// Node budget for [`residual_envelope_scan`]. The scan clones and
/// re-serializes the whole envelope, so it needs its own explicit bound
/// independent of whatever recursion limit `serde_json` enforces on the way
/// in - relying on that limit implicitly was itself one of the findings
/// this budget closes.
const RESIDUAL_SCAN_MAX_DEPTH: usize = 64;
const RESIDUAL_SCAN_MAX_NODES: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResidualScanError {
    SerializationFailed,
    BudgetExceeded,
}

/// Detection-only scan of the whole envelope after redaction has run.
///
/// Each string is scanned *independently* - object keys separately from
/// their values, one leaf at a time - rather than by scanning the
/// serialized JSON text. That matters: contextual-entropy detection
/// looks backwards a fixed window for a secret-shaped cue, so scanning
/// concatenated JSON would let a key like `"vector_key"` act as the cue
/// for the hash sitting next to it and flag every envelope. Per-leaf
/// scanning removes that adjacency entirely.
///
/// Returns `Err` on a serialization failure or a budget overrun rather than
/// silently reporting "nothing found" - an envelope this scan cannot
/// actually cover must never be treated as verified clean.
fn residual_envelope_scan(
    redactor: &DeterministicTraceRedactor,
    envelope: &TraceContributionEnvelope,
) -> Result<RedactionReport, ResidualScanError> {
    let mut report = RedactionReport::default();
    let serialized =
        serde_json::to_value(envelope).map_err(|_| ResidualScanError::SerializationFailed)?;
    let mut nodes = 0usize;
    scan_json_leaves(redactor, &serialized, &mut report, 0, &mut nodes)?;
    Ok(report)
}

fn scan_json_leaves(
    redactor: &DeterministicTraceRedactor,
    value: &Value,
    report: &mut RedactionReport,
    depth: usize,
    nodes: &mut usize,
) -> Result<(), ResidualScanError> {
    if depth > RESIDUAL_SCAN_MAX_DEPTH {
        return Err(ResidualScanError::BudgetExceeded);
    }
    match value {
        Value::String(text) => {
            *nodes += 1;
            if *nodes > RESIDUAL_SCAN_MAX_NODES {
                return Err(ResidualScanError::BudgetExceeded);
            }
            let (_, child_report) = redactor.redact_text(text);
            report.merge(child_report);
        }
        Value::Array(items) => {
            for item in items {
                scan_json_leaves(redactor, item, report, depth + 1, nodes)?;
            }
        }
        Value::Object(entries) => {
            for (key, item) in entries {
                *nodes += 1;
                if *nodes > RESIDUAL_SCAN_MAX_NODES {
                    return Err(ResidualScanError::BudgetExceeded);
                }
                let (_, child_report) = redactor.redact_text(key);
                report.merge(child_report);
                scan_json_leaves(redactor, item, report, depth + 1, nodes)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// The result of attempting a complete, evidence-backed reassessment of an
/// envelope's residual PII risk after a scrub pass.
///
/// Downgrade from the prior risk is permitted ONLY when every field here
/// demonstrates it is safe: `complete_coverage` (every content-bearing
/// field was processed, no budget was exceeded, no field was skipped),
/// `useful_classifier_result` (the pass actually produced usable evidence,
/// not just an unconvincing empty result), and a clean, successfully-run
/// residual scan (`residual_findings` empty). Any ambiguity must leave this
/// assessment unable to downgrade; [`resolve_post_scrub_risk`] then falls
/// back to the old, safe, max-with-prior combination.
#[derive(Debug, Clone)]
struct PostScrubAssessment {
    complete_coverage: bool,
    useful_classifier_result: bool,
    findings: RedactionReport,
    residual_findings: RedactionReport,
}

impl PostScrubAssessment {
    fn residual_clean(&self) -> bool {
        self.residual_findings.counts.is_empty()
            && self.residual_findings.pii_labels_present.is_empty()
            && !self.residual_findings.blocked_secret_detected
            && !self.residual_findings.key_finding_detected
            && !self.residual_findings.coverage_incomplete
    }

    fn can_downgrade(&self) -> bool {
        self.complete_coverage && self.useful_classifier_result && self.residual_clean()
    }
}

/// Combine a prior residual-PII risk with a post-scrub assessment's derived
/// risk. Downgrade only when the assessment proves it is safe to do so;
/// otherwise the prior risk is preserved (never lowered) via
/// `max_residual_risk`, exactly as before this pass existed.
///
/// The forced-High set below is deliberately asymmetric between `findings`
/// (what this pass detected and then redacted) and `residual_findings` (what
/// the detection-only scan saw in the envelope *after* the pass finished).
/// A secret in `findings` is gone; a secret in `residual_findings` is still
/// there. Forcing High on `findings.blocked_secret_detected` was the
/// server-side half of issue #373 and pinned every scrubbed envelope at
/// High. `key_finding_detected` stays in the set on both sides, because keys
/// are detected but never rewritten, and so a key finding is present in the
/// envelope no matter which pass reported it.
fn resolve_post_scrub_risk(
    prior_risk: ResidualPiiRisk,
    derived_risk: ResidualPiiRisk,
    assessment: &PostScrubAssessment,
) -> ResidualPiiRisk {
    if assessment.findings.key_finding_detected
        || assessment.findings.coverage_incomplete
        || assessment.residual_findings.blocked_secret_detected
        || assessment.residual_findings.key_finding_detected
        || assessment.residual_findings.coverage_incomplete
    {
        return ResidualPiiRisk::High;
    }
    if assessment.can_downgrade() {
        derived_risk
    } else {
        max_residual_risk(prior_risk, derived_risk)
    }
}

/// Classify what a redaction pass leaves behind.
///
/// "Residual" means what is still in the envelope after the pass, not what
/// the pass had to work on. High is reserved for conditions redaction did
/// not resolve:
///
/// 1. Secrets found and removed, nothing left over -> the scrubber working.
///    Medium (the found-and-removed floor), never High. Before issue #373
///    this returned High on `blocked_secret_detected`, which is set at
///    DETECTION time immediately before the span is redacted, so every real
///    coding session was High and Medium was unreachable.
/// 2. Something survived -> High. Survivors are not visible in this report,
///    which describes one pass's own findings; they are reported by
///    [`residual_envelope_scan`], a detection-only pass over the finished
///    envelope, and reach the classification through
///    [`resolve_post_scrub_risk`]'s `residual_findings`. A residual scan that
///    could not be run at all forces High at that call site.
/// 3. A key finding -> High. `key_finding_detected` marks an object key
///    flagged as PII-bearing, and keys are never rewritten (a rewrite can
///    collide with a sibling key and silently drop data), so the finding is
///    still present in the envelope. Redaction structurally cannot resolve
///    it, and no consent flag or clean scan can talk it down.
/// 4. Coverage gaps -> High. A configured filter that was unavailable,
///    errored, or skipped content leaves text nothing examined; an empty
///    report from a broken filter is not evidence of cleanliness.
fn residual_risk(consent: &ConsentMetadata, report: &RedactionReport) -> ResidualPiiRisk {
    // Case 3: not resolvable by redaction, so it is genuinely residual.
    if report.key_finding_detected {
        return ResidualPiiRisk::High;
    }

    // Case 4: fail closed. The pass cannot vouch for what it never saw.
    if report.coverage_incomplete {
        return ResidualPiiRisk::High;
    }

    // Case 1: PII the pass actually found and removed raises the floor to
    // Medium regardless of what the consent flags claim. A contributor
    // who under-reports risk should not be able to land in accepted
    // storage with a Low classification just because the flags are
    // clean; the pass has direct evidence the flags are wrong.
    if report.blocked_secret_detected
        || !report.counts.is_empty()
        || !report.pii_labels_present.is_empty()
    {
        return ResidualPiiRisk::Medium;
    }

    if consent.message_text_included || consent.tool_payloads_included {
        return ResidualPiiRisk::Medium;
    }

    ResidualPiiRisk::Low
}

/// What content-bearing surfaces an envelope actually carries.
///
/// Used to check `ConsentMetadata::{message_text_included,tool_payloads_included}`
/// against the payload those flags claim to describe. The flags are a factual
/// declaration (`docs/trace-spec.md`), not a client preference.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EnvelopeContentPresence {
    pub message_text: bool,
    pub tool_payloads: bool,
}

/// Inspect an envelope for content that must be declared in consent flags.
///
/// Minimum concordance rule (issue #208):
/// - non-empty `redacted_content` on user / assistant / reasoning (and other
///   prose event types), or `outcome.human_correction`, implies message text;
/// - tool-call / tool-result / http content, or a non-null `structured_payload`,
///   implies tool payloads.
///
/// A bare `tool_name` deliberately does NOT imply tool payloads. The name is
/// metadata about which tool ran, not the payload the flag declares, and
/// stripping payloads while keeping names is a supported privacy mode -- it is
/// what keeps a trace structurally trainable when content is absent. Counting
/// the name would correct every structure-preserved trace upward to
/// `tool_payloads_included = true`, push it to Medium residual risk, and
/// quarantine it on a default deployment for payloads it does not carry.
///
/// This matches the client-side derivation in the contributor crate, which
/// makes the same call. The two halves must agree: if the client declares
/// honestly and the server then corrects that declaration upward anyway, the
/// contributor is penalised for telling the truth.
///
/// Does not mutate the envelope. Callers that need enforcement should use
/// [`reconcile_consent_declarations`], which only corrects flags upward.
pub fn derive_envelope_content_presence(
    envelope: &TraceContributionEnvelope,
) -> EnvelopeContentPresence {
    let mut presence = EnvelopeContentPresence::default();

    for event in &envelope.events {
        if event
            .redacted_content
            .as_ref()
            .is_some_and(|content| !content.is_empty())
        {
            match event.event_type {
                TraceContributionEventType::UserMessage
                | TraceContributionEventType::AssistantMessage
                | TraceContributionEventType::Reasoning
                | TraceContributionEventType::RoutingDecision
                | TraceContributionEventType::Feedback => presence.message_text = true,
                TraceContributionEventType::ToolCall
                | TraceContributionEventType::ToolResult
                | TraceContributionEventType::HttpExchange => presence.tool_payloads = true,
            }
        }
        if !event.structured_payload.is_null() {
            presence.tool_payloads = true;
        }
    }

    if envelope
        .outcome
        .human_correction
        .as_ref()
        .is_some_and(|text| !text.is_empty())
    {
        presence.message_text = true;
    }

    presence
}

/// Correct under-reported consent declarations to match the envelope payload.
///
/// Only moves flags from `false` → `true`. Over-reporting (true flags on an
/// empty payload) is left alone: that is a stricter declaration and does not
/// open an acceptance path the payload did not earn.
///
/// Returns the presence that was derived, so callers can log or assert.
pub fn reconcile_consent_declarations(
    envelope: &mut TraceContributionEnvelope,
) -> EnvelopeContentPresence {
    let presence = derive_envelope_content_presence(envelope);
    let mut corrected = false;
    if presence.message_text && !envelope.consent.message_text_included {
        envelope.consent.message_text_included = true;
        corrected = true;
    }
    if presence.tool_payloads && !envelope.consent.tool_payloads_included {
        envelope.consent.tool_payloads_included = true;
        corrected = true;
    }
    if corrected {
        let warning =
            "Server corrected under-reported consent declarations to match envelope payload."
                .to_string();
        if !envelope.privacy.warnings.contains(&warning) {
            envelope.privacy.warnings.push(warning);
        }
    }
    presence
}

fn max_residual_risk(left: ResidualPiiRisk, right: ResidualPiiRisk) -> ResidualPiiRisk {
    use ResidualPiiRisk::{High, Low, Medium};
    match (left, right) {
        (High, _) | (_, High) => High,
        (Medium, _) | (_, Medium) => Medium,
        (Low, Low) => Low,
    }
}

fn merge_privacy_warnings(existing: &mut Vec<String>, new_warnings: Vec<String>) {
    for warning in new_warnings {
        if !existing.contains(&warning) {
            existing.push(warning);
        }
    }
}

fn privacy_warnings(risk: ResidualPiiRisk) -> Vec<String> {
    match risk {
        ResidualPiiRisk::Low => Vec::new(),
        ResidualPiiRisk::Medium => vec![
            "Message text or tool payloads were included after local redaction; server-side re-scrub is still required.".to_string(),
        ],
        ResidualPiiRisk::High => vec![
            "Secret-like content was detected after deterministic scrubbing; keep this trace quarantined until reviewed.".to_string(),
        ],
    }
}

fn build_trace_card(
    consent_scopes: &[ConsentScope],
    channel: TraceChannel,
    revocation_handle: Uuid,
    events: &[TraceContributionEvent],
) -> TraceCard {
    let consent_scope = consent_scopes
        .first()
        .copied()
        .unwrap_or(ConsentScope::DebuggingEvaluation);
    let tool_categories = events
        .iter()
        .filter_map(|event| event.tool_category.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    TraceCard {
        consent_scope,
        redaction_pipeline_version: DETERMINISTIC_REDACTION_PIPELINE_VERSION.to_string(),
        source_channel: channel_label(channel).to_string(),
        tool_categories,
        allowed_uses: allowed_uses_for_scopes(consent_scopes),
        retention_policy: "private_corpus_revocable".to_string(),
        revocation_handle: revocation_handle.to_string(),
    }
}

fn allowed_uses_for_scopes(scopes: &[ConsentScope]) -> Vec<TraceAllowedUse> {
    if scopes.is_empty() {
        return default_allowed_uses_for_scope(ConsentScope::DebuggingEvaluation);
    }

    scopes
        .iter()
        .flat_map(|scope| default_allowed_uses_for_scope(*scope))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn default_allowed_uses_for_scope(scope: ConsentScope) -> Vec<TraceAllowedUse> {
    match scope {
        ConsentScope::DebuggingEvaluation => vec![
            TraceAllowedUse::Debugging,
            TraceAllowedUse::Evaluation,
            TraceAllowedUse::AggregateAnalytics,
        ],
        ConsentScope::BenchmarkOnly => vec![
            TraceAllowedUse::Evaluation,
            TraceAllowedUse::BenchmarkGeneration,
            TraceAllowedUse::AggregateAnalytics,
        ],
        ConsentScope::RankingTraining => vec![
            TraceAllowedUse::Debugging,
            TraceAllowedUse::Evaluation,
            TraceAllowedUse::RankingModelTraining,
            TraceAllowedUse::AggregateAnalytics,
        ],
        ConsentScope::ModelTraining => vec![
            TraceAllowedUse::Debugging,
            TraceAllowedUse::Evaluation,
            TraceAllowedUse::RankingModelTraining,
            TraceAllowedUse::ModelTraining,
            TraceAllowedUse::AggregateAnalytics,
        ],
        // public_attribution is a profile-management consent, not a
        // trace-content use. It grants the contributor's pseudonym to
        // be linked to a publicly-visible handle via the community
        // surface; it does NOT permit any new operations on envelope
        // bodies. Returning empty here means a claim scoped to only
        // public_attribution cannot submit traces (the envelope
        // validation refuses an empty allowed_uses list).
        ConsentScope::PublicAttribution => Vec::new(),
    }
}

pub fn retention_policy_for_allowed_use(allowed_use: TraceAllowedUse) -> TraceRetentionPolicy {
    match allowed_use {
        TraceAllowedUse::Debugging | TraceAllowedUse::Evaluation => TraceRetentionPolicy {
            name: "private_corpus_revocable".to_string(),
            class: TraceRetentionClass::PrivateCorpusRevocable,
            revocable: true,
            max_age_days: Some(730),
            allows_derived_artifacts: true,
        },
        TraceAllowedUse::BenchmarkGeneration => TraceRetentionPolicy {
            name: "benchmark_revocable".to_string(),
            class: TraceRetentionClass::BenchmarkRevocable,
            revocable: true,
            max_age_days: Some(1095),
            allows_derived_artifacts: true,
        },
        TraceAllowedUse::RankingModelTraining | TraceAllowedUse::ModelTraining => {
            TraceRetentionPolicy {
                name: "training_revocable".to_string(),
                class: TraceRetentionClass::TrainingRevocable,
                revocable: true,
                max_age_days: Some(1095),
                allows_derived_artifacts: true,
            }
        }
        TraceAllowedUse::AggregateAnalytics => TraceRetentionPolicy {
            name: "aggregate_only".to_string(),
            class: TraceRetentionClass::AggregateOnly,
            revocable: false,
            max_age_days: None,
            allows_derived_artifacts: false,
        },
    }
}

pub fn retention_policy_for_trace(envelope: &TraceContributionEnvelope) -> TraceRetentionPolicy {
    let strongest = envelope
        .trace_card
        .allowed_uses
        .iter()
        .copied()
        .max_by_key(|allowed_use| match allowed_use {
            TraceAllowedUse::ModelTraining => 5,
            TraceAllowedUse::RankingModelTraining => 4,
            TraceAllowedUse::BenchmarkGeneration => 3,
            TraceAllowedUse::Evaluation => 2,
            TraceAllowedUse::Debugging => 1,
            TraceAllowedUse::AggregateAnalytics => 0,
        })
        .unwrap_or(TraceAllowedUse::Debugging);
    let mut policy = retention_policy_for_allowed_use(strongest);
    if !envelope.consent.revocable {
        policy.revocable = false;
    }
    policy
}

pub fn derived_artifact_invalidation_marker(
    envelope: &TraceContributionEnvelope,
    reason: impl Into<String>,
) -> DerivedArtifactInvalidationMarker {
    DerivedArtifactInvalidationMarker {
        schema_version: "ironclaw.trace_derived_artifact_invalidation.v1".to_string(),
        submission_id: envelope.submission_id,
        trace_id: envelope.trace_id,
        revocation_handle_hash: canonical_hash(&envelope.contributor.revocation_handle.to_string()),
        redaction_hash: envelope.privacy.redaction_hash.clone(),
        artifact_prefixes: derived_artifact_prefixes(envelope),
        reason: reason.into(),
        created_at: Utc::now(),
    }
}

pub fn derived_artifact_prefixes(envelope: &TraceContributionEnvelope) -> Vec<String> {
    let trace_id = envelope.trace_id;
    let submission_id = envelope.submission_id;
    vec![
        format!("trace:{trace_id}"),
        format!("submission:{submission_id}"),
        format!("summary:{trace_id}"),
        format!("embedding:{trace_id}"),
        format!("benchmark:{trace_id}"),
        format!("training_example:{trace_id}"),
    ]
}

pub fn trace_dataset_eligibility(
    envelope: &TraceContributionEnvelope,
    requested_use: TraceAllowedUse,
    revoked: bool,
) -> TraceDatasetEligibility {
    let retention_policy = retention_policy_for_allowed_use(requested_use);
    let mut reasons = Vec::new();

    if revoked {
        reasons.push("submission has been revoked".to_string());
    }
    if !envelope.trace_card.allowed_uses.contains(&requested_use) {
        reasons.push("requested use is outside consent scope".to_string());
    }
    if !envelope.consent.revocable && retention_policy.revocable {
        reasons.push("trace consent is not revocable for a revocable dataset class".to_string());
    }
    match envelope.privacy.residual_pii_risk {
        ResidualPiiRisk::Low => {}
        ResidualPiiRisk::Medium => {
            if matches!(
                requested_use,
                TraceAllowedUse::BenchmarkGeneration
                    | TraceAllowedUse::RankingModelTraining
                    | TraceAllowedUse::ModelTraining
            ) {
                reasons.push(
                    "medium residual privacy risk is limited to debugging, evaluation, or aggregate analytics"
                        .to_string(),
                );
            }
        }
        ResidualPiiRisk::High => {
            reasons.push("high residual privacy risk is not dataset eligible".to_string());
        }
    }
    if envelope
        .privacy
        .warnings
        .iter()
        .any(|warning| warning.to_ascii_lowercase().contains("quarantined"))
    {
        reasons.push("trace is quarantined by privacy warning".to_string());
    }

    TraceDatasetEligibility {
        eligible: reasons.is_empty(),
        requested_use,
        retention_policy,
        reasons,
    }
}

fn channel_label(channel: TraceChannel) -> &'static str {
    match channel {
        TraceChannel::Web => "web",
        TraceChannel::Cli => "cli",
        TraceChannel::Telegram => "telegram",
        TraceChannel::Slack => "slack",
        TraceChannel::Routine => "routine",
        TraceChannel::Other => "other",
    }
}

fn tool_category_for(tool_name: &str) -> String {
    let lower = tool_name.to_ascii_lowercase();
    if lower.contains("http") || lower.contains("browser") || lower.contains("web") {
        "network".to_string()
    } else if lower.contains("file")
        || lower.contains("fs")
        || lower.contains("workspace")
        || lower.contains("shell")
        || lower.contains("exec")
    {
        "workspace".to_string()
    } else if lower.contains("memory") || lower.contains("search") {
        "retrieval".to_string()
    } else if lower.contains("calendar") || lower.contains("email") || lower.contains("slack") {
        "external_app".to_string()
    } else {
        "other".to_string()
    }
}

fn side_effect_for(
    event_type: TraceContributionEventType,
    tool_name: Option<&str>,
) -> SideEffectLevel {
    match event_type {
        TraceContributionEventType::UserMessage
        | TraceContributionEventType::AssistantMessage
        | TraceContributionEventType::Reasoning
        | TraceContributionEventType::Feedback => SideEffectLevel::None,
        TraceContributionEventType::RoutingDecision => SideEffectLevel::None,
        TraceContributionEventType::ToolResult => SideEffectLevel::None,
        TraceContributionEventType::HttpExchange => SideEffectLevel::ReadOnly,
        TraceContributionEventType::ToolCall => tool_name
            .map(classify_tool_side_effect)
            .unwrap_or(SideEffectLevel::Unknown),
    }
}

fn classify_tool_side_effect(tool_name: &str) -> SideEffectLevel {
    let lower = tool_name.to_ascii_lowercase();
    if lower.contains("write")
        || lower.contains("create")
        || lower.contains("delete")
        || lower.contains("send")
        || lower.contains("post")
    {
        if lower.contains("email") || lower.contains("calendar") || lower.contains("slack") {
            SideEffectLevel::ExternalWrite
        } else {
            SideEffectLevel::LocalWrite
        }
    } else if lower.contains("auth") || lower.contains("credential") || lower.contains("token") {
        SideEffectLevel::CredentialUse
    } else {
        SideEffectLevel::ReadOnly
    }
}

pub fn canonical_summary_for_embedding(envelope: &TraceContributionEnvelope) -> String {
    canonical_whole_trace_representation(envelope)
}

pub fn canonical_representations_for_embedding(
    envelope: &TraceContributionEnvelope,
) -> Vec<CanonicalTraceRepresentation> {
    let mut representations = Vec::new();
    push_canonical_representation(
        &mut representations,
        envelope,
        CanonicalRepresentationKind::WholeTrace,
        0,
        canonical_whole_trace_representation(envelope),
    );

    for (index, content) in canonical_turn_representations(envelope)
        .into_iter()
        .enumerate()
    {
        push_canonical_representation(
            &mut representations,
            envelope,
            CanonicalRepresentationKind::Turn,
            index,
            content,
        );
    }

    let tool_sequence = canonical_tool_sequence_representation(envelope);
    if !tool_sequence.is_empty() {
        push_canonical_representation(
            &mut representations,
            envelope,
            CanonicalRepresentationKind::ToolSequence,
            0,
            tool_sequence,
        );
    }

    let error_outcome = canonical_error_outcome_representation(envelope);
    if !error_outcome.is_empty() {
        push_canonical_representation(
            &mut representations,
            envelope,
            CanonicalRepresentationKind::ErrorOutcome,
            0,
            error_outcome,
        );
    }

    if let Some(correction) = canonical_correction_representation(envelope) {
        push_canonical_representation(
            &mut representations,
            envelope,
            CanonicalRepresentationKind::Correction,
            0,
            correction,
        );
    }

    representations
}

fn push_canonical_representation(
    representations: &mut Vec<CanonicalTraceRepresentation>,
    envelope: &TraceContributionEnvelope,
    kind: CanonicalRepresentationKind,
    index: usize,
    content: String,
) {
    let canonical_hash = canonical_hash(&content);
    let hash_fragment = canonical_hash
        .strip_prefix("sha256:")
        .unwrap_or(&canonical_hash)
        .chars()
        .take(16)
        .collect::<String>();
    representations.push(CanonicalTraceRepresentation {
        kind,
        vector_key: format!(
            "trace:{}:{:?}:{}:{}",
            envelope.trace_id, kind, index, hash_fragment
        )
        .to_ascii_lowercase(),
        canonical_hash,
        content,
    });
}

fn canonical_whole_trace_representation(envelope: &TraceContributionEnvelope) -> String {
    let mut lines = Vec::new();
    lines.push(format!("Outcome: {:?}", envelope.outcome.task_success));
    if !envelope.replay.required_tools.is_empty() {
        lines.push(format!(
            "Tools used: {}",
            envelope.replay.required_tools.join(", ")
        ));
    }
    let failure_modes = envelope
        .outcome
        .failure_modes
        .iter()
        .map(|mode| format!("{mode:?}"))
        .collect::<Vec<_>>();
    if !failure_modes.is_empty() {
        lines.push(format!("Failure modes: {}", failure_modes.join(", ")));
    }
    lines.push(format!(
        "User correction included: {}",
        envelope.outcome.human_correction.is_some()
    ));
    lines.push("Redacted summary:".to_string());

    for event in envelope.events.iter().take(12) {
        let mut line = format!("  {:?}:", event.event_type);
        if let Some(tool_name) = &event.tool_name {
            line.push_str(&format!(" tool={tool_name}"));
        }
        if let Some(content) = &event.redacted_content {
            line.push(' ');
            line.push_str(content);
        } else if !event.structured_payload.is_null() {
            line.push_str(" payload=");
            line.push_str(&safe_payload_summary(&event.structured_payload));
        }
        lines.push(line);
    }

    lines.join("\n")
}

fn canonical_turn_representations(envelope: &TraceContributionEnvelope) -> Vec<String> {
    let mut turns = Vec::new();
    let mut current = Vec::new();
    let mut turn_index = 0usize;

    for event in &envelope.events {
        if event.event_type == TraceContributionEventType::UserMessage && !current.is_empty() {
            turns.push(canonical_turn_content(turn_index, &current));
            current.clear();
            turn_index += 1;
        }
        current.push(event);
    }
    if !current.is_empty() {
        turns.push(canonical_turn_content(turn_index, &current));
    }

    turns
}

fn canonical_turn_content(turn_index: usize, events: &[&TraceContributionEvent]) -> String {
    let mut lines = vec![format!("Turn: {turn_index}")];
    for event in events {
        lines.push(canonical_event_line(event));
    }
    lines.join("\n")
}

fn canonical_tool_sequence_representation(envelope: &TraceContributionEnvelope) -> String {
    let mut lines = Vec::new();
    for event in envelope
        .events
        .iter()
        .filter(|event| event.event_type == TraceContributionEventType::ToolCall)
    {
        let tool_name = event.tool_name.as_deref().unwrap_or("unknown");
        let category = event.tool_category.as_deref().unwrap_or("unknown");
        lines.push(format!(
            "Tool: name={tool_name} category={category} side_effect={:?} success={}",
            event.side_effect,
            event
                .success
                .map(|success| success.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        ));
    }
    lines.join("\n")
}

fn canonical_error_outcome_representation(envelope: &TraceContributionEnvelope) -> String {
    let has_error_signal = !envelope.outcome.error_taxonomy.is_empty()
        || !envelope.outcome.failure_modes.is_empty()
        || matches!(
            envelope.outcome.task_success,
            TaskSuccess::Failure | TaskSuccess::Partial
        )
        || envelope
            .events
            .iter()
            .any(|event| !event.failure_modes.is_empty() || event.success == Some(false));
    if !has_error_signal {
        return String::new();
    }

    let mut lines = vec![format!("Task success: {:?}", envelope.outcome.task_success)];
    if !envelope.outcome.error_taxonomy.is_empty() {
        lines.push(format!(
            "Error taxonomy: {}",
            envelope.outcome.error_taxonomy.join(", ")
        ));
    }
    if !envelope.outcome.failure_modes.is_empty() {
        lines.push(format!(
            "Outcome failure modes: {}",
            envelope
                .outcome
                .failure_modes
                .iter()
                .map(|mode| format!("{mode:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for event in envelope
        .events
        .iter()
        .filter(|event| !event.failure_modes.is_empty() || event.success == Some(false))
    {
        lines.push(canonical_event_line(event));
    }
    lines.join("\n")
}

fn canonical_correction_representation(envelope: &TraceContributionEnvelope) -> Option<String> {
    let correction = envelope.outcome.human_correction.as_ref()?;
    let mut lines = vec![format!("Correction: {correction}")];
    if !envelope.outcome.failure_modes.is_empty() {
        lines.push(format!(
            "Failure modes: {}",
            envelope
                .outcome
                .failure_modes
                .iter()
                .map(|mode| format!("{mode:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Some(lines.join("\n"))
}

fn canonical_event_line(event: &TraceContributionEvent) -> String {
    let mut line = format!("{:?}:", event.event_type);
    if let Some(tool_name) = &event.tool_name {
        line.push_str(&format!(" tool={tool_name}"));
    }
    if let Some(content) = &event.redacted_content {
        line.push(' ');
        line.push_str(content);
    } else if !event.structured_payload.is_null() {
        line.push_str(" payload=");
        line.push_str(&safe_payload_summary(&event.structured_payload));
    }
    if !event.failure_modes.is_empty() {
        line.push_str(" failure_modes=");
        line.push_str(
            &event
                .failure_modes
                .iter()
                .map(|mode| format!("{mode:?}"))
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    line
}

/// Key names only, never values, at a bounded width.
///
/// A tool call's arguments sit under an `arguments` key, so summarising only
/// the top level would render every call in the corpus as `keys(arguments)`
/// -- identical text for a filesystem read and a calendar write. The argument
/// key names are most of what distinguishes one call from another in a
/// payload-redacted corpus, and this text is what the duplicate and novelty
/// scores are computed over (see #211, where tool-name-only canonical text
/// already collapsed 330 traces into 236 distinct hashes). Descend through
/// the wrapper so those names survive, and keep the wrapper visible so the
/// two shapes cannot be confused.
fn safe_payload_summary(payload: &Value) -> String {
    match payload {
        Value::Object(map) => {
            let keys = map
                .iter()
                .take(8)
                .map(|(key, value)| match value {
                    // One level only. Deeper nesting is not more signal, and
                    // an unbounded walk over an attacker-shaped payload is
                    // not something this runs.
                    Value::Object(inner) if REPLAY_ARGUMENT_KEYS.contains(&key.as_str()) => {
                        let inner_keys =
                            inner.keys().take(8).map(String::as_str).collect::<Vec<_>>();
                        format!("{key}:[{}]", inner_keys.join(","))
                    }
                    _ => key.clone(),
                })
                .collect::<Vec<_>>();
            format!("keys({})", keys.join(","))
        }
        Value::Array(items) => format!("array(len={})", items.len()),
        Value::String(_) => "redacted_string".to_string(),
        Value::Null => "null".to_string(),
        Value::Bool(_) | Value::Number(_) => "scalar".to_string(),
    }
}

fn canonical_hash(content: &str) -> String {
    let digest = Sha256::digest(content.as_bytes());
    format!("sha256:{}", hex::encode(digest))
}

fn redaction_hash(events: &[TraceContributionEvent], counts: &BTreeMap<String, u32>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(events).unwrap_or_default());
    hasher.update(serde_json::to_vec(counts).unwrap_or_default());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

fn redact_tool_specific_payload(
    tool_name: Option<&str>,
    value: &Value,
    report: &mut RedactionReport,
) -> Value {
    let Some(profile) = tool_name.and_then(tool_payload_profile) else {
        return value.clone();
    };
    redact_tool_specific_value(value, profile, None, report)
}

fn redact_tool_specific_value(
    value: &Value,
    profile: ToolPayloadProfile,
    field_name: Option<&str>,
    report: &mut RedactionReport,
) -> Value {
    if let Some(action) = field_name.and_then(|field| tool_redaction_action(profile, field)) {
        report.increment("tool_sensitive_field");
        report.increment(format!("tool_sensitive_field:{}", action.label()));
        report.add_pii_label(action.label());
        return apply_tool_redaction_action(value, action);
    }

    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, child)| {
                    (
                        key.clone(),
                        redact_tool_specific_value(child, profile, Some(key), report),
                    )
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|child| redact_tool_specific_value(child, profile, None, report))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[derive(Debug, Clone, Copy)]
enum ToolPayloadProfile {
    Browser,
    Calendar,
    Database,
    Email,
    Filesystem,
    IssueTracker,
    Messaging,
}

fn tool_payload_profile(tool_name: &str) -> Option<ToolPayloadProfile> {
    let lower = tool_name.to_ascii_lowercase();
    if lower.contains("email") || lower.contains("gmail") {
        Some(ToolPayloadProfile::Email)
    } else if lower.contains("calendar") {
        Some(ToolPayloadProfile::Calendar)
    } else if lower.contains("slack")
        || lower.contains("telegram")
        || lower.contains("signal")
        || lower.contains("discord")
    {
        Some(ToolPayloadProfile::Messaging)
    } else if lower.contains("github")
        || lower.contains("gitlab")
        || lower.contains("linear")
        || lower.contains("issue")
        || lower.contains("pull_request")
        || lower.contains("pr_")
    {
        Some(ToolPayloadProfile::IssueTracker)
    } else if lower.contains("browser")
        || lower.contains("http")
        || lower.contains("fetch")
        || lower.contains("url")
        || lower.contains("web")
    {
        Some(ToolPayloadProfile::Browser)
    } else if lower.contains("sql")
        || lower.contains("db")
        || lower.contains("database")
        || lower.contains("postgres")
        || lower.contains("libsql")
        || lower.contains("mysql")
    {
        Some(ToolPayloadProfile::Database)
    } else if lower.contains("file")
        || lower.contains("fs")
        || lower.contains("workspace")
        || lower.contains("shell")
        || lower.contains("exec")
    {
        Some(ToolPayloadProfile::Filesystem)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy)]
enum ToolRedactionAction {
    Replace(&'static str),
    SanitizeUrl(&'static str),
    RedactObjectValues(&'static str),
    SummarizeCollection(&'static str),
}

impl ToolRedactionAction {
    fn label(self) -> &'static str {
        match self {
            ToolRedactionAction::Replace(label)
            | ToolRedactionAction::SanitizeUrl(label)
            | ToolRedactionAction::RedactObjectValues(label)
            | ToolRedactionAction::SummarizeCollection(label) => label,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ToolSensitiveFieldRule {
    matcher: ToolFieldMatcher,
    action: ToolRedactionAction,
}

#[derive(Debug, Clone, Copy)]
enum ToolFieldMatcher {
    Exact(&'static [&'static str]),
    Contains(&'static [&'static str]),
}

const EMAIL_RULES: &[ToolSensitiveFieldRule] = &[
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "to",
            "cc",
            "bcc",
            "from",
            "reply_to",
            "replyto",
            "recipient",
            "recipients",
            "sender",
        ]),
        action: ToolRedactionAction::SummarizeCollection("email_participant"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "subject", "body", "text", "html", "snippet", "message", "raw", "mime",
        ]),
        action: ToolRedactionAction::Replace("email_content"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&["headers", "header"]),
        action: ToolRedactionAction::RedactObjectValues("email_header"),
    },
];

const CALENDAR_RULES: &[ToolSensitiveFieldRule] = &[
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Contains(&["attendee", "participant", "organizer", "creator"]),
        action: ToolRedactionAction::SummarizeCollection("calendar_participant"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "summary",
            "title",
            "description",
            "location",
            "notes",
            "calendar_id",
            "hangout_link",
            "conference_data",
            "conference_uri",
        ]),
        action: ToolRedactionAction::Replace("calendar_content"),
    },
];

const MESSAGING_RULES: &[ToolSensitiveFieldRule] = &[
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Contains(&[
            "channel",
            "conversation",
            "user",
            "member",
            "team",
            "workspace",
            "chat",
        ]),
        action: ToolRedactionAction::Replace("message_identity"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "text",
            "message",
            "body",
            "blocks",
            "attachments",
            "permalink",
            "thread",
            "thread_ts",
        ]),
        action: ToolRedactionAction::Replace("message_content"),
    },
];

const ISSUE_TRACKER_RULES: &[ToolSensitiveFieldRule] = &[
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "title",
            "body",
            "description",
            "comment",
            "comments",
            "summary",
            "content",
        ]),
        action: ToolRedactionAction::Replace("issue_content"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&["url", "html_url", "api_url", "web_url", "href"]),
        action: ToolRedactionAction::SanitizeUrl("private_url"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Contains(&[
            "author",
            "assignee",
            "reviewer",
            "requester",
            "creator",
            "owner",
            "repo",
            "repository",
            "org",
            "organization",
            "project",
            "team",
            "user",
        ]),
        action: ToolRedactionAction::Replace("issue_identity"),
    },
];

const BROWSER_RULES: &[ToolSensitiveFieldRule] = &[
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&["url", "href", "referrer", "referer", "current_url"]),
        action: ToolRedactionAction::SanitizeUrl("private_url"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&["headers", "header", "cookies", "cookie"]),
        action: ToolRedactionAction::RedactObjectValues("browser_header"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&["body", "html", "text", "title", "content", "dom"]),
        action: ToolRedactionAction::Replace("browser_content"),
    },
];

const DATABASE_RULES: &[ToolSensitiveFieldRule] = &[
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "query",
            "sql",
            "statement",
            "prepared_statement",
            "connection_string",
            "database_url",
        ]),
        action: ToolRedactionAction::Replace("database_content"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "params",
            "parameters",
            "args",
            "arguments",
            "values",
            "binds",
            "bindings",
            "query_params",
        ]),
        action: ToolRedactionAction::SummarizeCollection("database_query_param"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "row", "rows", "record", "records", "result", "results", "data",
        ]),
        action: ToolRedactionAction::SummarizeCollection("database_row"),
    },
];

const FILESYSTEM_RULES: &[ToolSensitiveFieldRule] = &[
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Contains(&["path", "file", "filename", "cwd", "directory"]),
        action: ToolRedactionAction::Replace("local_path"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "content", "contents", "command", "stdout", "stderr", "diff", "patch",
        ]),
        action: ToolRedactionAction::Replace("workspace_content"),
    },
];

fn tool_redaction_action(
    profile: ToolPayloadProfile,
    field_name: &str,
) -> Option<ToolRedactionAction> {
    let lower = field_name.to_ascii_lowercase();

    profile_rules(profile)
        .iter()
        .find(|rule| field_matches(&lower, rule.matcher))
        .map(|rule| rule.action)
}

fn profile_rules(profile: ToolPayloadProfile) -> &'static [ToolSensitiveFieldRule] {
    match profile {
        ToolPayloadProfile::Email => EMAIL_RULES,
        ToolPayloadProfile::Calendar => CALENDAR_RULES,
        ToolPayloadProfile::Messaging => MESSAGING_RULES,
        ToolPayloadProfile::IssueTracker => ISSUE_TRACKER_RULES,
        ToolPayloadProfile::Browser => BROWSER_RULES,
        ToolPayloadProfile::Database => DATABASE_RULES,
        ToolPayloadProfile::Filesystem => FILESYSTEM_RULES,
    }
}

fn field_matches(lower_field_name: &str, matcher: ToolFieldMatcher) -> bool {
    match matcher {
        ToolFieldMatcher::Exact(names) => names.contains(&lower_field_name),
        ToolFieldMatcher::Contains(fragments) => fragments
            .iter()
            .any(|fragment| lower_field_name.contains(fragment)),
    }
}

fn apply_tool_redaction_action(value: &Value, action: ToolRedactionAction) -> Value {
    match action {
        ToolRedactionAction::Replace(label) => redacted_scalar_or_summary(label, value),
        ToolRedactionAction::SanitizeUrl(label) => sanitize_url_value(value, label),
        ToolRedactionAction::RedactObjectValues(label) => redact_object_values(value, label),
        ToolRedactionAction::SummarizeCollection(label) => summarize_collection(label, value),
    }
}

fn redacted_scalar_or_summary(label: &str, value: &Value) -> Value {
    match value {
        Value::Array(_) | Value::Object(_) => summarize_collection(label, value),
        _ => Value::String(redacted_marker(label)),
    }
}

fn redact_object_values(value: &Value, label: &str) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.keys()
                .map(|key| (key.clone(), Value::String(redacted_marker(label))))
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| redact_object_values(item, label))
                .collect(),
        ),
        _ => Value::String(redacted_marker(label)),
    }
}

fn summarize_collection(label: &str, value: &Value) -> Value {
    match value {
        Value::Array(items) => serde_json::json!({
            "redacted": redacted_marker(label),
            "count": items.len(),
        }),
        Value::Object(map) => serde_json::json!({
            "redacted": redacted_marker(label),
            "field_count": map.len(),
        }),
        _ => Value::String(redacted_marker(label)),
    }
}

fn sanitize_url_value(value: &Value, label: &str) -> Value {
    match value {
        Value::String(url) => Value::String(sanitize_private_url(url, label)),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| sanitize_url_value(item, label))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, child)| (key.clone(), sanitize_url_value(child, label)))
                .collect(),
        ),
        _ => Value::String(redacted_marker(label)),
    }
}

fn sanitize_private_url(raw_url: &str, label: &str) -> String {
    let Ok(mut url) = url::Url::parse(raw_url) else {
        return redacted_marker(label);
    };

    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return redacted_marker(label);
    }

    let has_private_components =
        url.path() != "/" || url.query().is_some() || url.fragment().is_some();
    if has_private_components {
        url.set_path("/[REDACTED_PATH]");
    }
    url.set_query(None);
    url.set_fragment(None);
    if !url.username().is_empty() {
        let _ = url.set_username("");
    }
    let _ = url.set_password(None);
    url.to_string()
}

fn redacted_marker(label: &str) -> String {
    format!("[REDACTED:{label}]")
}

fn count_sensitive_field_redactions(before: &Value, after: &Value, report: &mut RedactionReport) {
    match (before, after) {
        (Value::Object(before_map), Value::Object(after_map)) => {
            for (key, before_value) in before_map {
                if let Some(after_value) = after_map.get(key) {
                    count_sensitive_field_redactions(before_value, after_value, report);
                }
            }
        }
        (Value::Array(before_items), Value::Array(after_items)) => {
            for (before_value, after_value) in before_items.iter().zip(after_items.iter()) {
                count_sensitive_field_redactions(before_value, after_value, report);
            }
        }
        (before_value, Value::String(redacted))
            if redacted == "[REDACTED]" && before_value != after =>
        {
            report.increment("sensitive_field");
        }
        _ => {}
    }
}

fn apply_redaction_ranges(input: &str, ranges: &[std::ops::Range<usize>]) -> String {
    apply_labeled_ranges(input, ranges, "[REDACTED]")
}

/// Whole-block PEM redaction. Runs before the leak scan so an entire
/// `-----BEGIN ... PRIVATE KEY-----` .. `-----END ... PRIVATE KEY-----`
/// block (header, base64 body, and footer) is replaced in one pass rather
/// than leaving the base64 body to survive header-only redaction.
fn apply_pem_block_redaction(input: &str, report: &mut RedactionReport) -> String {
    let ranges: Vec<std::ops::Range<usize>> = pem_block_regex()
        .find_iter(input)
        .map(|matched| matched.start()..matched.end())
        .collect();
    if ranges.is_empty() {
        return input.to_string();
    }
    for _ in &ranges {
        report.increment("secret");
        report.increment("secret:pem_private_key");
        report.blocked_secret_detected = true;
    }
    apply_labeled_ranges(input, &ranges, "<REDACTED_PRIVATE_KEY>")
}

fn apply_placeholder_regex(
    input: &str,
    regex: &Regex,
    label: &str,
    state: &mut RedactionState,
    report: &mut RedactionReport,
) -> String {
    let mut result = String::with_capacity(input.len());
    let mut last_end = 0usize;
    let mut changed = false;

    for mat in regex.find_iter(input) {
        let candidate = mat.as_str();
        if candidate.contains("<PRIVATE_") || candidate.contains("[REDACTED") {
            continue;
        }
        result.push_str(&input[last_end..mat.start()]);
        let placeholder = state.placeholders.placeholder_for(label, candidate);
        result.push_str(&placeholder);
        last_end = mat.end();
        report.increment(label);
        report.add_pii_label(label);
        changed = true;
    }

    if !changed {
        return input.to_string();
    }
    result.push_str(&input[last_end..]);
    result
}

fn apply_labeled_ranges(
    input: &str,
    ranges: &[std::ops::Range<usize>],
    replacement: &str,
) -> String {
    if ranges.is_empty() {
        return input.to_string();
    }

    let mut ranges = ranges.to_vec();
    ranges.sort_by_key(|range| range.start);

    let mut result = String::with_capacity(input.len());
    let mut last_end = 0;
    for range in ranges {
        if range.start < last_end {
            continue;
        }
        result.push_str(&input[last_end..range.start]);
        result.push_str(replacement);
        last_end = range.end;
    }
    result.push_str(&input[last_end..]);
    result
}

fn private_email_regex() -> &'static Regex {
    static PRIVATE_EMAIL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        // safety: hardcoded regex is covered by unit tests and should always compile.
        Regex::new(r"(?i)\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b")
            .expect("hardcoded private email regex must compile")
    });
    &PRIVATE_EMAIL_REGEX
}

fn pem_block_regex() -> &'static Regex {
    static PEM_BLOCK_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        // safety: hardcoded regex is covered by unit tests and should always compile.
        Regex::new(r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----")
            .expect("hardcoded PEM block regex must compile")
    });
    &PEM_BLOCK_REGEX
}

fn local_path_regex() -> &'static Regex {
    static LOCAL_PATH_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        // safety: hardcoded regex is covered by unit tests and should always compile.
        Regex::new(r#"(?x)(?:/Users|/home|/private/var|/tmp)/[^\s'"`<>{}\[\]]+"#)
            .expect("hardcoded local path regex must compile")
    });
    &LOCAL_PATH_REGEX
}

fn trace_queue_secret_like_reason_regex() -> &'static Regex {
    static TRACE_QUEUE_SECRET_LIKE_REASON_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        // safety: hardcoded regex is covered by queue diagnostics tests.
        Regex::new(r"(?ix)\b(?:sk|pk|rk|ghp|gho|ghu|glpat|xox[baprs])[-_a-z0-9]{8,}\b")
            .expect("hardcoded trace queue secret-like reason regex must compile")
    });
    &TRACE_QUEUE_SECRET_LIKE_REASON_REGEX
}

fn remote_credit_explanation_url_regex() -> &'static Regex {
    static REMOTE_CREDIT_EXPLANATION_URL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        // safety: hardcoded regex is covered by local status-history safety tests.
        Regex::new(r#"(?i)\bhttps?://[^\s'"`<>{}\[\]]+"#)
            .expect("hardcoded remote credit explanation URL regex must compile")
    });
    &REMOTE_CREDIT_EXPLANATION_URL_REGEX
}

fn remote_credit_explanation_tenant_ref_regex() -> &'static Regex {
    static REMOTE_CREDIT_EXPLANATION_TENANT_REF_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        // safety: hardcoded regex is covered by local status-history safety tests.
        Regex::new(r"(?i)\btenant[-_][a-z0-9][a-z0-9_-]{1,}\b")
            .expect("hardcoded remote credit explanation tenant-ref regex must compile")
    });
    &REMOTE_CREDIT_EXPLANATION_TENANT_REF_REGEX
}

fn placeholder_label_fragment(label: &str) -> String {
    let raw = label
        .strip_prefix("private_")
        .unwrap_or(label)
        .to_ascii_uppercase();
    raw.chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn path_to_string(path: PathBuf) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceSubmissionReceipt {
    #[serde(default = "default_submission_status")]
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_points_pending: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_points_final: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub explanation: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceSubmissionStatusRequest {
    pub submission_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceSubmissionStatusUpdate {
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub status: String,
    pub credit_points_pending: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_points_final: Option<f32>,
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub credit_points_ledger: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_points_total: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub explanation: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delayed_credit_explanations: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consent_scopes: Vec<ConsentScope>,
}

pub fn apply_credit_estimate_to_envelope(envelope: &mut TraceContributionEnvelope) {
    let estimate = estimate_initial_credit(envelope);
    envelope.value.submission_score = estimate.submission_score;
    envelope.value.credit_points_pending = estimate.credit_points_pending;
    envelope.value.explanation = estimate.explanation;
    envelope.value_card.scorecard = estimate.scorecard;
    envelope.value_card.user_visible_explanation = envelope.value.explanation.clone();
}

#[cfg(test)]
mod training_dynamics_tests {
    use super::{CartographyBucket, TrainingDynamicsSignals, reduce_token_confidences};

    fn approx(actual: Option<f32>, expected: f32) {
        let actual = actual.expect("value present");
        assert!(
            (actual - expected).abs() < 1e-4,
            "expected ~{expected}, got {actual}"
        );
    }

    // -- reduction ------------------------------------------------------------

    /// Nothing observed means nothing claimed. An empty capture must not
    /// produce a confident-looking zero.
    #[test]
    fn empty_input_measures_nothing() {
        let signals = reduce_token_confidences(&[]);
        assert_eq!(signals, TrainingDynamicsSignals::default());
        assert!(signals.mean_confidence.is_none());
        assert!(signals.cartography_bucket.is_none());
    }

    #[test]
    fn mean_confidence_is_the_mean_of_chosen_token_probabilities() {
        let signals = reduce_token_confidences(&[0.2, 0.4, 0.6, 0.8]);
        approx(signals.mean_confidence, 0.5);
    }

    /// Population standard deviation, not sample: we are describing the tokens
    /// we have, not estimating a parameter of a population we sampled from.
    #[test]
    fn variability_is_population_standard_deviation() {
        let signals = reduce_token_confidences(&[0.2, 0.4, 0.6, 0.8]);
        // mean 0.5; deviations 0.3/0.1/0.1/0.3 → variance 0.05 → sd ~0.2236
        approx(signals.variability, 0.223_607);
    }

    #[test]
    fn a_single_token_has_no_variability() {
        let signals = reduce_token_confidences(&[0.42]);
        approx(signals.mean_confidence, 0.42);
        approx(signals.variability, 0.0);
    }

    /// `correctness` asks whether the work was right. No arrangement of
    /// log-probabilities answers that, so the reduction must leave it unset
    /// rather than substitute confidence for correctness.
    #[test]
    fn correctness_is_never_inferred_from_confidence() {
        for probs in [
            vec![0.99, 0.99, 0.99],
            vec![0.01, 0.01, 0.01],
            vec![0.5, 0.9, 0.1],
        ] {
            assert!(
                reduce_token_confidences(&probs).correctness.is_none(),
                "correctness must come from an outcome signal, never from confidence"
            );
        }
    }

    // -- bucketing ------------------------------------------------------------

    #[test]
    fn steady_and_confident_is_easy() {
        let signals = reduce_token_confidences(&[0.95, 0.93, 0.97, 0.94]);
        assert_eq!(signals.cartography_bucket, Some(CartographyBucket::Easy));
    }

    #[test]
    fn steady_and_unconfident_is_hard() {
        let signals = reduce_token_confidences(&[0.10, 0.12, 0.08, 0.11]);
        assert_eq!(signals.cartography_bucket, Some(CartographyBucket::Hard));
    }

    /// High dispersion wins regardless of the mean: a run that swings between
    /// certainty and doubt is the interesting case, and averaging hides it.
    #[test]
    fn high_variability_is_ambiguous_whatever_the_mean() {
        let low_mean = reduce_token_confidences(&[0.99, 0.01, 0.99, 0.01]);
        assert_eq!(
            low_mean.cartography_bucket,
            Some(CartographyBucket::Ambiguous)
        );
        let high_mean = reduce_token_confidences(&[1.0, 0.4, 1.0, 0.4]);
        assert_eq!(
            high_mean.cartography_bucket,
            Some(CartographyBucket::Ambiguous)
        );
    }

    // -- validation -----------------------------------------------------------

    /// These arrive inside a submitted envelope, so they are contributor-
    /// supplied and adversarial by default.
    #[test]
    fn well_formed_signals_validate() {
        assert!(
            reduce_token_confidences(&[0.3, 0.7])
                .validation_error()
                .is_none()
        );
        assert!(
            TrainingDynamicsSignals::default()
                .validation_error()
                .is_none()
        );
        assert!(
            TrainingDynamicsSignals {
                mean_confidence: Some(0.0),
                variability: Some(1.0),
                correctness: Some(1.0),
                cartography_bucket: Some(CartographyBucket::Unknown),
            }
            .validation_error()
            .is_none()
        );
    }

    #[test]
    fn out_of_range_values_are_rejected() {
        for (field, signals) in [
            (
                "mean_confidence",
                TrainingDynamicsSignals {
                    mean_confidence: Some(1.5),
                    ..Default::default()
                },
            ),
            (
                "mean_confidence",
                TrainingDynamicsSignals {
                    mean_confidence: Some(-0.1),
                    ..Default::default()
                },
            ),
            (
                "variability",
                TrainingDynamicsSignals {
                    variability: Some(9.0),
                    ..Default::default()
                },
            ),
            (
                "correctness",
                TrainingDynamicsSignals {
                    correctness: Some(-2.0),
                    ..Default::default()
                },
            ),
        ] {
            let error = signals.validation_error();
            assert_eq!(
                error,
                Some(field),
                "expected {field} to be rejected, got {error:?}"
            );
        }
    }

    /// JSON cannot carry NaN or infinity, but the type can be built in process
    /// and a future transport might. Bounds that only hold for JSON are not
    /// bounds.
    #[test]
    fn non_finite_values_are_rejected() {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let signals = TrainingDynamicsSignals {
                mean_confidence: Some(value),
                ..Default::default()
            };
            assert_eq!(signals.validation_error(), Some("mean_confidence"));
        }
    }

    /// Everything the reduction can produce must pass the validation the
    /// server applies, or we have shipped a producer that fails our own gate.
    #[test]
    fn reduction_output_always_validates() {
        let corpora: Vec<Vec<f32>> = vec![
            vec![0.0, 0.0, 0.0],
            vec![1.0, 1.0, 1.0],
            vec![0.0, 1.0],
            vec![1.0, 0.0, 1.0, 0.0, 1.0],
            vec![0.5],
            (0..1000).map(|i| (i % 101) as f32 / 100.0).collect(),
        ];
        for probs in corpora {
            let signals = reduce_token_confidences(&probs);
            assert_eq!(
                signals.validation_error(),
                None,
                "reduction produced something the server would reject: {signals:?}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Pins what each consent scope PERMITS, not merely what it is called.
    ///
    /// `tests/consent_policy_pin.rs` pins the scope names, the wire strings and
    /// the variant set, so a rename or an addition cannot slip past. It cannot
    /// see this mapping, which is private to the crate -- and this mapping is
    /// the part a contributor actually consented to. Adding
    /// `TraceAllowedUse::ModelTraining` to `DebuggingEvaluation` here would
    /// widen the default scope for every future submission, break no test, and
    /// require no policy-version bump.
    ///
    /// So: changing this table changes what <https://tracecommons.ai/legal/>
    /// part C means. Publish the new text and bump
    /// `TRACE_CONTRIBUTION_POLICY_VERSION` (and `src/policy.ts` in the
    /// community repo) before you change it here.
    #[test]
    fn scope_permissions_match_the_published_document() {
        use super::{ConsentScope, TraceAllowedUse, default_allowed_uses_for_scope};

        let cases: &[(ConsentScope, &[TraceAllowedUse])] = &[
            (
                ConsentScope::DebuggingEvaluation,
                &[
                    TraceAllowedUse::Debugging,
                    TraceAllowedUse::Evaluation,
                    TraceAllowedUse::AggregateAnalytics,
                ],
            ),
            (
                ConsentScope::BenchmarkOnly,
                &[
                    TraceAllowedUse::Evaluation,
                    TraceAllowedUse::BenchmarkGeneration,
                    TraceAllowedUse::AggregateAnalytics,
                ],
            ),
            (
                ConsentScope::RankingTraining,
                &[
                    TraceAllowedUse::Debugging,
                    TraceAllowedUse::Evaluation,
                    TraceAllowedUse::RankingModelTraining,
                    TraceAllowedUse::AggregateAnalytics,
                ],
            ),
            (
                ConsentScope::ModelTraining,
                &[
                    TraceAllowedUse::Debugging,
                    TraceAllowedUse::Evaluation,
                    TraceAllowedUse::RankingModelTraining,
                    TraceAllowedUse::ModelTraining,
                    TraceAllowedUse::AggregateAnalytics,
                ],
            ),
            // Deliberately empty: public_attribution is a profile-management
            // consent and grants no trace-content use. Part C.5 of the
            // published document says exactly this.
            (ConsentScope::PublicAttribution, &[]),
        ];

        for (scope, expected) in cases {
            assert_eq!(
                default_allowed_uses_for_scope(*scope),
                expected.to_vec(),
                "consent scope {scope:?} no longer permits what part C of \
                 https://tracecommons.ai/legal/ says it permits. Traces \
                 already submitted under this scope were consented on the \
                 published wording, so widening it here changes the meaning \
                 of consent already given",
            );
        }
    }

    #[test]
    fn read_privacy_env_prefers_canonical_then_legacy() {
        use super::read_privacy_env;
        let _guard = ENV_LOCK.lock().unwrap();
        let canonical = "TRACE_PRIVACY_FILTER_TEST_CANONICAL_XYZ";
        let legacy = "IRONCLAW_TRACE_PRIVACY_FILTER_TEST_CANONICAL_XYZ";
        // SAFETY: holding ENV_LOCK serializes env mutation across all
        // env-touching tests in this crate. Edition 2024 marks these
        // unsafe because env is process-global state.
        unsafe {
            std::env::remove_var(canonical);
            std::env::remove_var(legacy);
            assert_eq!(read_privacy_env(canonical, legacy), None);

            std::env::set_var(legacy, "legacy-value");
            assert_eq!(
                read_privacy_env(canonical, legacy).as_deref(),
                Some("legacy-value")
            );

            std::env::set_var(canonical, "canonical-value");
            assert_eq!(
                read_privacy_env(canonical, legacy).as_deref(),
                Some("canonical-value")
            );

            std::env::remove_var(canonical);
            std::env::remove_var(legacy);
        }
    }

    #[test]
    fn privacy_filter_config_error_messages_are_stable() {
        use super::PrivacyFilterConfigError;
        let e = PrivacyFilterConfigError::UnknownBackend {
            value: "junk".into(),
        };
        assert_eq!(
            e.to_string(),
            "unknown TRACE_PRIVACY_FILTER_BACKEND value: junk"
        );
        let e = PrivacyFilterConfigError::MissingEnv {
            backend: "near-ai",
            var: "TRACE_NEAR_AI_PRIVACY_API_KEY",
        };
        assert_eq!(
            e.to_string(),
            "missing required env var for backend near-ai: TRACE_NEAR_AI_PRIVACY_API_KEY"
        );
        let e = PrivacyFilterConfigError::FeatureDisabled {
            backend: "near-ai",
            feature: "near-ai-privacy-filter",
        };
        assert_eq!(
            e.to_string(),
            "backend near-ai requires the near-ai-privacy-filter cargo feature"
        );
        let e = PrivacyFilterConfigError::InvalidEnv {
            var: "TRACE_NEAR_AI_PRIVACY_TIMEOUT_MS",
            reason: "not a number".into(),
        };
        assert_eq!(
            e.to_string(),
            "invalid env var TRACE_NEAR_AI_PRIVACY_TIMEOUT_MS: not a number"
        );
    }

    #[test]
    fn redaction_pipeline_version_emits_per_backend_suffix() {
        use super::{
            DETERMINISTIC_REDACTION_PIPELINE_VERSION, PrivacyFilterBackendTag,
            redaction_pipeline_version,
        };
        assert_eq!(
            redaction_pipeline_version(PrivacyFilterBackendTag::None),
            DETERMINISTIC_REDACTION_PIPELINE_VERSION
        );
        assert_eq!(
            redaction_pipeline_version(PrivacyFilterBackendTag::Sidecar),
            format!("{DETERMINISTIC_REDACTION_PIPELINE_VERSION}+privacy-filter-sidecar-v1")
        );
        assert_eq!(
            redaction_pipeline_version(PrivacyFilterBackendTag::NearAi),
            format!("{DETERMINISTIC_REDACTION_PIPELINE_VERSION}+privacy-filter-near-ai-v1")
        );
    }

    #[test]
    fn privacy_filter_adapter_from_env_returns_none_when_unset() {
        use super::privacy_filter_adapter_from_env;
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("TRACE_PRIVACY_FILTER_BACKEND");
        }
        let result = privacy_filter_adapter_from_env().expect("should be Ok");
        assert!(result.is_none());
    }

    #[test]
    fn privacy_filter_adapter_from_env_rejects_unknown_backend() {
        use super::{PrivacyFilterConfigError, privacy_filter_adapter_from_env};
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("TRACE_PRIVACY_FILTER_BACKEND", "garbage");
        }
        match privacy_filter_adapter_from_env() {
            Err(PrivacyFilterConfigError::UnknownBackend { value }) => {
                assert_eq!(value, "garbage")
            }
            Err(other) => panic!("expected UnknownBackend, got err {other:?}"),
            Ok(_) => panic!("expected UnknownBackend, got Ok"),
        }
        unsafe {
            std::env::remove_var("TRACE_PRIVACY_FILTER_BACKEND");
        }
    }

    #[test]
    fn privacy_filter_adapter_from_env_requires_near_ai_key() {
        use super::{PrivacyFilterConfigError, privacy_filter_adapter_from_env};
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("TRACE_PRIVACY_FILTER_BACKEND", "near-ai");
            std::env::remove_var("TRACE_NEAR_AI_PRIVACY_API_KEY");
        }
        match privacy_filter_adapter_from_env() {
            Err(PrivacyFilterConfigError::MissingEnv { backend, var }) => {
                assert_eq!(backend, "near-ai");
                assert_eq!(var, "TRACE_NEAR_AI_PRIVACY_API_KEY");
            }
            // When feature is off, FeatureDisabled is also acceptable here:
            Err(PrivacyFilterConfigError::FeatureDisabled { backend, feature }) => {
                assert_eq!(backend, "near-ai");
                assert_eq!(feature, "near-ai-privacy-filter");
            }
            Err(other) => panic!("unexpected err: {other:?}"),
            Ok(_) => panic!("expected MissingEnv or FeatureDisabled, got Ok"),
        }
        unsafe {
            std::env::remove_var("TRACE_PRIVACY_FILTER_BACKEND");
        }
    }

    #[derive(Debug)]
    struct AlwaysFailingPrivacyFilterAdapter;

    #[async_trait::async_trait]
    impl super::PrivacyFilterAdapter for AlwaysFailingPrivacyFilterAdapter {
        async fn redact_text(
            &self,
            _text: &str,
        ) -> Result<Option<super::SafePrivacyFilterRedaction>, super::TraceContributionError>
        {
            Err(super::TraceContributionError::RedactionFailed {
                reason: "synthetic adapter failure for tests".to_string(),
            })
        }
    }

    #[tokio::test]
    async fn near_ai_runtime_error_propagates_fail_closed() {
        use super::{
            DeterministicTraceRedactor, PrivacyFilterBackendTag, RedactionReport,
            TraceContributionError,
        };
        use std::sync::Arc;
        let adapter = Arc::new(AlwaysFailingPrivacyFilterAdapter);
        let redactor = DeterministicTraceRedactor::bare()
            .with_privacy_filter(adapter, PrivacyFilterBackendTag::NearAi);
        let mut report = RedactionReport::default();
        let mut summary = None;
        let result = redactor
            .apply_privacy_filter_to_text(
                "alice@example.com".to_string(),
                &mut report,
                &mut summary,
            )
            .await;
        match result {
            Err(TraceContributionError::RedactionFailed { reason }) => {
                assert!(
                    reason.contains("synthetic adapter failure"),
                    "unexpected reason: {reason}"
                );
            }
            Ok(text) => panic!("expected error; got Ok({text:?})"),
        }
        // No fallback warning or counter should be emitted in fail-closed
        // mode.
        let dump = format!("{:?}", report);
        assert!(
            !dump.contains("sidecar_failure") && !dump.contains("near_ai_failure"),
            "fail-closed must not emit a backend_failure counter: {dump}"
        );
    }

    #[tokio::test]
    async fn sidecar_runtime_error_falls_back_with_backend_label() {
        use super::{DeterministicTraceRedactor, PrivacyFilterBackendTag, RedactionReport};
        use std::sync::Arc;
        let adapter = Arc::new(AlwaysFailingPrivacyFilterAdapter);
        let redactor = DeterministicTraceRedactor::bare()
            .with_privacy_filter(adapter, PrivacyFilterBackendTag::Sidecar);
        let mut report = RedactionReport::default();
        let mut summary = None;
        let text = redactor
            .apply_privacy_filter_to_text(
                "alice@example.com".to_string(),
                &mut report,
                &mut summary,
            )
            .await
            .expect("sidecar must swallow runtime errors");
        // Original text is returned to the caller (sidecar legacy
        // contract).
        assert_eq!(text, "alice@example.com");
        let dump = format!("{:?}", report);
        assert!(
            dump.contains("privacy_filter:sidecar_failure"),
            "expected sidecar_failure counter to be incremented; got {dump}"
        );
        assert!(
            dump.contains("sidecar backend failed"),
            "expected backend-aware warning; got {dump}"
        );
        // Case 4 (issue #373): the configured filter did not examine this
        // text, so the pass must not be able to speak for it.
        assert!(
            report.coverage_incomplete,
            "a filter fallback must mark the pass as not covering the text"
        );
        let consent = super::ConsentMetadata {
            policy_version: super::TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
            scopes: vec![super::ConsentScope::DebuggingEvaluation],
            message_text_included: false,
            tool_payloads_included: false,
            revocable: true,
        };
        assert_eq!(
            super::residual_risk(&consent, &report),
            super::ResidualPiiRisk::High,
            "a coverage gap must fail closed to High"
        );
    }

    #[test]
    fn legacy_privacy_env_emits_deprecation_warning_once() {
        use super::{read_privacy_env, reset_legacy_privacy_env_warning_for_tests};
        let _guard = ENV_LOCK.lock().unwrap();
        reset_legacy_privacy_env_warning_for_tests();
        let canonical = "TRACE_PRIVACY_FILTER_TEST_DEPR_ABCDEF";
        let legacy = "IRONCLAW_TRACE_PRIVACY_FILTER_TEST_DEPR_ABCDEF";
        unsafe {
            std::env::remove_var(canonical);
            std::env::set_var(legacy, "legacy-only");
        }
        // First read should trigger the warning (best-effort: we cannot
        // capture stderr without extra deps, but we can verify the
        // atomic transitioned). The call itself must not panic.
        assert_eq!(
            read_privacy_env(canonical, legacy).as_deref(),
            Some("legacy-only")
        );
        // After the first emission, the atomic is set; subsequent reads
        // must not reset it.
        assert!(
            super::LEGACY_PRIVACY_ENV_WARNED.load(std::sync::atomic::Ordering::SeqCst),
            "warning latch must be set after first legacy read"
        );
        // Second read; latch should remain true (idempotent).
        let _ = read_privacy_env(canonical, legacy);
        assert!(super::LEGACY_PRIVACY_ENV_WARNED.load(std::sync::atomic::Ordering::SeqCst));
        unsafe {
            std::env::remove_var(legacy);
        }
        reset_legacy_privacy_env_warning_for_tests();
    }

    #[test]
    fn privacy_filter_adapter_from_env_requires_sidecar_command() {
        use super::{PrivacyFilterConfigError, privacy_filter_adapter_from_env};
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("TRACE_PRIVACY_FILTER_BACKEND", "sidecar");
            std::env::remove_var("TRACE_PRIVACY_FILTER_COMMAND");
            std::env::remove_var("IRONCLAW_TRACE_PRIVACY_FILTER_COMMAND");
        }
        match privacy_filter_adapter_from_env() {
            Err(PrivacyFilterConfigError::MissingEnv { backend, var }) => {
                assert_eq!(backend, "sidecar");
                assert_eq!(var, "TRACE_PRIVACY_FILTER_COMMAND");
            }
            Err(other) => panic!("unexpected err: {other:?}"),
            Ok(_) => panic!("expected MissingEnv, got Ok"),
        }
        unsafe {
            std::env::remove_var("TRACE_PRIVACY_FILTER_BACKEND");
        }
    }

    #[test]
    fn redact_text_strips_broadened_secret_shapes() {
        use super::*;
        let r = DeterministicTraceRedactor::bare();
        // JWT (three base64url segments)
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N";
        let (out, rep) = r.redact_text(&format!("Authorization: Bearer {jwt}"));
        assert!(!out.contains(jwt), "jwt survived: {out}");
        assert!(rep.blocked_secret_detected);
        // npm + google
        let (o2, _) = r.redact_text("token npm_abcdefghijklmnopqrstuvwxyz0123456789 done");
        assert!(!o2.contains("npm_abcdefghijklmnopqrstuvwxyz0123456789"));
        let (o3, _) = r.redact_text("key AIzaSyA1234567890abcdefghijklmnopqrstuvw end");
        assert!(!o3.contains("AIzaSyA1234567890abcdefghijklmnopqrstuvw"));
    }

    #[test]
    fn redact_text_removes_entire_pem_block_not_just_header() {
        use super::*;
        let r = DeterministicTraceRedactor::bare();
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA1234secretbody5678\nabcDEFghiJKL==\n-----END RSA PRIVATE KEY-----";
        let (out, rep) = r.redact_text(&format!("here is a key:\n{pem}\ntrailing"));
        assert!(
            !out.contains("1234secretbody5678"),
            "pem body survived: {out}"
        );
        assert!(
            !out.contains("abcDEFghiJKL"),
            "pem body line 2 survived: {out}"
        );
        assert!(out.contains("trailing"));
        assert!(rep.blocked_secret_detected);
    }

    #[test]
    fn redact_text_catches_orphan_pem_header_without_end() {
        use super::*;
        let r = DeterministicTraceRedactor::bare();
        let truncated = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAAsecretbytes";
        let (out, _) = r.redact_text(truncated);
        assert!(
            !out.contains("secretbytes"),
            "orphan pem body survived: {out}"
        );
    }

    #[test]
    fn contextual_entropy_redacts_unknown_key_after_cue() {
        use super::*;
        let r = DeterministicTraceRedactor::bare();
        // opaque high-entropy token, no known prefix, but preceded by a cue
        let secret = "Zx9Qk2Lm7Pv4Rt8Wy1Nb6Hd3Fg5Jc0Ae";
        let (out, rep) = r.redact_text(&format!("api_key: {secret}"));
        assert!(!out.contains(secret), "cue-adjacent secret survived: {out}");
        assert!(rep.blocked_secret_detected);
    }

    #[test]
    fn contextual_entropy_redacts_unspaced_assignment() {
        use super::*;
        let r = DeterministicTraceRedactor::new(vec![]).unwrap();
        let secret = "Zx9Qk2Lm7Pv4Rt8Wy1Nb6Hd3Fg5Jc0Ae";
        // Same secret and cue as the spaced case above; only the space is gone.
        // The cue is inside the candidate, so the window check cannot see it.
        for text in [
            format!("api_key={secret}"),
            format!("password={secret}"),
            // Compound names are the common real shape in an env dump.
            format!("OPENAI_API_KEY={secret}"),
            format!("x-api-key={secret}"),
            format!("TRACE_SERVICE_ACCESS_TOKEN={secret}"),
        ] {
            let (out, rep) = r.redact_text(&text);
            assert!(!out.contains(secret), "cue-glued secret survived: {out}");
            assert!(
                rep.blocked_secret_detected,
                "glued secret did not set blocked_secret_detected: {text}"
            );
        }
    }

    // `contextual_entropy_keeps_outer_cue_when_inner_value_is_short` (a
    // "password: api_key=<12-char value>" case) was removed here: the
    // whole-token reading that catches it is [`has_secret_cue`]'s pre-existing
    // cue-window check running against `candidate.start()`, which is
    // unchanged by this pass's `=`-split addition and already caught this
    // shape before it. Verified by reverting the split addition in
    // `contextual_entropy_secret_ranges` back to a single whole-token read and
    // confirming the case above still redacts. A genuinely split-dependent
    // case needs the value to be reached ONLY via a split re-anchor, which
    // `contextual_entropy_redacts_unspaced_assignment` and
    // `contextual_entropy_still_redacts_credential_named_tokens_when_glued`
    // already cover (self-cue immediately before the value, invisible to the
    // whole-token window).

    #[test]
    fn contextual_entropy_redacts_cue_named_values_consistently_across_spellings() {
        use super::*;
        let r = DeterministicTraceRedactor::new(vec![]).unwrap();
        // Opaque cursors carry the `token` cue and are now redacted in the
        // glued spelling too. That is over-redaction of non-secret content,
        // which the redaction policy accepts: over-redaction is tolerable,
        // under-redaction is the defect. It also makes the two spellings
        // agree, since the spaced form is already redacted today.
        let cursor = "eyJvZmZzZXQiOjEwMCwic29ydCI6ImFzYyJ9==";
        for text in [
            format!("page_token: {cursor}"),
            format!("page_token={cursor}"),
        ] {
            let (out, _) = r.redact_text(&text);
            assert_ne!(out, text, "cue-named value not redacted: {text}");
        }
    }

    #[test]
    fn contextual_entropy_still_redacts_credential_named_tokens_when_glued() {
        use super::*;
        let r = DeterministicTraceRedactor::new(vec![]).unwrap();
        let secret = "Zx9Qk2Lm7Pv4Rt8Wy1Nb6Hd3Fg5Jc0Ae";
        for name in ["access_token", "refresh_token", "client_secret"] {
            let (out, rep) = r.redact_text(&format!("{name}={secret}"));
            assert!(!out.contains(secret), "{name} glued secret survived: {out}");
            assert!(rep.blocked_secret_detected);
        }
    }

    #[test]
    fn contextual_entropy_split_restores_the_identifier_allowlist() {
        use super::*;
        let r = DeterministicTraceRedactor::new(vec![]).unwrap();
        // None of these has a cue before the whole candidate (the key name is
        // glued to the value, so the candidate itself starts at the key
        // name), so `redact_text` alone cannot tell whether the value
        // survived because a split re-anchor found it and the allowlist
        // correctly excluded it, or because no cue was ever found for it at
        // all -- the old, pre-split detector produces the same "untouched"
        // output for the latter reason, which is why an earlier version of
        // this test (asserting only on `redact_text`'s output) passed
        // unchanged on the pre-split code and proved nothing about the split
        // path. Assert directly on the split position instead: a cue IS
        // found there, and the allowlist excludes the narrowed value anyway.
        for text in [
            "token=550e8400-e29b-41d4-a716-446655440000",
            "api_key=550e8400-e29b-41d4-a716-446655440000",
            "secret=0123456789abcdef0123456789abcdef01234567",
            "access_token=msg_01ABCDEFghijklmnopqrstuvwx",
        ] {
            let split = text.find('=').map(|i| i + 1).expect("fixture has an =");
            assert!(
                has_secret_cue(text, split),
                "split position is not reached by a cue: {text}"
            );
            assert!(
                !is_cued_secret(text, split, text.len(), true, None),
                "split reading did not apply the identifier allowlist to the \
                 narrowed value: {text}"
            );
            // End to end: the identifier must also survive the full pass.
            let (out, _) = r.redact_text(text);
            assert_eq!(out, text, "structural identifier was redacted: {out}");
        }
    }
    #[test]
    fn contextual_entropy_reads_past_junk_assignments_to_reach_the_cue() {
        use super::*;
        let r = DeterministicTraceRedactor::new(vec![]).unwrap();
        // Readings are not capped. A cap would let an attacker push the real
        // cue past it with junk `k0=k1=...` prefixes and keep the secret,
        // which the fail-closed rule forbids.
        let secret = "Zx9Qk2Lm7Pv4Rt8Wy1Nb6Hd3Fg5Jc0Ae";
        for count in [7usize, 8, 20, 64] {
            let prefix: String = (0..count).map(|i| format!("k{i}=")).collect();
            let (out, rep) = r.redact_text(&format!("{prefix}api_key={secret}"));
            assert!(
                !out.contains(secret),
                "secret survived behind {count} junk assignments: {out}"
            );
            assert!(rep.blocked_secret_detected);
        }
    }

    #[test]
    fn contextual_entropy_measures_material_beyond_the_sample_window() {
        use super::*;
        let r = DeterministicTraceRedactor::new(vec![]).unwrap();
        // A value whose entropy sits past ENTROPY_SAMPLE_BYTES. The whole-token
        // reading measures the whole token, so the spaced form keeps the
        // decision it had before sampling existed; the re-anchored reading
        // samples from the END, where a glued value lives, so the glued form is
        // covered too. Sampling from the front instead published both.
        let opaque: String = (0..4000)
            .map(|index| {
                const ALPHABET: &[u8] =
                    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
                ALPHABET[(index * 7 + 3) % ALPHABET.len()] as char
            })
            .collect();
        let padding = "a".repeat(ENTROPY_SAMPLE_BYTES);
        for text in [
            format!("api_key: {padding}{opaque}"),
            format!("api_key={padding}{opaque}"),
        ] {
            let (out, rep) = r.redact_text(&text);
            assert!(
                !out.contains(&opaque),
                "material beyond the sample window was published"
            );
            assert!(rep.blocked_secret_detected);
        }
    }

    #[test]
    fn contextual_entropy_measures_both_ends_of_a_long_value() {
        use super::*;
        let r = DeterministicTraceRedactor::new(vec![]).unwrap();
        // A single sample anchor has a blind spot at the opposite end. These
        // three arrangements put the opaque material at the start, the end, and
        // either side of a flat middle; all must be treated as secrets.
        let opaque: String = (0..4000)
            .map(|index| {
                const ALPHABET: &[u8] =
                    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
                ALPHABET[(index * 7 + 3) % ALPHABET.len()] as char
            })
            .collect();
        let flat = "a".repeat(ENTROPY_SAMPLE_BYTES);
        for body in [
            format!("{opaque}{flat}"),
            format!("{flat}{opaque}"),
            format!("{}{}{}", &opaque[..1000], flat, &opaque[1000..]),
        ] {
            for text in [format!("api_key: {body}"), format!("api_key={body}")] {
                let (out, rep) = r.redact_text(&text);
                assert_ne!(out, text, "long opaque value was published");
                assert!(rep.blocked_secret_detected);
            }
        }
    }

    #[test]
    fn contextual_entropy_finds_a_secret_between_sample_windows_on_a_long_candidate() {
        use super::*;
        // A fixed number of windows spread evenly across a long candidate
        // leaves a gap between windows that grows with the candidate's
        // length: at ~500 KB with 16 fixed 512-byte windows the gap is
        // roughly 33 KB, far wider than any real secret. An opaque value
        // placed in that gap, surrounded by low-entropy filler, was never
        // sampled at all. Use the glued (self-cue) form so the reading that
        // measures this is the bounded, windowed one
        // (`entropy_sample_bits`), not the whole-token unbounded reading.
        let r = DeterministicTraceRedactor::new(vec![]).unwrap();
        let secret = "Zx9Qk2Lm7Pv4Rt8Wy1Nb6Hd3Fg5Jc0Ae";
        let filler = "a".repeat(250_000);
        let body = format!("{filler}{secret}{filler}");
        let text = format!("api_key={body}");
        let (out, rep) = r.redact_text(&text);
        assert!(
            !out.contains(secret),
            "secret buried in the middle of a long low-entropy candidate survived"
        );
        assert!(rep.blocked_secret_detected);
    }

    /// Payload size for the contextual-entropy tripwires below.
    ///
    /// Deliberately its own number rather than the deployed input cap. These
    /// guard against superlinear cost, and a megabyte exposes that just as
    /// well as sixteen -- a quadratic pass at this size already blows the
    /// bound by orders of magnitude. Sizing them off the cap instead meant a
    /// cap change silently made each tripwire 16x slower and pushed them
    /// against their wall-clock bounds for reasons unrelated to the property
    /// under test.
    const ENTROPY_TRIPWIRE_PAYLOAD_BYTES: usize = 1024 * 1024;

    #[test]
    fn contextual_entropy_stays_bounded_on_many_separate_assignments() {
        use super::*;
        let r = DeterministicTraceRedactor::new(vec![]).unwrap();
        // Thousands of separate glued assignments, at the tripwire size.
        // Each one contributes a range, and comparing every new range against
        // every accumulated range was quadratic: a megabyte produced tens of
        // thousands of ranges and on the order of a billion comparisons.
        let unit = "api_key=Zx9Qk2Lm7Pv4Rt8Wy1 ";
        let payload = unit.repeat(ENTROPY_TRIPWIRE_PAYLOAD_BYTES / unit.len());
        let started = std::time::Instant::now();
        let (out, rep) = r.redact_text(&payload);
        let elapsed = started.elapsed();
        assert_ne!(out, payload, "assigned values were published");
        assert!(rep.blocked_secret_detected);
        assert!(
            elapsed < std::time::Duration::from_secs(20),
            "many-assignment input took {elapsed:?}"
        );
    }

    #[test]
    fn contextual_entropy_stays_bounded_on_equals_dense_input() {
        use super::*;
        let r = DeterministicTraceRedactor::new(vec![]).unwrap();
        // At only 1024 repetitions these payloads are far too small to
        // distinguish "entropy is bounded per reading" from "entropy is
        // gated behind the cheap checks": both the sampled-but-ungated and
        // the gated-and-sampled implementations finish quickly at this
        // scale, so this only guards the sampling bound added earlier in
        // this pass's history, not the ordering fixed below.
        for payload in [
            "QUJDREVGR0hJSktMTU5PUFFS=".repeat(1024),
            "api_key=aaaa".repeat(1024),
        ] {
            let started = std::time::Instant::now();
            let _ = r.redact_text(&payload);
            let elapsed = started.elapsed();
            assert!(
                elapsed < std::time::Duration::from_secs(20),
                "equals-dense input took {elapsed:?}"
            );
        }
    }

    #[test]
    fn contextual_entropy_gates_on_cue_before_entropy_on_equals_dense_input_near_max_size() {
        use super::*;
        let r = DeterministicTraceRedactor::new(vec![]).unwrap();
        // Every `=` in the input starts another reading in
        // `contextual_entropy_secret_ranges`, and the regex class
        // ([A-Za-z0-9+/=_.\-]) means a run of `x=` pairs is ONE candidate
        // spanning nearly the whole input, so a megabyte of them produces on
        // the order of half a million readings.
        // There is no cue word anywhere in "x=", so `is_cued_secret` must
        // reject every one of them on the cheap length/cue/allowlist checks
        // BEFORE computing entropy: computing entropy first, even a bounded
        // sample, on every one of half a million readings is the CPU
        // denial-of-service this test guards against. 1024 repetitions (the
        // test above) is far too small to show the difference; this uses the
        // full tripwire payload.
        let unit = "x=";
        let payload = unit.repeat(ENTROPY_TRIPWIRE_PAYLOAD_BYTES / unit.len());
        assert_eq!(payload.len(), ENTROPY_TRIPWIRE_PAYLOAD_BYTES);
        let started = std::time::Instant::now();
        let (out, rep) = r.redact_text(&payload);
        let elapsed = started.elapsed();
        assert_eq!(
            out, payload,
            "no cue anywhere in the payload, nothing should be redacted"
        );
        assert!(!rep.blocked_secret_detected);
        // A loose tripwire for the ordering regression, not a benchmark:
        // computing entropy before checking for a cue on this input took far
        // longer than this bound; gating on the cue first keeps it well
        // under even in an unoptimised build on slow CI.
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "equals-dense input with no cue anywhere took {elapsed:?}; entropy is likely being \
             computed before the cheap cue/length/allowlist gates"
        );
    }

    #[test]
    fn contextual_entropy_stays_bounded_when_a_cue_repeats_densely_through_a_long_candidate() {
        use super::*;
        let r = DeterministicTraceRedactor::new(vec![]).unwrap();
        // Gating entropy on `has_secret_cue` (the fix above) only removes the
        // cost when there is no cue at all. An attacker can trivially make a
        // cue precede nearly every `=` in one long candidate just by
        // repeating cue text, so the cheap gate alone does not bound cost:
        // if each of those readings independently rescanned the whole
        // remaining candidate for entropy, cost would still be quadratic in
        // the number of repetitions, only gated on "has a cue" instead of
        // "has an `=`". This candidate has no real secret anywhere (repeated
        // cue text plus filler is not opaque), so nothing should be flagged,
        // but the pass must still finish quickly -- it can only do that by
        // reusing one per-candidate entropy profile across every reading
        // instead of rebuilding it per `=`.
        let unit = "api_key=";
        let payload = unit.repeat(ENTROPY_TRIPWIRE_PAYLOAD_BYTES / unit.len());
        let started = std::time::Instant::now();
        let (out, rep) = r.redact_text(&payload);
        let elapsed = started.elapsed();
        assert_eq!(
            out, payload,
            "no real secret in this payload, nothing should be redacted"
        );
        assert!(!rep.blocked_secret_detected);
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "cue-dense candidate with no real secret took {elapsed:?}; entropy is likely being \
             recomputed from scratch per `=` instead of via a cached per-candidate profile"
        );
    }

    #[test]
    fn contextual_entropy_keeps_cue_name_when_redacting_assigned_value() {
        use super::*;
        let r = DeterministicTraceRedactor::new(vec![]).unwrap();
        let secret = "Zx9Qk2Lm7Pv4Rt8Wy1Nb6Hd3Fg5Jc0Ae";
        let (out, _) = r.redact_text(&format!("api_key={secret}"));
        // The field name is diagnostic, not sensitive: keep it readable.
        assert!(
            out.contains("api_key="),
            "cue name was consumed with the value: {out}"
        );
        assert!(!out.contains(secret));
    }

    #[test]
    fn contextual_entropy_spares_ids_and_hashes_and_uncued_tokens() {
        use super::*;
        let r = DeterministicTraceRedactor::bare();
        // message id after a cue-shaped word must NOT be redacted (allowlisted prefix)
        let (o1, _) = r.redact_text("token: msg_01ABCDEFghijklmnopqrstuvwx");
        assert!(
            o1.contains("msg_01ABCDEFghijklmnopqrstuvwx"),
            "allowlisted id got redacted: {o1}"
        );
        // git sha after cue must survive (hex len 40)
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let (o2, _) = r.redact_text(&format!("key {sha}"));
        assert!(o2.contains(sha), "git sha got redacted: {o2}");
        // high-entropy token with NO cue nearby must survive (avoids shredding base64 content)
        let blob = "CAESabcdef0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let (o3, _) = r.redact_text(&format!("the encoded value {blob} appears here"));
        assert!(o3.contains(blob), "uncued blob got redacted: {o3}");
    }

    /// This pass's re-anchoring after `=` only covers unspaced *assignment*
    /// glue (`api_key=<secret>`). It documents that boundary against the
    /// pre-existing evasions/exclusions rather than leaving it assumed. None
    /// of the five cases below are things this pass is meant to fix; each
    /// comment says why. Do not "fix" these here -- they are separate
    /// decisions with false-positive tradeoffs (see PR discussion for Fix 4).
    #[test]
    fn contextual_entropy_documents_the_glued_assignment_boundary() {
        use super::*;
        let r = DeterministicTraceRedactor::new(vec![]).unwrap();

        // 1. Zero-separator glue: no `=` between the cue word and the value,
        //    so there is nothing for this pass's re-anchoring to split on.
        //    The cue and the value are one token and the cue-window check
        //    never sees a cue word immediately before the candidate. NOT
        //    addressed by this pass.
        let secret = "Zx9Qk2Lm7Pv4Rt8Wy1Nb6Hd3Fg5Jc0Ae";
        for text in [format!("api_key{secret}"), format!("Bearer{secret}")] {
            let (out, _) = r.redact_text(&text);
            assert!(
                out.contains(&secret[..secret.len().min(8)]),
                "zero-separator glue was unexpectedly caught (boundary moved, update this test \
                 and the comment on contextual_entropy_secret_ranges): {out}"
            );
        }

        // 2. UUID-shaped value: intentionally allowlisted as a structural
        //    identifier (`is_allowlisted_entropy_candidate`'s `uuid_regex`
        //    check), even when cued.
        let (out, _) = r.redact_text("api_key=550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(out, "api_key=550e8400-e29b-41d4-a716-446655440000");

        // 3. Lowercase hex, length >= 32: intentionally treated as a content
        //    hash (sha256/git blob), not a secret, even when cued.
        let (out, _) = r.redact_text("secret=0123456789abcdef0123456789abcdef01234567");
        assert_eq!(out, "secret=0123456789abcdef0123456789abcdef01234567");

        // 4. Shorter than `ENTROPY_MIN_LEN` (16 chars): intentionally too
        //    short to gate reliably, even when cued and opaque.
        assert!("Zx9Qk2Lm7P".len() < ENTROPY_MIN_LEN);
        let (out, _) = r.redact_text("api_key=Zx9Qk2Lm7P");
        assert_eq!(out, "api_key=Zx9Qk2Lm7P");

        // 5. Below `ENTROPY_BITS_MIN` (3.2 bits/char): intentionally treated
        //    as not opaque enough, even when cued and long enough.
        let (out, _) = r.redact_text("api_key=aaaaaaaaaaaaaaaaaaaa");
        assert_eq!(out, "api_key=aaaaaaaaaaaaaaaaaaaa");
    }

    /// Regression: the redaction report's own metric-key literals (as
    /// embedded in `PrivacyMetadata::redaction_counts`, e.g.
    /// `"secret:contextual_entropy": 1`) must never be mistaken by the
    /// cue-gated entropy pass for a surviving secret. Without the
    /// `REPORT_METRIC_LABELS` allowlist, a fail-closed re-scan of a
    /// finished envelope (as `envelope_has_residual_secret` performs) would
    /// find "secret:" immediately followed by the report's own
    /// "contextual_entropy" counter name and wrongly flag it as a survivor
    /// -- refusing every session whose redaction pipeline legitimately
    /// found and redacted something.
    #[test]
    fn contextual_entropy_spares_its_own_report_metric_labels() {
        use super::*;
        let r = DeterministicTraceRedactor::bare();
        for label in REPORT_METRIC_LABELS {
            let json_like = format!("\"secret:{label}\":1");
            let (out, rep) = r.redact_text(&json_like);
            assert!(
                out.contains(label),
                "report metric label {label:?} was wrongly redacted: {out}"
            );
            assert!(
                !rep.blocked_secret_detected,
                "report metric label {label:?} wrongly tripped the fail-closed guard"
            );
        }
    }

    fn sample_envelope_with_event_content(content: &str) -> super::TraceContributionEnvelope {
        use super::*;
        let now = Utc::now();
        TraceContributionEnvelope {
            schema_version: TRACE_CONTRIBUTION_SCHEMA_VERSION.to_string(),
            trace_id: Uuid::new_v4(),
            submission_id: Uuid::new_v4(),
            created_at: now,
            ironclaw: IronclawTraceMetadata {
                version: "1".to_string(),
                engine_version: None,
                feature_flags: BTreeMap::new(),
                channel: TraceChannel::Cli,
                model_name: None,
            },
            consent: ConsentMetadata {
                policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
                scopes: vec![ConsentScope::DebuggingEvaluation],
                message_text_included: true,
                tool_payloads_included: false,
                revocable: true,
            },
            contributor: ContributorMetadata {
                pseudonymous_contributor_id: None,
                tenant_scope_ref: None,
                credit_account_ref: None,
                revocation_handle: Uuid::new_v4(),
            },
            privacy: PrivacyMetadata {
                redaction_pipeline_version: DETERMINISTIC_REDACTION_PIPELINE_VERSION.to_string(),
                redaction_counts: BTreeMap::new(),
                privacy_filter_summary: None,
                pii_labels_present: Vec::new(),
                residual_pii_risk: ResidualPiiRisk::Low,
                redaction_hash: "sha256:placeholder".to_string(),
                warnings: Vec::new(),
            },
            events: vec![TraceContributionEvent {
                event_id: Uuid::new_v4(),
                parent_event_id: None,
                event_type: TraceContributionEventType::UserMessage,
                timestamp: now,
                redacted_content: Some(content.to_string()),
                structured_payload: Value::Null,
                tool_name: None,
                tool_category: None,
                tool_call_id: None,
                latency_ms: None,
                token_counts: None,
                cost_usd: None,
                success: None,
                failure_modes: Vec::new(),
                side_effect: SideEffectLevel::None,
            }],
            outcome: OutcomeMetadata::default(),
            replay: ReplayMetadata {
                replayable: false,
                required_tools: Vec::new(),
                tool_manifest_hashes: BTreeMap::new(),
                expected_assertions: Vec::new(),
                replay_notes: Vec::new(),
            },
            embedding_analysis: None,
            value: ValueMetadata::default(),
            trace_card: TraceCard::default(),
            value_card: TraceValueCard::default(),
            hindsight: None,
            training_dynamics: None,
            process_evaluation: None,
        }
    }

    #[cfg(feature = "near-ai-privacy-filter")]
    #[tokio::test]
    async fn backstop_reredacts_prose_and_marks_pipeline() {
        use crate::trace_contribution::*;
        struct Stub;
        #[async_trait::async_trait]
        impl PrivacyFilterAdapter for Stub {
            async fn redact_text(
                &self,
                text: &str,
            ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
                if text.contains("jane@example.com") {
                    // Mirrors NearAiPrivacyFilterAdapter::apply_spans: report.counts
                    // uses the "privacy_filter:{label}" key while summary.by_label
                    // uses the bare label for the SAME span, as two parallel tallies.
                    let mut report = RedactionReport::default();
                    report.increment("privacy_filter:private_email");
                    report.add_pii_label("private_email");
                    Ok(Some(SafePrivacyFilterRedaction {
                        redacted_text: text.replace("jane@example.com", "[REDACTED:private_email]"),
                        summary: SafePrivacyFilterSummary {
                            schema_version: 1,
                            output_mode: "redacted_text_only".into(),
                            span_count: 1,
                            by_label: std::collections::BTreeMap::from([(
                                "private_email".into(),
                                1,
                            )]),
                            decoded_mismatch: false,
                        },
                        report,
                    }))
                } else {
                    Ok(None)
                }
            }
        }
        let mut env = sample_envelope_with_event_content("email jane@example.com now");
        rescrub_envelope_prose_pii_with(&Stub, &mut env)
            .await
            .unwrap();
        assert!(
            env.events[0]
                .redacted_content
                .as_deref()
                .unwrap()
                .contains("[REDACTED:private_email]")
        );
        assert!(
            env.privacy
                .redaction_pipeline_version
                .contains("near-ai-pii-backstop-v1")
        );
        assert!(
            env.privacy
                .pii_labels_present
                .iter()
                .any(|l| l == "private_email")
        );
        // report.counts (report-keyed) must be folded into redaction_counts exactly
        // once, not doubled by also folding summary.by_label (which counts the same
        // span under the bare label).
        assert_eq!(
            env.privacy
                .redaction_counts
                .get("privacy_filter:private_email")
                .copied(),
            Some(1)
        );
        assert_eq!(env.privacy.redaction_counts.get("private_email"), None);
        // The summary itself must still be preserved, disjoint from redaction_counts.
        let summary = env.privacy.privacy_filter_summary.as_ref().unwrap();
        assert_eq!(summary.by_label.get("private_email").copied(), Some(1));
        // Idempotent suffix: running again does not double-append.
        rescrub_envelope_prose_pii_with(&Stub, &mut env)
            .await
            .unwrap();
        assert_eq!(
            env.privacy
                .redaction_pipeline_version
                .matches("near-ai-pii-backstop-v1")
                .count(),
            1
        );
        // Second pass finds no more "jane@example.com" (already redacted), so the
        // Stub returns None and the count stays at 1 — never doubled by summary
        // folding on either pass.
        assert_eq!(
            env.privacy
                .redaction_counts
                .get("privacy_filter:private_email")
                .copied(),
            Some(1)
        );
    }

    /// Builds a JSON value nested `depth` levels deep, used to trip the
    /// residual scan's own depth budget without going anywhere near
    /// `serde_json`'s recursion limit (which the old code relied on
    /// implicitly - see the depth-budget finding this closes).
    #[cfg(feature = "near-ai-privacy-filter")]
    fn deeply_nested_json(depth: usize) -> serde_json::Value {
        let mut value = serde_json::json!("deep leaf");
        for _ in 0..depth {
            value = serde_json::json!([value]);
        }
        value
    }

    /// Test 7 (required test list): when the residual scan itself cannot
    /// complete - here, a depth budget overrun rather than a literal
    /// serialization error, since `serde_json::to_value` silently maps
    /// non-finite floats to `null` instead of erroring in this version -
    /// the pass must force High rather than silently reporting clean. This
    /// exercises the exact code path the serialization-failure branch
    /// shares: `residual_envelope_scan` returning `Err`, not `Ok(default)`.
    #[cfg(feature = "near-ai-privacy-filter")]
    #[test]
    fn residual_scan_failure_forces_high_risk() {
        use crate::trace_contribution::*;
        let mut env = sample_envelope_with_event_content("please list the files");
        env.privacy.residual_pii_risk = ResidualPiiRisk::Low;
        // Nested well past RESIDUAL_SCAN_MAX_DEPTH, in a field the async
        // structured-payload classifier never touches, so this isolates
        // the residual-scan budget specifically.
        env.replay
            .expected_assertions
            .push(deeply_nested_json(RESIDUAL_SCAN_MAX_DEPTH + 8));
        let redactor = DeterministicTraceRedactor::new(vec![]).unwrap();

        // Sanity: confirm the scan itself actually fails on this fixture,
        // otherwise the test would vacuously pass for the wrong reason.
        assert!(residual_envelope_scan(&redactor, &env).is_err());

        rescrub_trace_envelope_with(&redactor, &mut env);

        assert_eq!(
            env.privacy.residual_pii_risk,
            ResidualPiiRisk::High,
            "a residual scan that cannot complete must force High"
        );
    }

    #[cfg(feature = "near-ai-privacy-filter")]
    #[tokio::test]
    async fn residual_scan_failure_forces_high_in_async_backstop() {
        use crate::trace_contribution::*;
        struct NeverFindsAnything;
        #[async_trait::async_trait]
        impl PrivacyFilterAdapter for NeverFindsAnything {
            async fn redact_text(
                &self,
                _text: &str,
            ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
                Ok(None)
            }
        }
        let mut env = sample_envelope_with_event_content("please list the files");
        env.privacy.residual_pii_risk = ResidualPiiRisk::Low;
        // Same isolation as the sync test above: nested in a field the
        // structured-payload classifier never visits, so only the residual
        // scan's own budget is exercised.
        env.replay
            .expected_assertions
            .push(deeply_nested_json(RESIDUAL_SCAN_MAX_DEPTH + 8));

        rescrub_envelope_prose_pii_with(&NeverFindsAnything, &mut env)
            .await
            .unwrap();

        assert_eq!(
            env.privacy.residual_pii_risk,
            ResidualPiiRisk::High,
            "async backstop must force High when its residual scan cannot complete"
        );
    }

    /// Test 3 (required test list): a classifier response that reports
    /// "zero spans found" for everything it touched must NOT, by itself, be
    /// trusted enough to downgrade a HIGH prior risk. Without corroborating
    /// evidence (a healthy canary round-trip), the assessment stays
    /// incomplete and the prior risk is preserved.
    #[cfg(feature = "near-ai-privacy-filter")]
    #[tokio::test]
    async fn zero_span_classifier_response_cannot_lower_high_risk() {
        use crate::trace_contribution::*;
        struct AlwaysEmptySpans;
        #[async_trait::async_trait]
        impl PrivacyFilterAdapter for AlwaysEmptySpans {
            async fn redact_text(
                &self,
                text: &str,
            ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
                // Mirrors `{"data":[{"spans":[]}]}`: a real 200 response
                // that found nothing, on both the real field AND the
                // canary probe text. Text is returned unchanged.
                Ok(Some(SafePrivacyFilterRedaction {
                    redacted_text: text.to_string(),
                    summary: SafePrivacyFilterSummary {
                        schema_version: 1,
                        output_mode: "redacted_text_only".into(),
                        span_count: 0,
                        by_label: std::collections::BTreeMap::new(),
                        decoded_mismatch: false,
                    },
                    report: RedactionReport::default(),
                }))
            }
        }
        let mut env = sample_envelope_with_event_content("please list the files");
        env.privacy.residual_pii_risk = ResidualPiiRisk::High;
        env.consent.message_text_included = false;
        env.consent.tool_payloads_included = false;

        rescrub_envelope_prose_pii_with(&AlwaysEmptySpans, &mut env)
            .await
            .unwrap();

        assert_eq!(
            env.privacy.residual_pii_risk,
            ResidualPiiRisk::High,
            "a zero-span response must not, by itself, downgrade a HIGH prior risk"
        );
    }

    /// Test 4 (required test list): a classifier that returns `None` for
    /// every field (no redaction ever produced, i.e. no result at all, not
    /// even an explicit empty-spans response) must not be able to lower a
    /// HIGH prior risk either. Same fail-closed floor as the zero-span case.
    #[cfg(feature = "near-ai-privacy-filter")]
    #[tokio::test]
    async fn missing_classifier_result_cannot_lower_high_risk() {
        use crate::trace_contribution::*;
        struct NeverFindsAnything;
        #[async_trait::async_trait]
        impl PrivacyFilterAdapter for NeverFindsAnything {
            async fn redact_text(
                &self,
                _text: &str,
            ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
                Ok(None)
            }
        }
        let mut env = sample_envelope_with_event_content("please list the files");
        env.privacy.residual_pii_risk = ResidualPiiRisk::High;
        env.consent.message_text_included = false;
        env.consent.tool_payloads_included = false;

        rescrub_envelope_prose_pii_with(&NeverFindsAnything, &mut env)
            .await
            .unwrap();

        assert_eq!(
            env.privacy.residual_pii_risk,
            ResidualPiiRisk::High,
            "a classifier producing no result at all must not downgrade a HIGH prior risk"
        );
    }

    /// A classifier that passes the canary but finds nothing in real content
    /// must NOT downgrade a HIGH prior risk, even with complete coverage and
    /// no budget overrun. Under the previous rule (healthy canary => trust the
    /// emptiness) this case downgraded; findings are now the only evidence.
    #[cfg(feature = "near-ai-privacy-filter")]
    #[tokio::test]
    async fn canary_healthy_but_no_findings_cannot_lower_high_risk() {
        use crate::trace_contribution::*;
        struct CanaryHealthyButFindsNoRealPii;
        #[async_trait::async_trait]
        impl PrivacyFilterAdapter for CanaryHealthyButFindsNoRealPii {
            async fn redact_text(
                &self,
                text: &str,
            ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
                let canary_values = synthetic_privacy_filter_canary_values();
                if !canary_values.iter().any(|v| text.contains(v.as_str())) {
                    return Ok(None);
                }
                let mut redacted = text.to_string();
                let mut report = RedactionReport::default();
                for v in &canary_values {
                    if redacted.contains(v.as_str()) {
                        redacted = redacted.replace(v.as_str(), "[REDACTED:unknown]");
                        report.increment("privacy_filter:unknown");
                        report.add_pii_label("unknown");
                    }
                }
                Ok(Some(SafePrivacyFilterRedaction {
                    redacted_text: redacted,
                    summary: SafePrivacyFilterSummary {
                        schema_version: 1,
                        output_mode: "redacted_text_only".into(),
                        span_count: canary_values.len() as u32,
                        by_label: std::collections::BTreeMap::new(),
                        decoded_mismatch: false,
                    },
                    report,
                }))
            }
        }

        // Ordinary content, well inside every budget, so coverage is
        // complete: the only thing between this and a downgrade is whether a
        // zero-finding pass counts as evidence.
        let mut env = sample_envelope_with_event_content("please list the files");
        env.privacy.residual_pii_risk = ResidualPiiRisk::High;
        env.consent.message_text_included = false;
        env.consent.tool_payloads_included = false;

        rescrub_envelope_prose_pii_with(&CanaryHealthyButFindsNoRealPii, &mut env)
            .await
            .unwrap();

        assert_eq!(
            env.privacy.residual_pii_risk,
            ResidualPiiRisk::High,
            "a healthy canary is liveness, not proof the content is clean"
        );
    }

    /// Test 5 (required test list): exceeding the structured-payload budget
    /// (depth, in this case) must mark coverage incomplete and refuse to
    /// downgrade, even though nothing PII-shaped was ever found.
    #[cfg(feature = "near-ai-privacy-filter")]
    #[tokio::test]
    async fn structured_payload_budget_overrun_cannot_lower_high_risk() {
        use crate::trace_contribution::*;
        // A canary-aware stub: it DOES redact the synthetic canary probe
        // (so a canary round-trip reports healthy) but never finds
        // anything in real content. This isolates the budget/coverage gate
        // from the zero-finding/canary gate (Task 2) - without a
        // canary-aware stub, `NeverFindsAnything` would also fail the
        // canary check on its own and the test would not distinguish
        // "coverage incomplete" from "classifier result not useful".
        struct CanaryHealthyButFindsNoRealPii;
        #[async_trait::async_trait]
        impl PrivacyFilterAdapter for CanaryHealthyButFindsNoRealPii {
            async fn redact_text(
                &self,
                text: &str,
            ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
                let canary_values = synthetic_privacy_filter_canary_values();
                if !canary_values.iter().any(|v| text.contains(v.as_str())) {
                    return Ok(None);
                }
                let mut redacted = text.to_string();
                let mut report = RedactionReport::default();
                for v in &canary_values {
                    if redacted.contains(v.as_str()) {
                        redacted = redacted.replace(v.as_str(), "[REDACTED:unknown]");
                        report.increment("privacy_filter:unknown");
                        report.add_pii_label("unknown");
                    }
                }
                Ok(Some(SafePrivacyFilterRedaction {
                    redacted_text: redacted,
                    summary: SafePrivacyFilterSummary {
                        schema_version: 1,
                        output_mode: "redacted_text_only".into(),
                        span_count: canary_values.len() as u32,
                        by_label: std::collections::BTreeMap::new(),
                        decoded_mismatch: false,
                    },
                    report,
                }))
            }
        }
        let mut env = sample_envelope_with_event_content("please list the files");
        env.privacy.residual_pii_risk = ResidualPiiRisk::High;
        env.consent.message_text_included = false;
        env.consent.tool_payloads_included = false;

        // Nest well past STRUCTURED_PAYLOAD_MAX_DEPTH so the budget trips
        // before any leaf is even reached.
        let mut nested = serde_json::json!("deep leaf");
        for _ in 0..(STRUCTURED_PAYLOAD_MAX_DEPTH + 4) {
            nested = serde_json::json!([nested]);
        }
        env.events[0].structured_payload = nested;

        rescrub_envelope_prose_pii_with(&CanaryHealthyButFindsNoRealPii, &mut env)
            .await
            .unwrap();

        assert_eq!(
            env.privacy.residual_pii_risk,
            ResidualPiiRisk::High,
            "a structured-payload budget overrun must preserve, not lower, the prior risk"
        );
    }

    /// Prose fields the scrub pass never submits to the classifier
    /// (`replay.replay_notes` here) must block a downgrade when they carry
    /// text. The classifier below DOES find real PII, so
    /// `useful_classifier_result` is true and the structured budget is
    /// untouched - coverage is the only thing standing between this envelope
    /// and a downgrade.
    ///
    /// The note's PII is prose ("their mother Mary in Baltimore"), not a
    /// patterned secret, so the deterministic residual scan cannot see it
    /// either. Without the coverage gate it would ride through untouched
    /// under a lowered risk label.
    #[cfg(feature = "near-ai-privacy-filter")]
    #[tokio::test]
    async fn uncovered_prose_field_blocks_downgrade() {
        use crate::trace_contribution::*;
        struct FindsRealEmail;
        #[async_trait::async_trait]
        impl PrivacyFilterAdapter for FindsRealEmail {
            async fn redact_text(
                &self,
                text: &str,
            ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
                if !text.contains("ada@example.com") {
                    return Ok(None);
                }
                let mut report = RedactionReport::default();
                report.increment("privacy_filter:private_email");
                report.add_pii_label("private_email");
                Ok(Some(SafePrivacyFilterRedaction {
                    redacted_text: text.replace("ada@example.com", "[REDACTED:private_email]"),
                    summary: SafePrivacyFilterSummary {
                        schema_version: 1,
                        output_mode: "redacted_text_only".into(),
                        span_count: 1,
                        by_label: std::collections::BTreeMap::new(),
                        decoded_mismatch: false,
                    },
                    report,
                }))
            }
        }

        let build = || {
            let mut env = sample_envelope_with_event_content("mail ada@example.com");
            env.privacy.residual_pii_risk = ResidualPiiRisk::High;
            env.consent.message_text_included = false;
            env.consent.tool_payloads_included = false;
            env
        };

        // Control: identical envelope with no uncovered prose. This must
        // downgrade, which is what makes the assertion below non-vacuous -
        // the only difference between the two cases is the note.
        let mut clean = build();
        clean.replay.replay_notes.clear();
        rescrub_envelope_prose_pii_with(&FindsRealEmail, &mut clean)
            .await
            .unwrap();
        assert_ne!(
            clean.privacy.residual_pii_risk,
            ResidualPiiRisk::High,
            "control: with full coverage and real findings this envelope downgrades"
        );

        let mut gapped = build();
        gapped.replay.replay_notes =
            vec!["they asked about their mother Mary in Baltimore".to_string()];
        rescrub_envelope_prose_pii_with(&FindsRealEmail, &mut gapped)
            .await
            .unwrap();
        assert_eq!(
            gapped.privacy.residual_pii_risk,
            ResidualPiiRisk::High,
            "prose in a field the classifier never saw must block the downgrade"
        );
    }

    /// Test 6 (required test list): string leaves AND object keys inside
    /// `structured_payload` must reach the classifier. A leaf finding gets
    /// redacted in place; a KEY finding cannot be safely rewritten (collision
    /// risk), so it forces High instead.
    #[cfg(feature = "near-ai-privacy-filter")]
    #[tokio::test]
    async fn structured_payload_leaves_and_keys_reach_classifier() {
        use crate::trace_contribution::*;
        struct DetectsEmail;
        #[async_trait::async_trait]
        impl PrivacyFilterAdapter for DetectsEmail {
            async fn redact_text(
                &self,
                text: &str,
            ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
                if text.contains("alice@example.com") {
                    let mut report = RedactionReport::default();
                    report.increment("privacy_filter:private_email");
                    report.add_pii_label("private_email");
                    Ok(Some(SafePrivacyFilterRedaction {
                        redacted_text: text
                            .replace("alice@example.com", "[REDACTED:private_email]"),
                        summary: SafePrivacyFilterSummary {
                            schema_version: 1,
                            output_mode: "redacted_text_only".into(),
                            span_count: 1,
                            by_label: std::collections::BTreeMap::from([(
                                "private_email".into(),
                                1,
                            )]),
                            decoded_mismatch: false,
                        },
                        report,
                    }))
                } else {
                    Ok(None)
                }
            }
        }
        let mut env = sample_envelope_with_event_content("no prose PII here");
        env.privacy.residual_pii_risk = ResidualPiiRisk::Medium;
        env.events[0].structured_payload = serde_json::json!({
            "reviewer_alice@example.com": "argument value with alice@example.com inside",
        });

        rescrub_envelope_prose_pii_with(&DetectsEmail, &mut env)
            .await
            .unwrap();

        let payload = &env.events[0].structured_payload;
        let value_text = payload
            .get("reviewer_alice@example.com")
            .and_then(Value::as_str)
            .expect("key is left unrewritten by design");
        assert!(
            value_text.contains("[REDACTED:private_email]"),
            "structured payload string leaf must reach the classifier and be redacted: {value_text}"
        );
        assert!(
            !value_text.contains("alice@example.com"),
            "the raw email must not survive in the leaf value: {value_text}"
        );
        // The key itself is a finding too (classifier flags it), but per the
        // documented design choice it is not rewritten - it forces High
        // instead of being silently dropped or losing sibling data to a
        // rewrite collision.
        assert_eq!(
            env.privacy.residual_pii_risk,
            ResidualPiiRisk::High,
            "a classifier finding on an object KEY must force High"
        );
    }

    fn bare_envelope() -> super::TraceContributionEnvelope {
        use super::*;
        let now = Utc::now();
        TraceContributionEnvelope {
            schema_version: TRACE_CONTRIBUTION_SCHEMA_VERSION.to_string(),
            trace_id: Uuid::new_v4(),
            submission_id: Uuid::new_v4(),
            created_at: now,
            ironclaw: IronclawTraceMetadata {
                version: "1".to_string(),
                engine_version: None,
                feature_flags: BTreeMap::new(),
                channel: TraceChannel::Cli,
                model_name: None,
            },
            consent: ConsentMetadata {
                policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
                scopes: vec![ConsentScope::DebuggingEvaluation],
                message_text_included: false,
                tool_payloads_included: false,
                revocable: true,
            },
            contributor: ContributorMetadata {
                pseudonymous_contributor_id: Some("sha256:contributor".to_string()),
                tenant_scope_ref: None,
                credit_account_ref: None,
                revocation_handle: Uuid::new_v4(),
            },
            privacy: PrivacyMetadata {
                redaction_pipeline_version: DETERMINISTIC_REDACTION_PIPELINE_VERSION.to_string(),
                redaction_counts: BTreeMap::new(),
                privacy_filter_summary: None,
                pii_labels_present: Vec::new(),
                residual_pii_risk: ResidualPiiRisk::Low,
                redaction_hash: "sha256:placeholder".to_string(),
                warnings: Vec::new(),
            },
            events: Vec::new(),
            outcome: OutcomeMetadata::default(),
            replay: ReplayMetadata {
                replayable: false,
                required_tools: Vec::new(),
                tool_manifest_hashes: BTreeMap::new(),
                expected_assertions: Vec::new(),
                replay_notes: Vec::new(),
            },
            embedding_analysis: None,
            value: ValueMetadata::default(),
            trace_card: TraceCard::default(),
            value_card: TraceValueCard::default(),
            hindsight: None,
            training_dynamics: None,
            process_evaluation: None,
        }
    }

    fn message_event(content: &str) -> super::TraceContributionEvent {
        use super::*;
        TraceContributionEvent {
            event_id: Uuid::new_v4(),
            parent_event_id: None,
            event_type: TraceContributionEventType::UserMessage,
            timestamp: Utc::now(),
            redacted_content: Some(content.to_string()),
            structured_payload: Value::Null,
            tool_name: None,
            tool_category: None,
            tool_call_id: None,
            latency_ms: None,
            token_counts: None,
            cost_usd: None,
            success: None,
            failure_modes: Vec::new(),
            side_effect: SideEffectLevel::None,
        }
    }

    #[test]
    fn reconcile_consent_raises_message_text_for_prose_events() {
        use super::*;
        let mut envelope = bare_envelope();
        envelope
            .events
            .push(message_event("Project Vega acquisition closes Friday"));

        let presence = reconcile_consent_declarations(&mut envelope);

        assert!(presence.message_text);
        assert!(!presence.tool_payloads);
        assert!(envelope.consent.message_text_included);
        assert!(!envelope.consent.tool_payloads_included);
        assert!(
            envelope
                .privacy
                .warnings
                .iter()
                .any(|w| w.contains("under-reported consent"))
        );
    }

    #[test]
    fn reconcile_consent_raises_tool_payloads_for_structured_payload_alone() {
        use super::*;
        let mut envelope = bare_envelope();
        envelope.events.push(TraceContributionEvent {
            event_id: Uuid::new_v4(),
            parent_event_id: None,
            event_type: TraceContributionEventType::AssistantMessage,
            timestamp: Utc::now(),
            redacted_content: None,
            structured_payload: serde_json::json!({"command": "ls"}),
            tool_name: None,
            tool_category: None,
            tool_call_id: None,
            latency_ms: None,
            token_counts: None,
            cost_usd: None,
            success: None,
            failure_modes: Vec::new(),
            side_effect: SideEffectLevel::None,
        });

        let presence = reconcile_consent_declarations(&mut envelope);

        assert!(!presence.message_text);
        assert!(presence.tool_payloads);
        assert!(envelope.consent.tool_payloads_included);
    }

    #[test]
    fn reconcile_consent_raises_tool_payloads_for_a_structured_payload() {
        use super::*;
        let mut envelope = bare_envelope();
        envelope.events.push(TraceContributionEvent {
            event_id: Uuid::new_v4(),
            parent_event_id: None,
            event_type: TraceContributionEventType::ToolCall,
            timestamp: Utc::now(),
            redacted_content: None,
            structured_payload: serde_json::json!({"command": "ls -la"}),
            tool_name: Some("Bash".to_string()),
            tool_category: Some("shell".to_string()),
            tool_call_id: None,
            latency_ms: None,
            token_counts: None,
            cost_usd: None,
            success: None,
            failure_modes: Vec::new(),
            side_effect: SideEffectLevel::None,
        });

        let presence = reconcile_consent_declarations(&mut envelope);

        assert!(!presence.message_text);
        assert!(presence.tool_payloads);
        assert!(!envelope.consent.message_text_included);
        assert!(envelope.consent.tool_payloads_included);
    }

    /// A tool name without a payload must NOT be corrected upward.
    ///
    /// Stripping payloads while keeping tool names is a supported privacy
    /// mode -- it is what keeps a trace structurally trainable when content is
    /// absent. Raising the flag here would push every structure-preserved
    /// trace to Medium residual risk and quarantine it on a default
    /// deployment, for payloads it does not carry.
    ///
    /// The contributor client makes the same call when it builds the
    /// declaration. The two halves must agree, or a client that declares
    /// honestly gets corrected upward anyway and is penalised for it.
    #[test]
    fn reconcile_consent_leaves_a_bare_tool_name_alone() {
        use super::*;
        let mut envelope = bare_envelope();
        envelope.events.push(TraceContributionEvent {
            event_id: Uuid::new_v4(),
            parent_event_id: None,
            event_type: TraceContributionEventType::ToolCall,
            timestamp: Utc::now(),
            redacted_content: None,
            structured_payload: Value::Null,
            tool_name: Some("Bash".to_string()),
            tool_category: Some("shell".to_string()),
            tool_call_id: None,
            latency_ms: None,
            token_counts: None,
            cost_usd: None,
            success: None,
            failure_modes: Vec::new(),
            side_effect: SideEffectLevel::None,
        });

        let presence = reconcile_consent_declarations(&mut envelope);

        assert!(!presence.tool_payloads, "a tool name is not a tool payload");
        assert!(
            !envelope.consent.tool_payloads_included,
            "an honest false declaration must survive reconciliation"
        );
        assert!(!presence.message_text);
    }

    #[test]
    fn reconcile_consent_raises_message_text_for_human_correction() {
        use super::*;
        let mut envelope = bare_envelope();
        envelope.outcome.human_correction = Some("use the other API key".to_string());

        let presence = reconcile_consent_declarations(&mut envelope);

        assert!(presence.message_text);
        assert!(envelope.consent.message_text_included);
    }

    #[test]
    fn reconcile_consent_never_lowers_over_reported_flags() {
        use super::*;
        let mut envelope = bare_envelope();
        envelope.consent.message_text_included = true;
        envelope.consent.tool_payloads_included = true;

        let presence = reconcile_consent_declarations(&mut envelope);

        assert!(!presence.message_text);
        assert!(!presence.tool_payloads);
        assert!(envelope.consent.message_text_included);
        assert!(envelope.consent.tool_payloads_included);
        assert!(envelope.privacy.warnings.is_empty());
    }

    #[test]
    fn empty_content_does_not_force_consent_flags() {
        use super::*;
        let mut envelope = bare_envelope();
        envelope.events.push(message_event(""));
        envelope.events.push(TraceContributionEvent {
            event_id: Uuid::new_v4(),
            parent_event_id: None,
            event_type: TraceContributionEventType::ToolResult,
            timestamp: Utc::now(),
            redacted_content: Some(String::new()),
            structured_payload: Value::Null,
            tool_name: Some(String::new()),
            tool_category: None,
            tool_call_id: None,
            latency_ms: None,
            token_counts: None,
            cost_usd: None,
            success: None,
            failure_modes: Vec::new(),
            side_effect: SideEffectLevel::None,
        });

        let presence = reconcile_consent_declarations(&mut envelope);

        assert!(!presence.message_text);
        assert!(!presence.tool_payloads);
        assert!(!envelope.consent.message_text_included);
        assert!(!envelope.consent.tool_payloads_included);
    }

    #[test]
    fn rescrub_raises_risk_for_clean_prose_under_reported_as_low() {
        use super::*;
        // The reproduction from issue #208: ordinary prose matches no
        // deterministic detector, consent says false/false, and without
        // concordance residual_risk would stay Low → Accepted.
        let mut envelope = bare_envelope();
        envelope
            .events
            .push(message_event("Project Vega acquisition closes Friday"));
        assert_eq!(envelope.privacy.residual_pii_risk, ResidualPiiRisk::Low);
        assert!(!envelope.consent.message_text_included);

        rescrub_trace_envelope(&mut envelope).expect("rescrub succeeds");

        assert!(
            envelope.consent.message_text_included,
            "server must correct the under-reported message-text declaration"
        );
        assert_eq!(
            envelope.privacy.residual_pii_risk,
            ResidualPiiRisk::Medium,
            "content-bearing prose must not stay Low after concordance"
        );
    }

    fn deterministic_only_constructor_ignores_inherited_backend() {
        use super::DeterministicTraceRedactor;

        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: ENV_LOCK serializes process-environment mutation across
        // every env-touching test in this crate.
        unsafe {
            std::env::set_var("TRACE_PRIVACY_FILTER_BACKEND", "garbage");
        }
        let redactor =
            DeterministicTraceRedactor::deterministic_only(vec!["/Users/preview/private".into()]);
        let has_filter = redactor.attached_privacy_filter().is_some();
        let (redacted, _) = redactor.redact_text("open /Users/preview/private/file.txt");
        unsafe {
            std::env::remove_var("TRACE_PRIVACY_FILTER_BACKEND");
        }
        assert!(!has_filter);
        assert!(!redacted.contains("/Users/preview/private"));
    }

    fn contextual_entropy_applies_cued_secret_shape_decisions() {
        use super::*;
        let r = DeterministicTraceRedactor::new(vec![]).unwrap();

        // Row 1 — Accept, documented. No separator means no boundary without
        // splitting inside arbitrary identifiers; FP cost is too high.
        let secret = "Zx9Qk2Lm7Pv4Rt8Wy1Nb6Hd3Fg5Jc0Ae";
        for text in [format!("api_key{secret}"), format!("Bearer{secret}")] {
            let (out, _) = r.redact_text(&text);
            assert!(
                out.contains(&secret[..secret.len().min(8)]),
                "zero-separator glue was unexpectedly caught (boundary moved, update this test \
                 and the comment on contextual_entropy_secret_ranges): {out}"
            );
        }

        // Row 2 — Redact. Cue + short opaque value; floor is 8, not 16.
        let short = "Q7vM2xP9sL4nR8k"; // 15
        assert!(short.len() < 16 && short.len() >= ENTROPY_MIN_LEN);
        for text in [format!("api_key: {short}"), format!("api_key={short}")] {
            let (out, rep) = r.redact_text(&text);
            assert!(!out.contains(short), "short cued secret survived: {out}");
            assert!(rep.blocked_secret_detected);
        }

        // Row 3 — Keep allowlisted. ~105k structural IDs vs ~20 real secrets.
        let (out, _) = r.redact_text("api_key=550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(out, "api_key=550e8400-e29b-41d4-a716-446655440000");

        // Row 4 — Redact when cued. Hex allowlist narrowed to the uncued case.
        let hex40 = "0123456789abcdef0123456789abcdef01234567";
        let hex64 = "a1b2c3d4e5f6789012345678abcdef0123456789abcdef0123456789abcdef01";
        for text in [
            format!("secret={hex40}"),
            format!("api_key: {hex64}"),
            format!("api_key={hex64}"),
        ] {
            let (out, rep) = r.redact_text(&text);
            assert_ne!(out, text, "cued content-hash-shaped secret survived: {out}");
            assert!(rep.blocked_secret_detected);
        }

        // Sub-threshold entropy — still not opaque enough, even when cued.
        let (out, _) = r.redact_text("api_key=aaaaaaaaaaaaaaaaaaaa");
        assert_eq!(out, "api_key=aaaaaaaaaaaaaaaaaaaa");
    }

    fn contextual_entropy_fp_budget_for_cued_shape_changes() {
        use super::*;
        let r = DeterministicTraceRedactor::bare();

        // Uncued content hashes / shas must still survive (row 4 narrows the
        // allowlist to the *cued* case only; uncued path never reached it).
        let sha40 = "0123456789abcdef0123456789abcdef01234567";
        let sha64 = "a1b2c3d4e5f6789012345678abcdef0123456789abcdef0123456789abcdef01";
        for text in [
            format!("commit {sha40}"),
            format!("digest {sha64}"),
            format!("blob {sha64} verified"),
        ] {
            let (out, rep) = r.redact_text(&text);
            assert_eq!(out, text, "uncued hash was redacted: {out}");
            assert!(!rep.blocked_secret_detected);
        }

        // UUID stays allowlisted even when cued (row 3).
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        for text in [format!("token: {uuid}"), format!("api_key={uuid}")] {
            let (out, _) = r.redact_text(&text);
            assert!(out.contains(uuid), "cued UUID was redacted: {out}");
        }

        // Prefixed structural IDs stay allowlisted even when cued.
        let (out, _) = r.redact_text("token: msg_01ABCDEFghijklmnopqrstuvwx");
        assert!(out.contains("msg_01ABCDEFghijklmnopqrstuvwx"));

        // Git short SHAs (7–8 hex) stay allowlisted even when cued — the FP
        // rate on `api_key: deadbeef`-style short hex dominates recall here.
        for sha in ["deadbee", "deadbeef"] {
            let (out, rep) = r.redact_text(&format!("api_key: {sha}"));
            assert!(
                out.contains(sha),
                "short git sha was redacted (FP budget): {out}"
            );
            assert!(!rep.blocked_secret_detected);
        }

        // Low-entropy short values after a cue must survive (row 2 lowers
        // length, not the entropy floor).
        for text in [
            "password: password",
            "api_key: staging1",
            "token: aaaaaaaa",
            "secret: none1234",
        ] {
            let (out, rep) = r.redact_text(text);
            assert_eq!(out, text, "low-entropy cued value was redacted: {out}");
            assert!(!rep.blocked_secret_detected);
        }

        // Sub-floor length even with a cue and high opacity — still too short.
        assert!("Zx9Qk2L".len() < ENTROPY_MIN_LEN);
        let (out, _) = r.redact_text("api_key=Zx9Qk2L");
        assert_eq!(out, "api_key=Zx9Qk2L");

        // Uncued short opaque tokens must survive (candidate class is wider
        // now, but the cue gate is the FP control).
        let short = "Q7vM2xP9sL4nR8k";
        let (out, rep) = r.redact_text(&format!("the cursor {short} appears here"));
        assert!(
            out.contains(short),
            "uncued short opaque was redacted: {out}"
        );
        assert!(!rep.blocked_secret_detected);
    }

    #[test]
    fn successfully_redacted_secret_is_medium_not_high() {
        use super::*;

        let report = RedactionReport {
            counts: BTreeMap::from([
                ("secret".to_string(), 1),
                ("secret:openai_api_key".to_string(), 1),
            ]),
            blocked_secret_detected: true,
            ..Default::default()
        };

        let consent = ConsentMetadata {
            policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
            scopes: vec![ConsentScope::DebuggingEvaluation],
            message_text_included: false,
            tool_payloads_included: false,
            revocable: true,
        };

        assert_eq!(
            residual_risk(&consent, &report),
            ResidualPiiRisk::Medium,
            "successful secret scrub must be Medium, not terminal High"
        );
    }

    #[test]
    fn unredactable_key_finding_still_forces_high() {
        use super::*;

        let report = RedactionReport {
            key_finding_detected: true,
            ..Default::default()
        };

        let consent = ConsentMetadata {
            policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
            scopes: vec![ConsentScope::DebuggingEvaluation],
            message_text_included: false,
            tool_payloads_included: false,
            revocable: true,
        };

        assert_eq!(
            residual_risk(&consent, &report),
            ResidualPiiRisk::High,
            "unredactable key findings must stay High"
        );
    }

    #[test]
    fn residual_secret_hit_still_forces_high() {
        use super::*;

        let findings = RedactionReport {
            counts: BTreeMap::from([("secret".to_string(), 1)]),
            blocked_secret_detected: true,
            ..Default::default()
        };

        let residual_findings = RedactionReport {
            counts: BTreeMap::from([("secret".to_string(), 1)]),
            blocked_secret_detected: true,
            ..Default::default()
        };

        let assessment = PostScrubAssessment {
            complete_coverage: true,
            useful_classifier_result: true,
            findings,
            residual_findings,
        };

        assert_eq!(
            resolve_post_scrub_risk(
                ResidualPiiRisk::Medium,
                ResidualPiiRisk::Medium,
                &assessment
            ),
            ResidualPiiRisk::High,
            "a post-scrub residual secret hit must force High"
        );
    }

    /// Issue #373 case 4, at the classification rule itself: a pass whose
    /// configured filter never examined the text has no standing to report
    /// "clean", so an otherwise empty report must still be High.
    #[test]
    fn coverage_gap_alone_forces_high() {
        use super::*;

        let report = RedactionReport {
            coverage_incomplete: true,
            ..Default::default()
        };
        let consent = ConsentMetadata {
            policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
            scopes: vec![ConsentScope::DebuggingEvaluation],
            message_text_included: false,
            tool_payloads_included: false,
            revocable: true,
        };

        // Same consent + empty findings, minus the coverage gap, is Low.
        // That contrast is the point: the gap is doing the work.
        assert_eq!(
            residual_risk(&consent, &RedactionReport::default()),
            ResidualPiiRisk::Low
        );
        assert_eq!(
            residual_risk(&consent, &report),
            ResidualPiiRisk::High,
            "an unexamined field must fail closed, not report clean"
        );
    }

    /// A key finding is unresolvable by redaction, so nothing else in the
    /// report or the consent flags may talk it down - not a clean set of
    /// counts, not a fully-covered pass.
    #[test]
    fn key_finding_forces_high_regardless_of_everything_else() {
        use super::*;

        let consent = ConsentMetadata {
            policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
            scopes: vec![ConsentScope::DebuggingEvaluation],
            message_text_included: false,
            tool_payloads_included: false,
            revocable: true,
        };
        let report = RedactionReport {
            key_finding_detected: true,
            ..Default::default()
        };
        assert_eq!(residual_risk(&consent, &report), ResidualPiiRisk::High);

        // And through the post-scrub resolver, with every downgrade
        // precondition satisfied and a clean residual scan.
        let assessment = PostScrubAssessment {
            complete_coverage: true,
            useful_classifier_result: true,
            findings: RedactionReport {
                key_finding_detected: true,
                ..Default::default()
            },
            residual_findings: RedactionReport::default(),
        };
        assert_eq!(
            resolve_post_scrub_risk(ResidualPiiRisk::Low, ResidualPiiRisk::Low, &assessment),
            ResidualPiiRisk::High,
            "a key finding must survive an otherwise perfect reassessment"
        );
    }

    /// Issue #373 case 1, end to end through the pass that builds the
    /// envelope a contributor sends. A session that pasted an API key is the
    /// ordinary case, not the dangerous one: the key is gone, the telemetry
    /// records that it was found, and the tier is the found-and-removed
    /// floor rather than terminal High.
    #[tokio::test]
    async fn originating_scrub_of_a_secret_lands_medium_not_high() {
        use super::*;

        let secret = "sk-abcdefghijklmnopqrstuvwxyz012345";
        let started = Utc::now();
        let turn = RawTraceCaptureTurn {
            user_input: format!("this failed: export OPENAI_API_KEY={secret}"),
            response: Some("rotate that key".to_string()),
            tool_calls: Vec::new(),
            started_at: started,
            completed_at: Some(started + chrono::Duration::milliseconds(10)),
            state: Some("Completed".to_string()),
        };
        let raw = RawTraceContribution::from_capture_turns(
            &[turn],
            RecordedTraceContributionOptions {
                include_message_text: true,
                ..RecordedTraceContributionOptions::default()
            },
        );

        let envelope = DeterministicTraceRedactor::bare()
            .redact_trace(raw)
            .await
            .expect("deterministic redaction succeeds");

        let serialized = serde_json::to_string(&envelope).expect("envelope serializes");
        assert!(
            !serialized.contains(secret),
            "the secret must not survive the originating scrub"
        );
        assert!(
            envelope
                .privacy
                .redaction_counts
                .keys()
                .any(|key| key == "secret" || key.starts_with("secret:")),
            "the finding must still be recorded: {:?}",
            envelope.privacy.redaction_counts
        );
        assert_eq!(
            envelope.privacy.residual_pii_risk,
            ResidualPiiRisk::Medium,
            "found-and-removed is the Medium floor, not High"
        );
    }

    /// Issue #373 case 2, with a real survivor rather than a stubbed report.
    /// `replay.tool_manifest_hashes` is redacted by KEY only - the typed
    /// re-scrub traversal reinserts each value untouched - so a secret
    /// parked in a value is still in the stored envelope after the pass.
    /// That is exactly the "detect-then-redact bug, or a value the
    /// string-leaf pass never visited" the residual scan exists to catch,
    /// and ingest re-derives it itself rather than taking the client's word.
    #[test]
    fn a_secret_that_survives_the_rescrub_forces_high() {
        use super::*;

        let secret = "sk-abcdefghijklmnopqrstuvwxyz012345";
        let mut env = sample_envelope_with_event_content("please list the files");
        env.privacy.residual_pii_risk = ResidualPiiRisk::Low;
        env.replay.tool_manifest_hashes.insert(
            "some_tool".to_string(),
            format!("export OPENAI_API_KEY={secret}"),
        );

        let redactor = DeterministicTraceRedactor::bare();

        // Sanity: the typed pass really does leave this field alone, so the
        // test is exercising a survivor and not a redaction.
        rescrub_trace_envelope_with(&redactor, &mut env);
        let serialized = serde_json::to_string(&env).expect("envelope serializes");
        assert!(
            serialized.contains(secret),
            "fixture is wrong: the survivor was redacted, so nothing residual remains"
        );

        assert_eq!(
            env.privacy.residual_pii_risk,
            ResidualPiiRisk::High,
            "a secret still present after the pass must be High"
        );
    }

    /// Fail-closed for an older client. Such a client marks case 1 as High
    /// (the pre-#373 rule), and the server has no way to tell that apart
    /// from a genuine survivor, because nothing in the envelope records
    /// which it was. The re-scrub must therefore leave it High: absence of a
    /// recorded verdict is not evidence of a clean one, and the client's
    /// number is a floor the server may raise and never silently lower.
    #[test]
    fn a_high_from_an_older_client_is_not_downgraded_by_a_clean_rescrub() {
        use super::*;

        let mut env = sample_envelope_with_event_content("please list the files");
        env.privacy.residual_pii_risk = ResidualPiiRisk::High;

        let redactor = DeterministicTraceRedactor::bare();
        rescrub_trace_envelope_with(&redactor, &mut env);

        assert_eq!(
            env.privacy.residual_pii_risk,
            ResidualPiiRisk::High,
            "an unexplained prior High must not be downgraded by a silent client"
        );
    }

    /// The other side of that floor: a current client that reports Medium
    /// for a scrubbed session keeps Medium through the server pass. Without
    /// this the fix would not actually reach the corpus - the tier the
    /// operator flag acts on is the one the server stores.
    #[test]
    fn a_medium_from_a_current_client_survives_the_server_rescrub() {
        use super::*;

        let mut env = sample_envelope_with_event_content("please list the files");
        env.privacy.residual_pii_risk = ResidualPiiRisk::Medium;

        let redactor = DeterministicTraceRedactor::bare();
        rescrub_trace_envelope_with(&redactor, &mut env);

        assert_eq!(
            env.privacy.residual_pii_risk,
            ResidualPiiRisk::Medium,
            "a clean server pass must neither raise nor drop the reported tier"
        );
    }

    #[test]
    fn scrub_pass_secret_alone_does_not_block_downgrade_to_medium() {
        use super::*;

        let findings = RedactionReport {
            counts: BTreeMap::from([("secret".to_string(), 1)]),
            blocked_secret_detected: true,
            ..Default::default()
        };

        let assessment = PostScrubAssessment {
            complete_coverage: true,
            useful_classifier_result: true,
            findings,
            residual_findings: RedactionReport::default(),
        };

        assert!(
            assessment.can_downgrade(),
            "clean residual + complete coverage must allow downgrade"
        );
        assert_eq!(
            resolve_post_scrub_risk(ResidualPiiRisk::High, ResidualPiiRisk::Medium, &assessment),
            ResidualPiiRisk::Medium,
            "successful scrub must not pin High when residual scan is clean"
        );
    }

    fn scoring_envelope(risk: super::ResidualPiiRisk) -> super::TraceContributionEnvelope {
        use super::*;

        let now = Utc::now();
        TraceContributionEnvelope {
            schema_version: TRACE_CONTRIBUTION_SCHEMA_VERSION.to_string(),
            trace_id: Uuid::new_v4(),
            submission_id: Uuid::new_v4(),
            created_at: now,
            ironclaw: IronclawTraceMetadata {
                version: "1".to_string(),
                engine_version: None,
                feature_flags: BTreeMap::new(),
                channel: TraceChannel::Cli,
                model_name: None,
            },
            consent: ConsentMetadata {
                policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
                scopes: vec![ConsentScope::DebuggingEvaluation],
                message_text_included: true,
                tool_payloads_included: false,
                revocable: true,
            },
            contributor: ContributorMetadata {
                pseudonymous_contributor_id: None,
                tenant_scope_ref: None,
                credit_account_ref: None,
                revocation_handle: Uuid::new_v4(),
            },
            privacy: PrivacyMetadata {
                redaction_pipeline_version: DETERMINISTIC_REDACTION_PIPELINE_VERSION.to_string(),
                redaction_counts: BTreeMap::new(),
                privacy_filter_summary: None,
                pii_labels_present: Vec::new(),
                residual_pii_risk: risk,
                redaction_hash: "sha256:placeholder".to_string(),
                warnings: Vec::new(),
            },
            events: vec![TraceContributionEvent {
                event_id: Uuid::new_v4(),
                parent_event_id: None,
                event_type: TraceContributionEventType::UserMessage,
                timestamp: now,
                redacted_content: Some("ordinary work".to_string()),
                structured_payload: Value::Null,
                tool_name: None,
                tool_category: None,
                tool_call_id: None,
                latency_ms: None,
                token_counts: None,
                cost_usd: None,
                success: None,
                failure_modes: Vec::new(),
                side_effect: SideEffectLevel::None,
            }],
            outcome: OutcomeMetadata::default(),
            // replayable: false is the realistic case for a recorded session,
            // and it is what makes the medium band unreachable under the old
            // formula: 0.20 of the weight is gone before anything is measured.
            replay: ReplayMetadata {
                replayable: false,
                required_tools: Vec::new(),
                tool_manifest_hashes: BTreeMap::new(),
                expected_assertions: Vec::new(),
                replay_notes: Vec::new(),
            },
            embedding_analysis: None,
            value: ValueMetadata::default(),
            trace_card: TraceCard::default(),
            value_card: TraceValueCard::default(),
            hindsight: None,
            training_dynamics: None,
            process_evaluation: None,
        }
    }

    #[test]
    fn privacy_gate_and_risk_score_are_the_same_signal() {
        use super::*;
        // The reason the subtractive term was redundant: these are
        // complementary functions of one enum. If they ever stop being
        // complementary, the argument for applying only the gate needs
        // revisiting, so pin it.
        for risk in [
            ResidualPiiRisk::Low,
            ResidualPiiRisk::Medium,
            ResidualPiiRisk::High,
        ] {
            assert!(
                (privacy_gate(risk) + privacy_risk_score(risk) - 1.0).abs() < f32::EPSILON,
                "gate and risk score must remain complementary for {risk:?}"
            );
        }
    }

    #[test]
    fn medium_risk_work_can_earn_credit() {
        use super::*;
        // Every medium-risk submission in the pilot corpus scored exactly
        // zero - ten of ten - because risk was penalised twice: a 0.5 gate
        // and a flat -0.30. Accepting medium-risk work while guaranteeing it
        // earns nothing made the accept flag and the scorer disagree.
        let scored = compute_value_scorecard(&scoring_envelope(ResidualPiiRisk::Medium));
        assert!(
            scored.credit_points_estimate > 0.0,
            "medium-risk work that is accepted must be able to earn credit, got {}",
            scored.credit_points_estimate
        );
    }

    #[test]
    fn dropping_the_double_penalty_leaves_low_risk_untouched() {
        use super::*;
        // The change is confined to the medium band, and this is why rather
        // than a pinned number: the term that was removed evaluated to
        // `0.60 * privacy_risk_score(Low)`, and that factor is zero. So the
        // 331 low-risk submissions already in the corpus keep their scores
        // by construction, not by coincidence of the current weights.
        assert_eq!(
            privacy_risk_score(ResidualPiiRisk::Low),
            0.0,
            "the removed subtraction was already inert for low risk"
        );
        let scored = compute_value_scorecard(&scoring_envelope(ResidualPiiRisk::Low));
        assert!(
            scored.credit_points_estimate > 0.0,
            "low-risk work must still earn credit, got {}",
            scored.credit_points_estimate
        );
    }

    #[test]
    fn risk_bands_stay_ordered_and_high_earns_nothing() {
        use super::*;
        let low = compute_value_scorecard(&scoring_envelope(ResidualPiiRisk::Low));
        let medium = compute_value_scorecard(&scoring_envelope(ResidualPiiRisk::Medium));
        let high = compute_value_scorecard(&scoring_envelope(ResidualPiiRisk::High));

        assert!(
            low.credit_points_estimate > medium.credit_points_estimate,
            "low must still out-earn medium: {} vs {}",
            low.credit_points_estimate,
            medium.credit_points_estimate
        );
        assert_eq!(
            high.credit_points_estimate, 0.0,
            "high risk earns nothing regardless of quality"
        );
        // The blast-radius claim is about both outputs, not just credit.
        // For high risk the gate is 0.0, so the quality terms contribute
        // nothing and `raw` was negative before this change and is
        // non-positive after; either way it clamps to 0. Asserting the score
        // too keeps "only the medium band moves" honest.
        // `submission_score` is the name this value carries once it reaches an
        // envelope; on the scorecard itself the field is `online_score`
        // (`let submission_score = scorecard.online_score` in
        // `estimate_initial_credit`). The test was written against the
        // downstream name, so it never compiled.
        assert_eq!(
            high.online_score, 0.0,
            "high-risk submission score must clamp to zero either side of this change"
        );
    }

    #[test]
    fn rescrub_of_successfully_redacted_secret_lands_medium() {
        use super::*;

        let now = Utc::now();
        let secret = "sk-abcdefghijklmnopqrstuvwxyz012345";
        let mut env = TraceContributionEnvelope {
            schema_version: TRACE_CONTRIBUTION_SCHEMA_VERSION.to_string(),
            trace_id: Uuid::new_v4(),
            submission_id: Uuid::new_v4(),
            created_at: now,
            ironclaw: IronclawTraceMetadata {
                version: "1".to_string(),
                engine_version: None,
                feature_flags: BTreeMap::new(),
                channel: TraceChannel::Cli,
                model_name: None,
            },
            consent: ConsentMetadata {
                policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
                scopes: vec![ConsentScope::DebuggingEvaluation],
                message_text_included: true,
                tool_payloads_included: false,
                revocable: true,
            },
            contributor: ContributorMetadata {
                pseudonymous_contributor_id: None,
                tenant_scope_ref: None,
                credit_account_ref: None,
                revocation_handle: Uuid::new_v4(),
            },
            privacy: PrivacyMetadata {
                redaction_pipeline_version: DETERMINISTIC_REDACTION_PIPELINE_VERSION.to_string(),
                redaction_counts: BTreeMap::new(),
                privacy_filter_summary: None,
                pii_labels_present: Vec::new(),
                residual_pii_risk: ResidualPiiRisk::Low,
                redaction_hash: "sha256:placeholder".to_string(),
                warnings: Vec::new(),
            },
            events: vec![TraceContributionEvent {
                event_id: Uuid::new_v4(),
                parent_event_id: None,
                event_type: TraceContributionEventType::UserMessage,
                timestamp: now,
                redacted_content: Some(format!("export OPENAI_API_KEY={secret}")),
                structured_payload: Value::Null,
                tool_name: None,
                tool_category: None,
                tool_call_id: None,
                latency_ms: None,
                token_counts: None,
                cost_usd: None,
                success: None,
                failure_modes: Vec::new(),
                side_effect: SideEffectLevel::None,
            }],
            outcome: OutcomeMetadata::default(),
            replay: ReplayMetadata {
                replayable: false,
                required_tools: Vec::new(),
                tool_manifest_hashes: BTreeMap::new(),
                expected_assertions: Vec::new(),
                replay_notes: Vec::new(),
            },
            embedding_analysis: None,
            value: ValueMetadata::default(),
            trace_card: TraceCard::default(),
            value_card: TraceValueCard::default(),
            hindsight: None,
            training_dynamics: None,
            process_evaluation: None,
        };

        let redactor = DeterministicTraceRedactor::bare();
        rescrub_trace_envelope_with(&redactor, &mut env);

        let content = env.events[0]
            .redacted_content
            .as_deref()
            .expect("content present");
        assert!(
            !content.contains(secret),
            "secret must be removed from stored content: {content}"
        );
        assert!(
            env.privacy.redaction_counts.contains_key("secret")
                || env
                    .privacy
                    .redaction_counts
                    .keys()
                    .any(|k| k.starts_with("secret:")),
            "redaction telemetry must record the secret finding: {:?}",
            env.privacy.redaction_counts
        );
        assert_eq!(
            env.privacy.residual_pii_risk,
            ResidualPiiRisk::Medium,
            "successful secret scrub must land Medium (quarantine-with-override), not High"
        );
    }

    /// A single large event -- an agent pasting a whole file, say -- must
    /// reach the sidecar rather than being refused for length before it is
    /// ever spawned. The guard used to sit at 1 MiB while a whole envelope
    /// could be many times that, so one big event failed the entire
    /// submission (the external-filter pass propagates with `?`, so there is
    /// no partial success to fall back on).
    ///
    /// The adapter points at a command that does not exist, so the call
    /// cannot succeed; what this asserts is WHICH failure comes back. Past
    /// the guard means the spawn was attempted.
    #[tokio::test]
    async fn sidecar_input_guard_admits_a_field_the_envelope_cap_allows() {
        use crate::trace_contribution::{
            CommandPrivacyFilterAdapter, MAX_TRACE_ENVELOPE_BYTES, PrivacyFilterAdapter,
        };
        let adapter = CommandPrivacyFilterAdapter::new(
            "/nonexistent/trace-commons-privacy-filter-does-not-exist",
        );
        let at_cap = "x".repeat(MAX_TRACE_ENVELOPE_BYTES);
        let error = adapter
            .redact_text(&at_cap)
            .await
            .expect_err("no such command, so this cannot succeed");
        let reason = format!("{error:?}");
        assert!(
            !reason.contains("input exceeded limit"),
            "a field the envelope cap allows must reach the sidecar: {reason}"
        );
        assert!(
            reason.contains("failed to spawn"),
            "expected the spawn failure that proves the guard was passed: {reason}"
        );
    }

    /// The other direction: above the cap it is still a clean, label-only
    /// length refusal, not a spawn of a doomed subprocess.
    #[tokio::test]
    async fn sidecar_input_guard_still_refuses_above_the_cap() {
        use crate::trace_contribution::{
            CommandPrivacyFilterAdapter, MAX_TRACE_ENVELOPE_BYTES, PrivacyFilterAdapter,
        };
        let adapter = CommandPrivacyFilterAdapter::new(
            "/nonexistent/trace-commons-privacy-filter-does-not-exist",
        );
        let over_cap = "x".repeat(MAX_TRACE_ENVELOPE_BYTES + 1);
        let error = adapter
            .redact_text(&over_cap)
            .await
            .expect_err("above the cap must refuse");
        assert!(
            format!("{error:?}").contains("input exceeded limit"),
            "expected the length refusal: {error:?}"
        );
    }

    // --- Replay sufficiency -------------------------------------------
    //
    // A downstream consumer measured 330 pilot envelopes and found every one
    // scoring `replayability: 1.0` while none could be turned back into a
    // runnable task. The old formula restated `replay.replayable`, an
    // emitter-set boolean, so it could not fail. These pin the properties a
    // replay score has to have instead: a task to issue, arguments to issue
    // it with, and an answer to grade against.

    fn replay_event(
        event_type: super::TraceContributionEventType,
        tool_name: Option<&str>,
        tool_call_id: Option<&str>,
        content: Option<&str>,
        payload: super::Value,
    ) -> super::TraceContributionEvent {
        use super::*;
        TraceContributionEvent {
            event_id: Uuid::new_v4(),
            parent_event_id: None,
            event_type,
            timestamp: Utc::now(),
            redacted_content: content.map(str::to_string),
            structured_payload: payload,
            tool_name: tool_name.map(str::to_string),
            tool_category: None,
            tool_call_id: tool_call_id.map(str::to_string),
            latency_ms: None,
            token_counts: None,
            cost_usd: None,
            success: None,
            failure_modes: Vec::new(),
            side_effect: SideEffectLevel::None,
        }
    }

    /// The shape the web-history capture path actually emits: tool names and
    /// nothing else, with `replayable` asserted true.
    fn web_history_shaped_envelope() -> super::TraceContributionEnvelope {
        use super::*;
        let mut envelope = scoring_envelope(ResidualPiiRisk::Low);
        envelope.replay.replayable = true;
        envelope.replay.required_tools = vec!["gmail__list_messages".to_string()];
        envelope.events = vec![
            replay_event(
                TraceContributionEventType::UserMessage,
                None,
                None,
                None,
                serde_json::json!({"state": "Completed"}),
            ),
            replay_event(
                TraceContributionEventType::ToolCall,
                Some("gmail__list_messages"),
                None,
                None,
                serde_json::json!({"has_result": true, "has_error": false}),
            ),
            replay_event(
                TraceContributionEventType::AssistantMessage,
                None,
                None,
                None,
                Value::Null,
            ),
        ];
        envelope
    }

    /// The shape a benchmark item needs: a prompt, arguments on the call, and
    /// a result carrying what the agent observed.
    fn seedable_envelope() -> super::TraceContributionEnvelope {
        use super::*;
        let mut envelope = scoring_envelope(ResidualPiiRisk::Low);
        envelope.replay.replayable = true;
        envelope.replay.required_tools = vec!["gmail__list_messages".to_string()];
        envelope.events = vec![
            replay_event(
                TraceContributionEventType::UserMessage,
                None,
                None,
                Some("summarise my unread mail"),
                Value::Null,
            ),
            replay_event(
                TraceContributionEventType::ToolCall,
                Some("gmail__list_messages"),
                Some("call-1"),
                None,
                serde_json::json!({"arguments": {"label": "UNREAD"}}),
            ),
            replay_event(
                TraceContributionEventType::ToolResult,
                Some("gmail__list_messages"),
                Some("call-1"),
                Some("2 unread threads"),
                Value::Null,
            ),
            replay_event(
                TraceContributionEventType::AssistantMessage,
                None,
                None,
                Some("you have 2 unread threads"),
                Value::Null,
            ),
        ];
        envelope
    }

    #[test]
    fn replayability_is_zero_when_nothing_replayable_survived_redaction() {
        use super::*;
        // The exact corpus finding: `replayable: true`, tool names recorded,
        // no prompt, no arguments, no results. Nothing here can be replayed,
        // so nothing here may score as replayable.
        let scored = compute_value_scorecard(&web_history_shaped_envelope());
        assert_eq!(
            scored.replayability, 0.0,
            "a trace with no prompt, arguments or results is not replayable"
        );
    }

    #[test]
    fn replayability_is_one_for_a_seedable_trace() {
        use super::*;
        let scored = compute_value_scorecard(&seedable_envelope());
        assert_eq!(
            scored.replayability, 1.0,
            "prompt + arguments + a result per call is everything replay needs"
        );
    }

    #[test]
    fn replayability_beats_the_metadata_only_shape() {
        use super::*;
        // The property that matters more than either endpoint: the metric has
        // to separate these two at all. The old one scored them equal.
        let seedable = compute_value_scorecard(&seedable_envelope());
        let metadata_only = compute_value_scorecard(&web_history_shaped_envelope());
        assert!(
            seedable.replayability > metadata_only.replayability,
            "seedable {} must out-score metadata-only {}",
            seedable.replayability,
            metadata_only.replayability
        );
    }

    #[test]
    fn replayability_degrades_when_only_some_calls_carry_arguments() {
        use super::*;
        // Partial coverage is partial credit, not all-or-nothing: a trace half
        // of whose calls are seedable is worth more than one with none and
        // less than one that is fully seedable.
        let mut envelope = seedable_envelope();
        envelope.events.push(replay_event(
            TraceContributionEventType::ToolCall,
            Some("slack__post_message"),
            Some("call-2"),
            None,
            serde_json::json!({"has_result": true}),
        ));
        let partial = compute_value_scorecard(&envelope).replayability;
        let full = compute_value_scorecard(&seedable_envelope()).replayability;
        assert!(
            partial > 0.0 && partial < full,
            "partial coverage must land strictly between: {partial} vs {full}"
        );
    }

    #[test]
    fn a_result_on_a_call_does_not_pass_for_its_arguments() {
        use super::*;
        // The measure read `redacted_content` as evidence of arguments, and
        // the web-history path put the tool's RESULT in a call's content. So
        // the arguments third could be earned by a trace that never recorded
        // a single argument -- the exact cheap satisfaction this metric
        // exists to refuse. Arguments come from the payload or not at all.
        let mut envelope = scoring_envelope(ResidualPiiRisk::Low);
        envelope.replay.replayable = true;
        envelope.events = vec![
            replay_event(
                TraceContributionEventType::UserMessage,
                None,
                None,
                Some("summarise my unread mail"),
                Value::Null,
            ),
            replay_event(
                TraceContributionEventType::ToolCall,
                Some("gmail__list_messages"),
                Some("call-1"),
                Some("2 unread threads"),
                serde_json::json!({"has_arguments": false}),
            ),
        ];
        let sufficiency = replay_sufficiency(&envelope);
        assert_eq!(
            sufficiency.tool_calls_with_arguments, 0,
            "a result sitting in the call's content is not an argument"
        );
    }

    #[test]
    fn an_emitter_declaring_a_trace_unreplayable_is_still_believed() {
        use super::*;
        // Sufficiency can only ever lower the score. An emitter that knows the
        // trace cannot be replayed keeps the last word.
        let mut envelope = seedable_envelope();
        envelope.replay.replayable = false;
        assert_eq!(
            compute_value_scorecard(&envelope).replayability,
            0.0,
            "replayable: false is authoritative"
        );
    }

    #[test]
    fn a_trace_with_no_tool_calls_needs_only_a_prompt() {
        use super::*;
        // The tool-free traces in the corpus: there are no calls to carry
        // arguments, so the prompt is the whole of what replay needs. Absent
        // guarding, dividing by zero calls would score them 0 or NaN.
        let mut envelope = scoring_envelope(ResidualPiiRisk::Low);
        envelope.replay.replayable = true;
        envelope.replay.required_tools = Vec::new();
        envelope.events = vec![replay_event(
            TraceContributionEventType::UserMessage,
            None,
            None,
            Some("what is the capital of France"),
            Value::Null,
        )];
        let scored = compute_value_scorecard(&envelope);
        assert!(
            scored.replayability.is_finite(),
            "a tool-free trace must not divide by zero calls"
        );
        assert_eq!(
            scored.replayability, 1.0,
            "a prompt is all a tool-free trace needs to be re-issued"
        );
    }

    #[test]
    fn quality_does_not_reward_redacted_length() {
        use super::*;
        // `quality` was `event_count / 8.0`, so the more content redaction
        // stripped, the higher a trace scored. Forty empty events must not
        // out-score four that carry what they claim to.
        let mut padded = web_history_shaped_envelope();
        let filler = padded.events[1].clone();
        while padded.events.len() < 40 {
            let mut event = filler.clone();
            event.event_id = Uuid::new_v4();
            padded.events.push(event);
        }
        let padded_quality = compute_value_scorecard(&padded).quality;
        let substantive_quality = compute_value_scorecard(&seedable_envelope()).quality;
        assert!(
            substantive_quality > padded_quality,
            "content must out-score length: {substantive_quality} vs {padded_quality}"
        );
    }

    #[test]
    fn padding_a_trace_with_empty_events_cannot_raise_its_score() {
        use super::*;
        // The sharper form of the same property: appending contentless events
        // is the cheapest thing an emitter can do, so it must never pay. This
        // is what carried the pilot corpus to a 0.813 mean.
        let base = compute_value_scorecard(&web_history_shaped_envelope()).quality;
        let mut padded = web_history_shaped_envelope();
        let filler = padded.events[1].clone();
        for _ in 0..30 {
            let mut event = filler.clone();
            event.event_id = Uuid::new_v4();
            padded.events.push(event);
        }
        let padded_quality = compute_value_scorecard(&padded).quality;
        assert!(
            padded_quality <= base,
            "padding raised quality from {base} to {padded_quality}"
        );
    }

    #[test]
    fn the_scorecard_explains_which_replay_inputs_are_missing() {
        use super::*;
        // The consumer's complaint was not only the number but that "Replay
        // metadata is present." told them nothing. Make the explanation name
        // what is absent.
        let scored = compute_value_scorecard(&web_history_shaped_envelope());
        let explanation = scored.explanation.join(" ");
        assert!(
            explanation.contains("argument"),
            "explanation must name the missing replay inputs, got: {explanation}"
        );
    }

    // --- Privacy filter observability ---------------------------------
    //
    // An unset backend resolves to `Ok(None)` and builds a redactor that
    // silently performs no prose-PII filtering. That is indistinguishable at
    // runtime from a filter that ran and found nothing, and nothing anywhere
    // reports which backend is live. These pin the two properties that make
    // the difference observable: the backend can be asked for by name, and a
    // deployment can demand that one exists.

    #[test]
    fn privacy_filter_backend_from_env_reports_none_when_unset() {
        use super::{PrivacyFilterBackendTag, privacy_filter_backend_from_env};
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("TRACE_PRIVACY_FILTER_BACKEND");
        }
        let backend = privacy_filter_backend_from_env().expect("unset is not an error");
        assert_eq!(
            backend,
            PrivacyFilterBackendTag::None,
            "an unset backend must be reportable as None rather than invisible"
        );
    }

    #[test]
    fn privacy_filter_backend_from_env_reports_the_configured_backend() {
        use super::{PrivacyFilterBackendTag, privacy_filter_backend_from_env};
        // Sidecar rather than near-ai: the near-ai adapter is behind an
        // optional cargo feature that this crate's own test build does not
        // enable, and the property under test is backend reporting, not which
        // backend. The server crate compiles the near-ai feature in
        // unconditionally.
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("TRACE_PRIVACY_FILTER_BACKEND", "sidecar");
            std::env::set_var("TRACE_PRIVACY_FILTER_COMMAND", "/bin/true");
        }
        let backend = privacy_filter_backend_from_env();
        unsafe {
            std::env::remove_var("TRACE_PRIVACY_FILTER_BACKEND");
            std::env::remove_var("TRACE_PRIVACY_FILTER_COMMAND");
        }
        assert_eq!(
            backend.expect("configured backend must resolve"),
            PrivacyFilterBackendTag::Sidecar,
            "a configured backend must be reportable by name"
        );
    }

    #[test]
    fn privacy_filter_backend_from_env_surfaces_a_misconfigured_backend() {
        use super::privacy_filter_backend_from_env;
        // A backend named without its required configuration is the one
        // combination that must never resolve quietly. Before this, it
        // surfaced per-submission instead of at boot, which is how a filter
        // stays broken unnoticed.
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("TRACE_PRIVACY_FILTER_BACKEND", "sidecar");
            std::env::remove_var("TRACE_PRIVACY_FILTER_COMMAND");
            std::env::remove_var("IRONCLAW_TRACE_PRIVACY_FILTER_COMMAND");
        }
        let result = privacy_filter_backend_from_env();
        unsafe {
            std::env::remove_var("TRACE_PRIVACY_FILTER_BACKEND");
        }
        assert!(
            result.is_err(),
            "a backend named without its configuration must be an error, not a silent None"
        );
    }

    #[test]
    fn privacy_filter_backend_labels_are_stable() {
        use super::PrivacyFilterBackendTag;
        // These labels reach operational surfaces and stored envelope
        // summaries, so they are a contract rather than a debug string.
        assert_eq!(PrivacyFilterBackendTag::None.label(), "none");
        assert_eq!(PrivacyFilterBackendTag::Sidecar.label(), "sidecar");
        assert_eq!(PrivacyFilterBackendTag::NearAi.label(), "near_ai");
    }

    // --- Capture-path metadata ----------------------------------------
    //
    // A downstream consumer measured 330 envelopes from this path and could
    // rebuild none of them. Three of the gaps are fixable here: tool calls
    // carried no arguments field at all (so no consent setting could ever
    // ship them), turn timing was captured but never turned into a duration,
    // and reasoning existed on the tool call but was never emitted as the
    // `Reasoning` event the schema defines.

    fn capture_turn_with_tool_call() -> super::RawTraceCaptureTurn {
        use super::*;
        let started = Utc::now();
        RawTraceCaptureTurn {
            user_input: "summarise my unread mail".to_string(),
            response: Some("you have 2 unread threads".to_string()),
            tool_calls: vec![RawTraceCaptureToolCall {
                name: "gmail__list_messages".to_string(),
                id: Some("call-1".to_string()),
                arguments: Some(serde_json::json!({"label": "UNREAD"})),
                result_preview: Some("2 unread threads".to_string()),
                error: None,
                rationale: Some("the user asked about unread mail".to_string()),
            }],
            started_at: started,
            completed_at: Some(started + chrono::Duration::milliseconds(2500)),
            state: Some("Completed".to_string()),
        }
    }

    #[test]
    fn capture_tool_calls_carry_their_arguments_when_payloads_are_consented() {
        use super::*;
        // The single field that decides whether a trace can become a
        // benchmark item. Before this it did not exist on the capture type,
        // so no consent setting could produce it.
        let options = RecordedTraceContributionOptions {
            include_tool_payloads: true,
            ..RecordedTraceContributionOptions::default()
        };
        let raw =
            RawTraceContribution::from_capture_turns(&[capture_turn_with_tool_call()], options);
        let call = raw
            .events
            .iter()
            .find(|event| event.event_type == TraceContributionEventType::ToolCall)
            .expect("a tool call event");
        assert_eq!(
            call.structured_payload.get("arguments"),
            Some(&serde_json::json!({"label": "UNREAD"})),
            "consented tool payloads must carry the call arguments"
        );
    }

    #[test]
    fn capture_tool_calls_withhold_arguments_without_consent() {
        use super::*;
        // The flag still governs the content. Absent consent the envelope
        // reports only that arguments existed, which is shape, not payload.
        let raw = RawTraceContribution::from_capture_turns(
            &[capture_turn_with_tool_call()],
            RecordedTraceContributionOptions::default(),
        );
        let call = raw
            .events
            .iter()
            .find(|event| event.event_type == TraceContributionEventType::ToolCall)
            .expect("a tool call event");
        assert!(
            call.structured_payload.get("arguments").is_none(),
            "unconsented payloads must not carry arguments: {:?}",
            call.structured_payload
        );
        assert_eq!(
            call.structured_payload.get("has_arguments"),
            Some(&serde_json::json!(true)),
            "the shape signal survives without the payload"
        );
    }

    #[test]
    fn capture_turns_record_their_duration() {
        use super::*;
        // `started_at` and `completed_at` were both captured and neither was
        // ever turned into a latency. The corpus consequently had no duration
        // anywhere in it.
        let raw = RawTraceContribution::from_capture_turns(
            &[capture_turn_with_tool_call()],
            RecordedTraceContributionOptions::default(),
        );
        let response = raw
            .events
            .iter()
            .find(|event| event.event_type == TraceContributionEventType::AssistantMessage)
            .expect("an assistant response event");
        assert_eq!(
            response.latency_ms,
            Some(2500),
            "the response event must carry how long the turn took"
        );
    }

    #[test]
    fn capture_turns_without_a_completion_time_have_no_duration() {
        use super::*;
        // Do not invent one. A turn that never recorded completion has no
        // measurable duration, and a fabricated zero would be worse than an
        // absent field.
        let mut turn = capture_turn_with_tool_call();
        turn.completed_at = None;
        let raw = RawTraceContribution::from_capture_turns(
            &[turn],
            RecordedTraceContributionOptions::default(),
        );
        let response = raw
            .events
            .iter()
            .find(|event| event.event_type == TraceContributionEventType::AssistantMessage)
            .expect("an assistant response event");
        assert_eq!(
            response.latency_ms, None,
            "an unknown duration stays unknown"
        );
    }

    #[test]
    fn capture_rationale_becomes_a_reasoning_event() {
        use super::*;
        // `Reasoning` is defined in the schema and was never emitted. The
        // rationale existed all along, buried on the tool call, where a
        // consumer could not see that a reasoning step had occurred or where
        // in the sequence it sat.
        let options = RecordedTraceContributionOptions {
            include_message_text: true,
            ..RecordedTraceContributionOptions::default()
        };
        let raw =
            RawTraceContribution::from_capture_turns(&[capture_turn_with_tool_call()], options);
        let reasoning = raw
            .events
            .iter()
            .find(|event| event.event_type == TraceContributionEventType::Reasoning)
            .expect("a reasoning event");
        assert_eq!(
            reasoning.content.as_deref(),
            Some("the user asked about unread mail"),
            "consented reasoning carries its text"
        );

        let reasoning_index = raw
            .events
            .iter()
            .position(|event| event.event_type == TraceContributionEventType::Reasoning)
            .expect("reasoning position");
        let call_index = raw
            .events
            .iter()
            .position(|event| event.event_type == TraceContributionEventType::ToolCall)
            .expect("tool call position");
        assert!(
            reasoning_index < call_index,
            "reasoning must precede the call it explains"
        );
    }

    #[test]
    fn capture_reasoning_withholds_text_without_message_consent() {
        use super::*;
        // Reasoning is prose, so it is governed by message-text consent
        // rather than tool payloads. The event still appears: knowing a
        // reasoning step happened is shape, and shape is not content.
        let raw = RawTraceContribution::from_capture_turns(
            &[capture_turn_with_tool_call()],
            RecordedTraceContributionOptions::default(),
        );
        let reasoning = raw
            .events
            .iter()
            .find(|event| event.event_type == TraceContributionEventType::Reasoning)
            .expect("a reasoning event");
        assert!(
            reasoning.content.is_none(),
            "unconsented reasoning must not carry its text"
        );
    }

    // --- Event metadata the schema modelled and no emitter could reach ---
    //
    // `parent_event_id`, `tool_call_id`, `success` and `failure_modes` are all
    // on the envelope event and were all hardcoded empty in the one place a
    // raw event becomes an envelope event, because the raw event had nowhere
    // to carry them. None of the four is user content, which is why they are
    // the cheapest thing that makes a failed trace diagnosable.

    #[test]
    fn capture_tool_calls_emit_the_result_they_returned() {
        use super::*;
        // `ToolResult` is defined in the schema and this path never emitted
        // one: the observation the agent acted on was folded into the call,
        // so a consumer had a call with no answer to grade against.
        let options = RecordedTraceContributionOptions {
            include_tool_payloads: true,
            ..RecordedTraceContributionOptions::default()
        };
        let raw =
            RawTraceContribution::from_capture_turns(&[capture_turn_with_tool_call()], options);
        let call = raw
            .events
            .iter()
            .find(|event| event.event_type == TraceContributionEventType::ToolCall)
            .expect("a tool call event");
        let result = raw
            .events
            .iter()
            .find(|event| event.event_type == TraceContributionEventType::ToolResult)
            .expect("a tool result event");
        assert_eq!(
            result.content.as_deref(),
            Some("2 unread threads"),
            "the result event carries what the tool returned"
        );
        assert_eq!(
            result.parent_event_id,
            Some(call.event_id),
            "a result must name the call it answers"
        );
        assert_eq!(
            result.tool_call_id.as_deref(),
            Some("call-1"),
            "both halves of a call carry the harness's id for it"
        );
        assert_eq!(
            call.content, None,
            "the RESULT must not be reported as the call's own content"
        );
    }

    #[test]
    fn a_failed_tool_call_names_itself() {
        use super::*;
        // The corpus finding: 86 traces were labelled `failure` and the only
        // trace of it was `state: "Failed"` on a user message, which named no
        // tool. `success` is a boolean, not content, so it is set whatever
        // the payload consent says.
        let mut turn = capture_turn_with_tool_call();
        turn.tool_calls[0].result_preview = None;
        turn.tool_calls[0].error = Some("gmail: 401 unauthorized".to_string());
        let raw = RawTraceContribution::from_capture_turns(
            &[turn],
            RecordedTraceContributionOptions::default(),
        );
        let call = raw
            .events
            .iter()
            .find(|event| event.event_type == TraceContributionEventType::ToolCall)
            .expect("a tool call event");
        assert_eq!(
            call.success,
            Some(false),
            "an errored call must say so, and it is the only thing that names \
             which tool failed"
        );
        assert!(
            call.content.is_none(),
            "without payload consent the error text stays off the envelope"
        );
    }

    #[test]
    fn a_tool_call_with_no_recorded_outcome_claims_none() {
        use super::*;
        // `None` is not failure. A capture that recorded neither a result nor
        // an error does not know how the call went, and guessing would put a
        // fabricated outcome on a real trace.
        let mut turn = capture_turn_with_tool_call();
        turn.tool_calls[0].result_preview = None;
        turn.tool_calls[0].error = None;
        let raw = RawTraceContribution::from_capture_turns(
            &[turn],
            RecordedTraceContributionOptions::default(),
        );
        let call = raw
            .events
            .iter()
            .find(|event| event.event_type == TraceContributionEventType::ToolCall)
            .expect("a tool call event");
        assert_eq!(call.success, None, "an unknown outcome stays unknown");
        assert!(
            !raw.events
                .iter()
                .any(|event| event.event_type == TraceContributionEventType::ToolResult),
            "no result was recorded, so no result event may be invented"
        );
    }

    #[test]
    fn capture_turns_carry_the_model_they_ran_on() {
        use super::*;
        // Hardcoded `None` before this, which is why `model_name` was present
        // on 3 of 330 pilot envelopes.
        let options = RecordedTraceContributionOptions {
            model_name: Some("gpt-5.2".to_string()),
            ..RecordedTraceContributionOptions::default()
        };
        let raw =
            RawTraceContribution::from_capture_turns(&[capture_turn_with_tool_call()], options);
        assert_eq!(raw.ironclaw.model_name.as_deref(), Some("gpt-5.2"));
    }

    #[tokio::test]
    async fn redaction_preserves_event_metadata() {
        use super::*;
        // The conversion from raw to envelope hardcoded all four of these,
        // so an emitter that populated them had them silently dropped on the
        // way through redaction.
        let options = RecordedTraceContributionOptions {
            include_tool_payloads: true,
            ..RecordedTraceContributionOptions::default()
        };
        let mut raw =
            RawTraceContribution::from_capture_turns(&[capture_turn_with_tool_call()], options);
        let call_event_id = raw
            .events
            .iter()
            .find(|event| event.event_type == TraceContributionEventType::ToolCall)
            .expect("a tool call event")
            .event_id;
        // Only an emitter that classified the failure can set this, so plant
        // one to prove the field survives the trip.
        for event in raw.events.iter_mut() {
            if event.event_type == TraceContributionEventType::ToolResult {
                event.failure_modes = vec![TraceFailureMode::UnrecoverableToolFailure];
            }
        }
        let envelope = DeterministicTraceRedactor::deterministic_only(Vec::new())
            .redact_trace(raw)
            .await
            .expect("redaction succeeds");
        let result = envelope
            .events
            .iter()
            .find(|event| event.event_type == TraceContributionEventType::ToolResult)
            .expect("a tool result event");
        assert_eq!(result.parent_event_id, Some(call_event_id));
        assert_eq!(result.tool_call_id.as_deref(), Some("call-1"));
        assert_eq!(result.success, Some(true));
        assert_eq!(
            result.failure_modes,
            vec![TraceFailureMode::UnrecoverableToolFailure]
        );
    }

    #[test]
    fn a_recorded_trace_pairs_its_results_with_its_calls() {
        use super::*;
        // The recorded-trace path already had ids on both halves and used
        // them for nothing.
        let trace = crate::llm::recording::TraceFile {
            model_name: "claude-fable-5".to_string(),
            memory_snapshot: Vec::new(),
            http_exchanges: Vec::new(),
            steps: vec![crate::llm::recording::TraceStep {
                request_hint: None,
                response: crate::llm::recording::TraceResponse::ToolCalls {
                    tool_calls: vec![crate::llm::recording::TraceToolCall {
                        id: "call-9".to_string(),
                        name: "shell".to_string(),
                        arguments: serde_json::json!({"command": "ls"}),
                    }],
                    input_tokens: 10,
                    output_tokens: 2,
                },
                expected_tool_results: vec![crate::llm::recording::ExpectedToolResult {
                    tool_call_id: "call-9".to_string(),
                    name: "shell".to_string(),
                    content: "src".to_string(),
                }],
            }],
        };
        let raw = RawTraceContribution::from_recorded_trace(
            &trace,
            RecordedTraceContributionOptions::default(),
        );
        let call = raw
            .events
            .iter()
            .find(|event| event.event_type == TraceContributionEventType::ToolCall)
            .expect("a tool call event");
        let result = raw
            .events
            .iter()
            .find(|event| event.event_type == TraceContributionEventType::ToolResult)
            .expect("a tool result event");
        assert_eq!(call.tool_call_id.as_deref(), Some("call-9"));
        assert_eq!(result.parent_event_id, Some(call.event_id));
    }

    #[test]
    fn argument_key_names_survive_the_arguments_wrapper() {
        use super::*;
        // The canonical text is what duplicate and novelty scores are
        // computed over. Arguments live under an `arguments` key, so a
        // top-level-only summary renders every call in the corpus as
        // `keys(arguments)` -- a filesystem read and a calendar write
        // becoming the same string. #211 is already about tool-name-only
        // canonical text collapsing this corpus; this would have widened it.
        let mut envelope = scoring_envelope(ResidualPiiRisk::Low);
        envelope.events = vec![replay_event(
            TraceContributionEventType::ToolCall,
            Some("gmail__list_messages"),
            None,
            None,
            serde_json::json!({"arguments": {"label": "UNREAD", "max": 10}}),
        )];
        let canonical = canonical_summary_for_embedding(&envelope);
        assert!(
            canonical.contains("arguments:[label,max]"),
            "argument key names must reach the canonical text: {canonical}"
        );
    }

    #[test]
    fn two_calls_with_different_arguments_do_not_canonicalise_alike() {
        use super::*;
        let call = |payload| {
            let mut envelope = scoring_envelope(ResidualPiiRisk::Low);
            envelope.events = vec![replay_event(
                TraceContributionEventType::ToolCall,
                Some("builtin__http"),
                None,
                None,
                payload,
            )];
            canonical_summary_for_embedding(&envelope)
        };
        assert_ne!(
            call(serde_json::json!({"arguments": {"url": "x"}})),
            call(serde_json::json!({"arguments": {"method": "POST", "body": "y"}})),
            "different argument shapes must not produce identical canonical text"
        );
    }

    #[test]
    fn payload_values_never_reach_the_canonical_text() {
        use super::*;
        // Key names only, at both levels. This text is stored and embedded.
        let mut envelope = scoring_envelope(ResidualPiiRisk::Low);
        envelope.events = vec![replay_event(
            TraceContributionEventType::ToolCall,
            Some("gmail__list_messages"),
            None,
            None,
            serde_json::json!({"arguments": {"label": "TOP-SECRET-VALUE"}}),
        )];
        let canonical = canonical_summary_for_embedding(&envelope);
        assert!(
            !canonical.contains("TOP-SECRET-VALUE"),
            "a payload value must never be summarised into canonical text: {canonical}"
        );
    }
}
