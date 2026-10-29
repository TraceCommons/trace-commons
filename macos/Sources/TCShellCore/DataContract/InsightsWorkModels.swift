import Foundation

/// `insights_week`'s `work` and the `insights_benchmarks` method (product
/// owner, 2026-10-29): Claude and Codex spend at API list prices, lines,
/// edits and cache savings for the week, and which model does each kind of
/// work best. Every figure is a sum as far as known with a `partial` flag:
/// a minimum while partial, never a zero for unknown.
extension DaemonData {
    /// `insights_week`'s `work`; `nil` from a daemon that predates it.
    public struct InsightsWork: Codable, Equatable, Sendable {
        public let basis: String
        public let week: InsightsWorkTotals
        public let lastWeek: InsightsWorkTotals?
        public let byTool: [InsightsWorkTotals]
        public let byModel: [InsightsWorkModel]
        public let byFolder: [InsightsWorkTotals]
        public let byDay: [InsightsWorkDay]
        public let lastWeekByDay: [InsightsWorkDay]?

        public init(basis: String = "api_list", week: InsightsWorkTotals, lastWeek: InsightsWorkTotals? = nil,
                    byTool: [InsightsWorkTotals] = [], byModel: [InsightsWorkModel] = [],
                    byFolder: [InsightsWorkTotals] = [], byDay: [InsightsWorkDay] = [],
                    lastWeekByDay: [InsightsWorkDay]? = nil) {
            self.basis = basis; self.week = week; self.lastWeek = lastWeek; self.byTool = byTool
            self.byModel = byModel; self.byFolder = byFolder; self.byDay = byDay; self.lastWeekByDay = lastWeekByDay
        }

        public enum CodingKeys: String, CodingKey {
            case basis, week
            case lastWeek = "last_week"
            case byTool = "by_tool"
            case byModel = "by_model"
            case byFolder = "by_folder"
            case byDay = "by_day"
            case lastWeekByDay = "last_week_by_day"
        }
    }

    /// One scope's totals: the week, a tool (`key` `claude_code` or
    /// `codex`) or a folder (`key` its local label, `nil` when unknown).
    public struct InsightsWorkTotals: Codable, Equatable, Sendable {
        public var key: String?
        public let sessions: Int
        public let pricedSessions: Int
        public let unpricedSessions: Int
        public let usdMicrosKnown: Int64
        public let usdPartial: Bool
        public let cacheSavingsUsdMicrosKnown: Int64
        public let linesAddedKnown: Int64
        public let linesRemovedKnown: Int64
        public let linesPartial: Bool
        public let editsKnown: Int64
        public let editsUndoneKnown: Int64
        public let editsDeniedKnown: Int64
        public let editsPartial: Bool

        public init(key: String? = nil, sessions: Int, pricedSessions: Int? = nil, unpricedSessions: Int = 0,
                    usdMicrosKnown: Int64, usdPartial: Bool = false, cacheSavingsUsdMicrosKnown: Int64 = 0,
                    linesAddedKnown: Int64 = 0, linesRemovedKnown: Int64 = 0, linesPartial: Bool = false,
                    editsKnown: Int64 = 0, editsUndoneKnown: Int64 = 0, editsDeniedKnown: Int64 = 0,
                    editsPartial: Bool = false) {
            self.key = key; self.sessions = sessions; self.pricedSessions = pricedSessions ?? sessions
            self.unpricedSessions = unpricedSessions; self.usdMicrosKnown = usdMicrosKnown
            self.usdPartial = usdPartial; self.cacheSavingsUsdMicrosKnown = cacheSavingsUsdMicrosKnown
            self.linesAddedKnown = linesAddedKnown; self.linesRemovedKnown = linesRemovedKnown
            self.linesPartial = linesPartial; self.editsKnown = editsKnown; self.editsUndoneKnown = editsUndoneKnown
            self.editsDeniedKnown = editsDeniedKnown; self.editsPartial = editsPartial
        }

        /// Edits neither undone nor denied, as far as known.
        public var editsKeptKnown: Int64 { max(0, editsKnown - editsUndoneKnown - editsDeniedKnown) }

        public enum CodingKeys: String, CodingKey {
            case key, sessions
            case pricedSessions = "priced_sessions"
            case unpricedSessions = "unpriced_sessions"
            case usdMicrosKnown = "usd_micros_known"
            case usdPartial = "usd_partial"
            case cacheSavingsUsdMicrosKnown = "cache_savings_usd_micros_known"
            case linesAddedKnown = "lines_added_known"
            case linesRemovedKnown = "lines_removed_known"
            case linesPartial = "lines_partial"
            case editsKnown = "edits_known"
            case editsUndoneKnown = "edits_undone_known"
            case editsDeniedKnown = "edits_denied_known"
            case editsPartial = "edits_partial"
        }
    }

    /// One model's week: its spend at list prices and its tokens.
    public struct InsightsWorkModel: Codable, Equatable, Sendable {
        public let key: String
        public let tool: String
        public let sessions: Int
        public let tokens: Int64
        public let usdMicrosKnown: Int64
        public let usdPartial: Bool

        public init(key: String, tool: String, sessions: Int, tokens: Int64, usdMicrosKnown: Int64,
                    usdPartial: Bool = false) {
            self.key = key; self.tool = tool; self.sessions = sessions; self.tokens = tokens
            self.usdMicrosKnown = usdMicrosKnown; self.usdPartial = usdPartial
        }

        public enum CodingKeys: String, CodingKey {
            case key, tool, sessions, tokens
            case usdMicrosKnown = "usd_micros_known"
            case usdPartial = "usd_partial"
        }
    }

    /// One local day's spend at list prices.
    public struct InsightsWorkDay: Codable, Equatable, Sendable {
        public let date: String
        public let usdMicrosKnown: Int64
        public let usdPartial: Bool

        public init(date: String, usdMicrosKnown: Int64, usdPartial: Bool = false) {
            self.date = date; self.usdMicrosKnown = usdMicrosKnown; self.usdPartial = usdPartial
        }

        public enum CodingKeys: String, CodingKey {
            case date
            case usdMicrosKnown = "usd_micros_known"
            case usdPartial = "usd_partial"
        }
    }

    /// `insights_benchmarks`: per kind of work, the person's models on
    /// quality (edits kept), value (cost per session at list prices) and
    /// speed (active time per session), from their own sessions.
    public struct InsightsBenchmarks: Codable, Equatable, Sendable {
        public let enabled: Bool
        public let readable: Bool?
        public let weeks: Int?
        public let sessions: Int?
        public let minSessions: Int?
        public let categories: [InsightsBenchmarkCategory]?

        public init(enabled: Bool = true, readable: Bool? = true, weeks: Int? = 4, sessions: Int? = nil,
                    minSessions: Int? = 5, categories: [InsightsBenchmarkCategory]?) {
            self.enabled = enabled; self.readable = readable; self.weeks = weeks; self.sessions = sessions
            self.minSessions = minSessions; self.categories = categories
        }

        public enum CodingKeys: String, CodingKey {
            case enabled, readable, weeks, sessions, categories
            case minSessions = "min_sessions"
        }
    }

    public struct InsightsBenchmarkCategory: Codable, Equatable, Sendable, Identifiable {
        public let key: String
        public let sessions: Int
        public let models: [InsightsBenchmarkModel]
        public let quality: InsightsBenchmarkPick?
        public let value: InsightsBenchmarkPick?
        public let speed: InsightsBenchmarkPick?
        public let tip: InsightsBenchmarkTip?
        public var id: String { key }

        public init(key: String, sessions: Int, models: [InsightsBenchmarkModel] = [],
                    quality: InsightsBenchmarkPick? = nil, value: InsightsBenchmarkPick? = nil,
                    speed: InsightsBenchmarkPick? = nil, tip: InsightsBenchmarkTip? = nil) {
            self.key = key; self.sessions = sessions; self.models = models
            self.quality = quality; self.value = value; self.speed = speed; self.tip = tip
        }
    }

    public struct InsightsBenchmarkModel: Codable, Equatable, Sendable {
        public let label: String
        public let tool: String
        public let sessions: Int
        public let keptPermille: Int?
        public let usdMicrosMedian: Int64?
        public let activeSecsMedian: Int64?

        public init(label: String, tool: String, sessions: Int, keptPermille: Int? = nil,
                    usdMicrosMedian: Int64? = nil, activeSecsMedian: Int64? = nil) {
            self.label = label; self.tool = tool; self.sessions = sessions; self.keptPermille = keptPermille
            self.usdMicrosMedian = usdMicrosMedian; self.activeSecsMedian = activeSecsMedian
        }

        public enum CodingKeys: String, CodingKey {
            case label, tool, sessions
            case keptPermille = "kept_permille"
            case usdMicrosMedian = "usd_micros_median"
            case activeSecsMedian = "active_secs_median"
        }
    }

    /// The leader and the runner-up. `figure` is permille for quality,
    /// micro-dollars for value and seconds for speed.
    public struct InsightsBenchmarkPick: Codable, Equatable, Sendable {
        public let leader: InsightsBenchmarkEntry
        public let next: InsightsBenchmarkEntry

        public init(leader: InsightsBenchmarkEntry, next: InsightsBenchmarkEntry) {
            self.leader = leader; self.next = next
        }
    }

    public struct InsightsBenchmarkEntry: Codable, Equatable, Sendable {
        public let label: String
        public let tool: String
        public let figure: Int64

        public init(label: String, tool: String, figure: Int64) {
            self.label = label; self.tool = tool; self.figure = figure
        }
    }

    /// One tip, from the person's own sessions. `kind` names which: the
    /// other fields are those that kind carries.
    public struct InsightsBenchmarkTip: Codable, Equatable, Sendable {
        public let kind: String
        public var fasterPermille: Int? = nil
        public var sessionsWithout: Int? = nil
        public var sessions: Int? = nil
        public var cheap: String? = nil
        public var expensive: String? = nil
        public var costRatioPermille: Int? = nil
        public var better: String? = nil
        public var worse: String? = nil
        public var ratioPermille: Int? = nil
        public var sessionsWith: Int? = nil
        public var files: Int? = nil

        public init(kind: String) { self.kind = kind }

        public enum CodingKeys: String, CodingKey {
            case kind, sessions, cheap, expensive, better, worse, files
            case fasterPermille = "faster_permille"
            case sessionsWithout = "sessions_without"
            case costRatioPermille = "cost_ratio_permille"
            case ratioPermille = "ratio_permille"
            case sessionsWith = "sessions_with"
        }
    }
}
