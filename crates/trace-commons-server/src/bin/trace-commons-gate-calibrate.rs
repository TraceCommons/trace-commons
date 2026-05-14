//! Offline gate calibration helper + A2.1 perplexity-scorer bake-off harness.
//!
//! Two subcommands:
//!
//! * `calibrate` (default when no subcommand is given) — reads JSONL on stdin
//!   where each line is `{"plaintext": "..."}`, runs the standard
//!   `EnclaveGateOrchestrator` pipeline (real perplexity scorer + real
//!   embedder + in-memory mock vector index), and emits JSONL on stdout with
//!   the three numeric metrics needed to choose gate floors:
//!
//!   ```json
//!   {"perplexity_micros": N, "tail_fraction_micros": N, "novelty_score_micros": N}
//!   ```
//!
//!   This subcommand requires the `local-gpu-models` feature at build time.
//!
//! * `bake-off` — runs the A2.1 model bake-off against a fixed corpus,
//!   producing a JSON + markdown report. Uses the deterministic mock scorer
//!   so it is exercisable on CPU-only CI hosts.
//!
//! Both subcommands intentionally have no auth, no DB, no audit chain, no
//! credit emission. They exist to derive offline numbers without touching
//! production state. Hash-only diagnostics on failure.

// Bring the gate_calibrate/ submodules into the binary so the bake-off
// subcommand can call them directly. These same files are also pulled into
// integration tests via `#[path = ...]`, which is why they live under
// src/bin/gate_calibrate/ rather than under the library crate.
#[path = "gate_calibrate/bakeoff_corpus.rs"]
mod bakeoff_corpus;
#[path = "gate_calibrate/bakeoff_manifest.rs"]
mod bakeoff_manifest;
#[path = "gate_calibrate/bakeoff_metrics.rs"]
mod bakeoff_metrics;
#[path = "gate_calibrate/bakeoff_report.rs"]
mod bakeoff_report;
#[path = "gate_calibrate/run_candidate_eval.rs"]
mod run_candidate_eval;

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "trace-commons-gate-calibrate")]
#[command(about = "Offline gate calibration + bake-off harness", long_about = None)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run the existing env-driven calibration pass (requires
    /// `local-gpu-models` build feature). Reads JSONL from stdin, writes
    /// JSONL to stdout.
    Calibrate,
    /// Run the A2.1 perplexity-scorer bake-off and write a JSON + markdown
    /// report. The real-scorer path requires the `local-gpu-models` build
    /// feature and CUDA hardware (`--hardware=a10` or `--hardware=h100`);
    /// `--mock-scorer` is available for dry runs on CPU hosts and emits
    /// reports flagged `mock_scorer: true` so they cannot be confused with
    /// a production bake-off.
    BakeOff(BakeOffArgs),
}

#[derive(Args, Debug)]
struct BakeOffArgs {
    /// Path to the candidate manifest (TOML).
    #[arg(long)]
    candidates: std::path::PathBuf,
    /// Path to the corpus tarball (.tar.zst).
    #[arg(long)]
    corpus: std::path::PathBuf,
    /// Hardware tier label. `cpu` is the only target without CUDA inference;
    /// `a10` / `h100` are accepted so report metadata can record the host.
    #[arg(long, value_enum, default_value_t = HardwareTier::H100)]
    hardware: HardwareTier,
    /// Output path for the JSON report. A `.md` companion is written
    /// alongside when the extension is `.json`.
    #[arg(long)]
    report_out: std::path::PathBuf,
    /// Number of times to re-score the determinism sample. The decision rule
    /// requires at least 2; the default of 3 matches the spec.
    #[arg(long, default_value_t = 3)]
    determinism_repeat_runs: u32,
    /// Comma-separated candidate ids to skip. Useful for partial reruns when
    /// a single candidate is being investigated separately.
    #[arg(long)]
    skip_models: Option<String>,
    /// Use the deterministic MockPerplexityScorer instead of a real scorer.
    /// Reports built with this flag set carry `mock_scorer: true` and the
    /// markdown banner `[MOCK SCORER - NOT VALID FOR PRODUCTION DECISIONS]`,
    /// so they cannot be confused with real bake-off results.
    #[arg(long, default_value_t = false)]
    mock_scorer: bool,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum HardwareTier {
    A10,
    H100,
    Cpu,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let cli = Cli::parse();
    match cli.cmd.unwrap_or(Cmd::Calibrate) {
        Cmd::Calibrate => run_calibrate().await,
        Cmd::BakeOff(args) => run_bakeoff(args).await,
    }
}

/// Install a default tracing subscriber so the bake-off's `tracing::info!`
/// markers are visible without the operator having to set `RUST_LOG`. The
/// default is `info` for the bake-off modules; `RUST_LOG`, if set, fully
/// overrides. `try_init` is used so a parent process that already installed
/// a subscriber (e.g. an integration test harness) wins.
fn init_tracing() {
    let filter = std::env::var("RUST_LOG")
        .unwrap_or_else(|_| "info,trace_commons_gate_calibrate=info,trace_commons_server=info".into());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

// ---------------------------------------------------------------------------
// `calibrate` subcommand — gated behind local-gpu-models feature
// ---------------------------------------------------------------------------

#[cfg(feature = "local-gpu-models")]
async fn run_calibrate() -> anyhow::Result<()> {
    calibrate_impl::run().await
}

#[cfg(not(feature = "local-gpu-models"))]
async fn run_calibrate() -> anyhow::Result<()> {
    anyhow::bail!(
        "CalibrateMissingFeature: the `calibrate` subcommand requires the \
         `local-gpu-models` build feature; rebuild with \
         `--features local-gpu-models`"
    )
}

#[cfg(feature = "local-gpu-models")]
mod calibrate_impl {
    use std::io::{BufRead, Write};

    use anyhow::Context;
    use serde::{Deserialize, Serialize};
    use trace_commons_gate_enclave::embedder_fastembed::FastEmbedTextEmbedder;
    use trace_commons_gate_enclave::perplexity_local::{CandleDeviceKind, LocalPerplexityScorer};
    use trace_commons_gate_enclave::vector_index::{MockVectorIndex, VectorIndex};
    use trace_commons_gate_enclave::{
        EnclaveGateOrchestrator, EnclaveGateOrchestratorConfig, OrchestrationDecision,
    };
    use uuid::Uuid;

    // Env vars: same names as in trace-commons-ingest.rs. We re-declare them
    // locally (rather than depending on the binary crate) because this
    // binary lives in the same crate but we keep the dependency direction
    // clean.
    const TRACE_COMMONS_PERPLEXITY_MODEL_ID: &str = "TRACE_COMMONS_PERPLEXITY_MODEL_ID";
    const TRACE_COMMONS_PERPLEXITY_MODEL_PATH: &str = "TRACE_COMMONS_PERPLEXITY_MODEL_PATH";
    const TRACE_COMMONS_PERPLEXITY_DEVICE: &str = "TRACE_COMMONS_PERPLEXITY_DEVICE";
    const TRACE_COMMONS_PERPLEXITY_MAX_TOKENS: &str = "TRACE_COMMONS_PERPLEXITY_MAX_TOKENS";
    const TRACE_COMMONS_PERPLEXITY_TAIL_LOGPROB_CUTOFF: &str =
        "TRACE_COMMONS_PERPLEXITY_TAIL_LOGPROB_CUTOFF";
    /// Selects the candle backend for `Cmd::Calibrate`'s scorer (and for the
    /// production gate-service load in `trace-commons-ingest`). Default `"llama"`
    /// preserves back-compat with A2.1 deployments; valid values are
    /// `"llama"`, `"qwen3"`, `"gemma3"`, `"gemma4"`. Parsed via
    /// As of A2.3 this env var is **deprecated** and ignored — mistralrs
    /// auto-detects the architecture from `config.json`. If the operator
    /// still sets it, the calibrate path emits a deprecation warning at
    /// startup and continues with auto-detection.
    const TRACE_COMMONS_PERPLEXITY_MODEL_ARCH: &str = "TRACE_COMMONS_PERPLEXITY_MODEL_ARCH";
    const TRACE_COMMONS_PERPLEXITY_DEFAULT_MODEL_ID: &str = "meta-llama/Llama-3.1-8B-Instruct";
    const TRACE_COMMONS_PERPLEXITY_DEFAULT_MAX_TOKENS: usize = 16_384;
    const TRACE_COMMONS_PERPLEXITY_DEFAULT_TAIL_LOGPROB_CUTOFF: f32 = -8.0;
    const TRACE_COMMONS_EMBEDDER_MODEL_ID: &str = "TRACE_COMMONS_EMBEDDER_MODEL_ID";
    const TRACE_COMMONS_EMBEDDER_CACHE_DIR: &str = "TRACE_COMMONS_EMBEDDER_CACHE_DIR";
    const TRACE_COMMONS_EMBEDDER_MAX_TOKENS: &str = "TRACE_COMMONS_EMBEDDER_MAX_TOKENS";
    const TRACE_COMMONS_EMBEDDER_MATRYOSHKA_DIM: &str = "TRACE_COMMONS_EMBEDDER_MATRYOSHKA_DIM";
    const TRACE_COMMONS_EMBEDDER_DEFAULT_MODEL_ID: &str = "BAAI/bge-large-en-v1.5";
    const TRACE_COMMONS_EMBEDDER_DEFAULT_CACHE_DIR: &str = "/var/cache/trace-commons-embedder";
    const TRACE_COMMONS_EMBEDDER_DEFAULT_MAX_TOKENS: usize = 512;
    const TRACE_COMMONS_GATE_TOP_K: &str = "TRACE_COMMONS_GATE_TOP_K";
    const TRACE_COMMONS_GATE_POLICY_VERSION: &str = "TRACE_COMMONS_GATE_POLICY_VERSION";

    #[derive(Debug, Deserialize)]
    struct InLine {
        plaintext: String,
    }

    #[derive(Debug, Serialize)]
    struct OutLine {
        perplexity_micros: u64,
        tail_fraction_micros: u64,
        novelty_score_micros: u64,
    }

    pub async fn run() -> anyhow::Result<()> {
        // ----------------------------- env -----------------------------
        let model_id = std::env::var(TRACE_COMMONS_PERPLEXITY_MODEL_ID)
            .unwrap_or_else(|_| TRACE_COMMONS_PERPLEXITY_DEFAULT_MODEL_ID.to_string());
        let model_path = std::env::var(TRACE_COMMONS_PERPLEXITY_MODEL_PATH)
            .context("CalibrateMissingEnv: TRACE_COMMONS_PERPLEXITY_MODEL_PATH")?;
        let device_raw =
            std::env::var(TRACE_COMMONS_PERPLEXITY_DEVICE).unwrap_or_else(|_| "cuda".to_string());
        let device = CandleDeviceKind::from_env_str(&device_raw)
            .context("CalibrateBadEnv: TRACE_COMMONS_PERPLEXITY_DEVICE")?;
        let max_tokens = match std::env::var(TRACE_COMMONS_PERPLEXITY_MAX_TOKENS) {
            Ok(raw) => raw
                .trim()
                .parse::<usize>()
                .context("CalibrateBadEnv: TRACE_COMMONS_PERPLEXITY_MAX_TOKENS")?,
            Err(_) => TRACE_COMMONS_PERPLEXITY_DEFAULT_MAX_TOKENS,
        };
        anyhow::ensure!(max_tokens > 0, "CalibrateBadEnv: max_tokens_zero");
        let tail_cutoff = match std::env::var(TRACE_COMMONS_PERPLEXITY_TAIL_LOGPROB_CUTOFF) {
            Ok(raw) => raw
                .trim()
                .parse::<f32>()
                .context("CalibrateBadEnv: TRACE_COMMONS_PERPLEXITY_TAIL_LOGPROB_CUTOFF")?,
            Err(_) => TRACE_COMMONS_PERPLEXITY_DEFAULT_TAIL_LOGPROB_CUTOFF,
        };
        anyhow::ensure!(tail_cutoff.is_finite(), "CalibrateBadEnv: tail_cutoff");

        let embedder_model_id = std::env::var(TRACE_COMMONS_EMBEDDER_MODEL_ID)
            .unwrap_or_else(|_| TRACE_COMMONS_EMBEDDER_DEFAULT_MODEL_ID.to_string());
        let embedder_cache_dir = std::env::var(TRACE_COMMONS_EMBEDDER_CACHE_DIR)
            .unwrap_or_else(|_| TRACE_COMMONS_EMBEDDER_DEFAULT_CACHE_DIR.to_string());
        let embedder_max_tokens = match std::env::var(TRACE_COMMONS_EMBEDDER_MAX_TOKENS) {
            Ok(raw) => raw
                .trim()
                .parse::<usize>()
                .context("CalibrateBadEnv: TRACE_COMMONS_EMBEDDER_MAX_TOKENS")?,
            Err(_) => TRACE_COMMONS_EMBEDDER_DEFAULT_MAX_TOKENS,
        };
        anyhow::ensure!(
            embedder_max_tokens > 0,
            "CalibrateBadEnv: embedder_max_tokens_zero"
        );
        let embedder_matryoshka_dim = match std::env::var(TRACE_COMMONS_EMBEDDER_MATRYOSHKA_DIM) {
            Ok(raw) => {
                let t = raw.trim();
                if t.is_empty() {
                    None
                } else {
                    Some(
                        t.parse::<usize>()
                            .context("CalibrateBadEnv: TRACE_COMMONS_EMBEDDER_MATRYOSHKA_DIM")?,
                    )
                }
            }
            Err(_) => None,
        };
        let top_k = match std::env::var(TRACE_COMMONS_GATE_TOP_K) {
            Ok(raw) => raw
                .trim()
                .parse::<usize>()
                .context("CalibrateBadEnv: TRACE_COMMONS_GATE_TOP_K")?,
            Err(_) => 5,
        };
        anyhow::ensure!(top_k > 0, "CalibrateBadEnv: top_k_zero");
        let gate_policy_version = std::env::var(TRACE_COMMONS_GATE_POLICY_VERSION)
            .unwrap_or_else(|_| "calibrate-v1".to_string());

        // A2.3: TRACE_COMMONS_PERPLEXITY_MODEL_ARCH is deprecated.
        // mistralrs auto-detects the architecture from `config.json`.
        // We continue to honor the env var's *presence* by emitting a
        // deprecation warning so operators flip their configs, but the
        // value is otherwise ignored.
        if std::env::var(TRACE_COMMONS_PERPLEXITY_MODEL_ARCH).is_ok() {
            tracing::warn!(
                deprecated_env = TRACE_COMMONS_PERPLEXITY_MODEL_ARCH,
                "TRACE_COMMONS_PERPLEXITY_MODEL_ARCH is deprecated; mistralrs auto-detects \
                 architecture from config.json"
            );
        }

        // ----------------------------- build -----------------------------
        let scorer =
            LocalPerplexityScorer::try_new(model_id, &model_path, device, tail_cutoff, max_tokens)
                .context("CalibrateInit: LocalPerplexityScorerInitFailed")?;

        let embedder = FastEmbedTextEmbedder::try_new(
            embedder_model_id,
            &embedder_cache_dir,
            embedder_matryoshka_dim,
            embedder_max_tokens,
        )
        .await
        .context("CalibrateInit: FastEmbedTextEmbedderInitFailed")?;

        let index = MockVectorIndex::new();

        // Floors are intentionally zero — calibration is the process that
        // chooses real floors. The gate_version_hash is a stable placeholder
        // because no audit row consumes it from this binary.
        let cfg = EnclaveGateOrchestratorConfig {
            gate_policy_version,
            gate_version_hash: "sha256:calibrate".to_string(),
            perplexity_floor_micros: 0,
            tail_fraction_floor_micros: 0,
            novelty_floor_micros: 0,
            top_k,
        };
        let orchestrator = EnclaveGateOrchestrator::new(scorer, embedder, index, cfg);

        // A single calibration "tenant" so all embeddings share one index.
        let tenant_ref = "calibrate-tenant";

        // ----------------------------- stream ----------------------------
        let stdin = std::io::stdin();
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        let mut reader = stdin.lock();

        let mut line = String::new();
        loop {
            line.clear();
            let n = reader
                .read_line(&mut line)
                .context("CalibrateIo: stdin_read_failed")?;
            if n == 0 {
                break;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let in_line: InLine = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => {
                    // Skip malformed lines; calibration is best-effort.
                    eprintln!("CalibrateWarn: bad_jsonl_line_skipped");
                    continue;
                }
            };
            let decision: OrchestrationDecision = orchestrator
                .evaluate(in_line.plaintext.as_bytes(), tenant_ref)
                .context("CalibrateEvaluateFailed")?;
            let out_line = OutLine {
                perplexity_micros: decision.perplexity_micros,
                tail_fraction_micros: decision.tail_fraction_micros,
                novelty_score_micros: decision.novelty_score_micros,
            };
            let serialized =
                serde_json::to_string(&out_line).context("CalibrateIo: serialize_failed")?;
            writeln!(out, "{serialized}").context("CalibrateIo: stdout_write_failed")?;
            // The orchestrator already inserted into the index when both
            // gates passed; under zero floors, every trace passes, so the
            // index grows monotonically. That's the intended behavior:
            // later traces see earlier ones as neighbors, producing a
            // realistic within-run novelty distribution.
        }

        // Sanity check the index actually accumulated entries; if it didn't,
        // either the input was empty or something's wrong upstream.
        let _ = (Uuid::nil(), <MockVectorIndex as VectorIndex>::nearest);

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// `bake-off` subcommand
// ---------------------------------------------------------------------------

impl From<HardwareTier> for run_candidate_eval::DeviceKind {
    fn from(t: HardwareTier) -> Self {
        match t {
            // A10 / H100 hosts both run the CUDA query path; the label is
            // preserved in operator-visible logs even though the VRAM
            // measurement code only cares about CUDA vs not-CUDA.
            HardwareTier::A10 | HardwareTier::H100 => run_candidate_eval::DeviceKind::Cuda,
            HardwareTier::Cpu => run_candidate_eval::DeviceKind::NonCuda,
        }
    }
}

async fn run_bakeoff(args: BakeOffArgs) -> anyhow::Result<()> {
    let manifest = bakeoff_manifest::parse_manifest_file(&args.candidates)?;
    for w in manifest.warnings() {
        tracing::warn!(warning = %w, "bakeoff_manifest_warning");
    }
    let corpus = bakeoff_corpus::load_corpus(&args.corpus)?;
    let manifest_sha = sha256_of_file(&args.candidates)?;
    let corpus_sha = sha256_of_file(&args.corpus)?;

    let skip: std::collections::BTreeSet<String> = args
        .skip_models
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    // `--hardware=cpu` paired with a real candle scorer is unsupported:
    // the candle Llama loader needs CUDA at any reasonable model size.
    // Refuse early with a named error class before attempting any load
    // so operators get a self-explanatory diagnostic rather than a
    // generic candle failure deep in the stack.
    if matches!(args.hardware, HardwareTier::Cpu) && !args.mock_scorer {
        anyhow::bail!(
            "BakeoffCpuRequiresMockScorer: real candle scorers need \
             --hardware=a10 or --hardware=h100; rerun with --mock-scorer \
             for a CPU dry run"
        );
    }

    // Reject the real-scorer path up front when the build feature is off,
    // so we don't half-execute and write a misleading partial report.
    #[cfg(not(feature = "local-gpu-models"))]
    if !args.mock_scorer {
        anyhow::bail!(
            "BakeoffRealScorerRequiresFeature: rebuild with \
             --features local-gpu-models, or pass --mock-scorer for \
             dry runs"
        );
    }

    let device_kind: run_candidate_eval::DeviceKind = args.hardware.into();
    let total = manifest.candidates.len();
    tracing::info!(
        candidate_count = total,
        corpus_sha256 = %corpus_sha,
        manifest_sha256 = %manifest_sha,
        hardware = ?args.hardware,
        mock_scorer = args.mock_scorer,
        "bakeoff_start"
    );

    let mut results: Vec<bakeoff_report::CandidateResult> = Vec::new();

    // Helper: persist an incremental partial-report snapshot after each
    // candidate completes (success or failure). Keeps `winner_id = None`
    // and `partial = true` until the final write at end of run.
    let snapshot = |results: &[bakeoff_report::CandidateResult]| -> bakeoff_report::Report {
        bakeoff_report::Report {
            generated_at: chrono::Utc::now().to_rfc3339(),
            corpus_sha256: corpus_sha.clone(),
            manifest_sha256: manifest_sha.clone(),
            candidates: results.to_vec(),
            winner_id: None,
            decision_rule_version: 1,
            mock_scorer: args.mock_scorer,
            ctx_max_tokens: 4096,
            determinism_gate_value: bakeoff_report::DETERMINISM_GATE,
            partial: true,
        }
    };

    // Shared mock scorer for the --mock-scorer path; built lazily so the
    // real-scorer arm doesn't pay for it.
    let mock_scorer = if args.mock_scorer {
        Some(trace_commons_gate_enclave::perplexity::MockPerplexityScorer::new())
    } else {
        None
    };

    for (i, c) in manifest.candidates.iter().enumerate() {
        if skip.contains(&c.id) {
            tracing::info!(candidate_id = %c.id, "bakeoff_skip_candidate");
            continue;
        }

        tracing::info!(
            candidate_id = %c.id,
            candidate_index = i,
            total,
            "bakeoff_candidate_load_start"
        );
        let load_start = std::time::Instant::now();

        // Each candidate's load + eval runs inside a closure-y block that
        // returns Result<CandidateResult, (error_class, anyhow::Error)>.
        // A failure here turns into a placeholder row + a tracing::warn,
        // not a propagated `?`. The point of this whole change is to keep
        // already-scored candidates in the report when a later one falls
        // over during load.
        let result: Result<bakeoff_report::CandidateResult, (&'static str, anyhow::Error)> = 'eval: {
            if let Some(scorer) = mock_scorer.as_ref() {
                tracing::info!(
                    candidate_id = %c.id,
                    load_elapsed_seconds = load_start.elapsed().as_secs_f64(),
                    "bakeoff_candidate_load_done"
                );
                let score_start = std::time::Instant::now();
                match run_candidate_eval::run_candidate_eval(
                    scorer,
                    c,
                    &corpus,
                    args.determinism_repeat_runs,
                    device_kind,
                )
                .await
                {
                    Ok(r) => {
                        tracing::info!(
                            candidate_id = %c.id,
                            score_elapsed_seconds = score_start.elapsed().as_secs_f64(),
                            auc = r.discrimination_auc,
                            throughput_tps = r.throughput_tps,
                            "bakeoff_candidate_done"
                        );
                        break 'eval Ok(r);
                    }
                    Err(e) => break 'eval Err(("RunCandidateEvalFailed", e)),
                }
            }

            #[cfg(feature = "local-gpu-models")]
            {
                use trace_commons_gate_enclave::perplexity_local::{
                    CandleDeviceKind, LocalPerplexityScorer,
                };
                // Map operator-facing HardwareTier to the local-device
                // enum. The selector is informational under mistralrs
                // (which picks its compute device from build features),
                // but we keep the parameter for env-var-shape continuity.
                // The Cpu arm is unreachable because the
                // BakeoffCpuRequiresMockScorer guard above bailed.
                let candle_device = match args.hardware {
                    HardwareTier::A10 | HardwareTier::H100 => CandleDeviceKind::Cuda(0),
                    HardwareTier::Cpu => unreachable!(
                        "BakeoffCpuRequiresMockScorer guard should have refused this path"
                    ),
                };
                // Matches TRACE_COMMONS_PERPLEXITY_DEFAULT_TAIL_LOGPROB_CUTOFF
                // from the calibrate path.
                const TAIL_LOGPROB_CUTOFF: f32 = -8.0;
                let max_tokens = run_candidate_eval::ctx_for(&c.arch);
                // A2.3: arch dispatch is gone — mistralrs auto-detects the
                // architecture from `config.json`. The `CandidateArch` on
                // the candidate is informational (used for `ctx_for`); no
                // backend translation is required at this call site.
                let scorer = match LocalPerplexityScorer::try_new(
                    c.id.clone(),
                    c.path.clone(),
                    candle_device,
                    TAIL_LOGPROB_CUTOFF,
                    max_tokens,
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        break 'eval Err(("LocalPerplexityScorerLoadFailed", e));
                    }
                };
                tracing::info!(
                    candidate_id = %c.id,
                    load_elapsed_seconds = load_start.elapsed().as_secs_f64(),
                    "bakeoff_candidate_load_done"
                );
                let score_start = std::time::Instant::now();
                match run_candidate_eval::run_candidate_eval(
                    &scorer,
                    c,
                    &corpus,
                    args.determinism_repeat_runs,
                    device_kind,
                )
                .await
                {
                    Ok(r) => {
                        tracing::info!(
                            candidate_id = %c.id,
                            score_elapsed_seconds = score_start.elapsed().as_secs_f64(),
                            auc = r.discrimination_auc,
                            throughput_tps = r.throughput_tps,
                            "bakeoff_candidate_done"
                        );
                        break 'eval Ok(r);
                    }
                    Err(e) => break 'eval Err(("RunCandidateEvalFailed", e)),
                }
            }

            // No feature, no mock scorer: the early-return at top of
            // run_bakeoff already bailed; reaching here is impossible but
            // the compiler can't prove it for the `local-gpu-models = off`
            // build, so synthesize a fail-closed branch.
            #[cfg(not(feature = "local-gpu-models"))]
            {
                break 'eval Err((
                    "BakeoffRealScorerRequiresFeature",
                    anyhow::anyhow!("real scorer path reached without local-gpu-models feature"),
                ));
            }
        };

        let candidate_result = match result {
            Ok(r) => r,
            Err((class, e)) => {
                tracing::warn!(
                    candidate_id = %c.id,
                    err = %hash_err(&e),
                    error_class = class,
                    "bakeoff_candidate_failed"
                );
                bakeoff_report::CandidateResult::failed(
                    c.id.clone(),
                    run_candidate_eval::map_license(&c.license),
                    c.params_b.unwrap_or(0),
                    c.release_date_unix.unwrap_or(0),
                    class,
                )
            }
        };

        results.push(candidate_result);

        // Incremental snapshot after every candidate (success or failure).
        // Atomic-rename so a process kill mid-write cannot leave a half-
        // written report.json on disk.
        let partial_report = snapshot(&results);
        if let Err(e) = bakeoff_report::write_report_atomic(&partial_report, &args.report_out) {
            tracing::warn!(
                err = %hash_err(&e),
                error_class = "BakeoffIncrementalWriteFailed",
                "incremental report write failed; continuing"
            );
        }
    }

    // Final write: compute winner and flip partial=false. `pick_winner`
    // already excludes any candidate with `passed_determinism_gate = false`,
    // which covers every failed-load row (they're constructed with that
    // flag false), so failed candidates never win.
    let winner_id = bakeoff_report::pick_winner(&results).map(|w| w.id.clone());
    let report = bakeoff_report::Report {
        generated_at: chrono::Utc::now().to_rfc3339(),
        corpus_sha256: corpus_sha,
        manifest_sha256: manifest_sha,
        candidates: results,
        winner_id,
        decision_rule_version: 1,
        mock_scorer: args.mock_scorer,
        ctx_max_tokens: 4096,
        determinism_gate_value: bakeoff_report::DETERMINISM_GATE,
        partial: false,
    };

    bakeoff_report::write_report_atomic(&report, &args.report_out)?;
    tracing::info!(
        winner_id = ?report.winner_id,
        written_to = %args.report_out.display(),
        "bakeoff_complete"
    );
    Ok(())
}

/// Stable hash of an `anyhow::Error` so warn lines don't carry raw error
/// text that might include operator-secret material (file paths, etc).
fn hash_err(e: &anyhow::Error) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(format!("{e:?}").as_bytes());
    format!("sha256:{:x}", h.finalize())
}

/// Streaming sha256 of a file. 64 KiB read buffer; output is the canonical
/// `sha256:<hex>` label matching the corpus-loader convention.
fn sha256_of_file(path: &std::path::Path) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(path)
        .map_err(|e| anyhow::anyhow!("sha256_of_file: open {}: {e}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| anyhow::anyhow!("sha256_of_file: read {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    let digest = h.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write as _;
        write!(&mut hex, "{b:02x}").ok();
    }
    Ok(format!("sha256:{hex}"))
}

// ----------------------------- tests -----------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn defaults_to_calibrate_when_no_subcommand_given() {
        // Bare invocation with no subcommand selects the calibration path so
        // existing env-driven scripts keep working unmodified.
        let cli = Cli::parse_from(["trace-commons-gate-calibrate"]);
        assert!(matches!(cli.cmd, None));
    }

    #[test]
    fn parses_bake_off_subcommand_with_required_args() {
        let cli = Cli::parse_from([
            "trace-commons-gate-calibrate",
            "bake-off",
            "--candidates=/tmp/manifest.toml",
            "--corpus=/tmp/corpus.tar.zst",
            "--report-out=/tmp/report.json",
            "--mock-scorer",
        ]);
        match cli.cmd {
            Some(Cmd::BakeOff(args)) => {
                assert_eq!(
                    args.candidates,
                    std::path::PathBuf::from("/tmp/manifest.toml")
                );
                assert_eq!(args.corpus, std::path::PathBuf::from("/tmp/corpus.tar.zst"));
                assert_eq!(
                    args.report_out,
                    std::path::PathBuf::from("/tmp/report.json")
                );
                assert!(args.mock_scorer);
                assert_eq!(args.determinism_repeat_runs, 3);
            }
            other => panic!("expected BakeOff subcommand, got {other:?}"),
        }
    }

    // Below: the original mock-orchestration test, preserved verbatim under
    // the new module layout so the existing test suite remains green.
    mod calibrate_legacy {
        //! Per-line orchestration test using all-mock components, so this
        //! suite runs without GPU / model downloads.
        use serde::Serialize;
        use trace_commons_gate_enclave::embedder::MockEmbedder;
        use trace_commons_gate_enclave::perplexity::MockPerplexityScorer;
        use trace_commons_gate_enclave::vector_index::MockVectorIndex;
        use trace_commons_gate_enclave::{EnclaveGateOrchestrator, EnclaveGateOrchestratorConfig};

        #[derive(Serialize)]
        struct OutShape {
            perplexity_micros: u64,
            tail_fraction_micros: u64,
            novelty_score_micros: u64,
        }

        #[test]
        fn one_line_round_trip_produces_expected_jsonl_shape() {
            let orch = EnclaveGateOrchestrator::new(
                MockPerplexityScorer::new(),
                MockEmbedder::new(),
                MockVectorIndex::new(),
                EnclaveGateOrchestratorConfig::mock_default(),
            );
            let decision = orch
                .evaluate(b"hello calibration", "calibrate-tenant")
                .expect("mock evaluate");
            let out = OutShape {
                perplexity_micros: decision.perplexity_micros,
                tail_fraction_micros: decision.tail_fraction_micros,
                novelty_score_micros: decision.novelty_score_micros,
            };
            let s = serde_json::to_string(&out).expect("serialize");
            let v: serde_json::Value = serde_json::from_str(&s).expect("parse");
            assert!(
                v.get("perplexity_micros")
                    .and_then(|x| x.as_u64())
                    .is_some()
            );
            assert!(
                v.get("tail_fraction_micros")
                    .and_then(|x| x.as_u64())
                    .is_some()
            );
            assert!(
                v.get("novelty_score_micros")
                    .and_then(|x| x.as_u64())
                    .is_some()
            );
            let obj = v.as_object().expect("object");
            assert_eq!(obj.len(), 3, "exactly three calibration metrics");
        }
    }
}
