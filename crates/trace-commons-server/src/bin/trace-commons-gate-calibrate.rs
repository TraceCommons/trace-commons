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
#[path = "gate_calibrate/tail_floor.rs"]
mod tail_floor;

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
    /// Propose a value for `TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS`
    /// by joining a pilot-bootstrap sidecar JSONL against the
    /// `trace_gate_decisions` table and computing a percentile of the
    /// observed `tail_fraction_micros` distribution. Operator-only; no
    /// tenant scoping (every matching decision row contributes).
    TailFloor(tail_floor::TailFloorArgs),
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
    /// Which scorer(s) to run on this corpus. `perplexity` is the default
    /// (back-compat with A2.3c / A2.4 / A2.6 reports). `token-rarity` runs
    /// only the Phase A.5 per-token-rarity scorer. `both` runs both and
    /// emits per-candidate `metrics.perplexity` + `metrics.token_rarity`
    /// blocks so AUC curves can be compared on the same corpus.
    #[arg(long, value_enum, default_value_t = ScorerSelection::Perplexity)]
    scorer: ScorerSelection,
    /// K parameter for the per-token rarity scorer. Ignored when
    /// `--scorer perplexity`. Default 10 matches the Python prototype's
    /// default; bump only when sweeping K as part of a calibration pass.
    #[arg(long, default_value_t = 10)]
    token_rarity_k: usize,
    /// Scoring backend for the real-scorer arm. `local` keeps the existing
    /// mistralrs (CUDA) path — manifest candidates are loaded and scored on
    /// the local GPU. `near-ai` posts each scoring request to a NEAR AI Cloud
    /// `/v1/completions` direct endpoint; the per-candidate `model_path` is
    /// ignored and the configured `--near-ai-model` is used instead. Ignored
    /// under `--mock-scorer`.
    #[arg(long, value_enum, default_value_t = ScorerBackend::Local)]
    scorer_backend: ScorerBackend,
    /// Base URL of the NEAR AI direct-completions endpoint, including the
    /// `/v1` suffix and no trailing slash. Required when
    /// `--scorer-backend=near-ai`. Example:
    /// `https://qwen3-6-35b.completions.near.ai/v1`.
    #[arg(long)]
    near_ai_base_url: Option<String>,
    /// NEAR AI hosted model ID, e.g. `Qwen/Qwen3.6-35B-A3B-FP8`. Required
    /// when `--scorer-backend=near-ai`.
    #[arg(long)]
    near_ai_model: Option<String>,
    /// HTTP timeout in seconds for each NEAR AI scoring request.
    #[arg(long, default_value_t = 60)]
    near_ai_timeout_secs: u64,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum ScorerSelection {
    Perplexity,
    TokenRarity,
    Both,
}

impl ScorerSelection {
    fn use_perplexity(&self) -> bool {
        matches!(self, ScorerSelection::Perplexity | ScorerSelection::Both)
    }
    fn use_token_rarity(&self) -> bool {
        matches!(self, ScorerSelection::TokenRarity | ScorerSelection::Both)
    }
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum HardwareTier {
    A10,
    H100,
    Cpu,
}

/// Real-scorer backend selection. `Local` is the existing mistralrs (CUDA)
/// path; `NearAi` swaps in [`NearAiPerplexityScorer`], which posts to a
/// TEE-hosted vLLM and supports both the perplexity and per-token rarity
/// metrics in one HTTP round-trip.
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum ScorerBackend {
    Local,
    NearAi,
}

/// Env var carrying the NEAR AI Cloud bearer token. CLI flag intentionally
/// avoided: API keys must not appear in process listings or shell history.
#[cfg(feature = "near-ai-scorer")]
const TRACE_COMMONS_NEAR_AI_API_KEY: &str = "TRACE_COMMONS_NEAR_AI_API_KEY";

/// Set when the binary is compiled with `--features local-gpu-models-cuda`,
/// which enables mistralrs's CUDA backend. Used by the bake-off startup
/// guard to refuse `--hardware=a10` / `--hardware=h100` on CPU-only builds
/// so mistralrs cannot silently fall back to CPU inference.
#[cfg(feature = "local-gpu-models-cuda")]
const HAS_CUDA: bool = true;
#[cfg(not(feature = "local-gpu-models-cuda"))]
const HAS_CUDA: bool = false;

/// Pure guard predicate exposed for unit testing. Returns the named error
/// class label when the operator-selected hardware tier requires CUDA but
/// `has_cuda` is false; returns `None` otherwise. Mock-scorer dry runs
/// bypass the guard because they never invoke mistralrs.
fn cuda_hardware_guard(
    hardware: HardwareTier,
    mock_scorer: bool,
    has_cuda: bool,
) -> Option<&'static str> {
    if matches!(hardware, HardwareTier::A10 | HardwareTier::H100) && !mock_scorer && !has_cuda {
        Some("BakeoffCudaHardwareRequiresCudaFeature")
    } else {
        None
    }
}

#[cfg(test)]
mod cuda_guard_tests {
    use super::{HAS_CUDA, HardwareTier, cuda_hardware_guard};

    #[test]
    fn refuses_h100_without_cuda_feature() {
        assert_eq!(
            cuda_hardware_guard(HardwareTier::H100, false, false),
            Some("BakeoffCudaHardwareRequiresCudaFeature"),
        );
    }

    #[test]
    fn refuses_a10_without_cuda_feature() {
        assert_eq!(
            cuda_hardware_guard(HardwareTier::A10, false, false),
            Some("BakeoffCudaHardwareRequiresCudaFeature"),
        );
    }

    #[test]
    fn allows_cpu_without_cuda_feature() {
        assert_eq!(cuda_hardware_guard(HardwareTier::Cpu, false, false), None);
    }

    #[test]
    fn allows_h100_when_cuda_feature_present() {
        assert_eq!(cuda_hardware_guard(HardwareTier::H100, false, true), None);
    }

    #[test]
    fn allows_mock_scorer_dry_run_on_any_hardware() {
        // --mock-scorer never invokes mistralrs, so the CUDA mismatch
        // cannot bite. The guard must let dry runs through.
        assert_eq!(cuda_hardware_guard(HardwareTier::H100, true, false), None);
    }

    #[test]
    fn default_feature_build_lacks_cuda() {
        // The whole point of this guard: under the default feature set
        // (and the `local-gpu-models` CPU-only build), HAS_CUDA is false
        // so the silent-CPU-fallback footgun cannot recur. Asserted via
        // the guard predicate (which clippy doesn't flag as a constant
        // assertion the way `assert!(!HAS_CUDA)` is).
        let result = cuda_hardware_guard(HardwareTier::H100, false, HAS_CUDA);
        #[cfg(not(feature = "local-gpu-models-cuda"))]
        assert_eq!(result, Some("BakeoffCudaHardwareRequiresCudaFeature"));
        #[cfg(feature = "local-gpu-models-cuda")]
        assert_eq!(result, None);
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let cli = Cli::parse();
    match cli.cmd.unwrap_or(Cmd::Calibrate) {
        Cmd::Calibrate => run_calibrate().await,
        Cmd::BakeOff(args) => run_bakeoff(args).await,
        Cmd::TailFloor(args) => tail_floor::run(args).await,
    }
}

/// Install a default tracing subscriber so the bake-off's `tracing::info!`
/// markers are visible without the operator having to set `RUST_LOG`. The
/// default is `info` for the bake-off modules; `RUST_LOG`, if set, fully
/// overrides. `try_init` is used so a parent process that already installed
/// a subscriber (e.g. an integration test harness) wins.
fn init_tracing() {
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| {
        "info,trace_commons_gate_calibrate=info,trace_commons_server=info".into()
    });
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
            // Chunking knobs at their production defaults; calibration scores
            // through the same chunked path as the gate.
            chunk_target_tokens: 2048,
            chunk_max_tokens: 3072,
            chunk_cap: 16,
            chunk_min_tokens: 64,
            embed_insert_novelty_micros: 50_000,
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
    // Compute the preregistered no-model controls once, before any scorer or
    // candidate is loaded. They depend only on the already-resident corpus.
    let baselines = bakeoff_report::BaselineResults::from_corpus(&corpus.novel, &corpus.duplicate);
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

    // Reject the real-scorer path up front when the matching build feature
    // is off, so we don't half-execute and write a misleading partial report.
    // `--scorer-backend=local` needs `local-gpu-models`; `=near-ai` needs
    // `near-ai-scorer`. Mock dry runs bypass both.
    if !args.mock_scorer {
        match args.scorer_backend {
            ScorerBackend::Local => {
                #[cfg(not(feature = "local-gpu-models"))]
                anyhow::bail!(
                    "BakeoffRealScorerRequiresFeature: --scorer-backend=local \
                     requires --features local-gpu-models; rebuild, switch \
                     to --scorer-backend=near-ai, or pass --mock-scorer for \
                     a dry run"
                );
            }
            ScorerBackend::NearAi => {
                #[cfg(not(feature = "near-ai-scorer"))]
                anyhow::bail!(
                    "BakeoffNearAiScorerRequiresFeature: --scorer-backend=near-ai \
                     requires --features near-ai-scorer; rebuild or pass \
                     --mock-scorer for a dry run"
                );
            }
        }
    }

    // Refuse CUDA hardware selection when the binary lacks the cuda
    // backend feature. Without this guard, mistralrs silently falls
    // back to CPU on `--hardware=h100` / `--hardware=a10`, which (as
    // of 2026-05-14) burned hours of Lambda time before being aborted.
    // Fail closed with a named error class so operators see the cause.
    // NEAR AI mode does no local inference, so the CUDA guard does not
    // apply — the `--hardware` flag is purely a report-metadata label in
    // that mode.
    if args.scorer_backend == ScorerBackend::Local {
        if let Some(label) = cuda_hardware_guard(args.hardware, args.mock_scorer, HAS_CUDA) {
            anyhow::bail!(
                "{label}: --hardware={:?} requires a binary built with \
                 --features local-gpu-models-cuda; rebuild or rerun with \
                 --mock-scorer for a dry run",
                args.hardware,
            );
        }
    }

    // NEAR AI mode: validate flags + env var up front, build the scorer
    // once, reuse across candidates. The model is fixed by config; the
    // per-candidate `model_path` is intentionally ignored on this arm. A
    // mismatch between candidate.id and the configured model is logged
    // per-iteration so operators notice if they fed the wrong manifest.
    #[cfg(feature = "near-ai-scorer")]
    let near_ai_scorer: Option<trace_commons_gate_enclave::NearAiPerplexityScorer> =
        if args.scorer_backend == ScorerBackend::NearAi && !args.mock_scorer {
            let base_url = args.near_ai_base_url.clone().ok_or_else(|| {
                anyhow::anyhow!(
                    "BakeoffNearAiBaseUrlMissing: --near-ai-base-url is required \
                     when --scorer-backend=near-ai"
                )
            })?;
            let model = args.near_ai_model.clone().ok_or_else(|| {
                anyhow::anyhow!(
                    "BakeoffNearAiModelMissing: --near-ai-model is required \
                     when --scorer-backend=near-ai"
                )
            })?;
            let api_key = std::env::var(TRACE_COMMONS_NEAR_AI_API_KEY).map_err(|_| {
                anyhow::anyhow!(
                    "BakeoffNearAiApiKeyMissing: env var {} is required when \
                     --scorer-backend=near-ai",
                    TRACE_COMMONS_NEAR_AI_API_KEY
                )
            })?;
            let cfg = trace_commons_gate_enclave::NearAiScorerConfig {
                base_url,
                model,
                api_key,
                // Same tail cutoff as the local scorer for cross-backend
                // comparability of `tail_fraction_micros`.
                tail_logprob_cutoff: -8.0,
                logprobs_top_k: 5,
                timeout: std::time::Duration::from_secs(args.near_ai_timeout_secs),
            };
            Some(trace_commons_gate_enclave::NearAiPerplexityScorer::try_new(
                cfg,
            )?)
        } else {
            None
        };

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
            decision_rule_version: bakeoff_report::DECISION_RULE_VERSION,
            mock_scorer: args.mock_scorer,
            ctx_max_tokens: 4096,
            determinism_gate_value: bakeoff_report::DETERMINISM_GATE,
            baselines: baselines.clone(),
            partial: true,
        }
    };

    // Shared mock scorers for the --mock-scorer path; built lazily so the
    // real-scorer arm doesn't pay for them. Constructed per-selection so
    // `--scorer perplexity` doesn't allocate a rarity mock (and vice versa).
    let mock_perplexity = if args.mock_scorer && args.scorer.use_perplexity() {
        Some(trace_commons_gate_enclave::perplexity::MockPerplexityScorer::new())
    } else {
        None
    };
    let mock_token_rarity = if args.mock_scorer && args.scorer.use_token_rarity() {
        Some(trace_commons_gate_enclave::perplexity::MockTokenRarityScorer::new())
    } else {
        None
    };

    // Real-scorer per-token rarity is only available on `--scorer-backend=near-ai`
    // — the NEAR AI scorer impl's both `PerplexityScorer` and `TokenRarityScorer`
    // off one HTTP round-trip. The local mistralrs path still owes the A.5a
    // implementation; refuse rarity selections there with the historic error
    // class so existing operator playbooks keep working.
    if !args.mock_scorer
        && args.scorer != ScorerSelection::Perplexity
        && args.scorer_backend == ScorerBackend::Local
    {
        anyhow::bail!(
            "BakeoffRealRarityNotImplemented: real-scorer per-token rarity is \
             not wired through on --scorer-backend=local; rerun with --scorer \
             perplexity, switch to --scorer-backend=near-ai, or pair your \
             rarity scorer selection with --mock-scorer for a dry run"
        );
    }

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
            if args.mock_scorer {
                tracing::info!(
                    candidate_id = %c.id,
                    load_elapsed_seconds = load_start.elapsed().as_secs_f64(),
                    "bakeoff_candidate_load_done"
                );
                let score_start = std::time::Instant::now();
                let eval_scorers = run_candidate_eval::EvalScorers {
                    perplexity: mock_perplexity.as_ref().map(|s| {
                        s as &dyn trace_commons_gate_enclave::perplexity::PerplexityScorer
                    }),
                    token_rarity: mock_token_rarity.as_ref().map(|s| {
                        s as &dyn trace_commons_gate_enclave::perplexity::TokenRarityScorer
                    }),
                    token_rarity_k: args.token_rarity_k,
                };
                match run_candidate_eval::run_candidate_eval(
                    eval_scorers,
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

            match args.scorer_backend {
                ScorerBackend::Local => {
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
                        // Local real-scorer path is perplexity-only today; the
                        // BakeoffRealRarityNotImplemented guard above already
                        // refused rarity selections on this backend.
                        match run_candidate_eval::run_candidate_eval(
                            run_candidate_eval::EvalScorers::perplexity_only(&scorer),
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
                    // The early-return at top of run_bakeoff already bailed;
                    // synthesize a fail-closed branch so the `local-gpu-models = off`
                    // build still type-checks.
                    #[cfg(not(feature = "local-gpu-models"))]
                    {
                        break 'eval Err((
                            "BakeoffRealScorerRequiresFeature",
                            anyhow::anyhow!(
                                "local real-scorer path reached without local-gpu-models feature"
                            ),
                        ));
                    }
                }
                ScorerBackend::NearAi => {
                    #[cfg(feature = "near-ai-scorer")]
                    {
                        // Pre-built scorer from the per-run init above.
                        // `expect` is safe: the early-return guards refused
                        // the NEAR AI arm without --mock-scorer when the
                        // scorer wasn't constructed.
                        let scorer = near_ai_scorer
                            .as_ref()
                            .expect("near_ai_scorer constructed when --scorer-backend=near-ai");
                        // The configured model is fixed; the manifest's
                        // per-candidate `path` is ignored. Warn (label-only)
                        // when the candidate id doesn't match so the operator
                        // notices a mismatched manifest.
                        if c.id != args.near_ai_model.as_deref().unwrap_or_default() {
                            tracing::warn!(
                                candidate_id = %c.id,
                                error_class = "BakeoffNearAiCandidateModelMismatch",
                                "candidate id differs from --near-ai-model; \
                                 NEAR AI mode scores the configured model"
                            );
                        }
                        tracing::info!(
                            candidate_id = %c.id,
                            load_elapsed_seconds = load_start.elapsed().as_secs_f64(),
                            "bakeoff_candidate_load_done"
                        );
                        let score_start = std::time::Instant::now();
                        // NEAR AI implements both `PerplexityScorer` and
                        // `TokenRarityScorer` off one HTTP round-trip per
                        // entry; build whichever side of EvalScorers the
                        // operator selected.
                        let eval_scorers = run_candidate_eval::EvalScorers {
                            perplexity: args.scorer.use_perplexity().then_some(
                                scorer as &dyn trace_commons_gate_enclave::perplexity::PerplexityScorer,
                            ),
                            token_rarity: args.scorer.use_token_rarity().then_some(
                                scorer as &dyn trace_commons_gate_enclave::perplexity::TokenRarityScorer,
                            ),
                            token_rarity_k: args.token_rarity_k,
                        };
                        match run_candidate_eval::run_candidate_eval(
                            eval_scorers,
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
                    #[cfg(not(feature = "near-ai-scorer"))]
                    {
                        break 'eval Err((
                            "BakeoffNearAiScorerRequiresFeature",
                            anyhow::anyhow!(
                                "near-ai real-scorer path reached without near-ai-scorer feature"
                            ),
                        ));
                    }
                }
            }
        };

        let mut candidate_result = match result {
            Ok(r) => r,
            Err((class, e)) => {
                tracing::warn!(
                    candidate_id = %c.id,
                    err = %hash_err(&e),
                    error_class = class,
                    "bakeoff_candidate_failed"
                );
                run_candidate_eval::failed_candidate_result(c, class, &e)
            }
        };
        bakeoff_report::record_baseline_dominance(&mut candidate_result, &baselines);

        results.push(candidate_result);

        // Incremental snapshot after every candidate (success or failure).
        // Atomic-rename so a process kill mid-write cannot leave a half-
        // written report.json on disk.
        let partial_report = snapshot(&results);
        // Fail the run immediately. Continuing could leave a stale complete
        // report from an earlier run at this authoritative path.
        bakeoff_report::write_report_atomic(&partial_report, &args.report_out)
            .map_err(|e| anyhow::anyhow!("BakeoffIncrementalWriteFailed: {}", hash_err(&e)))?;
    }

    // Final write: compute winner and flip partial=false. `pick_winner`
    // already excludes any candidate with `passed_determinism_gate = false`,
    // which covers every failed-load row (they're constructed with that
    // flag false), so failed candidates never win.
    let winner_id = bakeoff_report::pick_winner(&results, &baselines).map(|w| w.id.clone());
    let report = bakeoff_report::Report {
        generated_at: chrono::Utc::now().to_rfc3339(),
        corpus_sha256: corpus_sha,
        manifest_sha256: manifest_sha,
        candidates: results,
        winner_id,
        decision_rule_version: bakeoff_report::DECISION_RULE_VERSION,
        mock_scorer: args.mock_scorer,
        ctx_max_tokens: 4096,
        determinism_gate_value: bakeoff_report::DETERMINISM_GATE,
        baselines,
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
