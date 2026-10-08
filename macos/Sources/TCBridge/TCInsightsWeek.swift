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
    public let session_ref: String
    public let tokens: UInt64
}

/// One harness's line. Lines are never summed together.
public struct InsightsWeekSource: Decodable, Sendable, Equatable, Identifiable {
    public let source: String
    public let sessions: UInt32
    public let tokens: UInt64?
    public let cache_share: InsightsShareFigure?
    public let largest_session: InsightsLargestSession?
    /// Why "vs last week" is not a figure, e.g. `needs_counter_pass`.
    public let change: String
    /// Why "Your best week" is not a figure.
    public let best_week: String
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
