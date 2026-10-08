//! The lever of the week: the pattern kind that rose most against its own
//! recent baseline. An observation only; no advice is attached (owner
//! decision D2). Feed T only.
//!
//! Kinds are ranked by ratio, never by raw figure: their units differ
//! (result-size estimates against whole-context sums).

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use super::analytics_constants::{
    LEVER_BASELINE_WEEKS, LEVER_DISMISSALS_TO_DISABLE, LEVER_FLOOR_TOKENS, LEVER_RATIO,
    LEVER_SUPPRESSION_WEEKS,
};
use super::goals::WeekFigures;
use super::patterns::PatternKind;
use super::week_rollup::{Feed, Unavailable};

/// "Not useful" on a kind, stored as the kind and ISO week only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeverDismissal {
    pub kind: PatternKind,
    pub week_start: NaiveDate,
}

/// A kind turned back on in Settings, as the kind and ISO week only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeverReenable {
    pub kind: PatternKind,
    pub week_start: NaiveDate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeverPick {
    pub kind: PatternKind,
    pub figure: u64,
    /// Twice the baseline median, so an even-count median stays an integer.
    pub baseline_twice_median: u64,
    /// figure / median, per mille, rounded half up.
    pub ratio_permille: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeverOutcome {
    pub pick: Option<LeverPick>,
    /// Kinds off after repeated dismissals, until re-enabled in Settings.
    pub disabled: Vec<PatternKind>,
}

/// Pick the lever for `this` week from `earlier` weeks (any order).
///
/// A kind's baseline is the median of its figure over the
/// [`LEVER_BASELINE_WEEKS`] most recent earlier comparable weeks that have
/// it; with an even count the median is the mean of the middle two. A
/// candidate's figure reaches [`LEVER_FLOOR_TOKENS`] and its ratio reaches
/// [`LEVER_RATIO`]; the highest ratio wins, ties going to the fixed kind
/// order. A dismissal in week W suppresses its kind from W through
/// W + [`LEVER_SUPPRESSION_WEEKS`]; [`LEVER_DISMISSALS_TO_DISABLE`]
/// dismissals since the kind was last re-enabled turn it off.
pub fn lever(
    feed: Feed,
    this: &WeekFigures,
    earlier: &[WeekFigures],
    dismissals: &[LeverDismissal],
    reenables: &[LeverReenable],
) -> Result<LeverOutcome, Unavailable> {
    if feed != Feed::CounterPass {
        return Err(Unavailable::NeedsCounterPass);
    }
    if !this.comparable {
        return Err(Unavailable::BelowCoverageFloor);
    }
    let mut history: Vec<&WeekFigures> = earlier
        .iter()
        .filter(|week| week.comparable && week.week_start < this.week_start)
        .collect();
    history.sort_by(|a, b| b.week_start.cmp(&a.week_start));

    let mut disabled = Vec::new();
    let mut any_baseline = false;
    let mut best: Option<LeverPick> = None;
    for kind in PatternKind::ALL {
        let mut baseline: Vec<u64> = history
            .iter()
            .filter_map(|week| week.patterns.get(&kind).copied())
            .take(LEVER_BASELINE_WEEKS)
            .collect();
        if baseline.len() < LEVER_BASELINE_WEEKS {
            continue;
        }
        any_baseline = true;
        let reenabled_at = reenables
            .iter()
            .filter(|r| r.kind == kind)
            .map(|r| r.week_start)
            .max();
        let kind_dismissals: Vec<NaiveDate> = dismissals
            .iter()
            .filter(|d| d.kind == kind && reenabled_at.is_none_or(|at| d.week_start > at))
            .map(|d| d.week_start)
            .collect();
        if kind_dismissals.len() >= LEVER_DISMISSALS_TO_DISABLE {
            disabled.push(kind);
            continue;
        }
        let suppressed = kind_dismissals.iter().any(|week| {
            let weeks_since = (this.week_start - *week).num_days().div_euclid(7);
            (0..=LEVER_SUPPRESSION_WEEKS).contains(&weeks_since)
        });
        if suppressed {
            continue;
        }
        let Some(figure) = this.patterns.get(&kind).copied() else {
            continue;
        };
        baseline.sort_unstable();
        let middle = baseline.len() / 2;
        let twice_median = if baseline.len() % 2 == 0 {
            baseline[middle - 1] + baseline[middle]
        } else {
            baseline[middle] * 2
        };
        if figure < LEVER_FLOOR_TOKENS || !LEVER_RATIO.reached_by(figure * 2, twice_median) {
            continue;
        }
        let candidate = LeverPick {
            kind,
            figure,
            baseline_twice_median: twice_median,
            ratio_permille: ((u128::from(figure) * 4_000 + u128::from(twice_median))
                / (u128::from(twice_median) * 2)) as u64,
        };
        // Strictly higher ratio replaces; a tie keeps the earlier kind.
        let higher = best.is_none_or(|current| {
            u128::from(candidate.figure) * u128::from(current.baseline_twice_median)
                > u128::from(current.figure) * u128::from(candidate.baseline_twice_median)
        });
        if higher {
            best = Some(candidate);
        }
    }
    if !any_baseline {
        return Err(Unavailable::InsufficientHistory);
    }
    Ok(LeverOutcome {
        pick: best,
        disabled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::analytics_constants::Ratio;
    use chrono::{Duration, NaiveDate};

    fn monday(weeks_ago: i64) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 5).unwrap() - Duration::days(7 * weeks_ago)
    }

    fn week(weeks_ago: i64, comparable: bool, figures: &[(PatternKind, u64)]) -> WeekFigures {
        WeekFigures {
            week_start: monday(weeks_ago),
            comparable,
            patterns: figures.iter().copied().collect(),
            ..WeekFigures::default()
        }
    }

    const READS: PatternKind = PatternKind::RepeatedReads;
    const LONG: PatternKind = PatternKind::LongContext;

    // Baseline for READS: 100K, 200K, 300K, 1M -> median 250K.
    fn history() -> Vec<WeekFigures> {
        vec![
            week(4, true, &[(READS, 1_000_000)]),
            week(3, true, &[(READS, 300_000)]),
            week(2, true, &[(READS, 100_000)]),
            week(1, true, &[(READS, 200_000)]),
        ]
    }

    fn pick(this: u64) -> Option<LeverPick> {
        lever(
            Feed::CounterPass,
            &week(0, true, &[(READS, this)]),
            &history(),
            &[],
            &[],
        )
        .unwrap()
        .pick
    }

    #[test]
    fn the_lever_needs_feed_t_and_a_comparable_week() {
        let this = week(0, true, &[(READS, 400_000)]);
        assert_eq!(
            lever(Feed::Saved, &this, &history(), &[], &[]),
            Err(Unavailable::NeedsCounterPass)
        );
        let thin = week(0, false, &[(READS, 400_000)]);
        assert_eq!(
            lever(Feed::CounterPass, &thin, &history(), &[], &[]),
            Err(Unavailable::BelowCoverageFloor)
        );
    }

    #[test]
    fn the_baseline_needs_four_earlier_comparable_weeks() {
        assert_eq!(LEVER_BASELINE_WEEKS, 4);
        let mut short = history();
        short[0].comparable = false;
        assert_eq!(
            lever(
                Feed::CounterPass,
                &week(0, true, &[(READS, 400_000)]),
                &short,
                &[],
                &[]
            ),
            Err(Unavailable::InsufficientHistory)
        );
    }

    #[test]
    fn the_baseline_is_the_median_of_the_four_most_recent() {
        let found = pick(500_000).unwrap();
        assert_eq!(found.kind, READS);
        assert_eq!(found.baseline_twice_median, 500_000);
        assert_eq!(found.ratio_permille, 2_000);
        // An older fifth week is ignored.
        let mut longer = history();
        longer.insert(0, week(5, true, &[(READS, 1)]));
        let found = lever(
            Feed::CounterPass,
            &week(0, true, &[(READS, 500_000)]),
            &longer,
            &[],
            &[],
        )
        .unwrap()
        .pick
        .unwrap();
        assert_eq!(found.baseline_twice_median, 500_000);
    }

    #[test]
    fn the_ratio_floor_is_exactly_one_and_a_quarter() {
        assert_eq!(LEVER_RATIO, Ratio::new(5, 4));
        assert!(pick(312_500).is_some());
        assert!(pick(312_499).is_none());
    }

    #[test]
    fn the_figure_floor_is_100k() {
        let low_history = vec![
            week(4, true, &[(READS, 10)]),
            week(3, true, &[(READS, 10)]),
            week(2, true, &[(READS, 10)]),
            week(1, true, &[(READS, 10)]),
        ];
        let run = |figure| {
            lever(
                Feed::CounterPass,
                &week(0, true, &[(READS, figure)]),
                &low_history,
                &[],
                &[],
            )
            .unwrap()
            .pick
        };
        assert_eq!(LEVER_FLOOR_TOKENS, 100_000);
        assert!(run(99_999).is_none());
        assert!(run(100_000).is_some());
    }

    #[test]
    fn a_zero_baseline_is_not_a_candidate() {
        let zero = vec![
            week(4, true, &[(READS, 0)]),
            week(3, true, &[(READS, 0)]),
            week(2, true, &[(READS, 0)]),
            week(1, true, &[(READS, 0)]),
        ];
        let outcome = lever(
            Feed::CounterPass,
            &week(0, true, &[(READS, 900_000)]),
            &zero,
            &[],
            &[],
        )
        .unwrap();
        assert_eq!(outcome.pick, None);
    }

    fn two_kinds() -> Vec<WeekFigures> {
        (1..=4)
            .rev()
            .map(|ago| week(ago, true, &[(READS, 100_000), (LONG, 1_000_000)]))
            .collect()
    }

    #[test]
    fn the_highest_ratio_wins_not_the_biggest_figure() {
        let this = week(0, true, &[(READS, 200_000), (LONG, 1_300_000)]);
        let found = lever(Feed::CounterPass, &this, &two_kinds(), &[], &[])
            .unwrap()
            .pick
            .unwrap();
        assert_eq!(found.kind, READS);
    }

    #[test]
    fn a_tie_goes_to_the_fixed_kind_order() {
        let this = week(0, true, &[(READS, 200_000), (LONG, 2_000_000)]);
        let found = lever(Feed::CounterPass, &this, &two_kinds(), &[], &[])
            .unwrap()
            .pick
            .unwrap();
        assert_eq!(found.kind, READS);
    }

    #[test]
    fn a_dismissed_kind_is_suppressed_for_four_weeks() {
        assert_eq!(LEVER_SUPPRESSION_WEEKS, 4);
        let this = week(0, true, &[(READS, 300_000), (LONG, 1_300_000)]);
        let run = |ago| {
            lever(
                Feed::CounterPass,
                &this,
                &two_kinds(),
                &[LeverDismissal {
                    kind: READS,
                    week_start: monday(ago),
                }],
                &[],
            )
            .unwrap()
            .pick
            .unwrap()
            .kind
        };
        assert_eq!(run(0), LONG);
        assert_eq!(run(4), LONG);
        assert_eq!(run(5), READS);
    }

    #[test]
    fn three_dismissals_turn_a_kind_off_until_re_enabled() {
        assert_eq!(LEVER_DISMISSALS_TO_DISABLE, 3);
        let this = week(0, true, &[(READS, 300_000)]);
        let dismissals: Vec<_> = [20, 14, 8]
            .iter()
            .map(|ago| LeverDismissal {
                kind: READS,
                week_start: monday(*ago),
            })
            .collect();
        let off = lever(Feed::CounterPass, &this, &two_kinds(), &dismissals, &[]).unwrap();
        assert_eq!(off.pick, None);
        assert_eq!(off.disabled, vec![READS]);

        let back = lever(
            Feed::CounterPass,
            &this,
            &two_kinds(),
            &dismissals,
            &[LeverReenable {
                kind: READS,
                week_start: monday(7),
            }],
        )
        .unwrap();
        assert_eq!(back.pick.unwrap().kind, READS);
        assert!(back.disabled.is_empty());
    }

    #[test]
    fn two_dismissals_do_not_turn_a_kind_off() {
        let this = week(0, true, &[(READS, 300_000)]);
        let dismissals: Vec<_> = [20, 14]
            .iter()
            .map(|ago| LeverDismissal {
                kind: READS,
                week_start: monday(*ago),
            })
            .collect();
        let outcome = lever(Feed::CounterPass, &this, &two_kinds(), &dismissals, &[]).unwrap();
        assert_eq!(outcome.pick.unwrap().kind, READS);
    }
}
