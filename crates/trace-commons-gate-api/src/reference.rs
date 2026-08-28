// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reference gate implementations.
//!
//! These are real, deliberately simple scorers — not mocks. They ship with the
//! open reference server so a clone can gate traces without a proprietary
//! backend. They are uncalibrated and materially weaker than a production
//! scorer, and they are meant to be: an operator running these should expect
//! coarse gating, not the hosted service's behavior.

use sha2::{Digest, Sha256};

use crate::embedder::{Embedder, MOCK_EMBEDDING_DIM};
use crate::perplexity::{PerplexityResult, PerplexityScorer};

/// Perplexity proxy built from the Shannon entropy of the byte distribution.
///
/// For a memoryless model over the observed symbol distribution, perplexity is
/// exactly `exp(H)`. That makes this an honest — if unsophisticated —
/// content-complexity signal: repetitive content scores low, varied content
/// scores high. It has no model, so it cannot distinguish "surprising to a
/// language model" from "high-entropy bytes"; a production scorer must.
#[derive(Debug, Default, Clone)]
pub struct ReferencePerplexityScorer;

impl ReferencePerplexityScorer {
    pub fn new() -> Self {
        Self
    }
}

impl PerplexityScorer for ReferencePerplexityScorer {
    fn score(&self, plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
        if plaintext.is_empty() {
            return Ok(PerplexityResult {
                aggregate_perplexity_micros: 0,
                tail_fraction_micros: 0,
                tokens_scored: 0,
            });
        }

        let mut counts = [0u64; 256];
        for b in plaintext {
            counts[*b as usize] += 1;
        }
        let total = plaintext.len() as f64;

        // Shannon entropy in nats, so exp() gives perplexity directly.
        let mut entropy_nats = 0.0f64;
        for c in counts.iter().copied().filter(|c| *c > 0) {
            let p = c as f64 / total;
            entropy_nats -= p * p.ln();
        }
        let perplexity = entropy_nats.exp();

        // Tail signal: the share of bytes drawn from symbols rarer than
        // uniform-over-observed-alphabet. Same lower-bound semantics as the
        // aggregate, and it stays in [0, 1] before micros conversion.
        let distinct = counts.iter().filter(|c| **c > 0).count() as f64;
        let uniform_share = 1.0 / distinct;
        let rare_bytes: u64 = counts
            .iter()
            .copied()
            .filter(|c| *c > 0 && (*c as f64 / total) < uniform_share)
            .sum();
        let tail_fraction = rare_bytes as f64 / total;

        // Same 4-bytes-per-token proxy the chunker uses, so token accounting
        // is comparable across scorers.
        let tokens_scored = (plaintext.len() as u64).div_ceil(4).max(1);

        Ok(PerplexityResult {
            aggregate_perplexity_micros: (perplexity * 1_000_000.0).round() as u64,
            tail_fraction_micros: (tail_fraction * 1_000_000.0).round() as u64,
            tokens_scored,
        })
    }
}

/// Feature-hashed bag-of-tokens embedder.
///
/// Splits on ASCII whitespace, lowercases, hashes each token into one of
/// [`MOCK_EMBEDDING_DIM`] buckets, and L2-normalizes. Texts sharing tokens land
/// near each other, so `1 - cosine` is a usable novelty signal. It has no
/// semantic model: paraphrases with no shared tokens look maximally novel,
/// which is precisely the weakness a production embedder fixes.
#[derive(Debug, Default, Clone)]
pub struct ReferenceEmbedder;

impl ReferenceEmbedder {
    pub fn new() -> Self {
        Self
    }

    /// Stable bucket assignment for a token. Uses the first 8 bytes of
    /// SHA-256 so the mapping is fixed across runs, processes, and releases —
    /// vectors persisted by one build must stay comparable to the next.
    fn bucket(token: &str) -> usize {
        let mut h = Sha256::new();
        h.update(b"trace_commons_gate_api.reference_embedder.v1\n");
        h.update(token.as_bytes());
        let out = h.finalize();
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&out[0..8]);
        (u64::from_be_bytes(buf) % MOCK_EMBEDDING_DIM as u64) as usize
    }
}

impl Embedder for ReferenceEmbedder {
    fn embed(&self, plaintext: &[u8]) -> anyhow::Result<Vec<f32>> {
        let mut v = vec![0.0f32; MOCK_EMBEDDING_DIM];

        // Lossy UTF-8: trace plaintext is expected to be text, and a decoding
        // failure must not refuse the gate. Invalid sequences become U+FFFD,
        // which simply hashes to its own bucket.
        let text = String::from_utf8_lossy(plaintext);
        for token in text.split_ascii_whitespace() {
            let lowered = token.to_lowercase();
            v[Self::bucket(&lowered)] += 1.0;
        }

        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_random_bytes_score_higher_than_repetitive_bytes() {
        // The core honesty property: unlike a hash-derived mock, this scorer's
        // output tracks actual content complexity.
        let s = ReferencePerplexityScorer::new();
        let repetitive = vec![b'a'; 4096];
        let varied: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        let low = s.score(&repetitive).unwrap();
        let high = s.score(&varied).unwrap();
        assert!(
            high.aggregate_perplexity_micros > low.aggregate_perplexity_micros,
            "varied {} must exceed repetitive {}",
            high.aggregate_perplexity_micros,
            low.aggregate_perplexity_micros
        );
    }

    #[test]
    fn reference_perplexity_is_deterministic() {
        let s = ReferencePerplexityScorer::new();
        assert_eq!(
            s.score(b"hello world").unwrap(),
            s.score(b"hello world").unwrap()
        );
    }

    #[test]
    fn single_byte_input_has_zero_entropy() {
        // One distinct symbol means H = 0, so exp(H) = 1.0 = 1_000_000 micros.
        let s = ReferencePerplexityScorer::new();
        let r = s.score(&vec![b'z'; 512]).unwrap();
        assert_eq!(r.aggregate_perplexity_micros, 1_000_000);
    }

    #[test]
    fn empty_input_scores_zero_without_panicking() {
        let s = ReferencePerplexityScorer::new();
        let r = s.score(b"").unwrap();
        assert_eq!(r.aggregate_perplexity_micros, 0);
        assert_eq!(r.tokens_scored, 0);
    }

    #[test]
    fn reference_embedder_is_unit_norm_and_correct_dim() {
        let e = ReferenceEmbedder::new();
        let v = e
            .embed(b"the quick brown fox jumps over the lazy dog")
            .unwrap();
        assert_eq!(v.len(), MOCK_EMBEDDING_DIM);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-3, "expected unit norm, got {norm}");
    }

    #[test]
    fn reference_embedder_is_semantically_sensitive() {
        // Two texts sharing most tokens must be closer than two that share
        // none. A hash-derived mock cannot satisfy this.
        let e = ReferenceEmbedder::new();
        let a = e.embed(b"alpha beta gamma delta epsilon").unwrap();
        let near = e.embed(b"alpha beta gamma delta zeta").unwrap();
        let far = e.embed(b"quartz sphinx judge vex nymph").unwrap();
        let dot = |x: &[f32], y: &[f32]| -> f32 { x.iter().zip(y).map(|(p, q)| p * q).sum() };
        assert!(
            dot(&a, &near) > dot(&a, &far),
            "near {} must exceed far {}",
            dot(&a, &near),
            dot(&a, &far)
        );
    }

    #[test]
    fn reference_embedder_handles_empty_input() {
        let e = ReferenceEmbedder::new();
        let v = e.embed(b"").unwrap();
        assert_eq!(v.len(), MOCK_EMBEDDING_DIM);
        assert!(
            v.iter().all(|x| *x == 0.0),
            "empty input yields the zero vector"
        );
    }
}
