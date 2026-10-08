//! User-set goals, each evaluated per comparable week and shown as weekly
//! marks. No run of weeks is counted (owner decision D1). Goals compare a
//! week only with the user's own past weeks, and only under feed T.

use std::collections::BTreeMap;

use chrono::{Duration, NaiveDate};
use serde::{Deserialize, Serialize};

use super::analytics_constants::WEEKLY_MARKS;
use super::patterns::{LongContextFigure, PatternFigure, PatternKind, SessionPatterns};
use super::week_rollup::{AnalyticsSource, Feed, Unavailable};
use super::week_rollup::{WeekRollup, comparable};

/// One week's figures, as goals and the lever read them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeekFigures {
    pub week_start: NaiveDate,
    pub comparable: bool,
    /// Per source; a source with no known figure is absent, never zero.
    pub tokens: BTreeMap<AnalyticsSource, u64>,
    pub cache_share_permille: BTreeMap<AnalyticsSource, u64>,
    /// Per kind; absent when unknown or when no session had tool calls.
    pub patterns: BTreeMap<PatternKind, u64>,
}

impl WeekFigures {
    /// Figures for a rolled-up week and the patterns of its Claude sessions.
    /// Pattern estimates are summed in bytes and rounded once, for the week.
    pub fn from_rollup(rollup: &WeekRollup, sessions: &[&SessionPatterns]) -> Self {
        let mut figures = WeekFigures {
            week_start: rollup.week_start,
            comparable: comparable(rollup).is_ok(),
            ..WeekFigures::default()
        };
        for line in &rollup.sources {
            if let Some(tokens) = line.tokens {
                figures.tokens.insert(line.source, tokens);
            }
            if let Some(share) = line.cache_share {
                figures
                    .cache_share_permille
                    .insert(line.source, share.permille());
            }
        }
        if !sessions.is_empty() {
            let mut merged = SessionPatterns::default();
            let mut edit_fail_edit: Option<PatternFigure> = None;
            let mut long_context = LongContextFigure::default();
            for session in sessions {
                merged.repeated_reads.merge(&session.repeated_reads);
                merged.retried_calls.merge(&session.retried_calls);
                if let Some(figure) = &session.edit_fail_edit {
                    edit_fail_edit
                        .get_or_insert_with(PatternFigure::default)
                        .merge(figure);
                }
                long_context.merge(&session.long_context);
            }
            merged.edit_fail_edit = edit_fail_edit;
            merged.long_context = long_context;
            for kind in PatternKind::ALL {
                if let Some(figure) = merged.figure(kind) {
                    figures.patterns.insert(kind, figure);
                }
            }
        }
        figures
    }
}

/// A user-set threshold. Claude and Codex figures are never combined, so the
/// source-scoped goals name their source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Goal {
    CacheShareAtLeast {
        source: AnalyticsSource,
        permille: u64,
    },
    RepeatedReadsUnder {
        tokens: u64,
    },
    LongContextUnder {
        tokens: u64,
    },
    WeeklyTokensUnder {
        source: AnalyticsSource,
        tokens: u64,
    },
}

impl Goal {
    fn figure(&self, week: &WeekFigures) -> Option<u64> {
        match self {
            Self::CacheShareAtLeast { source, .. } => {
                week.cache_share_permille.get(source).copied()
            }
            Self::RepeatedReadsUnder { .. } => {
                week.patterns.get(&PatternKind::RepeatedReads).copied()
            }
            Self::LongContextUnder { .. } => week.patterns.get(&PatternKind::LongContext).copied(),
            Self::WeeklyTokensUnder { source, .. } => week.tokens.get(source).copied(),
        }
    }

    fn met(&self, figure: u64) -> bool {
        match self {
            Self::CacheShareAtLeast { permille, .. } => figure >= *permille,
            Self::RepeatedReadsUnder { tokens }
            | Self::LongContextUnder { tokens }
            | Self::WeeklyTokensUnder { tokens, .. } => figure < *tokens,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeekMark {
    Met,
    NotMet,
    /// Not comparable, or no figure. Never read as not met.
    NoFigure,
}

/// "Down from {x} last week." / "Up from {x} last week."
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "direction", rename_all = "snake_case")]
pub enum GoalChange {
    Down { from: u64 },
    Up { from: u64 },
    Same { from: u64 },
}

/// Marks for the last [`WEEKLY_MARKS`] calendar weeks, oldest first, and
/// the change from last week. Deliberately no count of weeks in a row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalMarks {
    pub marks: Vec<WeekMark>,
    pub change: Option<GoalChange>,
}

/// Evaluate a goal over `weeks`, oldest first, whose last entry is the week
/// being shown.
pub fn goal_marks(
    feed: Feed,
    goal: &Goal,
    weeks: &[WeekFigures],
) -> Result<GoalMarks, Unavailable> {
    if feed != Feed::CounterPass {
        return Err(Unavailable::NeedsCounterPass);
    }
    let Some(this) = weeks.last() else {
        return Err(Unavailable::InsufficientHistory);
    };
    let figure_at = |offset: usize| {
        let start = this.week_start - Duration::days(7 * offset as i64);
        weeks
            .iter()
            .find(|week| week.week_start == start)
            .filter(|week| week.comparable)
            .and_then(|week| goal.figure(week))
    };
    let marks = (0..WEEKLY_MARKS)
        .rev()
        .map(|offset| match figure_at(offset) {
            Some(figure) if goal.met(figure) => WeekMark::Met,
            Some(_) => WeekMark::NotMet,
            None => WeekMark::NoFigure,
        })
        .collect();
    let change = match (figure_at(0), figure_at(1)) {
        (Some(now), Some(from)) => Some(match now.cmp(&from) {
            std::cmp::Ordering::Less => GoalChange::Down { from },
            std::cmp::Ordering::Greater => GoalChange::Up { from },
            std::cmp::Ordering::Equal => GoalChange::Same { from },
        }),
        _ => None,
    };
    Ok(GoalMarks { marks, change })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::week_rollup::{
        AnalyticsSource, CodexObserved, SessionBody, SessionInput, week_rollup,
    };
    use chrono::{Duration, NaiveDate, TimeZone, Utc};

    fn monday(weeks_ago: i64) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 5).unwrap() - Duration::days(7 * weeks_ago)
    }

    fn week(weeks_ago: i64, comparable: bool, tokens: Option<u64>) -> WeekFigures {
        let mut figures = WeekFigures {
            week_start: monday(weeks_ago),
            comparable,
            ..WeekFigures::default()
        };
        if let Some(tokens) = tokens {
            figures.tokens.insert(AnalyticsSource::ClaudeCode, tokens);
            figures
                .cache_share_permille
                .insert(AnalyticsSource::ClaudeCode, tokens / 1_000);
            figures.patterns.insert(PatternKind::RepeatedReads, tokens);
            figures.patterns.insert(PatternKind::LongContext, tokens);
        }
        figures
    }

    fn under(tokens: u64) -> Goal {
        Goal::WeeklyTokensUnder {
            source: AnalyticsSource::ClaudeCode,
            tokens,
        }
    }

    #[test]
    fn goals_need_the_counter_pass_feed() {
        for feed in [Feed::Saved, Feed::Ledger] {
            assert_eq!(
                goal_marks(feed, &under(10), &[week(0, true, Some(5))]),
                Err(Unavailable::NeedsCounterPass)
            );
        }
    }

    #[test]
    fn six_marks_oldest_first_padded_with_no_figure() {
        let marks = goal_marks(
            Feed::CounterPass,
            &under(100),
            &[week(1, true, Some(50)), week(0, true, Some(500))],
        )
        .unwrap();
        assert_eq!(marks.marks.len(), WEEKLY_MARKS);
        assert_eq!(
            marks.marks,
            vec![
                WeekMark::NoFigure,
                WeekMark::NoFigure,
                WeekMark::NoFigure,
                WeekMark::NoFigure,
                WeekMark::Met,
                WeekMark::NotMet,
            ]
        );
    }

    #[test]
    fn only_the_last_six_weeks_are_marked() {
        let weeks: Vec<_> = (0..9).rev().map(|ago| week(ago, true, Some(50))).collect();
        let marks = goal_marks(Feed::CounterPass, &under(100), &weeks).unwrap();
        assert_eq!(marks.marks, vec![WeekMark::Met; WEEKLY_MARKS]);
    }

    #[test]
    fn a_week_that_is_not_comparable_or_has_no_figure_is_never_not_met() {
        let marks = goal_marks(
            Feed::CounterPass,
            &under(100),
            &[week(1, false, Some(500)), week(0, true, None)],
        )
        .unwrap();
        assert_eq!(marks.marks[4], WeekMark::NoFigure);
        assert_eq!(marks.marks[5], WeekMark::NoFigure);
    }

    #[test]
    fn thresholds_compare_exactly() {
        let at_least = Goal::CacheShareAtLeast {
            source: AnalyticsSource::ClaudeCode,
            permille: 500,
        };
        let met = goal_marks(
            Feed::CounterPass,
            &at_least,
            &[week(0, true, Some(500_000))],
        )
        .unwrap();
        assert_eq!(met.marks[5], WeekMark::Met);
        let not = goal_marks(
            Feed::CounterPass,
            &at_least,
            &[week(0, true, Some(499_000))],
        )
        .unwrap();
        assert_eq!(not.marks[5], WeekMark::NotMet);

        let equal =
            goal_marks(Feed::CounterPass, &under(100), &[week(0, true, Some(100))]).unwrap();
        assert_eq!(equal.marks[5], WeekMark::NotMet);
        for goal in [
            Goal::RepeatedReadsUnder { tokens: 101 },
            Goal::LongContextUnder { tokens: 101 },
        ] {
            let marks = goal_marks(Feed::CounterPass, &goal, &[week(0, true, Some(100))]).unwrap();
            assert_eq!(marks.marks[5], WeekMark::Met, "{goal:?}");
        }
    }

    #[test]
    fn change_needs_both_weeks_comparable() {
        let down = goal_marks(
            Feed::CounterPass,
            &under(1_000),
            &[week(1, true, Some(800)), week(0, true, Some(600))],
        )
        .unwrap();
        assert_eq!(down.change, Some(GoalChange::Down { from: 800 }));
        let up = goal_marks(
            Feed::CounterPass,
            &under(1_000),
            &[week(1, true, Some(600)), week(0, true, Some(800))],
        )
        .unwrap();
        assert_eq!(up.change, Some(GoalChange::Up { from: 600 }));
        let thin = goal_marks(
            Feed::CounterPass,
            &under(1_000),
            &[week(1, false, Some(600)), week(0, true, Some(800))],
        )
        .unwrap();
        assert_eq!(thin.change, None);
        // A gap week in between is not "last week".
        let gap = goal_marks(
            Feed::CounterPass,
            &under(1_000),
            &[week(2, true, Some(600)), week(0, true, Some(800))],
        )
        .unwrap();
        assert_eq!(gap.change, None);
    }

    #[test]
    fn marks_carry_no_run_count() {
        let marks = goal_marks(Feed::CounterPass, &under(1), &[week(0, true, Some(5))]).unwrap();
        let value = serde_json::to_value(&marks).unwrap();
        let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, vec!["change".to_string(), "marks".to_string()]);
    }

    #[test]
    fn figures_come_from_the_rollup_and_merged_patterns() {
        let at = Utc.with_ymd_and_hms(2026, 10, 6, 9, 0, 0).unwrap();
        let input = [SessionInput {
            session_ref: "c".into(),
            import_seq: 1,
            placed_at: None,
            body: SessionBody::Codex {
                observed: Some(CodexObserved {
                    first_at: at,
                    last_at: at,
                    input: 1_000,
                    cached_input: 250,
                    output: 10,
                    total: 1_010,
                    baseline_excluded: false,
                }),
            },
        }];
        let rollup = week_rollup(Feed::CounterPass, &input, monday(0), &Utc);
        let figures = WeekFigures::from_rollup(&rollup, &[]);
        assert!(!figures.comparable);
        assert_eq!(figures.tokens[&AnalyticsSource::Codex], 1_010);
        assert_eq!(figures.cache_share_permille[&AnalyticsSource::Codex], 250);
        assert_eq!(figures.patterns.get(&PatternKind::RepeatedReads), None);
    }
}
