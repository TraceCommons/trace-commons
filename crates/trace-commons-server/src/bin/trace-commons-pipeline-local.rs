//! Local/test HTTP runner and corpus client for the versioned pipeline.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use trace_commons_gate_api::pipeline::{Microcredits, Phase, ReasonCode, ReviewRecommendation};
use trace_commons_protocol::trace_contribution::{
    DeterministicTraceRedactor, RawTraceCaptureTurn, RawTraceContribution,
    RecordedTraceContributionOptions, ResidualPiiRisk, TraceRedactor,
};
use trace_commons_server::config::DatabaseConfig;
use trace_commons_server::db::Database;
use trace_commons_server::db::postgres::PgBackend;
use trace_commons_server::secrets::SecretsCrypto;
use trace_commons_server::trace_artifact_store::{
    LocalEncryptedTraceArtifactStore, TraceArtifactStore,
};
use trace_commons_server::trace_corpus_storage::TraceCorpusStore;
use trace_commons_server::trace_score_attestation::{
    ATTESTATION_SIGNING_KEY_UNCONFIGURED, AttestationConfig, AttestationSigningState,
    sign_versioned_score_attestation,
};
use trace_commons_server::versioned_pipeline::{
    PhaseOutcomeRecord, PipelineInspection, PipelineReceiptResult, PipelineReviewClaim,
    PipelineRunState, PipelineService, PipelineSubmitReceipt,
};
use trace_commons_server::versioned_pipeline_product::{
    PIPELINE_EXPORT_ITEM_MAX, PipelineContributorStatus, PipelineExportSnapshot,
    PipelineProductStore, sha256_prefixed as product_sha256_prefixed,
};
use trace_commons_server::versioned_pipeline_qualification::ProductionDependencyProfile;
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(name = "trace-commons-pipeline-local")]
#[command(about = "Local/test-only versioned pipeline")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Serve(ServeArgs),
    Corpus(CorpusArgs),
}

#[derive(Debug, clap::Args)]
struct ServeArgs {
    #[arg(long, env = "TRACE_COMMONS_DATABASE_URL")]
    database_url: String,
    #[arg(long, default_value = "127.0.0.1:3917")]
    bind: SocketAddr,
    #[arg(long, default_value = ".local/pipeline-artifacts")]
    artifact_root: PathBuf,
    #[arg(long)]
    allow_minimal_policies: bool,
    #[arg(long)]
    compatibility_policies: bool,
    #[arg(long)]
    skip_migrations: bool,
    #[arg(long)]
    fail_phase: Option<PhaseArg>,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PhaseArg {
    Admission,
    Review,
    Score,
    Settle,
}

impl From<PhaseArg> for Phase {
    fn from(value: PhaseArg) -> Self {
        match value {
            PhaseArg::Admission => Self::Admission,
            PhaseArg::Review => Self::Review,
            PhaseArg::Score => Self::Score,
            PhaseArg::Settle => Self::Settle,
        }
    }
}

#[derive(Debug, clap::Args)]
struct CorpusArgs {
    #[arg(long, default_value = "http://127.0.0.1:3917")]
    base_url: String,
    #[arg(long, env = "TRACE_COMMONS_PIPELINE_CORPUS_SUBMIT_TOKEN")]
    submit_token: String,
    #[arg(long, env = "TRACE_COMMONS_PIPELINE_CORPUS_WORKER_TOKEN")]
    worker_token: String,
    #[arg(long, env = "TRACE_COMMONS_PIPELINE_CORPUS_REVIEWER_TOKEN")]
    reviewer_token: String,
    #[arg(long, env = "TRACE_COMMONS_PIPELINE_CORPUS_INSPECT_TOKEN")]
    inspect_token: String,
    #[arg(long, default_value = "docs/redesign/fixtures/minimal-corpus-v1.json")]
    fixtures: PathBuf,
    #[arg(long, default_value = ".local/pipeline-report-v4.json")]
    json_report: PathBuf,
    #[arg(long, default_value = ".local/pipeline-report-v4.md")]
    markdown_report: PathBuf,
    #[arg(long, default_value_t = 30)]
    timeout_seconds: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalRole {
    Contributor,
    Reviewer,
    Worker,
    Operator,
    Exporter,
    LifecycleWorker,
}

#[derive(Debug, Clone)]
struct LocalAuth {
    tenant_id: String,
    principal_ref: String,
    role: LocalRole,
}

struct HttpState {
    pipeline: Arc<PipelineService>,
    backend: Arc<PgBackend>,
    product: PipelineProductStore,
    attestation_signing: Option<AttestationSigningState>,
    tokens: BTreeMap<String, LocalAuth>,
}

#[derive(Debug)]
struct HttpError {
    status: StatusCode,
    label: &'static str,
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.label })),
        )
            .into_response()
    }
}

type HttpResult<T> = Result<T, HttpError>;

#[derive(Debug, Deserialize)]
struct WorkerQuery {
    stop_before: Option<PhaseArg>,
    #[serde(default = "default_worker_limit")]
    limit: u16,
}

const fn default_worker_limit() -> u16 {
    1
}

#[derive(Debug, Deserialize)]
struct ReviewAssessmentRequest {
    lease_token: Uuid,
    recommendation: ReviewRecommendation,
    reason_code: String,
    resolved_quarantine_reasons: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SubmissionStatusRequest {
    submission_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize)]
struct ExportSnapshotRequest {
    allowed_use: String,
    purpose: String,
    #[serde(default = "default_export_limit")]
    limit: usize,
}

const fn default_export_limit() -> usize {
    PIPELINE_EXPORT_ITEM_MAX
}

#[derive(Debug, Serialize)]
struct ScoreAttestationResponse {
    attestation: String,
}

#[derive(Debug, Serialize)]
struct PipelineReadinessResponse {
    live: bool,
    database: bool,
    package_integrity: bool,
    production_eligible: bool,
    safe_blockers: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CorpusFile {
    schema: String,
    fixtures: Vec<CorpusFixture>,
}

#[derive(Debug, Deserialize)]
struct CorpusFixture {
    label: String,
    trace_id: Uuid,
    submission_id: Uuid,
    created_at: DateTime<Utc>,
    input: String,
    secret_probe: String,
    #[serde(default = "default_privacy_risk")]
    privacy_risk: String,
    #[serde(default = "default_admission_decision")]
    expected_admission_decision: String,
    #[serde(default = "default_outcome_count")]
    expected_outcome_count: usize,
    #[serde(default)]
    review_recommendation: Option<ReviewRecommendation>,
}

fn default_privacy_risk() -> String {
    "low".to_string()
}

fn default_admission_decision() -> String {
    "admit".to_string()
}

const fn default_outcome_count() -> usize {
    4
}

#[derive(Debug, Serialize)]
struct CorpusReport {
    schema: &'static str,
    corpus_digest: String,
    code_revision: &'static str,
    bundle_id: String,
    configuration_identities: BTreeMap<&'static str, &'static str>,
    expected_fixture_count: usize,
    completed_fixture_count: usize,
    failure_count: usize,
    duration_ms: u128,
    fixtures: Vec<FixtureReport>,
}

#[derive(Debug, Serialize)]
struct FixtureReport {
    label: String,
    request_content_hash: String,
    run_id: Uuid,
    submission_id: Uuid,
    state: PipelineRunState,
    phase_count: usize,
    admission_decision: String,
    expected_admission_decision: String,
    expected_outcome_count: usize,
    phases: Vec<PhaseReport>,
    approved_revision_id: Option<Uuid>,
    index_membership: String,
    score_microcredits: Option<u64>,
    finalized_microcredits: Option<u64>,
    batch_hash: Option<String>,
    index_command_hash: Option<String>,
    index_write_state: String,
    credit_write_state: String,
    payout_state: String,
    public_processing_state:
        trace_commons_server::versioned_pipeline_product::PipelineProcessingStatus,
    public_credit_state: trace_commons_server::versioned_pipeline_product::PipelineCreditStatus,
    public_payout_state: Option<String>,
    replay_same_run: bool,
    changed_content_refused: bool,
    failure_label: Option<String>,
    attempt_count: u32,
    max_attempts: u32,
    next_attempt_at: DateTime<Utc>,
    time_in_phase_ms: i64,
}

#[derive(Debug, Serialize)]
struct PhaseReport {
    outcome_id: Uuid,
    phase: Phase,
    outcome_schema_id: String,
    outcome_schema_version: u32,
    decision: serde_json::Value,
    evidence: serde_json::Value,
    evaluation: serde_json::Value,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
    let cli = Cli::parse();
    match cli.command {
        Command::Serve(args) => serve(args).await,
        Command::Corpus(args) => run_corpus(args).await,
    }
}

async fn serve(args: ServeArgs) -> anyhow::Result<()> {
    anyhow::ensure!(
        args.allow_minimal_policies,
        "minimal policies require --allow-minimal-policies"
    );
    anyhow::ensure!(
        args.bind.ip() == IpAddr::from([127, 0, 0, 1]) || args.bind.ip().is_loopback(),
        "minimal policies may bind only to loopback"
    );
    anyhow::ensure!(
        !matches!(args.fail_phase, Some(PhaseArg::Admission)),
        "pipeline failure injection supports asynchronous phases only"
    );
    let master_key = std::env::var("TRACE_COMMONS_PIPELINE_MASTER_KEY")
        .map_err(|_| anyhow::anyhow!("TRACE_COMMONS_PIPELINE_MASTER_KEY is required"))?;
    let tokens = parse_tokens(
        &std::env::var("TRACE_COMMONS_PIPELINE_TOKENS")
            .map_err(|_| anyhow::anyhow!("TRACE_COMMONS_PIPELINE_TOKENS is required"))?,
    )?;
    let backend =
        Arc::new(PgBackend::new(&DatabaseConfig::from_postgres_url(&args.database_url, 8)).await?);
    if !args.skip_migrations {
        backend.run_migrations().await?;
    }
    let artifact_store: Arc<dyn TraceArtifactStore> =
        Arc::new(LocalEncryptedTraceArtifactStore::new(
            args.artifact_root,
            SecretsCrypto::new(SecretString::from(master_key))?,
        ));
    let pipeline = Arc::new(if args.compatibility_policies {
        anyhow::ensure!(
            args.fail_phase.is_none(),
            "compatibility policies do not support --fail-phase"
        );
        PipelineService::new_compatibility(backend.clone(), artifact_store)?
    } else {
        PipelineService::new(
            backend.clone(),
            artifact_store,
            args.fail_phase.map(Into::into),
        )?
    });
    let attestation_signing = AttestationConfig::from_env()?
        .as_ref()
        .map(AttestationSigningState::build)
        .transpose()?;
    let bundle_id = pipeline.bundle_id().to_string();
    let state = Arc::new(HttpState {
        pipeline,
        product: PipelineProductStore::new(backend.clone()),
        backend,
        attestation_signing,
        tokens,
    });
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/health", get(|| async { "ok" }))
        .route("/ready", get(readiness_handler))
        .route("/v1/pipeline/submissions", post(submit_handler))
        .route("/v1/traces", post(submit_handler))
        .route("/v1/traces/{submission_id}", delete(withdraw_handler))
        .route(
            "/v1/account/traces/{submission_id}/withdraw",
            post(withdraw_handler),
        )
        .route(
            "/v1/contributors/me/submission-status",
            post(submission_status_handler),
        )
        .route(
            "/v1/contributors/me/credit",
            get(contributor_credit_handler),
        )
        .route(
            "/v1/contributors/me/credit-events",
            get(contributor_credit_events_handler),
        )
        .route(
            "/v1/contributors/me/score-attestation",
            get(score_attestation_handler),
        )
        .route(
            "/.well-known/trace-commons-attestation-keyset.json",
            get(attestation_keyset_handler),
        )
        .route("/v1/exports", post(create_export_snapshot_handler))
        .route(
            "/v1/exports/{snapshot_id}/complete",
            post(complete_export_snapshot_handler),
        )
        .route(
            "/v1/admin/pipeline-lifecycle-summary",
            get(lifecycle_summary_handler),
        )
        .route(
            "/v1/admin/pipeline-operational-summary",
            get(operational_summary_handler),
        )
        .route(
            "/v1/admin/pipeline-runs/{run_id}/traceability",
            get(forensic_trace_handler),
        )
        .route(
            "/v1/workers/pipeline-index-invalidation/{run_id}",
            post(index_invalidation_handler),
        )
        .route("/v1/pipeline/worker", post(worker_handler))
        .route("/v1/pipeline/runs/{run_id}", get(inspect_handler))
        .route(
            "/v1/pipeline/runs/{run_id}/review-claim",
            post(review_claim_handler),
        )
        .route(
            "/v1/pipeline/runs/{run_id}/review-assessment",
            post(review_assessment_handler),
        )
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(args.bind).await?;
    tracing::info!(
        bind = %args.bind,
        bundle_id = %bundle_id,
        "local versioned pipeline listening"
    );
    axum::serve(listener, app).await?;
    Ok(())
}

async fn readiness_handler(
    State(state): State<Arc<HttpState>>,
) -> (StatusCode, Json<PipelineReadinessResponse>) {
    let database = state.backend.readiness_probe().await.is_ok();
    let package_integrity = state.pipeline.default_package_hash().is_ok();
    let mut safe_blockers = ProductionDependencyProfile::local_test().blockers();
    if !database {
        safe_blockers.push("database_unavailable".to_string());
    }
    if !package_integrity {
        safe_blockers.push("bundle_package_invalid".to_string());
    }
    let response = PipelineReadinessResponse {
        live: true,
        database,
        package_integrity,
        production_eligible: safe_blockers.is_empty(),
        safe_blockers,
    };
    let status = if database {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(response))
}

async fn submit_handler(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    body: Bytes,
) -> HttpResult<Json<PipelineSubmitReceipt>> {
    let auth = authenticate(&state, &headers, LocalRole::Contributor)?;
    let idempotency_key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .ok_or(HttpError {
            status: StatusCode::BAD_REQUEST,
            label: "idempotency_key_required",
        })?;
    let result = state
        .pipeline
        .submit(&auth.tenant_id, &auth.principal_ref, idempotency_key, &body)
        .await
        .map_err(|_| HttpError {
            status: StatusCode::BAD_REQUEST,
            label: "submission_failed",
        })?;
    match result {
        PipelineReceiptResult::Created(run) => Ok(Json(receipt(run, false))),
        PipelineReceiptResult::Replayed(run) => Ok(Json(receipt(run, true))),
        PipelineReceiptResult::ContentConflict => Err(HttpError {
            status: StatusCode::CONFLICT,
            label: "idempotency_content_conflict",
        }),
    }
}

async fn withdraw_handler(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    AxumPath(submission_id): AxumPath<Uuid>,
) -> HttpResult<Json<trace_commons_server::trace_corpus_storage::TraceWithdrawalRecord>> {
    let auth = authenticate(&state, &headers, LocalRole::Contributor)?;
    let withdrawal = state
        .pipeline
        .withdraw_submission(&auth.tenant_id, submission_id, &auth.principal_ref)
        .await
        .map_err(|_| HttpError {
            status: StatusCode::NOT_FOUND,
            label: "submission_not_found",
        })?;
    Ok(Json(withdrawal))
}

async fn submission_status_handler(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    Json(body): Json<SubmissionStatusRequest>,
) -> HttpResult<
    Json<Vec<trace_commons_server::versioned_pipeline_product::PipelineContributorStatus>>,
> {
    let auth = authenticate(&state, &headers, LocalRole::Contributor)?;
    if body.submission_ids.len()
        > trace_commons_server::versioned_pipeline_product::PIPELINE_STATUS_BATCH_MAX
    {
        return Err(HttpError {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            label: "submission_status_batch_too_large",
        });
    }
    let statuses = state
        .product
        .contributor_statuses(&auth.tenant_id, &auth.principal_ref, &body.submission_ids)
        .await
        .map_err(|_| HttpError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            label: "submission_status_unavailable",
        })?;
    Ok(Json(statuses))
}

async fn contributor_credit_handler(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
) -> HttpResult<Json<trace_commons_server::versioned_pipeline_product::PipelineContributorCredit>> {
    let auth = authenticate(&state, &headers, LocalRole::Contributor)?;
    let credit = state
        .product
        .contributor_credit(&auth.tenant_id, &auth.principal_ref)
        .await
        .map_err(|_| HttpError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            label: "contributor_credit_unavailable",
        })?;
    Ok(Json(credit))
}

async fn contributor_credit_events_handler(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
) -> HttpResult<
    Json<Vec<trace_commons_server::versioned_pipeline_product::PipelineContributorStatus>>,
> {
    let auth = authenticate(&state, &headers, LocalRole::Contributor)?;
    let statuses = state
        .product
        .own_contributor_statuses(&auth.tenant_id, &auth.principal_ref)
        .await
        .map_err(|_| HttpError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            label: "contributor_credit_events_unavailable",
        })?
        .into_iter()
        .filter(|status| status.score_outcome_id.is_some())
        .collect();
    Ok(Json(statuses))
}

async fn score_attestation_handler(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
) -> HttpResult<Json<ScoreAttestationResponse>> {
    let auth = authenticate(&state, &headers, LocalRole::Contributor)?;
    let signer = state.attestation_signing.as_ref().ok_or(HttpError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        label: ATTESTATION_SIGNING_KEY_UNCONFIGURED,
    })?;
    let entries = state
        .product
        .own_score_attestation_entries(&auth.tenant_id, &auth.principal_ref)
        .await
        .map_err(|_| HttpError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            label: "score_attestation_unavailable",
        })?;
    let attestation = sign_versioned_score_attestation(
        signer,
        &auth.tenant_id,
        &auth.principal_ref,
        entries,
        Utc::now(),
    )
    .map_err(|_| HttpError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        label: "score_attestation_signing_failed",
    })?;
    Ok(Json(ScoreAttestationResponse { attestation }))
}

async fn attestation_keyset_handler(
    State(state): State<Arc<HttpState>>,
) -> HttpResult<Json<serde_json::Value>> {
    let signer = state.attestation_signing.as_ref().ok_or(HttpError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        label: ATTESTATION_SIGNING_KEY_UNCONFIGURED,
    })?;
    Ok(Json(signer.keyset_json()))
}

async fn create_export_snapshot_handler(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    Json(body): Json<ExportSnapshotRequest>,
) -> HttpResult<Json<PipelineExportSnapshot>> {
    let auth = authenticate(&state, &headers, LocalRole::Exporter)?;
    let idempotency_key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .ok_or(HttpError {
            status: StatusCode::BAD_REQUEST,
            label: "idempotency_key_required",
        })?;
    let request_key_hash = product_sha256_prefixed(idempotency_key.as_bytes());
    let purpose_hash = product_sha256_prefixed(body.purpose.as_bytes());
    let snapshot = state
        .product
        .create_export_snapshot(
            &auth.tenant_id,
            &auth.principal_ref,
            &request_key_hash,
            &body.allowed_use,
            &purpose_hash,
            body.limit,
        )
        .await
        .map_err(|_| HttpError {
            status: StatusCode::BAD_REQUEST,
            label: "export_snapshot_failed",
        })?;
    Ok(Json(snapshot))
}

async fn complete_export_snapshot_handler(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    AxumPath(snapshot_id): AxumPath<Uuid>,
) -> HttpResult<Json<PipelineExportSnapshot>> {
    let auth = authenticate(&state, &headers, LocalRole::Exporter)?;
    let snapshot = state
        .product
        .complete_export_snapshot(&auth.tenant_id, &auth.principal_ref, snapshot_id)
        .await
        .map_err(|_| HttpError {
            status: StatusCode::NOT_FOUND,
            label: "export_snapshot_not_found",
        })?;
    Ok(Json(snapshot))
}

async fn lifecycle_summary_handler(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
) -> HttpResult<Json<trace_commons_server::versioned_pipeline_product::PipelineLifecycleSummary>> {
    let auth = authenticate(&state, &headers, LocalRole::Operator)?;
    let summary = state
        .product
        .lifecycle_summary(&auth.tenant_id)
        .await
        .map_err(|_| HttpError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            label: "lifecycle_summary_unavailable",
        })?;
    Ok(Json(summary))
}

async fn operational_summary_handler(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
) -> HttpResult<Json<trace_commons_server::versioned_pipeline_product::PipelineOperationalSummary>>
{
    let auth = authenticate(&state, &headers, LocalRole::Operator)?;
    state
        .product
        .operational_summary(&auth.tenant_id)
        .await
        .map(Json)
        .map_err(|_| HttpError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            label: "operational_summary_unavailable",
        })
}

async fn forensic_trace_handler(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    AxumPath(run_id): AxumPath<Uuid>,
) -> HttpResult<Json<trace_commons_server::versioned_pipeline_product::PipelineForensicTrace>> {
    let auth = authenticate(&state, &headers, LocalRole::Operator)?;
    state
        .product
        .forensic_trace(&auth.tenant_id, run_id)
        .await
        .map_err(|_| HttpError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            label: "traceability_unavailable",
        })?
        .map(Json)
        .ok_or(HttpError {
            status: StatusCode::NOT_FOUND,
            label: "pipeline_run_not_found",
        })
}

async fn index_invalidation_handler(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    AxumPath(run_id): AxumPath<Uuid>,
) -> HttpResult<Json<trace_commons_server::versioned_pipeline::PipelineRunRecord>> {
    let auth = authenticate(&state, &headers, LocalRole::LifecycleWorker)?;
    let run = state
        .pipeline
        .process_index_invalidation(&auth.tenant_id, run_id)
        .await
        .map_err(|_| HttpError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            label: "index_invalidation_failed",
        })?
        .ok_or(HttpError {
            status: StatusCode::NOT_FOUND,
            label: "pipeline_run_not_found",
        })?;
    Ok(Json(run))
}

async fn worker_handler(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    Query(query): Query<WorkerQuery>,
) -> HttpResult<Json<Option<PipelineInspection>>> {
    let auth = authenticate(&state, &headers, LocalRole::Worker)?;
    if query.limit != 1 {
        return Err(HttpError {
            status: StatusCode::BAD_REQUEST,
            label: "worker_limit_invalid",
        });
    }
    let processed = state
        .pipeline
        .process_one(&auth.tenant_id, query.stop_before.map(Into::into))
        .await
        .map_err(|_| HttpError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            label: "worker_failed",
        })?;
    let inspection = match processed {
        Some(run) => state
            .pipeline
            .inspect(&auth.tenant_id, run.run_id)
            .await
            .map_err(|_| HttpError {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                label: "inspection_failed",
            })?,
        None => None,
    };
    Ok(Json(inspection))
}

async fn inspect_handler(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    AxumPath(run_id): AxumPath<Uuid>,
) -> HttpResult<Json<PipelineInspection>> {
    let auth = authenticate_any(&state, &headers)?;
    let inspection = state
        .pipeline
        .inspect(&auth.tenant_id, run_id)
        .await
        .map_err(|_| HttpError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            label: "inspection_failed",
        })?
        .ok_or(HttpError {
            status: StatusCode::NOT_FOUND,
            label: "pipeline_run_not_found",
        })?;
    if auth.role == LocalRole::Contributor {
        let submission = state
            .backend
            .get_trace_submission(&auth.tenant_id, inspection.run.submission_id)
            .await
            .map_err(|_| HttpError {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                label: "inspection_failed",
            })?;
        if !submission.is_some_and(|row| row.auth_principal_ref == auth.principal_ref) {
            return Err(HttpError {
                status: StatusCode::NOT_FOUND,
                label: "pipeline_run_not_found",
            });
        }
    }
    Ok(Json(inspection))
}

async fn review_claim_handler(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    AxumPath(run_id): AxumPath<Uuid>,
) -> HttpResult<Json<PipelineReviewClaim>> {
    let auth = authenticate(&state, &headers, LocalRole::Reviewer)?;
    state
        .pipeline
        .claim_review(
            &auth.tenant_id,
            run_id,
            &auth.principal_ref,
            chrono::Duration::minutes(5),
        )
        .await
        .map_err(|_| HttpError {
            status: StatusCode::CONFLICT,
            label: "review_claim_failed",
        })?
        .map(Json)
        .ok_or(HttpError {
            status: StatusCode::CONFLICT,
            label: "review_claim_unavailable",
        })
}

async fn review_assessment_handler(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    AxumPath(run_id): AxumPath<Uuid>,
    Json(body): Json<ReviewAssessmentRequest>,
) -> HttpResult<StatusCode> {
    let auth = authenticate(&state, &headers, LocalRole::Reviewer)?;
    let reason = ReasonCode::new(body.reason_code).map_err(|_| HttpError {
        status: StatusCode::BAD_REQUEST,
        label: "review_reason_invalid",
    })?;
    let resolved = body
        .resolved_quarantine_reasons
        .into_iter()
        .map(ReasonCode::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| HttpError {
            status: StatusCode::BAD_REQUEST,
            label: "review_reason_invalid",
        })?;
    state
        .pipeline
        .record_review_assessment(
            &PipelineReviewClaim {
                tenant_id: auth.tenant_id,
                run_id,
                reviewer_principal_ref: auth.principal_ref,
                lease_token: body.lease_token,
                lease_expires_at: Utc::now(),
            },
            body.recommendation,
            reason,
            resolved,
        )
        .await
        .map_err(|_| HttpError {
            status: StatusCode::CONFLICT,
            label: "review_assessment_failed",
        })?;
    Ok(StatusCode::NO_CONTENT)
}

fn authenticate(
    state: &HttpState,
    headers: &HeaderMap,
    required: LocalRole,
) -> HttpResult<LocalAuth> {
    let auth = authenticate_any(state, headers)?;
    if auth.role != required {
        return Err(HttpError {
            status: StatusCode::FORBIDDEN,
            label: "role_forbidden",
        });
    }
    Ok(auth)
}

fn authenticate_any(state: &HttpState, headers: &HeaderMap) -> HttpResult<LocalAuth> {
    let token = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(HttpError {
            status: StatusCode::UNAUTHORIZED,
            label: "authentication_required",
        })?;
    state.tokens.get(token).cloned().ok_or(HttpError {
        status: StatusCode::UNAUTHORIZED,
        label: "authentication_required",
    })
}

fn parse_tokens(value: &str) -> anyhow::Result<BTreeMap<String, LocalAuth>> {
    let mut tokens = BTreeMap::new();
    for entry in value.split(';').filter(|entry| !entry.trim().is_empty()) {
        let fields = entry.split(',').collect::<Vec<_>>();
        anyhow::ensure!(
            fields.len() == 4,
            "pipeline token entries require token,tenant,principal,role"
        );
        let role = match fields[3] {
            "contributor" => LocalRole::Contributor,
            "reviewer" => LocalRole::Reviewer,
            "worker" => LocalRole::Worker,
            "operator" => LocalRole::Operator,
            "exporter" => LocalRole::Exporter,
            "lifecycle_worker" => LocalRole::LifecycleWorker,
            _ => anyhow::bail!("unknown pipeline token role"),
        };
        anyhow::ensure!(
            !fields[..3].iter().any(|field| field.trim().is_empty()),
            "pipeline token fields cannot be empty"
        );
        anyhow::ensure!(
            is_safe_principal_ref(fields[2]),
            "pipeline principal must be a hash-shaped role reference"
        );
        tokens.insert(
            fields[0].to_string(),
            LocalAuth {
                tenant_id: fields[1].to_string(),
                principal_ref: fields[2].to_string(),
                role,
            },
        );
    }
    anyhow::ensure!(
        !tokens.is_empty(),
        "at least one pipeline token is required"
    );
    Ok(tokens)
}

fn is_safe_principal_ref(value: &str) -> bool {
    [
        "principal_sha256:",
        "reviewer_sha256:",
        "worker_sha256:",
        "operator_sha256:",
        "exporter_sha256:",
        "lifecycle_worker_sha256:",
    ]
    .iter()
    .find_map(|prefix| value.strip_prefix(prefix))
    .is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    })
}

fn receipt(
    run: trace_commons_server::versioned_pipeline::PipelineRunRecord,
    replayed: bool,
) -> PipelineSubmitReceipt {
    PipelineSubmitReceipt {
        run_id: run.run_id,
        submission_id: run.submission_id,
        bundle_id: run.bundle_id,
        request_content_hash: run.request_content_hash,
        replayed,
        state: run.state,
        next_phase: run.next_phase,
    }
}

async fn run_corpus(args: CorpusArgs) -> anyhow::Result<()> {
    let started = Instant::now();
    let fixture_bytes = tokio::fs::read(&args.fixtures).await?;
    let corpus_digest = sha256_prefixed(&fixture_bytes);
    let corpus: CorpusFile = serde_json::from_slice(&fixture_bytes)?;
    anyhow::ensure!(
        corpus.schema == "trace_commons.pipeline_corpus.v1",
        "unsupported corpus schema"
    );
    let client = reqwest::Client::new();
    let mut reports = Vec::new();
    let mut bundle_id = None;
    for fixture in &corpus.fixtures {
        let request_bytes = build_request_bytes(fixture).await?;
        anyhow::ensure!(
            !request_bytes
                .windows(fixture.secret_probe.len())
                .any(|window| window == fixture.secret_probe.as_bytes()),
            "fixture secret survived local redaction"
        );
        let key = format!("corpus-v1:{}", fixture.label);
        let first = submit_http(
            &client,
            &args.base_url,
            &args.submit_token,
            &key,
            &request_bytes,
        )
        .await?;
        bundle_id.get_or_insert_with(|| first.bundle_id.clone());
        let replay = submit_http(
            &client,
            &args.base_url,
            &args.submit_token,
            &key,
            &request_bytes,
        )
        .await?;
        let mut changed_request = request_bytes.clone();
        changed_request.push(b' ');
        let changed_content_refused = submit_conflict_http(
            &client,
            &args.base_url,
            &args.submit_token,
            &key,
            &changed_request,
        )
        .await?;
        if let Some(recommendation) = fixture.review_recommendation {
            let claim =
                claim_review_http(&client, &args.base_url, &args.reviewer_token, first.run_id)
                    .await?;
            submit_review_assessment_http(
                &client,
                &args.base_url,
                &args.reviewer_token,
                first.run_id,
                &claim,
                recommendation,
            )
            .await?;
        }
        let deadline = Instant::now() + Duration::from_secs(args.timeout_seconds);
        let inspection = loop {
            let current =
                inspect_http(&client, &args.base_url, &args.inspect_token, first.run_id).await?;
            if matches!(
                current.run.state,
                PipelineRunState::Complete | PipelineRunState::Failed
            ) {
                break current;
            }
            anyhow::ensure!(Instant::now() < deadline, "pipeline completion timed out");
            run_worker_http(&client, &args.base_url, &args.worker_token).await?;
            tokio::time::sleep(Duration::from_millis(25)).await;
        };
        let public_status = submission_status_http(
            &client,
            &args.base_url,
            &args.submit_token,
            first.submission_id,
        )
        .await?;
        reports.push(fixture_report(
            fixture,
            inspection,
            public_status,
            replay.run_id == first.run_id,
            changed_content_refused,
        )?);
    }
    let failure_count = reports
        .iter()
        .filter(|report| {
            let expected_public_processing = if report.expected_outcome_count < 4 {
                trace_commons_server::versioned_pipeline_product::PipelineProcessingStatus::Rejected
            } else {
                trace_commons_server::versioned_pipeline_product::PipelineProcessingStatus::Complete
            };
            report.state != PipelineRunState::Complete
                || report.phase_count != report.expected_outcome_count
                || report.admission_decision != report.expected_admission_decision
                || report.public_processing_state != expected_public_processing
                || !report.replay_same_run
                || !report.changed_content_refused
        })
        .count();
    let report = CorpusReport {
        schema: "trace_commons.pipeline_corpus_report.v4",
        corpus_digest,
        code_revision: trace_commons_build_info::COMMIT,
        bundle_id: bundle_id.unwrap_or_default(),
        configuration_identities: BTreeMap::from([
            ("artifact_store", "local_encrypted_v1"),
            ("database", "postgresql"),
            ("external_payout", "disabled"),
        ]),
        expected_fixture_count: corpus.fixtures.len(),
        completed_fixture_count: reports
            .iter()
            .filter(|report| report.state == PipelineRunState::Complete)
            .count(),
        failure_count,
        duration_ms: started.elapsed().as_millis(),
        fixtures: reports,
    };
    let json = serde_json::to_vec_pretty(&report)?;
    for fixture in &corpus.fixtures {
        anyhow::ensure!(
            !json
                .windows(fixture.secret_probe.len())
                .any(|window| window == fixture.secret_probe.as_bytes()),
            "fixture secret appeared in report"
        );
    }
    write_parent(&args.json_report, &json).await?;
    let markdown = markdown_report(&report);
    write_parent(&args.markdown_report, markdown.as_bytes()).await?;
    anyhow::ensure!(failure_count == 0, "corpus report contains failures");
    Ok(())
}

async fn build_request_bytes(fixture: &CorpusFixture) -> anyhow::Result<Vec<u8>> {
    let turn = RawTraceCaptureTurn {
        user_input: fixture.input.clone(),
        response: Some("Completed the local fixture safely.".to_string()),
        tool_calls: Vec::new(),
        started_at: fixture.created_at,
        completed_at: Some(fixture.created_at + chrono::Duration::seconds(1)),
        state: Some("completed".to_string()),
    };
    let mut raw = RawTraceContribution::from_capture_turns(
        &[turn],
        RecordedTraceContributionOptions {
            include_message_text: true,
            ..RecordedTraceContributionOptions::default()
        },
    );
    raw.trace_id = fixture.trace_id;
    raw.submission_id = fixture.submission_id;
    raw.created_at = fixture.created_at;
    raw.contributor.revocation_handle = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("tracecommons:corpus-revocation:{}", fixture.label).as_bytes(),
    );
    for (index, event) in raw.events.iter_mut().enumerate() {
        event.event_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("tracecommons:corpus-event:{}:{index}", fixture.label).as_bytes(),
        );
    }
    let redactor = DeterministicTraceRedactor::try_default()?;
    let mut envelope = redactor.redact_trace(raw).await?;
    envelope.privacy.residual_pii_risk = match fixture.privacy_risk.as_str() {
        "low" => ResidualPiiRisk::Low,
        "medium" => ResidualPiiRisk::Medium,
        "high" => ResidualPiiRisk::High,
        _ => anyhow::bail!("fixture privacy risk is invalid"),
    };
    Ok(serde_json::to_vec(&envelope)?)
}

async fn submit_http(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    key: &str,
    body: &[u8],
) -> anyhow::Result<PipelineSubmitReceipt> {
    let response = client
        .post(format!("{base_url}/v1/pipeline/submissions"))
        .bearer_auth(token)
        .header("idempotency-key", key)
        .header("content-type", "application/json")
        .body(body.to_vec())
        .send()
        .await?;
    anyhow::ensure!(
        response.status().is_success(),
        "submission returned {}",
        response.status()
    );
    Ok(response.json().await?)
}

async fn run_worker_http(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
) -> anyhow::Result<()> {
    let response = client
        .post(format!("{base_url}/v1/pipeline/worker"))
        .bearer_auth(token)
        .send()
        .await?;
    anyhow::ensure!(
        response.status().is_success(),
        "worker returned {}",
        response.status()
    );
    Ok(())
}

async fn claim_review_http(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    run_id: Uuid,
) -> anyhow::Result<PipelineReviewClaim> {
    let response = client
        .post(format!("{base_url}/v1/pipeline/runs/{run_id}/review-claim"))
        .bearer_auth(token)
        .send()
        .await?;
    anyhow::ensure!(
        response.status().is_success(),
        "review claim returned {}",
        response.status()
    );
    Ok(response.json().await?)
}

async fn submit_review_assessment_http(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    run_id: Uuid,
    claim: &PipelineReviewClaim,
    recommendation: ReviewRecommendation,
) -> anyhow::Result<()> {
    let approve = recommendation == ReviewRecommendation::Approve;
    let response = client
        .post(format!(
            "{base_url}/v1/pipeline/runs/{run_id}/review-assessment"
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "lease_token": claim.lease_token,
            "recommendation": recommendation,
            "reason_code": if approve {
                "privacy_resolved"
            } else {
                "review_privacy_rejected"
            },
            "resolved_quarantine_reasons": if approve {
                vec!["privacy_review_required"]
            } else {
                Vec::<&str>::new()
            },
        }))
        .send()
        .await?;
    anyhow::ensure!(
        response.status().is_success(),
        "review assessment returned {}",
        response.status()
    );
    Ok(())
}

async fn submit_conflict_http(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    key: &str,
    body: &[u8],
) -> anyhow::Result<bool> {
    let response = client
        .post(format!("{base_url}/v1/pipeline/submissions"))
        .bearer_auth(token)
        .header("idempotency-key", key)
        .header("content-type", "application/json")
        .body(body.to_vec())
        .send()
        .await?;
    Ok(response.status() == StatusCode::CONFLICT)
}

async fn inspect_http(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    run_id: Uuid,
) -> anyhow::Result<PipelineInspection> {
    let response = client
        .get(format!("{base_url}/v1/pipeline/runs/{run_id}"))
        .bearer_auth(token)
        .send()
        .await?;
    anyhow::ensure!(
        response.status().is_success(),
        "inspection returned {}",
        response.status()
    );
    Ok(response.json().await?)
}

async fn submission_status_http(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    submission_id: Uuid,
) -> anyhow::Result<PipelineContributorStatus> {
    let response = client
        .post(format!("{base_url}/v1/contributors/me/submission-status"))
        .bearer_auth(token)
        .json(&serde_json::json!({"submission_ids": [submission_id]}))
        .send()
        .await?;
    anyhow::ensure!(
        response.status().is_success(),
        "public submission status returned {}",
        response.status()
    );
    let mut statuses = response.json::<Vec<PipelineContributorStatus>>().await?;
    anyhow::ensure!(
        statuses.len() == 1,
        "public submission status omitted an owned run"
    );
    Ok(statuses.remove(0))
}

fn fixture_report(
    fixture: &CorpusFixture,
    inspection: PipelineInspection,
    public_status: PipelineContributorStatus,
    replay_same_run: bool,
    changed_content_refused: bool,
) -> anyhow::Result<FixtureReport> {
    let admission_decision = inspection
        .outcomes
        .iter()
        .find(|outcome| outcome.phase == Phase::Admission)
        .and_then(|outcome| {
            if outcome.decision.as_str() == Some("Admit") {
                Some("admit")
            } else if outcome.decision.get("Quarantine").is_some() {
                Some("quarantine")
            } else if outcome.decision.get("Reject").is_some() {
                Some("reject")
            } else {
                None
            }
        })
        .unwrap_or("unknown")
        .to_string();
    let score_microcredits =
        outcome_microcredits(&inspection.outcomes, Phase::Score, "credit_microcredits");
    let finalized_microcredits = outcome_microcredits(
        &inspection.outcomes,
        Phase::Settle,
        "credit_microcredits_finalized",
    );
    let batch_hash = inspection
        .outcomes
        .iter()
        .find(|outcome| outcome.phase == Phase::Settle)
        .and_then(|outcome| outcome.decision.get("settlement_batch_ref_hash"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let phases: Vec<PhaseReport> = inspection.outcomes.into_iter().map(phase_report).collect();
    Ok(FixtureReport {
        label: fixture.label.clone(),
        request_content_hash: inspection.run.request_content_hash.clone(),
        run_id: inspection.run.run_id,
        submission_id: inspection.run.submission_id,
        state: inspection.run.state,
        phase_count: phases.len(),
        admission_decision,
        expected_admission_decision: fixture.expected_admission_decision.clone(),
        expected_outcome_count: fixture.expected_outcome_count,
        phases,
        approved_revision_id: inspection.run.approved_revision_id,
        index_membership: inspection.run.index_membership.clone(),
        score_microcredits,
        finalized_microcredits,
        batch_hash,
        index_command_hash: inspection.run.index_command_hash.clone(),
        index_write_state: inspection.run.index_write_state.clone(),
        credit_write_state: inspection.run.credit_write_state.clone(),
        payout_state: inspection.run.payout_state.clone(),
        public_processing_state: public_status.processing,
        public_credit_state: public_status.credit,
        public_payout_state: public_status.payout,
        replay_same_run,
        changed_content_refused,
        failure_label: inspection.run.last_error_label,
        attempt_count: inspection.run.attempt_count,
        max_attempts: inspection.run.max_attempts,
        next_attempt_at: inspection.run.next_attempt_at,
        time_in_phase_ms: (Utc::now() - inspection.run.phase_started_at)
            .num_milliseconds()
            .max(0),
    })
}

fn phase_report(outcome: PhaseOutcomeRecord) -> PhaseReport {
    PhaseReport {
        outcome_id: outcome.outcome_id,
        phase: outcome.phase,
        outcome_schema_id: outcome.outcome_schema.id,
        outcome_schema_version: outcome.outcome_schema.version,
        decision: outcome.decision,
        evidence: outcome.evidence,
        evaluation: outcome.evaluation,
    }
}

fn outcome_microcredits(outcomes: &[PhaseOutcomeRecord], phase: Phase, field: &str) -> Option<u64> {
    outcomes
        .iter()
        .find(|outcome| outcome.phase == phase)
        .and_then(|outcome| outcome.decision.get(field))
        .and_then(|value| serde_json::from_value::<Microcredits>(value.clone()).ok())
        .map(Microcredits::get)
}

fn markdown_report(report: &CorpusReport) -> String {
    let mut output = format!(
        "# Minimal pipeline corpus\n\nBundle: `{}`\n\nCompleted: {}/{}. Failures: {}.\n\n",
        report.bundle_id,
        report.completed_fixture_count,
        report.expected_fixture_count,
        report.failure_count
    );
    for fixture in &report.fixtures {
        output.push_str(&format!(
            "- `{}`: Admission `{}`, state `{:?}`, {} outcomes, {} attempts, {} ms in phase, score {} microcredits, index `{}` (`{}`), credit `{}`, payout `{}`, public processing `{:?}`, public credit `{:?}`, public payout `{}`\n",
            fixture.label,
            fixture.admission_decision,
            fixture.state,
            fixture.phase_count,
            fixture.attempt_count,
            fixture.time_in_phase_ms,
            fixture.score_microcredits.unwrap_or(0),
            fixture.index_membership,
            fixture.index_write_state,
            fixture.credit_write_state,
            fixture.payout_state,
            fixture.public_processing_state,
            fixture.public_credit_state,
            fixture.public_payout_state.as_deref().unwrap_or("none"),
        ));
    }
    output
}

async fn write_parent(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(path, bytes).await?;
    Ok(())
}

fn sha256_prefixed(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_configuration_rejects_raw_principal_identity() {
        assert!(parse_tokens("token,tenant,user@example.com,contributor").is_err());
        assert!(
            parse_tokens(&format!(
                "token,tenant,principal_sha256:{},contributor",
                "a".repeat(64)
            ))
            .is_ok()
        );
    }

    #[test]
    fn worker_reviewer_export_and_lifecycle_roles_are_distinct() {
        let tokens = parse_tokens(&format!(
            "review,tenant,reviewer_sha256:{},reviewer;\
             worker,tenant,worker_sha256:{},worker;\
             export,tenant,exporter_sha256:{},exporter;\
             lifecycle,tenant,lifecycle_worker_sha256:{},lifecycle_worker",
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64),
            "d".repeat(64),
        ))
        .unwrap();
        assert_eq!(tokens["review"].role, LocalRole::Reviewer);
        assert_eq!(tokens["worker"].role, LocalRole::Worker);
        assert_eq!(tokens["export"].role, LocalRole::Exporter);
        assert_eq!(tokens["lifecycle"].role, LocalRole::LifecycleWorker);
        assert_ne!(tokens["review"].role, tokens["worker"].role);
        assert_ne!(tokens["export"].role, tokens["lifecycle"].role);
    }
}
