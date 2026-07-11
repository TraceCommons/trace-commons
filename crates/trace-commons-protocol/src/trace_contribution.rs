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
pub const DETERMINISTIC_REDACTION_PIPELINE_VERSION: &str = "ironclaw-deterministic-secret-path-v2";
pub const PRIVACY_FILTER_SIDECAR_PIPELINE_SUFFIX: &str = "privacy-filter-sidecar-v1";
pub const PRIVACY_FILTER_NEAR_AI_PIPELINE_SUFFIX: &str = "privacy-filter-near-ai-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivacyFilterBackendTag {
    None,
    Sidecar,
    NearAi,
}
pub const SERVER_RESCRUB_PIPELINE_SUFFIX: &str = "server-rescrub-v1";
#[cfg(feature = "near-ai-privacy-filter")]
pub const NEAR_AI_PII_BACKSTOP_PIPELINE_SUFFIX: &str = "near-ai-pii-backstop-v1";
pub const PRIVACY_FILTER_CANARY_VERSION: &str = "trace-privacy-filter-canary-v1";
pub const PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES: usize = 1024 * 1024;
pub const PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDOUT_BYTES: usize = 1024 * 1024;
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TrainingDynamicsSignals {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_confidence: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variability: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correctness: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cartography_bucket: Option<CartographyBucket>,
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

pub fn compute_value_scorecard(envelope: &TraceContributionEnvelope) -> TraceValueScorecard {
    let schema_validity = if envelope.schema_version == TRACE_CONTRIBUTION_SCHEMA_VERSION {
        1.0
    } else {
        0.0
    };
    let privacy_risk = privacy_risk_score(envelope.privacy.residual_pii_risk);
    let gate = privacy_gate(envelope.privacy.residual_pii_risk);
    let event_count = envelope.events.len() as f32;
    let quality = (event_count / 8.0).clamp(0.15, 1.0);
    let replayability = if envelope.replay.replayable { 1.0 } else { 0.0 };
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

    let raw = gate
        * schema_validity
        * (0.25 * quality
            + 0.20 * replayability
            + 0.20 * novelty
            + 0.15 * coverage_bonus
            + 0.10 * difficulty
            + 0.10 * user_correction_value)
        - 0.40 * duplicate_penalty
        - 0.60 * privacy_risk;
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
        explanation.push("Replay metadata is present.".to_string());
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawTraceContributionEvent {
    pub event_id: Uuid,
    pub event_type: TraceContributionEventType,
    pub timestamp: DateTime<Utc>,
    pub content: Option<String>,
    pub structured_payload: Value,
    pub tool_name: Option<String>,
    pub latency_ms: Option<u64>,
    pub token_counts: Option<TokenCounts>,
    pub cost_usd: Option<Decimal>,
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

        for step in &trace.steps {
            match &step.response {
                TraceResponse::UserInput { content } => {
                    events.push(RawTraceContributionEvent {
                        event_id: Uuid::new_v4(),
                        event_type: TraceContributionEventType::UserMessage,
                        timestamp: created_at,
                        content: options.include_message_text.then(|| content.clone()),
                        structured_payload: Value::Null,
                        tool_name: None,
                        latency_ms: None,
                        token_counts: None,
                        cost_usd: None,
                    });
                }
                TraceResponse::Text {
                    content,
                    input_tokens,
                    output_tokens,
                } => {
                    events.push(RawTraceContributionEvent {
                        event_id: Uuid::new_v4(),
                        event_type: TraceContributionEventType::AssistantMessage,
                        timestamp: created_at,
                        content: options.include_message_text.then(|| content.clone()),
                        structured_payload: Value::Null,
                        tool_name: None,
                        latency_ms: None,
                        token_counts: Some(TokenCounts {
                            input_tokens: *input_tokens,
                            output_tokens: *output_tokens,
                        }),
                        cost_usd: None,
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

                        events.push(RawTraceContributionEvent {
                            event_id: Uuid::new_v4(),
                            event_type: TraceContributionEventType::ToolCall,
                            timestamp: created_at,
                            content: None,
                            structured_payload,
                            tool_name: Some(tool_call.name.clone()),
                            latency_ms: None,
                            token_counts: Some(TokenCounts {
                                input_tokens: *input_tokens,
                                output_tokens: *output_tokens,
                            }),
                            cost_usd: None,
                        });
                    }
                }
            }

            for expected in &step.expected_tool_results {
                required_tools.insert(expected.name.clone());
                events.push(RawTraceContributionEvent {
                    event_id: Uuid::new_v4(),
                    event_type: TraceContributionEventType::ToolResult,
                    timestamp: created_at,
                    content: options
                        .include_tool_payloads
                        .then(|| expected.content.clone()),
                    structured_payload: serde_json::json!({
                        "tool_call_id": expected.tool_call_id,
                    }),
                    tool_name: Some(expected.name.clone()),
                    latency_ms: None,
                    token_counts: None,
                    cost_usd: None,
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
                event_type: TraceContributionEventType::HttpExchange,
                timestamp: created_at,
                content: options
                    .include_tool_payloads
                    .then(|| exchange.response.body.clone()),
                structured_payload,
                tool_name: Some("http".to_string()),
                latency_ms: None,
                token_counts: None,
                cost_usd: None,
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
                    event_type: TraceContributionEventType::UserMessage,
                    timestamp: turn.started_at,
                    content: options
                        .include_message_text
                        .then(|| turn.user_input.clone()),
                    structured_payload: serde_json::json!({
                        "state": turn.state,
                    }),
                    tool_name: None,
                    latency_ms: None,
                    token_counts: None,
                    cost_usd: None,
                });
            }

            for tool_call in &turn.tool_calls {
                required_tools.insert(tool_call.name.clone());
                let structured_payload = if options.include_tool_payloads {
                    serde_json::json!({
                        "result_preview": tool_call.result_preview,
                        "error": tool_call.error,
                        "rationale": tool_call.rationale,
                    })
                } else {
                    serde_json::json!({
                        "has_result": tool_call.result_preview.is_some(),
                        "has_error": tool_call.error.is_some(),
                    })
                };
                let content = options
                    .include_tool_payloads
                    .then(|| {
                        tool_call
                            .result_preview
                            .as_deref()
                            .or(tool_call.error.as_deref())
                            .unwrap_or("")
                            .to_string()
                    })
                    .filter(|content| !content.is_empty());

                events.push(RawTraceContributionEvent {
                    event_id: Uuid::new_v4(),
                    event_type: TraceContributionEventType::ToolCall,
                    timestamp: turn.completed_at.unwrap_or(turn.started_at),
                    content,
                    structured_payload,
                    tool_name: Some(tool_call.name.clone()),
                    latency_ms: None,
                    token_counts: None,
                    cost_usd: None,
                });
            }

            if let Some(response) = &turn.response {
                events.push(RawTraceContributionEvent {
                    event_id: Uuid::new_v4(),
                    event_type: TraceContributionEventType::AssistantMessage,
                    timestamp: turn.completed_at.unwrap_or(turn.started_at),
                    content: options.include_message_text.then(|| response.clone()),
                    structured_payload: Value::Null,
                    tool_name: None,
                    latency_ms: None,
                    token_counts: None,
                    cost_usd: None,
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
                model_name: None,
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
    pub blocked_secret_detected: bool,
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
    match backend {
        PrivacyFilterBackendTag::None => "none",
        PrivacyFilterBackendTag::Sidecar => "sidecar",
        PrivacyFilterBackendTag::NearAi => "near_ai",
    }
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

/// Byte ranges of high-entropy tokens that sit within [`CUE_WINDOW`] chars
/// after a secret cue (authorization/bearer/api_key/secret/password/token/
/// key=/: ...), excluding allowlisted ID/UUID/hash shapes. Fail-closed: when
/// unsure whether a token is a structural identifier, it is redacted.
///
/// This exists to catch secrets in formats not covered by
/// [`secret_leak_patterns`] (unknown provider key shapes, ad hoc tokens,
/// etc). Cue-gating is mandatory, not a nicety: an ungated entropy scan over
/// real transcripts flags on the order of 105k tokens (message ids, base64
/// content, UUIDs) against ~20 real secrets, which makes plain entropy
/// scanning useless in practice.
fn contextual_entropy_secret_ranges(content: &str) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    for candidate in entropy_candidate_regex().find_iter(content) {
        let token = candidate.as_str();
        if token.len() < ENTROPY_MIN_LEN {
            continue;
        }
        if is_allowlisted_entropy_candidate(token) {
            continue;
        }
        if token_shannon_entropy(token) < ENTROPY_BITS_MIN {
            continue;
        }
        let mut window_start = candidate.start().saturating_sub(CUE_WINDOW);
        while window_start > 0 && !content.is_char_boundary(window_start) {
            window_start -= 1;
        }
        let window = &content[window_start..candidate.start()];
        if !secret_cue_regex().is_match(window) {
            continue;
        }
        ranges.push(candidate.start()..candidate.end());
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
        for entropy_range in contextual_entropy_secret_ranges(&redacted) {
            let overlaps = ranges.iter().any(|existing| {
                existing.start < entropy_range.end && entropy_range.start < existing.end
            });
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

            let tool_call_id = raw_event
                .structured_payload
                .get("tool_call_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let tool_category = raw_event.tool_name.as_deref().map(tool_category_for);
            let side_effect = side_effect_for(raw_event.event_type, raw_event.tool_name.as_deref());

            events.push(TraceContributionEvent {
                event_id: raw_event.event_id,
                parent_event_id: None,
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
                success: None,
                failure_modes: Vec::new(),
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

    let blocked_secret_detected = report.blocked_secret_detected;
    for (label, count) in report.counts {
        *envelope.privacy.redaction_counts.entry(label).or_insert(0) += count;
    }
    for label in report.pii_labels_present {
        if !envelope.privacy.pii_labels_present.contains(&label) {
            envelope.privacy.pii_labels_present.push(label);
        }
    }

    let server_pass_risk = residual_risk(
        &envelope.consent,
        &RedactionReport {
            counts: BTreeMap::new(),
            pii_labels_present: Vec::new(),
            warnings: Vec::new(),
            blocked_secret_detected,
        },
    );
    envelope.privacy.residual_pii_risk =
        max_residual_risk(envelope.privacy.residual_pii_risk, server_pass_risk);
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

/// Runs an async prose-PII filter (e.g. the NEAR AI backstop) over an
/// already-produced envelope's free-text fields — `events[*].redacted_content`
/// and `outcome.human_correction`. Tool-call `structured_payload` JSON is out
/// of scope; this is a prose filter only.
///
/// Two-pass and atomic: every `adapter.redact_text` call is awaited and
/// collected in the first pass, so any adapter error is returned before any
/// field of `envelope` is mutated. The second pass applies the collected
/// text replacements and metadata updates without further awaits.
#[cfg(feature = "near-ai-privacy-filter")]
pub async fn rescrub_envelope_prose_pii_with(
    adapter: &dyn PrivacyFilterAdapter,
    envelope: &mut TraceContributionEnvelope,
) -> Result<(), TraceContributionError> {
    let mut event_updates: Vec<(usize, String)> = Vec::new();
    let mut correction_update: Option<String> = None;
    let mut report = RedactionReport::default();
    let mut summary: Option<SafePrivacyFilterSummary> = None;

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

    // Second pass: no awaits below this line, so the updates collected
    // above are applied atomically.
    for (index, redacted_text) in event_updates {
        envelope.events[index].redacted_content = Some(redacted_text);
    }
    if let Some(redacted_text) = correction_update {
        envelope.outcome.human_correction = Some(redacted_text);
    }

    let blocked_secret_detected = report.blocked_secret_detected;
    for (label, count) in report.counts {
        *envelope.privacy.redaction_counts.entry(label).or_insert(0) += count;
    }
    for label in report.pii_labels_present {
        if !envelope.privacy.pii_labels_present.contains(&label) {
            envelope.privacy.pii_labels_present.push(label);
        }
    }
    if let Some(summary) = &summary {
        for (label, count) in &summary.by_label {
            *envelope
                .privacy
                .redaction_counts
                .entry(label.clone())
                .or_insert(0) += count;
            if !envelope.privacy.pii_labels_present.contains(label) {
                envelope.privacy.pii_labels_present.push(label.clone());
            }
        }
        merge_privacy_filter_summary(&mut envelope.privacy.privacy_filter_summary, summary);
    }

    let backstop_pass_risk = residual_risk(
        &envelope.consent,
        &RedactionReport {
            counts: BTreeMap::new(),
            pii_labels_present: Vec::new(),
            warnings: Vec::new(),
            blocked_secret_detected,
        },
    );
    envelope.privacy.residual_pii_risk =
        max_residual_risk(envelope.privacy.residual_pii_risk, backstop_pass_risk);
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

fn residual_risk(consent: &ConsentMetadata, report: &RedactionReport) -> ResidualPiiRisk {
    if report.blocked_secret_detected {
        return ResidualPiiRisk::High;
    }

    if consent.message_text_included || consent.tool_payloads_included {
        return ResidualPiiRisk::Medium;
    }

    ResidualPiiRisk::Low
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

fn safe_payload_summary(payload: &Value) -> String {
    match payload {
        Value::Object(map) => {
            let keys = map.keys().take(8).cloned().collect::<Vec<_>>();
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
mod tests {
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
        let redactor = DeterministicTraceRedactor::try_default()
            .expect("default redactor")
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
        let redactor = DeterministicTraceRedactor::try_default()
            .expect("default redactor")
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
        let r = DeterministicTraceRedactor::new(vec![]).unwrap();
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
        let r = DeterministicTraceRedactor::new(vec![]).unwrap();
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
        let r = DeterministicTraceRedactor::new(vec![]).unwrap();
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
        let r = DeterministicTraceRedactor::new(vec![]).unwrap();
        // opaque high-entropy token, no known prefix, but preceded by a cue
        let secret = "Zx9Qk2Lm7Pv4Rt8Wy1Nb6Hd3Fg5Jc0Ae";
        let (out, rep) = r.redact_text(&format!("api_key: {secret}"));
        assert!(!out.contains(secret), "cue-adjacent secret survived: {out}");
        assert!(rep.blocked_secret_detected);
    }

    #[test]
    fn contextual_entropy_spares_ids_and_hashes_and_uncued_tokens() {
        use super::*;
        let r = DeterministicTraceRedactor::new(vec![]).unwrap();
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
        let r = DeterministicTraceRedactor::new(vec![]).unwrap();
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

    #[cfg(feature = "near-ai-privacy-filter")]
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
            async fn redact_text(&self, text: &str)
                -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
                if text.contains("jane@example.com") {
                    Ok(Some(SafePrivacyFilterRedaction {
                        redacted_text: text.replace("jane@example.com", "[REDACTED:private_email]"),
                        summary: SafePrivacyFilterSummary {
                            schema_version: 1, output_mode: "redacted_text_only".into(),
                            span_count: 1,
                            by_label: std::collections::BTreeMap::from([("private_email".into(), 1)]),
                            decoded_mismatch: false,
                        },
                        report: RedactionReport::default(),
                    }))
                } else { Ok(None) }
            }
        }
        let mut env = sample_envelope_with_event_content("email jane@example.com now");
        rescrub_envelope_prose_pii_with(&Stub, &mut env).await.unwrap();
        assert!(env.events[0].redacted_content.as_deref().unwrap().contains("[REDACTED:private_email]"));
        assert!(env.privacy.redaction_pipeline_version.contains("near-ai-pii-backstop-v1"));
        assert!(env.privacy.pii_labels_present.iter().any(|l| l == "private_email"));
        // Idempotent suffix: running again does not double-append.
        rescrub_envelope_prose_pii_with(&Stub, &mut env).await.unwrap();
        assert_eq!(env.privacy.redaction_pipeline_version.matches("near-ai-pii-backstop-v1").count(), 1);
    }
}
