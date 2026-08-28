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
    /// Token-weighted share of the scored trace sitting in chunks whose own
    /// perplexity clears `qualifying_chunk_floor_micros`:
    /// `sum{ n_c : ppl_c >= floor } / sum_c n_c`.
    ///
    /// A composition statistic, not an average. The representative is `exp`
    /// of a token-weighted mean log-perplexity, so it is exponentially
    /// compressed at the low end: a quarter-substantive trace sits under 10%
    /// of the way up its range, crushed in with pure boilerplate exactly
    /// where an admission decision has to separate them. This is linear in
    /// composition across the whole range, and unlike an average it does not
    /// regress toward the corpus mean as traces grow.
    ///
    /// Shadow mode: computed and persisted, gates nothing. It may not become
    /// a floor until calibration shows non-degenerate spread — see
    /// `docs/superpowers/specs/2026-08-28-qualifying-token-mass-design.md`,
    /// and see `tail_fraction_micros` for what a mass statistic looks like
    /// when its threshold is placed where nothing lands (81% of pilot traces
    /// score exactly 0).
    pub qualifying_token_fraction_micros: u64,
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
    qualifying_chunk_floor_micros: u64,
) -> ChunkedPerplexityAggregate {
    let total_tokens: u64 = chunks.iter().map(|c| c.tokens).sum();
    if total_tokens == 0 {
        return ChunkedPerplexityAggregate {
            representative_perplexity_micros: 0,
            peak_perplexity_micros: 0,
            tail_fraction_micros: 0,
            tokens_scored: 0,
            qualifying_token_fraction_micros: 0,
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

    // Compared in micros rather than in f64 so the bar a chunk is held to is
    // the same value the decision reports, with no float drift between them.
    // No `min_chunk_tokens` guard: peak needs one because a max over tiny
    // chunks is noise, but token weighting already shrinks a tiny chunk to
    // near-nothing, and a second knob would only add a way to be wrong.
    let qualifying_tokens: u64 = chunks
        .iter()
        .filter(|c| c.tokens > 0)
        .filter(|c| {
            saturating_micros_f64((c.sum_nll / c.tokens as f64).exp())
                >= qualifying_chunk_floor_micros
        })
        .map(|c| c.tokens)
        .sum();

    ChunkedPerplexityAggregate {
        representative_perplexity_micros,
        peak_perplexity_micros,
        tail_fraction_micros: saturating_micros_f64(total_tail as f64 / total_tokens as f64),
        tokens_scored: total_tokens,
        qualifying_token_fraction_micros: saturating_micros_f64(
            qualifying_tokens as f64 / total_tokens as f64,
        ),
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
        let agg = aggregate_chunked_perplexity(&[c], 1, TEST_FLOOR_MICROS);
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
        let agg = aggregate_chunked_perplexity(&[a, b], 1, TEST_FLOOR_MICROS);
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
        let agg = aggregate_chunked_perplexity(&[a, b, c], 64, TEST_FLOOR_MICROS);
        let want = (3.0_f64.exp() * 1_000_000.0) as u64;
        assert!(agg.peak_perplexity_micros.abs_diff(want) <= 2);
    }

    #[test]
    fn peak_falls_back_to_representative_when_no_chunk_is_eligible() {
        let a = chunk(10.0, 10, 0, vec![]);
        let agg = aggregate_chunked_perplexity(&[a], 64, TEST_FLOOR_MICROS);
        assert_eq!(
            agg.peak_perplexity_micros,
            agg.representative_perplexity_micros
        );
    }

    /// Perplexity floor used by the qualifying-mass tests: e^2 ~= 7.389.
    /// Chunks at mean_nll 3.0 (ppl ~20.1) clear it; chunks at mean_nll 0.5
    /// (ppl ~1.65) do not.
    const TEST_FLOOR_MICROS: u64 = 7_389_056;

    #[test]
    fn every_chunk_clearing_the_floor_qualifies_the_whole_trace() {
        let a = chunk(300.0, 100, 0, vec![]);
        let b = chunk(600.0, 200, 0, vec![]);
        let agg = aggregate_chunked_perplexity(&[a, b], 1, TEST_FLOOR_MICROS);
        assert_eq!(agg.qualifying_token_fraction_micros, 1_000_000);
    }

    #[test]
    fn no_chunk_clearing_the_floor_qualifies_nothing() {
        let a = chunk(50.0, 100, 0, vec![]);
        let b = chunk(100.0, 200, 0, vec![]);
        let agg = aggregate_chunked_perplexity(&[a, b], 1, TEST_FLOOR_MICROS);
        assert_eq!(agg.qualifying_token_fraction_micros, 0);
    }

    /// The statistic weights by tokens, not by chunk count. One large
    /// qualifying chunk against many small failing ones must score high;
    /// counting chunks would score it low. This is the test that separates
    /// the two readings.
    #[test]
    fn qualifying_mass_is_token_weighted_not_chunk_counted() {
        let big = chunk(2_700.0, 900, 0, vec![]);
        let small = || chunk(50.0, 100, 0, vec![]);
        let agg =
            aggregate_chunked_perplexity(&[big, small(), small(), small()], 1, TEST_FLOOR_MICROS);
        // 900 qualifying of 1200 total = 0.75. Chunk-counting would say 0.25.
        assert_eq!(agg.qualifying_token_fraction_micros, 750_000);
    }

    /// No `min_chunk_tokens` guard, unlike peak: token weighting already
    /// shrinks a tiny chunk to near-nothing, so a second knob would only add
    /// a way to be wrong. A 4-token qualifying fragment among 996 failing
    /// tokens is 0.4%, not an admission.
    #[test]
    fn a_tiny_qualifying_chunk_is_weighted_down_rather_than_guarded_out() {
        let bulk = chunk(498.0, 996, 0, vec![]);
        let fragment = chunk(40.0, 4, 0, vec![]);
        let agg = aggregate_chunked_perplexity(&[bulk, fragment], 64, TEST_FLOOR_MICROS);
        assert_eq!(agg.qualifying_token_fraction_micros, 4_000);
    }

    #[test]
    fn single_chunk_qualifying_mass_is_the_chunks_own_indicator() {
        let clears =
            aggregate_chunked_perplexity(&[chunk(300.0, 100, 0, vec![])], 1, TEST_FLOOR_MICROS);
        assert_eq!(clears.qualifying_token_fraction_micros, 1_000_000);
        let fails =
            aggregate_chunked_perplexity(&[chunk(50.0, 100, 0, vec![])], 1, TEST_FLOOR_MICROS);
        assert_eq!(fails.qualifying_token_fraction_micros, 0);
    }

    /// Degenerate input collapses to zero, matching this module's fail-closed
    /// convention for every other field.
    #[test]
    fn degenerate_input_qualifies_nothing() {
        let empty = aggregate_chunked_perplexity(&[], 1, TEST_FLOOR_MICROS);
        assert_eq!(empty.qualifying_token_fraction_micros, 0);
        let zero_tokens =
            aggregate_chunked_perplexity(&[chunk(0.0, 0, 0, vec![])], 1, TEST_FLOOR_MICROS);
        assert_eq!(zero_tokens.qualifying_token_fraction_micros, 0);
    }

    #[test]
    fn qualifying_mass_is_invariant_to_chunk_order() {
        let a = chunk(300.0, 100, 0, vec![]);
        let b = chunk(50.0, 100, 0, vec![]);
        let c = chunk(900.0, 300, 0, vec![]);
        let forward =
            aggregate_chunked_perplexity(&[a.clone(), b.clone(), c.clone()], 1, TEST_FLOOR_MICROS);
        let reversed = aggregate_chunked_perplexity(&[c, b, a], 1, TEST_FLOOR_MICROS);
        assert_eq!(
            forward.qualifying_token_fraction_micros,
            reversed.qualifying_token_fraction_micros
        );
    }

    /// The property the whole design rests on.
    ///
    /// Hold substantive content fixed (mean_nll 3.0) and pad with boilerplate
    /// (mean_nll 0.5). Qualifying mass equals the substantive token share
    /// exactly. The representative is `exp` of a token-weighted mean log
    /// perplexity, so it is monotone in composition but exponentially
    /// compressed at the low end: a quarter-substantive trace sits under 10%
    /// of the way up its range, where a fixed floor cannot separate it from
    /// pure boilerplate.
    ///
    /// Asserting both halves means the test fails if the new statistic is not
    /// linear in composition, OR if the mean resolves the low end better than
    /// the design claims.
    #[test]
    fn qualifying_mass_resolves_composition_where_the_mean_is_compressed() {
        // 100 tokens per chunk; `substantive` of `total` chunks clear the floor.
        let mix = |substantive: usize, total: usize| {
            let mut chunks = Vec::new();
            for i in 0..total {
                if i < substantive {
                    chunks.push(chunk(300.0, 100, 0, vec![]));
                } else {
                    chunks.push(chunk(50.0, 100, 0, vec![]));
                }
            }
            aggregate_chunked_perplexity(&chunks, 1, TEST_FLOOR_MICROS)
        };

        let none = mix(0, 4);
        let all = mix(4, 4);
        let quarter = mix(1, 4);

        // Mass is linear in composition: exactly the substantive share.
        assert_eq!(none.qualifying_token_fraction_micros, 0);
        assert_eq!(quarter.qualifying_token_fraction_micros, 250_000);
        assert_eq!(all.qualifying_token_fraction_micros, 1_000_000);

        // The mean traverses a disproportionately small share of its own
        // range over that same interval.
        let lo = none.representative_perplexity_micros as f64;
        let hi = all.representative_perplexity_micros as f64;
        let at_quarter = quarter.representative_perplexity_micros as f64;
        let travelled = (at_quarter - lo) / (hi - lo);
        assert!(
            travelled < 0.10,
            "a quarter-substantive trace should sit under 10% of the mean's \
             range; got {travelled:.4}"
        );
    }

    #[test]
    fn tail_fraction_is_exact_over_all_tokens() {
        // 1 tail token of 4 + 3 tail tokens of 6 = 4/10 = 0.4.
        let a = chunk(4.0, 4, 1, vec![]);
        let b = chunk(6.0, 6, 3, vec![]);
        let agg = aggregate_chunked_perplexity(&[a, b], 1, TEST_FLOOR_MICROS);
        assert_eq!(agg.tail_fraction_micros, 400_000);
    }

    #[test]
    fn zero_total_tokens_is_all_zero() {
        let agg = aggregate_chunked_perplexity(&[chunk(0.0, 0, 0, vec![])], 64, TEST_FLOOR_MICROS);
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
