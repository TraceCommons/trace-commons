//! Perplexity scoring trait + deterministic mock.

use sha2::{Digest, Sha256};
pub use trace_commons_gate_api::{
    ChunkPerplexity, PerplexityResult, PerplexityScorer, TokenRarityResult, TokenRarityScorer,
};

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
}
