// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

#![cfg_attr(not(test), forbid(unsafe_code))]

//! `trace-commons-review` — reviewer-audience operator CLI.
//!
//! Twelve subcommands matching the reviewer-bearer endpoints on
//! trace-commons-server. All requests are issued through the foundation
//! `trace_commons_operator_client::Client`, which enforces the host
//! allowlist and never logs bearer tokens or query strings.

#[path = "operator_common/mod.rs"]
mod operator_common;

use std::io::{Write, stdout};

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use reqwest::Method;
use serde_json::Value;
use trace_commons_operator_client::Client;
use uuid::Uuid;

use operator_common::{
    PrivacyRisk, borrow_query, build_client, emit_json, render_items, render_kv_fields,
};

const DEFAULT_BEARER_ENV: &str = "TRACE_COMMONS_REVIEWER_BEARER";

#[derive(Debug, Parser)]
#[command(
    name = "trace-commons-review",
    about = "Reviewer-audience CLI for trace-commons-server.",
    // The semver does not move when a deploy does, so --version carries the
    // commit the binary was built from as well.
    version = trace_commons_build_info::version_line(env!("CARGO_PKG_VERSION"))
)]
pub(crate) struct Cli {
    /// Hosted endpoint. Reads `TRACE_COMMONS_ENDPOINT` when unset on the CLI.
    #[arg(long, env = "TRACE_COMMONS_ENDPOINT")]
    pub endpoint: String,

    /// Name of the env var holding the reviewer bearer token.
    #[arg(long, default_value = DEFAULT_BEARER_ENV)]
    pub bearer_token_env: String,

    /// Defense-in-depth host allowlist (CSV). Empty means permissive.
    #[arg(long, env = "TRACE_COMMONS_ALLOWED_HOSTS")]
    pub allowed_hosts: Option<String>,

    /// Emit the server response as JSON (with a sanitized request envelope).
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: ReviewSubcommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ReviewSubcommand {
    /// List quarantined submissions.
    QuarantineList(QuarantineListArgs),
    /// List the active-learning review queue (404-tolerant).
    ActiveLearningReviewQueue(ActiveLearningArgs),
    /// Record an approve/reject decision for a submission.
    ReviewDecision(ReviewDecisionArgs),
    /// Claim the review lease for a specific submission.
    ReviewLeaseClaim(LeaseClaimArgs),
    /// Claim the next available review lease.
    ReviewLeaseClaimNext(LeaseClaimNextArgs),
    /// Claim up to `limit` review leases atomically.
    ReviewLeaseClaimBatch(LeaseClaimBatchArgs),
    /// Release the review lease for a submission.
    ReviewLeaseRelease(LeaseReleaseArgs),
    /// Append a delayed credit event for a submission.
    AppendCreditEvent(AppendCreditEventArgs),
    /// List pipeline runs waiting for review.
    PipelineQuarantine(PipelineQuarantineArgs),
    /// Claim a pipeline run for review.
    PipelineClaim(PipelineClaimArgs),
    /// Save a freshly rechecked version of a held pipeline run.
    PipelineRecheck(PipelineRecheckArgs),
    /// Record an assessment of a pipeline run or exact rechecked version.
    PipelineAssessment(PipelineAssessmentArgs),
}

#[derive(Debug, clap::Args)]
pub(crate) struct QuarantineListArgs {
    #[arg(long, value_enum)]
    pub lease_filter: Option<ReviewLeaseFilter>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct ActiveLearningArgs {
    #[arg(long)]
    pub limit: Option<usize>,
    #[arg(long, value_enum)]
    pub privacy_risk: Option<PrivacyRisk>,
    #[arg(long, value_enum)]
    pub lease_filter: Option<ReviewLeaseFilter>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct ReviewDecisionArgs {
    #[arg(long)]
    pub submission_id: Uuid,
    #[arg(long, value_enum)]
    pub decision: ReviewDecision,
    #[arg(long)]
    pub reason: Option<String>,
    #[arg(long)]
    pub credit_points_pending: Option<f32>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct LeaseClaimArgs {
    #[arg(long)]
    pub submission_id: Uuid,
    #[arg(long)]
    pub lease_ttl_seconds: Option<i64>,
    #[arg(long)]
    pub review_due_at: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct LeaseClaimNextArgs {
    #[arg(long)]
    pub lease_ttl_seconds: Option<i64>,
    #[arg(long)]
    pub review_due_at: Option<String>,
    #[arg(long, value_enum)]
    pub privacy_risk: Option<PrivacyRisk>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct LeaseClaimBatchArgs {
    #[arg(long)]
    pub limit: Option<usize>,
    #[arg(long)]
    pub lease_ttl_seconds: Option<i64>,
    #[arg(long)]
    pub review_due_at: Option<String>,
    #[arg(long, value_enum)]
    pub privacy_risk: Option<PrivacyRisk>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct LeaseReleaseArgs {
    #[arg(long)]
    pub submission_id: Uuid,
}

#[derive(Debug, clap::Args)]
pub(crate) struct AppendCreditEventArgs {
    #[arg(long)]
    pub submission_id: Uuid,
    #[arg(long, value_enum)]
    pub event_type: CreditEventType,
    #[arg(long)]
    pub credit_points_delta: f32,
    #[arg(long)]
    pub reason: String,
    #[arg(long)]
    pub external_ref: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct PipelineQuarantineArgs {
    #[arg(long)]
    pub limit: Option<usize>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct PipelineClaimArgs {
    #[arg(long)]
    pub run_id: Uuid,
}

#[derive(Debug, clap::Args)]
pub(crate) struct PipelineRecheckArgs {
    #[arg(long)]
    pub run_id: Uuid,
    #[arg(long)]
    pub lease_token: Uuid,
    /// Stable request ID to reuse when retrying this recheck.
    #[arg(long)]
    pub recheck_id: Uuid,
    /// Hash of the current stored pass or latest rechecked version.
    #[arg(long, value_parser = parse_pipeline_hash)]
    pub expected_source_hash: String,
}

#[derive(Debug, clap::Args)]
pub(crate) struct PipelineAssessmentArgs {
    #[arg(long)]
    pub run_id: Uuid,
    #[arg(long)]
    pub lease_token: Uuid,
    #[arg(long, value_enum)]
    pub decision: ReviewDecision,
    #[arg(long, value_parser = parse_pipeline_reason)]
    pub reason: String,
    /// Repeat for each hold resolved by an approval.
    #[arg(long = "resolved-quarantine-reason", value_parser = parse_pipeline_reason)]
    pub resolved_quarantine_reasons: Vec<String>,
    /// Exact version being assessed; required once a rechecked version exists.
    #[arg(long, requires = "candidate_content_hash")]
    pub candidate_id: Option<Uuid>,
    #[arg(long, requires = "candidate_id", value_parser = parse_pipeline_hash)]
    pub candidate_content_hash: Option<String>,
}

fn parse_pipeline_hash(value: &str) -> std::result::Result<String, String> {
    match value.strip_prefix("sha256:") {
        Some(hex)
            if hex.len() == 64
                && hex
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) =>
        {
            Ok(value.to_owned())
        }
        _ => Err("expected sha256: followed by 64 lowercase hexadecimal characters".into()),
    }
}

fn parse_pipeline_reason(value: &str) -> std::result::Result<String, String> {
    if (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        Ok(value.to_owned())
    } else {
        Err("reason must contain 1 to 64 lowercase letters, digits or underscores".into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ReviewLeaseFilter {
    All,
    Mine,
    Available,
    Active,
    Expired,
}

impl ReviewLeaseFilter {
    fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Mine => "mine",
            Self::Available => "available",
            Self::Active => "active",
            Self::Expired => "expired",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ReviewDecision {
    Approve,
    Reject,
}

impl ReviewDecision {
    fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Reject => "reject",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum CreditEventType {
    BenchmarkConversion,
    RegressionCatch,
    TrainingUtility,
    RankingUtility,
    ReviewerBonus,
    AbusePenalty,
}

impl CreditEventType {
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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = build_review_client(&cli)?;
    dispatch(&client, &cli.endpoint, cli.command, cli.json).await
}

// A recheck can classify many fields. Keep the normal client timeout for
// every other operation, but let a recheck finish within its 30-minute
// review claim plus a minute for the server to return its committed result.
fn build_review_client(cli: &Cli) -> Result<Client> {
    if !matches!(&cli.command, ReviewSubcommand::PipelineRecheck(_)) {
        return build_client(
            &cli.endpoint,
            &cli.bearer_token_env,
            cli.allowed_hosts.as_deref(),
        );
    }
    let mut builder = Client::builder(&cli.endpoint, &cli.bearer_token_env)
        .timeout(std::time::Duration::from_secs(31 * 60));
    if let Some(csv) = cli.allowed_hosts.as_deref() {
        builder = builder.host_allowlist(
            trace_commons_operator_client::host_allowlist::HostAllowlist::from_csv(csv),
        );
    }
    Ok(builder.build()?)
}

async fn dispatch(
    client: &Client,
    endpoint: &str,
    cmd: ReviewSubcommand,
    json: bool,
) -> Result<()> {
    match cmd {
        ReviewSubcommand::QuarantineList(args) => {
            quarantine_list(client, endpoint, args, json).await
        }
        ReviewSubcommand::ActiveLearningReviewQueue(args) => {
            active_learning(client, endpoint, args, json).await
        }
        ReviewSubcommand::ReviewDecision(args) => {
            review_decision(client, endpoint, args, json).await
        }
        ReviewSubcommand::ReviewLeaseClaim(args) => lease_claim(client, endpoint, args, json).await,
        ReviewSubcommand::ReviewLeaseClaimNext(args) => {
            lease_claim_next(client, endpoint, args, json).await
        }
        ReviewSubcommand::ReviewLeaseClaimBatch(args) => {
            lease_claim_batch(client, endpoint, args, json).await
        }
        ReviewSubcommand::ReviewLeaseRelease(args) => {
            lease_release(client, endpoint, args, json).await
        }
        ReviewSubcommand::AppendCreditEvent(args) => {
            append_credit_event(client, endpoint, args, json).await
        }
        ReviewSubcommand::PipelineQuarantine(args) => {
            pipeline_quarantine(client, endpoint, args, json).await
        }
        ReviewSubcommand::PipelineClaim(args) => pipeline_claim(client, endpoint, args, json).await,
        ReviewSubcommand::PipelineRecheck(args) => {
            pipeline_recheck(client, endpoint, args, json).await
        }
        ReviewSubcommand::PipelineAssessment(args) => {
            pipeline_assessment(client, endpoint, args, json).await
        }
    }
}

async fn pipeline_quarantine(
    client: &Client,
    endpoint: &str,
    args: PipelineQuarantineArgs,
    json: bool,
) -> Result<()> {
    let path = "/v1/review/pipeline/quarantine";
    let owned = args
        .limit
        .map(|limit| vec![("limit", limit.to_string())])
        .unwrap_or_default();
    let query = borrow_query(&owned);
    let value: Value = client
        .call_json::<(), Value>(Method::GET, path, &query, None)
        .await?;
    if json {
        emit_json(endpoint, "GET", path, &value)?;
    } else {
        render_items(
            &mut stdout(),
            "Pipeline review queue",
            &value,
            &[
                "run_id",
                "submission_id",
                "phase",
                "hold_reason",
                "assessment_superseded",
                "review_source_hash",
                "review_candidate",
                "created_at",
            ],
        )?;
    }
    Ok(())
}

async fn pipeline_claim(
    client: &Client,
    endpoint: &str,
    args: PipelineClaimArgs,
    json: bool,
) -> Result<()> {
    let path = format!("/v1/review/pipeline/runs/{}/claim", args.run_id);
    let value: Value = client
        .call_json::<(), Value>(Method::POST, &path, &[], None)
        .await?;
    if json {
        emit_json(endpoint, "POST", &path, &value)?;
    } else {
        let mut out = stdout();
        writeln!(out, "Claimed pipeline review for {}", args.run_id)?;
        render_kv_fields(
            &mut out,
            &value,
            &[
                ("  lease token", "lease_token"),
                ("  expires at", "lease_expires_at"),
            ],
        )?;
    }
    Ok(())
}

async fn pipeline_recheck(
    client: &Client,
    endpoint: &str,
    args: PipelineRecheckArgs,
    json: bool,
) -> Result<()> {
    parse_pipeline_hash(&args.expected_source_hash).map_err(anyhow::Error::msg)?;
    let path = format!("/v1/review/pipeline/runs/{}/recheck", args.run_id);
    let body = serde_json::json!({
        "lease_token":args.lease_token,"recheck_id":args.recheck_id,
        "expected_source_hash":args.expected_source_hash,
    });
    let value: Value = client
        .call_json::<Value, Value>(Method::POST, &path, &[], Some(&body))
        .await?;
    if json {
        emit_json(endpoint, "POST", &path, &value)?;
    } else {
        let mut out = stdout();
        writeln!(out, "Saved rechecked version for {}", args.run_id)?;
        render_kv_fields(
            &mut out,
            &value,
            &[
                ("  candidate id", "candidate_id"),
                ("  content hash", "content_hash"),
                ("  source hash", "source_hash"),
                ("  risk", "residual_pii_risk"),
                ("  coverage complete", "coverage_complete"),
            ],
        )?;
    }
    Ok(())
}

fn pipeline_assessment_body(args: &PipelineAssessmentArgs) -> Result<Value> {
    if args.candidate_id.is_some() != args.candidate_content_hash.is_some() {
        anyhow::bail!("candidate id and content hash must be supplied together");
    }
    parse_pipeline_reason(&args.reason).map_err(anyhow::Error::msg)?;
    for reason in &args.resolved_quarantine_reasons {
        parse_pipeline_reason(reason).map_err(anyhow::Error::msg)?;
    }
    let mut body = serde_json::json!({
        "lease_token":args.lease_token,"recommendation":args.decision.as_str(),
        "reason":args.reason,"resolved_quarantine_reasons":args.resolved_quarantine_reasons,
    });
    if let (Some(candidate_id), Some(content_hash)) =
        (args.candidate_id, args.candidate_content_hash.as_ref())
    {
        parse_pipeline_hash(content_hash).map_err(anyhow::Error::msg)?;
        body["candidate_id"] = serde_json::json!(candidate_id);
        body["candidate_content_hash"] = serde_json::json!(content_hash);
    }
    Ok(body)
}

async fn pipeline_assessment(
    client: &Client,
    endpoint: &str,
    args: PipelineAssessmentArgs,
    json: bool,
) -> Result<()> {
    let body = pipeline_assessment_body(&args)?;
    let path = format!("/v1/review/pipeline/runs/{}/assessment", args.run_id);
    let value: Value = client
        .call_json::<Value, Value>(Method::POST, &path, &[], Some(&body))
        .await?;
    if json {
        emit_json(endpoint, "POST", &path, &value)?;
    } else {
        let mut out = stdout();
        writeln!(out, "Recorded pipeline assessment for {}", args.run_id)?;
        render_kv_fields(&mut out, &value, &[("  assessment id", "assessment_id")])?;
    }
    Ok(())
}

async fn quarantine_list(
    client: &Client,
    endpoint: &str,
    args: QuarantineListArgs,
    json: bool,
) -> Result<()> {
    let path = "/v1/review/quarantine";
    let mut owned: Vec<(&str, String)> = Vec::new();
    if let Some(filter) = args.lease_filter {
        owned.push(("lease_filter", filter.as_str().to_string()));
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
            "Central quarantined traces",
            &value,
            &[
                "submission_id",
                "privacy_risk",
                "submission_score",
                "review_escalation_state",
                "review_age_hours",
                "review_escalation_reasons",
                "review_assigned_to_principal_ref",
                "review_lease_expires_at",
                "review_due_at",
                "received_at",
                "canonical_summary",
            ],
        )?;
    }
    Ok(())
}

async fn active_learning(
    client: &Client,
    endpoint: &str,
    args: ActiveLearningArgs,
    json: bool,
) -> Result<()> {
    let path = "/v1/review/active-learning";
    let mut owned: Vec<(&str, String)> = Vec::new();
    if let Some(limit) = args.limit {
        owned.push(("limit", limit.to_string()));
    }
    if let Some(privacy_risk) = args.privacy_risk {
        owned.push(("privacy_risk", privacy_risk.as_str().to_string()));
    }
    if let Some(filter) = args.lease_filter {
        owned.push(("lease_filter", filter.as_str().to_string()));
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
            "Central active-learning review queue",
            &value,
            &[
                "submission_id",
                "status",
                "privacy_risk",
                "priority_score",
                "priority_reasons",
                "review_escalation_state",
                "review_age_hours",
                "review_escalation_reasons",
                "review_assigned_to_principal_ref",
                "review_lease_expires_at",
                "review_due_at",
                "received_at",
            ],
        )?;
    }
    Ok(())
}

async fn review_decision(
    client: &Client,
    endpoint: &str,
    args: ReviewDecisionArgs,
    json: bool,
) -> Result<()> {
    let path = format!("/v1/review/{}/decision", args.submission_id);
    let mut body = serde_json::json!({ "decision": args.decision.as_str() });
    if let Some(reason) = args.reason.as_ref() {
        body["reason"] = Value::String(reason.clone());
    }
    if let Some(credit) = args.credit_points_pending {
        body["credit_points_pending"] = serde_json::json!(credit);
    }
    let value: Value = client
        .call_json::<Value, Value>(Method::POST, &path, &[], Some(&body))
        .await?;
    if json {
        emit_json(endpoint, "POST", &path, &value)?;
    } else {
        let mut out = stdout();
        writeln!(
            out,
            "Recorded central review decision for {}",
            args.submission_id
        )?;
        render_kv_fields(
            &mut out,
            &value,
            &[
                ("  status", "status"),
                ("  pending credit", "credit_points_pending"),
                ("  final credit", "credit_points_final"),
            ],
        )?;
    }
    Ok(())
}

async fn lease_claim(
    client: &Client,
    endpoint: &str,
    args: LeaseClaimArgs,
    json: bool,
) -> Result<()> {
    let path = format!("/v1/review/{}/lease", args.submission_id);
    let body = lease_body(args.lease_ttl_seconds, args.review_due_at.as_deref(), None)?;
    let value: Value = client
        .call_json::<Value, Value>(Method::POST, &path, &[], Some(&body))
        .await?;
    if json {
        emit_json(endpoint, "POST", &path, &value)?;
    } else {
        let mut out = stdout();
        writeln!(
            out,
            "Claimed central review lease for {}",
            args.submission_id
        )?;
        render_lease_fields(&mut out, &value)?;
    }
    Ok(())
}

async fn lease_claim_next(
    client: &Client,
    endpoint: &str,
    args: LeaseClaimNextArgs,
    json: bool,
) -> Result<()> {
    let path = "/v1/review/leases/claim-next";
    let body = lease_body(
        args.lease_ttl_seconds,
        args.review_due_at.as_deref(),
        args.privacy_risk,
    )?;
    let value: Value = client
        .call_json::<Value, Value>(Method::POST, path, &[], Some(&body))
        .await?;
    if json {
        emit_json(endpoint, "POST", path, &value)?;
    } else {
        let mut out = stdout();
        if let Some(submission_id) = value.get("submission_id").and_then(Value::as_str) {
            writeln!(out, "Claimed next central review lease for {submission_id}")?;
        } else {
            writeln!(out, "Claimed next central review lease")?;
        }
        render_lease_fields(&mut out, &value)?;
    }
    Ok(())
}

async fn lease_claim_batch(
    client: &Client,
    endpoint: &str,
    args: LeaseClaimBatchArgs,
    json: bool,
) -> Result<()> {
    if let Some(limit) = args.limit {
        if limit == 0 {
            anyhow::bail!("--limit must be greater than 0");
        }
    }
    let path = "/v1/review/leases/claim-batch";
    let mut body = lease_body(
        args.lease_ttl_seconds,
        args.review_due_at.as_deref(),
        args.privacy_risk,
    )?;
    if let Some(limit) = args.limit {
        body["limit"] = serde_json::json!(limit);
    }
    let value: Value = client
        .call_json::<Value, Value>(Method::POST, path, &[], Some(&body))
        .await?;
    if json {
        emit_json(endpoint, "POST", path, &value)?;
    } else {
        let mut out = stdout();
        let count = value
            .get("claim_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        writeln!(out, "Claimed {count} central review leases")?;
        if let Some(claimed) = value.get("claimed").and_then(Value::as_array) {
            for claim in claimed {
                render_lease_fields(&mut out, claim)?;
            }
        }
    }
    Ok(())
}

async fn lease_release(
    client: &Client,
    endpoint: &str,
    args: LeaseReleaseArgs,
    json: bool,
) -> Result<()> {
    let path = format!("/v1/review/{}/lease", args.submission_id);
    let value: Value = client
        .call_json::<(), Value>(Method::DELETE, &path, &[], None)
        .await?;
    if json {
        emit_json(endpoint, "DELETE", &path, &value)?;
    } else {
        let mut out = stdout();
        writeln!(
            out,
            "Released central review lease for {}",
            args.submission_id
        )?;
        render_lease_fields(&mut out, &value)?;
    }
    Ok(())
}

async fn append_credit_event(
    client: &Client,
    endpoint: &str,
    args: AppendCreditEventArgs,
    json: bool,
) -> Result<()> {
    let path = format!("/v1/review/{}/credit-events", args.submission_id);
    let mut body = serde_json::json!({
        "event_type": args.event_type.as_str(),
        "credit_points_delta": args.credit_points_delta,
        "reason": args.reason,
    });
    if let Some(external_ref) = args.external_ref.as_ref() {
        body["external_ref"] = Value::String(external_ref.clone());
    }
    let value: Value = client
        .call_json::<Value, Value>(Method::POST, &path, &[], Some(&body))
        .await?;
    if json {
        emit_json(endpoint, "POST", &path, &value)?;
    } else {
        let mut out = stdout();
        writeln!(
            out,
            "Appended central delayed credit event for {}",
            args.submission_id
        )?;
        render_kv_fields(
            &mut out,
            &value,
            &[
                ("  event id", "event_id"),
                ("  event type", "event_type"),
                ("  delta", "credit_points_delta"),
                ("  reason", "reason"),
            ],
        )?;
    }
    Ok(())
}

// --- helpers ---

fn lease_body(
    lease_ttl_seconds: Option<i64>,
    review_due_at: Option<&str>,
    privacy_risk: Option<PrivacyRisk>,
) -> Result<Value> {
    let mut body = serde_json::Map::new();
    if let Some(ttl) = lease_ttl_seconds {
        if ttl <= 0 {
            anyhow::bail!("--lease-ttl-seconds must be greater than 0");
        }
        body.insert("lease_ttl_seconds".into(), serde_json::json!(ttl));
    }
    if let Some(due_at) = review_due_at {
        let trimmed = due_at.trim();
        if !trimmed.is_empty() {
            chrono::DateTime::parse_from_rfc3339(trimmed)
                .map_err(|e| anyhow::anyhow!("--review-due-at must be RFC3339: {e}"))?;
            body.insert("review_due_at".into(), Value::String(trimmed.to_string()));
        }
    }
    if let Some(privacy_risk) = privacy_risk {
        body.insert(
            "privacy_risk".into(),
            Value::String(privacy_risk.as_str().to_string()),
        );
    }
    Ok(Value::Object(body))
}

fn render_lease_fields<W: Write>(out: &mut W, value: &Value) -> std::io::Result<()> {
    render_kv_fields(
        out,
        value,
        &[
            ("  tenant id", "tenant_id"),
            ("  tenant storage ref", "tenant_storage_ref"),
            ("  trace id", "trace_id"),
            ("  status", "status"),
            ("  assigned principal", "review_assigned_to_principal_ref"),
            ("  assigned at", "review_assigned_at"),
            ("  lease expires at", "review_lease_expires_at"),
            ("  review due at", "review_due_at"),
        ],
    )
}

// --- tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use trace_commons_operator_client::host_allowlist::HostAllowlist;
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn unique_bearer_env() -> String {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        format!("TC_REVIEW_TEST_BEARER_{n}")
    }

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    struct EnvGuard {
        name: String,
    }

    impl EnvGuard {
        #[allow(unsafe_code)]
        fn set(name: String, value: &str) -> Self {
            let _lock = env_lock();
            // SAFETY: serialized via `env_lock` and uses a unique variable name per test.
            unsafe { std::env::set_var(&name, value) };
            EnvGuard { name }
        }
    }

    impl Drop for EnvGuard {
        #[allow(unsafe_code)]
        fn drop(&mut self) {
            let _lock = env_lock();
            // SAFETY: serialized via `env_lock`; see `EnvGuard::set`.
            unsafe { std::env::remove_var(&self.name) };
        }
    }

    #[test]
    fn cli_parses_quarantine_list_minimal() {
        let cli = Cli::try_parse_from([
            "trace-commons-review",
            "--endpoint",
            "https://api.example",
            "quarantine-list",
        ])
        .unwrap();
        match cli.command {
            ReviewSubcommand::QuarantineList(args) => assert!(args.lease_filter.is_none()),
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_quarantine_list_with_filter() {
        let cli = Cli::try_parse_from([
            "trace-commons-review",
            "--endpoint",
            "https://api.example",
            "quarantine-list",
            "--lease-filter",
            "mine",
        ])
        .unwrap();
        match cli.command {
            ReviewSubcommand::QuarantineList(args) => {
                assert_eq!(args.lease_filter, Some(ReviewLeaseFilter::Mine));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_review_decision() {
        let id = Uuid::new_v4();
        let cli = Cli::try_parse_from([
            "trace-commons-review",
            "--endpoint",
            "https://api.example",
            "review-decision",
            "--submission-id",
            &id.to_string(),
            "--decision",
            "approve",
        ])
        .unwrap();
        match cli.command {
            ReviewSubcommand::ReviewDecision(args) => {
                assert_eq!(args.submission_id, id);
                assert_eq!(args.decision, ReviewDecision::Approve);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn cli_rejects_review_decision_invalid_choice() {
        let id = Uuid::new_v4();
        let err = Cli::try_parse_from([
            "trace-commons-review",
            "--endpoint",
            "https://api.example",
            "review-decision",
            "--submission-id",
            &id.to_string(),
            "--decision",
            "maybe",
        ])
        .unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn cli_parses_active_learning_full() {
        let cli = Cli::try_parse_from([
            "trace-commons-review",
            "--endpoint",
            "https://api.example",
            "active-learning-review-queue",
            "--limit",
            "10",
            "--privacy-risk",
            "low",
            "--lease-filter",
            "active",
        ])
        .unwrap();
        match cli.command {
            ReviewSubcommand::ActiveLearningReviewQueue(args) => {
                assert_eq!(args.limit, Some(10));
                assert_eq!(args.privacy_risk, Some(PrivacyRisk::Low));
                assert_eq!(args.lease_filter, Some(ReviewLeaseFilter::Active));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_lease_claim() {
        let id = Uuid::new_v4();
        Cli::try_parse_from([
            "trace-commons-review",
            "--endpoint",
            "https://api.example",
            "review-lease-claim",
            "--submission-id",
            &id.to_string(),
            "--lease-ttl-seconds",
            "120",
        ])
        .unwrap();
    }

    #[test]
    fn cli_parses_lease_claim_next() {
        Cli::try_parse_from([
            "trace-commons-review",
            "--endpoint",
            "https://api.example",
            "review-lease-claim-next",
            "--privacy-risk",
            "high",
        ])
        .unwrap();
    }

    #[test]
    fn cli_parses_lease_claim_batch() {
        Cli::try_parse_from([
            "trace-commons-review",
            "--endpoint",
            "https://api.example",
            "review-lease-claim-batch",
            "--limit",
            "5",
        ])
        .unwrap();
    }

    #[test]
    fn cli_parses_lease_release() {
        let id = Uuid::new_v4();
        Cli::try_parse_from([
            "trace-commons-review",
            "--endpoint",
            "https://api.example",
            "review-lease-release",
            "--submission-id",
            &id.to_string(),
        ])
        .unwrap();
    }

    #[test]
    fn cli_parses_append_credit_event() {
        let id = Uuid::new_v4();
        Cli::try_parse_from([
            "trace-commons-review",
            "--endpoint",
            "https://api.example",
            "append-credit-event",
            "--submission-id",
            &id.to_string(),
            "--event-type",
            "benchmark-conversion",
            "--credit-points-delta",
            "1.5",
            "--reason",
            "good trace",
        ])
        .unwrap();
    }

    #[test]
    fn lease_body_rejects_non_positive_ttl() {
        let err = lease_body(Some(0), None, None).unwrap_err();
        assert!(err.to_string().contains("greater than 0"));
    }

    #[test]
    fn lease_body_rejects_bad_due_at() {
        let err = lease_body(None, Some("yesterday"), None).unwrap_err();
        assert!(err.to_string().contains("RFC3339"));
    }

    #[tokio::test]
    async fn quarantine_list_happy_path_hits_expected_route() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/review/quarantine"))
            .and(query_param("lease_filter", "mine"))
            .and(header("authorization", "Bearer reviewer-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    {"submission_id": "sub-1", "privacy_risk": "low"}
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "reviewer-secret");
        let mut builder = Client::builder(server.uri(), env);
        builder = builder.host_allowlist(HostAllowlist::permissive());
        let client = builder.build().unwrap();
        quarantine_list(
            &client,
            &server.uri(),
            QuarantineListArgs {
                lease_filter: Some(ReviewLeaseFilter::Mine),
            },
            true,
        )
        .await
        .expect("happy path");
    }

    #[tokio::test]
    async fn review_decision_posts_expected_body() {
        let id = Uuid::new_v4();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/v1/review/{id}/decision")))
            .and(body_json(serde_json::json!({
                "decision": "approve",
                "reason": "looks fine",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "approved",
                "credit_points_final": 1.0,
            })))
            .expect(1)
            .mount(&server)
            .await;

        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder(server.uri(), env).build().unwrap();
        review_decision(
            &client,
            &server.uri(),
            ReviewDecisionArgs {
                submission_id: id,
                decision: ReviewDecision::Approve,
                reason: Some("looks fine".into()),
                credit_points_pending: None,
            },
            true,
        )
        .await
        .expect("decision posted");
    }
    fn pipeline_cli(command: &[&str]) -> Result<Cli, clap::Error> {
        let mut args = vec!["trace-commons-review", "--endpoint", "https://api.example"];
        args.extend_from_slice(command);
        Cli::try_parse_from(args)
    }

    #[test]
    fn cli_parses_pipeline_commands_and_exact_candidate_binding() {
        let run = Uuid::new_v4().to_string();
        let lease = Uuid::new_v4().to_string();
        let recheck = Uuid::new_v4().to_string();
        let candidate = Uuid::new_v4().to_string();
        let source_hash = format!("sha256:{}", "a".repeat(64));
        let candidate_hash = format!("sha256:{}", "b".repeat(64));
        assert!(matches!(
            pipeline_cli(&["pipeline-quarantine", "--limit", "25"])
                .unwrap()
                .command,
            ReviewSubcommand::PipelineQuarantine(PipelineQuarantineArgs { limit: Some(25) })
        ));
        assert!(matches!(
            pipeline_cli(&["pipeline-claim", "--run-id", &run])
                .unwrap()
                .command,
            ReviewSubcommand::PipelineClaim(_)
        ));
        match pipeline_cli(&[
            "pipeline-recheck",
            "--run-id",
            &run,
            "--lease-token",
            &lease,
            "--recheck-id",
            &recheck,
            "--expected-source-hash",
            &source_hash,
        ])
        .unwrap()
        .command
        {
            ReviewSubcommand::PipelineRecheck(args) => {
                assert_eq!(args.run_id.to_string(), run);
                assert_eq!(args.recheck_id.to_string(), recheck);
                assert_eq!(args.expected_source_hash, source_hash);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
        match pipeline_cli(&[
            "pipeline-assessment",
            "--run-id",
            &run,
            "--lease-token",
            &lease,
            "--decision",
            "approve",
            "--reason",
            "privacy_rechecked",
            "--resolved-quarantine-reason",
            "privacy_risk_high_review_required",
            "--candidate-id",
            &candidate,
            "--candidate-content-hash",
            &candidate_hash,
        ])
        .unwrap()
        .command
        {
            ReviewSubcommand::PipelineAssessment(args) => {
                assert_eq!(args.decision, ReviewDecision::Approve);
                assert_eq!(args.candidate_id.unwrap().to_string(), candidate);
                assert_eq!(
                    args.candidate_content_hash.as_deref(),
                    Some(candidate_hash.as_str())
                );
                assert_eq!(
                    args.resolved_quarantine_reasons,
                    ["privacy_risk_high_review_required"]
                );
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn pipeline_assessment_requires_both_candidate_fields() {
        let run = Uuid::new_v4().to_string();
        let lease = Uuid::new_v4().to_string();
        let candidate = Uuid::new_v4().to_string();
        let hash = format!("sha256:{}", "a".repeat(64));
        let mut base = vec![
            "pipeline-assessment",
            "--run-id",
            &run,
            "--lease-token",
            &lease,
            "--decision",
            "reject",
            "--reason",
            "privacy_unresolved",
        ];
        assert!(pipeline_cli(&base).is_ok());
        base.extend(["--candidate-id", &candidate]);
        assert!(pipeline_cli(&base).is_err());
        base.truncate(base.len() - 2);
        base.extend(["--candidate-content-hash", &hash]);
        assert!(pipeline_cli(&base).is_err());
    }

    #[test]
    fn pipeline_arguments_reject_invalid_hashes_and_free_text_reasons() {
        let run = Uuid::new_v4().to_string();
        let lease = Uuid::new_v4().to_string();
        let recheck = Uuid::new_v4().to_string();
        assert!(
            pipeline_cli(&[
                "pipeline-recheck",
                "--run-id",
                &run,
                "--lease-token",
                &lease,
                "--recheck-id",
                &recheck,
                "--expected-source-hash",
                "not_a_hash"
            ])
            .is_err()
        );
        assert!(
            pipeline_cli(&[
                "pipeline-assessment",
                "--run-id",
                &run,
                "--lease-token",
                &lease,
                "--decision",
                "approve",
                "--reason",
                "looks fine"
            ])
            .is_err()
        );
        assert!(
            pipeline_cli(&[
                "pipeline-assessment",
                "--run-id",
                &run,
                "--lease-token",
                &lease,
                "--decision",
                "approve",
                "--reason",
                "privacy_rechecked",
                "--resolved-quarantine-reason",
                "free text"
            ])
            .is_err()
        );
    }

    #[tokio::test]
    async fn pipeline_quarantine_uses_pipeline_route_and_limit() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/review/pipeline/quarantine"))
            .and(query_param("limit", "25"))
            .and(header("authorization", "Bearer reviewer-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .expect(1)
            .mount(&server)
            .await;
        let env = unique_bearer_env();
        let _guard = EnvGuard::set(env.clone(), "reviewer-secret");
        let client = Client::builder(server.uri(), env).build().unwrap();
        pipeline_quarantine(
            &client,
            &server.uri(),
            PipelineQuarantineArgs { limit: Some(25) },
            true,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn pipeline_claim_posts_to_run_claim_route() {
        let run_id = Uuid::new_v4();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/v1/review/pipeline/runs/{run_id}/claim")))
            .and(header("authorization", "Bearer reviewer-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "lease_token": Uuid::new_v4(), "lease_expires_at": "2026-11-15T12:00:00Z"
            })))
            .expect(1)
            .mount(&server)
            .await;
        let env = unique_bearer_env();
        let _guard = EnvGuard::set(env.clone(), "reviewer-secret");
        let client = Client::builder(server.uri(), env).build().unwrap();
        pipeline_claim(&client, &server.uri(), PipelineClaimArgs { run_id }, true)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn pipeline_recheck_posts_idempotency_source_and_lease_in_body() {
        let run_id = Uuid::new_v4();
        let lease_token = Uuid::new_v4();
        let recheck_id = Uuid::new_v4();
        let expected_source_hash = format!("sha256:{}", "a".repeat(64));
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/v1/review/pipeline/runs/{run_id}/recheck")))
            .and(header("authorization", "Bearer reviewer-secret"))
            .and(body_json(serde_json::json!({
                "lease_token":lease_token,"recheck_id":recheck_id,
                "expected_source_hash":expected_source_hash,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "candidate_id":recheck_id,"content_hash":format!("sha256:{}", "b".repeat(64)),
                "coverage_complete":true,
            })))
            .expect(1)
            .mount(&server)
            .await;
        let env = unique_bearer_env();
        let _guard = EnvGuard::set(env.clone(), "reviewer-secret");
        let client = Client::builder(server.uri(), env).build().unwrap();
        pipeline_recheck(
            &client,
            &server.uri(),
            PipelineRecheckArgs {
                run_id,
                lease_token,
                recheck_id,
                expected_source_hash,
            },
            true,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn pipeline_assessment_posts_exact_candidate_and_review_reasons() {
        let run_id = Uuid::new_v4();
        let lease_token = Uuid::new_v4();
        let candidate_id = Uuid::new_v4();
        let candidate_content_hash = format!("sha256:{}", "b".repeat(64));
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!(
                "/v1/review/pipeline/runs/{run_id}/assessment"
            )))
            .and(body_json(serde_json::json!({
                "lease_token":lease_token,"recommendation":"approve","reason":"privacy_rechecked",
                "resolved_quarantine_reasons":["privacy_risk_high_review_required"],
                "candidate_id":candidate_id,"candidate_content_hash":candidate_content_hash,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "assessment_id":Uuid::new_v4(),
            })))
            .expect(1)
            .mount(&server)
            .await;
        let env = unique_bearer_env();
        let _guard = EnvGuard::set(env.clone(), "reviewer-secret");
        let client = Client::builder(server.uri(), env).build().unwrap();
        pipeline_assessment(
            &client,
            &server.uri(),
            PipelineAssessmentArgs {
                run_id,
                lease_token,
                decision: ReviewDecision::Approve,
                reason: "privacy_rechecked".into(),
                resolved_quarantine_reasons: vec!["privacy_risk_high_review_required".into()],
                candidate_id: Some(candidate_id),
                candidate_content_hash: Some(candidate_content_hash),
            },
            true,
        )
        .await
        .unwrap();
    }

    #[test]
    fn legacy_pipeline_assessment_omits_candidate_fields_and_rejects_partial_pair() {
        let mut args = PipelineAssessmentArgs {
            run_id: Uuid::new_v4(),
            lease_token: Uuid::new_v4(),
            decision: ReviewDecision::Reject,
            reason: "privacy_unresolved".into(),
            resolved_quarantine_reasons: Vec::new(),
            candidate_id: None,
            candidate_content_hash: None,
        };
        let body = pipeline_assessment_body(&args).unwrap();
        assert_eq!(body["recommendation"], "reject");
        assert!(body.get("candidate_id").is_none());
        assert!(body.get("candidate_content_hash").is_none());
        args.candidate_id = Some(Uuid::new_v4());
        assert!(pipeline_assessment_body(&args).is_err());
    }
}
