//! `tracedao-pilot-bootstrap` — replay HuggingFace agent-traces datasets into a
//! running `tracedao-ingest` for Phase A.6 pilot-bootstrap calibration.
//!
//! See `docs/superpowers/specs/2026-05-14-pilot-bootstrap-harness-design.md`
//! and `docs/operator/pilot-bootstrap.md` for the design + operator runbook.

use std::path::PathBuf;

use clap::Parser;

mod pilot_bootstrap;

use pilot_bootstrap::run_pilot_bootstrap;

#[derive(Debug, Clone, Parser)]
#[command(
    name = "tracedao-pilot-bootstrap",
    about = "Replay HuggingFace agent-traces datasets into a running tracedao-ingest"
)]
struct Cli {
    /// Source HF dataset id. Default: jedisct1/agent-traces-swival
    #[arg(long, default_value = "jedisct1/agent-traces-swival")]
    source: String,

    /// Per-dataset translator (`swival`, `pi-mono`, `deepseek-agent`).
    /// Auto-detected from `--source` if unset.
    #[arg(long)]
    translator: Option<String>,

    /// Total number of submissions to attempt.
    #[arg(long, default_value_t = 1000)]
    count: usize,

    /// Target ingest base URL (no trailing slash).
    #[arg(long, default_value = "http://localhost:3907")]
    target: String,

    /// Tenant bearer token. Falls back to the
    /// `TRACE_COMMONS_PILOT_TENANT_TOKEN` env var.
    #[arg(long, env = "TRACE_COMMONS_PILOT_TENANT_TOKEN")]
    tenant_token: String,

    /// Rate limit (requests per second).
    #[arg(long, default_value_t = 1.0)]
    rate: f64,

    /// Output sidecar JSONL path. Created if missing; appended otherwise.
    #[arg(long, default_value = "./pilot-bootstrap-sidecar.jsonl")]
    sidecar: PathBuf,

    /// Deterministic seed for row sampling.
    #[arg(long, default_value_t = 42)]
    seed: u64,

    /// Local cache directory for downloaded HF parquet shards.
    /// Defaults to the standard `hf-hub` cache (`~/.cache/huggingface`).
    #[arg(long)]
    cache_dir: Option<PathBuf>,

    /// Print the parsed configuration and exit without submitting.
    #[arg(long)]
    dry_run: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .json()
        .init();

    let config = pilot_bootstrap::PilotBootstrapConfig {
        source: cli.source,
        translator: cli.translator,
        count: cli.count,
        target: cli.target,
        tenant_token: cli.tenant_token,
        rate: cli.rate,
        sidecar: cli.sidecar,
        seed: cli.seed,
        cache_dir: cli.cache_dir,
        dry_run: cli.dry_run,
    };

    run_pilot_bootstrap(config).await
}
