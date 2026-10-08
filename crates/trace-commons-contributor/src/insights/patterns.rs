//! The four Patterns figures for one session, from tool-call order and usage
//! counters. Token figures for tool patterns are estimates from result size.
//! Pure: no I/O and no copy. The figures overlap and are never summed.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use trace_commons_protocol::insights_usage_series::{
    KeyedDigest, ToolCallRecord, ToolKind, UsageSeries,
};

use super::analytics_constants::{
    EDIT_FAIL_EDIT_CARD, EDIT_FAIL_EDIT_WINDOW_CALLS, LONG_CONTEXT_TOKENS, RESULT_BYTES_PER_TOKEN,
    RETRY_WINDOW_CALLS, TOKEN_ESTIMATE_ROUNDING,
};
use super::markers::{RereadEvent, shrink_turns, usage_markers};

/// A count of pattern occurrences and the result bytes behind them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatternFigure {
    pub count: u32,
    /// Sum of the stated result sizes of the counted calls.
    pub result_bytes: u64,
    /// Counted calls whose result size is unknown; their bytes are not in
    /// `result_bytes`, and they are not read as zero.
    pub calls_without_size: u32,
}

impl PatternFigure {
    /// About how many tokens, from result size. `None` when every counted
    /// call's size is unknown.
    pub fn estimated_tokens(&self) -> Option<u64> {
        if self.count > 0 && self.calls_without_size >= self.count {
            return None;
        }
        Some(estimate_tokens_from_bytes(self.result_bytes))
    }

    pub fn merge(&mut self, other: &PatternFigure) {
        self.count += other.count;
        self.result_bytes += other.result_bytes;
        self.calls_without_size += other.calls_without_size;
    }

    fn add_call(&mut self, call: &ToolCallRecord) {
        self.count += 1;
        self.add_bytes(call);
    }

    fn add_bytes(&mut self, call: &ToolCallRecord) {
        match call.result_bytes {
            Some(bytes) => self.result_bytes += u64::from(bytes),
            None => self.calls_without_size += 1,
        }
    }
}

/// Result bytes over [`RESULT_BYTES_PER_TOKEN`], rounded half up to a
/// multiple of [`TOKEN_ESTIMATE_ROUNDING`].
pub fn estimate_tokens_from_bytes(bytes: u64) -> u64 {
    let tokens = bytes / RESULT_BYTES_PER_TOKEN;
    (tokens + TOKEN_ESTIMATE_ROUNDING / 2) / TOKEN_ESTIMATE_ROUNDING * TOKEN_ESTIMATE_ROUNDING
}

/// One file read again, named only by its keyed digest and extension. Its
/// label is its position, lettered by `markers::letter_label`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RereadFile {
    pub path_key: KeyedDigest,
    pub path_ext: Option<String>,
    /// Repeated reads of this file (the first read is not counted).
    pub reads: u32,
    /// Of those, the reads with a context shrink since the previous read.
    pub after_shrink: u32,
    pub figure: PatternFigure,
}

/// Input sent on turns after context reached the long-context threshold.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LongContextFigure {
    pub turns: u32,
    pub tokens: u64,
    /// Turn pairs where either side's usage is unknown.
    pub unknown_turns: u32,
}

impl LongContextFigure {
    /// `None` when no pair was known and some were unknown.
    pub fn tokens(&self) -> Option<u64> {
        (self.turns > 0 || self.unknown_turns == 0).then_some(self.tokens)
    }

    pub fn merge(&mut self, other: &LongContextFigure) {
        self.turns += other.turns;
        self.tokens += other.tokens;
        self.unknown_turns += other.unknown_turns;
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPatterns {
    pub repeated_reads: PatternFigure,
    pub repeated_reads_after_shrink: u32,
    /// Files in order of their first repeated read.
    pub reread_files: Vec<RereadFile>,
    #[serde(skip)]
    pub reread_events: Vec<RereadEvent>,
    pub retried_calls: PatternFigure,
    /// `None` while the card is held (owner decision D9).
    pub edit_fail_edit: Option<PatternFigure>,
    pub long_context: LongContextFigure,
}

/// The four pattern kinds, in their fixed display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternKind {
    RepeatedReads,
    RetriedCalls,
    /// Inferred from the order of tool calls (owner decision D9).
    EditFailEdit,
    LongContext,
}

impl PatternKind {
    pub const ALL: [PatternKind; 4] = [
        Self::RepeatedReads,
        Self::RetriedCalls,
        Self::EditFailEdit,
        Self::LongContext,
    ];
}

impl SessionPatterns {
    /// A kind's token figure: a rounded result-size estimate for the tool
    /// patterns, counted input for long context. `None` when unknown or held.
    pub fn figure(&self, kind: PatternKind) -> Option<u64> {
        match kind {
            PatternKind::RepeatedReads => self.repeated_reads.estimated_tokens(),
            PatternKind::RetriedCalls => self.retried_calls.estimated_tokens(),
            PatternKind::EditFailEdit => self
                .edit_fail_edit
                .as_ref()
                .and_then(PatternFigure::estimated_tokens),
            PatternKind::LongContext => self.long_context.tokens(),
        }
    }
}

/// The four Patterns figures for one session.
pub fn session_patterns(series: &UsageSeries) -> SessionPatterns {
    let shrinks = shrink_turns(&usage_markers(&series.turns));
    let mut found = SessionPatterns::default();
    repeated_reads(series, &shrinks, &mut found);
    found.retried_calls = retried_calls(&series.tool_calls);
    found.edit_fail_edit = EDIT_FAIL_EDIT_CARD.then(|| edit_fail_edit(&series.tool_calls));
    found.long_context = long_context(series);
    found
}

/// The edit tools that reset a file for repeated reads. NotebookEdit is one;
/// the edit, failed command, edit cycle uses [`is_cycle_edit`] instead, which
/// follows that card's own definition.
fn is_reread_edit(tool: ToolKind) -> bool {
    matches!(
        tool,
        ToolKind::Edit | ToolKind::Write | ToolKind::MultiEdit | ToolKind::NotebookEdit
    )
}

fn is_cycle_edit(tool: ToolKind) -> bool {
    matches!(tool, ToolKind::Edit | ToolKind::Write | ToolKind::MultiEdit)
}

fn is_lookup(tool: ToolKind) -> bool {
    matches!(tool, ToolKind::Read | ToolKind::Grep | ToolKind::Glob)
}

fn repeated_reads(series: &UsageSeries, shrinks: &[u32], found: &mut SessionPatterns) {
    // Per file: the turn of its last read, cleared by an edit tool call.
    let mut last_read: BTreeMap<KeyedDigest, Option<u32>> = BTreeMap::new();
    let mut file_index: BTreeMap<KeyedDigest, usize> = BTreeMap::new();
    for call in &series.tool_calls {
        let Some(path_key) = call.path_key else {
            continue;
        };
        if is_reread_edit(call.tool) {
            last_read.insert(path_key, None);
            continue;
        }
        if call.tool != ToolKind::Read {
            continue;
        }
        if let Some(Some(previous_turn)) = last_read.get(&path_key).copied() {
            let after_shrink = shrinks
                .iter()
                .any(|turn| previous_turn < *turn && *turn <= call.turn_ordinal);
            let index = *file_index.entry(path_key).or_insert_with(|| {
                found.reread_files.push(RereadFile {
                    path_key,
                    path_ext: call.path_ext.clone(),
                    reads: 0,
                    after_shrink: 0,
                    figure: PatternFigure::default(),
                });
                found.reread_files.len() - 1
            });
            let file = &mut found.reread_files[index];
            file.reads += 1;
            file.figure.add_call(call);
            found.repeated_reads.add_call(call);
            if after_shrink {
                file.after_shrink += 1;
                found.repeated_reads_after_shrink += 1;
            }
            found.reread_events.push(RereadEvent {
                turn_ordinal: call.turn_ordinal,
                file_index: index,
            });
        }
        last_read.insert(path_key, Some(call.turn_ordinal));
    }
}

fn retried_calls(calls: &[ToolCallRecord]) -> PatternFigure {
    let mut figure = PatternFigure::default();
    for (index, call) in calls.iter().enumerate() {
        if !call.paired {
            continue;
        }
        let earliest = index.saturating_sub(RETRY_WINDOW_CALLS);
        for earlier in calls[earliest..index].iter().rev() {
            if earlier.paired && earlier.tool == call.tool && earlier.args_key == call.args_key {
                figure.add_call(call);
                break;
            }
            if !is_lookup(earlier.tool) {
                break;
            }
        }
    }
    figure
}

fn edit_fail_edit(calls: &[ToolCallRecord]) -> PatternFigure {
    let mut figure = PatternFigure::default();
    for (start, first) in calls.iter().enumerate() {
        let Some(file) = first.path_key.filter(|_| is_cycle_edit(first.tool)) else {
            continue;
        };
        let last = (start + EDIT_FAIL_EDIT_WINDOW_CALLS - 1).min(calls.len().saturating_sub(1));
        let mut failed = false;
        for end in start + 1..=last {
            let call = &calls[end];
            let edits_file = is_cycle_edit(call.tool) && call.path_key == Some(file);
            if call.tool == ToolKind::Bash && call.success == Some(false) {
                failed = true;
            } else if edits_file {
                if failed {
                    figure.count += 1;
                    for cycle_call in &calls[start..=end] {
                        figure.add_bytes(cycle_call);
                    }
                }
                // Either the cycle closed, or a later edit of the file starts
                // its own: the first edit is the last one before the failure.
                break;
            }
        }
    }
    figure
}

fn long_context(series: &UsageSeries) -> LongContextFigure {
    let mut figure = LongContextFigure::default();
    for pair in series.turns.windows(2) {
        match (pair[0].context(), pair[1].context()) {
            (Some(previous), Some(current)) => {
                if previous >= LONG_CONTEXT_TOKENS {
                    figure.turns += 1;
                    figure.tokens += current;
                }
            }
            _ => figure.unknown_turns += 1,
        }
    }
    figure
}

/// A week's re-read files: merged by keyed digest in first-seen order across
/// the sessions given, so a label is a position, not a ranking.
pub fn merge_reread_files<'a>(
    sessions: impl IntoIterator<Item = &'a SessionPatterns>,
) -> Vec<RereadFile> {
    let mut merged: Vec<RereadFile> = Vec::new();
    let mut index: BTreeMap<KeyedDigest, usize> = BTreeMap::new();
    for session in sessions {
        for file in &session.reread_files {
            match index.get(&file.path_key) {
                Some(&at) => {
                    let into = &mut merged[at];
                    into.reads += file.reads;
                    into.after_shrink += file.after_shrink;
                    into.figure.merge(&file.figure);
                }
                None => {
                    index.insert(file.path_key, merged.len());
                    merged.push(file.clone());
                }
            }
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::markers::tests::ctx;
    use trace_commons_protocol::insights_usage_series::{
        KeyedDigest, ToolCallRecord, ToolKind, UsageSeries,
    };

    fn call(tool: ToolKind, args: u8, path: Option<u8>, bytes: Option<u32>) -> ToolCallRecord {
        ToolCallRecord {
            turn_ordinal: 0,
            tool,
            args_key: KeyedDigest([args; 32]),
            path_key: path.map(|p| KeyedDigest([p; 32])),
            path_ext: path.map(|_| "rs".to_string()),
            result_bytes: bytes,
            success: Some(true),
            paired: true,
        }
    }

    fn read(path: u8, bytes: u32) -> ToolCallRecord {
        call(ToolKind::Read, path, Some(path), Some(bytes))
    }

    fn edit(path: u8) -> ToolCallRecord {
        call(ToolKind::Edit, 100 + path, Some(path), Some(40))
    }

    fn bash(args: u8, ok: Option<bool>) -> ToolCallRecord {
        let mut call = call(ToolKind::Bash, args, None, Some(400));
        call.success = ok;
        call
    }

    fn series(calls: Vec<ToolCallRecord>) -> UsageSeries {
        UsageSeries {
            turns: vec![ctx(0, 1_000)],
            tool_calls: calls,
            truncated: false,
        }
    }

    fn on_turn(mut call: ToolCallRecord, turn: u32) -> ToolCallRecord {
        call.turn_ordinal = turn;
        call
    }

    #[test]
    fn a_second_read_with_no_edit_between_is_repeated() {
        let found = session_patterns(&series(vec![read(1, 800), read(1, 1_200)]));
        assert_eq!(found.repeated_reads.count, 1);
        assert_eq!(found.repeated_reads.result_bytes, 1_200);
        assert_eq!(found.reread_events.len(), 1);
    }

    #[test]
    fn every_edit_tool_resets_a_file() {
        for tool in [
            ToolKind::Edit,
            ToolKind::Write,
            ToolKind::MultiEdit,
            ToolKind::NotebookEdit,
        ] {
            let found = session_patterns(&series(vec![
                read(1, 8),
                call(tool, 50, Some(1), Some(8)),
                read(1, 8),
            ]));
            assert_eq!(found.repeated_reads.count, 0, "{tool:?}");
        }
    }

    #[test]
    fn an_edit_to_another_file_or_a_bash_command_does_not_reset() {
        let found = session_patterns(&series(vec![
            read(1, 8),
            edit(2),
            bash(9, Some(true)),
            read(1, 8),
        ]));
        assert_eq!(found.repeated_reads.count, 1);
    }

    #[test]
    fn a_read_with_no_path_is_not_a_file_read() {
        let found = session_patterns(&series(vec![
            call(ToolKind::Read, 1, None, Some(8)),
            call(ToolKind::Read, 1, None, Some(8)),
        ]));
        assert_eq!(found.repeated_reads.count, 0);
    }

    #[test]
    fn repeated_reads_split_on_a_shrink_between_them() {
        let calls = vec![on_turn(read(1, 8), 0), on_turn(read(1, 8), 2)];
        let shrinking = UsageSeries {
            turns: vec![ctx(0, 100_000), ctx(1, 10_000), ctx(2, 12_000)],
            tool_calls: calls.clone(),
            truncated: false,
        };
        let found = session_patterns(&shrinking);
        assert_eq!(found.repeated_reads.count, 1);
        assert_eq!(found.repeated_reads_after_shrink, 1);
        assert_eq!(found.reread_files[0].after_shrink, 1);

        let flat = UsageSeries {
            turns: vec![ctx(0, 100_000), ctx(1, 100_000), ctx(2, 100_000)],
            tool_calls: calls,
            truncated: false,
        };
        assert_eq!(session_patterns(&flat).repeated_reads_after_shrink, 0);
    }

    #[test]
    fn a_shrink_on_the_first_reads_turn_is_not_between() {
        let shrinking = UsageSeries {
            turns: vec![ctx(0, 100_000), ctx(1, 10_000), ctx(2, 12_000)],
            tool_calls: vec![on_turn(read(1, 8), 1), on_turn(read(1, 8), 2)],
            truncated: false,
        };
        assert_eq!(session_patterns(&shrinking).repeated_reads_after_shrink, 0);
    }

    #[test]
    fn reread_files_are_lettered_by_first_reread_and_keep_only_the_extension() {
        let found = session_patterns(&series(vec![
            read(7, 4),
            read(3, 4),
            read(3, 4),
            read(7, 4),
            read(7, 4),
        ]));
        let files: Vec<_> = found
            .reread_files
            .iter()
            .map(|f| (f.path_key, f.reads, f.path_ext.as_deref()))
            .collect();
        assert_eq!(
            files,
            vec![
                (KeyedDigest([3; 32]), 1, Some("rs")),
                (KeyedDigest([7; 32]), 2, Some("rs")),
            ]
        );
        assert_eq!(found.reread_events[0].file_index, 0);
        assert_eq!(found.reread_events[2].file_index, 1);
    }

    #[test]
    fn a_retry_repeats_tool_and_arguments_with_only_reads_between() {
        let found = session_patterns(&series(vec![
            bash(1, Some(false)),
            read(5, 4),
            call(ToolKind::Grep, 6, None, Some(4)),
            call(ToolKind::Glob, 7, None, Some(4)),
            bash(1, Some(false)),
        ]));
        assert_eq!(found.retried_calls.count, 1);
        assert_eq!(found.retried_calls.result_bytes, 400);
    }

    #[test]
    fn any_other_call_between_makes_it_a_rerun() {
        for between in [
            edit(1),
            bash(9, Some(true)),
            call(ToolKind::Other, 9, None, Some(4)),
        ] {
            let found = session_patterns(&series(vec![
                bash(1, Some(true)),
                between.clone(),
                bash(1, Some(true)),
            ]));
            assert_eq!(found.retried_calls.count, 0, "{between:?}");
        }
    }

    #[test]
    fn the_retry_window_is_five_calls() {
        let mut within = vec![bash(1, Some(true))];
        within.extend((0..RETRY_WINDOW_CALLS as u8 - 1).map(|i| read(20 + i, 4)));
        within.push(bash(1, Some(true)));
        assert_eq!(session_patterns(&series(within)).retried_calls.count, 1);

        let mut beyond = vec![bash(1, Some(true))];
        beyond.extend((0..RETRY_WINDOW_CALLS as u8).map(|i| read(20 + i, 4)));
        beyond.push(bash(1, Some(true)));
        assert_eq!(session_patterns(&series(beyond)).retried_calls.count, 0);
    }

    #[test]
    fn an_unpaired_call_is_not_a_retry() {
        let mut second = bash(1, None);
        second.paired = false;
        let found = session_patterns(&series(vec![bash(1, Some(true)), second]));
        assert_eq!(found.retried_calls.count, 0);
    }

    #[test]
    fn edit_failed_command_edit_counts_cycles() {
        let found = session_patterns(&series(vec![edit(1), bash(2, Some(false)), edit(1)]));
        let cycle = found.edit_fail_edit.unwrap();
        assert_eq!(cycle.count, 1);
        assert_eq!(cycle.result_bytes, 40 + 400 + 40);
    }

    #[test]
    fn the_cycle_window_is_six_calls() {
        let mut within = vec![edit(1), bash(2, Some(false))];
        within.extend((0..EDIT_FAIL_EDIT_WINDOW_CALLS as u8 - 3).map(|i| read(20 + i, 4)));
        within.push(edit(1));
        assert_eq!(
            session_patterns(&series(within))
                .edit_fail_edit
                .unwrap()
                .count,
            1
        );
        let mut beyond = vec![edit(1), bash(2, Some(false))];
        beyond.extend((0..EDIT_FAIL_EDIT_WINDOW_CALLS as u8 - 2).map(|i| read(20 + i, 4)));
        beyond.push(edit(1));
        assert_eq!(
            session_patterns(&series(beyond))
                .edit_fail_edit
                .unwrap()
                .count,
            0
        );
    }

    #[test]
    fn the_cycle_needs_a_stated_failure_and_the_same_file() {
        for middle in [bash(2, Some(true)), bash(2, None)] {
            let found = session_patterns(&series(vec![edit(1), middle, edit(1)]));
            assert_eq!(found.edit_fail_edit.unwrap().count, 0);
        }
        let other_file = session_patterns(&series(vec![edit(1), bash(2, Some(false)), edit(2)]));
        assert_eq!(other_file.edit_fail_edit.unwrap().count, 0);
    }

    #[test]
    fn notebook_edit_does_not_open_or_close_a_cycle() {
        let notebook = call(ToolKind::NotebookEdit, 9, Some(1), Some(4));
        let found = session_patterns(&series(vec![
            notebook.clone(),
            bash(2, Some(false)),
            notebook,
        ]));
        assert_eq!(found.edit_fail_edit.unwrap().count, 0);
    }

    #[test]
    fn cycles_chain_but_a_double_edit_counts_once() {
        let chained = session_patterns(&series(vec![
            edit(1),
            bash(2, Some(false)),
            edit(1),
            bash(2, Some(false)),
            edit(1),
        ]));
        assert_eq!(chained.edit_fail_edit.unwrap().count, 2);
        let double = session_patterns(&series(vec![
            edit(1),
            edit(1),
            bash(2, Some(false)),
            edit(1),
        ]));
        assert_eq!(double.edit_fail_edit.unwrap().count, 1);
    }

    #[test]
    fn long_context_sums_turns_after_the_threshold() {
        let limit = LONG_CONTEXT_TOKENS as u32;
        let found = session_patterns(&UsageSeries {
            turns: vec![
                ctx(0, limit - 1),
                ctx(1, limit),
                ctx(2, 210_000),
                ctx(3, 220_000),
            ],
            tool_calls: vec![],
            truncated: false,
        });
        assert_eq!(found.long_context.turns, 2);
        assert_eq!(found.long_context.tokens, 210_000 + 220_000);
        assert_eq!(found.long_context.unknown_turns, 0);
    }

    #[test]
    fn long_context_counts_an_unknown_pair_as_unknown_not_zero() {
        let mut turns = vec![ctx(0, 300_000), ctx(1, 300_000)];
        turns[0].output = None;
        let found = session_patterns(&UsageSeries {
            turns,
            tool_calls: vec![],
            truncated: false,
        });
        assert_eq!(found.long_context.turns, 0);
        assert_eq!(found.long_context.unknown_turns, 1);
        assert_eq!(found.long_context.tokens(), None);
    }

    #[test]
    fn a_figure_with_no_sized_call_has_no_token_estimate() {
        let found = session_patterns(&series(vec![
            call(ToolKind::Read, 1, Some(1), None),
            call(ToolKind::Read, 1, Some(1), None),
        ]));
        assert_eq!(found.repeated_reads.count, 1);
        assert_eq!(found.repeated_reads.calls_without_size, 1);
        assert_eq!(found.repeated_reads.estimated_tokens(), None);
    }

    #[test]
    fn an_empty_figure_is_a_known_zero() {
        let found = session_patterns(&series(vec![]));
        assert_eq!(found.repeated_reads.estimated_tokens(), Some(0));
    }

    #[test]
    fn estimates_are_bytes_over_four_rounded_to_ten_thousand() {
        assert_eq!(estimate_tokens_from_bytes(20_000), 10_000);
        assert_eq!(estimate_tokens_from_bytes(19_996), 0);
        assert_eq!(estimate_tokens_from_bytes(60_000), 20_000);
        assert_eq!(estimate_tokens_from_bytes(4 * 14_999), 10_000);
        assert_eq!(estimate_tokens_from_bytes(4 * 15_000), 20_000);
    }

    #[test]
    fn figures_merge_across_sessions() {
        let mut total = PatternFigure::default();
        total.merge(&PatternFigure {
            count: 2,
            result_bytes: 10,
            calls_without_size: 1,
        });
        total.merge(&PatternFigure {
            count: 1,
            result_bytes: 5,
            calls_without_size: 0,
        });
        assert_eq!(
            total,
            PatternFigure {
                count: 3,
                result_bytes: 15,
                calls_without_size: 1
            }
        );
    }

    #[test]
    fn week_files_merge_by_key_in_first_seen_order() {
        let one = session_patterns(&series(vec![
            read(4, 8),
            read(4, 8),
            read(2, 8),
            read(2, 8),
        ]));
        let two = session_patterns(&series(vec![read(2, 8), read(2, 8), read(2, 8)]));
        let merged = merge_reread_files([&one, &two]);
        let summary: Vec<_> = merged.iter().map(|f| (f.path_key.0[0], f.reads)).collect();
        assert_eq!(summary, vec![(4, 1), (2, 3)]);
    }

    #[test]
    fn pattern_kinds_have_a_fixed_order() {
        assert_eq!(
            PatternKind::ALL,
            [
                PatternKind::RepeatedReads,
                PatternKind::RetriedCalls,
                PatternKind::EditFailEdit,
                PatternKind::LongContext,
            ]
        );
    }

    #[test]
    fn figure_reads_each_kind_and_keeps_unknown_unknown() {
        let found = session_patterns(&series(vec![read(1, 40_000), read(1, 40_000)]));
        assert_eq!(found.figure(PatternKind::RepeatedReads), Some(10_000));
        assert_eq!(found.figure(PatternKind::RetriedCalls), Some(10_000));
        assert_eq!(found.figure(PatternKind::EditFailEdit), Some(0));
        assert_eq!(found.figure(PatternKind::LongContext), Some(0));
        let unsized_reads = session_patterns(&series(vec![
            call(ToolKind::Read, 1, Some(1), None),
            call(ToolKind::Read, 1, Some(1), None),
        ]));
        assert_eq!(unsized_reads.figure(PatternKind::RepeatedReads), None);
    }
}
