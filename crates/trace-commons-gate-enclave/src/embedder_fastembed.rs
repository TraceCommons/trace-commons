//! fastembed-rs-backed `Embedder` (Phase A3).
//!
//! Loads a configured sentence-embedding model via `fastembed::TextEmbedding`
//! (ONNX Runtime under the hood) and returns L2-normalized `Vec<f32>` vectors
//! for use by the orchestrator's novelty half. Default model is
//! `BAAI/bge-large-en-v1.5` (1024 dims).
//!
//! ### Honest scope
//!
//! The [`l2_normalize_in_place`] helper is factored out so the unit-test
//! surface does not depend on fastembed or ONNX Runtime. The full inference
//! pipeline is hardware-untested in CI: the integration test below is
//! `#[ignore]`d and only runs when `TRACE_COMMONS_EMBEDDER_INTEGRATION=1` is set.
//! First production deployment must validate runtime correctness before the
//! gate drives credit emission.
//!
//! ### Hash-only logging
//!
//! `model_id` is operator-configured and safe to log as a label. Plaintext
//! never logs, never serializes, never leaves process memory beyond the
//! orchestrator → vector index handoff.

/// L2-normalize a vector in place. Returns an error if the vector contains
/// any non-finite element or if the resulting norm is below an
/// `f32::EPSILON * len()` epsilon (which would imply a vector that is
/// numerically indistinguishable from zero — the gate cannot rely on its
/// direction).
///
/// Public so it can be unit-tested without involving fastembed.
pub fn l2_normalize_in_place(v: &mut [f32]) -> anyhow::Result<()> {
    if v.iter().any(|x| !x.is_finite()) {
        anyhow::bail!("EmbedderNonFiniteVector: input contains NaN or Inf");
    }
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    let epsilon = f32::EPSILON * (v.len().max(1) as f32);
    // Inputs are guaranteed finite above, so a direct comparison is safe and
    // equivalent to the prior `!(norm > epsilon)` form.
    if norm <= epsilon {
        anyhow::bail!("EmbedderDegenerateVector: norm {norm} not above epsilon {epsilon}");
    }
    for x in v.iter_mut() {
        *x /= norm;
    }
    Ok(())
}

#[cfg(test)]
mod normalize_tests {
    use super::*;

    fn approx_eq(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn known_input_three_four() {
        let mut v = vec![3.0_f32, 4.0_f32];
        l2_normalize_in_place(&mut v).unwrap();
        assert!(approx_eq(v[0], 0.6), "got {}", v[0]);
        assert!(approx_eq(v[1], 0.8), "got {}", v[1]);
    }

    #[test]
    fn zero_vector_is_rejected() {
        // Degenerate input: norm below epsilon — cannot fail-closed by
        // pretending the vector has a direction.
        let mut v = vec![0.0_f32; 8];
        assert!(l2_normalize_in_place(&mut v).is_err());
    }

    #[test]
    fn single_positive_element() {
        let mut v = vec![7.5_f32];
        l2_normalize_in_place(&mut v).unwrap();
        assert!(approx_eq(v[0], 1.0), "got {}", v[0]);
    }

    #[test]
    fn single_negative_element() {
        let mut v = vec![-2.25_f32];
        l2_normalize_in_place(&mut v).unwrap();
        assert!(approx_eq(v[0], -1.0), "got {}", v[0]);
    }

    #[test]
    fn already_unit_norm_idempotent() {
        let mut v = vec![0.6_f32, 0.8_f32];
        l2_normalize_in_place(&mut v).unwrap();
        assert!(approx_eq(v[0], 0.6));
        assert!(approx_eq(v[1], 0.8));
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-6, "got {norm}");
    }

    #[test]
    fn rejects_nan_input() {
        let mut v = vec![1.0_f32, f32::NAN, 2.0_f32];
        let err = l2_normalize_in_place(&mut v).expect_err("NaN must be rejected");
        assert!(format!("{err}").contains("EmbedderNonFiniteVector"));
    }

    #[test]
    fn rejects_inf_input() {
        let mut v = vec![1.0_f32, f32::INFINITY, 2.0_f32];
        let err = l2_normalize_in_place(&mut v).expect_err("Inf must be rejected");
        assert!(format!("{err}").contains("EmbedderNonFiniteVector"));
    }

    #[test]
    fn empty_slice_is_rejected() {
        let mut v: Vec<f32> = Vec::new();
        // norm = 0, below any epsilon, so we reject. The caller should not
        // be feeding empty vectors into the embedder normalization helper.
        assert!(l2_normalize_in_place(&mut v).is_err());
    }
}

// ---------------------------------------------------------------------------
// FastEmbedTextEmbedder — feature-gated on `local-gpu-models`.
// ---------------------------------------------------------------------------

#[cfg(any(feature = "local-gpu-models", feature = "near-ai-scorer"))]
mod fastembed_impl {
    use super::l2_normalize_in_place;
    use crate::embedder::Embedder;
    use anyhow::Context;
    use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
    use std::path::Path;
    use std::str::FromStr;
    use std::sync::Mutex;

    /// fastembed-rs-backed embedder.
    ///
    /// The underlying `TextEmbedding::embed` requires `&mut self`, so the
    /// model is held under a `Mutex`. The gate worker is the only concurrent
    /// caller in practice, so contention is negligible.
    ///
    /// Output dimension is determined by the configured model
    /// (`BGE-large-en-v1.5` = 1024). If `matryoshka_truncate_dim` is set, the
    /// vector is sliced to the first N dimensions and re-normalized before
    /// return.
    pub struct FastEmbedTextEmbedder {
        model: Mutex<TextEmbedding>,
        model_id: String,
        output_dim: usize,
        matryoshka_truncate_dim: Option<usize>,
        #[allow(dead_code)] // recorded for safe_status; tokenizer enforces ctx
        max_tokens: usize,
    }

    impl FastEmbedTextEmbedder {
        /// Construct a new embedder. Loads model files from `cache_dir`
        /// (downloading from HuggingFace on first run via `hf-hub`).
        ///
        /// `model_id` accepts either a fastembed `EmbeddingModel` Display
        /// string (e.g. `"BAAI/bge-large-en-v1.5"`) or the enum-name form
        /// (`"BGELargeENV15"`). On parse failure, falls back to the BGE-large
        /// default.
        ///
        /// `max_tokens` is recorded but actual token-aware truncation is
        /// delegated to fastembed's tokenizer (which enforces the model's
        /// nominal context window). The field exists for safe_status
        /// reporting and future char-level prefiltering if needed.
        ///
        /// Construction is async so it composes with the gate-service
        /// binary's async startup path. (Sibling
        /// `LocalPerplexityScorer::try_new` is sync as of A2.3 because
        /// mistralrs is driven from an owned tokio runtime inside the
        /// scorer itself.) The underlying fastembed call is sync; we wrap with
        /// `tokio::task::spawn_blocking` to keep the executor responsive
        /// while ONNX initializes.
        pub async fn try_new(
            model_id: impl Into<String>,
            cache_dir: impl AsRef<Path>,
            matryoshka_truncate_dim: Option<usize>,
            max_tokens: usize,
        ) -> anyhow::Result<Self> {
            anyhow::ensure!(max_tokens > 0, "EmbedderMaxTokensMustBePositive");
            if let Some(d) = matryoshka_truncate_dim {
                anyhow::ensure!(d > 0, "EmbedderMatryoshkaDimMustBePositive");
            }

            let model_id = model_id.into();
            let cache_dir_buf = cache_dir.as_ref().to_path_buf();
            // Resolve the model id through a deliberate alias map (HF repo
            // id → fastembed `model_code`) plus enum-name fallback. Refuse
            // unrecognized ids at startup; the previous silent fallback to
            // BGE-large could mask a typo and stamp every audit row with
            // the wrong gate version.
            let model_enum = parse_embedding_model(&model_id).with_context(|| {
                format!("EmbedderModelIdUnrecognized: {}", redact_label(&model_id))
            })?;

            // Determine the model's native output dimension from fastembed's
            // metadata. We do this before constructing the model so that
            // `output_dim` is stable even if downstream truncation is in
            // play.
            let native_dim = TextEmbedding::list_supported_models()
                .into_iter()
                .find(|info| info.model == model_enum)
                .map(|info| info.dim)
                .with_context(|| {
                    format!("EmbedderModelInfoMissing: {}", redact_label(&model_id))
                })?;

            let output_dim = matryoshka_truncate_dim.unwrap_or(native_dim);
            anyhow::ensure!(
                matryoshka_truncate_dim
                    .map(|d| d <= native_dim)
                    .unwrap_or(true),
                "EmbedderMatryoshkaDimExceedsNative"
            );

            let model_enum_for_load = model_enum;
            let cache_dir_for_load = cache_dir_buf.clone();
            let max_tokens_for_load = max_tokens;
            let model = tokio::task::spawn_blocking(move || {
                // `with_max_length` enforces token-level truncation inside
                // the fastembed tokenizer; without it, oversized inputs run
                // through the model at native context length and the
                // operator-configured `max_tokens` bound is ignored.
                let opts = InitOptions::new(model_enum_for_load)
                    .with_cache_dir(cache_dir_for_load)
                    .with_max_length(max_tokens_for_load)
                    .with_show_download_progress(false);
                TextEmbedding::try_new(opts)
            })
            .await
            .context("EmbedderInitJoinError")?
            .context("EmbedderInitFailed")?;

            Ok(Self {
                model: Mutex::new(model),
                model_id,
                output_dim,
                matryoshka_truncate_dim,
                max_tokens,
            })
        }

        /// Operator-safe label for status surfaces. Returns the configured
        /// `model_id` as-is (it's already operator-controlled and contains no
        /// secrets).
        pub fn model_id(&self) -> &str {
            &self.model_id
        }

        /// Configured output dimensionality (after optional matryoshka
        /// truncation).
        pub fn output_dim(&self) -> usize {
            self.output_dim
        }

        /// Internal helper returning a `Result`. Errors propagate through
        /// the `Embedder` trait so the orchestrator refuses the evaluation
        /// rather than silently emitting a zero vector that the novelty
        /// math would interpret as "maximally novel".
        fn try_embed(&self, plaintext: &[u8]) -> anyhow::Result<Vec<f32>> {
            // Decode lossily. The trace is already redacted upstream and
            // the embedder tokenizer handles odd whitespace fine.
            let text = String::from_utf8_lossy(plaintext).into_owned();
            if text.trim().is_empty() {
                anyhow::bail!("EmbedderEmptyInput: empty plaintext rejected");
            }

            let mut model = self
                .model
                .lock()
                .map_err(|_| anyhow::anyhow!("EmbedderMutexPoisoned"))?;
            // fastembed handles tokenizer-aware truncation internally up to
            // the configured `max_length`. batch_size=Some(1) matches the
            // one-trace-per-call shape.
            let mut embeddings = model
                .embed(vec![text], Some(1))
                .context("EmbedderInferenceFailed")?;
            drop(model);

            let mut v = embeddings
                .pop()
                .context("EmbedderInferenceReturnedNoVectors")?;

            // Optional matryoshka truncation.
            if let Some(d) = self.matryoshka_truncate_dim {
                anyhow::ensure!(
                    v.len() >= d,
                    "EmbedderDimMismatch: native vector shorter than matryoshka cap"
                );
                v.truncate(d);
            }

            anyhow::ensure!(
                v.len() == self.output_dim,
                "EmbedderDimMismatch: expected {}, got {}",
                self.output_dim,
                v.len()
            );

            l2_normalize_in_place(&mut v).context("EmbedderNormalizationFailed")?;
            Ok(v)
        }
    }

    impl Embedder for FastEmbedTextEmbedder {
        fn embed(&self, plaintext: &[u8]) -> anyhow::Result<Vec<f32>> {
            // Fail-closed: any inference / normalization failure refuses the
            // evaluation. The previous zero-vector fallback would have made
            // every degenerate trace look maximally novel.
            self.try_embed(plaintext)
        }
    }

    /// Explicit alias map from common HuggingFace repo ids to fastembed
    /// `model_code` strings. fastembed's `list_supported_models` already
    /// stores a `model_code` per variant, but the upstream values use the
    /// Xenova mirrors for some entries; operators are likelier to configure
    /// the canonical HF id (e.g. `BAAI/bge-large-en-v1.5`). This table is
    /// the only place to add a new alias.
    fn repo_id_alias(model_id: &str) -> Option<&'static str> {
        match model_id {
            "BAAI/bge-large-en-v1.5" => Some("Xenova/bge-large-en-v1.5"),
            "BAAI/bge-base-en-v1.5" => Some("Xenova/bge-base-en-v1.5"),
            "BAAI/bge-small-en-v1.5" => Some("Xenova/bge-small-en-v1.5"),
            _ => None,
        }
    }

    /// Parse a configured `model_id` string into a fastembed
    /// `EmbeddingModel`. Resolution order:
    ///   1. The explicit `repo_id_alias` table.
    ///   2. Direct `ModelInfo::model_code` match against
    ///      `list_supported_models`.
    ///   3. The `EmbeddingModel::from_str` enum-name form.
    ///
    /// Returns `Err` when none match. The previous silent fallback to
    /// `BGELargeENV15` is gone — a typo would otherwise stamp every audit
    /// row with the wrong gate version while logging only a warning.
    fn parse_embedding_model(model_id: &str) -> anyhow::Result<EmbeddingModel> {
        let canonical = repo_id_alias(model_id).unwrap_or(model_id);
        for info in TextEmbedding::list_supported_models() {
            if info.model_code == canonical {
                return Ok(info.model);
            }
        }
        if let Ok(m) = EmbeddingModel::from_str(model_id) {
            return Ok(m);
        }
        anyhow::bail!("unrecognized embedder model id")
    }

    /// Truncate a string for log output. `model_id` is operator-configured
    /// and safe, but this keeps log lines bounded.
    fn redact_label(s: &str) -> String {
        if s.len() > 64 {
            format!("{}...", &s[..64])
        } else {
            s.to_string()
        }
    }

    // ----- Integration test (ignored, env-gated) -------------------------

    #[cfg(test)]
    mod integration_tests {
        use super::*;

        fn cosine_sim(a: &[f32], b: &[f32]) -> f32 {
            assert_eq!(a.len(), b.len());
            a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
        }

        /// Smoke + determinism + similarity-ordering. Behind `#[ignore]` and
        /// `TRACE_COMMONS_EMBEDDER_INTEGRATION=1`. Downloads BGE-large on first
        /// run.
        #[tokio::test]
        #[ignore]
        async fn fastembed_smoke_determinism_and_similarity() {
            if std::env::var("TRACE_COMMONS_EMBEDDER_INTEGRATION")
                .ok()
                .as_deref()
                != Some("1")
            {
                eprintln!("skipping: set TRACE_COMMONS_EMBEDDER_INTEGRATION=1 to run");
                return;
            }
            let cache_dir =
                std::env::var("TRACE_COMMONS_EMBEDDER_CACHE_DIR").unwrap_or_else(|_| {
                    std::env::temp_dir()
                        .join("trace-commons-embedder-it")
                        .to_string_lossy()
                        .into_owned()
                });
            let _ = std::fs::create_dir_all(&cache_dir);

            let embedder =
                FastEmbedTextEmbedder::try_new("BAAI/bge-large-en-v1.5", &cache_dir, None, 512)
                    .await
                    .expect("embedder constructs");

            // (1) Smoke: vector is unit-normalized and has the expected dim.
            let v = embedder
                .embed(b"the quick brown fox jumps over the lazy dog")
                .expect("embed must succeed");
            assert_eq!(v.len(), embedder.output_dim());
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!((norm - 1.0).abs() < 1e-3, "expected unit norm, got {norm}");

            // (2) Determinism: same input → exact equality.
            let v2 = embedder
                .embed(b"the quick brown fox jumps over the lazy dog")
                .expect("embed must succeed");
            assert_eq!(v, v2, "embedder must be deterministic");

            // (3) Similarity ordering: paraphrase pair beats mixed pair.
            let a = embedder
                .embed(b"the cat sat on the mat in the warm afternoon sun")
                .expect("embed");
            let b = embedder
                .embed(b"a cat was resting on a rug during the warm afternoon")
                .expect("embed");
            let c = embedder
                .embed(b"quantum chromodynamics predicts color confinement at low energies")
                .expect("embed");
            let sim_paraphrase = cosine_sim(&a, &b);
            let sim_mixed = cosine_sim(&a, &c);
            assert!(
                sim_paraphrase > sim_mixed,
                "paraphrase sim {sim_paraphrase} should exceed mixed sim {sim_mixed}"
            );
        }
    }
}

#[cfg(any(feature = "local-gpu-models", feature = "near-ai-scorer"))]
pub use fastembed_impl::FastEmbedTextEmbedder;
