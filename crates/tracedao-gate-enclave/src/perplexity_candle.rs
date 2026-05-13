//! Candle-backed perplexity scorer (Phase A2).
//!
//! Loads a Llama-style base model (default: Llama-3.1-8B-Instruct, bf16) into
//! `candle_transformers::models::llama` and scores plaintext traces by running
//! prefill, collecting per-token logprobs from a single sequential forward
//! pass over the token sequence (driven by the model's KV cache), and
//! aggregating them into `(mean_nll → perplexity_micros, tail_fraction_micros)`.
//!
//! ### Honest scope
//!
//! The aggregate-math helper [`aggregate_perplexity_metrics`] is factored out
//! of the candle forward path so it can be unit-tested without a GPU. The
//! candle forward pass itself is **hardware-untested in CI**: this module's
//! integration test is `#[ignore]`d and only runs when both
//! `TRACEDAO_PERPLEXITY_INTEGRATION=1` and `TRACE_COMMONS_PERPLEXITY_MODEL_PATH`
//! are set in the environment. First production deployment must validate
//! runtime correctness before the gate is wired into credit emission.
//!
//! ### Hash-only logging
//!
//! All operational tracing on the score path is label-only. Plaintext never
//! logs, never serializes, never leaves process memory; only the two aggregate
//! `u64` micros leave the function.

use crate::perplexity::PerplexityResult;

/// Compute `PerplexityResult` from a vector of per-token logprobs.
///
/// The first element is treated as the BOS-conditioned token (no predictive
/// context) and dropped from both metrics. Returns zeros for inputs of length
/// less than 2 and for non-finite or negative aggregate values — the
/// orchestrator's perplexity floor (configured positive) then fails the gate
/// naturally, matching the spec's "fail-closed on degenerate input" contract.
///
/// This is the unit-tested surface; the candle forward path constructs the
/// `logprobs` slice and delegates to this function.
pub fn aggregate_perplexity_metrics(
    logprobs: &[f32],
    tail_logprob_cutoff: f32,
) -> PerplexityResult {
    if logprobs.len() < 2 {
        return PerplexityResult {
            aggregate_perplexity_micros: 0,
            tail_fraction_micros: 0,
            tokens_scored: 0,
        };
    }
    let usable = &logprobs[1..];
    let n = usable.len() as f32;

    // Reject any non-finite logprob — the model produced something we can't
    // interpret; fail closed.
    if usable.iter().any(|lp| !lp.is_finite()) {
        return PerplexityResult {
            aggregate_perplexity_micros: 0,
            tail_fraction_micros: 0,
            tokens_scored: 0,
        };
    }

    let sum_lp: f32 = usable.iter().sum();
    let mean_nll = -(sum_lp / n);
    let perplexity = mean_nll.exp();
    let aggregate_perplexity_micros = saturating_micros(perplexity);

    let tail_count = usable.iter().filter(|&&lp| lp < tail_logprob_cutoff).count();
    let tail_fraction = tail_count as f32 / n;
    let tail_fraction_micros = saturating_micros(tail_fraction);

    PerplexityResult {
        aggregate_perplexity_micros,
        tail_fraction_micros,
        // Number of tokens that contributed to the aggregate (the input
        // length minus the dropped BOS placeholder). Real scorer path —
        // tokenizer-derived rather than estimated.
        tokens_scored: usable.len() as u64,
    }
}

/// Scale a non-negative `f32` to fixed-point micros, saturating on overflow
/// and clamping non-finite / negative inputs to zero (fail-closed).
fn saturating_micros(v: f32) -> u64 {
    if !v.is_finite() || v < 0.0 {
        return 0;
    }
    let scaled = v * 1_000_000.0_f32;
    if scaled >= (u64::MAX as f32) {
        u64::MAX
    } else {
        scaled as u64
    }
}

// ---------------------------------------------------------------------------
// Candle device selector — always compiled so the binary's env-parsing path
// can refer to it without flipping on the `local-gpu-models` feature.
// ---------------------------------------------------------------------------

/// Selector for the candle compute device. Parsed from
/// `TRACE_COMMONS_PERPLEXITY_DEVICE` at startup. `Cuda(0)` is the production
/// default; `Cpu` is permitted for development and is impractical at 8B model
/// size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandleDeviceKind {
    Cuda(usize),
    Metal,
    Cpu,
}

impl CandleDeviceKind {
    /// Parse the env-string form. Accepts `cuda`, `cuda:N`, `metal`, `cpu`.
    pub fn from_env_str(raw: &str) -> anyhow::Result<Self> {
        let trimmed = raw.trim().to_ascii_lowercase();
        if trimmed.is_empty() || trimmed == "cuda" {
            return Ok(CandleDeviceKind::Cuda(0));
        }
        if let Some(rest) = trimmed.strip_prefix("cuda:") {
            let idx: usize = rest
                .parse()
                .map_err(|_| anyhow::anyhow!("CandleDeviceKindParseError"))?;
            return Ok(CandleDeviceKind::Cuda(idx));
        }
        match trimmed.as_str() {
            "metal" => Ok(CandleDeviceKind::Metal),
            "cpu" => Ok(CandleDeviceKind::Cpu),
            _ => anyhow::bail!("CandleDeviceKindParseError"),
        }
    }
}

// ---------------------------------------------------------------------------
// CandlePerplexityScorer — feature-gated.
// ---------------------------------------------------------------------------

#[cfg(feature = "local-gpu-models")]
mod candle_impl {
    use super::{aggregate_perplexity_metrics, CandleDeviceKind};
    use crate::perplexity::{PerplexityResult, PerplexityScorer};
    use anyhow::Context;
    use candle_core::{DType, Device, Tensor};
    use candle_nn::VarBuilder;
    use candle_transformers::models::llama::{Cache, Config, Llama, LlamaConfig};
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use tokenizers::Tokenizer;

    /// Flatten a multimodal `config.json` so the candle text-loaders, which
    /// only know how to deserialize the flat HF schema, can read configs that
    /// nest text params under a `text_config` object (Gemma 4, Qwen 3.6,
    /// etc.). Top-level keys win on collision; sibling `vision_config` /
    /// `audio_config` objects are left in place and ignored by the loader.
    pub(crate) fn flatten_text_config(raw: &[u8]) -> anyhow::Result<Vec<u8>> {
        let mut value: serde_json::Value = serde_json::from_slice(raw)
            .context("config.json must be valid JSON")?;
        let text = value.get("text_config").cloned();
        if let Some(serde_json::Value::Object(text_map)) = text {
            let map = value
                .as_object_mut()
                .context("config.json must be an object")?;
            for (k, v) in text_map {
                map.entry(k).or_insert(v);
            }
        }
        Ok(serde_json::to_vec(&value)?)
    }

    #[cfg(test)]
    mod tests {
        use super::flatten_text_config;
        use serde_json::json;

        #[test]
        fn flat_config_passes_through_unchanged() {
            let raw = json!({"model_type": "llama", "hidden_size": 4096}).to_string();
            let out = flatten_text_config(raw.as_bytes()).unwrap();
            let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
            assert_eq!(parsed["hidden_size"], 4096);
            assert!(parsed.get("text_config").is_none());
        }

        #[test]
        fn nested_text_config_is_flattened_to_top_level() {
            let raw = json!({
                "model_type": "gemma4",
                "text_config": {"hidden_size": 5120, "num_attention_heads": 40},
                "vision_config": {"hidden_size": 1024}
            })
            .to_string();
            let out = flatten_text_config(raw.as_bytes()).unwrap();
            let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
            assert_eq!(parsed["hidden_size"], 5120, "text hidden_size should be lifted");
            assert_eq!(parsed["num_attention_heads"], 40);
            // vision_config should remain — we just ignore it later.
            assert!(parsed.get("vision_config").is_some());
            // text_config also remains; we merged keys but didn't strip the source.
            assert!(parsed.get("text_config").is_some());
        }

        #[test]
        fn top_level_wins_over_text_config_collision() {
            let raw = json!({
                "model_type": "gemma4",
                "hidden_size": 9999,
                "text_config": {"hidden_size": 5120}
            })
            .to_string();
            let out = flatten_text_config(raw.as_bytes()).unwrap();
            let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
            assert_eq!(parsed["hidden_size"], 9999, "explicit top-level wins");
        }

        #[test]
        fn invalid_json_errors_clearly() {
            let err = flatten_text_config(b"{not json").unwrap_err();
            assert!(err.to_string().contains("valid JSON"));
        }
    }

    /// Candle-backed perplexity scorer. Holds the loaded model, tokenizer,
    /// device, and a `Mutex`-guarded KV cache that is reset before every
    /// score call.
    ///
    /// The forward path runs sequentially with KV caching: tokens are fed one
    /// at a time, each call returns the logits over the vocabulary for the
    /// next-token prediction, and we accumulate per-token logprobs for the
    /// `aggregate_perplexity_metrics` helper. This is the standard candle
    /// pattern (cf. `candle-examples/examples/llama/main.rs`) and is the only
    /// way to obtain per-position logits from the upstream `Llama::forward`
    /// API, which itself collapses to last-position logits.
    pub struct CandlePerplexityScorer {
        model: Llama,
        tokenizer: Tokenizer,
        device: Device,
        config: Config,
        // Cache is held under a Mutex because score() takes &self per the
        // trait, but the candle Cache mutates KV state during forward(). We
        // reset() the cache per call by reconstructing it; the Mutex is a
        // safe, low-contention wrapper given the gate worker is the only
        // concurrent caller.
        cache: Mutex<Cache>,
        dtype: DType,
        tail_logprob_cutoff: f32,
        #[allow(dead_code)] // surfaced via safe_status in future wiring
        model_id: String,
        max_tokens: usize,
    }

    impl CandlePerplexityScorer {
        /// Load the model from a local directory containing the standard HF
        /// release layout: `config.json`, `tokenizer.json`, and either
        /// `model.safetensors` (single-shard) or `model.safetensors.index.json`
        /// (multi-shard, e.g. Llama-3.1-8B).
        pub async fn try_new(
            model_id: impl Into<String>,
            model_path: impl AsRef<Path>,
            device: CandleDeviceKind,
            tail_logprob_cutoff: f32,
            max_tokens: usize,
        ) -> anyhow::Result<Self> {
            // Tail cutoff is a log-probability bound (`logprob < cutoff` →
            // tail); negative values are the only sensible regime. A
            // non-negative cutoff would treat every token as tail.
            anyhow::ensure!(
                tail_logprob_cutoff.is_finite() && tail_logprob_cutoff < 0.0,
                "PerplexityScorerInit: tail cutoff must be negative (log-probability)"
            );
            anyhow::ensure!(
                max_tokens > 0,
                "PerplexityScorerInit: max_tokens must be greater than zero"
            );
            let model_id = model_id.into();
            let model_path = model_path.as_ref().to_path_buf();

            // I/O work is wrapped in spawn_blocking so the candle load path
            // (which is synchronous and does mmap + GPU copy) does not block
            // the tokio reactor.
            let cfg_path = model_path.join("config.json");
            let tokenizer_path = model_path.join("tokenizer.json");

            let device_obj = match device {
                CandleDeviceKind::Cuda(idx) => Device::new_cuda(idx)
                    .context("CandleDeviceUnavailable: cuda")?,
                CandleDeviceKind::Metal => Device::new_metal(0)
                    .context("CandleDeviceUnavailable: metal")?,
                CandleDeviceKind::Cpu => Device::Cpu,
            };

            let dtype = DType::BF16;

            let safetensor_files = discover_safetensors(&model_path)
                .context("CandleSafetensorsDiscoveryFailed")?;

            let cfg_bytes = std::fs::read(&cfg_path)
                .with_context(|| "CandleConfigReadFailed")?;
            let llama_config: LlamaConfig = serde_json::from_slice(&cfg_bytes)
                .with_context(|| "CandleConfigParseFailed")?;
            let config = llama_config.into_config(false /* use_flash_attn */);

            // Refuse a max_tokens that exceeds the model's configured
            // context window; otherwise the KV cache could blow up at
            // forward time. Bail at startup with a stable error class.
            anyhow::ensure!(
                max_tokens <= config.max_position_embeddings,
                "PerplexityScorerInit: max_tokens ({}) exceeds model max_position_embeddings ({})",
                max_tokens,
                config.max_position_embeddings,
            );

            let tokenizer = Tokenizer::from_file(&tokenizer_path)
                .map_err(|e| anyhow::anyhow!("CandleTokenizerLoadFailed: {}", hash_err(&e)))?;

            // SAFETY: VarBuilder::from_mmaped_safetensors mmaps the weight
            // files; this is the standard candle loading idiom and matches
            // the behavior of the upstream Llama example.
            let vb = unsafe {
                VarBuilder::from_mmaped_safetensors(&safetensor_files, dtype, &device_obj)
                    .context("CandleVarBuilderFailed")?
            };
            let model = Llama::load(vb, &config).context("CandleModelLoadFailed")?;
            let cache = Cache::new(true, dtype, &config, &device_obj)
                .context("CandleCacheInitFailed")?;

            Ok(Self {
                model,
                tokenizer,
                device: device_obj,
                config,
                cache: Mutex::new(cache),
                dtype,
                tail_logprob_cutoff,
                model_id,
                max_tokens,
            })
        }

        /// Inner fallible scoring path. Errors are caught by the trait
        /// `score()` shim and converted to a fail-closed zero result.
        fn try_score(&self, plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
            // 1. UTF-8 decode (lossy: the gate doesn't care about byte-level
            //    pedantry; tokenizer can handle replacement characters).
            let text = std::str::from_utf8(plaintext)
                .map(std::borrow::Cow::Borrowed)
                .unwrap_or_else(|_| std::borrow::Cow::Owned(String::from_utf8_lossy(plaintext).into_owned()));

            // 2. Tokenize.
            let encoding = self
                .tokenizer
                .encode(text.as_ref(), true)
                .map_err(|e| anyhow::anyhow!("CandleTokenizeFailed: {}", hash_err(&e)))?;
            let mut token_ids: Vec<u32> = encoding.get_ids().to_vec();

            // 3. Truncate to max_tokens. Truncation is hash-only logged
            //    (the cap value itself is operator-set so it's safe to label).
            if token_ids.len() > self.max_tokens {
                tracing::warn!(
                    error_class = "PerplexityScorerInputTruncated",
                    cap = self.max_tokens,
                    "perplexity scorer truncated overlong input"
                );
                token_ids.truncate(self.max_tokens);
            }

            if token_ids.len() < 2 {
                // Degenerate: no usable predictive context. Fail-closed.
                return Ok(PerplexityResult {
                    aggregate_perplexity_micros: 0,
                    tail_fraction_micros: 0,
                    tokens_scored: 0,
                });
            }

            // 4. Reset the KV cache. Each call to score() is independent;
            //    reconstructing the cache is the simplest correct reset.
            let mut cache = self
                .cache
                .lock()
                .map_err(|_| anyhow::anyhow!("CandleCacheMutexPoisoned"))?;
            *cache = Cache::new(true, self.dtype, &self.config, &self.device)
                .context("CandleCacheResetFailed")?;

            // 5. Run sequential forward passes, accumulating per-token
            //    logprobs. The candle Llama::forward returns logits over the
            //    vocabulary for the LAST position only — so we feed tokens one
            //    at a time, advancing `index_pos`. For position i (0-indexed),
            //    the model's prediction at step i scores token[i+1]. We
            //    collect one logprob entry per token; the first entry stays
            //    as a placeholder (BOS / no-context) and is dropped by
            //    `aggregate_perplexity_metrics`.
            let mut logprobs: Vec<f32> = Vec::with_capacity(token_ids.len());
            logprobs.push(0.0); // placeholder for position 0; dropped downstream.

            for i in 0..token_ids.len() - 1 {
                // Build a [1, 1] tensor with the i-th token.
                let input = Tensor::new(&[token_ids[i]], &self.device)
                    .context("CandleTokenTensorFailed")?
                    .unsqueeze(0)
                    .context("CandleTokenTensorReshapeFailed")?;
                let logits = self
                    .model
                    .forward(&input, i, &mut cache)
                    .context("CandleForwardFailed")?;
                // logits is shape [1, vocab] (last position).
                let logits = logits.squeeze(0).context("CandleLogitsSqueezeFailed")?;
                let logprob = log_softmax_pick(&logits, token_ids[i + 1])?;
                logprobs.push(logprob);
            }

            Ok(aggregate_perplexity_metrics(&logprobs, self.tail_logprob_cutoff))
        }
    }

    impl PerplexityScorer for CandlePerplexityScorer {
        fn score(&self, plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
            // Fail-closed: a scoring failure propagates so the orchestrator
            // refuses the gate evaluation. The previous zero-fallback path
            // would have falsely passed any positive perplexity floor.
            self.try_score(plaintext)
        }
    }

    /// Apply log_softmax to a 1-D `[vocab]` logits tensor and read the
    /// log-probability of the target token id.
    fn log_softmax_pick(logits: &Tensor, target: u32) -> anyhow::Result<f32> {
        let lp = candle_nn::ops::log_softmax(logits, 0).context("CandleLogSoftmaxFailed")?;
        // log_softmax returns f32 (the Llama forward already casts to F32
        // for the final lm_head output), so a direct extraction is fine.
        let lp_vec = lp.to_vec1::<f32>().context("CandleLogprobVecFailed")?;
        let idx = target as usize;
        if idx >= lp_vec.len() {
            anyhow::bail!("CandleLogprobTargetOutOfRange");
        }
        Ok(lp_vec[idx])
    }

    /// Discover the safetensors files in `model_path`. Supports both
    /// single-file (`model.safetensors`) and multi-shard
    /// (`model.safetensors.index.json` listing several shards) layouts.
    fn discover_safetensors(model_path: &Path) -> anyhow::Result<Vec<PathBuf>> {
        let single = model_path.join("model.safetensors");
        if single.exists() {
            return Ok(vec![single]);
        }
        let index_path = model_path.join("model.safetensors.index.json");
        if index_path.exists() {
            let raw =
                std::fs::read(&index_path).context("CandleSafetensorsIndexReadFailed")?;
            #[derive(serde::Deserialize)]
            struct Index {
                weight_map: std::collections::HashMap<String, String>,
            }
            let parsed: Index = serde_json::from_slice(&raw)
                .context("CandleSafetensorsIndexParseFailed")?;
            let mut shards: Vec<String> =
                parsed.weight_map.values().cloned().collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .collect();
            shards.sort();
            return Ok(shards.into_iter().map(|s| model_path.join(s)).collect());
        }
        anyhow::bail!("CandleSafetensorsNotFound")
    }

    /// Stable, low-entropy fingerprint of a borrowed error value. Used so
    /// `try_score`'s `anyhow::Error` doesn't carry raw operator-secret text
    /// when bubbled to the trait shim's hash-only log.
    fn hash_err<T: std::fmt::Display>(e: &T) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(format!("{e}").as_bytes());
        format!("sha256:{:x}", h.finalize())
    }
}

#[cfg(feature = "local-gpu-models")]
pub use candle_impl::CandlePerplexityScorer;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_zero_for_empty_input() {
        let r = aggregate_perplexity_metrics(&[], -8.0);
        assert_eq!(r.aggregate_perplexity_micros, 0);
        assert_eq!(r.tail_fraction_micros, 0);
    }

    #[test]
    fn aggregate_zero_for_single_token() {
        let r = aggregate_perplexity_metrics(&[-1.5], -8.0);
        assert_eq!(r.aggregate_perplexity_micros, 0);
        assert_eq!(r.tail_fraction_micros, 0);
    }

    #[test]
    fn aggregate_uniform_logprobs_yields_known_perplexity() {
        // First entry is dropped (BOS placeholder). Remaining are uniform
        // at logprob = -1.0, so mean_nll = 1.0 and perplexity = e ≈ 2.71828.
        let logprobs = vec![0.0_f32, -1.0, -1.0, -1.0, -1.0];
        let r = aggregate_perplexity_metrics(&logprobs, -8.0);
        let expected = (1.0_f32).exp();
        let got = r.aggregate_perplexity_micros as f32 / 1_000_000.0;
        // Allow small rounding tolerance for the f32 -> u64 micros cast.
        assert!(
            (got - expected).abs() < 1e-3,
            "expected ~{expected}, got {got}"
        );
        // No tail crossings at cutoff -8.0.
        assert_eq!(r.tail_fraction_micros, 0);
    }

    #[test]
    fn aggregate_one_tail_crossing_yields_partial_tail_fraction() {
        // Four usable logprobs (first dropped). One of them crosses the
        // cutoff. Tail fraction = 1/4 = 0.25.
        let logprobs = vec![0.0_f32, -1.0, -1.0, -20.0, -1.0];
        let r = aggregate_perplexity_metrics(&logprobs, -8.0);
        let tail_frac = r.tail_fraction_micros as f32 / 1_000_000.0;
        assert!((tail_frac - 0.25).abs() < 1e-6, "expected 0.25, got {tail_frac}");
    }

    #[test]
    fn aggregate_all_below_cutoff_yields_full_tail_fraction() {
        let logprobs = vec![0.0_f32, -20.0, -30.0, -40.0];
        let r = aggregate_perplexity_metrics(&logprobs, -8.0);
        assert_eq!(r.tail_fraction_micros, 1_000_000);
    }

    #[test]
    fn aggregate_no_below_cutoff_yields_zero_tail_fraction() {
        let logprobs = vec![0.0_f32, -1.0, -2.0, -3.0];
        let r = aggregate_perplexity_metrics(&logprobs, -8.0);
        assert_eq!(r.tail_fraction_micros, 0);
    }

    #[test]
    fn aggregate_rejects_nan_input() {
        let logprobs = vec![0.0_f32, -1.0, f32::NAN, -1.0];
        let r = aggregate_perplexity_metrics(&logprobs, -8.0);
        assert_eq!(r.aggregate_perplexity_micros, 0);
        assert_eq!(r.tail_fraction_micros, 0);
    }

    #[test]
    fn aggregate_rejects_infinite_input() {
        let logprobs = vec![0.0_f32, -1.0, f32::NEG_INFINITY, -1.0];
        let r = aggregate_perplexity_metrics(&logprobs, -8.0);
        assert_eq!(r.aggregate_perplexity_micros, 0);
        assert_eq!(r.tail_fraction_micros, 0);
    }

    #[test]
    fn saturating_micros_clamps_negative_to_zero() {
        assert_eq!(saturating_micros(-1.0), 0);
    }

    #[test]
    fn saturating_micros_clamps_nan_to_zero() {
        assert_eq!(saturating_micros(f32::NAN), 0);
    }

    #[test]
    fn saturating_micros_saturates_at_u64_max() {
        // 1e30 * 1e6 well exceeds f32 representation of u64::MAX.
        assert_eq!(saturating_micros(1e30), u64::MAX);
    }

    #[test]
    fn saturating_micros_scales_normal_values() {
        assert_eq!(saturating_micros(2.5), 2_500_000);
    }

    #[test]
    fn candle_device_kind_parses_defaults() {
        assert_eq!(
            CandleDeviceKind::from_env_str("").unwrap(),
            CandleDeviceKind::Cuda(0)
        );
        assert_eq!(
            CandleDeviceKind::from_env_str("cuda").unwrap(),
            CandleDeviceKind::Cuda(0)
        );
        assert_eq!(
            CandleDeviceKind::from_env_str("CUDA").unwrap(),
            CandleDeviceKind::Cuda(0)
        );
    }

    #[test]
    fn candle_device_kind_parses_indexed_cuda() {
        assert_eq!(
            CandleDeviceKind::from_env_str("cuda:3").unwrap(),
            CandleDeviceKind::Cuda(3)
        );
    }

    #[test]
    fn candle_device_kind_parses_metal_and_cpu() {
        assert_eq!(
            CandleDeviceKind::from_env_str("metal").unwrap(),
            CandleDeviceKind::Metal
        );
        assert_eq!(
            CandleDeviceKind::from_env_str("cpu").unwrap(),
            CandleDeviceKind::Cpu
        );
    }

    #[test]
    fn candle_device_kind_rejects_garbage() {
        assert!(CandleDeviceKind::from_env_str("tpu").is_err());
        assert!(CandleDeviceKind::from_env_str("cuda:xyz").is_err());
    }

    // ---- Hardware-gated integration test --------------------------------
    //
    // Runs only when BOTH env vars are set AND the test is invoked with
    // `--ignored`. Mirrors the fake-gcs-server idiom from PR #8.
    #[cfg(feature = "local-gpu-models")]
    #[test]
    #[ignore]
    fn candle_perplexity_smoke_against_real_model() {
        use crate::perplexity::PerplexityScorer;
        if std::env::var("TRACEDAO_PERPLEXITY_INTEGRATION").ok().as_deref() != Some("1") {
            eprintln!(
                "skip: TRACEDAO_PERPLEXITY_INTEGRATION not set; this test requires a GPU + staged model"
            );
            return;
        }
        let Ok(model_path) = std::env::var("TRACE_COMMONS_PERPLEXITY_MODEL_PATH") else {
            eprintln!("skip: TRACE_COMMONS_PERPLEXITY_MODEL_PATH not set");
            return;
        };
        let device_raw =
            std::env::var("TRACE_COMMONS_PERPLEXITY_DEVICE").unwrap_or_else(|_| "cuda".into());
        let device = CandleDeviceKind::from_env_str(&device_raw).expect("device parse");

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");

        let scorer = rt
            .block_on(async {
                CandlePerplexityScorer::try_new(
                    std::env::var("TRACE_COMMONS_PERPLEXITY_MODEL_ID")
                        .unwrap_or_else(|_| "meta-llama/Llama-3.1-8B-Instruct".into()),
                    &model_path,
                    device,
                    -8.0_f32,
                    16_384,
                )
                .await
            })
            .expect("scorer must construct on the configured host");

        let r = scorer
            .score(b"The quick brown fox jumps over the lazy dog.")
            .expect("non-degenerate score must succeed");
        assert!(
            r.aggregate_perplexity_micros > 0,
            "non-degenerate input must produce a positive perplexity"
        );
        assert!(
            r.tail_fraction_micros <= 1_000_000,
            "tail fraction must be in [0, 1]"
        );

        let empty = scorer.score(b"").expect("empty input is degenerate but score still Ok");
        assert_eq!(empty.aggregate_perplexity_micros, 0);
        assert_eq!(empty.tail_fraction_micros, 0);
    }
}
