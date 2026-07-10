//! Perplexity scoring trait + deterministic mock.

use sha2::{Digest, Sha256};

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

/// Deterministic mock: result is a hash-derived projection of the plaintext.
///
/// The numbers are stable across runs, so tests can assert on exact values.
/// Identical plaintexts produce identical results.
#[derive(Debug, Default, Clone)]
pub struct MockPerplexityScorer;

impl MockPerplexityScorer {
    pub fn new() -> Self {
        Self
    }
}

impl PerplexityScorer for MockPerplexityScorer {
    fn score(&self, plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
        let mut h = Sha256::new();
        h.update(b"trace_gate_enclave.mock_perplexity.v1\n");
        h.update(plaintext);
        let out = h.finalize();
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&out[0..8]);
        let aggregate = u64::from_be_bytes(buf);
        buf.copy_from_slice(&out[8..16]);
        let tail = u64::from_be_bytes(buf);
        // Rough token estimate from byte length (≈4 bytes/token for English
        // text). The mock is hash-derived; the precise value isn't load-
        // bearing — bake-off throughput accounting just needs a non-zero
        // count proportional to the work done.
        let tokens_scored = (plaintext.len() as u64).div_ceil(4).max(1);
        Ok(PerplexityResult {
            // Squeeze into a 0..10_000_000 micros band so values look like
            // real perplexity figures (1.0 - 10.0 in floating-point space).
            aggregate_perplexity_micros: aggregate % 10_000_000,
            tail_fraction_micros: tail % 10_000_000,
            tokens_scored,
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

/// Deterministic mock for the per-token rarity scorer. Hash-derived output,
/// stable across runs, identical plaintexts produce identical results.
///
/// Distinct from `MockPerplexityScorer` so the bake-off's `--scorer both`
/// path can run two independent mock scorers and produce uncorrelated
/// numbers across the perplexity and rarity columns. Reusing a single hash
/// for both would make the columns identical, which would silently mask
/// real-world delta in CI.
#[derive(Debug, Default, Clone)]
pub struct MockTokenRarityScorer;

impl MockTokenRarityScorer {
    pub fn new() -> Self {
        Self
    }
}

impl TokenRarityScorer for MockTokenRarityScorer {
    fn score_rarity(&self, plaintext: &[u8], k: usize) -> anyhow::Result<TokenRarityResult> {
        if k == 0 {
            // Parallel to `per_token_rarity_micros`: K=0 collapses to zero
            // rather than emitting a meaningless exp(0)=1 artifact.
            let tokens_scored = (plaintext.len() as u64).div_ceil(4).max(1);
            return Ok(TokenRarityResult {
                token_rarity_micros: 0,
                tokens_scored,
                k: 0,
            });
        }
        let mut h = Sha256::new();
        // Distinct prefix from `MockPerplexityScorer` so the two mocks
        // produce uncorrelated streams — see type-level comment above.
        h.update(b"trace_gate_enclave.mock_token_rarity.v1\n");
        h.update(plaintext);
        h.update(b"\nk=");
        h.update((k as u64).to_be_bytes());
        let out = h.finalize();
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&out[0..8]);
        let rarity = u64::from_be_bytes(buf);
        let tokens_scored = (plaintext.len() as u64).div_ceil(4).max(1);
        // Same 0..10_000_000 micros band as the perplexity mock so report
        // values from both columns are visually comparable.
        Ok(TokenRarityResult {
            token_rarity_micros: rarity % 10_000_000,
            tokens_scored,
            // K is capped at tokens_scored in spirit; the mock has no real
            // tokenization so we report the requested K verbatim, clamped
            // to u32 because the report serializes it as such.
            k: k.min(u32::MAX as usize) as u32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_perplexity_is_deterministic() {
        let s = MockPerplexityScorer::new();
        let a = s.score(b"hello world").unwrap();
        let b = s.score(b"hello world").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn mock_perplexity_differs_per_input() {
        let s = MockPerplexityScorer::new();
        let a = s.score(b"hello world").unwrap();
        let b = s.score(b"hello WORLD").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn mock_token_rarity_is_deterministic() {
        let s = MockTokenRarityScorer::new();
        let a = s.score_rarity(b"hello world", 10).unwrap();
        let b = s.score_rarity(b"hello world", 10).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn mock_token_rarity_differs_per_input() {
        let s = MockTokenRarityScorer::new();
        let a = s.score_rarity(b"hello world", 10).unwrap();
        let b = s.score_rarity(b"hello WORLD", 10).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn mock_token_rarity_differs_per_k() {
        // Different K should hash to different output streams so reports
        // built with two K values aren't artificially correlated.
        let s = MockTokenRarityScorer::new();
        let a = s.score_rarity(b"hello world", 8).unwrap();
        let b = s.score_rarity(b"hello world", 16).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn mock_token_rarity_zero_when_k_is_zero() {
        let s = MockTokenRarityScorer::new();
        let r = s.score_rarity(b"hello world", 0).unwrap();
        assert_eq!(r.token_rarity_micros, 0);
        assert_eq!(r.k, 0);
    }

    #[test]
    fn mock_perplexity_and_token_rarity_streams_are_uncorrelated() {
        // The two mocks must use distinct hash prefixes so report rows
        // populated with both don't end up with identical numeric tails.
        let p = MockPerplexityScorer::new();
        let r = MockTokenRarityScorer::new();
        let pr = p.score(b"hello world").unwrap();
        let rr = r.score_rarity(b"hello world", 10).unwrap();
        assert_ne!(pr.aggregate_perplexity_micros, rr.token_rarity_micros);
    }

    #[test]
    fn default_score_chunk_derives_from_score_within_tolerance() {
        // exp(sum_nll / tokens) must reproduce score()'s aggregate within
        // f64 ln/exp round-trip tolerance.
        let s = MockPerplexityScorer::new();
        let whole = s.score(b"hello world").unwrap();
        let chunk = s.score_chunk(b"hello world").unwrap();
        assert_eq!(chunk.tokens, whole.tokens_scored);
        let rebuilt = ((chunk.sum_nll / chunk.tokens as f64).exp() * 1_000_000.0) as u64;
        let diff = rebuilt.abs_diff(whole.aggregate_perplexity_micros);
        assert!(diff <= 2, "ln/exp round trip drifted by {diff} micros");
        // Mock cannot expose raw logprobs.
        assert!(chunk.logprobs.is_empty());
        // tail_tokens is clamped to tokens even though the mock's
        // tail_fraction_micros band can exceed 1.0.
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
