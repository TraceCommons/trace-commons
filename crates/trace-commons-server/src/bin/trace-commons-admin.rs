//! `trace-commons-admin` — admin-audience operator CLI.
//!
//! Twelve subcommands matching the admin-bearer endpoints on
//! trace-commons-server. All requests are issued through the foundation
//! `trace_commons_operator_client::Client`.

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

use operator_common::{render_items, render_json_map, render_kv_fields, sanitized_url};

const DEFAULT_BEARER_ENV: &str = "TRACE_COMMONS_ADMIN_BEARER";

#[derive(Debug, Parser)]
#[command(
    name = "trace-commons-admin",
    about = "Admin-audience CLI for trace-commons-server.",
    version
)]
pub(crate) struct Cli {
    #[arg(long, env = "TRACE_COMMONS_ENDPOINT")]
    pub endpoint: String,
    #[arg(long, default_value = DEFAULT_BEARER_ENV)]
    pub bearer_token_env: String,
    #[arg(long, env = "TRACE_COMMONS_ALLOWED_HOSTS")]
    pub allowed_hosts: Option<String>,
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: AdminSubcommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum AdminSubcommand {
    MaintenanceRun(MaintenanceRunArgs),
    RetentionJobsList(RetentionJobsListArgs),
    RetentionJobItems(RetentionJobItemsArgs),
    ExportAccessGrantsList(ExportAccessGrantsListArgs),
    ExportJobsList(ExportJobsListArgs),
    BenchmarkConvert(BenchmarkConvertArgs),
    BenchmarkLifecycleUpdate(BenchmarkLifecycleArgs),
    ReplayDatasetExport(ReplayDatasetExportArgs),
    ReplayExportManifests,
    AnalyticsSummary,
    OperationalSummary,
    ConfigStatus,
}

#[derive(Debug, clap::Args)]
pub(crate) struct MaintenanceRunArgs {
    #[arg(long)]
    pub purpose: String,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub backfill_db_mirror: bool,
    #[arg(long)]
    pub index_vectors: bool,
    #[arg(long)]
    pub reconcile_db_mirror: bool,
    #[arg(long)]
    pub verify_audit_chain: bool,
    /// Prune the export cache as part of this run (default: true).
    #[arg(long, default_value_t = true)]
    pub prune_export_cache: bool,
    #[arg(long)]
    pub max_export_age_hours: Option<i64>,
    #[arg(long)]
    pub purge_expired_before: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct RetentionJobsListArgs {
    #[arg(long)]
    pub limit: Option<usize>,
    #[arg(long, value_enum)]
    pub status: Option<RetentionJobStatus>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct RetentionJobItemsArgs {
    #[arg(long)]
    pub retention_job_id: Uuid,
    #[arg(long)]
    pub limit: Option<usize>,
    #[arg(long, value_enum)]
    pub action: Option<RetentionJobItemAction>,
    #[arg(long, value_enum)]
    pub status: Option<RetentionJobItemStatus>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct ExportAccessGrantsListArgs {
    #[arg(long)]
    pub limit: Option<usize>,
    #[arg(long, value_enum)]
    pub status: Option<ExportAccessGrantStatus>,
    #[arg(long)]
    pub dataset_kind: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct ExportJobsListArgs {
    #[arg(long)]
    pub limit: Option<usize>,
    #[arg(long, value_enum)]
    pub status: Option<ExportJobStatus>,
    #[arg(long)]
    pub dataset_kind: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct BenchmarkConvertArgs {
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
}

#[derive(Debug, clap::Args)]
pub(crate) struct BenchmarkLifecycleArgs {
    #[arg(long)]
    pub conversion_id: Uuid,
    #[arg(long, value_enum)]
    pub registry_status: Option<BenchmarkRegistryStatus>,
    #[arg(long)]
    pub registry_ref: Option<String>,
    #[arg(long)]
    pub published_at: Option<String>,
    #[arg(long, value_enum)]
    pub evaluation_status: Option<BenchmarkEvaluationStatus>,
    #[arg(long)]
    pub evaluator_ref: Option<String>,
    #[arg(long)]
    pub evaluated_at: Option<String>,
    #[arg(long)]
    pub score: Option<f32>,
    #[arg(long)]
    pub pass_count: Option<u32>,
    #[arg(long)]
    pub fail_count: Option<u32>,
    #[arg(long)]
    pub reason: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct ReplayDatasetExportArgs {
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
}

// --- value enums ---

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
pub(crate) enum RetentionJobStatus {
    Planned,
    Running,
    DryRun,
    Complete,
    Failed,
    Paused,
}

impl RetentionJobStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Running => "running",
            Self::DryRun => "dry_run",
            Self::Complete => "complete",
            Self::Failed => "failed",
            Self::Paused => "paused",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum RetentionJobItemAction {
    Revoke,
    Expire,
    Purge,
}

impl RetentionJobItemAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Revoke => "revoke",
            Self::Expire => "expire",
            Self::Purge => "purge",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum RetentionJobItemStatus {
    Pending,
    Done,
    Failed,
    Skipped,
}

impl RetentionJobItemStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ExportAccessGrantStatus {
    Active,
    Consumed,
    Revoked,
    Expired,
}

impl ExportAccessGrantStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Consumed => "consumed",
            Self::Revoked => "revoked",
            Self::Expired => "expired",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ExportJobStatus {
    Queued,
    Running,
    Complete,
    Failed,
    Cancelled,
    Expired,
}

impl ExportJobStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Complete => "complete",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
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
pub(crate) enum BenchmarkRegistryStatus {
    Candidate,
    Published,
    Revoked,
}

impl BenchmarkRegistryStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Published => "published",
            Self::Revoked => "revoked",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum BenchmarkEvaluationStatus {
    NotRun,
    Queued,
    Running,
    Passed,
    Failed,
    Inconclusive,
}

impl BenchmarkEvaluationStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::NotRun => "not_run",
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Inconclusive => "inconclusive",
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = build_client(&cli)?;
    dispatch(&client, &cli.endpoint, cli.command, cli.json).await
}

fn build_client(cli: &Cli) -> Result<Client> {
    let mut builder = Client::builder(&cli.endpoint, &cli.bearer_token_env);
    if let Some(csv) = cli.allowed_hosts.as_deref() {
        builder = builder.host_allowlist(HostAllowlist::from_csv(csv));
    }
    Ok(builder.build()?)
}

async fn dispatch(client: &Client, endpoint: &str, cmd: AdminSubcommand, json: bool) -> Result<()> {
    match cmd {
        AdminSubcommand::MaintenanceRun(args) => {
            maintenance_run(client, endpoint, args, json).await
        }
        AdminSubcommand::RetentionJobsList(args) => {
            retention_jobs_list(client, endpoint, args, json).await
        }
        AdminSubcommand::RetentionJobItems(args) => {
            retention_job_items(client, endpoint, args, json).await
        }
        AdminSubcommand::ExportAccessGrantsList(args) => {
            export_access_grants_list(client, endpoint, args, json).await
        }
        AdminSubcommand::ExportJobsList(args) => {
            export_jobs_list(client, endpoint, args, json).await
        }
        AdminSubcommand::BenchmarkConvert(args) => {
            benchmark_convert(client, endpoint, args, json).await
        }
        AdminSubcommand::BenchmarkLifecycleUpdate(args) => {
            benchmark_lifecycle_update(client, endpoint, args, json).await
        }
        AdminSubcommand::ReplayDatasetExport(args) => {
            replay_dataset_export(client, endpoint, args, json).await
        }
        AdminSubcommand::ReplayExportManifests => {
            replay_export_manifests(client, endpoint, json).await
        }
        AdminSubcommand::AnalyticsSummary => analytics_summary(client, endpoint, json).await,
        AdminSubcommand::OperationalSummary => operational_summary(client, endpoint, json).await,
        AdminSubcommand::ConfigStatus => config_status(client, endpoint, json).await,
    }
}

async fn maintenance_run(
    client: &Client,
    endpoint: &str,
    args: MaintenanceRunArgs,
    json: bool,
) -> Result<()> {
    let trimmed = args.purpose.trim();
    if trimmed.is_empty() {
        anyhow::bail!("--purpose must be non-empty");
    }
    let path = "/v1/admin/maintenance";
    let mut body = serde_json::json!({
        "purpose": trimmed,
        "dry_run": args.dry_run,
    });
    if args.backfill_db_mirror {
        body["backfill_db_mirror"] = Value::Bool(true);
    }
    if args.index_vectors {
        body["index_vectors"] = Value::Bool(true);
    }
    if args.reconcile_db_mirror {
        body["reconcile_db_mirror"] = Value::Bool(true);
    }
    if args.verify_audit_chain {
        body["verify_audit_chain"] = Value::Bool(true);
    }
    if !args.prune_export_cache {
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
        writeln!(out, "Trace Commons maintenance run requested.")?;
        render_kv_fields(
            &mut out,
            &value,
            &[
                ("  audit event id", "audit_event_id"),
                ("  purpose", "purpose"),
                ("  dry run", "dry_run"),
                ("  revoked submissions", "revoked_submission_count"),
                ("  expired submissions", "expired_submission_count"),
                ("  records marked revoked", "records_marked_revoked"),
                ("  records marked expired", "records_marked_expired"),
                ("  records marked purged", "records_marked_purged"),
                ("  derived marked revoked", "derived_marked_revoked"),
                ("  derived marked expired", "derived_marked_expired"),
                ("  export cache files pruned", "export_cache_files_pruned"),
                (
                    "  export provenance invalidated",
                    "export_provenance_invalidated",
                ),
                (
                    "  benchmark artifacts invalidated",
                    "benchmark_artifacts_invalidated",
                ),
                ("  trace object files deleted", "trace_object_files_deleted"),
                (
                    "  encrypted artifacts deleted",
                    "encrypted_artifacts_deleted",
                ),
                ("  DB mirror backfilled", "db_mirror_backfilled"),
                ("  vectors indexed", "vector_entries_indexed"),
            ],
        )?;
    }
    Ok(())
}

async fn retention_jobs_list(
    client: &Client,
    endpoint: &str,
    args: RetentionJobsListArgs,
    json: bool,
) -> Result<()> {
    let path = "/v1/admin/retention/jobs";
    let mut owned: Vec<(&str, String)> = Vec::new();
    if let Some(limit) = args.limit {
        owned.push(("limit", limit.to_string()));
    }
    if let Some(status) = args.status {
        owned.push(("status", status.as_str().to_string()));
    }
    let query = borrow_query(&owned);
    let value: Value = client
        .call_json::<(), Value>(Method::GET, path, &query, None)
        .await?;
    if json {
        emit_json(endpoint, "GET", path, &value)?;
    } else {
        render_items(
            &mut stdout(),
            "Central retention maintenance jobs",
            &value,
            &[
                "retention_job_id",
                "status",
                "purpose",
                "dry_run",
                "selected_revoked_count",
                "selected_expired_count",
                "started_at",
                "completed_at",
                "created_at",
            ],
        )?;
    }
    Ok(())
}

async fn retention_job_items(
    client: &Client,
    endpoint: &str,
    args: RetentionJobItemsArgs,
    json: bool,
) -> Result<()> {
    let path = format!("/v1/admin/retention/jobs/{}/items", args.retention_job_id);
    let mut owned: Vec<(&str, String)> = Vec::new();
    if let Some(limit) = args.limit {
        owned.push(("limit", limit.to_string()));
    }
    if let Some(action) = args.action {
        owned.push(("action", action.as_str().to_string()));
    }
    if let Some(status) = args.status {
        owned.push(("status", status.as_str().to_string()));
    }
    let query = borrow_query(&owned);
    let value: Value = client
        .call_json::<(), Value>(Method::GET, &path, &query, None)
        .await?;
    if json {
        emit_json(endpoint, "GET", &path, &value)?;
    } else {
        render_items(
            &mut stdout(),
            "Central retention maintenance job items",
            &value,
            &[
                "submission_id",
                "action",
                "status",
                "reason",
                "verified_at",
                "updated_at",
            ],
        )?;
    }
    Ok(())
}

async fn export_access_grants_list(
    client: &Client,
    endpoint: &str,
    args: ExportAccessGrantsListArgs,
    json: bool,
) -> Result<()> {
    let path = "/v1/admin/export/access-grants";
    let mut owned: Vec<(&str, String)> = Vec::new();
    if let Some(limit) = args.limit {
        owned.push(("limit", limit.to_string()));
    }
    if let Some(status) = args.status {
        owned.push(("status", status.as_str().to_string()));
    }
    if let Some(kind) = args.dataset_kind {
        owned.push(("dataset_kind", kind));
    }
    let query = borrow_query(&owned);
    let value: Value = client
        .call_json::<(), Value>(Method::GET, path, &query, None)
        .await?;
    if json {
        emit_json(endpoint, "GET", path, &value)?;
    } else {
        render_items(
            &mut stdout(),
            "Central export access grants",
            &value,
            &[
                "grant_id",
                "export_job_id",
                "status",
                "requested_dataset_kind",
                "purpose",
                "max_item_cap",
                "requested_at",
                "expires_at",
            ],
        )?;
    }
    Ok(())
}

async fn export_jobs_list(
    client: &Client,
    endpoint: &str,
    args: ExportJobsListArgs,
    json: bool,
) -> Result<()> {
    let path = "/v1/admin/export/jobs";
    let mut owned: Vec<(&str, String)> = Vec::new();
    if let Some(limit) = args.limit {
        owned.push(("limit", limit.to_string()));
    }
    if let Some(status) = args.status {
        owned.push(("status", status.as_str().to_string()));
    }
    if let Some(kind) = args.dataset_kind {
        owned.push(("dataset_kind", kind));
    }
    let query = borrow_query(&owned);
    let value: Value = client
        .call_json::<(), Value>(Method::GET, path, &query, None)
        .await?;
    if json {
        emit_json(endpoint, "GET", path, &value)?;
    } else {
        render_items(
            &mut stdout(),
            "Central export jobs",
            &value,
            &[
                "export_job_id",
                "status",
                "requested_dataset_kind",
                "purpose",
                "item_count",
                "result_manifest_id",
                "started_at",
                "finished_at",
            ],
        )?;
    }
    Ok(())
}

async fn benchmark_convert(
    client: &Client,
    endpoint: &str,
    args: BenchmarkConvertArgs,
    json: bool,
) -> Result<()> {
    let trimmed = args.purpose.trim();
    if trimmed.is_empty() {
        anyhow::bail!("--purpose must be non-empty");
    }
    let path = "/v1/benchmarks/convert";
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
        writeln!(out, "Trace Commons benchmark conversion requested.")?;
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

async fn benchmark_lifecycle_update(
    client: &Client,
    endpoint: &str,
    args: BenchmarkLifecycleArgs,
    json: bool,
) -> Result<()> {
    if let Some(score) = args.score {
        if !(0.0..=1.0).contains(&score) {
            anyhow::bail!("--score must be between 0.0 and 1.0");
        }
    }
    let mut body = serde_json::Map::new();
    let mut registry = serde_json::Map::new();
    if let Some(status) = args.registry_status {
        registry.insert("status".into(), Value::String(status.as_str().to_string()));
    }
    if let Some(rref) = args.registry_ref {
        registry.insert("registry_ref".into(), Value::String(rref));
    }
    if let Some(at) = args.published_at {
        registry.insert("published_at".into(), Value::String(at));
    }
    if !registry.is_empty() {
        body.insert("registry".into(), Value::Object(registry));
    }
    let mut evaluation = serde_json::Map::new();
    if let Some(status) = args.evaluation_status {
        evaluation.insert("status".into(), Value::String(status.as_str().to_string()));
    }
    if let Some(eref) = args.evaluator_ref {
        evaluation.insert("evaluator_ref".into(), Value::String(eref));
    }
    if let Some(at) = args.evaluated_at {
        evaluation.insert("evaluated_at".into(), Value::String(at));
    }
    if let Some(score) = args.score {
        evaluation.insert("score".into(), serde_json::json!(score));
    }
    if let Some(pc) = args.pass_count {
        evaluation.insert("pass_count".into(), serde_json::json!(pc));
    }
    if let Some(fc) = args.fail_count {
        evaluation.insert("fail_count".into(), serde_json::json!(fc));
    }
    if !evaluation.is_empty() {
        body.insert("evaluation".into(), Value::Object(evaluation));
    }
    if body.is_empty() {
        anyhow::bail!(
            "benchmark lifecycle update requires at least one registry or evaluation field"
        );
    }
    if let Some(reason) = args.reason {
        body.insert("reason".into(), Value::String(reason));
    }
    let body = Value::Object(body);
    let path = format!("/v1/benchmarks/{}/lifecycle", args.conversion_id);
    let value: Value = client
        .call_json::<Value, Value>(Method::POST, &path, &[], Some(&body))
        .await?;
    if json {
        emit_json(endpoint, "POST", &path, &value)?;
    } else {
        let mut out = stdout();
        writeln!(out, "Trace Commons benchmark lifecycle updated.")?;
        render_kv_fields(
            &mut out,
            &value,
            &[
                ("  conversion id", "conversion_id"),
                ("  purpose", "purpose"),
            ],
        )?;
        if let Some(registry) = value.get("registry") {
            render_kv_fields(
                &mut out,
                registry,
                &[
                    ("  registry status", "status"),
                    ("  registry ref", "registry_ref"),
                ],
            )?;
        }
        if let Some(evaluation) = value.get("evaluation") {
            render_kv_fields(
                &mut out,
                evaluation,
                &[
                    ("  evaluation status", "status"),
                    ("  evaluator ref", "evaluator_ref"),
                    ("  score", "score"),
                ],
            )?;
        }
    }
    Ok(())
}

async fn replay_dataset_export(
    client: &Client,
    endpoint: &str,
    args: ReplayDatasetExportArgs,
    json: bool,
) -> Result<()> {
    let path = "/v1/datasets/replay";
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

async fn replay_export_manifests(client: &Client, endpoint: &str, json: bool) -> Result<()> {
    let path = "/v1/datasets/replay/manifests";
    let value: Value = client
        .call_json::<(), Value>(Method::GET, path, &[], None)
        .await?;
    if json {
        emit_json(endpoint, "GET", path, &value)?;
    } else {
        render_items(
            &mut stdout(),
            "Central replay export manifests",
            &value,
            &[
                "export_manifest_id",
                "purpose_code",
                "item_count",
                "source_submission_ids_hash",
                "generated_at",
                "invalidated_at",
            ],
        )?;
    }
    Ok(())
}

async fn analytics_summary(client: &Client, endpoint: &str, json: bool) -> Result<()> {
    let path = "/v1/analytics/summary";
    let value: Value = client
        .call_json::<(), Value>(Method::GET, path, &[], None)
        .await?;
    if json {
        emit_json(endpoint, "GET", path, &value)?;
    } else {
        let mut out = stdout();
        writeln!(out, "Trace Commons analytics summary:")?;
        render_kv_fields(
            &mut out,
            &value,
            &[
                ("  tenant", "tenant_id"),
                ("  submissions", "submissions_total"),
                ("  min cell count", "min_cell_count"),
                ("  suppressed cells", "suppressed_cell_count"),
                ("  duplicate groups", "duplicate_groups"),
                ("  average novelty", "average_novelty_score"),
            ],
        )?;
        render_json_map(&mut out, "  by status", value.get("by_status"))?;
        render_json_map(&mut out, "  by privacy risk", value.get("by_privacy_risk"))?;
        render_json_map(&mut out, "  by task success", value.get("by_task_success"))?;
        if let Some(pe) = value.get("process_evaluation") {
            render_kv_fields(
                &mut out,
                pe,
                &[("  process evaluated traces", "evaluated_traces")],
            )?;
            render_json_map(&mut out, "  process labels", pe.get("by_label"))?;
            render_json_map(&mut out, "  process ratings", pe.get("by_rating"))?;
            render_json_map(&mut out, "  process score bands", pe.get("by_score_band"))?;
        }
    }
    Ok(())
}

async fn operational_summary(client: &Client, endpoint: &str, json: bool) -> Result<()> {
    let path = "/v1/admin/operational-summary";
    let value: Value = client
        .call_json::<(), Value>(Method::GET, path, &[], None)
        .await?;
    if json {
        emit_json(endpoint, "GET", path, &value)?;
    } else {
        let mut out = stdout();
        writeln!(out, "Trace Commons operational summary:")?;
        render_kv_fields(
            &mut out,
            &value,
            &[
                ("  tenant", "tenant_id"),
                ("  tenant storage ref", "tenant_storage_ref"),
                ("  generated at", "generated_at"),
            ],
        )?;
        render_subsection(&mut out, "  submissions", value.get("submissions"))?;
        render_subsection(&mut out, "  review sla", value.get("review_sla"))?;
        render_subsection(
            &mut out,
            "  export",
            value.get("exports").or_else(|| value.get("export")),
        )?;
        render_subsection(&mut out, "  retention", value.get("retention"))?;
        render_subsection(&mut out, "  vectors", value.get("vectors"))?;
        render_subsection(&mut out, "  delayed credit", value.get("delayed_credit"))?;
        render_subsection(&mut out, "  promotion gates", value.get("promotion_gates"))?;
    }
    Ok(())
}

async fn config_status(client: &Client, endpoint: &str, _json: bool) -> Result<()> {
    let path = "/v1/admin/config-status";
    let value: Value = client
        .call_json::<(), Value>(Method::GET, path, &[], None)
        .await?;
    emit_json(endpoint, "GET", path, &value)
}

// --- helpers ---

fn borrow_query<'a>(owned: &'a [(&'a str, String)]) -> Vec<(&'a str, &'a str)> {
    owned.iter().map(|(k, v)| (*k, v.as_str())).collect()
}

/// Render a subsection of the operational summary as a list of `field = value`
/// scalar lines plus any nested maps rendered via [`render_json_map`].
fn render_subsection<W: Write>(
    out: &mut W,
    heading: &str,
    value: Option<&Value>,
) -> std::io::Result<()> {
    let Some(obj) = value.and_then(Value::as_object) else {
        return Ok(());
    };
    if obj.is_empty() {
        return Ok(());
    }
    writeln!(out, "{heading}:")?;
    for (k, v) in obj {
        if v.is_object() {
            render_json_map(out, &format!("    {k}"), Some(v))?;
        } else if let Some(arr) = v.as_array() {
            let items: Vec<String> = arr
                .iter()
                .filter_map(|v| {
                    v.as_str()
                        .map(ToString::to_string)
                        .or_else(|| v.as_i64().map(|n| n.to_string()))
                        .or_else(|| v.as_u64().map(|n| n.to_string()))
                        .or_else(|| v.as_bool().map(|b| b.to_string()))
                })
                .collect();
            if !items.is_empty() {
                writeln!(out, "    {k}: {}", items.join(" "))?;
            }
        } else if !v.is_null() {
            let s = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            writeln!(out, "    {k}: {s}")?;
        }
    }
    Ok(())
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
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn unique_bearer_env() -> String {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        format!("TC_ADMIN_TEST_BEARER_{n}")
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

    #[test]
    fn cli_parses_analytics_summary() {
        let cli = Cli::try_parse_from([
            "trace-commons-admin",
            "--endpoint",
            "https://api.example",
            "analytics-summary",
        ])
        .unwrap();
        assert!(matches!(cli.command, AdminSubcommand::AnalyticsSummary));
    }

    #[test]
    fn cli_parses_operational_summary() {
        Cli::try_parse_from([
            "trace-commons-admin",
            "--endpoint",
            "https://api.example",
            "operational-summary",
        ])
        .unwrap();
    }

    #[test]
    fn cli_parses_config_status() {
        Cli::try_parse_from([
            "trace-commons-admin",
            "--endpoint",
            "https://api.example",
            "config-status",
        ])
        .unwrap();
    }

    #[test]
    fn cli_parses_maintenance_run_full() {
        let cli = Cli::try_parse_from([
            "trace-commons-admin",
            "--endpoint",
            "https://api.example",
            "maintenance-run",
            "--purpose",
            "monthly retention",
            "--dry-run",
            "--backfill-db-mirror",
            "--index-vectors",
            "--reconcile-db-mirror",
            "--verify-audit-chain",
            "--max-export-age-hours",
            "48",
        ])
        .unwrap();
        match cli.command {
            AdminSubcommand::MaintenanceRun(args) => {
                assert_eq!(args.purpose, "monthly retention");
                assert!(args.dry_run);
                assert!(args.backfill_db_mirror);
                assert!(args.index_vectors);
                assert!(args.reconcile_db_mirror);
                assert!(args.verify_audit_chain);
                assert_eq!(args.max_export_age_hours, Some(48));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_retention_jobs_list() {
        Cli::try_parse_from([
            "trace-commons-admin",
            "--endpoint",
            "https://api.example",
            "retention-jobs-list",
            "--limit",
            "10",
            "--status",
            "running",
        ])
        .unwrap();
    }

    #[test]
    fn cli_parses_retention_job_items() {
        let id = Uuid::new_v4();
        Cli::try_parse_from([
            "trace-commons-admin",
            "--endpoint",
            "https://api.example",
            "retention-job-items",
            "--retention-job-id",
            &id.to_string(),
            "--action",
            "purge",
            "--status",
            "done",
        ])
        .unwrap();
    }

    #[test]
    fn cli_parses_export_access_grants_list() {
        Cli::try_parse_from([
            "trace-commons-admin",
            "--endpoint",
            "https://api.example",
            "export-access-grants-list",
            "--status",
            "active",
            "--dataset-kind",
            "ranking",
        ])
        .unwrap();
    }

    #[test]
    fn cli_parses_export_jobs_list() {
        Cli::try_parse_from([
            "trace-commons-admin",
            "--endpoint",
            "https://api.example",
            "export-jobs-list",
            "--status",
            "complete",
        ])
        .unwrap();
    }

    #[test]
    fn cli_parses_benchmark_convert() {
        Cli::try_parse_from([
            "trace-commons-admin",
            "--endpoint",
            "https://api.example",
            "benchmark-convert",
            "--purpose",
            "weekly",
            "--limit",
            "50",
            "--consent-scope",
            "benchmark-only",
            "--status",
            "accepted",
            "--privacy-risk",
            "low",
        ])
        .unwrap();
    }

    #[test]
    fn cli_parses_benchmark_lifecycle_update() {
        let id = Uuid::new_v4();
        Cli::try_parse_from([
            "trace-commons-admin",
            "--endpoint",
            "https://api.example",
            "benchmark-lifecycle-update",
            "--conversion-id",
            &id.to_string(),
            "--registry-status",
            "published",
            "--evaluation-status",
            "passed",
            "--score",
            "0.85",
        ])
        .unwrap();
    }

    #[test]
    fn cli_parses_replay_dataset_export() {
        Cli::try_parse_from([
            "trace-commons-admin",
            "--endpoint",
            "https://api.example",
            "replay-dataset-export",
            "--purpose",
            "ranking",
            "--consent-scope",
            "ranking-training",
            "--status",
            "accepted",
            "--privacy-risk",
            "medium",
            "--limit",
            "100",
        ])
        .unwrap();
    }

    #[test]
    fn cli_parses_replay_export_manifests() {
        Cli::try_parse_from([
            "trace-commons-admin",
            "--endpoint",
            "https://api.example",
            "replay-export-manifests",
        ])
        .unwrap();
    }

    #[test]
    fn cli_rejects_unknown_subcommand() {
        let err = Cli::try_parse_from([
            "trace-commons-admin",
            "--endpoint",
            "https://api.example",
            "bogus-subcommand",
        ])
        .unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn cli_rejects_maintenance_run_missing_purpose() {
        let err = Cli::try_parse_from([
            "trace-commons-admin",
            "--endpoint",
            "https://api.example",
            "maintenance-run",
        ])
        .unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn cli_rejects_benchmark_lifecycle_bad_score() {
        // The lifecycle args themselves accept any f32; the > 1.0 check is at
        // runtime. So this only verifies the parser is wired.
        let id = Uuid::new_v4();
        Cli::try_parse_from([
            "trace-commons-admin",
            "--endpoint",
            "https://api.example",
            "benchmark-lifecycle-update",
            "--conversion-id",
            &id.to_string(),
            "--score",
            "2.0",
        ])
        .unwrap();
    }

    #[tokio::test]
    async fn analytics_summary_happy_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/analytics/summary"))
            .and(header("authorization", "Bearer admin-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "tenant_id": "tenant-1",
                "submissions_total": 42,
                "by_status": {"accepted": 30, "rejected": 12},
            })))
            .expect(1)
            .mount(&server)
            .await;
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "admin-secret");
        let client = Client::builder(server.uri(), env).build().unwrap();
        analytics_summary(&client, &server.uri(), true)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn benchmark_convert_posts_expected_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/benchmarks/convert"))
            .and(body_json(serde_json::json!({
                "purpose": "weekly",
                "limit": 50,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "conversion_id": "conv-1",
                "item_count": 5,
            })))
            .expect(1)
            .mount(&server)
            .await;
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "admin-secret");
        let client = Client::builder(server.uri(), env).build().unwrap();
        benchmark_convert(
            &client,
            &server.uri(),
            BenchmarkConvertArgs {
                purpose: "weekly".into(),
                limit: Some(50),
                consent_scope: None,
                status: None,
                privacy_risk: None,
                external_ref: None,
            },
            true,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn retention_jobs_list_attaches_query_params() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/admin/retention/jobs"))
            .and(query_param("limit", "5"))
            .and(query_param("status", "running"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": []
            })))
            .expect(1)
            .mount(&server)
            .await;
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "admin-secret");
        let client = Client::builder(server.uri(), env).build().unwrap();
        retention_jobs_list(
            &client,
            &server.uri(),
            RetentionJobsListArgs {
                limit: Some(5),
                status: Some(RetentionJobStatus::Running),
            },
            true,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn benchmark_lifecycle_update_rejects_out_of_range_score() {
        let server = MockServer::start().await;
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder(server.uri(), env).build().unwrap();
        let err = benchmark_lifecycle_update(
            &client,
            &server.uri(),
            BenchmarkLifecycleArgs {
                conversion_id: Uuid::new_v4(),
                registry_status: None,
                registry_ref: None,
                published_at: None,
                evaluation_status: None,
                evaluator_ref: None,
                evaluated_at: None,
                score: Some(2.0),
                pass_count: None,
                fail_count: None,
                reason: None,
            },
            true,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("score"));
    }

    #[tokio::test]
    async fn benchmark_lifecycle_update_rejects_empty_body() {
        let server = MockServer::start().await;
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder(server.uri(), env).build().unwrap();
        let err = benchmark_lifecycle_update(
            &client,
            &server.uri(),
            BenchmarkLifecycleArgs {
                conversion_id: Uuid::new_v4(),
                registry_status: None,
                registry_ref: None,
                published_at: None,
                evaluation_status: None,
                evaluator_ref: None,
                evaluated_at: None,
                score: None,
                pass_count: None,
                fail_count: None,
                reason: None,
            },
            true,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("registry or evaluation"));
    }
}
