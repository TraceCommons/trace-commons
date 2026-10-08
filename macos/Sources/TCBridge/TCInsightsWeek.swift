import Foundation

// The token week over saved snapshots (feed S), as `week_overview` and
// `card_inputs` return it. Decode only: every figure is computed in the
// core, and an absent figure is unknown, never zero.

public struct InsightsWeekCoverage: Decodable, Sendable, Equatable {
    public let known, partial, unknown: UInt32
    /// Sessions per coverage reason, by wire label.
    public let reasons: [String: UInt32]
    public var sessions: UInt32 { known + partial + unknown }
}

/// A cache share with its parts, so the shell divides nothing.
public struct InsightsShareFigure: Decodable, Sendable, Equatable {
    public let numerator, denominator, permille: UInt64
}

public struct InsightsLargestSession: Decodable, Sendable, Equatable {
    /// The snapshot under feed S; absent under feed T, whose rows carry no
    /// reference that leaves the daemon.
    public let session_ref: String?
    public let tokens: UInt64
}

/// "Your best week": the previous best share, per mille, and whether this
/// week beat it. Feed T only.
public struct InsightsBestWeek: Decodable, Sendable, Equatable {
    public let previous_best_permille: UInt64
    public let is_new_best: Bool
}

/// One harness's line. Lines are never summed together.
public struct InsightsWeekSource: Decodable, Sendable, Equatable, Identifiable {
    public let source: String
    public let sessions: UInt32
    public let tokens: UInt64?
    public let cache_share: InsightsShareFigure?
    public let largest_session: InsightsLargestSession?
    /// "vs last week" per mille; feed T only, `nil` with `change` saying why.
    public let change_permille: Int64?
    /// Why "vs last week" is not a figure, e.g. `needs_counter_pass`.
    public let change: String?
    /// "Your best week"; feed T only, `nil` with `best_week` saying why.
    public let best: InsightsBestWeek?
    /// Why "Your best week" is not a figure.
    public let best_week: String?
    public var id: String { source }
}

public struct InsightsDayTokens: Decodable, Sendable, Equatable, Identifiable {
    public let date: String
    public let uncached, cache_read, cache_write, output: UInt64
    public var id: String { date }
}

public struct InsightsModelTokens: Decodable, Sendable, Equatable {
    /// `nil` is the unknown label.
    public let label: String?
    public let tokens: UInt64
}

public struct InsightsToolTokens: Decodable, Sendable, Equatable, Identifiable {
    public let source: String
    public let tokens: UInt64?
    public var id: String { source }
}

public struct InsightsWeekOverview: Decodable, Sendable, Equatable {
    public let feed: String
    public let week_start, week_end: String
    public let tz: Int32
    public let coverage: InsightsWeekCoverage
    public let undated_sessions: UInt32
    public let sessions: UInt32
    public let sources: [InsightsWeekSource]
    public let by_day: [InsightsDayTokens]?
    public let codex_interval_tokens: UInt64?
    /// In the core's fixed order: alphabetical by label, unknown last.
    public let by_model: [InsightsModelTokens]
    public let by_tool: [InsightsToolTokens]
    public let by_project: String
    /// Weeks with a dated saved session, newest first.
    public let weeks: [String]
}

public struct InsightsCardSession: Decodable, Sendable, Equatable, Identifiable {
    public let session_ref: String
    public let source: String?
    public let tokens: UInt64?
    public let cache_share: InsightsShareFigure?
    public let state: String
    public let reasons: [String]
    public var id: String { session_ref }
}

/// What makes up one Overview card's figure.
public struct InsightsCardInputs: Decodable, Sendable, Equatable {
    public let card: String
    public let feed: String
    public let week_start: String
    public let tz: Int32
    public let sources: [InsightsWeekSource]
    public let coverage: InsightsWeekCoverage
    public let sessions: [InsightsCardSession]
}

// Patterns ("Where tokens went") over saved snapshots (feed S), as
// `patterns` and `pattern_sessions` return them. Decode only.

/// One week's bar. `tokens == nil` is a gap, never a zero bar.
public struct InsightsPatternWeek: Decodable, Sendable, Equatable, Identifiable {
    public let week_start: String
    public let tokens: UInt64?
    public var id: String { week_start }
}

/// One Patterns card. Its `kind` is a wire label in the core's fixed order.
public struct InsightsPatternCard: Decodable, Sendable, Equatable, Identifiable {
    public let kind: String
    /// The headline; `nil` is unknown, never zero.
    public let tokens: UInt64?
    public let count: UInt32?
    /// Repeated reads only: how many files were read again.
    public let files: UInt32?
    public let sessions: UInt32
    /// `estimate_from_result_size` or `from_counters`.
    public let basis: String
    public let inferred: Bool
    /// Oldest first, ending at the week on screen.
    public let weeks: [InsightsPatternWeek]
    /// Per mille against last week; only ever sent under feed T.
    public let change: Int64?
    public let change_unavailable: String?
    public var id: String { kind }
}

/// A re-read file: a letter and the extension, never a path or a name.
public struct InsightsRereadRow: Decodable, Sendable, Equatable, Identifiable {
    public let letter: String
    public let ext: String?
    public let reads: UInt32
    public let after_shrink: UInt32
    public let tokens: UInt64?
    public var id: String { letter }
}

public struct InsightsWeekPatterns: Decodable, Sendable, Equatable {
    public let feed: String
    public let week_start, week_end: String
    public let tz: Int32
    public let coverage: InsightsWeekCoverage
    public let sessions: UInt32
    public let claude_sessions: UInt32
    /// Another harness is in the week: the figures cover Claude Code only.
    public let claude_only: Bool
    public let long_context_threshold: UInt64
    public let cards: [InsightsPatternCard]
    public let reread_files: [InsightsRereadRow]
    /// Weeks with a dated saved session, newest first.
    public let weeks: [String]
}

public struct InsightsPatternSession: Decodable, Sendable, Equatable, Identifiable {
    public let session_ref: String
    public let count: UInt32
    public let tokens: UInt64?
    public let state: String
    public let reasons: [String]
    public var id: String { session_ref }
}

/// The sessions behind one Patterns card, in the core's order.
public struct InsightsPatternSessions: Decodable, Sendable, Equatable {
    public let pattern: String
    public let feed: String
    public let week_start: String
    public let tz: Int32
    public let sessions: [InsightsPatternSession]
}

// Sessions (drill-in) over one saved snapshot (feed S), as `session_drill`
// returns it. Decode only.

/// One turn. Every counter is `nil` when any of the turn's counters is
/// unknown: no bar is drawn for it, never a zero bar.
public struct InsightsDrillTurn: Decodable, Sendable, Equatable, Identifiable {
    public let ordinal: UInt32
    public let uncached: UInt32?
    public let cache_read: UInt32?
    public let cache_write: UInt64?
    public let output: UInt32?
    public let context: UInt64?
    public var id: UInt32 { ordinal }
}

/// One lettered marker. Only its kind's fields are present.
public struct InsightsDrillMarker: Decodable, Sendable, Equatable, Identifiable {
    public let letter: String
    public let turn_ordinal: UInt32
    /// `cache_written_again`, `context_shrank`, `crossed_long_context` or
    /// `re_read`.
    public let kind: String
    /// `inferred_from_counters`, `from_counters` or `from_tool_calls`.
    public let basis: String
    public let pause_minutes: UInt32?
    public let cache_write: UInt64?
    public let context_from: UInt64?
    public let context: UInt64?
    /// A re-read file's position letter and its extension, never a name.
    public let file_letter: String?
    public let file_ext: String?
    public var id: String { letter }
}

public struct InsightsSessionDrill: Decodable, Sendable, Equatable {
    public let feed: String
    public let session_ref: String
    public let source: String?
    /// Local date of the first recorded event (`yyyy-MM-dd`).
    public let date: String?
    public let tz: Int32
    public let turns: UInt32?
    public let tokens: UInt64?
    /// Between the first and last recorded event; not active time.
    public let span_secs: UInt64?
    public let state: String
    public let reasons: [String]
    public let long_context_threshold: UInt64
    /// `nil` with `series_unavailable` saying why (`not_recorded` for Codex).
    public let series: [InsightsDrillTurn]?
    public let series_unavailable: String?
    public let markers: [InsightsDrillMarker]
}

// Goals, the lever of the week and the weekly summary card (feed T), as
// `comparisons` returns them, and the daemon's weekly figures it is given.

/// One of the daemon's kept weeks (`insights_week` `history`), passed to
/// `comparisons` unchanged. A source or kind absent from a map has no known
/// figure; it is never zero.
public struct InsightsWeekFigures: Codable, Sendable, Equatable {
    public let week_start: String
    public let comparable: Bool
    public let tokens: [String: UInt64]
    public let cache_share_permille: [String: UInt64]
    public let patterns: [String: UInt64]
    public let sessions: UInt32
    public let pattern_counts: [String: UInt32]
    public let reread_files: UInt32?
    public let past_threshold: InsightsThresholdCount?
}

/// Sessions that reached the user's own context threshold.
public struct InsightsThresholdCount: Codable, Sendable, Equatable {
    public let threshold: UInt64
    public let sessions: UInt32
}

/// A goal: its kind and the user's threshold. `source` names the harness for
/// the per-source kinds; Claude and Codex are never combined.
public struct InsightsGoal: Codable, Sendable, Equatable, Hashable {
    /// `cache_share_at_least`, `repeated_reads_under`, `long_context_under`
    /// or `weekly_tokens_under`.
    public let kind: String
    public let source: String?
    public let permille: UInt64?
    public let tokens: UInt64?
    public init(kind: String, source: String? = nil, permille: UInt64? = nil, tokens: UInt64? = nil) {
        self.kind = kind; self.source = source; self.permille = permille; self.tokens = tokens
    }
}

/// "Down from {x} last week." / "Up from {x} last week."
public struct InsightsGoalChange: Decodable, Sendable, Equatable {
    /// `down`, `up` or `same`.
    public let direction: String
    public let from: UInt64
}

/// Six weekly marks, oldest first, and the change from last week. No run of
/// weeks is counted.
public struct InsightsGoalMarks: Decodable, Sendable, Equatable {
    /// `met`, `not_met` or `no_figure` (never read as not met).
    public let marks: [String]
    public let change: InsightsGoalChange?
}

public struct InsightsGoalView: Decodable, Sendable, Equatable, Identifiable {
    public let id: String
    public let goal: InsightsGoal
    /// The figure in the week on screen; `nil` when unknown or not compared.
    public let figure: UInt64?
    public let marks: InsightsGoalMarks?
    public let unavailable: String?
}

/// The lever's kind and the figures its line names.
public struct InsightsLeverView: Decodable, Sendable, Equatable {
    public let kind: String
    public let tokens: UInt64
    public let count: UInt32?
    public let files: UInt32?
    public let ratio_permille: UInt64
}

public struct InsightsLeverState: Decodable, Sendable, Equatable {
    public let pick: InsightsLeverView?
    /// Kinds off after repeated "Not useful".
    public let disabled: [String]
    public let unavailable: String?
}

public struct InsightsRecapSource: Decodable, Sendable, Equatable, Identifiable {
    public let source: String
    public let tokens: UInt64
    /// Against the week before; `nil` when that week is not comparable.
    public let change_permille: Int64?
    public var id: String { source }
}

/// One item on the summary card. Only its kind's fields are present.
public struct InsightsRecapItem: Decodable, Sendable, Equatable {
    /// `best_cache_week`, `goal` or `pattern_up`.
    public let kind: String
    public let source: String?
    public let permille: UInt64?
    public let previous_best_permille: UInt64?
    public let id: String?
    public let goal: InsightsGoal?
    public let met: Bool?
    public let pattern: String?
    public let up_permille: UInt64?
}

/// The weekly summary card for the last closed week.
public struct InsightsRecap: Decodable, Sendable, Equatable {
    public let week_start, week_end: String
    public let sessions: UInt32
    public let sources: [InsightsRecapSource]
    public let items: [InsightsRecapItem]
    /// Only when the user has set a context threshold.
    public let past_threshold: InsightsThresholdCount?
}

public struct InsightsComparisons: Decodable, Sendable, Equatable {
    public let feed: String
    public let week_start: String
    public let goals: [InsightsGoalView]
    public let lever: InsightsLeverState
    public let recap: InsightsRecap?
}

/// What a goal or lever write left in the store.
public struct InsightsGoalState: Decodable, Sendable, Equatable {
    public struct Stored: Decodable, Sendable, Equatable {
        public let id: String
        public let goal: InsightsGoal
    }
    public let goals: [Stored]
    public let recap_opened_week: String?
}
