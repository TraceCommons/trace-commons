//! Backend-agnostic storage contracts for Trace Commons corpus metadata.
//!
//! These types describe the DB-backed production storage surface without
//! changing the current file-backed ingest path.

use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::DatabaseError;

fn default_trace_ranking_min_label_source_count() -> u32 {
    1
}

fn default_trace_ranking_joined_evidence_hash() -> String {
    "sha256:legacy".to_string()
}

fn default_trace_ranking_worker_run_status() -> TraceRankingWorkerRunStatus {
    TraceRankingWorkerRunStatus::Completed
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceCorpusStatus {
    Received,
    Accepted,
    Quarantined,
    Rejected,
    Revoked,
    Expired,
    Purged,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceObjectArtifactKind {
    SubmittedEnvelope,
    RescrubbedEnvelope,
    ReviewSnapshot,
    BenchmarkArtifact,
    ExportArtifact,
    WorkerIntermediate,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceDerivedStatus {
    Current,
    Invalidated,
    Superseded,
    Revoked,
    Expired,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceVectorEntryStatus {
    Active,
    Invalidated,
    Deleted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceVectorEntrySourceProjection {
    CanonicalSummary,
    RedactedMessages,
    RedactedToolSequence,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceWorkerKind {
    ServerRescrub,
    Summary,
    DuplicatePrecheck,
    Embedding,
    Ranking,
    BenchmarkConversion,
    ProcessEvaluation,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceAuditAction {
    Submit,
    Read,
    Review,
    CreditMutate,
    Revoke,
    Export,
    Retain,
    Purge,
    VectorIndex,
    BenchmarkConvert,
    ProcessEvaluate,
    PolicyUpdate,
    RankingWorkerRunRecovery,
    RankingCalibrationDatasetQuarantine,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceReviewLeaseAuditAction {
    Claim,
    Release,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceCreditEventType {
    Accepted,
    PrivacyRejection,
    DuplicateRejection,
    BenchmarkConversion,
    RegressionCatch,
    TrainingUtility,
    RankingUtility,
    ReviewerBonus,
    AbusePenalty,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceCreditSettlementState {
    Pending,
    Final,
    Reversed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceCreditSettlementBatchStatus {
    DryRun,
    Finalized,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceCreditSettlementNearStatus {
    Disabled,
    Pending,
    Submitted,
    Confirmed,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceBenchmarkRegistryOutboxOperation {
    Publish,
    Revoke,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceBenchmarkRegistryOutboxStatus {
    Pending,
    Submitted,
    Confirmed,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceCreditHoldReason {
    DuplicateClusterUnderReview,
    PrivacyRiskUnderReview,
    SuspectedAbuse,
    RevocationPropagation,
    AttestationDispute,
    PolicyMigration,
    LegalCompliance,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceRankingModelStatus {
    Candidate,
    Active,
    Deprecated,
    Archived,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TraceRankingLabelSource {
    FrontierLab,
    Reviewer,
    Benchmark,
    System,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceRankingUtilityCategory {
    ModelTraining,
    RankingTraining,
    Evaluation,
    Regression,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceRankingLabelOutcome {
    Useful,
    Neutral,
    Rejected,
    RegressionCaught,
    Disputed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceRankingCalibrationDatasetStatus {
    Candidate,
    Active,
    Deprecated,
    Archived,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRankingModelVersionWrite {
    pub tenant_id: String,
    pub model_version: String,
    pub feature_schema_version: String,
    pub policy_version: String,
    pub status: TraceRankingModelStatus,
    pub training_dataset_hash: String,
    pub calibration_dataset_hash: String,
    pub model_artifact_hash: String,
    pub actor_principal_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRankingModelVersionRecord {
    pub tenant_id: String,
    pub model_version: String,
    pub feature_schema_version: String,
    pub policy_version: String,
    pub status: TraceRankingModelStatus,
    pub training_dataset_hash: String,
    pub calibration_dataset_hash: String,
    pub model_artifact_hash: String,
    pub actor_principal_ref: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRankingCalibrationDatasetWrite {
    pub tenant_id: String,
    pub calibration_dataset_hash: String,
    pub target_use: String,
    pub policy_version: String,
    pub source_manifest_hash: String,
    pub source_count: u32,
    pub label_source_count: u32,
    pub label_actor_count: u32,
    pub status: TraceRankingCalibrationDatasetStatus,
    pub actor_principal_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRankingCalibrationDatasetStatusUpdate {
    pub tenant_id: String,
    pub calibration_dataset_hash: String,
    pub target_use: String,
    pub policy_version: String,
    pub status: TraceRankingCalibrationDatasetStatus,
    pub actor_principal_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRankingCalibrationDatasetRecord {
    pub tenant_id: String,
    pub calibration_dataset_hash: String,
    pub target_use: String,
    pub policy_version: String,
    pub source_manifest_hash: String,
    pub source_count: u32,
    pub label_source_count: u32,
    pub label_actor_count: u32,
    pub status: TraceRankingCalibrationDatasetStatus,
    pub actor_principal_ref: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceRankingFeatureWrite {
    pub tenant_id: String,
    pub ranking_feature_id: Uuid,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub target_use: String,
    pub feature_schema_version: String,
    pub feature_vector_hash: String,
    pub feature_names_hash: String,
    pub source_feature_hash: String,
    pub duplicate_score: Option<f32>,
    pub novelty_score: Option<f32>,
    pub privacy_risk_score: Option<f32>,
    pub quality_score: Option<f32>,
    pub coverage_tags: Vec<String>,
    pub actor_principal_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceRankingFeatureRecord {
    pub tenant_id: String,
    pub ranking_feature_id: Uuid,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub target_use: String,
    pub feature_schema_version: String,
    pub feature_vector_hash: String,
    pub feature_names_hash: String,
    pub source_feature_hash: String,
    pub duplicate_score: Option<f32>,
    pub novelty_score: Option<f32>,
    pub privacy_risk_score: Option<f32>,
    pub quality_score: Option<f32>,
    pub coverage_tags: Vec<String>,
    pub actor_principal_ref: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceRankingPredictionWrite {
    pub tenant_id: String,
    pub ranking_prediction_id: Uuid,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub target_use: String,
    pub model_version: String,
    pub feature_schema_version: String,
    pub prediction_policy_version: String,
    pub feature_vector_hash: String,
    pub predicted_utility_micros: i64,
    pub uncertainty_micros: i64,
    pub confidence: f32,
    pub risk_penalty_micros: i64,
    pub novelty_bonus_micros: i64,
    pub settlement_score_micros: i64,
    pub explanation_codes: Vec<String>,
    pub actor_principal_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceRankingPredictionRecord {
    pub tenant_id: String,
    pub ranking_prediction_id: Uuid,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub target_use: String,
    pub model_version: String,
    pub feature_schema_version: String,
    pub prediction_policy_version: String,
    pub feature_vector_hash: String,
    pub predicted_utility_micros: i64,
    pub uncertainty_micros: i64,
    pub confidence: f32,
    pub risk_penalty_micros: i64,
    pub novelty_bonus_micros: i64,
    pub settlement_score_micros: i64,
    pub explanation_codes: Vec<String>,
    pub actor_principal_ref: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRankingLabelWrite {
    pub tenant_id: String,
    pub ranking_label_id: Uuid,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub target_use: String,
    pub label_source: TraceRankingLabelSource,
    pub utility_category: TraceRankingUtilityCategory,
    pub label_outcome: TraceRankingLabelOutcome,
    pub utility_delta_micros: i64,
    pub evidence_hash: String,
    pub external_ref_hash: String,
    pub actor_principal_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRankingLabelRecord {
    pub tenant_id: String,
    pub ranking_label_id: Uuid,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub target_use: String,
    pub label_source: TraceRankingLabelSource,
    pub utility_category: TraceRankingUtilityCategory,
    pub label_outcome: TraceRankingLabelOutcome,
    pub utility_delta_micros: i64,
    pub evidence_hash: String,
    pub external_ref_hash: String,
    pub actor_principal_ref: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRankingPreferenceLabelWrite {
    pub tenant_id: String,
    pub preference_label_id: Uuid,
    pub preferred_submission_id: Uuid,
    pub preferred_trace_id: Uuid,
    pub rejected_submission_id: Uuid,
    pub rejected_trace_id: Uuid,
    pub target_use: String,
    pub label_source: TraceRankingLabelSource,
    pub utility_category: TraceRankingUtilityCategory,
    pub preference_strength_micros: i64,
    pub evidence_hash: String,
    pub external_ref_hash: String,
    pub actor_principal_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRankingPreferenceLabelRecord {
    pub tenant_id: String,
    pub preference_label_id: Uuid,
    pub preferred_submission_id: Uuid,
    pub preferred_trace_id: Uuid,
    pub rejected_submission_id: Uuid,
    pub rejected_trace_id: Uuid,
    pub target_use: String,
    pub label_source: TraceRankingLabelSource,
    pub utility_category: TraceRankingUtilityCategory,
    pub preference_strength_micros: i64,
    pub evidence_hash: String,
    pub external_ref_hash: String,
    pub actor_principal_ref: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceRankingCalibrationRunWrite {
    pub tenant_id: String,
    pub calibration_run_id: Uuid,
    pub model_version: String,
    pub target_use: String,
    pub policy_version: String,
    pub evaluation_dataset_hash: String,
    pub prediction_count: u32,
    pub label_count: u32,
    pub joined_label_prediction_count: u32,
    pub joined_label_source_count: u32,
    pub joined_evidence_hash: String,
    pub average_predicted_utility_micros: Option<i64>,
    pub average_label_utility_delta_micros: Option<i64>,
    pub average_absolute_error_micros: Option<i64>,
    pub max_label_source_average_absolute_error_micros: Option<i64>,
    pub max_error_label_source: Option<String>,
    pub mean_signed_error_micros: Option<i64>,
    pub low_confidence_prediction_count: u32,
    pub confidence_threshold: f32,
    pub min_label_count: u32,
    pub min_label_source_count: u32,
    pub max_average_absolute_error_micros: i64,
    pub promotable: bool,
    pub reason_codes: Vec<String>,
    pub report_hash: String,
    pub actor_principal_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceRankingCalibrationRunRecord {
    pub tenant_id: String,
    pub calibration_run_id: Uuid,
    pub model_version: String,
    pub target_use: String,
    pub policy_version: String,
    pub evaluation_dataset_hash: String,
    pub prediction_count: u32,
    pub label_count: u32,
    pub joined_label_prediction_count: u32,
    #[serde(default)]
    pub joined_label_source_count: u32,
    #[serde(default = "default_trace_ranking_joined_evidence_hash")]
    pub joined_evidence_hash: String,
    pub average_predicted_utility_micros: Option<i64>,
    pub average_label_utility_delta_micros: Option<i64>,
    pub average_absolute_error_micros: Option<i64>,
    #[serde(default)]
    pub max_label_source_average_absolute_error_micros: Option<i64>,
    #[serde(default)]
    pub max_error_label_source: Option<String>,
    pub mean_signed_error_micros: Option<i64>,
    pub low_confidence_prediction_count: u32,
    pub confidence_threshold: f32,
    pub min_label_count: u32,
    #[serde(default = "default_trace_ranking_min_label_source_count")]
    pub min_label_source_count: u32,
    pub max_average_absolute_error_micros: i64,
    pub promotable: bool,
    pub reason_codes: Vec<String>,
    pub report_hash: String,
    pub actor_principal_ref: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceRankingWorkerRunKind {
    Calibration,
    PredictionCredit,
    ModelPromotion,
    CreditCycle,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceRankingWorkerRunStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRankingWorkerRunWrite {
    pub tenant_id: String,
    pub ranking_worker_run_id: Uuid,
    pub run_kind: TraceRankingWorkerRunKind,
    pub status: TraceRankingWorkerRunStatus,
    pub dry_run: bool,
    pub reason_hash: String,
    pub model_version: Option<String>,
    pub target_use: Option<String>,
    pub policy_version: Option<String>,
    pub limit: u32,
    pub checked_count: u32,
    pub succeeded_count: u32,
    pub skipped_existing_count: u32,
    pub skipped_model_risk_count: u32,
    pub skipped_ineligible_count: u32,
    pub pending_after_count: u32,
    pub result_refs: Vec<String>,
    pub reason_counts: BTreeMap<String, u32>,
    pub actor_principal_ref: String,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub last_error_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRankingWorkerRunRecord {
    pub tenant_id: String,
    pub ranking_worker_run_id: Uuid,
    pub run_kind: TraceRankingWorkerRunKind,
    #[serde(default = "default_trace_ranking_worker_run_status")]
    pub status: TraceRankingWorkerRunStatus,
    pub dry_run: bool,
    pub reason_hash: String,
    pub model_version: Option<String>,
    pub target_use: Option<String>,
    pub policy_version: Option<String>,
    pub limit: u32,
    pub checked_count: u32,
    pub succeeded_count: u32,
    pub skipped_existing_count: u32,
    pub skipped_model_risk_count: u32,
    pub skipped_ineligible_count: u32,
    pub pending_after_count: u32,
    pub result_refs: Vec<String>,
    pub reason_counts: BTreeMap<String, u32>,
    pub actor_principal_ref: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_error_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceCreditAccountSettlementLineItem {
    pub credit_account_ref: String,
    pub credit_account_hash: String,
    pub settled_credit_delta_micros: i64,
    pub source_credit_event_ids: Vec<Uuid>,
    pub source_submission_ids: Vec<Uuid>,
    pub source_list_hash: String,
    pub near_status: TraceCreditSettlementNearStatus,
    pub near_outbox_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceUtilityAttestationWrite {
    pub tenant_id: String,
    pub attestation_id: Uuid,
    pub event_type: TraceCreditEventType,
    pub use_category: String,
    pub policy_version: String,
    pub evidence_hash: String,
    pub external_ref_hash: String,
    pub source_submission_ids: Vec<Uuid>,
    pub actor_principal_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceUtilityAttestationRecord {
    pub tenant_id: String,
    pub attestation_id: Uuid,
    pub event_type: TraceCreditEventType,
    pub use_category: String,
    pub policy_version: String,
    pub evidence_hash: String,
    pub external_ref_hash: String,
    pub source_submission_ids: Vec<Uuid>,
    pub actor_principal_ref: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceCreditSettlementBatchWrite {
    pub tenant_id: String,
    pub settlement_batch_id: Uuid,
    pub policy_version: String,
    pub status: TraceCreditSettlementBatchStatus,
    pub reason_hash: String,
    pub source_credit_event_ids: Vec<Uuid>,
    pub source_submission_ids: Vec<Uuid>,
    pub source_list_hash: String,
    pub settled_credit_points: String,
    pub settled_credit_micros: i64,
    pub line_items: Vec<TraceCreditAccountSettlementLineItem>,
    pub near_contract_id: Option<String>,
    pub ranking_model_version: Option<String>,
    pub ranking_target_use: Option<String>,
    pub ranking_calibration_run_id: Option<Uuid>,
    pub ranking_calibration_report_hash: Option<String>,
    pub ranking_calibration_joined_evidence_hash: Option<String>,
    pub ranking_credit_events_excluded_count: u32,
    pub ranking_credit_events_excluded_reason_counts: BTreeMap<String, u32>,
    pub actor_principal_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceCreditSettlementBatchRecord {
    pub tenant_id: String,
    pub settlement_batch_id: Uuid,
    pub policy_version: String,
    pub status: TraceCreditSettlementBatchStatus,
    pub reason_hash: String,
    pub source_credit_event_ids: Vec<Uuid>,
    pub source_submission_ids: Vec<Uuid>,
    pub source_list_hash: String,
    pub settled_credit_points: String,
    pub settled_credit_micros: i64,
    pub line_items: Vec<TraceCreditAccountSettlementLineItem>,
    pub near_contract_id: Option<String>,
    pub ranking_model_version: Option<String>,
    pub ranking_target_use: Option<String>,
    pub ranking_calibration_run_id: Option<Uuid>,
    pub ranking_calibration_report_hash: Option<String>,
    pub ranking_calibration_joined_evidence_hash: Option<String>,
    pub ranking_credit_events_excluded_count: u32,
    pub ranking_credit_events_excluded_reason_counts: BTreeMap<String, u32>,
    pub actor_principal_ref: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceCreditHoldWrite {
    pub tenant_id: String,
    pub hold_id: Uuid,
    pub credit_account_ref: String,
    pub credit_account_hash: String,
    pub reason: TraceCreditHoldReason,
    pub reason_hash: String,
    pub actor_principal_ref: String,
    pub released_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceCreditHoldRecord {
    pub tenant_id: String,
    pub hold_id: Uuid,
    pub credit_account_ref: String,
    pub credit_account_hash: String,
    pub reason: TraceCreditHoldReason,
    pub reason_hash: String,
    pub actor_principal_ref: String,
    pub created_at: DateTime<Utc>,
    pub released_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceNearCreditOutboxItemWrite {
    pub tenant_id: String,
    pub near_outbox_id: Uuid,
    pub settlement_batch_id: Uuid,
    pub credit_account_hash: String,
    pub near_call_json: serde_json::Value,
    pub status: TraceCreditSettlementNearStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceNearCreditOutboxItemRecord {
    pub tenant_id: String,
    pub near_outbox_id: Uuid,
    pub settlement_batch_id: Uuid,
    pub credit_account_hash: String,
    pub near_call_json: serde_json::Value,
    pub status: TraceCreditSettlementNearStatus,
    pub created_at: DateTime<Utc>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub near_transaction_hash: Option<String>,
    pub last_error_hash: Option<String>,
    pub confirmed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceBenchmarkRegistryOutboxItemWrite {
    pub tenant_id: String,
    pub benchmark_outbox_id: Uuid,
    pub conversion_id: Uuid,
    pub operation: TraceBenchmarkRegistryOutboxOperation,
    pub registry_ref: String,
    pub artifact_payload_hash: String,
    pub source_submission_ids_hash: String,
    pub evaluator_ref: Option<String>,
    pub evaluation_score: Option<f32>,
    pub status: TraceBenchmarkRegistryOutboxStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceBenchmarkRegistryOutboxItemRecord {
    pub tenant_id: String,
    pub benchmark_outbox_id: Uuid,
    pub conversion_id: Uuid,
    pub operation: TraceBenchmarkRegistryOutboxOperation,
    pub registry_ref: String,
    pub artifact_payload_hash: String,
    pub source_submission_ids_hash: String,
    pub evaluator_ref: Option<String>,
    pub evaluation_score: Option<f32>,
    pub status: TraceBenchmarkRegistryOutboxStatus,
    pub created_at: DateTime<Utc>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub external_receipt_ref: Option<String>,
    pub last_error_hash: Option<String>,
    pub confirmed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceSubmissionWrite {
    pub tenant_id: String,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub auth_principal_ref: String,
    pub contributor_pseudonym: Option<String>,
    pub submitted_tenant_scope_ref: Option<String>,
    pub schema_version: String,
    pub consent_policy_version: String,
    pub consent_scopes: Vec<String>,
    pub allowed_uses: Vec<String>,
    pub retention_policy_id: String,
    pub status: TraceCorpusStatus,
    pub privacy_risk: String,
    pub redaction_pipeline_version: String,
    pub redaction_counts: BTreeMap<String, u32>,
    pub redaction_hash: String,
    pub canonical_summary_hash: Option<String>,
    pub submission_score: Option<f32>,
    pub credit_points_pending: Option<f32>,
    pub credit_points_final: Option<f32>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceSubmissionRecord {
    pub tenant_id: String,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub status: TraceCorpusStatus,
    pub auth_principal_ref: String,
    pub contributor_pseudonym: Option<String>,
    pub submitted_tenant_scope_ref: Option<String>,
    pub schema_version: String,
    pub consent_policy_version: String,
    pub consent_scopes: Vec<String>,
    pub allowed_uses: Vec<String>,
    pub retention_policy_id: String,
    pub privacy_risk: String,
    pub redaction_pipeline_version: String,
    pub redaction_counts: BTreeMap<String, u32>,
    pub redaction_hash: String,
    pub canonical_summary_hash: Option<String>,
    pub submission_score: Option<f32>,
    pub credit_points_pending: Option<f32>,
    pub credit_points_final: Option<f32>,
    pub received_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub review_assigned_to_principal_ref: Option<String>,
    pub review_assigned_at: Option<DateTime<Utc>>,
    pub review_lease_expires_at: Option<DateTime<Utc>>,
    pub review_due_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub purged_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceTenantPolicyWrite {
    pub tenant_id: String,
    pub policy_version: String,
    pub allowed_consent_scopes: Vec<String>,
    pub allowed_uses: Vec<String>,
    pub updated_by_principal_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceTenantPolicyRecord {
    pub tenant_id: String,
    pub policy_version: String,
    pub allowed_consent_scopes: Vec<String>,
    pub allowed_uses: Vec<String>,
    pub updated_by_principal_ref: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceTenantAccessGrantRole {
    Contributor,
    Reviewer,
    Admin,
    ExportWorker,
    RetentionWorker,
    VectorWorker,
    BenchmarkWorker,
    UtilityWorker,
    ProcessEvalWorker,
    RevocationWorker,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceTenantAccessGrantStatus {
    Active,
    Revoked,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceTenantAccessGrantWrite {
    pub tenant_id: String,
    pub grant_id: Uuid,
    pub principal_ref: String,
    pub role: TraceTenantAccessGrantRole,
    pub status: TraceTenantAccessGrantStatus,
    pub allowed_consent_scopes: Vec<String>,
    pub allowed_uses: Vec<String>,
    pub issuer: Option<String>,
    pub audience: Option<String>,
    pub subject: Option<String>,
    pub issued_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_by_principal_ref: Option<String>,
    pub revoked_by_principal_ref: Option<String>,
    pub reason: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceTenantAccessGrantRecord {
    pub tenant_id: String,
    pub grant_id: Uuid,
    pub principal_ref: String,
    pub role: TraceTenantAccessGrantRole,
    pub status: TraceTenantAccessGrantStatus,
    pub allowed_consent_scopes: Vec<String>,
    pub allowed_uses: Vec<String>,
    pub issuer: Option<String>,
    pub audience: Option<String>,
    pub subject: Option<String>,
    pub issued_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_by_principal_ref: Option<String>,
    pub revoked_by_principal_ref: Option<String>,
    pub reason: Option<String>,
    pub metadata: BTreeMap<String, String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceObjectRefWrite {
    pub object_ref_id: Uuid,
    pub tenant_id: String,
    pub submission_id: Uuid,
    pub artifact_kind: TraceObjectArtifactKind,
    pub object_store: String,
    pub object_key: String,
    pub content_sha256: String,
    pub encryption_key_ref: String,
    pub size_bytes: i64,
    pub compression: Option<String>,
    pub created_by_job_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceObjectRefRecord {
    pub tenant_id: String,
    pub submission_id: Uuid,
    pub object_ref_id: Uuid,
    pub artifact_kind: TraceObjectArtifactKind,
    pub object_store: String,
    pub object_key: String,
    pub content_sha256: String,
    pub encryption_key_ref: String,
    pub size_bytes: i64,
    pub compression: Option<String>,
    pub created_by_job_id: Option<Uuid>,
    pub invalidated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceExportManifestWrite {
    pub tenant_id: String,
    pub export_manifest_id: Uuid,
    pub artifact_kind: TraceObjectArtifactKind,
    pub purpose_code: Option<String>,
    pub audit_event_id: Option<Uuid>,
    pub source_submission_ids: Vec<Uuid>,
    pub source_submission_ids_hash: String,
    pub item_count: u32,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceExportManifestMirrorWrite {
    pub manifest: TraceExportManifestWrite,
    pub object_refs: Vec<TraceObjectRefWrite>,
    pub items: Vec<TraceExportManifestItemWrite>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceExportManifestRecord {
    pub tenant_id: String,
    pub export_manifest_id: Uuid,
    pub artifact_kind: TraceObjectArtifactKind,
    pub purpose_code: Option<String>,
    pub audit_event_id: Option<Uuid>,
    pub source_submission_ids: Vec<Uuid>,
    pub source_submission_ids_hash: String,
    pub item_count: u32,
    pub generated_at: DateTime<Utc>,
    pub invalidated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceExportManifestItemInvalidationReason {
    Revoked,
    Expired,
    Purged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceExportManifestItemWrite {
    pub tenant_id: String,
    pub export_manifest_id: Uuid,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub derived_id: Option<Uuid>,
    pub object_ref_id: Option<Uuid>,
    pub vector_entry_id: Option<Uuid>,
    pub source_status_at_export: TraceCorpusStatus,
    pub source_hash_at_export: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceExportManifestItemRecord {
    pub tenant_id: String,
    pub export_manifest_id: Uuid,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub derived_id: Option<Uuid>,
    pub object_ref_id: Option<Uuid>,
    pub vector_entry_id: Option<Uuid>,
    pub source_status_at_export: TraceCorpusStatus,
    pub source_hash_at_export: String,
    pub source_invalidated_at: Option<DateTime<Utc>>,
    pub source_invalidation_reason: Option<TraceExportManifestItemInvalidationReason>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TenantScopedTraceObjectRef {
    pub tenant_id: String,
    pub submission_id: Uuid,
    pub object_ref_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceDerivedRecordWrite {
    pub derived_id: Uuid,
    pub tenant_id: String,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub status: TraceDerivedStatus,
    pub worker_kind: TraceWorkerKind,
    pub worker_version: String,
    pub input_object_ref: Option<TenantScopedTraceObjectRef>,
    pub input_hash: String,
    pub output_object_ref: Option<TenantScopedTraceObjectRef>,
    pub canonical_summary: Option<String>,
    pub canonical_summary_hash: Option<String>,
    pub summary_model: String,
    pub task_success: Option<String>,
    pub privacy_risk: Option<String>,
    pub event_count: Option<i32>,
    pub tool_sequence: Vec<String>,
    pub tool_categories: Vec<String>,
    pub coverage_tags: Vec<String>,
    pub duplicate_score: Option<f32>,
    pub novelty_score: Option<f32>,
    pub cluster_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceDerivedRecord {
    pub derived_id: Uuid,
    pub tenant_id: String,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub status: TraceDerivedStatus,
    pub worker_kind: TraceWorkerKind,
    pub worker_version: String,
    pub input_object_ref: Option<TenantScopedTraceObjectRef>,
    pub input_hash: String,
    pub output_object_ref: Option<TenantScopedTraceObjectRef>,
    pub canonical_summary: Option<String>,
    pub canonical_summary_hash: Option<String>,
    pub summary_model: String,
    pub task_success: Option<String>,
    pub privacy_risk: Option<String>,
    pub event_count: Option<i32>,
    pub tool_sequence: Vec<String>,
    pub tool_categories: Vec<String>,
    pub coverage_tags: Vec<String>,
    pub duplicate_score: Option<f32>,
    pub novelty_score: Option<f32>,
    pub cluster_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceVectorEntryWrite {
    pub tenant_id: String,
    pub submission_id: Uuid,
    pub derived_id: Uuid,
    pub vector_entry_id: Uuid,
    pub vector_store: String,
    pub embedding_model: String,
    pub embedding_dimension: i32,
    pub embedding_version: String,
    pub source_projection: TraceVectorEntrySourceProjection,
    pub source_hash: String,
    pub status: TraceVectorEntryStatus,
    pub nearest_trace_ids: Vec<String>,
    pub cluster_id: Option<String>,
    pub duplicate_score: Option<f32>,
    pub novelty_score: Option<f32>,
    pub indexed_at: Option<DateTime<Utc>>,
    pub invalidated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceVectorEntryRecord {
    pub tenant_id: String,
    pub submission_id: Uuid,
    pub derived_id: Uuid,
    pub vector_entry_id: Uuid,
    pub vector_store: String,
    pub embedding_model: String,
    pub embedding_dimension: i32,
    pub embedding_version: String,
    pub source_projection: TraceVectorEntrySourceProjection,
    pub source_hash: String,
    pub status: TraceVectorEntryStatus,
    pub nearest_trace_ids: Vec<String>,
    pub cluster_id: Option<String>,
    pub duplicate_score: Option<f32>,
    pub novelty_score: Option<f32>,
    pub indexed_at: Option<DateTime<Utc>>,
    pub invalidated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceAuditEventWrite {
    pub audit_event_id: Uuid,
    pub tenant_id: String,
    pub actor_principal_ref: String,
    pub actor_role: String,
    pub action: TraceAuditAction,
    pub reason: Option<String>,
    pub request_id: Option<String>,
    pub submission_id: Option<Uuid>,
    pub object_ref_id: Option<Uuid>,
    pub export_manifest_id: Option<Uuid>,
    pub decision_inputs_hash: Option<String>,
    pub previous_event_hash: Option<String>,
    pub event_hash: Option<String>,
    pub canonical_event_json: Option<String>,
    pub metadata: TraceAuditSafeMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceAuditEventRecord {
    pub audit_event_id: Uuid,
    pub tenant_id: String,
    pub audit_sequence: i64,
    pub actor_principal_ref: String,
    pub actor_role: String,
    pub action: TraceAuditAction,
    pub reason: Option<String>,
    pub request_id: Option<String>,
    pub submission_id: Option<Uuid>,
    pub object_ref_id: Option<Uuid>,
    pub export_manifest_id: Option<Uuid>,
    pub decision_inputs_hash: Option<String>,
    pub previous_event_hash: Option<String>,
    pub event_hash: Option<String>,
    pub canonical_event_json: Option<String>,
    pub metadata: TraceAuditSafeMetadata,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TraceAuditSafeMetadata {
    #[default]
    Empty,
    Submission {
        status: TraceCorpusStatus,
        privacy_risk: String,
    },
    ReviewDecision {
        decision: String,
        resulting_status: TraceCorpusStatus,
        reason_code: Option<String>,
    },
    ReviewLease {
        action: TraceReviewLeaseAuditAction,
        lease_expires_at: Option<DateTime<Utc>>,
        review_due_at: Option<DateTime<Utc>>,
    },
    TraceContentRead {
        surface: String,
        purpose_hash: Option<String>,
    },
    Export {
        artifact_kind: TraceObjectArtifactKind,
        purpose_code: Option<String>,
        item_count: u32,
    },
    Maintenance {
        dry_run: bool,
        action_counts: BTreeMap<String, u32>,
    },
    CreditMutation {
        event_type: TraceCreditEventType,
        credit_points_delta_micros: i64,
        reason_hash: String,
        external_ref_hash: Option<String>,
    },
    ProcessEvaluation {
        evaluator_version_hash: String,
        label_count: u32,
        rating_counts: BTreeMap<String, u32>,
        score_band: Option<String>,
        utility_credit_delta_micros: Option<i64>,
        utility_external_ref_hash: Option<String>,
    },
    TenantPolicy {
        policy_version: String,
        allowed_consent_scope_count: u32,
        allowed_use_count: u32,
        policy_projection_hash: String,
    },
    TenantAccessGrant {
        action: String,
        role: TraceTenantAccessGrantRole,
        status: TraceTenantAccessGrantStatus,
        allowed_consent_scope_count: u32,
        allowed_use_count: u32,
        grant_projection_hash: String,
    },
    RankingWorkerRunRecovery {
        ranking_worker_run_id: Uuid,
        run_kind: TraceRankingWorkerRunKind,
        recovered_status: TraceRankingWorkerRunStatus,
        reason_hash: String,
    },
    RankingCalibrationDatasetQuarantine {
        calibration_dataset_hash: String,
        target_use: String,
        policy_version: String,
        archived_source_manifest_hash: String,
        conflict_key_hash: String,
        reason_hash: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceCreditEventWrite {
    pub credit_event_id: Uuid,
    pub tenant_id: String,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub credit_account_ref: String,
    pub event_type: TraceCreditEventType,
    pub points_delta: String,
    pub reason: String,
    pub external_ref: Option<String>,
    pub actor_principal_ref: String,
    pub actor_role: String,
    pub settlement_state: TraceCreditSettlementState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceCreditEventRecord {
    pub credit_event_id: Uuid,
    pub tenant_id: String,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub credit_account_ref: String,
    pub event_type: TraceCreditEventType,
    pub points_delta: String,
    pub reason: String,
    pub external_ref: Option<String>,
    pub actor_principal_ref: String,
    pub actor_role: String,
    pub settlement_state: TraceCreditSettlementState,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceTombstoneWrite {
    pub tombstone_id: Uuid,
    pub tenant_id: String,
    pub submission_id: Uuid,
    pub trace_id: Option<Uuid>,
    pub redaction_hash: Option<String>,
    pub canonical_summary_hash: Option<String>,
    pub reason: String,
    pub effective_at: DateTime<Utc>,
    pub retain_until: Option<DateTime<Utc>>,
    pub created_by_principal_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceTombstoneRecord {
    pub tombstone_id: Uuid,
    pub tenant_id: String,
    pub submission_id: Uuid,
    pub trace_id: Option<Uuid>,
    pub redaction_hash: Option<String>,
    pub canonical_summary_hash: Option<String>,
    pub reason: String,
    pub effective_at: DateTime<Utc>,
    pub retain_until: Option<DateTime<Utc>>,
    pub created_by_principal_ref: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceRetentionJobStatus {
    Planned,
    Running,
    DryRun,
    Complete,
    Failed,
    Paused,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRetentionJobWrite {
    pub tenant_id: String,
    pub retention_job_id: Uuid,
    pub purpose: String,
    pub dry_run: bool,
    pub status: TraceRetentionJobStatus,
    pub requested_by_principal_ref: String,
    pub requested_by_role: String,
    pub purge_expired_before: Option<DateTime<Utc>>,
    pub prune_export_cache: bool,
    pub max_export_age_hours: Option<i64>,
    pub audit_event_id: Option<Uuid>,
    pub action_counts: BTreeMap<String, u32>,
    pub selected_revoked_count: u32,
    pub selected_expired_count: u32,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRetentionJobRecord {
    pub tenant_id: String,
    pub retention_job_id: Uuid,
    pub purpose: String,
    pub dry_run: bool,
    pub status: TraceRetentionJobStatus,
    pub requested_by_principal_ref: String,
    pub requested_by_role: String,
    pub purge_expired_before: Option<DateTime<Utc>>,
    pub prune_export_cache: bool,
    pub max_export_age_hours: Option<i64>,
    pub audit_event_id: Option<Uuid>,
    pub action_counts: BTreeMap<String, u32>,
    pub selected_revoked_count: u32,
    pub selected_expired_count: u32,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceRetentionJobItemAction {
    Revoke,
    Expire,
    Purge,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceRetentionJobItemStatus {
    Pending,
    Done,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRetentionJobItemWrite {
    pub tenant_id: String,
    pub retention_job_id: Uuid,
    pub submission_id: Uuid,
    pub action: TraceRetentionJobItemAction,
    pub status: TraceRetentionJobItemStatus,
    pub reason: String,
    pub action_counts: BTreeMap<String, u32>,
    pub verified_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRetentionJobItemRecord {
    pub tenant_id: String,
    pub retention_job_id: Uuid,
    pub submission_id: Uuid,
    pub action: TraceRetentionJobItemAction,
    pub status: TraceRetentionJobItemStatus,
    pub reason: String,
    pub action_counts: BTreeMap<String, u32>,
    pub verified_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceExportAccessGrantStatus {
    Active,
    Consumed,
    Revoked,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceExportAccessGrantWrite {
    pub tenant_id: String,
    pub export_job_id: Uuid,
    pub grant_id: Uuid,
    pub caller_principal_ref: String,
    pub requested_dataset_kind: String,
    pub purpose: String,
    pub max_item_cap: Option<u32>,
    pub status: TraceExportAccessGrantStatus,
    pub requested_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceExportAccessGrantRecord {
    pub tenant_id: String,
    pub export_job_id: Uuid,
    pub grant_id: Uuid,
    pub caller_principal_ref: String,
    pub requested_dataset_kind: String,
    pub purpose: String,
    pub max_item_cap: Option<u32>,
    pub status: TraceExportAccessGrantStatus,
    pub requested_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub metadata: BTreeMap<String, String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceExportJobStatus {
    Queued,
    Running,
    Complete,
    Failed,
    Cancelled,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceExportJobWrite {
    pub tenant_id: String,
    pub export_job_id: Uuid,
    pub grant_id: Uuid,
    pub caller_principal_ref: String,
    pub requested_dataset_kind: String,
    pub purpose: String,
    pub max_item_cap: Option<u32>,
    pub status: TraceExportJobStatus,
    pub requested_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
    pub result_manifest_id: Option<Uuid>,
    pub item_count: Option<u32>,
    pub last_error: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceExportJobStatusUpdate {
    pub status: TraceExportJobStatus,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub result_manifest_id: Option<Uuid>,
    pub item_count: Option<u32>,
    pub last_error: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceExportJobRecord {
    pub tenant_id: String,
    pub export_job_id: Uuid,
    pub grant_id: Uuid,
    pub caller_principal_ref: String,
    pub requested_dataset_kind: String,
    pub purpose: String,
    pub max_item_cap: Option<u32>,
    pub status: TraceExportJobStatus,
    pub requested_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
    pub result_manifest_id: Option<Uuid>,
    pub item_count: Option<u32>,
    pub last_error: Option<String>,
    pub metadata: BTreeMap<String, String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceRevocationPropagationTargetKind {
    ObjectRef,
    ExportManifest,
    ExportManifestItem,
    VectorEntry,
    DerivedRecord,
    BenchmarkArtifact,
    RankerArtifact,
    CreditSettlement,
    PhysicalDeleteReceipt,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TraceRevocationPropagationTarget {
    ObjectRef {
        object_ref_id: Uuid,
    },
    ExportManifest {
        export_manifest_id: Uuid,
    },
    ExportManifestItem {
        export_manifest_id: Uuid,
        source_submission_id: Uuid,
    },
    VectorEntry {
        vector_entry_id: Uuid,
    },
    DerivedRecord {
        derived_id: Uuid,
    },
    BenchmarkArtifact {
        derived_id: Option<Uuid>,
        object_ref_id: Option<Uuid>,
        export_manifest_id: Option<Uuid>,
        artifact_ref: Option<String>,
    },
    RankerArtifact {
        export_manifest_id: Option<Uuid>,
        object_ref_id: Option<Uuid>,
        artifact_ref: Option<String>,
    },
    CreditSettlement {
        credit_event_id: Uuid,
        credit_account_ref: String,
        settlement_state_at_selection: TraceCreditSettlementState,
    },
    PhysicalDeleteReceipt {
        object_ref_id: Option<Uuid>,
        object_store: String,
        object_key: String,
        receipt_sha256: String,
    },
}

impl TraceRevocationPropagationTarget {
    pub fn kind(&self) -> TraceRevocationPropagationTargetKind {
        match self {
            Self::ObjectRef { .. } => TraceRevocationPropagationTargetKind::ObjectRef,
            Self::ExportManifest { .. } => TraceRevocationPropagationTargetKind::ExportManifest,
            Self::ExportManifestItem { .. } => {
                TraceRevocationPropagationTargetKind::ExportManifestItem
            }
            Self::VectorEntry { .. } => TraceRevocationPropagationTargetKind::VectorEntry,
            Self::DerivedRecord { .. } => TraceRevocationPropagationTargetKind::DerivedRecord,
            Self::BenchmarkArtifact { .. } => {
                TraceRevocationPropagationTargetKind::BenchmarkArtifact
            }
            Self::RankerArtifact { .. } => TraceRevocationPropagationTargetKind::RankerArtifact,
            Self::CreditSettlement { .. } => TraceRevocationPropagationTargetKind::CreditSettlement,
            Self::PhysicalDeleteReceipt { .. } => {
                TraceRevocationPropagationTargetKind::PhysicalDeleteReceipt
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceRevocationPropagationAction {
    InvalidateMetadata,
    InvalidateExportMembership,
    InvalidateVector,
    InvalidateBenchmarkArtifact,
    InvalidateRankerArtifact,
    ReverseCreditSettlement,
    DeleteObjectPayload,
    RecordPhysicalDeleteReceipt,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceRevocationPropagationItemStatus {
    Pending,
    InProgress,
    Done,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRevocationPropagationItemWrite {
    pub tenant_id: String,
    pub propagation_item_id: Uuid,
    pub source_submission_id: Uuid,
    pub target: TraceRevocationPropagationTarget,
    pub action: TraceRevocationPropagationAction,
    pub status: TraceRevocationPropagationItemStatus,
    pub idempotency_key: String,
    pub reason: String,
    pub attempt_count: u32,
    pub last_error: Option<String>,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub evidence_hash: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRevocationPropagationItemStatusUpdate {
    pub status: TraceRevocationPropagationItemStatus,
    pub attempt_count: u32,
    pub last_error: Option<String>,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub evidence_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRevocationPropagationItemRecord {
    pub tenant_id: String,
    pub propagation_item_id: Uuid,
    pub source_submission_id: Uuid,
    pub trace_id: Uuid,
    pub target_kind: TraceRevocationPropagationTargetKind,
    pub target: TraceRevocationPropagationTarget,
    pub action: TraceRevocationPropagationAction,
    pub status: TraceRevocationPropagationItemStatus,
    pub idempotency_key: String,
    pub reason: String,
    pub attempt_count: u32,
    pub last_error: Option<String>,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub evidence_hash: Option<String>,
    pub metadata: BTreeMap<String, String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TraceArtifactInvalidationCounts {
    pub object_refs_invalidated: u64,
    pub derived_records_invalidated: u64,
}

#[async_trait]
pub trait TraceCorpusStore: Send + Sync {
    async fn upsert_trace_submission(
        &self,
        submission: TraceSubmissionWrite,
    ) -> Result<TraceSubmissionRecord, DatabaseError>;

    async fn get_trace_submission(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Result<Option<TraceSubmissionRecord>, DatabaseError>;

    async fn list_trace_submissions(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceSubmissionRecord>, DatabaseError>;

    async fn upsert_trace_tenant_policy(
        &self,
        policy: TraceTenantPolicyWrite,
    ) -> Result<TraceTenantPolicyRecord, DatabaseError>;

    async fn get_trace_tenant_policy(
        &self,
        tenant_id: &str,
    ) -> Result<Option<TraceTenantPolicyRecord>, DatabaseError>;

    async fn upsert_trace_tenant_access_grant(
        &self,
        grant: TraceTenantAccessGrantWrite,
    ) -> Result<TraceTenantAccessGrantRecord, DatabaseError>;

    async fn list_trace_tenant_access_grants(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceTenantAccessGrantRecord>, DatabaseError>;

    async fn list_active_trace_tenant_access_grants_for_principal(
        &self,
        tenant_id: &str,
        principal_ref: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<TraceTenantAccessGrantRecord>, DatabaseError>;

    async fn list_trace_credit_events(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceCreditEventRecord>, DatabaseError>;

    async fn update_trace_submission_status(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        status: TraceCorpusStatus,
        actor_principal_ref: &str,
        reason: Option<&str>,
    ) -> Result<(), DatabaseError>;

    async fn claim_trace_review_lease(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        actor_principal_ref: &str,
        lease_expires_at: DateTime<Utc>,
        review_due_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<Option<TraceSubmissionRecord>, DatabaseError>;

    async fn release_trace_review_lease(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        actor_principal_ref: &str,
    ) -> Result<Option<TraceSubmissionRecord>, DatabaseError>;

    async fn append_trace_object_ref(
        &self,
        object_ref: TraceObjectRefWrite,
    ) -> Result<(), DatabaseError>;

    async fn list_trace_object_refs(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Result<Vec<TraceObjectRefRecord>, DatabaseError>;

    async fn get_latest_active_trace_object_ref(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        artifact_kind: TraceObjectArtifactKind,
    ) -> Result<Option<TraceObjectRefRecord>, DatabaseError>;

    async fn append_trace_derived_record(
        &self,
        derived_record: TraceDerivedRecordWrite,
    ) -> Result<(), DatabaseError>;

    async fn list_trace_derived_records(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceDerivedRecord>, DatabaseError>;

    async fn upsert_trace_vector_entry(
        &self,
        vector_entry: TraceVectorEntryWrite,
    ) -> Result<TraceVectorEntryRecord, DatabaseError>;

    async fn list_trace_vector_entries(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceVectorEntryRecord>, DatabaseError>;

    async fn upsert_trace_ranking_model_version(
        &self,
        model_version: TraceRankingModelVersionWrite,
    ) -> Result<TraceRankingModelVersionRecord, DatabaseError>;

    async fn list_trace_ranking_model_versions(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceRankingModelVersionRecord>, DatabaseError>;

    async fn upsert_trace_ranking_calibration_dataset(
        &self,
        dataset: TraceRankingCalibrationDatasetWrite,
    ) -> Result<TraceRankingCalibrationDatasetRecord, DatabaseError>;

    async fn update_trace_ranking_calibration_dataset_status(
        &self,
        update: TraceRankingCalibrationDatasetStatusUpdate,
    ) -> Result<TraceRankingCalibrationDatasetRecord, DatabaseError>;

    async fn list_trace_ranking_calibration_datasets(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceRankingCalibrationDatasetRecord>, DatabaseError>;

    async fn upsert_trace_ranking_feature(
        &self,
        feature: TraceRankingFeatureWrite,
    ) -> Result<TraceRankingFeatureRecord, DatabaseError>;

    async fn list_trace_ranking_features(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceRankingFeatureRecord>, DatabaseError>;

    async fn upsert_trace_ranking_prediction(
        &self,
        prediction: TraceRankingPredictionWrite,
    ) -> Result<TraceRankingPredictionRecord, DatabaseError>;

    async fn list_trace_ranking_predictions(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceRankingPredictionRecord>, DatabaseError>;

    async fn upsert_trace_ranking_label(
        &self,
        label: TraceRankingLabelWrite,
    ) -> Result<TraceRankingLabelRecord, DatabaseError>;

    async fn list_trace_ranking_labels(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceRankingLabelRecord>, DatabaseError>;

    async fn upsert_trace_ranking_preference_label(
        &self,
        preference: TraceRankingPreferenceLabelWrite,
    ) -> Result<TraceRankingPreferenceLabelRecord, DatabaseError>;

    async fn list_trace_ranking_preference_labels(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceRankingPreferenceLabelRecord>, DatabaseError>;

    async fn upsert_trace_ranking_calibration_run(
        &self,
        run: TraceRankingCalibrationRunWrite,
    ) -> Result<TraceRankingCalibrationRunRecord, DatabaseError>;

    async fn list_trace_ranking_calibration_runs(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceRankingCalibrationRunRecord>, DatabaseError>;

    async fn upsert_trace_ranking_worker_run(
        &self,
        run: TraceRankingWorkerRunWrite,
    ) -> Result<TraceRankingWorkerRunRecord, DatabaseError>;

    async fn list_trace_ranking_worker_runs(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceRankingWorkerRunRecord>, DatabaseError>;

    async fn upsert_trace_export_manifest(
        &self,
        manifest: TraceExportManifestWrite,
    ) -> Result<TraceExportManifestRecord, DatabaseError>;

    async fn upsert_trace_export_manifest_mirror(
        &self,
        mirror: TraceExportManifestMirrorWrite,
    ) -> Result<TraceExportManifestRecord, DatabaseError>;

    async fn delete_trace_export_manifest_mirror(
        &self,
        tenant_id: &str,
        export_manifest_id: Uuid,
    ) -> Result<(), DatabaseError>;

    async fn list_trace_export_manifests(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceExportManifestRecord>, DatabaseError>;

    async fn upsert_trace_export_manifest_item(
        &self,
        item: TraceExportManifestItemWrite,
    ) -> Result<TraceExportManifestItemRecord, DatabaseError>;

    async fn list_trace_export_manifest_items(
        &self,
        tenant_id: &str,
        export_manifest_id: Uuid,
    ) -> Result<Vec<TraceExportManifestItemRecord>, DatabaseError>;

    async fn invalidate_trace_export_manifests_for_submission(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Result<u64, DatabaseError>;

    async fn invalidate_trace_export_manifest_items_for_submission(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        reason: TraceExportManifestItemInvalidationReason,
    ) -> Result<u64, DatabaseError>;

    async fn invalidate_trace_vector_entries_for_submission(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Result<u64, DatabaseError>;

    async fn invalidate_trace_vector_entry_for_submission(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        vector_entry_id: Uuid,
    ) -> Result<u64, DatabaseError>;

    async fn append_trace_audit_event(
        &self,
        audit_event: TraceAuditEventWrite,
    ) -> Result<(), DatabaseError>;

    async fn list_trace_audit_events(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceAuditEventRecord>, DatabaseError>;

    async fn append_trace_credit_event(
        &self,
        credit_event: TraceCreditEventWrite,
    ) -> Result<(), DatabaseError>;

    async fn upsert_trace_utility_attestation(
        &self,
        attestation: TraceUtilityAttestationWrite,
    ) -> Result<TraceUtilityAttestationRecord, DatabaseError>;

    async fn list_trace_utility_attestations(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceUtilityAttestationRecord>, DatabaseError>;

    async fn upsert_trace_credit_settlement_batch(
        &self,
        batch: TraceCreditSettlementBatchWrite,
    ) -> Result<TraceCreditSettlementBatchRecord, DatabaseError>;

    async fn list_trace_credit_settlement_batches(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceCreditSettlementBatchRecord>, DatabaseError>;

    async fn upsert_trace_credit_hold(
        &self,
        hold: TraceCreditHoldWrite,
    ) -> Result<TraceCreditHoldRecord, DatabaseError>;

    async fn list_trace_credit_holds(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceCreditHoldRecord>, DatabaseError>;

    async fn upsert_trace_near_credit_outbox_item(
        &self,
        item: TraceNearCreditOutboxItemWrite,
    ) -> Result<TraceNearCreditOutboxItemRecord, DatabaseError>;

    async fn list_trace_near_credit_outbox_items(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceNearCreditOutboxItemRecord>, DatabaseError>;

    async fn update_trace_near_credit_outbox_status(
        &self,
        tenant_id: &str,
        near_outbox_id: Uuid,
        status: TraceCreditSettlementNearStatus,
        near_transaction_hash: Option<String>,
        last_error_hash: Option<String>,
    ) -> Result<Option<TraceNearCreditOutboxItemRecord>, DatabaseError>;

    async fn upsert_trace_benchmark_registry_outbox_item(
        &self,
        item: TraceBenchmarkRegistryOutboxItemWrite,
    ) -> Result<TraceBenchmarkRegistryOutboxItemRecord, DatabaseError>;

    async fn list_trace_benchmark_registry_outbox_items(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceBenchmarkRegistryOutboxItemRecord>, DatabaseError>;

    async fn update_trace_benchmark_registry_outbox_status(
        &self,
        tenant_id: &str,
        benchmark_outbox_id: Uuid,
        status: TraceBenchmarkRegistryOutboxStatus,
        external_receipt_ref: Option<String>,
        last_error_hash: Option<String>,
    ) -> Result<Option<TraceBenchmarkRegistryOutboxItemRecord>, DatabaseError>;

    async fn write_trace_tombstone(
        &self,
        tombstone: TraceTombstoneWrite,
    ) -> Result<(), DatabaseError>;

    async fn list_trace_tombstones(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceTombstoneRecord>, DatabaseError>;

    async fn upsert_trace_retention_job(
        &self,
        job: TraceRetentionJobWrite,
    ) -> Result<TraceRetentionJobRecord, DatabaseError>;

    async fn upsert_trace_retention_job_item(
        &self,
        item: TraceRetentionJobItemWrite,
    ) -> Result<TraceRetentionJobItemRecord, DatabaseError>;

    async fn list_trace_retention_jobs(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceRetentionJobRecord>, DatabaseError>;

    async fn list_trace_retention_job_items(
        &self,
        tenant_id: &str,
        retention_job_id: Uuid,
    ) -> Result<Vec<TraceRetentionJobItemRecord>, DatabaseError>;

    async fn upsert_trace_export_access_grant(
        &self,
        grant: TraceExportAccessGrantWrite,
    ) -> Result<TraceExportAccessGrantRecord, DatabaseError>;

    async fn list_trace_export_access_grants(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceExportAccessGrantRecord>, DatabaseError>;

    async fn upsert_trace_export_job(
        &self,
        job: TraceExportJobWrite,
    ) -> Result<TraceExportJobRecord, DatabaseError>;

    async fn list_trace_export_jobs(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceExportJobRecord>, DatabaseError>;

    async fn update_trace_export_job_status(
        &self,
        tenant_id: &str,
        export_job_id: Uuid,
        update: TraceExportJobStatusUpdate,
    ) -> Result<Option<TraceExportJobRecord>, DatabaseError>;

    async fn upsert_trace_revocation_propagation_item(
        &self,
        item: TraceRevocationPropagationItemWrite,
    ) -> Result<TraceRevocationPropagationItemRecord, DatabaseError>;

    async fn list_trace_revocation_propagation_items(
        &self,
        tenant_id: &str,
        source_submission_id: Uuid,
    ) -> Result<Vec<TraceRevocationPropagationItemRecord>, DatabaseError>;

    async fn list_due_trace_revocation_propagation_items(
        &self,
        tenant_id: &str,
        now: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<TraceRevocationPropagationItemRecord>, DatabaseError>;

    async fn update_trace_revocation_propagation_item_status(
        &self,
        tenant_id: &str,
        propagation_item_id: Uuid,
        update: TraceRevocationPropagationItemStatusUpdate,
    ) -> Result<Option<TraceRevocationPropagationItemRecord>, DatabaseError>;

    async fn invalidate_trace_submission_artifacts(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        derived_status: TraceDerivedStatus,
    ) -> Result<TraceArtifactInvalidationCounts, DatabaseError>;

    async fn mark_trace_object_ref_deleted(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        object_store: &str,
        object_key: &str,
    ) -> Result<u64, DatabaseError>;
}
