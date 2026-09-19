// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Aggregate-only summaries of re-score results, for floor calibration.
//!
//! A dry-run re-score computes what production WOULD store for every
//! decided submission and writes none of it. What an operator needs from
//! that is a distribution, not rows. Everything here returns aggregates
//! only, and refuses to return even those below [`MIN_ROWS_FOR_PERCENTILES`]:
//! a percentile of three rows is a per-submission score with extra steps,
//! and the hash-only convention forbids logging those.

/// Below this many rows nothing but the count is reported.
pub const MIN_ROWS_FOR_PERCENTILES: usize = 20;

/// Nearest-rank percentiles: every reported value is one that was observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Percentiles {
    pub min: u64,
    pub p05: u64,
    pub p10: u64,
    pub p25: u64,
    pub p50: u64,
    pub p75: u64,
    pub p90: u64,
    pub p95: u64,
    pub max: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct DistributionSummary {
    pub count: usize,
    /// `None` below [`MIN_ROWS_FOR_PERCENTILES`].
    pub percentiles: Option<Percentiles>,
}

pub fn summarize(values: &[u64]) -> DistributionSummary {
    let count = values.len();
    if count < MIN_ROWS_FOR_PERCENTILES {
        return DistributionSummary {
            count,
            percentiles: None,
        };
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    // Nearest rank: the ceil(p/100 * n)-th smallest, 1-indexed. Integer
    // arithmetic, so there is no float rounding to disagree about.
    let at = |pct: usize| sorted[(pct * count).div_ceil(100).clamp(1, count) - 1];
    DistributionSummary {
        count,
        percentiles: Some(Percentiles {
            min: sorted[0],
            p05: at(5),
            p10: at(10),
            p25: at(25),
            p50: at(50),
            p75: at(75),
            p90: at(90),
            p95: at(95),
            max: sorted[count - 1],
        }),
    }
}

/// Share of `values` STRICTLY below `floor`, in micros -- the share a gate
/// predicate of `value >= floor` would refuse. `None` below
/// [`MIN_ROWS_FOR_PERCENTILES`], for the same reason percentiles are.
pub fn share_below_micros(values: &[u64], floor: u64) -> Option<u64> {
    if values.len() < MIN_ROWS_FOR_PERCENTILES {
        return None;
    }
    let below = values.iter().filter(|v| **v < floor).count() as u64;
    Some(below * 1_000_000 / values.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_are_nearest_rank_over_the_sorted_values() {
        let mut values: Vec<u64> = (1..=100).collect();
        values.reverse();
        let s = summarize(&values);
        assert_eq!(s.count, 100);
        let p = s.percentiles.expect("100 rows is enough");
        assert_eq!(
            (p.p05, p.p10, p.p25, p.p50, p.p75, p.p90, p.p95),
            (5, 10, 25, 50, 75, 90, 95)
        );
        assert_eq!((p.min, p.max), (1, 100));
    }

    #[test]
    fn nearest_rank_rounds_up_so_a_percentile_is_always_an_observed_value() {
        let values: Vec<u64> = (1..=20).map(|v| v * 10).collect();
        let p = summarize(&values).percentiles.unwrap();
        // ceil(0.05 * 20) = 1st, ceil(0.50 * 20) = 10th, ceil(0.95 * 20) = 19th.
        assert_eq!((p.p05, p.p50, p.p95), (10, 100, 190));
    }

    #[test]
    fn too_few_rows_report_a_count_and_nothing_else() {
        let values: Vec<u64> = (1..MIN_ROWS_FOR_PERCENTILES as u64).collect();
        let s = summarize(&values);
        assert_eq!(s.count, MIN_ROWS_FOR_PERCENTILES - 1);
        assert_eq!(s.percentiles, None);
        assert_eq!(share_below_micros(&values, 5), None);
    }

    #[test]
    fn no_rows_is_a_zero_count() {
        let s = summarize(&[]);
        assert_eq!((s.count, s.percentiles), (0, None));
    }

    #[test]
    fn share_below_is_strict_and_in_micros() {
        let values: Vec<u64> = (1..=100).collect();
        // 1..=25 are strictly below 26: a row AT the floor passes it, as in
        // the gate's `perplexity >= floor` predicate.
        assert_eq!(share_below_micros(&values, 26), Some(250_000));
        assert_eq!(share_below_micros(&values, 1), Some(0));
        assert_eq!(share_below_micros(&values, 1_000), Some(1_000_000));
    }
}
