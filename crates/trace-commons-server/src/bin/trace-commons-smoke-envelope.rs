//! `trace-commons-smoke-envelope` — deploy-probe helper that builds a minimal
//! `TraceContributionEnvelope` via the same redactor pipeline real clients use
//! and POSTs it to a running `trace-commons-ingest`'s `/v1/traces` endpoint.
//!
//! Intended pairing with `trace-commons-upload-claim-issuer --mint-test-claim`
//! (set `TRACE_COMMONS_MINT_TEST_CLAIM_CONSENT_SCOPES=debugging_evaluation`
//! and `TRACE_COMMONS_MINT_TEST_CLAIM_ALLOWED_USES=debugging` so the minted
//! claim covers the envelope's default consent scope).
//!
//! Prints the HTTP status and response body; exits non-zero if the request
//! fails to send. A 4xx with a structured error body is treated as a
//! successful probe — the operator inspects the body to decide.

use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use reqwest::Client;
use trace_commons_protocol::llm::recording::{TraceFile, TraceResponse, TraceStep};
use trace_commons_protocol::trace_contribution::{
    DeterministicTraceRedactor, RawTraceContribution, RecordedTraceContributionOptions,
    TraceContributionEnvelope, TraceRedactor,
};

#[derive(Parser, Debug)]
#[command(
    name = "trace-commons-smoke-envelope",
    about = "Build + POST a smoke envelope to /v1/traces",
    // The semver does not move when a deploy does, so --version carries the
    // commit the binary was built from as well.
    version = trace_commons_build_info::version_line(env!("CARGO_PKG_VERSION"))
)]
struct Args {
    /// Base URL of the running ingest (no trailing slash). Reads from
    /// `TRACE_COMMONS_SMOKE_TARGET` if not provided.
    #[arg(long, env = "TRACE_COMMONS_SMOKE_TARGET")]
    target: String,

    /// Bearer token (an upload-claim JWT). Reads from
    /// `TRACE_COMMONS_SMOKE_BEARER` if not provided.
    #[arg(long, env = "TRACE_COMMONS_SMOKE_BEARER", hide_env_values = true)]
    bearer: String,

    /// Body content for the synthetic user-message event.
    #[arg(long, default_value = "trace-commons-smoke-envelope probe")]
    body: String,

    /// Pseudonymous contributor id (any stable string; will be hashed by the
    /// redactor). The default lines up with the `--mint-test-claim` principal
    /// so the envelope attribution matches the bearer's claim.
    #[arg(
        long,
        default_value = "sha256:trace-commons-smoke-envelope-contributor"
    )]
    contributor: String,

    /// Tenant scope ref placed on the envelope's `contributor.tenant_scope_ref`.
    /// Defaults to the `--mint-test-claim` tenant.
    #[arg(long, default_value = "trace-upload-claim-issuer-test-tenant")]
    tenant_scope_ref: String,

    /// Request timeout in seconds.
    #[arg(long, default_value_t = 30)]
    timeout_secs: u64,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?;
    rt.block_on(run(args))
}

async fn run(args: Args) -> Result<()> {
    let envelope = build_envelope(&args.body, &args.contributor, &args.tenant_scope_ref).await?;
    let target = args.target.trim_end_matches('/').to_string();
    let url = format!("{target}/v1/traces");
    let client = Client::builder()
        .timeout(Duration::from_secs(args.timeout_secs))
        .build()
        .context("build reqwest client")?;
    let response = client
        .post(&url)
        .bearer_auth(&args.bearer)
        .json(&envelope)
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;
    let status = response.status();
    let body = response.text().await.context("read response body")?;
    println!("submission_id={}", envelope.submission_id);
    println!("trace_id={}", envelope.trace_id);
    println!("HTTP {}", status.as_u16());
    println!("{body}");
    Ok(())
}

async fn build_envelope(
    body: &str,
    contributor: &str,
    tenant_scope_ref: &str,
) -> Result<TraceContributionEnvelope> {
    let trace = TraceFile {
        model_name: "trace-commons-smoke-envelope".to_string(),
        memory_snapshot: Vec::new(),
        http_exchanges: Vec::new(),
        steps: vec![TraceStep {
            request_hint: None,
            response: TraceResponse::UserInput {
                content: body.to_string(),
            },
            expected_tool_results: Vec::new(),
        }],
    };
    let options = RecordedTraceContributionOptions {
        include_message_text: true,
        pseudonymous_contributor_id: Some(contributor.to_string()),
        tenant_scope_ref: Some(tenant_scope_ref.to_string()),
        ..Default::default()
    };
    let raw = RawTraceContribution::from_recorded_trace(&trace, options);
    DeterministicTraceRedactor::try_default()
        .map_err(|err| anyhow::anyhow!("privacy filter config invalid: {err}"))?
        .redact_trace(raw)
        .await
        .context("redact smoke envelope")
}
