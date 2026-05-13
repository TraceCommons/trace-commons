// Pure-function scoring primitives for the A2.1 perplexity-scorer bake-off.
// The decision rule (in a later slice) takes a weighted combination of these
// numbers across candidates to pick a winner. Keep them allocation-light and
// dependency-free so they can run inside calibration without dragging in the
// scoring runtime; the O(N*M) AUC is intentional because bake-off corpora are
// small and we want the algorithm legible against the spec rather than fast.

use std::cmp::Ordering;

/// Probability that a randomly drawn novel sample has higher perplexity than a
/// randomly drawn duplicate sample. Ties contribute 0.5 each (Mann–Whitney U
/// convention). Empty inputs return 0.5 — no information, no preference.
pub fn discrimination_auc(novel: &[f64], duplicate: &[f64]) -> f64 {
    if novel.is_empty() || duplicate.is_empty() {
        return 0.5;
    }
    let mut wins = 0.0_f64;
    for &n in novel {
        for &d in duplicate {
            wins += match n.partial_cmp(&d).unwrap_or(Ordering::Equal) {
                Ordering::Greater => 1.0,
                Ordering::Equal => 0.5,
                Ordering::Less => 0.0,
            };
        }
    }
    wins / (novel.len() as f64 * duplicate.len() as f64)
}

/// Median absolute relative perplexity delta between original and paraphrase.
/// `orig == 0.0` collapses to 0.0 for that pair — a zero-perplexity original
/// would mean nothing was scored, so the paraphrase delta isn't meaningful
/// and we shouldn't divide by zero either.
pub fn paraphrase_delta(pairs: &[(f64, f64)]) -> f64 {
    if pairs.is_empty() {
        return 0.0;
    }
    let mut deltas: Vec<f64> = pairs
        .iter()
        .map(|(orig, para)| {
            if *orig == 0.0 {
                0.0
            } else {
                (orig - para).abs() / orig.abs()
            }
        })
        .collect();
    median_inplace(&mut deltas)
}

/// Absolute difference of medians of `fraction_below_cutoff` between slices.
/// Larger spread means the candidate is sorting novel and duplicate traces
/// into different tail regions, which is what we want from a perplexity gate.
pub fn tail_fraction_range(novel: &[f64], duplicate: &[f64]) -> f64 {
    let mut n = novel.to_vec();
    let mut d = duplicate.to_vec();
    let med_n = median_inplace(&mut n);
    let med_d = median_inplace(&mut d);
    (med_d - med_n).abs()
}

// Sorts `values` in place and returns the median. Returns 0.0 on empty input
// so callers don't have to special-case missing slices at every site.
fn median_inplace(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        values[mid]
    } else {
        (values[mid - 1] + values[mid]) / 2.0
    }
}
