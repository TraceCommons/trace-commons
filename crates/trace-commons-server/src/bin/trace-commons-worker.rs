//! `trace-commons-worker` — worker-audience operator CLI.
//!
//! Eight subcommands matching the worker-route endpoints on
//! trace-commons-server. Each subcommand carries its own `--bearer-token-env`
//! flag with a per-route default, because every worker route has its own
//! bearer-token gate on the server. There is intentionally no top-level
//! `--bearer-token-env`: an operator running e.g. `worker-retention-maintenance`
//! must not be able to silently reuse the utility-credit token. All requests
//! flow through the foundation `trace_commons_operator_client::Client`, which
//! enforces the host allowlist and never logs bearer tokens.

#[path = "operator_common/mod.rs"]
mod operator_common;

use std::io::{Write, stdout};
use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use reqwest::Method;
use serde_json::Value;
use trace_commons_operator_client::{Client, format as oc_format, host_allowlist::HostAllowlist};
use uuid::Uuid;

use operator_common::{render_items, render_kv_fields, sanitized_url};

// --- per-route bearer env defaults (one per worker route gate) ---
const UTILITY_CREDIT_BEARER_ENV: &str = "TRACE_COMMONS_UTILITY_CREDIT_WORKER_BEARER";
const RETENTION_WORKER_BEARER_ENV: &str = "TRACE_COMMONS_RETENTION_WORKER_BEARER";
const VECTOR_WORKER_BEARER_ENV: &str = "TRACE_COMMONS_VECTOR_WORKER_BEARER";
const BENCHMARK_WORKER_BEARER_ENV: &str = "TRACE_COMMONS_BENCHMARK_WORKER_BEARER";
const EXPORT_WORKER_BEARER_ENV: &str = "TRACE_COMMONS_EXPORT_WORKER_BEARER";
const RANKER_WORKER_BEARER_ENV: &str = "TRACE_COMMONS_RANKER_WORKER_BEARER";
const PROCESS_EVALUATION_WORKER_BEARER_ENV: &str = "TRACE_COMMONS_PROCESS_EVALUATION_WORKER_BEARER";

#[derive(Debug, Parser)]
#[command(
    name = "trace-commons-worker",
    about = "Worker-audience CLI for trace-commons-server.",
    // The semver does not move when a deploy does, so --version carries the
    // commit the binary was built from as well.
    version = trace_commons_build_info::version_line(env!("CARGO_PKG_VERSION"))
)]
pub(crate) struct Cli {
    /// Hosted endpoint. Reads `TRACE_COMMONS_ENDPOINT` when unset on the CLI.
    #[arg(long, env = "TRACE_COMMONS_ENDPOINT")]
    pub endpoint: String,

    /// Defense-in-depth host allowlist (CSV). Empty means permissive.
    #[arg(long, env = "TRACE_COMMONS_ALLOWED_HOSTS")]
    pub allowed_hosts: Option<String>,

    /// Emit the server response as JSON (with a sanitized request envelope).
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: WorkerSubcommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum WorkerSubcommand {
    /// Append delayed utility-credit events through the worker route.
    WorkerUtilityCredit(WorkerUtilityCreditArgs),
    /// Run scoped central retention maintenance through the worker route.
    WorkerRetentionMaintenance(WorkerRetentionMaintenanceArgs),
    /// Index scoped vector metadata through the worker route.
    WorkerVectorIndex(WorkerVectorIndexArgs),
    /// Start benchmark conversion through the worker route.
    WorkerBenchmarkConvert(WorkerBenchmarkConvertArgs),
    /// Export an approved replay dataset slice through the worker route.
    WorkerReplayDatasetExport(WorkerReplayDatasetExportArgs),
    /// Export approved ranker training candidates through the worker route.
    WorkerRankerTrainingCandidates(WorkerRankerTrainingArgs),
    /// Export approved ranker training pairs through the worker route.
    WorkerRankerTrainingPairs(WorkerRankerTrainingArgs),
    /// Submit process-evaluation metadata through the worker route.
    ProcessEvaluationSubmit(ProcessEvaluationSubmitArgs),
}

#[derive(Debug, clap::Args)]
pub(crate) struct WorkerUtilityCreditArgs {
    #[arg(long, value_enum)]
    pub event_type: UtilityCreditEventType,
    #[arg(long)]
    pub credit_points_delta: f32,
    #[arg(long)]
    pub reason: String,
    #[arg(long)]
    pub external_ref: String,
    #[arg(value_name = "SUBMISSION_ID", num_args = 1..)]
    pub submission_ids: Vec<Uuid>,
    #[arg(long, default_value = UTILITY_CREDIT_BEARER_ENV)]
    pub bearer_token_env: String,
}

#[derive(Debug, clap::Args)]
pub(crate) struct WorkerRetentionMaintenanceArgs {
    #[arg(long)]
    pub purpose: Option<String>,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub purge_expired_before: Option<String>,
    #[arg(long)]
    pub max_export_age_hours: Option<i64>,
    /// Leave invalid export-cache files in place (default: prune).
    #[arg(long)]
    pub skip_export_cache_prune: bool,
    #[arg(long, default_value = RETENTION_WORKER_BEARER_ENV)]
    pub bearer_token_env: String,
}

#[derive(Debug, clap::Args)]
pub(crate) struct WorkerVectorIndexArgs {
    #[arg(long)]
    pub purpose: Option<String>,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long, default_value = VECTOR_WORKER_BEARER_ENV)]
    pub bearer_token_env: String,
}

#[derive(Debug, clap::Args)]
pub(crate) struct WorkerBenchmarkConvertArgs {
    #[arg(long)]
    pub purpose: String,
    #[arg(long)]
    pub limit: Option<usize>,
    #[arg(long, value_enum)]
    pub consent_scope: Option<ConsentScope>,
    #[arg(long, value_enum)]
    pub status: Option<CorpusStatus>,
    #[arg(long, value_enum)]
    pub privacy_risk: Option<PrivacyRisk>,
    #[arg(long)]
    pub external_ref: Option<String>,
    #[arg(long, default_value = BENCHMARK_WORKER_BEARER_ENV)]
    pub bearer_token_env: String,
}

#[derive(Debug, clap::Args)]
pub(crate) struct WorkerReplayDatasetExportArgs {
    #[arg(long)]
    pub purpose: Option<String>,
    #[arg(long, value_enum)]
    pub consent_scope: Option<ConsentScope>,
    #[arg(long, value_enum)]
    pub status: Option<CorpusStatus>,
    #[arg(long, value_enum)]
    pub privacy_risk: Option<PrivacyRisk>,
    #[arg(long)]
    pub limit: Option<usize>,
    /// If set, write the raw JSON response to this path.
    #[arg(long)]
    pub output: Option<PathBuf>,
    #[arg(long, default_value = EXPORT_WORKER_BEARER_ENV)]
    pub bearer_token_env: String,
}

#[derive(Debug, clap::Args)]
pub(crate) struct WorkerRankerTrainingArgs {
    #[arg(long)]
    pub purpose: Option<String>,
    #[arg(long, value_enum)]
    pub consent_scope: Option<ConsentScope>,
    #[arg(long, value_enum)]
    pub status: Option<CorpusStatus>,
    #[arg(long, value_enum)]
    pub privacy_risk: Option<PrivacyRisk>,
    #[arg(long)]
    pub limit: Option<usize>,
    /// If set, write the raw JSON response to this path.
    #[arg(long)]
    pub output: Option<PathBuf>,
    #[arg(long, default_value = RANKER_WORKER_BEARER_ENV)]
    pub bearer_token_env: String,
}

#[derive(Debug, clap::Args)]
pub(crate) struct ProcessEvaluationSubmitArgs {
    #[arg(long)]
    pub submission_id: Uuid,
    #[arg(long)]
    pub reason: String,
    #[arg(long)]
    pub evaluator_name: Option<String>,
    #[arg(long)]
    pub evaluator_version: String,
    #[arg(long = "label")]
    pub labels: Vec<String>,
    #[arg(long, value_enum)]
    pub tool_selection: Option<ProcessEvaluationRating>,
    #[arg(long, value_enum)]
    pub tool_argument_quality: Option<ProcessEvaluationRating>,
    #[arg(long, value_enum)]
    pub tool_ordering: Option<ProcessEvaluationRating>,
    #[arg(long, value_enum)]
    pub verification: Option<ProcessEvaluationRating>,
    #[arg(long, value_enum)]
    pub side_effect_safety: Option<ProcessEvaluationRating>,
    #[arg(long)]
    pub overall_score: Option<f32>,
    #[arg(long)]
    pub utility_credit_points_delta: Option<f32>,
    #[arg(long)]
    pub utility_external_ref: Option<String>,
    #[arg(long, default_value = PROCESS_EVALUATION_WORKER_BEARER_ENV)]
    pub bearer_token_env: String,
}

// --- value enums ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum UtilityCreditEventType {
    BenchmarkConversion,
    RegressionCatch,
    TrainingUtility,
    RankingUtility,
    ReviewerBonus,
    AbusePenalty,
}

impl UtilityCreditEventType {
    fn as_str(self) -> &'static str {
        match self {
            Self::BenchmarkConversion => "benchmark_conversion",
            Self::RegressionCatch => "regression_catch",
            Self::TrainingUtility => "training_utility",
            Self::RankingUtility => "ranking_utility",
            Self::ReviewerBonus => "reviewer_bonus",
            Self::AbusePenalty => "abuse_penalty",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum PrivacyRisk {
    Low,
    Medium,
    High,
}

impl PrivacyRisk {
    fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ConsentScope {
    DebuggingEvaluation,
    BenchmarkOnly,
    RankingTraining,
    ModelTraining,
}

impl ConsentScope {
    fn as_str(self) -> &'static str {
        match self {
            Self::DebuggingEvaluation => "debugging-evaluation",
            Self::BenchmarkOnly => "benchmark-only",
            Self::RankingTraining => "ranking-training",
            Self::ModelTraining => "model-training",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum CorpusStatus {
    Accepted,
    Quarantined,
    Rejected,
    Revoked,
    Expired,
    Purged,
}

impl CorpusStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Quarantined => "quarantined",
            Self::Rejected => "rejected",
            Self::Revoked => "revoked",
            Self::Expired => "expired",
            Self::Purged => "purged",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ProcessEvaluationRating {
    Good,
    Acceptable,
    Poor,
    Failing,
}

impl ProcessEvaluationRating {
    fn as_str(self) -> &'static str {
        match self {
            Self::Good => "good",
            Self::Acceptable => "acceptable",
            Self::Poor => "poor",
            Self::Failing => "failing",
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    dispatch(cli).await
}

/// Return the per-subcommand bearer-env name so the dispatcher can build the
/// right `Client`. Each worker route has its own gate, so we plumb the env
/// name from the matched subcommand variant rather than from a shared
/// top-level flag.
fn bearer_env(cmd: &WorkerSubcommand) -> &str {
    match cmd {
        WorkerSubcommand::WorkerUtilityCredit(a) => &a.bearer_token_env,
        WorkerSubcommand::WorkerRetentionMaintenance(a) => &a.bearer_token_env,
        WorkerSubcommand::WorkerVectorIndex(a) => &a.bearer_token_env,
        WorkerSubcommand::WorkerBenchmarkConvert(a) => &a.bearer_token_env,
        WorkerSubcommand::WorkerReplayDatasetExport(a) => &a.bearer_token_env,
        WorkerSubcommand::WorkerRankerTrainingCandidates(a) => &a.bearer_token_env,
        WorkerSubcommand::WorkerRankerTrainingPairs(a) => &a.bearer_token_env,
        WorkerSubcommand::ProcessEvaluationSubmit(a) => &a.bearer_token_env,
    }
}

fn build_client(
    endpoint: &str,
    bearer_token_env: &str,
    allowed_hosts: Option<&str>,
) -> Result<Client> {
    let mut builder = Client::builder(endpoint, bearer_token_env);
    if let Some(csv) = allowed_hosts {
        builder = builder.host_allowlist(HostAllowlist::from_csv(csv));
    }
    Ok(builder.build()?)
}

async fn dispatch(cli: Cli) -> Result<()> {
    let bearer = bearer_env(&cli.command).to_string();
    let client = build_client(&cli.endpoint, &bearer, cli.allowed_hosts.as_deref())?;
    let endpoint = cli.endpoint;
    let json = cli.json;
    match cli.command {
        WorkerSubcommand::WorkerUtilityCredit(args) => {
            worker_utility_credit(&client, &endpoint, args, json).await
        }
        WorkerSubcommand::WorkerRetentionMaintenance(args) => {
            worker_retention_maintenance(&client, &endpoint, args, json).await
        }
        WorkerSubcommand::WorkerVectorIndex(args) => {
            worker_vector_index(&client, &endpoint, args, json).await
        }
        WorkerSubcommand::WorkerBenchmarkConvert(args) => {
            worker_benchmark_convert(&client, &endpoint, args, json).await
        }
        WorkerSubcommand::WorkerReplayDatasetExport(args) => {
            worker_replay_dataset_export(&client, &endpoint, args, json).await
        }
        WorkerSubcommand::WorkerRankerTrainingCandidates(args) => {
            worker_ranker_training_export(
                &client,
                &endpoint,
                args,
                "/v1/workers/ranker/training-candidates",
                "ranker training candidates",
                "candidates",
                json,
            )
            .await
        }
        WorkerSubcommand::WorkerRankerTrainingPairs(args) => {
            worker_ranker_training_export(
                &client,
                &endpoint,
                args,
                "/v1/workers/ranker/training-pairs",
                "ranker training pairs",
                "pairs",
                json,
            )
            .await
        }
        WorkerSubcommand::ProcessEvaluationSubmit(args) => {
            process_evaluation_submit(&client, &endpoint, args, json).await
        }
    }
}

// --- handlers ---

async fn worker_utility_credit(
    client: &Client,
    endpoint: &str,
    args: WorkerUtilityCreditArgs,
    json: bool,
) -> Result<()> {
    if args.submission_ids.is_empty() {
        anyhow::bail!("at least one SUBMISSION_ID positional is required");
    }
    let reason = args.reason.trim();
    if reason.is_empty() {
        anyhow::bail!("--reason must not be empty");
    }
    let external_ref = args.external_ref.trim();
    if external_ref.is_empty() {
        anyhow::bail!("--external-ref must not be empty");
    }
    let path = "/v1/workers/utility-credit";
    let body = serde_json::json!({
        "event_type": args.event_type.as_str(),
        "credit_points_delta": args.credit_points_delta,
        "reason": reason,
        "external_ref": external_ref,
        "submission_ids": args.submission_ids,
    });
    let value: Value = client
        .call_json::<Value, Value>(Method::POST, path, &[], Some(&body))
        .await?;
    if json {
        emit_json(endpoint, "POST", path, &value)?;
    } else {
        let mut out = stdout();
        writeln!(out, "Trace Commons utility credit worker complete.")?;
        render_kv_fields(
            &mut out,
            &value,
            &[
                ("  appended", "appended_count"),
                ("  skipped existing", "skipped_existing_count"),
            ],
        )?;
    }
    Ok(())
}

async fn worker_retention_maintenance(
    client: &Client,
    endpoint: &str,
    args: WorkerRetentionMaintenanceArgs,
    json: bool,
) -> Result<()> {
    let path = "/v1/workers/retention-maintenance";
    let mut body = serde_json::json!({ "dry_run": args.dry_run });
    if let Some(purpose) = args.purpose {
        let trimmed = purpose.trim();
        if trimmed.is_empty() {
            anyhow::bail!("--purpose must not be empty");
        }
        body["purpose"] = Value::String(trimmed.to_string());
    }
    if args.skip_export_cache_prune {
        body["prune_export_cache"] = Value::Bool(false);
    }
    if let Some(hours) = args.max_export_age_hours {
        body["max_export_age_hours"] = Value::Number(hours.into());
    }
    if let Some(before) = args.purge_expired_before {
        body["purge_expired_before"] = Value::String(before);
    }
    let value: Value = client
        .call_json::<Value, Value>(Method::POST, path, &[], Some(&body))
        .await?;
    if json {
        emit_json(endpoint, "POST", path, &value)?;
    } else {
        let mut out = stdout();
        writeln!(out, "Trace Commons retention maintenance requested.")?;
        render_kv_fields(
            &mut out,
            &value,
            &[
                ("  audit event id", "audit_event_id"),
                ("  purpose", "purpose"),
                ("  dry run", "dry_run"),
                ("  records marked revoked", "records_marked_revoked"),
                ("  records marked expired", "records_marked_expired"),
                ("  records marked purged", "records_marked_purged"),
                ("  export cache files pruned", "export_cache_files_pruned"),
                (
                    "  export provenance invalidated",
                    "export_provenance_invalidated",
                ),
                (
                    "  benchmark artifacts invalidated",
                    "benchmark_artifacts_invalidated",
                ),
            ],
        )?;
    }
    Ok(())
}

async fn worker_vector_index(
    client: &Client,
    endpoint: &str,
    args: WorkerVectorIndexArgs,
    json: bool,
) -> Result<()> {
    let path = "/v1/workers/vector-index";
    let mut body = serde_json::json!({ "dry_run": args.dry_run });
    if let Some(purpose) = args.purpose {
        let trimmed = purpose.trim();
        if trimmed.is_empty() {
            anyhow::bail!("--purpose must not be empty");
        }
        body["purpose"] = Value::String(trimmed.to_string());
    }
    let value: Value = client
        .call_json::<Value, Value>(Method::POST, path, &[], Some(&body))
        .await?;
    if json {
        emit_json(endpoint, "POST", path, &value)?;
    } else {
        let mut out = stdout();
        writeln!(out, "Trace Commons vector index requested.")?;
        render_kv_fields(
            &mut out,
            &value,
            &[
                ("  audit event id", "audit_event_id"),
                ("  purpose", "purpose"),
                ("  dry run", "dry_run"),
                ("  vectors indexed", "vector_entries_indexed"),
            ],
        )?;
    }
    Ok(())
}

async fn worker_benchmark_convert(
    client: &Client,
    endpoint: &str,
    args: WorkerBenchmarkConvertArgs,
    json: bool,
) -> Result<()> {
    let trimmed = args.purpose.trim();
    if trimmed.is_empty() {
        anyhow::bail!("--purpose must be non-empty");
    }
    let path = "/v1/workers/benchmark-convert";
    let mut body = serde_json::json!({ "purpose": trimmed });
    if let Some(limit) = args.limit {
        body["limit"] = serde_json::json!(limit);
    }
    if let Some(scope) = args.consent_scope {
        body["consent_scope"] = Value::String(scope.as_str().to_string());
    }
    if let Some(status) = args.status {
        body["status"] = Value::String(status.as_str().to_string());
    }
    if let Some(risk) = args.privacy_risk {
        body["privacy_risk"] = Value::String(risk.as_str().to_string());
    }
    if let Some(external_ref) = args.external_ref {
        body["external_ref"] = Value::String(external_ref);
    }
    let value: Value = client
        .call_json::<Value, Value>(Method::POST, path, &[], Some(&body))
        .await?;
    if json {
        emit_json(endpoint, "POST", path, &value)?;
    } else {
        let mut out = stdout();
        writeln!(
            out,
            "Trace Commons benchmark conversion (worker) requested."
        )?;
        render_kv_fields(
            &mut out,
            &value,
            &[
                ("  conversion id", "conversion_id"),
                ("  audit event id", "audit_event_id"),
                ("  item count", "item_count"),
                ("  purpose", "purpose"),
            ],
        )?;
    }
    Ok(())
}

async fn worker_replay_dataset_export(
    client: &Client,
    endpoint: &str,
    args: WorkerReplayDatasetExportArgs,
    json: bool,
) -> Result<()> {
    let path = "/v1/workers/replay-export";
    let mut owned: Vec<(&str, String)> = Vec::new();
    if let Some(limit) = args.limit {
        owned.push(("limit", limit.to_string()));
    }
    if let Some(purpose) = args.purpose {
        let trimmed = purpose.trim();
        if trimmed.is_empty() {
            anyhow::bail!("--purpose must be non-empty");
        }
        owned.push(("purpose", trimmed.to_string()));
    }
    if let Some(scope) = args.consent_scope {
        owned.push(("consent_scope", scope.as_str().to_string()));
    }
    if let Some(status) = args.status {
        owned.push(("status", status.as_str().to_string()));
    }
    if let Some(risk) = args.privacy_risk {
        owned.push(("privacy_risk", risk.as_str().to_string()));
    }
    let query = borrow_query(&owned);
    let raw = client
        .call_raw::<()>(Method::GET, path, &query, None)
        .await?;
    let value: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);

    if let Some(output) = args.output.as_ref() {
        let pretty = if value.is_null() {
            raw.clone()
        } else {
            serde_json::to_string_pretty(&value)?
        };
        std::fs::write(output, pretty).map_err(|e| {
            anyhow::anyhow!(
                "failed to write replay dataset export {}: {}",
                output.display(),
                e
            )
        })?;
        if json {
            emit_json(endpoint, "GET", path, &value)?;
        } else {
            let mut out = stdout();
            writeln!(
                out,
                "Wrote central replay dataset export to {}",
                output.display()
            )?;
            render_kv_fields(
                &mut out,
                &value,
                &[
                    ("  export id", "export_id"),
                    ("  manifest id", "manifest_id"),
                    ("  audit event id", "audit_event_id"),
                    ("  item count", "item_count"),
                ],
            )?;
        }
        return Ok(());
    }
    emit_json(endpoint, "GET", path, &value)
}

#[allow(clippy::too_many_arguments)]
async fn worker_ranker_training_export(
    client: &Client,
    endpoint: &str,
    args: WorkerRankerTrainingArgs,
    path: &str,
    output_label: &str,
    item_field: &str,
    json: bool,
) -> Result<()> {
    let mut owned: Vec<(&str, String)> = Vec::new();
    if let Some(limit) = args.limit {
        owned.push(("limit", limit.to_string()));
    }
    if let Some(purpose) = args.purpose {
        let trimmed = purpose.trim();
        if trimmed.is_empty() {
            anyhow::bail!("--purpose must be non-empty");
        }
        owned.push(("purpose", trimmed.to_string()));
    }
    if let Some(scope) = args.consent_scope {
        owned.push(("consent_scope", scope.as_str().to_string()));
    }
    if let Some(status) = args.status {
        owned.push(("status", status.as_str().to_string()));
    }
    if let Some(risk) = args.privacy_risk {
        owned.push(("privacy_risk", risk.as_str().to_string()));
    }
    let query = borrow_query(&owned);
    let raw = client
        .call_raw::<()>(Method::GET, path, &query, None)
        .await?;
    let value: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);

    if let Some(output) = args.output.as_ref() {
        let pretty = if value.is_null() {
            raw.clone()
        } else {
            serde_json::to_string_pretty(&value)?
        };
        std::fs::write(output, pretty).map_err(|e| {
            anyhow::anyhow!(
                "failed to write ranker training export {}: {}",
                output.display(),
                e
            )
        })?;
        if json {
            emit_json(endpoint, "GET", path, &value)?;
        } else {
            let mut out = stdout();
            writeln!(
                out,
                "Wrote central {} export to {}",
                output_label,
                output.display()
            )?;
            render_kv_fields(
                &mut out,
                &value,
                &[
                    ("  export id", "export_id"),
                    ("  audit event id", "audit_event_id"),
                    ("  purpose", "purpose"),
                    ("  item count", "item_count"),
                ],
            )?;
        }
        return Ok(());
    }

    if json {
        emit_json(endpoint, "GET", path, &value)
    } else {
        let items_value = value.get(item_field).cloned().unwrap_or(Value::Null);
        let envelope = serde_json::json!({ "items": items_value });
        render_items(
            &mut stdout(),
            &format!("Central {output_label}"),
            &envelope,
            &[
                "submission_id",
                "trace_id",
                "ranker_score",
                "preferred_submission_id",
                "rejected_submission_id",
                "reason",
            ],
        )?;
        Ok(())
    }
}

async fn process_evaluation_submit(
    client: &Client,
    endpoint: &str,
    args: ProcessEvaluationSubmitArgs,
    json: bool,
) -> Result<()> {
    let reason = args.reason.trim();
    if reason.is_empty() {
        anyhow::bail!("--reason must not be empty");
    }
    let evaluator_version = args.evaluator_version.trim();
    if evaluator_version.is_empty() {
        anyhow::bail!("--evaluator-version must not be empty");
    }
    if let Some(overall_score) = args.overall_score {
        if !(0.0..=1.0).contains(&overall_score) {
            anyhow::bail!("--overall-score must be between 0.0 and 1.0");
        }
    }
    let utility_external_ref = if let Some(delta) = args.utility_credit_points_delta {
        if !delta.is_finite() || delta.abs() > 100.0 {
            anyhow::bail!("--utility-credit-points-delta must be finite with abs <= 100.0");
        }
        let external_ref = args
            .utility_external_ref
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "--utility-external-ref must be non-empty when --utility-credit-points-delta is set"
                )
            })?;
        Some(external_ref.to_string())
    } else {
        if args
            .utility_external_ref
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
        {
            anyhow::bail!("--utility-external-ref requires --utility-credit-points-delta");
        }
        None
    };

    let labels: Vec<String> = args
        .labels
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let mut process_evaluation = serde_json::json!({
        "evaluator_version": evaluator_version,
        "labels": labels,
    });
    if let Some(name) = args.evaluator_name.as_ref() {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            process_evaluation["evaluator_name"] = Value::String(trimmed.to_string());
        }
    }
    if let Some(v) = args.tool_selection {
        process_evaluation["tool_selection"] = Value::String(v.as_str().to_string());
    }
    if let Some(v) = args.tool_argument_quality {
        process_evaluation["tool_argument_quality"] = Value::String(v.as_str().to_string());
    }
    if let Some(v) = args.tool_ordering {
        process_evaluation["tool_ordering"] = Value::String(v.as_str().to_string());
    }
    if let Some(v) = args.verification {
        process_evaluation["verification"] = Value::String(v.as_str().to_string());
    }
    if let Some(v) = args.side_effect_safety {
        process_evaluation["side_effect_safety"] = Value::String(v.as_str().to_string());
    }
    if let Some(score) = args.overall_score {
        process_evaluation["overall_score"] = serde_json::json!(score);
    }

    let mut body = serde_json::json!({
        "submission_id": args.submission_id,
        "process_evaluation": process_evaluation,
        "reason": reason,
    });
    if let (Some(delta), Some(external_ref)) =
        (args.utility_credit_points_delta, utility_external_ref)
    {
        body["utility_credit_points_delta"] = serde_json::json!(delta);
        body["utility_external_ref"] = Value::String(external_ref);
    }

    let path = "/v1/workers/process-evaluation";
    let value: Value = client
        .call_json::<Value, Value>(Method::POST, path, &[], Some(&body))
        .await?;
    if json {
        emit_json(endpoint, "POST", path, &value)?;
    } else {
        let mut out = stdout();
        writeln!(
            out,
            "Trace Commons process evaluation submitted for {}.",
            args.submission_id
        )?;
        render_kv_fields(
            &mut out,
            &value,
            &[
                ("  tenant", "tenant_id"),
                ("  tenant storage ref", "tenant_storage_ref"),
                ("  trace id", "trace_id"),
                ("  status", "status"),
                ("  process eval value", "process_eval_value"),
                ("  review scorecard", "review_scorecard"),
                ("  output object ref", "output_object_ref_id"),
                ("  utility credit appended", "utility_credit_appended_count"),
                (
                    "  utility credit skipped existing",
                    "utility_credit_skipped_existing_count",
                ),
            ],
        )?;
    }
    Ok(())
}

// --- helpers ---

fn borrow_query<'a>(owned: &'a [(&'a str, String)]) -> Vec<(&'a str, &'a str)> {
    owned.iter().map(|(k, v)| (*k, v.as_str())).collect()
}

fn emit_json(endpoint: &str, method: &str, path: &str, value: &Value) -> Result<()> {
    let url = sanitized_url(endpoint, path);
    oc_format::emit_json(&mut stdout(), method, &url, value)?;
    Ok(())
}

// --- tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use wiremock::matchers::{body_json, header, method, path as wm_path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn unique_bearer_env() -> String {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        format!("TC_WORKER_TEST_BEARER_{n}")
    }

    struct EnvGuard {
        name: String,
    }
    impl EnvGuard {
        fn set(name: String, value: &str) -> Self {
            unsafe { std::env::set_var(&name, value) };
            EnvGuard { name }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe { std::env::remove_var(&self.name) };
        }
    }

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(
            ["trace-commons-worker", "--endpoint", "https://api.example"]
                .iter()
                .chain(args.iter())
                .copied(),
        )
        .expect("parse cli")
    }

    #[test]
    fn cli_parses_worker_utility_credit_minimal() {
        let sub = Uuid::new_v4().to_string();
        let cli = parse(&[
            "worker-utility-credit",
            "--event-type",
            "training-utility",
            "--credit-points-delta",
            "1.5",
            "--reason",
            "delayed training utility",
            "--external-ref",
            "job-1",
            &sub,
        ]);
        match cli.command {
            WorkerSubcommand::WorkerUtilityCredit(args) => {
                assert_eq!(args.event_type, UtilityCreditEventType::TrainingUtility);
                assert_eq!(args.credit_points_delta, 1.5);
                assert_eq!(args.submission_ids.len(), 1);
                assert_eq!(args.bearer_token_env, UTILITY_CREDIT_BEARER_ENV);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_worker_utility_credit_multiple_submissions() {
        let s1 = Uuid::new_v4().to_string();
        let s2 = Uuid::new_v4().to_string();
        let cli = parse(&[
            "worker-utility-credit",
            "--event-type",
            "ranking-utility",
            "--credit-points-delta",
            "0.5",
            "--reason",
            "x",
            "--external-ref",
            "y",
            &s1,
            &s2,
        ]);
        match cli.command {
            WorkerSubcommand::WorkerUtilityCredit(args) => {
                assert_eq!(args.submission_ids.len(), 2);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn cli_rejects_worker_utility_credit_invalid_event() {
        let err = Cli::try_parse_from([
            "trace-commons-worker",
            "--endpoint",
            "https://api.example",
            "worker-utility-credit",
            "--event-type",
            "bogus",
            "--credit-points-delta",
            "0.0",
            "--reason",
            "x",
            "--external-ref",
            "y",
            &Uuid::new_v4().to_string(),
        ])
        .unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn cli_parses_worker_retention_maintenance_full() {
        let cli = parse(&[
            "worker-retention-maintenance",
            "--purpose",
            "weekly",
            "--dry-run",
            "--max-export-age-hours",
            "48",
            "--skip-export-cache-prune",
        ]);
        match cli.command {
            WorkerSubcommand::WorkerRetentionMaintenance(args) => {
                assert_eq!(args.purpose.as_deref(), Some("weekly"));
                assert!(args.dry_run);
                assert_eq!(args.max_export_age_hours, Some(48));
                assert!(args.skip_export_cache_prune);
                assert_eq!(args.bearer_token_env, RETENTION_WORKER_BEARER_ENV);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_worker_vector_index() {
        let cli = parse(&["worker-vector-index", "--purpose", "p", "--dry-run"]);
        match cli.command {
            WorkerSubcommand::WorkerVectorIndex(args) => {
                assert_eq!(args.bearer_token_env, VECTOR_WORKER_BEARER_ENV);
                assert!(args.dry_run);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_worker_benchmark_convert() {
        let cli = parse(&[
            "worker-benchmark-convert",
            "--purpose",
            "weekly",
            "--limit",
            "10",
            "--consent-scope",
            "benchmark-only",
            "--status",
            "accepted",
            "--privacy-risk",
            "low",
        ]);
        match cli.command {
            WorkerSubcommand::WorkerBenchmarkConvert(args) => {
                assert_eq!(args.bearer_token_env, BENCHMARK_WORKER_BEARER_ENV);
                assert_eq!(args.consent_scope, Some(ConsentScope::BenchmarkOnly));
                assert_eq!(args.status, Some(CorpusStatus::Accepted));
                assert_eq!(args.privacy_risk, Some(PrivacyRisk::Low));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn cli_rejects_worker_benchmark_convert_missing_purpose() {
        let err = Cli::try_parse_from([
            "trace-commons-worker",
            "--endpoint",
            "https://api.example",
            "worker-benchmark-convert",
        ])
        .unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn cli_parses_worker_replay_dataset_export() {
        let cli = parse(&[
            "worker-replay-dataset-export",
            "--limit",
            "50",
            "--consent-scope",
            "model-training",
            "--privacy-risk",
            "medium",
            "--output",
            "/tmp/replay.json",
        ]);
        match cli.command {
            WorkerSubcommand::WorkerReplayDatasetExport(args) => {
                assert_eq!(args.bearer_token_env, EXPORT_WORKER_BEARER_ENV);
                assert_eq!(args.limit, Some(50));
                assert!(args.output.is_some());
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_worker_ranker_training_candidates() {
        let cli = parse(&[
            "worker-ranker-training-candidates",
            "--limit",
            "10",
            "--status",
            "accepted",
        ]);
        match cli.command {
            WorkerSubcommand::WorkerRankerTrainingCandidates(args) => {
                assert_eq!(args.bearer_token_env, RANKER_WORKER_BEARER_ENV);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_worker_ranker_training_pairs() {
        let cli = parse(&["worker-ranker-training-pairs", "--limit", "10"]);
        match cli.command {
            WorkerSubcommand::WorkerRankerTrainingPairs(args) => {
                assert_eq!(args.bearer_token_env, RANKER_WORKER_BEARER_ENV);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_process_evaluation_submit_minimal() {
        let sub = Uuid::new_v4().to_string();
        let cli = parse(&[
            "process-evaluation-submit",
            "--submission-id",
            &sub,
            "--reason",
            "manual review",
            "--evaluator-version",
            "v1",
        ]);
        match cli.command {
            WorkerSubcommand::ProcessEvaluationSubmit(args) => {
                assert_eq!(args.evaluator_version, "v1");
                assert_eq!(args.bearer_token_env, PROCESS_EVALUATION_WORKER_BEARER_ENV);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_process_evaluation_submit_full() {
        let sub = Uuid::new_v4().to_string();
        let cli = parse(&[
            "process-evaluation-submit",
            "--submission-id",
            &sub,
            "--reason",
            "manual review",
            "--evaluator-version",
            "v1",
            "--evaluator-name",
            "frontier-lab",
            "--label",
            "good-trace",
            "--label",
            "tools-correct",
            "--tool-selection",
            "good",
            "--tool-argument-quality",
            "acceptable",
            "--tool-ordering",
            "poor",
            "--verification",
            "good",
            "--side-effect-safety",
            "good",
            "--overall-score",
            "0.85",
            "--utility-credit-points-delta",
            "0.3",
            "--utility-external-ref",
            "job-x",
        ]);
        match cli.command {
            WorkerSubcommand::ProcessEvaluationSubmit(args) => {
                assert_eq!(args.labels.len(), 2);
                assert_eq!(args.overall_score, Some(0.85));
                assert_eq!(args.utility_credit_points_delta, Some(0.3));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn cli_rejects_unknown_subcommand() {
        let err = Cli::try_parse_from([
            "trace-commons-worker",
            "--endpoint",
            "https://api.example",
            "bogus-subcommand",
        ])
        .unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn bearer_env_resolves_per_subcommand() {
        // Build one CLI per subcommand and confirm `bearer_env()` returns the
        // correct per-route default for each. This protects against future
        // edits that accidentally collapse them onto a shared top-level flag.
        let sub = Uuid::new_v4().to_string();
        let cases: Vec<(&str, Vec<&str>, &str)> = vec![
            (
                "worker-utility-credit",
                vec![
                    "worker-utility-credit",
                    "--event-type",
                    "training-utility",
                    "--credit-points-delta",
                    "1.0",
                    "--reason",
                    "r",
                    "--external-ref",
                    "x",
                    &sub,
                ],
                UTILITY_CREDIT_BEARER_ENV,
            ),
            (
                "worker-retention-maintenance",
                vec!["worker-retention-maintenance"],
                RETENTION_WORKER_BEARER_ENV,
            ),
            (
                "worker-vector-index",
                vec!["worker-vector-index"],
                VECTOR_WORKER_BEARER_ENV,
            ),
            (
                "worker-benchmark-convert",
                vec!["worker-benchmark-convert", "--purpose", "p"],
                BENCHMARK_WORKER_BEARER_ENV,
            ),
            (
                "worker-replay-dataset-export",
                vec!["worker-replay-dataset-export"],
                EXPORT_WORKER_BEARER_ENV,
            ),
            (
                "worker-ranker-training-candidates",
                vec!["worker-ranker-training-candidates"],
                RANKER_WORKER_BEARER_ENV,
            ),
            (
                "worker-ranker-training-pairs",
                vec!["worker-ranker-training-pairs"],
                RANKER_WORKER_BEARER_ENV,
            ),
            (
                "process-evaluation-submit",
                vec![
                    "process-evaluation-submit",
                    "--submission-id",
                    &sub,
                    "--reason",
                    "r",
                    "--evaluator-version",
                    "v1",
                ],
                PROCESS_EVALUATION_WORKER_BEARER_ENV,
            ),
        ];
        for (label, args, expected_env) in cases {
            let cli = parse(&args);
            assert_eq!(
                bearer_env(&cli.command),
                expected_env,
                "subcommand {label} resolved wrong bearer env"
            );
        }
    }

    #[tokio::test]
    async fn worker_per_route_bearer_resolution_uses_distinct_envs() {
        // Each worker subcommand pulls its bearer from its OWN env. We set 8
        // distinct env vars to distinct tokens and confirm each handler sends
        // the right one to the mock server.
        let server = MockServer::start().await;

        let envs = [
            unique_bearer_env(),
            unique_bearer_env(),
            unique_bearer_env(),
            unique_bearer_env(),
            unique_bearer_env(),
            unique_bearer_env(),
            unique_bearer_env(),
            unique_bearer_env(),
        ];
        let tokens = [
            "utility-token-1",
            "retention-token-2",
            "vector-token-3",
            "benchmark-token-4",
            "export-token-5",
            "ranker-candidates-token-6",
            "ranker-pairs-token-7",
            "process-eval-token-8",
        ];
        let _guards: Vec<EnvGuard> = envs
            .iter()
            .zip(tokens.iter())
            .map(|(e, t)| EnvGuard::set(e.clone(), t))
            .collect();

        // Each route accepts any POST/GET and checks the authorization header
        // matches the expected token.
        let routes: [(&str, &str, &str); 8] = [
            ("POST", "/v1/workers/utility-credit", tokens[0]),
            ("POST", "/v1/workers/retention-maintenance", tokens[1]),
            ("POST", "/v1/workers/vector-index", tokens[2]),
            ("POST", "/v1/workers/benchmark-convert", tokens[3]),
            ("GET", "/v1/workers/replay-export", tokens[4]),
            ("GET", "/v1/workers/ranker/training-candidates", tokens[5]),
            ("GET", "/v1/workers/ranker/training-pairs", tokens[6]),
            ("POST", "/v1/workers/process-evaluation", tokens[7]),
        ];
        for (m, p, tok) in routes {
            Mock::given(method(m))
                .and(wm_path(p))
                .and(header("authorization", format!("Bearer {tok}").as_str()))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
                .expect(1)
                .mount(&server)
                .await;
        }

        // Fire each subcommand handler with its own env name.
        let endpoint = server.uri();

        let c0 = build_client(&endpoint, &envs[0], None).unwrap();
        worker_utility_credit(
            &c0,
            &endpoint,
            WorkerUtilityCreditArgs {
                event_type: UtilityCreditEventType::TrainingUtility,
                credit_points_delta: 1.0,
                reason: "r".into(),
                external_ref: "x".into(),
                submission_ids: vec![Uuid::new_v4()],
                bearer_token_env: envs[0].clone(),
            },
            true,
        )
        .await
        .expect("utility credit");

        let c1 = build_client(&endpoint, &envs[1], None).unwrap();
        worker_retention_maintenance(
            &c1,
            &endpoint,
            WorkerRetentionMaintenanceArgs {
                purpose: None,
                dry_run: false,
                purge_expired_before: None,
                max_export_age_hours: None,
                skip_export_cache_prune: false,
                bearer_token_env: envs[1].clone(),
            },
            true,
        )
        .await
        .expect("retention");

        let c2 = build_client(&endpoint, &envs[2], None).unwrap();
        worker_vector_index(
            &c2,
            &endpoint,
            WorkerVectorIndexArgs {
                purpose: None,
                dry_run: true,
                bearer_token_env: envs[2].clone(),
            },
            true,
        )
        .await
        .expect("vector");

        let c3 = build_client(&endpoint, &envs[3], None).unwrap();
        worker_benchmark_convert(
            &c3,
            &endpoint,
            WorkerBenchmarkConvertArgs {
                purpose: "p".into(),
                limit: None,
                consent_scope: None,
                status: None,
                privacy_risk: None,
                external_ref: None,
                bearer_token_env: envs[3].clone(),
            },
            true,
        )
        .await
        .expect("benchmark");

        let c4 = build_client(&endpoint, &envs[4], None).unwrap();
        worker_replay_dataset_export(
            &c4,
            &endpoint,
            WorkerReplayDatasetExportArgs {
                purpose: None,
                consent_scope: None,
                status: None,
                privacy_risk: None,
                limit: None,
                output: None,
                bearer_token_env: envs[4].clone(),
            },
            true,
        )
        .await
        .expect("replay");

        let c5 = build_client(&endpoint, &envs[5], None).unwrap();
        worker_ranker_training_export(
            &c5,
            &endpoint,
            WorkerRankerTrainingArgs {
                purpose: None,
                consent_scope: None,
                status: None,
                privacy_risk: None,
                limit: None,
                output: None,
                bearer_token_env: envs[5].clone(),
            },
            "/v1/workers/ranker/training-candidates",
            "ranker training candidates",
            "candidates",
            true,
        )
        .await
        .expect("ranker candidates");

        let c6 = build_client(&endpoint, &envs[6], None).unwrap();
        worker_ranker_training_export(
            &c6,
            &endpoint,
            WorkerRankerTrainingArgs {
                purpose: None,
                consent_scope: None,
                status: None,
                privacy_risk: None,
                limit: None,
                output: None,
                bearer_token_env: envs[6].clone(),
            },
            "/v1/workers/ranker/training-pairs",
            "ranker training pairs",
            "pairs",
            true,
        )
        .await
        .expect("ranker pairs");

        let c7 = build_client(&endpoint, &envs[7], None).unwrap();
        process_evaluation_submit(
            &c7,
            &endpoint,
            ProcessEvaluationSubmitArgs {
                submission_id: Uuid::new_v4(),
                reason: "r".into(),
                evaluator_name: None,
                evaluator_version: "v1".into(),
                labels: vec![],
                tool_selection: None,
                tool_argument_quality: None,
                tool_ordering: None,
                verification: None,
                side_effect_safety: None,
                overall_score: None,
                utility_credit_points_delta: None,
                utility_external_ref: None,
                bearer_token_env: envs[7].clone(),
            },
            true,
        )
        .await
        .expect("process evaluation");
    }

    #[tokio::test]
    async fn worker_utility_credit_posts_expected_body() {
        let server = MockServer::start().await;
        let sub = Uuid::new_v4();
        Mock::given(method("POST"))
            .and(wm_path("/v1/workers/utility-credit"))
            .and(body_json(serde_json::json!({
                "event_type": "training_utility",
                "credit_points_delta": 1.5,
                "reason": "delayed training",
                "external_ref": "job-1",
                "submission_ids": [sub],
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "appended_count": 1,
                "skipped_existing_count": 0,
            })))
            .expect(1)
            .mount(&server)
            .await;
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "utility-secret");
        let client = Client::builder(server.uri(), env.clone()).build().unwrap();
        worker_utility_credit(
            &client,
            &server.uri(),
            WorkerUtilityCreditArgs {
                event_type: UtilityCreditEventType::TrainingUtility,
                credit_points_delta: 1.5,
                reason: "delayed training".into(),
                external_ref: "job-1".into(),
                submission_ids: vec![sub],
                bearer_token_env: env,
            },
            true,
        )
        .await
        .expect("happy path");
    }

    #[tokio::test]
    async fn worker_utility_credit_rejects_empty_reason() {
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder("https://api.example", env.clone())
            .build()
            .unwrap();
        let err = worker_utility_credit(
            &client,
            "https://api.example",
            WorkerUtilityCreditArgs {
                event_type: UtilityCreditEventType::TrainingUtility,
                credit_points_delta: 1.0,
                reason: "  ".into(),
                external_ref: "x".into(),
                submission_ids: vec![Uuid::new_v4()],
                bearer_token_env: env,
            },
            true,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("reason"));
    }

    #[tokio::test]
    async fn worker_utility_credit_rejects_no_submissions() {
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder("https://api.example", env.clone())
            .build()
            .unwrap();
        let err = worker_utility_credit(
            &client,
            "https://api.example",
            WorkerUtilityCreditArgs {
                event_type: UtilityCreditEventType::TrainingUtility,
                credit_points_delta: 1.0,
                reason: "r".into(),
                external_ref: "x".into(),
                submission_ids: vec![],
                bearer_token_env: env,
            },
            true,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("SUBMISSION_ID"));
    }

    #[tokio::test]
    async fn process_evaluation_submit_rejects_bad_overall_score() {
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder("https://api.example", env.clone())
            .build()
            .unwrap();
        let err = process_evaluation_submit(
            &client,
            "https://api.example",
            ProcessEvaluationSubmitArgs {
                submission_id: Uuid::new_v4(),
                reason: "r".into(),
                evaluator_name: None,
                evaluator_version: "v1".into(),
                labels: vec![],
                tool_selection: None,
                tool_argument_quality: None,
                tool_ordering: None,
                verification: None,
                side_effect_safety: None,
                overall_score: Some(2.0),
                utility_credit_points_delta: None,
                utility_external_ref: None,
                bearer_token_env: env,
            },
            true,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("overall-score"));
    }

    #[tokio::test]
    async fn process_evaluation_submit_requires_external_ref_with_delta() {
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder("https://api.example", env.clone())
            .build()
            .unwrap();
        let err = process_evaluation_submit(
            &client,
            "https://api.example",
            ProcessEvaluationSubmitArgs {
                submission_id: Uuid::new_v4(),
                reason: "r".into(),
                evaluator_name: None,
                evaluator_version: "v1".into(),
                labels: vec![],
                tool_selection: None,
                tool_argument_quality: None,
                tool_ordering: None,
                verification: None,
                side_effect_safety: None,
                overall_score: None,
                utility_credit_points_delta: Some(0.2),
                utility_external_ref: None,
                bearer_token_env: env,
            },
            true,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("utility-external-ref"));
    }

    #[tokio::test]
    async fn worker_vector_index_posts_dry_run_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(wm_path("/v1/workers/vector-index"))
            .and(body_json(serde_json::json!({
                "dry_run": true,
                "purpose": "reindex",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "audit_event_id": "a-1",
                "purpose": "reindex",
                "dry_run": true,
                "vector_entries_indexed": 0,
            })))
            .expect(1)
            .mount(&server)
            .await;
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "vector-secret");
        let client = Client::builder(server.uri(), env.clone()).build().unwrap();
        worker_vector_index(
            &client,
            &server.uri(),
            WorkerVectorIndexArgs {
                purpose: Some("reindex".into()),
                dry_run: true,
                bearer_token_env: env,
            },
            true,
        )
        .await
        .expect("vector index dry run");
    }
}
