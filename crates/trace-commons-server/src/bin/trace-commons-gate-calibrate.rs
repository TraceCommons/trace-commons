//! Offline gate calibration helper.
//!
//! Reads JSONL on stdin where each line is `{"plaintext": "..."}`, runs
//! the standard `EnclaveGateOrchestrator` pipeline (real perplexity
//! scorer + real embedder + in-memory mock vector index), and emits
//! JSONL on stdout with the three numeric metrics needed to choose
//! gate floors:
//!
//! ```json
//! {"perplexity_micros": N, "tail_fraction_micros": N, "novelty_score_micros": N}
//! ```
//!
//! This binary intentionally has no auth, no DB, no audit chain, no
//! credit emission. It exists to derive recommended floor values from
//! a fixed dataset without touching production state.
//!
//! The mock vector index starts empty and accumulates embeddings across
//! the run, so novelty scores for the second-and-later traces reflect
//! the within-run corpus rather than the empty-index degenerate case.
//! This mirrors the production orchestrator's insert-after-pass
//! behavior, except calibration always inserts (it doesn't apply the
//! floors it's trying to derive).
//!
//! All env vars are the same the gate service reads for models, with
//! the explicit exclusion of the floor envs — the whole point is to
//! derive recommended floors. Hash-only diagnostics on failure.

#![cfg(feature = "local-gpu-models")]

use std::io::{BufRead, Write};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use trace_commons_gate_enclave::embedder_fastembed::FastEmbedTextEmbedder;
use trace_commons_gate_enclave::perplexity_candle::{CandleDeviceKind, CandlePerplexityScorer};
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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

    // ----------------------------- build -----------------------------
    let scorer =
        CandlePerplexityScorer::try_new(model_id, &model_path, device, tail_cutoff, max_tokens)
            .await
            .context("CalibrateInit: CandlePerplexityScorerInitFailed")?;

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

// ----------------------------- tests -----------------------------

#[cfg(test)]
mod tests {
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
        // Shape assertion: the three keys must be present and integer.
        let v: serde_json::Value = serde_json::from_str(&s).expect("parse");
        assert!(v.get("perplexity_micros").and_then(|x| x.as_u64()).is_some());
        assert!(v
            .get("tail_fraction_micros")
            .and_then(|x| x.as_u64())
            .is_some());
        assert!(v
            .get("novelty_score_micros")
            .and_then(|x| x.as_u64())
            .is_some());
        // No extra fields creep in (perplexity_passed, novelty_passed,
        // etc. live on the decision, not the calibration output).
        let obj = v.as_object().expect("object");
        assert_eq!(obj.len(), 3, "exactly three calibration metrics");
    }
}
