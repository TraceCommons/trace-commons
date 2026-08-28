// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Pure whole-trace aggregation over per-chunk scoring results.
//!
//! Representative = token-weighted whole-trace values (equal to a single
//! whole-trace call within float tolerance for one chunk). Peak = the
//! most-surprising / most-novel min-content-guarded chunk. All math is f64;
//! degenerate inputs collapse to zero (fail-closed — the configured floors
//! then refuse the gate naturally).

use crate::perplexity::ChunkPerplexity;
use crate::perplexity_local::per_token_rarity_micros;

/// Whole-trace perplexity aggregate. Micros fields saturate; non-finite
/// intermediates collapse to zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkedPerplexityAggregate {
    /// `exp( sum_c sum_nll_c / sum_c n_c )` — stored in the existing
    /// `perplexity_micros` column.
    pub representative_perplexity_micros: u64,
    /// `max_c exp(sum_nll_c / n_c)` over chunks with `n_c >=
    /// min_chunk_tokens`; falls back to the representative when no chunk is
    /// eligible. Stored in the new `peak_perplexity_micros` column.
    pub peak_perplexity_micros: u64,
    /// `sum_c tail_tokens_c / sum_c n_c` — the exact whole-trace fraction.
    pub tail_fraction_micros: u64,
    /// `sum_c n_c`.
    pub tokens_scored: u64,
}

fn saturating_micros_f64(v: f64) -> u64 {
    if !v.is_finite() || v < 0.0 {
        return 0;
    }
    let scaled = v * 1_000_000.0;
    if scaled >= u64::MAX as f64 {
        u64::MAX
    } else {
        scaled as u64
    }
}

pub fn aggregate_chunked_perplexity(
    chunks: &[ChunkPerplexity],
    min_chunk_tokens: u64,
) -> ChunkedPerplexityAggregate {
    let total_tokens: u64 = chunks.iter().map(|c| c.tokens).sum();
    if total_tokens == 0 {
        return ChunkedPerplexityAggregate {
            representative_perplexity_micros: 0,
            peak_perplexity_micros: 0,
            tail_fraction_micros: 0,
            tokens_scored: 0,
        };
    }
    let total_nll: f64 = chunks.iter().map(|c| c.sum_nll).sum();
    let total_tail: u64 = chunks.iter().map(|c| c.tail_tokens).sum();
    let representative = (total_nll / total_tokens as f64).exp();
    let representative_perplexity_micros = saturating_micros_f64(representative);

    let peak = chunks
        .iter()
        .filter(|c| c.tokens >= min_chunk_tokens && c.tokens > 0)
        .map(|c| (c.sum_nll / c.tokens as f64).exp())
        .fold(f64::NEG_INFINITY, f64::max);
    let peak_perplexity_micros = if peak.is_finite() {
        saturating_micros_f64(peak)
    } else {
        representative_perplexity_micros
    };

    ChunkedPerplexityAggregate {
        representative_perplexity_micros,
        peak_perplexity_micros,
        tail_fraction_micros: saturating_micros_f64(total_tail as f64 / total_tokens as f64),
        tokens_scored: total_tokens,
    }
}

/// Global top-K rarity over the concatenation of all chunks' usable
/// logprobs: `exp(-mean(K globally-rarest))`. Reuses
/// [`per_token_rarity_micros`] by prepending its expected BOS placeholder.
/// Chunks whose scorer exposed no raw logprobs contribute nothing.
pub fn global_rarity_micros_across_chunks(chunks: &[ChunkPerplexity], k: usize) -> u64 {
    let mut all: Vec<f32> =
        Vec::with_capacity(1 + chunks.iter().map(|c| c.logprobs.len()).sum::<usize>());
    all.push(0.0); // BOS placeholder dropped by the helper.
    for c in chunks {
        all.extend_from_slice(&c.logprobs);
    }
    per_token_rarity_micros(&all, k)
}

/// Token-weighted representative + min-content-guarded peak over per-chunk
/// novelty scores (micros). Weights are the chunks' scored-token counts;
/// all-zero weights fall back to an unweighted mean and an unguarded peak
/// so a degenerate scorer cannot zero out the novelty signal.
pub fn aggregate_chunked_novelty(
    novelty_micros: &[u64],
    chunk_tokens: &[u64],
    min_chunk_tokens: u64,
) -> (u64, u64) {
    assert_eq!(novelty_micros.len(), chunk_tokens.len());
    if novelty_micros.is_empty() {
        return (0, 0);
    }
    let total_tokens: u64 = chunk_tokens.iter().sum();
    let representative = if total_tokens == 0 {
        novelty_micros.iter().map(|&n| n as f64).sum::<f64>() / novelty_micros.len() as f64
    } else {
        novelty_micros
            .iter()
            .zip(chunk_tokens.iter())
            .map(|(&n, &t)| n as f64 * t as f64)
            .sum::<f64>()
            / total_tokens as f64
    };
    let peak = novelty_micros
        .iter()
        .zip(chunk_tokens.iter())
        .filter(|&(_, &t)| total_tokens == 0 || t >= min_chunk_tokens)
        .map(|(&n, _)| n)
        .max()
        .unwrap_or(representative as u64);
    (representative as u64, peak)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perplexity::ChunkPerplexity;

    fn chunk(sum_nll: f64, tokens: u64, tail_tokens: u64, logprobs: Vec<f32>) -> ChunkPerplexity {
        ChunkPerplexity {
            sum_nll,
            tokens,
            tail_tokens,
            logprobs,
        }
    }

    #[test]
    fn single_chunk_representative_equals_whole_call() {
        // One chunk of uniform logprob -1.0 over 4 tokens: perplexity = e.
        let c = chunk(4.0, 4, 0, vec![-1.0; 4]);
        let agg = aggregate_chunked_perplexity(&[c], 1);
        let want = (1.0_f64.exp() * 1_000_000.0) as u64;
        assert!(agg.representative_perplexity_micros.abs_diff(want) <= 2);
        assert_eq!(
            agg.peak_perplexity_micros,
            agg.representative_perplexity_micros
        );
        assert_eq!(agg.tail_fraction_micros, 0);
        assert_eq!(agg.tokens_scored, 4);
    }

    #[test]
    fn representative_is_token_weighted_across_chunks() {
        // Chunk A: 100 tokens at mean_nll 1.0. Chunk B: 300 tokens at
        // mean_nll 3.0. Weighted mean_nll = (100 + 900)/400 = 2.5.
        let a = chunk(100.0, 100, 0, vec![]);
        let b = chunk(900.0, 300, 0, vec![]);
        let agg = aggregate_chunked_perplexity(&[a, b], 1);
        let want = (2.5_f64.exp() * 1_000_000.0) as u64;
        assert!(agg.representative_perplexity_micros.abs_diff(want) <= 2);
        assert_eq!(agg.tokens_scored, 400);
    }

    #[test]
    fn peak_is_max_over_min_content_guarded_chunks() {
        // Chunk A: 100 tokens, mean_nll 1.0. Chunk B: 100 tokens, mean_nll
        // 3.0 (the peak). Chunk C: 4 tokens, mean_nll 10.0 — a tiny
        // surprising fragment that the 64-token guard must exclude.
        let a = chunk(100.0, 100, 0, vec![]);
        let b = chunk(300.0, 100, 0, vec![]);
        let c = chunk(40.0, 4, 0, vec![]);
        let agg = aggregate_chunked_perplexity(&[a, b, c], 64);
        let want = (3.0_f64.exp() * 1_000_000.0) as u64;
        assert!(agg.peak_perplexity_micros.abs_diff(want) <= 2);
    }

    #[test]
    fn peak_falls_back_to_representative_when_no_chunk_is_eligible() {
        let a = chunk(10.0, 10, 0, vec![]);
        let agg = aggregate_chunked_perplexity(&[a], 64);
        assert_eq!(
            agg.peak_perplexity_micros,
            agg.representative_perplexity_micros
        );
    }

    #[test]
    fn tail_fraction_is_exact_over_all_tokens() {
        // 1 tail token of 4 + 3 tail tokens of 6 = 4/10 = 0.4.
        let a = chunk(4.0, 4, 1, vec![]);
        let b = chunk(6.0, 6, 3, vec![]);
        let agg = aggregate_chunked_perplexity(&[a, b], 1);
        assert_eq!(agg.tail_fraction_micros, 400_000);
    }

    #[test]
    fn zero_total_tokens_is_all_zero() {
        let agg = aggregate_chunked_perplexity(&[chunk(0.0, 0, 0, vec![])], 64);
        assert_eq!(agg.representative_perplexity_micros, 0);
        assert_eq!(agg.peak_perplexity_micros, 0);
        assert_eq!(agg.tail_fraction_micros, 0);
        assert_eq!(agg.tokens_scored, 0);
    }

    #[test]
    fn global_rarity_selects_across_chunk_boundaries() {
        // Rarest two tokens live in DIFFERENT chunks: -5.0 (chunk A) and
        // -4.0 (chunk B). K=2 -> mean_nll 4.5 -> exp(4.5).
        let a = chunk(5.5, 2, 0, vec![-0.5, -5.0]);
        let b = chunk(4.1, 2, 0, vec![-4.0, -0.1]);
        let got = global_rarity_micros_across_chunks(&[a, b], 2);
        let want = (4.5_f64.exp() * 1_000_000.0) as u64;
        assert!(got.abs_diff(want) <= 40, "got {got}, want ~{want}");
    }

    #[test]
    fn global_rarity_ignores_chunks_without_logprobs() {
        let a = chunk(5.0, 5, 0, vec![]);
        let b = chunk(2.0, 2, 0, vec![-1.0, -1.0]);
        let got = global_rarity_micros_across_chunks(&[a, b], 2);
        let want = (1.0_f64.exp() * 1_000_000.0) as u64;
        assert!(got.abs_diff(want) <= 40);
    }

    #[test]
    fn novelty_representative_is_token_weighted_and_peak_guarded() {
        // Chunk novelties 0.2 (100 tok), 0.8 (100 tok), 1.0 (4 tok, guarded
        // out of peak). Representative = (0.2*100 + 0.8*100 + 1.0*4)/204.
        let (rep, peak) =
            aggregate_chunked_novelty(&[200_000, 800_000, 1_000_000], &[100, 100, 4], 64);
        let want_rep = ((0.2 * 100.0 + 0.8 * 100.0 + 1.0 * 4.0) / 204.0 * 1_000_000.0) as u64;
        assert!(rep.abs_diff(want_rep) <= 1);
        assert_eq!(peak, 800_000);
    }

    #[test]
    fn novelty_zero_weights_fall_back_to_uniform() {
        // All-zero token counts (degenerate scorer) must not divide by zero;
        // fall back to an unweighted mean and unguarded peak.
        let (rep, peak) = aggregate_chunked_novelty(&[200_000, 400_000], &[0, 0], 64);
        assert_eq!(rep, 300_000);
        assert_eq!(peak, 400_000);
    }
}
