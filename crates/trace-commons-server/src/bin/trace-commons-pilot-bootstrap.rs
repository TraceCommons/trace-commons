// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `trace-commons-pilot-bootstrap` — replay HuggingFace agent-traces datasets into a
//! running `trace-commons-ingest` for Phase A.6 pilot-bootstrap calibration.
//!
//! See `docs/superpowers/specs/2026-05-14-pilot-bootstrap-harness-design.md`
//! and `docs/operator/pilot-bootstrap.md` for the design + operator runbook.

use std::path::PathBuf;

use clap::Parser;

mod pilot_bootstrap;

use pilot_bootstrap::run_pilot_bootstrap;

#[derive(Debug, Clone, Parser)]
#[command(
    name = "trace-commons-pilot-bootstrap",
    about = "Replay HuggingFace agent-traces datasets into a running trace-commons-ingest",
    // The semver does not move when a deploy does, so --version carries the
    // commit the binary was built from as well.
    version = trace_commons_build_info::version_line(env!("CARGO_PKG_VERSION"))
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

    /// Local cache directory for downloaded HF JSONL session files.
    /// Defaults to the standard `hf-hub` cache (`~/.cache/huggingface`).
    #[arg(long)]
    cache_dir: Option<PathBuf>,

    /// Minimum word count for a session to be submitted. Sessions below
    /// this threshold are skipped silently.
    #[arg(long, default_value_t = pilot_bootstrap::DEFAULT_MIN_WORDS)]
    min_words: usize,

    /// Maximum word count for a session to be submitted. Sessions above
    /// this threshold are skipped silently.
    #[arg(long, default_value_t = pilot_bootstrap::DEFAULT_MAX_WORDS)]
    max_words: usize,

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
        min_words: cli.min_words,
        max_words: cli.max_words,
    };

    run_pilot_bootstrap(config).await
}
