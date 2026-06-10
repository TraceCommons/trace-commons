//! `trace-commons-tenant` — tenant-audience operator CLI.
//!
//! Twelve subcommands covering tenant-policy management, tenant access-grant
//! lifecycle, ranker training exports, audit-event listing, trace listing,
//! device-key registry operations, and a local privacy-filter canary helper.
//! All HTTP-backed subcommands
//! share a single `--bearer-token-env` defaulting to
//! `TRACE_COMMONS_TENANT_BEARER`. Two subcommands (`tenant-principal-ref`,
//! `privacy-filter-canary`) do not hit the server: the first derives a
//! principal ref locally; the second spawns a locally configured
//! privacy-filter sidecar subprocess and confirms it scrubs canary tokens.
//! See `docs/operator/operator-binaries.md` for the sidecar env-var
//! contract.

#[path = "operator_common/mod.rs"]
mod operator_common;

use std::io::{Write, stdout};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use reqwest::Method;
use serde_json::Value;
use sha2::{Digest, Sha256};
use trace_commons_operator_client::privacy_filter;
use trace_commons_operator_client::{Client, format as oc_format, host_allowlist::HostAllowlist};
use uuid::Uuid;

use operator_common::{render_items, render_kv_fields, sanitized_url};

const DEFAULT_BEARER_ENV: &str = "TRACE_COMMONS_TENANT_BEARER";

#[derive(Debug, Parser)]
#[command(
    name = "trace-commons-tenant",
    about = "Tenant-audience CLI for trace-commons-server.",
    version
)]
pub(crate) struct Cli {
    /// Hosted endpoint. Reads `TRACE_COMMONS_ENDPOINT` when unset on the CLI.
    /// Not required for `tenant-principal-ref` or `privacy-filter-canary`.
    #[arg(long, env = "TRACE_COMMONS_ENDPOINT")]
    pub endpoint: Option<String>,

    /// Name of the env var holding the tenant bearer token.
    #[arg(long, default_value = DEFAULT_BEARER_ENV)]
    pub bearer_token_env: String,

    /// Defense-in-depth host allowlist (CSV). Empty means permissive.
    #[arg(long, env = "TRACE_COMMONS_ALLOWED_HOSTS")]
    pub allowed_hosts: Option<String>,

    /// Emit the server response as JSON (with a sanitized request envelope).
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: TenantSubcommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum TenantSubcommand {
    /// Read the DB-backed tenant contribution policy.
    TenantPolicyGet,
    /// Write the DB-backed tenant contribution policy.
    TenantPolicySet(TenantPolicySetArgs),
    /// List DB-backed tenant access grants for hosted-agent permissioning.
    TenantAccessGrantsList(TenantAccessGrantsListArgs),
    /// List or revoke registered onboarding device keys.
    DeviceKeys(DeviceKeysArgs),
    /// Derive the stored principal_ref used by tenant access grants.
    TenantPrincipalRef(TenantPrincipalRefArgs),
    /// Create a DB-backed tenant access grant for an issuer-authorized principal.
    TenantAccessGrantCreate(TenantAccessGrantCreateArgs),
    /// Revoke a DB-backed tenant access grant.
    TenantAccessGrantRevoke(TenantAccessGrantRevokeArgs),
    /// Export approved ranker training candidates.
    RankerTrainingCandidates(RankerTrainingArgs),
    /// Export approved ranker training pairs.
    RankerTrainingPairs(RankerTrainingArgs),
    /// List central Trace Commons audit events.
    AuditEvents(AuditEventsArgs),
    /// List central trace metadata with optional reviewer filters.
    ListTraces(ListTracesArgs),
    /// Run a local Privacy Filter sidecar canary check.
    PrivacyFilterCanary(PrivacyFilterCanaryArgs),
}

#[derive(Debug, clap::Args)]
pub(crate) struct TenantPolicySetArgs {
    #[arg(long)]
    pub policy_version: String,
    #[arg(long, value_enum, value_delimiter = ',')]
    pub allowed_consent_scopes: Vec<ConsentScope>,
    #[arg(long, value_enum, value_delimiter = ',')]
    pub allowed_uses: Vec<AllowedUse>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct TenantAccessGrantsListArgs {
    #[arg(long)]
    pub limit: Option<usize>,
    #[arg(long, value_enum)]
    pub status: Option<TenantAccessGrantStatus>,
    #[arg(long, value_enum)]
    pub role: Option<TenantAccessGrantRole>,
    #[arg(long)]
    pub principal_ref: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct TenantPrincipalRefArgs {
    /// Env var name holding a static Trace Commons bearer token. Mutually
    /// exclusive with the signed-claim and device-key flags below.
    #[arg(long)]
    pub token_env: Option<String>,
    /// Tenant ID for a signed-claim principal.
    #[arg(long)]
    pub signed_tenant_id: Option<String>,
    /// Signed-claim principal_ref or `sub` value.
    #[arg(long)]
    pub signed_actor_ref: Option<String>,
    /// Tenant ID for an onboarding device-key principal.
    #[arg(long)]
    pub device_tenant_id: Option<String>,
    /// Registered onboarding device_key_id.
    #[arg(long)]
    pub device_key_id: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct TenantAccessGrantCreateArgs {
    #[arg(long)]
    pub principal_ref: String,
    #[arg(long, value_enum)]
    pub role: TenantAccessGrantRole,
    #[arg(long)]
    pub grant_id: Option<Uuid>,
    #[arg(long, value_enum, value_delimiter = ',')]
    pub allowed_consent_scopes: Vec<ConsentScope>,
    #[arg(long, value_enum, value_delimiter = ',')]
    pub allowed_uses: Vec<AllowedUse>,
    #[arg(long)]
    pub issuer: Option<String>,
    #[arg(long)]
    pub audience: Option<String>,
    #[arg(long)]
    pub subject: Option<String>,
    #[arg(long)]
    pub issued_at: Option<String>,
    #[arg(long)]
    pub expires_at: Option<String>,
    #[arg(long)]
    pub reason: String,
    /// Safe grant metadata as KEY=VALUE, repeatable.
    #[arg(long = "metadata", value_name = "KEY=VALUE")]
    pub metadata: Vec<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct TenantAccessGrantRevokeArgs {
    #[arg(long)]
    pub grant_id: Uuid,
    #[arg(long)]
    pub reason: String,
}

#[derive(Debug, clap::Args)]
pub(crate) struct DeviceKeysArgs {
    #[command(subcommand)]
    pub command: DeviceKeysSubcommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum DeviceKeysSubcommand {
    /// List registered device keys for the authenticated tenant.
    List(DeviceKeysListArgs),
    /// Revoke one registered device key.
    Revoke(DeviceKeyRevokeArgs),
}

#[derive(Debug, clap::Args)]
pub(crate) struct DeviceKeysListArgs {
    #[arg(long)]
    pub tenant: Option<String>,
    #[arg(long)]
    pub limit: Option<usize>,
    #[arg(long)]
    pub include_revoked: bool,
}

#[derive(Debug, clap::Args)]
pub(crate) struct DeviceKeyRevokeArgs {
    pub device_key_id: String,
}

#[derive(Debug, clap::Args)]
pub(crate) struct RankerTrainingArgs {
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
    #[arg(long)]
    pub output: Option<PathBuf>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct AuditEventsArgs {
    #[arg(long)]
    pub limit: Option<usize>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct ListTracesArgs {
    #[arg(long)]
    pub purpose: Option<String>,
    #[arg(long, value_enum)]
    pub consent_scope: Option<ConsentScope>,
    #[arg(long, value_enum)]
    pub status: Option<CorpusStatus>,
    #[arg(long)]
    pub limit: Option<usize>,
    #[arg(long)]
    pub coverage_tag: Option<String>,
    #[arg(long)]
    pub tool: Option<String>,
    #[arg(long, value_enum)]
    pub privacy_risk: Option<PrivacyRisk>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct PrivacyFilterCanaryArgs {
    #[arg(
        long,
        default_value = "Canary email alice@example.com with key sk-test-123 and path /Users/alice/private.txt"
    )]
    pub text: String,
    #[arg(long, default_value_t = 10)]
    pub timeout_seconds: u64,
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
pub(crate) enum ConsentScope {
    DebuggingEvaluation,
    BenchmarkOnly,
    RankingTraining,
    ModelTraining,
    PublicAttribution,
}

impl ConsentScope {
    /// Server wire value (snake_case).
    fn server_value(self) -> &'static str {
        match self {
            Self::DebuggingEvaluation => "debugging_evaluation",
            Self::BenchmarkOnly => "benchmark_only",
            Self::RankingTraining => "ranking_training",
            Self::ModelTraining => "model_training",
            Self::PublicAttribution => "public_attribution",
        }
    }
    /// Query-string value (kebab-case, matches reviewer/admin filters).
    fn query_value(self) -> &'static str {
        match self {
            Self::DebuggingEvaluation => "debugging-evaluation",
            Self::BenchmarkOnly => "benchmark-only",
            Self::RankingTraining => "ranking-training",
            Self::ModelTraining => "model-training",
            Self::PublicAttribution => "public-attribution",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum AllowedUse {
    Debugging,
    Evaluation,
    BenchmarkGeneration,
    RankingModelTraining,
    ModelTraining,
    AggregateAnalytics,
}

impl AllowedUse {
    fn server_value(self) -> &'static str {
        match self {
            Self::Debugging => "debugging",
            Self::Evaluation => "evaluation",
            Self::BenchmarkGeneration => "benchmark_generation",
            Self::RankingModelTraining => "ranking_model_training",
            Self::ModelTraining => "model_training",
            Self::AggregateAnalytics => "aggregate_analytics",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum TenantAccessGrantStatus {
    Active,
    Revoked,
    Expired,
}

impl TenantAccessGrantStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Revoked => "revoked",
            Self::Expired => "expired",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum TenantAccessGrantRole {
    Contributor,
    Reviewer,
    Admin,
    Benchmarker,
    Exporter,
}

impl TenantAccessGrantRole {
    fn server_value(self) -> &'static str {
        match self {
            Self::Contributor => "contributor",
            Self::Reviewer => "reviewer",
            Self::Admin => "admin",
            Self::Benchmarker => "benchmarker",
            Self::Exporter => "exporter",
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    dispatch(cli).await
}

fn build_client(cli: &Cli) -> Result<Client> {
    let endpoint = cli
        .endpoint
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("--endpoint (or TRACE_COMMONS_ENDPOINT) is required"))?;
    let mut builder = Client::builder(endpoint, &cli.bearer_token_env);
    if let Some(csv) = cli.allowed_hosts.as_deref() {
        builder = builder.host_allowlist(HostAllowlist::from_csv(csv));
    }
    Ok(builder.build()?)
}

async fn dispatch(cli: Cli) -> Result<()> {
    // Subcommands that don't hit the server: handle first so we don't require
    // an endpoint to be configured.
    match &cli.command {
        TenantSubcommand::TenantPrincipalRef(args) => {
            return tenant_principal_ref(args, cli.json);
        }
        TenantSubcommand::PrivacyFilterCanary(args) => {
            return privacy_filter_canary(args, cli.json).await;
        }
        _ => {}
    }
    let client = build_client(&cli)?;
    let endpoint = cli.endpoint.clone().unwrap();
    let json = cli.json;
    match cli.command {
        TenantSubcommand::TenantPolicyGet => tenant_policy_get(&client, &endpoint, json).await,
        TenantSubcommand::TenantPolicySet(args) => {
            tenant_policy_set(&client, &endpoint, args, json).await
        }
        TenantSubcommand::TenantAccessGrantsList(args) => {
            tenant_access_grants_list(&client, &endpoint, args, json).await
        }
        TenantSubcommand::DeviceKeys(args) => device_keys(&client, &endpoint, args, json).await,
        TenantSubcommand::TenantAccessGrantCreate(args) => {
            tenant_access_grant_create(&client, &endpoint, args, json).await
        }
        TenantSubcommand::TenantAccessGrantRevoke(args) => {
            tenant_access_grant_revoke(&client, &endpoint, args, json).await
        }
        TenantSubcommand::RankerTrainingCandidates(args) => {
            ranker_training_export(
                &client,
                &endpoint,
                args,
                "/v1/ranker/training-candidates",
                "ranker training candidates",
                "candidates",
                json,
            )
            .await
        }
        TenantSubcommand::RankerTrainingPairs(args) => {
            ranker_training_export(
                &client,
                &endpoint,
                args,
                "/v1/ranker/training-pairs",
                "ranker training pairs",
                "pairs",
                json,
            )
            .await
        }
        TenantSubcommand::AuditEvents(args) => audit_events(&client, &endpoint, args, json).await,
        TenantSubcommand::ListTraces(args) => list_traces(&client, &endpoint, args, json).await,
        TenantSubcommand::TenantPrincipalRef(_) | TenantSubcommand::PrivacyFilterCanary(_) => {
            unreachable!("handled above")
        }
    }
}

// --- handlers ---

async fn tenant_policy_get(client: &Client, endpoint: &str, json: bool) -> Result<()> {
    let path = "/v1/admin/tenant-policy";
    let value: Value = client
        .call_json::<(), Value>(Method::GET, path, &[], None)
        .await?;
    if json {
        emit_json(endpoint, "GET", path, &value)?;
    } else {
        render_tenant_policy(&value)?;
    }
    Ok(())
}

async fn tenant_policy_set(
    client: &Client,
    endpoint: &str,
    args: TenantPolicySetArgs,
    json: bool,
) -> Result<()> {
    let version = args.policy_version.trim();
    if version.is_empty() {
        anyhow::bail!("--policy-version must not be empty");
    }
    let path = "/v1/admin/tenant-policy";
    let body = serde_json::json!({
        "policy_version": version,
        "allowed_consent_scopes": args
            .allowed_consent_scopes
            .iter()
            .map(|s| s.server_value())
            .collect::<Vec<_>>(),
        "allowed_uses": args
            .allowed_uses
            .iter()
            .map(|u| u.server_value())
            .collect::<Vec<_>>(),
    });
    let value: Value = client
        .call_json::<Value, Value>(Method::PUT, path, &[], Some(&body))
        .await?;
    if json {
        emit_json(endpoint, "PUT", path, &value)?;
    } else {
        render_tenant_policy(&value)?;
    }
    Ok(())
}

fn render_tenant_policy(value: &Value) -> Result<()> {
    let mut out = stdout();
    writeln!(out, "Trace Commons tenant policy:")?;
    render_kv_fields(
        &mut out,
        value,
        &[
            ("  tenant", "tenant_id"),
            ("  policy version", "policy_version"),
            ("  allowed consent scopes", "allowed_consent_scopes"),
            ("  allowed uses", "allowed_uses"),
            ("  updated by", "updated_by_principal_ref"),
            ("  updated at", "updated_at"),
        ],
    )?;
    Ok(())
}

async fn tenant_access_grants_list(
    client: &Client,
    endpoint: &str,
    args: TenantAccessGrantsListArgs,
    json: bool,
) -> Result<()> {
    let path = "/v1/admin/tenant-access-grants";
    let mut owned: Vec<(&str, String)> = Vec::new();
    if let Some(limit) = args.limit {
        owned.push(("limit", limit.to_string()));
    }
    if let Some(status) = args.status {
        owned.push(("status", status.as_str().to_string()));
    }
    if let Some(role) = args.role {
        owned.push(("role", role.server_value().to_string()));
    }
    if let Some(principal_ref) = args.principal_ref {
        let trimmed = principal_ref.trim();
        if !trimmed.is_empty() {
            owned.push(("principal_ref", trimmed.to_string()));
        }
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
            "Central tenant access grants",
            &value,
            &[
                "grant_id",
                "principal_ref",
                "role",
                "status",
                "allowed_consent_scopes",
                "allowed_uses",
                "issuer",
                "subject",
                "expires_at",
                "revoked_at",
            ],
        )?;
    }
    Ok(())
}

async fn tenant_access_grant_create(
    client: &Client,
    endpoint: &str,
    args: TenantAccessGrantCreateArgs,
    json: bool,
) -> Result<()> {
    let principal_ref = args.principal_ref.trim();
    if principal_ref.is_empty() {
        anyhow::bail!("--principal-ref must not be empty");
    }
    let reason = args.reason.trim();
    if reason.is_empty() {
        anyhow::bail!("--reason must not be empty");
    }
    if let Some(at) = args.issued_at.as_deref() {
        require_rfc3339("--issued-at", at)?;
    }
    if let Some(at) = args.expires_at.as_deref() {
        require_rfc3339("--expires-at", at)?;
    }
    let mut body = serde_json::json!({
        "principal_ref": principal_ref,
        "role": args.role.server_value(),
        "allowed_consent_scopes": args
            .allowed_consent_scopes
            .iter()
            .map(|s| s.server_value())
            .collect::<Vec<_>>(),
        "allowed_uses": args
            .allowed_uses
            .iter()
            .map(|u| u.server_value())
            .collect::<Vec<_>>(),
        "reason": reason,
    });
    if let Some(grant_id) = args.grant_id {
        body["grant_id"] = Value::String(grant_id.to_string());
    }
    if let Some(s) = trimmed(args.issuer.as_deref()) {
        body["issuer"] = Value::String(s);
    }
    if let Some(s) = trimmed(args.audience.as_deref()) {
        body["audience"] = Value::String(s);
    }
    if let Some(s) = trimmed(args.subject.as_deref()) {
        body["subject"] = Value::String(s);
    }
    if let Some(s) = trimmed(args.issued_at.as_deref()) {
        body["issued_at"] = Value::String(s);
    }
    if let Some(s) = trimmed(args.expires_at.as_deref()) {
        body["expires_at"] = Value::String(s);
    }
    let metadata = parse_metadata(&args.metadata)?;
    if !metadata.is_empty() {
        body["metadata"] = serde_json::to_value(metadata)
            .map_err(|e| anyhow::anyhow!("failed to encode grant metadata: {e}"))?;
    }
    let path = "/v1/admin/tenant-access-grants";
    let value: Value = client
        .call_json::<Value, Value>(Method::POST, path, &[], Some(&body))
        .await?;
    if json {
        emit_json(endpoint, "POST", path, &value)?;
    } else {
        let mut out = stdout();
        writeln!(out, "Trace Commons tenant access grant created.")?;
        render_grant_fields(&mut out, &value)?;
    }
    Ok(())
}

async fn tenant_access_grant_revoke(
    client: &Client,
    endpoint: &str,
    args: TenantAccessGrantRevokeArgs,
    json: bool,
) -> Result<()> {
    let reason = args.reason.trim();
    if reason.is_empty() {
        anyhow::bail!("--reason must not be empty");
    }
    let path = format!("/v1/admin/tenant-access-grants/{}/revoke", args.grant_id);
    let body = serde_json::json!({ "reason": reason });
    let value: Value = client
        .call_json::<Value, Value>(Method::POST, &path, &[], Some(&body))
        .await?;
    if json {
        emit_json(endpoint, "POST", &path, &value)?;
    } else {
        let mut out = stdout();
        writeln!(out, "Trace Commons tenant access grant revoked.")?;
        render_grant_fields(&mut out, &value)?;
    }
    Ok(())
}

async fn device_keys(
    client: &Client,
    endpoint: &str,
    args: DeviceKeysArgs,
    json: bool,
) -> Result<()> {
    match args.command {
        DeviceKeysSubcommand::List(args) => device_keys_list(client, endpoint, args, json).await,
        DeviceKeysSubcommand::Revoke(args) => device_key_revoke(client, endpoint, args, json).await,
    }
}

async fn device_keys_list(
    client: &Client,
    endpoint: &str,
    args: DeviceKeysListArgs,
    json: bool,
) -> Result<()> {
    let path = "/v1/admin/device-keys";
    let mut owned: Vec<(&str, String)> = Vec::new();
    if let Some(tenant) = args.tenant {
        let trimmed = tenant.trim();
        if !trimmed.is_empty() {
            owned.push(("tenant", trimmed.to_string()));
        }
    }
    if let Some(limit) = args.limit {
        owned.push(("limit", limit.to_string()));
    }
    if args.include_revoked {
        owned.push(("include_revoked", "true".to_string()));
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
            "Trace Commons device keys",
            &value,
            &[
                "device_key_id",
                "tenant_id",
                "invite_subject_hash",
                "created_at",
                "revoked_at",
            ],
        )?;
    }
    Ok(())
}

async fn device_key_revoke(
    client: &Client,
    endpoint: &str,
    args: DeviceKeyRevokeArgs,
    json: bool,
) -> Result<()> {
    let device_key_id = args.device_key_id.trim();
    if device_key_id.is_empty() {
        anyhow::bail!("device_key_id must not be empty");
    }
    let path = format!("/v1/admin/device-keys/{device_key_id}/revoke");
    let value: Value = client
        .call_json::<(), Value>(Method::POST, &path, &[], None)
        .await?;
    if json {
        emit_json(endpoint, "POST", &path, &value)?;
    } else {
        let mut out = stdout();
        writeln!(out, "Trace Commons device key revoked.")?;
        render_kv_fields(
            &mut out,
            &value,
            &[
                ("  tenant", "tenant_id"),
                ("  device key id", "device_key_id"),
                ("  invite subject hash", "invite_subject_hash"),
                ("  created at", "created_at"),
                ("  revoked at", "revoked_at"),
            ],
        )?;
    }
    Ok(())
}

fn render_grant_fields<W: Write>(out: &mut W, value: &Value) -> std::io::Result<()> {
    render_kv_fields(
        out,
        value,
        &[
            ("  tenant", "tenant_id"),
            ("  grant id", "grant_id"),
            ("  principal", "principal_ref"),
            ("  role", "role"),
            ("  status", "status"),
            ("  allowed consent scopes", "allowed_consent_scopes"),
            ("  allowed uses", "allowed_uses"),
            ("  issuer", "issuer"),
            ("  audience", "audience"),
            ("  subject", "subject"),
            ("  expires at", "expires_at"),
            ("  revoked at", "revoked_at"),
        ],
    )
}

#[allow(clippy::too_many_arguments)]
async fn ranker_training_export(
    client: &Client,
    endpoint: &str,
    args: RankerTrainingArgs,
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
        owned.push(("consent_scope", scope.query_value().to_string()));
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
        let items = value.get(item_field).cloned().unwrap_or(Value::Null);
        let envelope = serde_json::json!({ "items": items });
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

async fn audit_events(
    client: &Client,
    endpoint: &str,
    args: AuditEventsArgs,
    json: bool,
) -> Result<()> {
    let path = "/v1/audit/events";
    let mut owned: Vec<(&str, String)> = Vec::new();
    if let Some(limit) = args.limit {
        owned.push(("limit", limit.to_string()));
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
            "Central audit events",
            &value,
            &["created_at", "kind", "submission_id", "status", "reason"],
        )?;
    }
    Ok(())
}

async fn list_traces(
    client: &Client,
    endpoint: &str,
    args: ListTracesArgs,
    json: bool,
) -> Result<()> {
    let path = "/v1/traces";
    let mut owned: Vec<(&str, String)> = Vec::new();
    if let Some(purpose) = args.purpose {
        let trimmed = purpose.trim();
        if trimmed.is_empty() {
            anyhow::bail!("--purpose must be non-empty");
        }
        owned.push(("purpose", trimmed.to_string()));
    }
    if let Some(scope) = args.consent_scope {
        owned.push(("consent_scope", scope.query_value().to_string()));
    }
    if let Some(status) = args.status {
        owned.push(("status", status.as_str().to_string()));
    }
    if let Some(limit) = args.limit {
        owned.push(("limit", limit.to_string()));
    }
    if let Some(tag) = args.coverage_tag {
        owned.push(("coverage_tag", tag));
    }
    if let Some(tool) = args.tool {
        owned.push(("tool", tool));
    }
    if let Some(risk) = args.privacy_risk {
        owned.push(("privacy_risk", risk.as_str().to_string()));
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
            "Central traces",
            &value,
            &[
                "submission_id",
                "status",
                "privacy_risk",
                "submission_score",
                "credit_points_pending",
                "review_assigned_to_principal_ref",
                "review_lease_expires_at",
                "review_due_at",
                "received_at",
            ],
        )?;
    }
    Ok(())
}

fn tenant_principal_ref(args: &TenantPrincipalRefArgs, json: bool) -> Result<()> {
    let token_env = trimmed(args.token_env.as_deref());
    let signed_tenant_id = trimmed(args.signed_tenant_id.as_deref());
    let signed_actor_ref = trimmed(args.signed_actor_ref.as_deref());
    let device_tenant_id = trimmed(args.device_tenant_id.as_deref());
    let device_key_id = trimmed(args.device_key_id.as_deref());
    let has_token = token_env.is_some();
    let has_signed = signed_tenant_id.is_some() || signed_actor_ref.is_some();
    let has_device = device_tenant_id.is_some() || device_key_id.is_some();
    let mode_count = [has_token, has_signed, has_device]
        .into_iter()
        .filter(|present| *present)
        .count();
    if mode_count != 1 {
        anyhow::bail!(
            "provide exactly one principal mode: --token-env, both signed-claim flags, or both device-key flags"
        );
    }
    let (mode, principal_ref) = if let Some(token_env) = token_env {
        let token = std::env::var(&token_env).map_err(|_| {
            anyhow::anyhow!(
                "{} is not set; refusing to derive a principal_ref from a missing token",
                token_env
            )
        })?;
        if token.trim().is_empty() {
            anyhow::bail!("{token_env} is empty; refusing to derive a principal_ref");
        }
        ("static_token", principal_storage_ref(&token))
    } else if has_signed {
        let tenant_id = signed_tenant_id.ok_or_else(|| {
            anyhow::anyhow!("--signed-tenant-id is required with --signed-actor-ref")
        })?;
        let actor_ref = signed_actor_ref.ok_or_else(|| {
            anyhow::anyhow!("--signed-actor-ref is required with --signed-tenant-id")
        })?;
        (
            "signed_claim",
            signed_claim_principal_ref(&tenant_id, &actor_ref),
        )
    } else {
        let tenant_id = device_tenant_id.ok_or_else(|| {
            anyhow::anyhow!("--device-tenant-id is required with --device-key-id")
        })?;
        let device_key_id = device_key_id.ok_or_else(|| {
            anyhow::anyhow!("--device-key-id is required with --device-tenant-id")
        })?;
        (
            "device_key",
            device_key_principal_ref(&tenant_id, &device_key_id),
        )
    };

    if json {
        let value = serde_json::json!({
            "mode": mode,
            "principal_ref": principal_ref,
        });
        oc_format::emit_json(
            &mut stdout(),
            "LOCAL",
            "trace-commons-tenant/principal-ref",
            &value,
        )?;
    } else {
        println!("{principal_ref}");
    }
    Ok(())
}

fn principal_storage_ref(value: &str) -> String {
    format!("principal_{}", sha256_prefixed(value))
}

fn signed_claim_principal_ref(tenant_id: &str, actor_ref: &str) -> String {
    let composed = format!("signed:{}:{}", tenant_id.trim(), actor_ref.trim());
    principal_storage_ref(&composed)
}

fn device_key_principal_ref(tenant_id: &str, device_key_id: &str) -> String {
    let composed = format!("device:{}:{}", tenant_id.trim(), device_key_id.trim());
    principal_storage_ref(&composed)
}

fn sha256_prefixed(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    format!("sha256:{}", hex::encode(digest))
}

async fn privacy_filter_canary(args: &PrivacyFilterCanaryArgs, json: bool) -> Result<()> {
    let adapter = privacy_filter::adapter_from_env().ok_or_else(|| {
        anyhow::anyhow!(
            "TRACE_COMMONS_PRIVACY_FILTER_COMMAND is not set; no local privacy filter sidecar is configured"
        )
    })?;
    let timeout = Duration::from_secs(args.timeout_seconds.max(1));
    let redaction = tokio::time::timeout(timeout, adapter.redact_text(&args.text))
        .await
        .map_err(|_| anyhow::anyhow!("privacy filter canary timed out after {:?}", timeout))?
        .map_err(|e| anyhow::anyhow!("privacy filter canary failed: {e}"))?;
    let Some(redaction) = redaction else {
        anyhow::bail!("privacy filter sidecar returned no redaction for canary text");
    };
    let leaked_tokens = privacy_filter::canary_leaked_tokens(&args.text, &redaction.redacted_text);
    let passed = leaked_tokens.is_empty();
    let input_hash = privacy_filter::canonical_hash(&args.text);
    let redacted_hash = privacy_filter::canonical_hash(&redaction.redacted_text);

    if json {
        let value = serde_json::json!({
            "canary_version": "trace-commons-privacy-filter-canary-v1",
            "passed": passed,
            "input_hash": input_hash,
            "redacted_output_hash": redacted_hash,
            "summary": redaction.summary,
            "report": redaction.report,
            "leaked_token_hashes": leaked_tokens,
        });
        oc_format::emit_json(
            &mut stdout(),
            "LOCAL",
            "trace-commons-tenant/privacy-filter-canary",
            &value,
        )?;
    } else {
        let mut out = stdout();
        writeln!(
            out,
            "privacy-filter canary: {}",
            if passed { "PASS" } else { "FAIL" }
        )?;
        writeln!(out, "  input_hash:           {input_hash}")?;
        writeln!(out, "  redacted_output_hash: {redacted_hash}")?;
        writeln!(
            out,
            "  span_count:           {}",
            redaction.summary.span_count
        )?;
        writeln!(
            out,
            "  schema_version:       {}",
            redaction.summary.schema_version
        )?;
        writeln!(
            out,
            "  decoded_mismatch:     {}",
            redaction.summary.decoded_mismatch
        )?;
        if !redaction.summary.by_label.is_empty() {
            writeln!(out, "  by_label:")?;
            for (label, count) in &redaction.summary.by_label {
                writeln!(out, "    {label}: {count}")?;
            }
        }
        if redaction.report.blocked_secret_detected {
            writeln!(out, "  blocked_secret_detected: true")?;
        }
        for warning in &redaction.report.warnings {
            writeln!(out, "  warning: {warning}")?;
        }
        if !leaked_tokens.is_empty() {
            writeln!(out, "  leaked_token_hashes:")?;
            for h in &leaked_tokens {
                writeln!(out, "    {h}")?;
            }
        }
    }

    if !passed {
        anyhow::bail!("privacy filter canary failed: sidecar output retained canary token(s)");
    }
    Ok(())
}

// --- helpers ---

fn borrow_query<'a>(owned: &'a [(&'a str, String)]) -> Vec<(&'a str, &'a str)> {
    owned.iter().map(|(k, v)| (*k, v.as_str())).collect()
}

fn trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn require_rfc3339(label: &str, value: &str) -> Result<()> {
    let trimmed_v = value.trim();
    if trimmed_v.is_empty() {
        anyhow::bail!("{label} must not be empty");
    }
    chrono::DateTime::parse_from_rfc3339(trimmed_v)
        .map_err(|e| anyhow::anyhow!("{label} must be RFC3339: {e}"))?;
    Ok(())
}

fn parse_metadata(values: &[String]) -> Result<std::collections::BTreeMap<String, String>> {
    let mut metadata = std::collections::BTreeMap::new();
    for value in values {
        let Some((key, raw)) = value.split_once('=') else {
            anyhow::bail!("--metadata values must use KEY=VALUE");
        };
        let key = key.trim();
        if key.is_empty() {
            anyhow::bail!("--metadata keys must not be empty");
        }
        metadata.insert(key.to_string(), raw.trim().to_string());
    }
    Ok(metadata)
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
    use wiremock::matchers::{body_json, header, method, path as wm_path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn unique_bearer_env() -> String {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        format!("TC_TENANT_TEST_BEARER_{n}")
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
            ["trace-commons-tenant", "--endpoint", "https://api.example"]
                .iter()
                .chain(args.iter())
                .copied(),
        )
        .expect("parse cli")
    }

    fn parse_no_endpoint(args: &[&str]) -> Cli {
        Cli::try_parse_from(["trace-commons-tenant"].iter().chain(args.iter()).copied())
            .expect("parse cli")
    }

    #[test]
    fn cli_parses_tenant_policy_get() {
        let cli = parse(&["tenant-policy-get"]);
        assert!(matches!(cli.command, TenantSubcommand::TenantPolicyGet));
        assert_eq!(cli.bearer_token_env, DEFAULT_BEARER_ENV);
    }

    #[test]
    fn cli_parses_tenant_policy_set() {
        let cli = parse(&[
            "tenant-policy-set",
            "--policy-version",
            "v1",
            "--allowed-consent-scopes",
            "benchmark-only,model-training",
            "--allowed-uses",
            "benchmark-generation,model-training",
        ]);
        match cli.command {
            TenantSubcommand::TenantPolicySet(args) => {
                assert_eq!(args.policy_version, "v1");
                assert_eq!(args.allowed_consent_scopes.len(), 2);
                assert_eq!(args.allowed_uses.len(), 2);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_tenant_access_grants_list() {
        let cli = parse(&[
            "tenant-access-grants-list",
            "--limit",
            "10",
            "--status",
            "active",
            "--role",
            "reviewer",
            "--principal-ref",
            "principal_xyz",
        ]);
        match cli.command {
            TenantSubcommand::TenantAccessGrantsList(args) => {
                assert_eq!(args.limit, Some(10));
                assert_eq!(args.status, Some(TenantAccessGrantStatus::Active));
                assert_eq!(args.role, Some(TenantAccessGrantRole::Reviewer));
                assert_eq!(args.principal_ref.as_deref(), Some("principal_xyz"));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_tenant_principal_ref_with_token_env() {
        // No --endpoint required for this subcommand.
        let cli = parse_no_endpoint(&["tenant-principal-ref", "--token-env", "SOME_TOKEN_VAR"]);
        match cli.command {
            TenantSubcommand::TenantPrincipalRef(args) => {
                assert_eq!(args.token_env.as_deref(), Some("SOME_TOKEN_VAR"));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_tenant_principal_ref_with_device_key() {
        let parsed = Cli::try_parse_from([
            "trace-commons-tenant",
            "tenant-principal-ref",
            "--device-tenant-id",
            "tenant-zaki-pilot",
            "--device-key-id",
            "sha256:37315969b52410607889e9cc858382b55cb0f2fc35c7051c7a7ab7ce9dcd7b38",
        ]);
        assert!(parsed.is_ok(), "{parsed:?}");
    }

    #[test]
    fn cli_parses_tenant_access_grant_create() {
        let cli = parse(&[
            "tenant-access-grant-create",
            "--principal-ref",
            "principal_xyz",
            "--role",
            "reviewer",
            "--allowed-consent-scopes",
            "benchmark-only",
            "--allowed-uses",
            "benchmark-generation",
            "--reason",
            "rotate",
            "--metadata",
            "issued_by=ops",
        ]);
        match cli.command {
            TenantSubcommand::TenantAccessGrantCreate(args) => {
                assert_eq!(args.role, TenantAccessGrantRole::Reviewer);
                assert_eq!(args.metadata.len(), 1);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_public_attribution_tenant_access_grant_create() {
        let parsed = Cli::try_parse_from([
            "trace-commons-tenant",
            "--endpoint",
            "https://api.example",
            "tenant-access-grant-create",
            "--principal-ref",
            "principal_xyz",
            "--role",
            "contributor",
            "--allowed-consent-scopes",
            "public-attribution",
            "--allowed-uses",
            "debugging,aggregate-analytics",
            "--reason",
            "pilot profile setup",
        ]);
        assert!(parsed.is_ok(), "{parsed:?}");
    }

    #[test]
    fn cli_rejects_tenant_access_grant_create_missing_reason() {
        let err = Cli::try_parse_from([
            "trace-commons-tenant",
            "--endpoint",
            "https://api.example",
            "tenant-access-grant-create",
            "--principal-ref",
            "p",
            "--role",
            "reviewer",
        ])
        .unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn cli_parses_tenant_access_grant_revoke() {
        let id = Uuid::new_v4();
        let cli = parse(&[
            "tenant-access-grant-revoke",
            "--grant-id",
            &id.to_string(),
            "--reason",
            "key compromise",
        ]);
        match cli.command {
            TenantSubcommand::TenantAccessGrantRevoke(args) => {
                assert_eq!(args.grant_id, id);
                assert_eq!(args.reason, "key compromise");
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_ranker_training_candidates() {
        let cli = parse(&[
            "ranker-training-candidates",
            "--limit",
            "5",
            "--status",
            "accepted",
            "--privacy-risk",
            "low",
        ]);
        match cli.command {
            TenantSubcommand::RankerTrainingCandidates(args) => {
                assert_eq!(args.limit, Some(5));
                assert_eq!(args.status, Some(CorpusStatus::Accepted));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_ranker_training_pairs() {
        let cli = parse(&["ranker-training-pairs", "--limit", "5"]);
        assert!(matches!(
            cli.command,
            TenantSubcommand::RankerTrainingPairs(_)
        ));
    }

    #[test]
    fn cli_parses_audit_events() {
        let cli = parse(&["audit-events", "--limit", "50"]);
        match cli.command {
            TenantSubcommand::AuditEvents(args) => assert_eq!(args.limit, Some(50)),
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_list_traces() {
        let cli = parse(&[
            "list-traces",
            "--purpose",
            "review",
            "--consent-scope",
            "debugging-evaluation",
            "--status",
            "accepted",
            "--limit",
            "20",
            "--coverage-tag",
            "ops",
            "--tool",
            "shell",
            "--privacy-risk",
            "low",
        ]);
        match cli.command {
            TenantSubcommand::ListTraces(args) => {
                assert_eq!(args.consent_scope, Some(ConsentScope::DebuggingEvaluation));
                assert_eq!(args.privacy_risk, Some(PrivacyRisk::Low));
                assert_eq!(args.coverage_tag.as_deref(), Some("ops"));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_privacy_filter_canary_defaults() {
        let cli = parse_no_endpoint(&["privacy-filter-canary"]);
        match cli.command {
            TenantSubcommand::PrivacyFilterCanary(args) => {
                assert!(args.text.contains("Canary"));
                assert_eq!(args.timeout_seconds, 10);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[tokio::test]
    async fn privacy_filter_canary_errors_when_unconfigured() {
        // Capture and clear the env vars the adapter reads so this test
        // is hermetic regardless of operator-level configuration.
        let keys = [
            "TRACE_COMMONS_PRIVACY_FILTER_COMMAND",
            "IRONCLAW_TRACE_PRIVACY_FILTER_COMMAND",
        ];
        let prior: Vec<_> = keys.iter().map(|k| (*k, std::env::var(k).ok())).collect();
        for k in &keys {
            unsafe { std::env::remove_var(k) };
        }
        let args = PrivacyFilterCanaryArgs {
            text: "trace-canary.person@example.invalid".to_string(),
            timeout_seconds: 2,
        };
        let err = privacy_filter_canary(&args, true).await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("TRACE_COMMONS_PRIVACY_FILTER_COMMAND"),
            "expected env-var error, got: {msg}"
        );
        // Confirm we no longer emit the legacy stub message.
        assert!(!msg.contains("not yet ported"), "stub message must be gone");
        for (k, v) in prior {
            match v {
                Some(value) => unsafe { std::env::set_var(k, value) },
                None => unsafe { std::env::remove_var(k) },
            }
        }
    }

    #[test]
    fn cli_rejects_unknown_subcommand() {
        let err = Cli::try_parse_from([
            "trace-commons-tenant",
            "--endpoint",
            "https://api.example",
            "bogus",
        ])
        .unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn tenant_principal_ref_token_env_mode_derives_stable_hash() {
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret-token-abc");
        let args = TenantPrincipalRefArgs {
            token_env: Some(env.clone()),
            signed_tenant_id: None,
            signed_actor_ref: None,
            device_tenant_id: None,
            device_key_id: None,
        };
        tenant_principal_ref(&args, true).expect("derive works");
        // Re-derive directly to confirm format.
        let p = principal_storage_ref("secret-token-abc");
        assert!(p.starts_with("principal_sha256:"));
    }

    #[test]
    fn tenant_principal_ref_signed_claim_mode_derives() {
        let args = TenantPrincipalRefArgs {
            token_env: None,
            signed_tenant_id: Some("tenant-1".into()),
            signed_actor_ref: Some("actor-xyz".into()),
            device_tenant_id: None,
            device_key_id: None,
        };
        tenant_principal_ref(&args, false).expect("derive works");
        let p = signed_claim_principal_ref("tenant-1", "actor-xyz");
        assert!(p.starts_with("principal_sha256:"));
    }

    #[test]
    fn tenant_principal_ref_signed_claim_mode_matches_issuer_hash_input() {
        assert_eq!(
            signed_claim_principal_ref("tenant-1", "actor-xyz"),
            principal_storage_ref("signed:tenant-1:actor-xyz")
        );
    }

    #[test]
    fn tenant_principal_ref_device_key_mode_matches_issuer_hash_input() {
        assert_eq!(
            device_key_principal_ref("tenant-1", "sha256:device-key"),
            principal_storage_ref("device:tenant-1:sha256:device-key")
        );
    }

    #[test]
    fn tenant_principal_ref_rejects_both_modes() {
        let args = TenantPrincipalRefArgs {
            token_env: Some("X".into()),
            signed_tenant_id: Some("t".into()),
            signed_actor_ref: Some("a".into()),
            device_tenant_id: None,
            device_key_id: None,
        };
        let err = tenant_principal_ref(&args, false).unwrap_err();
        assert!(err.to_string().contains("exactly one"));
    }

    #[test]
    fn tenant_principal_ref_rejects_neither_mode() {
        let args = TenantPrincipalRefArgs {
            token_env: None,
            signed_tenant_id: None,
            signed_actor_ref: None,
            device_tenant_id: None,
            device_key_id: None,
        };
        let err = tenant_principal_ref(&args, false).unwrap_err();
        assert!(err.to_string().contains("exactly one"));
    }

    #[test]
    fn tenant_principal_ref_rejects_missing_actor_ref() {
        let args = TenantPrincipalRefArgs {
            token_env: None,
            signed_tenant_id: Some("t".into()),
            signed_actor_ref: None,
            device_tenant_id: None,
            device_key_id: None,
        };
        let err = tenant_principal_ref(&args, false).unwrap_err();
        assert!(err.to_string().contains("signed-actor-ref"));
    }

    #[test]
    fn parse_metadata_round_trips_pairs() {
        let m = parse_metadata(&["k=v".to_string(), "a=b".to_string()]).unwrap();
        assert_eq!(m.get("k").map(String::as_str), Some("v"));
        assert_eq!(m.get("a").map(String::as_str), Some("b"));
    }

    #[test]
    fn parse_metadata_rejects_missing_equals() {
        let err = parse_metadata(&["nokey".to_string()]).unwrap_err();
        assert!(err.to_string().contains("KEY=VALUE"));
    }

    #[test]
    fn require_rfc3339_accepts_valid_and_rejects_garbage() {
        require_rfc3339("--issued-at", "2026-05-19T00:00:00Z").unwrap();
        let err = require_rfc3339("--issued-at", "yesterday").unwrap_err();
        assert!(err.to_string().contains("RFC3339"));
    }

    #[tokio::test]
    async fn tenant_policy_get_happy_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(wm_path("/v1/admin/tenant-policy"))
            .and(header("authorization", "Bearer tenant-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "tenant_id": "t-1",
                "policy_version": "v1",
                "allowed_consent_scopes": ["benchmark_only"],
                "allowed_uses": ["benchmark_generation"],
            })))
            .expect(1)
            .mount(&server)
            .await;
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "tenant-secret");
        let client = Client::builder(server.uri(), env).build().unwrap();
        tenant_policy_get(&client, &server.uri(), true)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn tenant_policy_set_puts_body() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(wm_path("/v1/admin/tenant-policy"))
            .and(body_json(serde_json::json!({
                "policy_version": "v2",
                "allowed_consent_scopes": ["benchmark_only", "model_training"],
                "allowed_uses": ["benchmark_generation"],
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "policy_version": "v2",
            })))
            .expect(1)
            .mount(&server)
            .await;
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder(server.uri(), env).build().unwrap();
        tenant_policy_set(
            &client,
            &server.uri(),
            TenantPolicySetArgs {
                policy_version: "v2".into(),
                allowed_consent_scopes: vec![
                    ConsentScope::BenchmarkOnly,
                    ConsentScope::ModelTraining,
                ],
                allowed_uses: vec![AllowedUse::BenchmarkGeneration],
            },
            true,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn tenant_policy_set_rejects_empty_version() {
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder("https://api.example", env).build().unwrap();
        let err = tenant_policy_set(
            &client,
            "https://api.example",
            TenantPolicySetArgs {
                policy_version: "  ".into(),
                allowed_consent_scopes: vec![],
                allowed_uses: vec![],
            },
            true,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("policy-version"));
    }

    #[tokio::test]
    async fn tenant_access_grants_list_passes_query() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(wm_path("/v1/admin/tenant-access-grants"))
            .and(query_param("limit", "10"))
            .and(query_param("status", "active"))
            .and(query_param("role", "reviewer"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "items": [] })),
            )
            .expect(1)
            .mount(&server)
            .await;
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder(server.uri(), env).build().unwrap();
        tenant_access_grants_list(
            &client,
            &server.uri(),
            TenantAccessGrantsListArgs {
                limit: Some(10),
                status: Some(TenantAccessGrantStatus::Active),
                role: Some(TenantAccessGrantRole::Reviewer),
                principal_ref: None,
            },
            true,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn tenant_access_grant_create_posts_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(wm_path("/v1/admin/tenant-access-grants"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "grant_id": "g-1" })),
            )
            .expect(1)
            .mount(&server)
            .await;
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder(server.uri(), env).build().unwrap();
        tenant_access_grant_create(
            &client,
            &server.uri(),
            TenantAccessGrantCreateArgs {
                principal_ref: "principal_xyz".into(),
                role: TenantAccessGrantRole::Reviewer,
                grant_id: None,
                allowed_consent_scopes: vec![],
                allowed_uses: vec![],
                issuer: None,
                audience: None,
                subject: None,
                issued_at: None,
                expires_at: None,
                reason: "rotate".into(),
                metadata: vec![],
            },
            true,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn tenant_access_grant_create_rejects_bad_rfc3339() {
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder("https://api.example", env).build().unwrap();
        let err = tenant_access_grant_create(
            &client,
            "https://api.example",
            TenantAccessGrantCreateArgs {
                principal_ref: "p".into(),
                role: TenantAccessGrantRole::Reviewer,
                grant_id: None,
                allowed_consent_scopes: vec![],
                allowed_uses: vec![],
                issuer: None,
                audience: None,
                subject: None,
                issued_at: Some("yesterday".into()),
                expires_at: None,
                reason: "rotate".into(),
                metadata: vec![],
            },
            true,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("RFC3339"));
    }

    #[tokio::test]
    async fn tenant_access_grant_revoke_posts_body() {
        let server = MockServer::start().await;
        let id = Uuid::new_v4();
        Mock::given(method("POST"))
            .and(wm_path(format!(
                "/v1/admin/tenant-access-grants/{id}/revoke"
            )))
            .and(body_json(serde_json::json!({ "reason": "key compromise" })))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "status": "revoked" })),
            )
            .expect(1)
            .mount(&server)
            .await;
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder(server.uri(), env).build().unwrap();
        tenant_access_grant_revoke(
            &client,
            &server.uri(),
            TenantAccessGrantRevokeArgs {
                grant_id: id,
                reason: "key compromise".into(),
            },
            true,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn audit_events_happy_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(wm_path("/v1/audit/events"))
            .and(query_param("limit", "25"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "items": [] })),
            )
            .expect(1)
            .mount(&server)
            .await;
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder(server.uri(), env).build().unwrap();
        audit_events(
            &client,
            &server.uri(),
            AuditEventsArgs { limit: Some(25) },
            true,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn list_traces_passes_query() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(wm_path("/v1/traces"))
            .and(query_param("status", "accepted"))
            .and(query_param("limit", "5"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "items": [] })),
            )
            .expect(1)
            .mount(&server)
            .await;
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder(server.uri(), env).build().unwrap();
        list_traces(
            &client,
            &server.uri(),
            ListTracesArgs {
                purpose: None,
                consent_scope: None,
                status: Some(CorpusStatus::Accepted),
                limit: Some(5),
                coverage_tag: None,
                tool: None,
                privacy_risk: None,
            },
            true,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn ranker_training_candidates_renders_envelope() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(wm_path("/v1/ranker/training-candidates"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "candidates": [
                    {"submission_id": "s-1", "ranker_score": 0.9}
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder(server.uri(), env).build().unwrap();
        ranker_training_export(
            &client,
            &server.uri(),
            RankerTrainingArgs {
                purpose: None,
                consent_scope: None,
                status: None,
                privacy_risk: None,
                limit: None,
                output: None,
            },
            "/v1/ranker/training-candidates",
            "ranker training candidates",
            "candidates",
            true,
        )
        .await
        .unwrap();
    }
}
