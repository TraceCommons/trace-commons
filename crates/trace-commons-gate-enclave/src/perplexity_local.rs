// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Local perplexity scorer (Phase A2.3).
//!
//! Loads a base language model via [`mistralrs`] and scores plaintext traces
//! by running a single forward pass with `return_raw_logits = true`,
//! aggregating per-token logprobs over the realized next-token sequence. The
//! model architecture is auto-detected by mistralrs from `config.json` —
//! there is no per-arch dispatch on our side anymore (A2.2's `ScorerBackend`
//! and `BackendArch` enums are removed in this slice).
//!
//! ### Honest scope
//!
//! The aggregate-math helper [`aggregate_perplexity_metrics`] stays factored
//! out so it can be unit-tested without a GPU. The mistralrs forward path
//! itself is **hardware-untested in CI**: this module's integration test is
//! `#[ignore]`d and only runs when both
//! `TRACE_COMMONS_PERPLEXITY_INTEGRATION=1` and
//! `TRACE_COMMONS_PERPLEXITY_MODEL_PATH` are set in the environment. First
//! production deployment must validate runtime correctness before the gate is
//! wired into credit emission.
//!
//! ### Hash-only logging
//!
//! All operational tracing on the score path is label-only. Plaintext never
//! logs, never serializes, never leaves process memory; only the two
//! aggregate `u64` micros leave the function.

use crate::perplexity::PerplexityResult;

/// Compute `PerplexityResult` from a vector of per-token logprobs.
///
/// The first element is treated as the BOS-conditioned token (no predictive
/// context) and dropped from both metrics. Returns zeros for inputs of length
/// less than 2 and for non-finite or negative aggregate values — the
/// orchestrator's perplexity floor (configured positive) then fails the gate
/// naturally, matching the spec's "fail-closed on degenerate input" contract.
///
/// This is the unit-tested surface; the mistralrs forward path constructs the
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

    let tail_count = usable
        .iter()
        .filter(|&&lp| lp < tail_logprob_cutoff)
        .count();
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

/// Compute the per-token rarity metric in fixed-point micros.
///
/// Per-token rarity is the Phase A.5 candidate replacement metric for
/// aggregate perplexity. It picks the `K` rarest tokens (lowest log-prob =
/// most surprising) from the trace and returns `exp(-mean(K rarest logprobs))`
/// — same exponent-of-negative-mean shape as perplexity, but restricted to
/// the rare-token tail rather than the whole sequence. Higher value means
/// the trace contains more genuinely-rare tokens, which is the novelty
/// signal the gate cares about.
///
/// The first element of `logprobs` is treated as the BOS-conditioned token
/// (no predictive context) and dropped, matching
/// [`aggregate_perplexity_metrics`]'s convention. `k` is capped at the
/// number of usable tokens; `k == 0` collapses to zero (the caller passed
/// a meaningless K).
///
/// Returns 0 for empty / single-token inputs and for non-finite inputs —
/// fail-closed, parallel to the aggregate helper.
///
/// This function is the unit-tested surface; the mistralrs forward path
/// builds the same `logprobs` slice that feeds the aggregate helper and
/// delegates to this function for the rarity column.
pub fn per_token_rarity_micros(logprobs: &[f32], k: usize) -> u64 {
    if logprobs.len() < 2 || k == 0 {
        return 0;
    }
    let usable = &logprobs[1..];
    if usable.iter().any(|lp| !lp.is_finite()) {
        return 0;
    }
    let k_eff = k.min(usable.len());
    // Sort a local copy ascending; the K lowest entries are the K rarest.
    // Bake-off traces are at most a few thousand tokens, so a full sort is
    // fine; partial-sort is an avoidable complexity here.
    let mut sorted: Vec<f32> = usable.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let rarest = &sorted[..k_eff];
    let mean_lp: f32 = rarest.iter().sum::<f32>() / (k_eff as f32);
    let mean_nll = -mean_lp;
    let rarity = mean_nll.exp();
    saturating_micros(rarity)
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
// Local-device selector — always compiled so the binary's env-parsing path
// can refer to it without flipping on the `local-gpu-models` feature.
// ---------------------------------------------------------------------------

/// Selector for the local compute device. Parsed from
/// `TRACE_COMMONS_PERPLEXITY_DEVICE` at startup. `Cuda(0)` is the production
/// default; `Cpu` is permitted for development and is impractical at 8B model
/// size.
///
/// The type name keeps the historical `Candle` prefix for back-compat of
/// operator-facing env-var documentation; mistralrs reads the same concept
/// and the parser is byte-identical to A2.2's.
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
// LocalPerplexityScorer — feature-gated.
// ---------------------------------------------------------------------------

#[cfg(feature = "local-gpu-models")]
mod local_impl {
    use super::{CandleDeviceKind, aggregate_perplexity_metrics};
    use crate::perplexity::{PerplexityResult, PerplexityScorer};
    use anyhow::Context;
    use mistralrs::{
        Constraint, MistralRs, Model, ModelBuilder, NormalRequest, Request, RequestMessage,
        ResponseOk, SamplingParams,
    };
    use std::path::Path;
    use std::sync::Mutex;
    use std::thread;
    use tokio::sync::mpsc::channel;

    /// mistralrs-backed perplexity scorer. Holds the loaded model (wrapped
    /// in a `Mutex` because mistralrs's forward path mutates internal KV
    /// state across calls), an owned single-thread tokio runtime, and per-
    /// call config (tail cutoff, max tokens). The public `score()` API is
    /// arch-agnostic — mistralrs dispatches internally based on
    /// `config.json`'s `model_type`.
    ///
    /// The scorer owns its own runtime so the sync
    /// [`PerplexityScorer::score`] trait method can bridge to mistralrs's
    /// async API via `runtime.block_on(self.async_score(...))`. This
    /// pattern decouples the scorer from the caller's runtime flavor — in
    /// particular it does not require a multi-thread runtime, which the
    /// in-crate test (and the Slice 0 PoC) intentionally do not use.
    ///
    /// `tokio::task::block_in_place` was tried first and rejected: it
    /// panics inside a current-thread runtime, which the integration test
    /// and most callers use.
    /// Internal scorer state owned by the dispatch thread.
    struct ScorerInner {
        model: Mutex<Model>,
        tail_logprob_cutoff: f32,
        #[allow(dead_code)]
        model_id: String,
        max_tokens: usize,
    }

    /// A job dispatched to the scorer thread. Each job carries the
    /// plaintext to score and a oneshot sender for the result. The
    /// dispatch thread owns the tokio runtime + the mistralrs model
    /// (the only thread that ever calls `runtime.block_on` and the only
    /// thread that ever touches the model lock). This isolates the
    /// mistralrs async I/O from whatever runtime / runtime flavor the
    /// caller of `score()` is running on — fixing the
    /// `"Cannot start a runtime from within a runtime"` panic that would
    /// otherwise fire when the production gate-service (under
    /// `#[tokio::main]`) calls `try_new` or `score`.
    enum Job {
        Score {
            plaintext: Vec<u8>,
            reply: std::sync::mpsc::Sender<anyhow::Result<PerplexityResult>>,
        },
        Shutdown,
    }

    pub struct LocalPerplexityScorer {
        /// Sender into the dispatch thread that owns the runtime + model.
        job_tx: std::sync::mpsc::Sender<Job>,
        /// Join handle for the dispatch thread, taken at shutdown.
        worker: Mutex<Option<thread::JoinHandle<()>>>,
    }

    impl Drop for LocalPerplexityScorer {
        fn drop(&mut self) {
            let _ = self.job_tx.send(Job::Shutdown);
            if let Some(handle) = self.worker.lock().ok().and_then(|mut g| g.take()) {
                let _ = handle.join();
            }
        }
    }

    impl LocalPerplexityScorer {
        /// Load the model via mistralrs's [`ModelBuilder`] from a local
        /// directory path containing the HF release layout (`config.json`,
        /// `tokenizer.json`, safetensors). mistralrs auto-detects the
        /// architecture from `config.json` — no `arch:` parameter at any
        /// layer (the A2.2 `BackendArch` enum is removed in this slice).
        ///
        /// `try_new` is **synchronous** even though mistralrs is async.
        /// It spawns a dedicated OS thread that owns a single-thread tokio
        /// runtime; the runtime is the ONLY runtime that touches the
        /// mistralrs model. The scorer holds a `mpsc::Sender<Job>` into
        /// that thread; the sync [`PerplexityScorer::score`] trait method
        /// dispatches a job and blocks on a oneshot reply.
        ///
        /// Why a dedicated thread (not just an owned `Runtime` in the
        /// caller thread): the production gate-service runs under
        /// `#[tokio::main]`, and the bake-off binary runs under
        /// `#[tokio::main]`. Calling `runtime.block_on(...)` from within
        /// any existing tokio runtime panics with `"Cannot start a runtime
        /// from within a runtime"`. The dedicated thread has no tokio
        /// context, so its `block_on` is always legal. This is the
        /// resolution of the open question in the spec around the async/
        /// sync bridge.
        ///
        /// Callers must NOT `.await` this constructor; A2.2's `await` on
        /// `CandlePerplexityScorer::try_new` is dropped at every call
        /// site in this slice.
        ///
        /// `_device` is accepted for env-var-shape compatibility but
        /// ignored: mistralrs picks a device automatically based on its
        /// own build features (CUDA, Metal, CPU). Operators who need a
        /// specific device flip the build feature, not this parameter.
        pub fn try_new(
            model_id: impl Into<String>,
            model_path: impl AsRef<Path>,
            _device: CandleDeviceKind,
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
            let model_path = model_path.as_ref();
            let model_path_str = model_path
                .to_str()
                .context("PerplexityScorerInit: non-utf8 model_path")?
                .to_string();

            // Spin up the dispatch thread. It builds its own runtime + the
            // mistralrs model on-thread, ships the load result back via a
            // oneshot, then enters the job loop.
            let (job_tx, job_rx) = std::sync::mpsc::channel::<Job>();
            let (init_tx, init_rx) = std::sync::mpsc::channel::<anyhow::Result<()>>();
            let model_id_for_thread = model_id.clone();
            let handle = thread::Builder::new()
                .name("local-perplexity-scorer".into())
                .spawn(move || {
                    let rt = match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .context("LocalPerplexityScorerRuntimeInitFailed")
                    {
                        Ok(rt) => rt,
                        Err(e) => {
                            let _ = init_tx.send(Err(e));
                            return;
                        }
                    };
                    let model = match rt
                        .block_on(async { ModelBuilder::new(model_path_str).build().await })
                    {
                        Ok(m) => m,
                        Err(e) => {
                            let _ = init_tx.send(Err(e).context("LocalPerplexityScorerLoadFailed"));
                            return;
                        }
                    };
                    let inner = ScorerInner {
                        model: Mutex::new(model),
                        tail_logprob_cutoff,
                        model_id: model_id_for_thread,
                        max_tokens,
                    };
                    let _ = init_tx.send(Ok(()));
                    // Job loop.
                    while let Ok(job) = job_rx.recv() {
                        match job {
                            Job::Shutdown => break,
                            Job::Score { plaintext, reply } => {
                                let out = rt.block_on(inner.async_score(&plaintext));
                                let _ = reply.send(out);
                            }
                        }
                    }
                })
                .context("LocalPerplexityScorerThreadSpawnFailed")?;

            // Wait for the worker to either complete model init or fail.
            match init_rx.recv() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    // Thread already exited with the error; reap.
                    let _ = handle.join();
                    return Err(e);
                }
                Err(_) => {
                    let _ = handle.join();
                    anyhow::bail!("LocalPerplexityScorerInitChannelClosed");
                }
            }

            Ok(Self {
                job_tx,
                worker: Mutex::new(Some(handle)),
            })
        }
    }

    impl ScorerInner {
        async fn async_score(&self, plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
            // 1. UTF-8 decode (lossy: the gate doesn't care about byte-level
            //    pedantry; tokenizer can handle replacement characters).
            let text = match std::str::from_utf8(plaintext) {
                Ok(s) => s.to_string(),
                Err(_) => String::from_utf8_lossy(plaintext).into_owned(),
            };

            let model_guard = self
                .model
                .lock()
                .map_err(|_| anyhow::anyhow!("LocalPerplexityScorerMutexPoisoned"))?;

            // 2. Tokenize. mistralrs's tokenize API takes an
            //    `Either<Vec<u32>, String>` via the `either` crate;
            //    `Right(text)` for raw text input.
            let tokens = model_guard
                .tokenize(either::Either::Right(text), None, false, false, None)
                .await
                .map_err(|e| {
                    anyhow::anyhow!("LocalPerplexityScorerTokenizeFailed: {}", hash_err(&e))
                })?;

            // 3. Truncate to max_tokens. Truncation is hash-only logged
            //    (the cap value itself is operator-set so it's safe to
            //    label).
            let mut input_tokens = tokens;
            if input_tokens.len() > self.max_tokens {
                tracing::warn!(
                    error_class = "PerplexityScorerInputTruncated",
                    cap = self.max_tokens,
                    "perplexity scorer truncated overlong input"
                );
                input_tokens.truncate(self.max_tokens);
            }

            if input_tokens.len() < 2 {
                // Degenerate: no usable predictive context. Fail-closed.
                return Ok(PerplexityResult {
                    aggregate_perplexity_micros: 0,
                    tail_fraction_micros: 0,
                    tokens_scored: 0,
                });
            }

            // 4. Build the raw-logits request. The default mistralrs response
            //    is a chat-completion message, not raw logits, so the
            //    `return_raw_logits: true` flag is critical — it changes the
            //    pipeline path to produce `ResponseOk::Raw` instead of
            //    `ResponseOk::CompletionChunk`.
            //
            //    `sampling_params.max_len = Some(0)` tells the pipeline to
            //    do prefill only and return logits over the input sequence;
            //    no token generation happens.
            let inner: &MistralRs = model_guard.inner();
            let (tx, mut rx) = channel(1);
            let request = Request::Normal(Box::new(NormalRequest {
                messages: RequestMessage::CompletionTokens(input_tokens.clone()),
                sampling_params: SamplingParams {
                    max_len: Some(0),
                    ..SamplingParams::deterministic()
                },
                response: tx,
                return_logprobs: false,
                is_streaming: false,
                id: 0,
                constraint: Constraint::None,
                suffix: None,
                tools: None,
                tool_choice: None,
                logits_processors: None,
                return_raw_logits: true,
                web_search_options: None,
                model_id: None,
                max_tool_rounds: None,
                tool_dispatch_url: None,
                truncate_sequence: false,
            }));
            inner
                .get_sender(None)
                .map_err(|e| {
                    anyhow::anyhow!("LocalPerplexityScorerSenderFailed: {}", hash_err(&e))
                })?
                .send(request)
                .await
                .map_err(|e| {
                    anyhow::anyhow!("LocalPerplexityScorerSendFailed: {}", hash_err(&e))
                })?;

            // 5. Recv + destructure. The channel is single-shot; the
            //    pipeline sends one response and closes.
            let response = rx
                .recv()
                .await
                .context("LocalPerplexityScorerResponseChannelClosed")?
                .as_result()
                .map_err(|e| anyhow::anyhow!("LocalPerplexityScorerError: {}", hash_err(&e)))?;
            let ResponseOk::Raw {
                logits_chunks,
                tokens: out_tokens,
            } = response
            else {
                anyhow::bail!("LocalPerplexityScorerUnexpectedResponseShape");
            };

            // 6. Aggregate. `logits_chunks[0]` is a `[seq_len, vocab]`
            //    tensor over the input sequence; convert to per-token
            //    logprobs of the realized next token via log_softmax +
            //    gather, then run through the existing aggregate helper
            //    (which handles BOS-drop, tail-fraction, and fail-closed
            //    semantics).
            let logprobs = logprobs_from_logits(&logits_chunks, &out_tokens)?;
            Ok(aggregate_perplexity_metrics(
                &logprobs,
                self.tail_logprob_cutoff,
            ))
        }
    }

    impl PerplexityScorer for LocalPerplexityScorer {
        fn score(&self, plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
            // Dispatch the job to the scorer thread (which owns the
            // runtime + model) and block on its reply. Fail-closed: a
            // scoring failure propagates so the orchestrator refuses the
            // gate evaluation; a closed reply channel is converted into a
            // stable error class so callers see a self-explanatory
            // diagnostic.
            let (reply_tx, reply_rx) = std::sync::mpsc::channel();
            self.job_tx
                .send(Job::Score {
                    plaintext: plaintext.to_vec(),
                    reply: reply_tx,
                })
                .map_err(|_| anyhow::anyhow!("LocalPerplexityScorerWorkerGone"))?;
            reply_rx
                .recv()
                .map_err(|_| anyhow::anyhow!("LocalPerplexityScorerReplyChannelClosed"))?
        }
    }

    /// Convert mistralrs's per-position logits (shape `[seq_len, vocab]`)
    /// plus the observed token sequence into per-token logprobs of the
    /// realized next token. The first slot is a placeholder for the
    /// BOS-conditioned position; `aggregate_perplexity_metrics` drops it.
    fn logprobs_from_logits(
        logits_chunks: &[mistralrs::Tensor],
        tokens: &[u32],
    ) -> anyhow::Result<Vec<f32>> {
        anyhow::ensure!(
            !logits_chunks.is_empty(),
            "LocalPerplexityScorerEmptyLogitsChunks"
        );
        let logits = &logits_chunks[0];
        // Shape can come back as [seq_len, vocab] or [1, seq_len, vocab];
        // squeeze a leading batch dim if present.
        let logits = if logits.dims().len() == 3 {
            logits.squeeze(0).map_err(|e| {
                anyhow::anyhow!("LocalPerplexityScorerSqueezeFailed: {}", hash_err(&e))
            })?
        } else {
            logits.clone()
        };
        let dims = logits.dims();
        anyhow::ensure!(
            dims.len() == 2,
            "LocalPerplexityScorerUnexpectedLogitsShape"
        );
        let seq_len = dims[0];
        let _vocab = dims[1];

        let lp_tensor =
            log_softmax_dim1(&logits).context("LocalPerplexityScorerLogSoftmaxFailed")?;
        let lp_vec: Vec<Vec<f32>> = lp_tensor
            .to_vec2::<f32>()
            .map_err(|e| anyhow::anyhow!("LocalPerplexityScorerToVec2Failed: {}", hash_err(&e)))?;
        anyhow::ensure!(
            lp_vec.len() == seq_len,
            "LocalPerplexityScorerLogprobShapeMismatch"
        );

        let usable = std::cmp::min(seq_len, tokens.len());
        let mut logprobs: Vec<f32> = Vec::with_capacity(usable);
        // Position 0 is a placeholder (BOS / no predictive context); the
        // aggregate helper drops it.
        logprobs.push(0.0);
        for i in 0..usable.saturating_sub(1) {
            let target = tokens[i + 1] as usize;
            let row = &lp_vec[i];
            if target >= row.len() {
                anyhow::bail!("LocalPerplexityScorerTokenOutOfVocab");
            }
            logprobs.push(row[target]);
        }
        Ok(logprobs)
    }

    /// Log-softmax along dim 1 of a 2-D `[rows, cols]` tensor, returning a
    /// tensor of the same shape. Implemented via the log-sum-exp trick
    /// (subtract per-row max, exp, sum, log) to avoid overflow. Casts to
    /// f32 first so bf16/f16 pipelines round through a stable space.
    fn log_softmax_dim1(logits: &mistralrs::Tensor) -> anyhow::Result<mistralrs::Tensor> {
        let logits_f32 = logits
            .to_dtype(mistralrs::DType::F32)
            .map_err(|e| anyhow::anyhow!("LocalPerplexityScorerDTypeFailed: {}", hash_err(&e)))?;
        let max = logits_f32
            .max_keepdim(1)
            .map_err(|e| anyhow::anyhow!("LocalPerplexityScorerMaxFailed: {}", hash_err(&e)))?;
        let shifted = logits_f32
            .broadcast_sub(&max)
            .map_err(|e| anyhow::anyhow!("LocalPerplexityScorerSubFailed: {}", hash_err(&e)))?;
        let exp = shifted
            .exp()
            .map_err(|e| anyhow::anyhow!("LocalPerplexityScorerExpFailed: {}", hash_err(&e)))?;
        let sum = exp
            .sum_keepdim(1)
            .map_err(|e| anyhow::anyhow!("LocalPerplexityScorerSumFailed: {}", hash_err(&e)))?;
        let log_sum = sum
            .log()
            .map_err(|e| anyhow::anyhow!("LocalPerplexityScorerLogFailed: {}", hash_err(&e)))?;
        shifted
            .broadcast_sub(&log_sum)
            .map_err(|e| anyhow::anyhow!("LocalPerplexityScorerSub2Failed: {}", hash_err(&e)))
    }

    /// Stable, low-entropy fingerprint of a borrowed error value. Used so
    /// the score path's `anyhow::Error` doesn't carry raw operator-secret
    /// text when bubbled to the trait shim's hash-only log.
    fn hash_err<T: std::fmt::Display>(e: &T) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(format!("{e}").as_bytes());
        format!("sha256:{:x}", h.finalize())
    }
}

#[cfg(feature = "local-gpu-models")]
pub use local_impl::LocalPerplexityScorer;

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
        assert!(
            (tail_frac - 0.25).abs() < 1e-6,
            "expected 0.25, got {tail_frac}"
        );
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

    // ----- per_token_rarity_micros: Phase A.5 candidate replacement metric.
    //
    // The reference formula is `exp(-mean(K rarest logprobs))`, matching the
    // Python prototype at `scripts/research/per-token-rarity-prototype.py`.
    // Every test below picks logprobs with hand-computed expected outputs so
    // any drift between the Rust port and the prototype shows up as a value
    // mismatch rather than a vague "AUC moved" downstream effect.

    #[test]
    fn rarity_zero_for_empty_or_single_token_input() {
        assert_eq!(per_token_rarity_micros(&[], 10), 0);
        assert_eq!(per_token_rarity_micros(&[-1.5], 10), 0);
    }

    #[test]
    fn rarity_zero_when_k_is_zero() {
        // K = 0 is a meaningless request; collapse to zero rather than
        // surfacing an exp(0) = 1 artifact that would look like real signal.
        let logprobs = vec![0.0_f32, -1.0, -2.0, -3.0];
        assert_eq!(per_token_rarity_micros(&logprobs, 0), 0);
    }

    #[test]
    fn rarity_matches_python_prototype_on_synthetic_fixture() {
        // Fixture: a 6-token "trace" whose usable (post-BOS-drop) logprobs are
        // [-0.5, -2.0, -1.0, -5.0, -0.1]. With K=2 the rarest are [-5.0,-2.0],
        // mean_nll = 3.5, rarity = exp(3.5) ≈ 33.1155.
        let logprobs = vec![0.0_f32, -0.5, -2.0, -1.0, -5.0, -0.1];
        let got = per_token_rarity_micros(&logprobs, 2);
        let want = 3.5_f32.exp() * 1_000_000.0;
        // Tolerance: f32 mean + exp accumulate sub-ULP noise; 1e-3 micros is
        // tighter than any decision-rule threshold.
        let diff = (got as f32 - want).abs();
        assert!(
            diff < 1.0,
            "rarity micros mismatch vs python prototype: got {got}, want ~{want}, diff {diff}"
        );
    }

    #[test]
    fn rarity_k_one_equals_exp_of_largest_negative_logprob() {
        // K=1 picks the single rarest token; rarity = exp(-min_logprob) =
        // exp(max(-lp)). This is the "task statement's K=1 invariant" test.
        let logprobs = vec![0.0_f32, -0.5, -2.0, -1.0, -5.0, -0.1];
        let got = per_token_rarity_micros(&logprobs, 1);
        let want = 5.0_f32.exp() * 1_000_000.0;
        let diff = (got as f32 - want).abs();
        assert!(
            diff < 1.0,
            "K=1 rarity should equal exp(-min_logprob): got {got}, want ~{want}"
        );
    }

    #[test]
    fn rarity_k_equals_len_collapses_to_aggregate_perplexity() {
        // K=len averages all usable logprobs — the same population the
        // aggregate-perplexity helper averages. So `per_token_rarity_micros`
        // at K=len equals `aggregate_perplexity_micros` (up to f32 rounding).
        // This is the "task statement's K=len invariant" expressed in the
        // prototype's exp-of-mean form, which is the formula we actually
        // ship; relating it to perplexity is the testable connection.
        let logprobs = vec![0.0_f32, -0.5, -2.0, -1.0, -5.0, -0.1];
        let usable_len = logprobs.len() - 1;
        let r_rarity = per_token_rarity_micros(&logprobs, usable_len);
        let r_agg = aggregate_perplexity_metrics(&logprobs, -8.0).aggregate_perplexity_micros;
        // Same f32 path on both sides; allow a single-micro slack.
        let diff = (r_rarity as i64 - r_agg as i64).abs();
        assert!(
            diff <= 1,
            "K=len rarity should equal aggregate perplexity: got {r_rarity}, want {r_agg}"
        );
    }

    #[test]
    fn rarity_caps_k_at_usable_length() {
        // K larger than the number of usable tokens caps to len; should NOT
        // panic or wrap. Result equals K=usable_len.
        let logprobs = vec![0.0_f32, -1.0, -2.0, -3.0];
        let capped = per_token_rarity_micros(&logprobs, 999);
        let at_len = per_token_rarity_micros(&logprobs, 3);
        assert_eq!(capped, at_len);
    }

    #[test]
    fn rarity_rejects_non_finite_logprobs() {
        // Parallel to the aggregate helper: NaN / Inf collapse to zero.
        let with_nan = vec![0.0_f32, -1.0, f32::NAN, -2.0];
        assert_eq!(per_token_rarity_micros(&with_nan, 2), 0);
        let with_inf = vec![0.0_f32, -1.0, f32::NEG_INFINITY, -2.0];
        assert_eq!(per_token_rarity_micros(&with_inf, 2), 0);
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
    fn local_perplexity_smoke_against_real_model() {
        use super::local_impl::LocalPerplexityScorer;
        use crate::perplexity::PerplexityScorer;
        if std::env::var("TRACE_COMMONS_PERPLEXITY_INTEGRATION")
            .ok()
            .as_deref()
            != Some("1")
        {
            eprintln!(
                "skip: TRACE_COMMONS_PERPLEXITY_INTEGRATION not set; this test requires a GPU + staged model"
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

        let scorer = LocalPerplexityScorer::try_new(
            std::env::var("TRACE_COMMONS_PERPLEXITY_MODEL_ID")
                .unwrap_or_else(|_| "meta-llama/Llama-3.1-8B-Instruct".into()),
            &model_path,
            device,
            -8.0_f32,
            16_384,
        )
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

        let empty = scorer
            .score(b"")
            .expect("empty input is degenerate but score still Ok");
        assert_eq!(empty.aggregate_perplexity_micros, 0);
        assert_eq!(empty.tail_fraction_micros, 0);
    }
}
