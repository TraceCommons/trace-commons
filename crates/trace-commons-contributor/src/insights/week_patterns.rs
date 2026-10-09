//! Patterns ("Where tokens went") for one local ISO week, and the sessions
//! behind each pattern.
//!
//! A week's figures are read from the Claude Code sessions the week's rollup
//! counts with known or partial coverage: tool calls on their turns dated in
//! the week and not already counted in an earlier snapshot, and long-context
//! input on those same turns. Codex records no tool calls, so a week with
//! Codex sessions says how many of its sessions the figures cover.
//!
//! The four figures overlap and are never summed. Repeated reads, retried
//! tool calls and "Edit, failed command, edit" are estimates from result
//! size, summed in bytes over the week and rounded once; long context is
//! counted input. "Edit, failed command, edit" is inferred from the order of
//! tool calls (owner decision D9, open), and is absent when that card is held.
//!
//! Re-read files are named only by a letter and the extension (owner
//! decision D8, open). A letter is the file's position in first-seen order,
//! not a ranking; the table lists the most re-read files first. No keyed
//! digest leaves this module.
//!
//! Week-to-week change needs feed T; any other feed types it as unavailable.
//! A week with no figure draws a gap in the weekly bars, never a zero bar,
//! and no run of weeks is counted (owner decision D1, open).

use chrono::{Duration, FixedOffset, NaiveDate};
use serde::{Deserialize, Serialize};

use super::LocalInsight;
use super::analytics_constants::LONG_CONTEXT_TOKENS;
use super::goals::{ThresholdCount, WeekFigures};
use super::markers::letter_label;
use super::patterns::{
    LongContextFigure, PatternFigure, PatternKind, SessionPatterns, merge_reread_files,
    session_patterns_on_turns,
};
use super::week_glance::{dated_weeks, saved_generation, saved_sessions};
use super::week_rollup::{
    AnalyticsSource, CoverageReason, CoverageState, Feed, SessionBody, SessionInput, Unavailable,
    WeekCoverage, WeekRollup, change_permille, comparable, counted_claude_turns, week_rollup,
};

/// Rows the "Most re-read files" table lists. A bound on the response, not
/// an analytics constant.
pub const MAX_REREAD_ROWS: usize = 5;

/// How a card's token figure was arrived at, for its derivation label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternBasis {
    /// "about, from result size".
    EstimateFromResultSize,
    /// "from counters".
    FromCounters,
}

/// One week's bar. `tokens: None` is a gap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatternWeek {
    pub week_start: NaiveDate,
    pub tokens: Option<u64>,
}

/// One Patterns card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatternCard {
    pub kind: PatternKind,
    /// The headline. `None` when no Claude Code session is counted this week,
    /// or when every counted occurrence's size is unknown.
    pub tokens: Option<u64>,
    /// Occurrences: repeated reads, retried calls, cycles, or long-context
    /// turns. `None` when no Claude Code session is counted this week.
    pub count: Option<u32>,
    /// Repeated reads only: how many files were read again.
    pub files: Option<u32>,
    /// Counted sessions with at least one occurrence ("See {n} sessions").
    pub sessions: u32,
    pub basis: PatternBasis,
    /// Inferred from the order of tool calls (owner decision D9, open).
    pub inferred: bool,
    /// Oldest first, ending at the week on screen.
    pub weeks: Vec<PatternWeek>,
    /// Per mille against the week before, rounded half away from zero.
    pub change: Option<i64>,
    /// Why `change` is absent.
    pub change_unavailable: Option<Unavailable>,
}

/// One row of "Most re-read files": a letter and the extension with its dot
/// (".rs"), never a path, a basename or a digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RereadRow {
    pub letter: String,
    pub ext: Option<String>,
    pub reads: u32,
    /// "After context shrank (inferred)".
    pub after_shrink: u32,
    pub tokens: Option<u64>,
}

/// The Patterns tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeekPatterns {
    pub feed: Feed,
    pub generation: u64,
    pub week_start: NaiveDate,
    pub week_end: NaiveDate,
    pub tz: i32,
    pub coverage: WeekCoverage,
    /// Sessions in the week, of any harness and coverage state.
    pub sessions: u32,
    /// Claude Code sessions whose tool calls and counters are read.
    pub claude_sessions: u32,
    /// A session of another harness is in the week, so the figures cover
    /// "Claude Code sessions only: {k} of {n}."
    pub claude_only: bool,
    /// The long-context threshold the card names.
    pub long_context_threshold: u64,
    /// In fixed order; a held kind is absent.
    pub cards: Vec<PatternCard>,
    /// Most re-read first, at most [`MAX_REREAD_ROWS`].
    pub reread_files: Vec<RereadRow>,
    /// Weeks with a dated saved session, newest first, for the week picker.
    pub weeks: Vec<NaiveDate>,
}

/// One session behind a card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatternSession {
    /// The snapshot ID. Never a path.
    pub session_ref: String,
    pub count: u32,
    pub tokens: Option<u64>,
    pub state: CoverageState,
    pub reasons: Vec<CoverageReason>,
}

/// "See {n} sessions": the counted sessions with an occurrence of one kind,
/// in the engine's order. Never sorted by value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatternSessions {
    pub pattern: PatternKind,
    pub feed: Feed,
    pub generation: u64,
    pub week_start: NaiveDate,
    pub tz: i32,
    pub sessions: Vec<PatternSession>,
}

/// One week: its rollup and the patterns of each Claude Code session it
/// counts, in rollup order.
struct PatternWeekData {
    rollup: WeekRollup,
    sessions: Vec<(usize, SessionPatterns)>,
}

fn week_data(
    feed: Feed,
    sessions: &[SessionInput],
    week_start: NaiveDate,
    tz: &FixedOffset,
) -> PatternWeekData {
    let rollup = week_rollup(feed, sessions, week_start, tz);
    let counted = counted_claude_turns(feed, sessions, week_start, tz);
    let mut found = Vec::new();
    for (row_index, row) in rollup.sessions.iter().enumerate() {
        if row.source != Some(AnalyticsSource::ClaudeCode) || row.state == CoverageState::Unknown {
            continue;
        }
        let Some((index, input)) = sessions
            .iter()
            .enumerate()
            .find(|(_, input)| input.session_ref == row.session_ref)
        else {
            continue;
        };
        let (SessionBody::Claude { series, .. }, Some(turns)) = (&input.body, &counted[index])
        else {
            continue;
        };
        found.push((row_index, session_patterns_on_turns(series, turns)));
    }
    PatternWeekData {
        rollup,
        sessions: found,
    }
}

/// Occurrences of `kind` in one session. `None` while the kind is held.
fn occurrences(found: &SessionPatterns, kind: PatternKind) -> Option<u32> {
    match kind {
        PatternKind::RepeatedReads => Some(found.repeated_reads.count),
        PatternKind::RetriedCalls => Some(found.retried_calls.count),
        PatternKind::EditFailEdit => found.edit_fail_edit.map(|figure| figure.count),
        PatternKind::LongContext => Some(found.long_context.turns),
    }
}

/// The week's merged figures, with estimates rounded once for the week.
/// `None` when no Claude Code session is counted.
fn merged(data: &PatternWeekData) -> Option<SessionPatterns> {
    if data.sessions.is_empty() {
        return None;
    }
    let mut total = SessionPatterns::default();
    let mut cycles: Option<PatternFigure> = None;
    let mut long_context = LongContextFigure::default();
    for (_, found) in &data.sessions {
        total.repeated_reads.merge(&found.repeated_reads);
        total.repeated_reads_after_shrink += found.repeated_reads_after_shrink;
        total.retried_calls.merge(&found.retried_calls);
        if let Some(figure) = &found.edit_fail_edit {
            cycles
                .get_or_insert_with(PatternFigure::default)
                .merge(figure);
        }
        long_context.merge(&found.long_context);
    }
    total.edit_fail_edit = cycles;
    total.long_context = long_context;
    total.reread_files = merge_reread_files(data.sessions.iter().map(|(_, found)| found));
    Some(total)
}

/// The kinds a card is drawn for: every kind except a held one.
fn shown_kinds() -> Vec<PatternKind> {
    PatternKind::ALL
        .into_iter()
        .filter(|kind| {
            *kind != PatternKind::EditFailEdit || super::analytics_constants::EDIT_FAIL_EDIT_CARD
        })
        .collect()
}

/// A bar's figure. Under feed T a week below the coverage floor is a gap.
fn bar(data: &PatternWeekData, kind: PatternKind) -> Option<u64> {
    if data.rollup.feed == Feed::CounterPass && comparable(&data.rollup).is_err() {
        return None;
    }
    merged(data)?.figure(kind)
}

fn change(
    this: &PatternWeekData,
    last: &PatternWeekData,
    kind: PatternKind,
) -> Result<i64, Unavailable> {
    comparable(&this.rollup)?;
    comparable(&last.rollup)?;
    let figure = |data: &PatternWeekData| merged(data).and_then(|found| found.figure(kind));
    let (Some(now), Some(before)) = (figure(this), figure(last)) else {
        return Err(Unavailable::NoFigure);
    };
    change_permille(now, before).ok_or(Unavailable::NoFigure)
}

fn monday_of(date: NaiveDate) -> NaiveDate {
    use chrono::Datelike;
    date - Duration::days(i64::from(date.weekday().num_days_from_monday()))
}

/// The Patterns tab over any feed's sessions, with `bar_weeks` weekly bars.
/// The generation and the week picker are the caller's.
pub fn week_patterns(
    feed: Feed,
    sessions: &[SessionInput],
    week_start: NaiveDate,
    tz: FixedOffset,
    bar_weeks: usize,
) -> WeekPatterns {
    let week_start = monday_of(week_start);
    // The bars' weeks plus the one before the first, oldest first; the week
    // before the week on screen is always among them.
    let span = bar_weeks.max(1);
    let data: Vec<PatternWeekData> = (0..=span)
        .rev()
        .map(|back| {
            week_data(
                feed,
                sessions,
                week_start - Duration::weeks(back as i64),
                &tz,
            )
        })
        .collect();
    let this = data.last().expect("the week on screen");
    let last = &data[data.len() - 2];
    let week = merged(this);
    let cards = shown_kinds()
        .into_iter()
        .map(|kind| {
            let (change, change_unavailable) = match change(this, last, kind) {
                Ok(permille) => (Some(permille), None),
                Err(reason) => (None, Some(reason)),
            };
            PatternCard {
                kind,
                tokens: week.as_ref().and_then(|found| found.figure(kind)),
                count: week.as_ref().and_then(|found| occurrences(found, kind)),
                files: (kind == PatternKind::RepeatedReads)
                    .then(|| week.as_ref().map(|found| found.reread_files.len() as u32))
                    .flatten(),
                sessions: this
                    .sessions
                    .iter()
                    .filter(|(_, found)| occurrences(found, kind).is_some_and(|n| n > 0))
                    .count() as u32,
                basis: match kind {
                    PatternKind::LongContext => PatternBasis::FromCounters,
                    _ => PatternBasis::EstimateFromResultSize,
                },
                inferred: kind == PatternKind::EditFailEdit,
                weeks: data[data.len() - span..]
                    .iter()
                    .map(|week| PatternWeek {
                        week_start: week.rollup.week_start,
                        tokens: bar(week, kind),
                    })
                    .collect(),
                change,
                change_unavailable,
            }
        })
        .collect();
    let mut reread_files: Vec<RereadRow> = week
        .as_ref()
        .map(|found| {
            found
                .reread_files
                .iter()
                .enumerate()
                .map(|(position, file)| RereadRow {
                    letter: letter_label(position),
                    ext: file.path_ext.as_ref().map(|ext| format!(".{ext}")),
                    reads: file.reads,
                    after_shrink: file.after_shrink,
                    tokens: file.figure.estimated_tokens(),
                })
                .collect()
        })
        .unwrap_or_default();
    // Most re-read first; ties keep first-seen order. The letter stays the
    // file's position.
    reread_files.sort_by_key(|row| std::cmp::Reverse(row.reads));
    reread_files.truncate(MAX_REREAD_ROWS);
    let rollup = &this.rollup;
    WeekPatterns {
        feed,
        generation: 0,
        week_start,
        week_end: week_start + Duration::days(6),
        tz: tz.local_minus_utc(),
        coverage: rollup.coverage.clone(),
        sessions: rollup.coverage.sessions(),
        claude_sessions: this.sessions.len() as u32,
        claude_only: rollup
            .sessions
            .iter()
            .any(|row| row.source != Some(AnalyticsSource::ClaudeCode)),
        long_context_threshold: LONG_CONTEXT_TOKENS,
        cards,
        reread_files,
        weeks: Vec::new(),
    }
}

/// One week's figures for goals, the lever and the weekly summary card, over
/// any feed's sessions: the rollup's per-source lines and the merged pattern
/// figures of the Claude Code sessions it counts. `threshold` is the user's
/// own context threshold; with none set nothing is counted against one.
pub fn week_figures(
    feed: Feed,
    sessions: &[SessionInput],
    week_start: NaiveDate,
    tz: FixedOffset,
    threshold: Option<u64>,
) -> WeekFigures {
    let data = week_data(feed, sessions, monday_of(week_start), &tz);
    let mut figures = WeekFigures::from_rollup(&data.rollup, &[]);
    if let Some(week) = merged(&data) {
        for kind in shown_kinds() {
            if let Some(figure) = week.figure(kind) {
                figures.patterns.insert(kind, figure);
            }
            if let Some(count) = occurrences(&week, kind) {
                figures.pattern_counts.insert(kind, count);
            }
        }
        figures.reread_files = Some(week.reread_files.len() as u32);
    }
    figures.past_threshold = threshold.map(|threshold| {
        let counted = counted_claude_turns(data.rollup.feed, sessions, data.rollup.week_start, &tz);
        let sessions = sessions
            .iter()
            .zip(&counted)
            .filter(|(input, turns)| {
                let (SessionBody::Claude { series, .. }, Some(turns)) = (&input.body, turns) else {
                    return false;
                };
                series.turns.iter().any(|turn| {
                    turns.contains(&turn.ordinal)
                        && turn.context().is_some_and(|context| context >= threshold)
                })
            })
            .count() as u32;
        ThresholdCount {
            threshold,
            sessions,
        }
    });
    figures
}

/// The sessions behind one kind in one week, over any feed's sessions.
pub fn pattern_sessions(
    feed: Feed,
    sessions: &[SessionInput],
    kind: PatternKind,
    week_start: NaiveDate,
    tz: FixedOffset,
) -> PatternSessions {
    let data = week_data(feed, sessions, week_start, &tz);
    PatternSessions {
        pattern: kind,
        feed,
        generation: 0,
        week_start: data.rollup.week_start,
        tz: tz.local_minus_utc(),
        sessions: data
            .sessions
            .iter()
            .filter_map(|(row_index, found)| {
                let count = occurrences(found, kind).filter(|n| *n > 0)?;
                let row = &data.rollup.sessions[*row_index];
                Some(PatternSession {
                    session_ref: row.session_ref.clone(),
                    count,
                    tokens: found.figure(kind),
                    state: row.state,
                    reasons: row.reasons.clone(),
                })
            })
            .collect(),
    }
}

/// Feed S: the Patterns tab over the saved snapshots.
pub fn saved_patterns(
    reports: &[LocalInsight],
    week_start: NaiveDate,
    tz: FixedOffset,
    bar_weeks: usize,
) -> WeekPatterns {
    let sessions = saved_sessions(reports);
    let mut patterns = week_patterns(Feed::Saved, &sessions, week_start, tz, bar_weeks);
    patterns.generation = saved_generation(reports);
    patterns.weeks = dated_weeks(&sessions, &tz);
    patterns
}

/// Feed S: the saved sessions behind one kind.
pub fn saved_pattern_sessions(
    reports: &[LocalInsight],
    kind: PatternKind,
    week_start: NaiveDate,
    tz: FixedOffset,
) -> PatternSessions {
    let sessions = saved_sessions(reports);
    let mut found = pattern_sessions(Feed::Saved, &sessions, kind, week_start, tz);
    found.generation = saved_generation(reports);
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::markers::tests::ctx;
    use chrono::{DateTime, TimeZone, Utc};
    use trace_commons_protocol::insights_usage_series::{
        KeyedDigest, ToolCallRecord, ToolKind, TurnRecord, UsageSeries,
    };

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// Noon UTC on a September 2026 day. 2026-09-14 is a Monday.
    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, day, 12, 0, 0).unwrap()
    }

    fn turn(ordinal: u32, day: u32, context: u32) -> TurnRecord {
        let mut turn = ctx(ordinal, context);
        turn.at = Some(at(day));
        turn
    }

    fn call(turn: u32, tool: ToolKind, key: u8, path: Option<u8>, bytes: u32) -> ToolCallRecord {
        ToolCallRecord {
            turn_ordinal: turn,
            tool,
            args_key: KeyedDigest([key; 32]),
            path_key: path.map(|p| KeyedDigest([p; 32])),
            path_ext: path.map(|_| "rs".to_string()),
            result_bytes: Some(bytes),
            success: Some(true),
            paired: true,
        }
    }

    fn read(turn: u32, path: u8, bytes: u32) -> ToolCallRecord {
        call(turn, ToolKind::Read, path, Some(path), bytes)
    }

    fn claude(
        name: &str,
        seq: u64,
        turns: Vec<TurnRecord>,
        calls: Vec<ToolCallRecord>,
    ) -> SessionInput {
        SessionInput {
            session_ref: name.to_string(),
            import_seq: seq,
            placed_at: None,
            harness_session: None,
            body: SessionBody::Claude {
                series: UsageSeries {
                    turns,
                    tool_calls: calls,
                    truncated: false,
                },
                model_labels: vec![],
            },
        }
    }

    fn codex(name: &str, seq: u64, day: u32) -> SessionInput {
        SessionInput {
            session_ref: name.to_string(),
            import_seq: seq,
            placed_at: None,
            harness_session: None,
            body: SessionBody::Codex {
                observed: Some(crate::insights::week_rollup::CodexObserved {
                    first_at: at(day),
                    last_at: at(day),
                    input: 100,
                    cached_input: 10,
                    output: 10,
                    total: 110,
                    baseline_excluded: false,
                }),
            },
        }
    }

    fn card(patterns: &WeekPatterns, kind: PatternKind) -> &PatternCard {
        patterns
            .cards
            .iter()
            .find(|card| card.kind == kind)
            .unwrap()
    }

    /// Two re-reads of file 1 (40,000 bytes each) and one of file 2 in a
    /// session on Tuesday 2026-09-15.
    /// Its turns carry message digests of their own, so sessions on one day
    /// are distinct sessions rather than overlapping imports.
    fn rereading(name: &str, seq: u64, day: u32) -> SessionInput {
        let keyed = |ordinal: u32| {
            let mut turn = turn(ordinal, day, 1_000);
            let mut key = [0xee; 32];
            key[0] = seq as u8;
            key[1] = ordinal as u8;
            turn.msg_key = Some(KeyedDigest(key));
            turn
        };
        claude(
            name,
            seq,
            vec![keyed(0), keyed(1)],
            vec![
                read(0, 1, 40_000),
                read(0, 2, 8),
                read(1, 1, 40_000),
                read(1, 2, 8),
                read(1, 1, 40_000),
            ],
        )
    }

    #[test]
    fn four_cards_in_fixed_order_with_the_token_figure_and_its_count() {
        let sessions = [rereading("a", 0, 15)];
        let week = week_patterns(Feed::Saved, &sessions, date(2026, 9, 16), utc(), 6);
        assert_eq!(week.week_start, date(2026, 9, 14));
        assert_eq!(week.week_end, date(2026, 9, 20));
        let kinds: Vec<_> = week.cards.iter().map(|card| card.kind).collect();
        assert_eq!(kinds, PatternKind::ALL);
        let reads = card(&week, PatternKind::RepeatedReads);
        // Three repeated reads of two files; 80,008 bytes / 4 rounds to 20K.
        assert_eq!(reads.count, Some(3));
        assert_eq!(reads.files, Some(2));
        assert_eq!(reads.tokens, Some(20_000));
        assert_eq!(reads.sessions, 1);
        assert_eq!(reads.basis, PatternBasis::EstimateFromResultSize);
        assert!(!reads.inferred);
        let cycles = card(&week, PatternKind::EditFailEdit);
        assert!(cycles.inferred);
        assert_eq!(cycles.count, Some(0));
        assert_eq!(cycles.tokens, Some(0));
        assert_eq!(cycles.sessions, 0);
        let long = card(&week, PatternKind::LongContext);
        assert_eq!(long.basis, PatternBasis::FromCounters);
        assert_eq!(long.files, None);
        assert_eq!(week.long_context_threshold, LONG_CONTEXT_TOKENS);
        assert_eq!(week.claude_sessions, 1);
        assert!(!week.claude_only);
    }

    #[test]
    fn with_no_claude_session_every_figure_is_unknown_never_zero() {
        let sessions = [codex("c", 0, 15)];
        let week = week_patterns(Feed::Saved, &sessions, date(2026, 9, 14), utc(), 6);
        assert_eq!(week.sessions, 1);
        assert_eq!(week.claude_sessions, 0);
        assert!(week.claude_only);
        for card in &week.cards {
            assert_eq!(card.tokens, None, "{:?}", card.kind);
            assert_eq!(card.count, None, "{:?}", card.kind);
            assert!(card.weeks.iter().all(|bar| bar.tokens.is_none()));
        }
        assert!(week.reread_files.is_empty());
    }

    #[test]
    fn codex_in_the_week_marks_the_figures_claude_only() {
        let sessions = [rereading("a", 0, 15), codex("c", 1, 16)];
        let week = week_patterns(Feed::Saved, &sessions, date(2026, 9, 14), utc(), 6);
        assert_eq!((week.claude_sessions, week.sessions), (1, 2));
        assert!(week.claude_only);
    }

    #[test]
    fn six_bars_oldest_first_with_absent_weeks_as_gaps() {
        // Sessions in the weeks of 08-31 and 09-14 only.
        let sessions = [rereading("old", 0, 1), rereading("new", 1, 15)];
        let week = week_patterns(Feed::Saved, &sessions, date(2026, 9, 14), utc(), 6);
        let bars = &card(&week, PatternKind::RepeatedReads).weeks;
        let starts: Vec<_> = bars.iter().map(|bar| bar.week_start).collect();
        assert_eq!(
            starts,
            [
                date(2026, 8, 10),
                date(2026, 8, 17),
                date(2026, 8, 24),
                date(2026, 8, 31),
                date(2026, 9, 7),
                date(2026, 9, 14),
            ]
        );
        let tokens: Vec<_> = bars.iter().map(|bar| bar.tokens).collect();
        assert_eq!(tokens, [None, None, None, Some(20_000), None, Some(20_000)]);
        // A week with a counted session and no occurrence is a drawn zero.
        let cycles = &card(&week, PatternKind::EditFailEdit).weeks;
        assert_eq!(cycles[3].tokens, Some(0));
        assert_eq!(cycles[4].tokens, None);
    }

    #[test]
    fn under_feed_s_no_week_is_compared() {
        let sessions = [rereading("old", 0, 8), rereading("new", 1, 15)];
        let week = week_patterns(Feed::Saved, &sessions, date(2026, 9, 14), utc(), 6);
        for card in &week.cards {
            assert_eq!(card.change, None);
            assert_eq!(card.change_unavailable, Some(Unavailable::NeedsCounterPass));
        }
    }

    #[test]
    fn under_feed_t_a_change_needs_both_weeks_comparable() {
        let week_of = |day: u32, from: u64| -> Vec<SessionInput> {
            (0..5)
                .map(|i| rereading(&format!("s{from}-{i}"), from + i, day))
                .collect()
        };
        let mut sessions = week_of(8, 0);
        sessions.extend(week_of(15, 10));
        let week = week_patterns(Feed::CounterPass, &sessions, date(2026, 9, 14), utc(), 6);
        let reads = card(&week, PatternKind::RepeatedReads);
        assert_eq!(reads.change, Some(0));
        assert_eq!(reads.change_unavailable, None);

        // Four sessions the week before: below the floor, so no change and a
        // gap where its bar would be.
        let mut thin = week_of(8, 0);
        thin.pop();
        thin.extend(week_of(15, 10));
        let week = week_patterns(Feed::CounterPass, &thin, date(2026, 9, 14), utc(), 6);
        let reads = card(&week, PatternKind::RepeatedReads);
        assert_eq!(reads.change, None);
        assert_eq!(
            reads.change_unavailable,
            Some(Unavailable::BelowCoverageFloor)
        );
        assert_eq!(reads.weeks[4].tokens, None);
        assert_eq!(reads.weeks[5].tokens, Some(100_000));
    }

    #[test]
    fn reread_rows_are_letters_and_extensions_most_read_first() {
        let sessions = [claude(
            "a",
            0,
            vec![turn(0, 15, 1_000), turn(1, 15, 1_000)],
            vec![
                read(0, 2, 8),
                read(0, 1, 40_000),
                read(1, 2, 8),
                read(1, 1, 40_000),
                read(1, 1, 40_000),
            ],
        )];
        let week = week_patterns(Feed::Saved, &sessions, date(2026, 9, 14), utc(), 6);
        let rows: Vec<_> = week
            .reread_files
            .iter()
            .map(|row| {
                (
                    row.letter.as_str(),
                    row.ext.as_deref(),
                    row.reads,
                    row.tokens,
                )
            })
            .collect();
        // File 2 was re-read first, so it is A; file 1 was re-read more.
        assert_eq!(
            rows,
            [
                ("B", Some(".rs"), 2, Some(20_000)),
                ("A", Some(".rs"), 1, Some(0)),
            ]
        );
        let wire = serde_json::to_value(&week).unwrap();
        let row = wire["reread_files"][0].as_object().unwrap();
        let fields: Vec<&str> = row.keys().map(String::as_str).collect();
        assert_eq!(
            fields,
            ["letter", "ext", "reads", "after_shrink", "tokens"],
            "no digest, path or name crosses"
        );
    }

    #[test]
    fn the_reread_table_is_bounded() {
        let calls: Vec<_> = (0..(MAX_REREAD_ROWS as u8 + 3))
            .flat_map(|path| [read(0, path, 8), read(1, path, 8)])
            .collect();
        let sessions = [claude(
            "a",
            0,
            vec![turn(0, 15, 1_000), turn(1, 15, 1_000)],
            calls,
        )];
        let week = week_patterns(Feed::Saved, &sessions, date(2026, 9, 14), utc(), 6);
        assert_eq!(week.reread_files.len(), MAX_REREAD_ROWS);
        assert_eq!(card(&week, PatternKind::RepeatedReads).files, Some(8));
    }

    #[test]
    fn a_session_spanning_two_weeks_counts_each_weeks_own_calls() {
        let sessions = [claude(
            "a",
            0,
            vec![turn(0, 13, 1_000), turn(1, 14, 1_000), turn(2, 14, 1_000)],
            vec![read(0, 1, 40_000), read(1, 1, 40_000), read(2, 1, 40_000)],
        )];
        // Sunday's first read is in the week before; Monday's two reads make
        // one repeat in this week, not two.
        let week = week_patterns(Feed::Saved, &sessions, date(2026, 9, 14), utc(), 6);
        assert_eq!(card(&week, PatternKind::RepeatedReads).count, Some(1));
        let before = week_patterns(Feed::Saved, &sessions, date(2026, 9, 7), utc(), 6);
        assert_eq!(card(&before, PatternKind::RepeatedReads).count, Some(0));
    }

    #[test]
    fn a_reimported_turns_calls_are_counted_once() {
        let keyed = |ordinal: u32, key: u8| {
            let mut turn = turn(ordinal, 15, 1_000);
            turn.msg_key = Some(KeyedDigest([key; 32]));
            turn
        };
        let calls = vec![read(0, 1, 40_000), read(1, 1, 40_000)];
        let first = claude("first", 0, vec![keyed(0, 1), keyed(1, 2)], calls.clone());
        // The regrown file repeats both turns and adds a third.
        let mut regrown_calls = calls;
        regrown_calls.push(read(2, 1, 40_000));
        let regrown = claude(
            "regrown",
            1,
            vec![keyed(0, 1), keyed(1, 2), keyed(2, 3)],
            regrown_calls,
        );
        let week = week_patterns(Feed::Saved, &[first, regrown], date(2026, 9, 14), utc(), 6);
        // One repeat in the first snapshot; the regrown one's only new call
        // is a read whose previous read is on a turn it does not count.
        assert_eq!(card(&week, PatternKind::RepeatedReads).count, Some(1));
        assert_eq!(card(&week, PatternKind::RepeatedReads).sessions, 1);
    }

    #[test]
    fn long_context_is_counted_input_on_the_weeks_turns() {
        let sessions = [claude(
            "a",
            0,
            vec![
                turn(0, 15, 250_000),
                turn(1, 15, 260_000),
                turn(2, 15, 270_000),
            ],
            vec![],
        )];
        let week = week_patterns(Feed::Saved, &sessions, date(2026, 9, 14), utc(), 6);
        let long = card(&week, PatternKind::LongContext);
        assert_eq!(long.count, Some(2));
        assert_eq!(long.tokens, Some(260_000 + 270_000));
        assert_eq!(long.sessions, 1);
    }

    #[test]
    fn pattern_sessions_list_the_sessions_with_an_occurrence_in_engine_order() {
        let quiet = claude("quiet", 2, vec![turn(0, 17, 1_000)], vec![read(0, 9, 8)]);
        let sessions = [rereading("b", 0, 15), rereading("a", 1, 16), quiet];
        let found = pattern_sessions(
            Feed::Saved,
            &sessions,
            PatternKind::RepeatedReads,
            date(2026, 9, 14),
            utc(),
        );
        let rows: Vec<_> = found
            .sessions
            .iter()
            .map(|row| (row.session_ref.as_str(), row.count, row.tokens))
            .collect();
        assert_eq!(rows, [("b", 3, Some(20_000)), ("a", 3, Some(20_000))]);
        assert_eq!(found.pattern, PatternKind::RepeatedReads);
        assert_eq!(found.week_start, date(2026, 9, 14));
        let week = week_patterns(Feed::Saved, &sessions, date(2026, 9, 14), utc(), 6);
        assert_eq!(
            card(&week, PatternKind::RepeatedReads).sessions as usize,
            found.sessions.len()
        );
        assert_eq!(week.claude_sessions, 3);
    }

    #[test]
    fn an_unknown_claude_session_is_not_read() {
        let mut turns = vec![turn(0, 15, 1_000), turn(1, 15, 1_000)];
        for turn in &mut turns {
            turn.output = None;
        }
        let sessions = [claude(
            "a",
            0,
            turns,
            vec![read(0, 1, 40_000), read(1, 1, 40_000)],
        )];
        let week = week_patterns(Feed::Saved, &sessions, date(2026, 9, 14), utc(), 6);
        assert_eq!(week.coverage.unknown, 1);
        assert_eq!(week.claude_sessions, 0);
        assert_eq!(card(&week, PatternKind::RepeatedReads).tokens, None);
    }

    #[test]
    fn week_figures_carry_the_figures_goals_and_the_lever_read() {
        let sessions = [rereading("a", 0, 15), codex("c", 1, 16)];
        let figures = week_figures(Feed::CounterPass, &sessions, date(2026, 9, 16), utc(), None);
        assert_eq!(figures.week_start, date(2026, 9, 14));
        assert_eq!(figures.sessions, 2);
        // Two sessions are below the floor.
        assert!(!figures.comparable);
        assert_eq!(figures.tokens[&AnalyticsSource::Codex], 110);
        assert_eq!(figures.cache_share_permille[&AnalyticsSource::Codex], 100);
        assert_eq!(figures.patterns[&PatternKind::RepeatedReads], 20_000);
        assert_eq!(figures.pattern_counts[&PatternKind::RepeatedReads], 3);
        assert_eq!(figures.reread_files, Some(2));
        assert_eq!(figures.past_threshold, None);
    }

    #[test]
    fn week_figures_with_no_claude_session_have_no_pattern_figure() {
        let figures = week_figures(
            Feed::CounterPass,
            &[codex("c", 0, 15)],
            date(2026, 9, 14),
            utc(),
            Some(1),
        );
        assert!(figures.patterns.is_empty());
        assert!(figures.pattern_counts.is_empty());
        assert_eq!(figures.reread_files, None);
        assert_eq!(
            figures.past_threshold,
            Some(ThresholdCount {
                threshold: 1,
                sessions: 0
            })
        );
    }

    #[test]
    fn sessions_past_the_threshold_are_counted_only_when_one_is_set() {
        // Every turn of these sessions sends 1,000 tokens of context.
        let sessions = [rereading("a", 0, 15), rereading("b", 1, 16)];
        let count = |threshold| {
            week_figures(
                Feed::CounterPass,
                &sessions,
                date(2026, 9, 14),
                utc(),
                threshold,
            )
            .past_threshold
        };
        assert_eq!(count(None), None);
        assert_eq!(
            count(Some(1_000)),
            Some(ThresholdCount {
                threshold: 1_000,
                sessions: 2
            })
        );
        assert_eq!(count(Some(1_001)).map(|found| found.sessions), Some(0));
    }

    #[test]
    fn the_bar_count_follows_the_request() {
        let sessions = [rereading("a", 0, 15)];
        let week = week_patterns(Feed::Saved, &sessions, date(2026, 9, 14), utc(), 1);
        assert_eq!(card(&week, PatternKind::RepeatedReads).weeks.len(), 1);
    }
}
