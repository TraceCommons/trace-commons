#if DEBUG
import Foundation
import TCBridge

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
/// Sessions drills into. Sample figures, never a pilot's. Every word is
/// still the core's copy: the answers are sampled, only the core's copy and
/// two Overview reads go to the real store, and a write is answered here
/// and kept nowhere, or refused. No write reaches the store.
enum InsightsSampleWeek {
    static var enabled: Bool {
        ProcessInfo.processInfo.environment["TRACE_COMMONS_INSIGHTS_SAMPLE"] == "1"
    }

    static let weekStart = "2026-10-05"

    /// The sample's answers; the store's for `storeReads` only. Anything
    /// else is refused here, so no write the window sends reaches the store.
    static func service(fallback: @escaping InsightsModel.Service) -> InsightsModel.Service {
        { request in
            if let answer = try response(request.operation) { return answer }
            guard storeReads.contains(request.operation.type) else { throw InsightsError.service(refusal) }
            return try await fallback(request)
        }
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

    private static func reread(_ letter: String, _ ext: String?, _ reads: Int, _ shrink: Int, _ tokens: Int) -> [String: Any] {
        ["letter": letter, "ext": ext ?? NSNull(), "reads": reads, "after_shrink": shrink, "tokens": tokens]
    }

    static var patterns: [String: Any] {
        [
            "feed": "counter_pass", "week_start": "2026-10-05", "week_end": "2026-10-11", "tz": 0,
            "coverage": coverage, "sessions": 37, "claude_sessions": 31, "claude_only": true,
            "long_context_threshold": 150_000,
            "cards": [
                card("repeated_reads", tokens: 2_300_000, count: 41, files: 6, sessions: 9,
                     basis: "estimate_from_result_size", inferred: false, change: 180,
                     weeks: [1_100_000, 1_600_000, nil, 1_900_000, 1_500_000, 2_300_000]),
                card("retried_calls", tokens: 410_000, count: 37, sessions: 7,
                     basis: "estimate_from_result_size", inferred: false, change: -60,
                     weeks: [520_000, 470_000, nil, 390_000, 440_000, 410_000]),
                card("edit_fail_edit", tokens: 180_000, count: 12, sessions: 5,
                     basis: "estimate_from_result_size", inferred: true, change: 90,
                     weeks: [120_000, 200_000, nil, 150_000, 165_000, 180_000]),
                card("long_context", tokens: 9_800_000, count: nil, sessions: 6,
                     basis: "from_counters", inferred: false, change: 310,
                     weeks: [4_100_000, 6_300_000, nil, 5_200_000, 7_500_000, 9_800_000]),
            ],
            "reread_files": [
                reread("A", "ts", 9, 2, 640_000), reread("B", "rs", 8, 0, 510_000), reread("C", "md", 7, 3, 420_000),
                reread("D", "json", 6, 0, 300_000), reread("E", nil, 5, 0, 150_000),
            ],
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
                ["id": "g2", "goal": ["kind": "repeated_reads_under", "tokens": 2_000_000], "figure": 2_300_000,
                 "marks": marks(["met", "met", "no_figure", "met", "met", "not_met"], "up", from: 1_500_000)],
            ],
            "lever": ["pick": ["kind": "repeated_reads", "tokens": 2_300_000, "count": 41, "files": 6, "ratio_permille": 1500],
                      "disabled": [String](), "card_enabled": true],
            "recap": [
                "week_start": "2026-10-05", "week_end": "2026-10-11", "sessions": 37,
                "sources": [["source": "claude_code", "tokens": 45_000_000, "change_permille": -120],
                            ["source": "codex", "tokens": 6_200_000, "change_permille": 40]],
                "items": [
                    ["kind": "best_cache_week", "source": "claude_code", "permille": 872, "previous_best_permille": 840],
                    ["kind": "goal", "id": "g1", "goal": cacheGoal, "met": true],
                    ["kind": "pattern_up", "pattern": "long_context", "up_permille": 310],
                ],
            ],
        ]
    }

    /// One trace, 52 turns, dated and sourced as the saved trace asked for
    /// (Claude Code otherwise): context grows to the threshold, the cache is
    /// written again after a pause, a file is read again, and context shrinks.
    static func session(_ snapshotID: String) -> [String: Any] {
        let saved = traces.first { $0.id == snapshotID }
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
                ["letter": "B", "turn_ordinal": 37, "kind": "re_read", "basis": "from_tool_calls",
                 "file_letter": "A", "file_ext": "ts"],
                ["letter": "C", "turn_ordinal": 46, "kind": "crossed_long_context", "basis": "from_counters",
                 "context_from": 149_000, "context": 152_000],
                ["letter": "D", "turn_ordinal": 48, "kind": "context_shrank", "basis": "inferred_from_counters",
                 "context_from": 155_000, "context": 60_000],
            ],
        ]
    }
}
#endif
