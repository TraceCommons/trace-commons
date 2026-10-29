#if DEBUG
import Foundation
import TCBridge
import TCShellCore

/// Debug builds only: `TRACE_COMMONS_INSIGHTS_SAMPLE=1` draws Insights from a
/// fixed sample week, so a design review can see every chart and section in
/// a dev build. A dev build is ad-hoc signed and cannot read the team
/// keychain group that holds the Insights digest keys, so its watched-folder
/// counting and saved analyses stay empty. A release build never reads the
/// variable.
///
/// The week is shaped as the core sends it: a watched-folder week (feed T)
/// with goals, the lever and the weekly summary, and the saved traces,
/// summary and group Analyze shows (`InsightsSampleAnalyze.swift`), which
/// Sessions drills into, and the week's counted sessions behind Overview's
/// Sessions breakdown, split by day. Sample figures, never a pilot's. Every
/// word is still the core's copy: the answers are sampled, only the core's
/// copy and two Overview reads go to the real store, and a write is
/// answered here and kept nowhere, or refused. No write reaches the store.
enum InsightsSampleWeek {
    static var enabled: Bool {
        ProcessInfo.processInfo.environment["TRACE_COMMONS_INSIGHTS_SAMPLE"] == "1"
    }

    static let weekStart = "2026-10-05"

    /// The sample's answers; the store's for `storeReads` only, and the
    /// core's for a chosen file analyzed but not saved, which reads that
    /// file and writes nothing. Anything else is refused here, so no write
    /// the window sends reaches the store.
    static func service(fallback: @escaping InsightsModel.Service) -> InsightsModel.Service {
        { request in
            if let answer = try response(request.operation) { return answer }
            guard reachesStore(request.operation) else { throw InsightsError.service(refusal) }
            return try await fallback(request)
        }
    }

    /// A read the sample leaves to the core: one of `storeReads`, or a
    /// chosen file analyzed without saving it.
    static func reachesStore(_ operation: InsightsRequest.Operation) -> Bool {
        storeReads.contains(operation.type) || (operation.type == "analyze" && operation.save != true)
    }

    /// The sample's answer to a request, decoded and checked as the core's
    /// are (`TCInsights.decode`); `nil` for one it does not answer. A drill
    /// answers for whichever trace was asked for.
    static func response(_ operation: InsightsRequest.Operation) throws -> InsightsResponse? {
        let type = operation.type
        let body: [String: Any]
        switch type {
        case "week_overview": body = ["type": type, "overview": overview]
        case "patterns": body = ["type": type, "patterns": patterns]
        case "comparisons": body = ["type": type, "comparisons": comparisons]
        case "session_drill": body = ["type": type, "session": session(operation.snapshot_id ?? "sample")]
        case "goal_set", "goal_delete", "lever_feedback", "recap_opened":
            body = ["type": type, "state": ["goals": [Any](), "lever_card_off": false]]
        default:
            guard let answer = try analyzeBody(operation) else { return nil }
            body = answer
        }
        return try TCInsights.decode(JSONSerialization.data(withJSONObject: body), for: operation)
    }

    static var weekOverview: InsightsWeekOverview {
        get throws { try decode(InsightsWeekOverview.self, overview) }
    }

    static var weekPatterns: InsightsWeekPatterns {
        get throws { try decode(InsightsWeekPatterns.self, patterns) }
    }

    private static func decode<T: Decodable>(_ type: T.Type, _ object: Any) throws -> T {
        try JSONDecoder().decode(T.self, from: JSONSerialization.data(withJSONObject: object))
    }

    // Build untyped JSON values per request, so no shared Any container crosses tasks.
    private static var coverage: [String: Any] {
        ["known": 34, "partial": 2, "unknown": 1, "reasons": [String: Any]()]
    }
    private static let weeks = ["2026-10-05", "2026-09-28", "2026-09-21", "2026-09-07", "2026-08-31"]

    private static func share(_ numerator: Int, _ denominator: Int, _ permille: Int) -> [String: Any] {
        ["numerator": numerator, "denominator": denominator, "permille": permille]
    }

    private static func day(_ date: String, _ uncached: Int, _ read: Int, _ write: Int, _ output: Int) -> [String: Any] {
        ["date": date, "uncached": uncached, "cache_read": read, "cache_write": write, "output": output]
    }

    static var overview: [String: Any] {
        [
            "feed": "counter_pass", "week_start": "2026-10-05", "week_end": "2026-10-11", "tz": 0,
            "coverage": coverage, "undated_sessions": 0, "sessions": 37,
            "sources": [
                ["source": "claude_code", "sessions": 31, "tokens": 45_000_000,
                 "cache_share": share(38_500_000, 44_150_000, 872), "largest_session": ["tokens": 4_400_000],
                 "change_permille": -120, "best": ["previous_best_permille": 840, "is_new_best": true]],
                ["source": "codex", "sessions": 6, "tokens": 6_200_000,
                 "cache_share": share(4_588_000, 6_200_000, 740), "largest_session": ["tokens": 1_300_000],
                 "change_permille": 40, "best_week": "not_best"],
            ],
            "by_day": [
                day("2026-10-05", 400_000, 6_100_000, 500_000, 120_000),
                day("2026-10-06", 600_000, 8_300_000, 700_000, 150_000),
                day("2026-10-07", 500_000, 9_000_000, 800_000, 180_000),
                day("2026-10-08", 400_000, 7_200_000, 600_000, 140_000),
                day("2026-10-09", 300_000, 5_400_000, 400_000, 100_000),
                day("2026-10-10", 50_000, 900_000, 100_000, 20_000),
                day("2026-10-11", 100_000, 1_600_000, 200_000, 30_000),
            ],
            "codex_interval_tokens": 6_200_000,
            "by_model": [
                ["label": "claude-haiku-5-5", "tokens": 2_100_000], ["label": "claude-opus-5-5", "tokens": 29_000_000],
                ["label": "claude-sonnet-5-5", "tokens": 14_000_000], ["label": "gpt-5-codex", "tokens": 6_200_000],
                ["label": NSNull(), "tokens": 800_000],
            ],
            "by_tool": [["source": "claude_code", "tokens": 45_000_000], ["source": "codex", "tokens": 6_200_000]],
            "by_project": "not_available_for_analyzed_files", "weeks": weeks,
        ]
    }

    /// Feed T's counted sessions for the Sessions breakdown, newest first:
    /// 30 Claude Code sessions whose days sum exactly to `by_day`, one with
    /// no known figure, and six Codex sessions, which are split by no day.
    /// Coverage as `coverage` says: 34 known, 2 partial, 1 unknown.
    static var counterSessions: InsightsCounterSessions {
        get throws {
            let rows = try decode([DaemonData.InsightsWeekSession].self, sessionRows)
            return InsightsCounterSessions(sessions: rows, column: .hidden)
        }
    }

    private static func claude(_ folder: String, _ start: String, _ end: String, _ days: [Int],
                               partial: Bool = false) -> [String: Any] {
        ["source": "claude_code", "folder": folder, "tokens": days.reduce(0, +), "by_day": days,
         "state": partial ? "partial" : "known", "reasons": partial ? ["some_turns_unknown"] : [String](),
         "started_at": start, "last_event_at": end]
    }

    private static func codex(_ folder: String, _ start: String, _ end: String, _ tokens: Int,
                              partial: Bool = false) -> [String: Any] {
        ["source": "codex", "folder": folder, "tokens": tokens,
         "state": partial ? "partial" : "known", "reasons": partial ? ["codex_baseline_excluded"] : [String](),
         "started_at": start, "last_event_at": end]
    }

    static var sessionRows: [[String: Any]] {
        let rows: [[String: Any]] = [
            claude("infra", "2026-10-11T14:09:00Z", "2026-10-11T16:57:00Z", [0, 0, 0, 0, 0, 0, 960_000]),
            claude("scripts", "2026-10-11T10:23:00Z", "2026-10-11T12:19:00Z", [0, 0, 0, 0, 0, 0, 390_000]),
            claude("design-system", "2026-10-11T09:16:00Z", "2026-10-11T10:08:00Z", [0, 0, 0, 0, 0, 0, 580_000]),
            claude("website", "2026-10-10T16:55:00Z", "2026-10-10T18:35:00Z", [0, 0, 0, 0, 0, 490_000, 0]),
            claude("notes", "2026-10-10T11:02:00Z", "2026-10-10T12:46:00Z", [0, 0, 0, 0, 0, 330_000, 0]),
            codex("website", "2026-10-09T17:40:00Z", "2026-10-09T18:25:00Z", 700_000),
            claude("trace-commons", "2026-10-09T15:48:00Z", "2026-10-09T16:24:00Z", [0, 0, 0, 0, 620_000, 0, 0]),
            claude("scripts", "2026-10-09T13:41:00Z", "2026-10-09T15:13:00Z", [0, 0, 0, 0, 1_240_000, 0, 0]),
            claude("design-system", "2026-10-09T10:34:00Z", "2026-10-09T11:02:00Z", [0, 0, 0, 0, 1_860_000, 0, 0]),
            claude("infra", "2026-10-09T09:27:00Z", "2026-10-10T01:51:00Z", [0, 0, 0, 0, 2_480_000, 250_000, 0],
                   partial: true),
            codex("trace-commons", "2026-10-09T08:20:00Z", "2026-10-09T09:07:00Z", 900_000),
            ["source": "claude_code", "folder": "notes", "tokens": NSNull(), "by_day": NSNull(),
             "state": "unknown", "reasons": ["no_usage_counters"],
             "started_at": "2026-10-08T19:05:00Z", "last_event_at": "2026-10-08T19:40:00Z"],
            claude("trace-commons", "2026-10-08T16:06:00Z", "2026-10-08T17:18:00Z", [0, 0, 0, 1_510_000, 0, 0, 0]),
            claude("scripts", "2026-10-08T15:59:00Z", "2026-10-08T17:07:00Z", [0, 0, 0, 1_810_000, 0, 0, 0]),
            claude("notes", "2026-10-08T14:20:00Z", "2026-10-08T15:40:00Z", [0, 0, 0, 750_000, 0, 0, 0]),
            claude("design-system", "2026-10-08T13:52:00Z", "2026-10-08T14:56:00Z", [0, 0, 0, 2_260_000, 0, 0, 0]),
            claude("website", "2026-10-08T11:13:00Z", "2026-10-08T13:29:00Z", [0, 0, 0, 1_210_000, 0, 0, 0]),
            codex("infra", "2026-10-08T09:30:00Z", "2026-10-08T10:45:00Z", 1_000_000, partial: true),
            claude("scripts", "2026-10-07T16:17:00Z", "2026-10-07T18:01:00Z", [0, 0, 2_200_000, 0, 0, 0, 0]),
            claude("design-system", "2026-10-07T15:10:00Z", "2026-10-08T01:50:00Z", [0, 0, 3_600_000, 800_000, 0, 0, 0]),
            claude("website", "2026-10-07T14:31:00Z", "2026-10-07T16:23:00Z", [0, 0, 1_200_000, 0, 0, 0, 0]),
            claude("trace-commons", "2026-10-07T11:24:00Z", "2026-10-07T12:12:00Z", [0, 0, 1_430_000, 0, 0, 0, 0]),
            claude("infra", "2026-10-07T10:45:00Z", "2026-10-07T12:45:00Z", [0, 0, 650_000, 0, 0, 0, 0]),
            claude("notes", "2026-10-07T09:38:00Z", "2026-10-07T10:34:00Z", [0, 0, 900_000, 0, 0, 0, 0]),
            codex("trace-commons", "2026-10-07T08:41:00Z", "2026-10-07T09:58:00Z", 1_200_000),
            claude("trace-commons", "2026-10-06T14:42:00Z", "2026-10-06T15:06:00Z", [0, 2_360_000, 0, 0, 0, 0, 0]),
            claude("infra", "2026-10-06T13:03:00Z", "2026-10-06T15:39:00Z", [0, 950_000, 0, 0, 0, 0, 0]),
            claude("scripts", "2026-10-06T11:35:00Z", "2026-10-07T01:55:00Z", [0, 3_310_000, 500_000, 0, 0, 0, 0]),
            claude("notes", "2026-10-06T10:56:00Z", "2026-10-06T11:28:00Z", [0, 1_130_000, 0, 0, 0, 0, 0]),
            claude("website", "2026-10-06T09:49:00Z", "2026-10-06T11:17:00Z", [0, 1_700_000, 0, 0, 0, 0, 0]),
            codex("scripts", "2026-10-05T18:10:00Z", "2026-10-05T19:20:00Z", 1_300_000),
            claude("design-system", "2026-10-05T16:28:00Z", "2026-10-05T17:44:00Z", [710_000, 0, 0, 0, 0, 0, 0]),
            claude("infra", "2026-10-05T15:21:00Z", "2026-10-05T17:33:00Z", [1_070_000, 0, 0, 0, 0, 0, 0]),
            claude("notes", "2026-10-05T13:14:00Z", "2026-10-05T14:22:00Z", [1_420_000, 0, 0, 0, 0, 0, 0]),
            claude("website", "2026-10-05T10:07:00Z", "2026-10-05T12:11:00Z", [1_780_000, 0, 0, 0, 0, 0, 0]),
            codex("notes", "2026-10-05T09:40:00Z", "2026-10-05T10:20:00Z", 1_100_000),
            claude("trace-commons", "2026-10-05T09:00:00Z", "2026-10-06T01:00:00Z", [2_140_000, 300_000, 0, 0, 0, 0, 0]),
        ]
        // The counter pass's short names, in its order.
        return rows.enumerated().map { index, row in row.merging(["session_ref": "t\(index + 1)"]) { $1 } }
    }

    /// Six weeks, oldest first; `nil` is a week with no figure.
    private static func patternWeeks(_ values: [Int?]) -> [[String: Any]] {
        let starts = ["2026-08-31", "2026-09-07", "2026-09-14", "2026-09-21", "2026-09-28", "2026-10-05"]
        return zip(starts, values).map { start, value in ["week_start": start, "tokens": value ?? NSNull()] }
    }

    private static func card(_ kind: String, tokens: Int, count: Int?, files: Int? = nil, sessions: Int,
                             basis: String, inferred: Bool, change: Int, weeks: [Int?]) -> [String: Any] {
        var card: [String: Any] = ["kind": kind, "tokens": tokens, "sessions": sessions, "basis": basis,
                                   "inferred": inferred, "change": change, "weeks": patternWeeks(weeks)]
        if let count { card["count"] = count }
        if let files { card["files"] = files }
        return card
    }

    static var patterns: [String: Any] {
        [
            "feed": "counter_pass", "week_start": "2026-10-05", "week_end": "2026-10-11", "tz": 0,
            "coverage": coverage, "sessions": 37, "claude_sessions": 31, "claude_only": true,
            "long_context_threshold": 150_000,
            "cards": [
                card("retried_calls", tokens: 410_000, count: 37, sessions: 7,
                     basis: "estimate_from_result_size", inferred: false, change: -60,
                     weeks: [520_000, 470_000, nil, 390_000, 440_000, 410_000]),
            ],
            "reread_files": [Any](),
            "weeks": weeks,
        ]
    }

    private static var cacheGoal: [String: Any] {
        ["kind": "cache_share_at_least", "source": "claude_code", "permille": 850]
    }

    private static func marks(_ marks: [String], _ direction: String, from: Int) -> [String: Any] {
        ["marks": marks, "change": ["direction": direction, "from": from]]
    }

    static var comparisons: [String: Any] {
        [
            "feed": "counter_pass", "week_start": "2026-10-05",
            "goals": [
                ["id": "g1", "goal": cacheGoal, "figure": 872,
                 "marks": marks(["not_met", "not_met", "no_figure", "met", "not_met", "met"], "up", from: 830)],
            ],
            "lever": ["pick": ["kind": "retried_calls", "tokens": 410_000, "count": 37, "ratio_permille": 1500],
                      "disabled": [String](), "card_enabled": true],
            "recap": [
                "week_start": "2026-10-05", "week_end": "2026-10-11", "sessions": 37,
                "sources": [["source": "claude_code", "tokens": 45_000_000, "change_permille": -120],
                            ["source": "codex", "tokens": 6_200_000, "change_permille": 40]],
                "items": [
                    ["kind": "best_cache_week", "source": "claude_code", "permille": 872, "previous_best_permille": 840],
                    ["kind": "goal", "id": "g1", "goal": cacheGoal, "met": true],
                    ["kind": "pattern_up", "pattern": "retried_calls", "up_permille": 310],
                ],
            ],
        ]
    }

    /// One trace, 52 turns, dated and sourced as the saved trace asked for
    /// (Claude Code otherwise): context grows to the threshold, the cache is
    /// written again after a pause, and context shrinks.
    /// A saved Codex trace is drawn as the core draws one: no per-turn
    /// series ("not recorded"), no turns and no markers.
    static func session(_ snapshotID: String) -> [String: Any] {
        let saved = traces.first { $0.id == snapshotID }
        if saved?.source == "codex", let saved {
            return [
                "feed": "saved", "session_ref": snapshotID, "source": "codex",
                "date": String(saved.firstEvent.prefix(10)), "tz": 0,
                "tokens": 1_200_000, "span_secs": 2_700, "state": "known", "reasons": [String](),
                "long_context_threshold": 150_000, "series_unavailable": "not_recorded",
                "markers": [Any](),
            ]
        }
        let turns: [[String: Any]] = (1 ... 52).map { turn in
            let context = turn < 48 ? 14_000 + 3_000 * turn : 60_000 + 3_000 * (turn - 48)
            let write = turn == 31 ? 96_000 : 3_000
            let uncached = turn == 31 ? 2_000 : 1_500
            return ["ordinal": turn, "uncached": uncached, "cache_read": context - write - uncached,
                    "cache_write": write, "output": 900, "context": context]
        }
        return [
            "feed": "saved", "session_ref": snapshotID, "source": saved?.source ?? "claude_code",
            "date": saved.map { String($0.firstEvent.prefix(10)) } ?? "2026-10-09", "tz": 0,
            "turns": 52, "tokens": 4_400_000, "span_secs": 6480, "state": "known", "reasons": [String](),
            "long_context_threshold": 150_000, "series": turns,
            "markers": [
                ["letter": "A", "turn_ordinal": 31, "kind": "cache_written_again", "basis": "inferred_from_counters",
                 "pause_minutes": 7, "cache_write": 96_000],
                ["letter": "B", "turn_ordinal": 48, "kind": "context_shrank", "basis": "inferred_from_counters",
                 "context_from": 155_000, "context": 60_000],
            ],
        ]
    }
}
#endif
