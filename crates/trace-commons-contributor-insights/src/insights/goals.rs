//! One week's figures, as the lever and the weekly summary card read them
//! (feed T only). The user-set threshold goals that also read them left
//! Insights (product owner, 2026-10-29): Goals are now the `/goal` goals
//! set in Claude and Codex, in `slash_goals`.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use super::patterns::{LongContextFigure, PatternFigure, PatternKind, SessionPatterns};
use super::week_rollup::AnalyticsSource;
use super::week_rollup::{WeekRollup, comparable};

/// One week's figures, as the lever and the summary card read them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeekFigures {
    pub week_start: NaiveDate,
    pub comparable: bool,
    /// Per source; a source with no known figure is absent, never zero.
    pub tokens: BTreeMap<AnalyticsSource, u64>,
    pub cache_share_permille: BTreeMap<AnalyticsSource, u64>,
    /// Per kind; absent when unknown or when no session had tool calls.
    pub patterns: BTreeMap<PatternKind, u64>,
    /// Sessions in the week, of any coverage state.
    #[serde(default)]
    pub sessions: u32,
    /// Occurrences per kind (reads, calls, cycles, long-context turns);
    /// absent beside an absent figure.
    #[serde(default)]
    pub pattern_counts: BTreeMap<PatternKind, u32>,
    /// Files read again; `None` when no Claude Code session is counted.
    #[serde(default)]
    pub reread_files: Option<u32>,
    /// Sessions whose context reached the user's own threshold. `None` while
    /// the user has set none: the threshold has no default.
    #[serde(default)]
    pub past_threshold: Option<ThresholdCount>,
}

/// "{n} sessions passed your {threshold} threshold."
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThresholdCount {
    pub threshold: u64,
    pub sessions: u32,
}

impl WeekFigures {
    /// Figures for a rolled-up week and the patterns of its Claude sessions.
    /// Pattern estimates are summed in bytes and rounded once, for the week.
    pub fn from_rollup(rollup: &WeekRollup, sessions: &[&SessionPatterns]) -> Self {
        let mut figures = WeekFigures {
            week_start: rollup.week_start,
            comparable: comparable(rollup).is_ok(),
            sessions: rollup.coverage.sessions(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::week_rollup::{
        AnalyticsSource, CodexObserved, Feed, SessionBody, SessionInput, week_rollup,
    };
    use chrono::{Duration, NaiveDate, TimeZone, Utc};

    fn monday(weeks_ago: i64) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 5).unwrap() - Duration::days(7 * weeks_ago)
    }

    #[test]
    fn figures_come_from_the_rollup_and_merged_patterns() {
        let at = Utc.with_ymd_and_hms(2026, 10, 6, 9, 0, 0).unwrap();
        let input = [SessionInput {
            session_ref: "c".into(),
            import_seq: 1,
            placed_at: None,
            harness_session: None,
            automated: false,
            facts: None,
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
                series: None,
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
