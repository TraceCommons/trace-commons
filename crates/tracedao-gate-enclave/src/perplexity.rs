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
}

/// Score a plaintext trace for perplexity. Real implementations run a local
/// LLM inside the enclave; the mock here is purely deterministic.
///
/// `score` returns `anyhow::Result` so an inference failure refuses the gate
/// evaluation rather than silently producing a zero result that would falsely
/// pass any positive floor. Callers MUST propagate the error.
pub trait PerplexityScorer: Send + Sync {
    fn score(&self, plaintext: &[u8]) -> anyhow::Result<PerplexityResult>;
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
        Ok(PerplexityResult {
            // Squeeze into a 0..10_000_000 micros band so values look like
            // real perplexity figures (1.0 - 10.0 in floating-point space).
            aggregate_perplexity_micros: aggregate % 10_000_000,
            tail_fraction_micros: tail % 10_000_000,
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
}
