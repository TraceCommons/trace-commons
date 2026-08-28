// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

/// Output of scoring a plaintext for perplexity.
///
/// The fields are kept in fixed-point micros so the host can persist them
/// without re-deriving precision rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerplexityResult {
    /// Aggregate perplexity across the trace, in micros. Larger values mean
    /// "more surprising" content — the gate-policy floor is a lower bound.
    pub aggregate_perplexity_micros: u64,
    /// Tail-fraction perplexity (e.g., 95th-percentile token surprise) in
    /// micros. Same lower-bound semantics as the aggregate.
    pub tail_fraction_micros: u64,
    /// Approximate token count scored, used for throughput accounting in the
    /// bake-off. Real scorers populate this from the tokenizer; the mock
    /// estimates from byte length. The field is informational — gate
    /// orchestration does not consume it.
    pub tokens_scored: u64,
}

/// Raw per-chunk scoring material for whole-trace aggregation. Unlike
/// [`PerplexityResult`] (already collapsed to micros), this keeps the sums
/// so the orchestrator can compute the token-weighted whole-trace mean
/// `exp(sum over chunks of sum_nll / sum over chunks of n)` exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkPerplexity {
    /// `-sum(logprob)` over the chunk's usable tokens (token 0 dropped).
    pub sum_nll: f64,
    /// Usable token count for the chunk (`n_c`).
    pub tokens: u64,
    /// Count of usable tokens with `logprob < tail_logprob_cutoff`.
    pub tail_tokens: u64,
    /// The usable (post-BOS-drop) per-token logprobs, for global top-K
    /// rarity across chunks. Empty when the scorer cannot expose raw
    /// logprobs (e.g. the mock) — rarity then simply has no contribution
    /// from that chunk.
    pub logprobs: Vec<f32>,
}

/// Score a plaintext trace for perplexity. Real implementations run a local
/// LLM inside the enclave; the mock here is purely deterministic.
///
/// `score` returns `anyhow::Result` so an inference failure refuses the gate
/// evaluation rather than silently producing a zero result that would falsely
/// pass any positive floor. Callers MUST propagate the error.
pub trait PerplexityScorer: Send + Sync {
    fn score(&self, plaintext: &[u8]) -> anyhow::Result<PerplexityResult>;

    /// Score one bounded chunk and return raw aggregation material. The
    /// default derives `(sum_nll, n, tail_tokens)` from [`score`]'s
    /// collapsed micros (exact within f64 ln/exp round-trip tolerance) and
    /// exposes no raw logprobs. Real scorers that hold per-token logprobs
    /// SHOULD override this to return them losslessly.
    fn score_chunk(&self, chunk: &[u8]) -> anyhow::Result<ChunkPerplexity> {
        let r = self.score(chunk)?;
        let tokens = r.tokens_scored;
        if tokens == 0 {
            return Ok(ChunkPerplexity {
                sum_nll: 0.0,
                tokens: 0,
                tail_tokens: 0,
                logprobs: Vec::new(),
            });
        }
        let perp = r.aggregate_perplexity_micros as f64 / 1_000_000.0;
        let mean_nll = if perp > 0.0 { perp.ln() } else { 0.0 };
        let tail_fraction = r.tail_fraction_micros as f64 / 1_000_000.0;
        let tail_tokens = ((tail_fraction * tokens as f64).round() as u64).min(tokens);
        Ok(ChunkPerplexity {
            sum_nll: mean_nll * tokens as f64,
            tokens,
            tail_tokens,
            logprobs: Vec::new(),
        })
    }
}

/// Output of scoring a plaintext for per-token rarity (Phase A.5 candidate
/// replacement metric for aggregate perplexity).
///
/// Higher `token_rarity_micros` means the trace contains more genuinely-rare
/// tokens under the candidate model — the novelty signal the gate cares
/// about. `k` is recorded so report rows can document the K value the scorer
/// was configured with (the bake-off flips it per-run; persisting it on the
/// result prevents misreading two reports that used different K).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenRarityResult {
    /// `exp(-mean(K rarest logprobs))` in fixed-point micros. Larger = more
    /// surprising token tail.
    pub token_rarity_micros: u64,
    /// Approximate token count scored; mirrors `PerplexityResult::tokens_scored`
    /// so the bake-off can keep one throughput-accounting path.
    pub tokens_scored: u64,
    /// Effective K used to compute the metric. May be smaller than the
    /// requested K when the trace tokenized to fewer usable tokens.
    pub k: u32,
}

/// Score a plaintext trace for per-token rarity. Same fail-closed contract
/// as [`PerplexityScorer`]: an inference failure propagates so the bake-off
/// loop counts it toward the per-candidate failure budget rather than
/// silently substituting a zero.
///
/// Implementations may share their model + tokenizer with a co-located
/// `PerplexityScorer` — the rarity metric is computed from the same
/// per-token logprob vector as aggregate perplexity, so a real scorer that
/// implements both traits does one forward pass per trace, not two.
pub trait TokenRarityScorer: Send + Sync {
    fn score_rarity(&self, plaintext: &[u8], k: usize) -> anyhow::Result<TokenRarityResult>;
}

/// A scoring failure that carries whether the trace was at fault.
///
/// The remote scoring backends this crate's `PerplexityScorer`
/// implementations talk to can fail for two very different reasons, and
/// callers that keep a per-trace attempt budget need to tell them apart: a
/// backend that rejected *this request* will reject it again, while a backend
/// that was simply unavailable says nothing about the trace at all. Charging
/// the second kind to a trace's budget eventually excludes a perfectly good
/// trace from scoring forever.
///
/// The split is carried as a variant, never as text inside `reason` -- test it
/// with [`ScorerFailure::is_transient`]. `Display` is `reason` verbatim so the
/// hash-only error labels a host records stay byte-identical to the labels the
/// scorer produced before this type existed.
///
/// Mirrors `TraceContributionError` in `trace-commons-protocol`, which does
/// the same job for the redaction path. That type is not reachable from here:
/// scoring backends depend on this crate, not on the protocol crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScorerFailure {
    /// The backend objected to this trace or to the request built from it: a
    /// prompt that does not fit, a malformed request, a response body whose
    /// shape is wrong. Retrying spends the caller's budget for nothing.
    ScorerFailed { reason: String },
    /// The backend was unavailable -- a transport error, a timeout, a 5xx, or
    /// an account/rate condition. Nothing is wrong with the trace.
    ///
    /// Callers that keep a per-trace attempt budget MUST NOT charge this to
    /// the trace.
    TransientScorerFailed { reason: String },
}

impl ScorerFailure {
    /// True when the failure was the backend's, not the trace's.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::TransientScorerFailed { .. })
    }

    /// The hash-safe failure label. Never contains trace content, a URL, or a
    /// credential -- backends are required to strip those before constructing
    /// this error.
    pub fn reason(&self) -> &str {
        match self {
            Self::ScorerFailed { reason } | Self::TransientScorerFailed { reason } => reason,
        }
    }
}

impl std::fmt::Display for ScorerFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reason())
    }
}

impl std::error::Error for ScorerFailure {}

/// Whether an HTTP status from a remote scoring backend may be charged to the
/// trace that provoked it.
///
/// The question is not "was this a server error" but "is the trace what the
/// backend objected to". Only three statuses describe the request we built
/// from the trace:
///
/// * `400 Bad Request` -- malformed, or a prompt past the context window.
/// * `413 Payload Too Large`
/// * `422 Unprocessable Entity`
///
/// Everything else is transient. That includes every `5xx`, and also `401`,
/// `402`, `403`, `404`, `408`, and `429`: a revoked key, an exhausted account
/// balance, a rate limit, or a misconfigured endpoint is a deployment
/// condition, and a deployment in that state must stall its queue visibly
/// rather than quietly excluding every trace it touches. `402` in particular
/// is the status that emptied a scoring backlog in production on 2026-08-26.
///
/// Callers bound the cost of the generous default with a consecutive-failure
/// circuit breaker, not by charging traces.
pub fn scorer_status_is_transient(status: u16) -> bool {
    !matches!(status, 400 | 413 | 422)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The transient/permanent split is carried by the variant, not by text
    /// inside `reason`. A caller must never have to parse an error string to
    /// decide whether a failure is the trace's fault.
    #[test]
    fn scorer_failure_transience_is_typed_not_parsed() {
        let transient = ScorerFailure::TransientScorerFailed {
            reason: "NearAiScorerHttpStatusError status=502 body_len=17".to_string(),
        };
        let permanent = ScorerFailure::ScorerFailed {
            reason: "NearAiScorerHttpStatusError status=502 body_len=17".to_string(),
        };
        assert!(transient.is_transient());
        assert!(!permanent.is_transient());
        // Identical `reason` text on both: the classification cannot be
        // recovered from the message, only from the variant.
        assert_eq!(transient.to_string(), permanent.to_string());
        assert_ne!(
            transient.is_transient(),
            permanent.is_transient(),
            "the same reason text must be able to carry either classification"
        );
    }

    /// `Display` is the reason verbatim, so a host that records
    /// `format!("{err}")` as a hash-only attempt label keeps writing exactly
    /// the labels it wrote before this type existed.
    #[test]
    fn scorer_failure_display_is_the_reason_verbatim() {
        let err = ScorerFailure::TransientScorerFailed {
            reason: "NearAiScorerHttpSendFailed: operation timed out".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "NearAiScorerHttpSendFailed: operation timed out"
        );
        assert_eq!(err.reason(), err.to_string());
    }

    /// The typed error survives an `anyhow` wrap and a `.context()` layer, so
    /// a driver several call frames above the scorer can still read the
    /// classification off the type.
    #[test]
    fn scorer_failure_survives_anyhow_context_for_downcast() {
        use anyhow::Context;
        let err: anyhow::Error = anyhow::Error::new(ScorerFailure::TransientScorerFailed {
            reason: "NearAiScorerHttpStatusError status=502 body_len=17".to_string(),
        });
        let wrapped = Err::<(), _>(err)
            .context("PerplexityScorerInferenceFailed")
            .context("TraceGateEvaluationFailed")
            .unwrap_err();
        let found = wrapped
            .downcast_ref::<ScorerFailure>()
            .expect("the concrete error must survive two context layers");
        assert!(found.is_transient());
    }

    /// Only the three statuses that describe the request built from the trace
    /// are chargeable. Everything else -- including the 402 that emptied a
    /// production backlog -- is the backend's problem.
    #[test]
    fn only_trace_attributable_statuses_are_permanent() {
        for permanent in [400u16, 413, 422] {
            assert!(
                !scorer_status_is_transient(permanent),
                "status {permanent} must be chargeable to the trace"
            );
        }
        for transient in [401u16, 402, 403, 404, 408, 429, 500, 502, 503, 504] {
            assert!(
                scorer_status_is_transient(transient),
                "status {transient} must not be chargeable to the trace"
            );
        }
    }
    use sha2::{Digest, Sha256};

    /// Local hash-derived fixture standing in for the enclave crate's
    /// `MockPerplexityScorer`, so the default `score_chunk` contract is
    /// tested where the trait lives.
    struct HashScorer;

    impl PerplexityScorer for HashScorer {
        fn score(&self, plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
            let mut h = Sha256::new();
            h.update(b"trace_commons_gate_api.test_scorer.v1\n");
            h.update(plaintext);
            let out = h.finalize();
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&out[0..8]);
            let aggregate = u64::from_be_bytes(buf);
            buf.copy_from_slice(&out[8..16]);
            let tail = u64::from_be_bytes(buf);
            let tokens_scored = (plaintext.len() as u64).div_ceil(4).max(1);
            Ok(PerplexityResult {
                aggregate_perplexity_micros: aggregate % 10_000_000,
                tail_fraction_micros: tail % 10_000_000,
                tokens_scored,
            })
        }
    }

    #[test]
    fn default_score_chunk_derives_from_score_within_tolerance() {
        let s = HashScorer;
        let whole = s.score(b"hello world").unwrap();
        let chunk = s.score_chunk(b"hello world").unwrap();
        assert_eq!(chunk.tokens, whole.tokens_scored);
        let rebuilt = ((chunk.sum_nll / chunk.tokens as f64).exp() * 1_000_000.0) as u64;
        let diff = rebuilt.abs_diff(whole.aggregate_perplexity_micros);
        assert!(diff <= 2, "ln/exp round trip drifted by {diff} micros");
        assert!(chunk.logprobs.is_empty());
        assert!(chunk.tail_tokens <= chunk.tokens);
    }

    #[test]
    fn default_score_chunk_zero_tokens_is_all_zero() {
        struct ZeroScorer;
        impl PerplexityScorer for ZeroScorer {
            fn score(&self, _plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
                Ok(PerplexityResult {
                    aggregate_perplexity_micros: 0,
                    tail_fraction_micros: 0,
                    tokens_scored: 0,
                })
            }
        }
        let c = ZeroScorer.score_chunk(b"").unwrap();
        assert_eq!(c.tokens, 0);
        assert_eq!(c.sum_nll, 0.0);
        assert_eq!(c.tail_tokens, 0);
    }
}
