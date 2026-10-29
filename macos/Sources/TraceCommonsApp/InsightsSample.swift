import Foundation
import TCBridge
import TCShellCore

/// The sample Insights draws under the get-started card while it has no
/// trace of the person's own (product owner, 2026-10-29): the sample week
/// (`InsightsSampleWeek`) as the daemon would send it, with spend at API
/// prices, edits and benchmarks to go with it. Claude and Codex
/// models, six folders and seven days. Sample figures, never a pilot's, and
/// never mixed with the person's own: each tab draws either its real
/// models or these, never both.
enum InsightsSample {
    private static func totals(_ key: String?, sessions: Int, usd: Int64,
                               edits: Int64, undone: Int64, denied: Int64,
                               cacheSavings: Int64 = 0) -> DaemonData.InsightsWorkTotals {
        .init(key: key, sessions: sessions, usdMicrosKnown: usd, cacheSavingsUsdMicrosKnown: cacheSavings,
              editsKnown: edits, editsUndoneKnown: undone,
              editsDeniedKnown: denied)
    }

    private static func days(_ start: String, _ dollars: [Double]) -> [DaemonData.InsightsWorkDay] {
        let parse = Date.ISO8601FormatStyle().year().month().day().dateSeparator(.dash)
        let first = (try? parse.parse(start)) ?? Date(timeIntervalSince1970: 0)
        let utc = TimeZone(secondsFromGMT: 0) ?? .current
        return dollars.enumerated().map { index, amount in
            let day = first.addingTimeInterval(Double(index) * 86_400)
            return .init(date: day.formatted(Date.ISO8601FormatStyle(timeZone: utc).year().month().day()),
                         usdMicrosKnown: Int64((amount * 1_000_000).rounded()))
        }
    }

    /// The sample week's spend at API list prices and edits. Its
    /// models' tokens are the sample week's.
    static let work = DaemonData.InsightsWork(
        week: totals(nil, sessions: 37, usd: 125_500_000, edits: 233, undone: 15,
                     denied: 6, cacheSavings: 61_300_000),
        lastWeek: totals(nil, sessions: 33, usd: 112_000_000, edits: 198, undone: 17,
                         denied: 4),
        byTool: [
            totals("claude_code", sessions: 31, usd: 115_700_000, edits: 190, undone: 12,
                   denied: 5),
            totals("codex", sessions: 6, usd: 9_800_000, edits: 43, undone: 3, denied: 1),
        ],
        byModel: [
            .init(key: "claude-opus-5-5", tool: "claude_code", sessions: 14, tokens: 29_000_000,
                  usdMicrosKnown: 96_400_000),
            .init(key: "claude-sonnet-5-5", tool: "claude_code", sessions: 13, tokens: 14_000_000,
                  usdMicrosKnown: 18_200_000),
            .init(key: "gpt-5-codex", tool: "codex", sessions: 6, tokens: 6_200_000, usdMicrosKnown: 9_800_000),
            .init(key: "claude-haiku-5-5", tool: "claude_code", sessions: 4, tokens: 2_100_000,
                  usdMicrosKnown: 1_100_000),
        ],
        byFolder: [
            totals("trace-commons", sessions: 7, usd: 38_200_000, edits: 71, undone: 4,
                   denied: 2),
            totals("design-system", sessions: 6, usd: 27_400_000, edits: 52, undone: 3,
                   denied: 1),
            totals("scripts", sessions: 7, usd: 22_100_000, edits: 44, undone: 3, denied: 1),
            totals("website", sessions: 6, usd: 16_300_000, edits: 31, undone: 2, denied: 1),
            totals("infra", sessions: 6, usd: 13_900_000, edits: 22, undone: 2, denied: 1),
            totals("notes", sessions: 5, usd: 7_600_000, edits: 13, undone: 1, denied: 0),
        ],
        byDay: days(InsightsSampleWeek.weekStart, [18.40, 24.90, 27.30, 21.60, 16.20, 6.80, 10.30]),
        lastWeekByDay: days("2026-09-28", [16.10, 21.30, 24.80, 19.90, 17.40, 5.20, 7.30]))

    private static func pick(_ leader: (String, String, Int64),
                             _ next: (String, String, Int64)) -> DaemonData.InsightsBenchmarkPick {
        .init(leader: .init(label: leader.0, tool: leader.1, figure: leader.2),
              next: .init(label: next.0, tool: next.1, figure: next.2))
    }

    private static let opus = "claude-opus-5-5", sonnet = "claude-sonnet-5-5", haiku = "claude-haiku-5-5"
    private static let codex = "gpt-5-codex", claude = "claude_code", codexTool = "codex"

    private static func tip(_ kind: String, _ fill: (inout DaemonData.InsightsBenchmarkTip) -> Void)
        -> DaemonData.InsightsBenchmarkTip {
        var tip = DaemonData.InsightsBenchmarkTip(kind: kind)
        fill(&tip)
        return tip
    }

    /// Four weeks of sample benchmarks: five kinds of work, two models
    /// compared on each.
    static let benchmarks = DaemonData.InsightsBenchmarks(weeks: 4, sessions: 107, minSessions: 5, categories: [
        .init(key: "bug_fix", sessions: 34,
              quality: pick((opus, claude, 930), (codex, codexTool, 880)),
              value: pick((sonnet, claude, 410_000), (opus, claude, 1_840_000)),
              speed: pick((codex, codexTool, 840), (opus, claude, 1_260)),
              tip: tip("test_first") { $0.fasterPermille = 350; $0.sessionsWithout = 12; $0.sessions = 34 }),
        .init(key: "feature", sessions: 28,
              quality: pick((opus, claude, 910), (sonnet, claude, 860)),
              value: pick((sonnet, claude, 620_000), (opus, claude, 2_450_000)),
              speed: pick((sonnet, claude, 1_500), (opus, claude, 2_100)),
              tip: tip("cheaper_same_quality") {
                  $0.cheap = sonnet; $0.expensive = opus; $0.costRatioPermille = 250
              }),
        .init(key: "refactor", sessions: 19,
              quality: pick((codex, codexTool, 940), (opus, claude, 900)),
              value: pick((codex, codexTool, 780_000), (opus, claude, 1_960_000)),
              speed: pick((opus, claude, 1_080), (codex, codexTool, 1_320)),
              tip: tip("fewer_undos") { $0.better = codex; $0.worse = opus; $0.ratioPermille = 2_300 }),
        .init(key: "tests", sessions: 17,
              quality: pick((sonnet, claude, 950), (opus, claude, 930)),
              value: pick((sonnet, claude, 290_000), (opus, claude, 1_100_000)),
              speed: pick((sonnet, claude, 660), (opus, claude, 900)),
              tip: tip("test_first") { $0.fasterPermille = 220; $0.sessionsWithout = 5; $0.sessions = 17 }),
        .init(key: "docs", sessions: 9,
              quality: pick((sonnet, claude, 970), (haiku, claude, 950)),
              value: pick((haiku, claude, 60_000), (sonnet, claude, 210_000)),
              speed: pick((haiku, claude, 240), (sonnet, claude, 420))),
    ])

    /// The sample week as `insights_week` sends it, before `work`: the
    /// rollup of the sample's counted traces, the core's Overview and
    /// Patterns, and each tool's change on last week.
    static var weekObject: [String: Any] {
        let overview = InsightsSampleWeek.overview
        let rollup: [String: Any] = [
            "week_start": InsightsSampleWeek.weekStart,
            "coverage": ["known": 34, "partial": 2, "unknown": 1, "reasons": [String: Int]()],
            "undated_sessions": 0,
            "sources": [
                ["source": "claude_code", "sessions": 31, "tokens": 45_000_000, "largest_session_tokens": 4_400_000,
                 "cache_share": ["numerator": 38_500_000, "denominator": 44_150_000]],
                ["source": "codex", "sessions": 6, "tokens": 6_200_000, "largest_session_tokens": 1_300_000,
                 "cache_share": ["numerator": 4_588_000, "denominator": 6_200_000]],
            ],
            "codex_interval_tokens": 6_200_000,
            "by_model": [[String: Any]](),
            "sessions": InsightsSampleWeek.sessionRows,
        ]
        let usage: [[String: Any]] = (overview["by_day"] as? [[String: Any]] ?? []).enumerated().map { index, day in
            let claude = ["uncached", "cache_read", "cache_write", "output"].reduce(0) { $0 + (day[$1] as? Int ?? 0) }
            let codex = [2_400_000, 0, 1_200_000, 1_000_000, 1_600_000, 0, 0][index % 7]
            return ["date": day["date"] ?? "", "claude_code": claude, "codex": codex]
        }
        return [
            "enabled": true, "feed": "counter_pass", "readable": true, "iso_week": "2026-W41",
            "week_start": InsightsSampleWeek.weekStart, "comparable": true,
            "change_vs_last_week": [["source": "claude_code", "permille": -120], ["source": "codex", "permille": 40]],
            "rollup": rollup, "overview": overview, "patterns": InsightsSampleWeek.patterns,
            "history": [[String: Any]](), "recap_card_enabled": false, "routing_available": false,
            "usage_by_day": usage,
        ]
    }

    /// `object` decoded as the daemon's week, with `work`.
    static func week(_ object: [String: Any] = weekObject,
                     work: DaemonData.InsightsWork? = work) throws -> DaemonData.InsightsWeek {
        let data = try JSONSerialization.data(withJSONObject: object)
        var week = try DaemonDataDecoding.decoder().decode(DaemonData.InsightsWeek.self, from: data)
        week.work = work
        return week
    }

    /// A Claude login and a Codex login, each with a weekly window part
    /// used, resetting a few days from `now`.
    static func subscriptions(now: Date = Date()) -> DaemonData.SubscriptionUsage {
        typealias Window = DaemonData.SubscriptionUsageWindow
        let day: TimeInterval = 86_400
        return .init(providers: [
            .init(provider: "claude", enabled: true, logins: [
                .init(id: "sample-claude", label: "Claude", defaultLogin: true, state: "observed", updatedAt: now,
                      windows: [
                          Window(lane: "session", usedPermille: 340, resetsAt: now.addingTimeInterval(2.4 * 3_600),
                                 durationMins: 300),
                          Window(lane: "weekly", usedPermille: 410, resetsAt: now.addingTimeInterval(3.2 * day),
                                 durationMins: 10_080),
                      ]),
            ]),
            .init(provider: "codex", enabled: true, logins: [
                .init(id: "sample-codex", label: "Codex", defaultLogin: true, state: "observed", updatedAt: now,
                      windows: [
                          Window(lane: "weekly", usedPermille: 220, resetsAt: now.addingTimeInterval(4.6 * day),
                                 durationMins: 10_080),
                      ]),
            ]),
        ])
    }
}

/// The sample's models, built once and never asked anything: their service
/// refuses every request, so nothing they show is read from the store, and
/// nothing is written.
@MainActor
final class InsightsSampleModels {
    let overview: InsightsOverviewModel
    let patterns: InsightsPatternsModel
    let comparisons: InsightsComparisonsModel
    let benchmarks: InsightsBenchmarksModel
    let subscriptions: SubscriptionUsageModel
    let week: DaemonData.InsightsWeek?

    init() {
        let refuse: InsightsModel.Service = { _ in throw InsightsError.service(InsightsSampleWeek.refusal) }
        overview = InsightsOverviewModel(service: refuse)
        patterns = InsightsPatternsModel(service: refuse)
        comparisons = InsightsComparisonsModel(service: refuse)
        benchmarks = InsightsBenchmarksModel(sample: InsightsSample.benchmarks)
        let usage = InsightsSample.subscriptions()
        subscriptions = SubscriptionUsageModel(reader: { _, _, _, _ in usage })
        week = try? InsightsSample.week()
        overview.showCounter(week?.coreOverview, sessions: try? InsightsSampleWeek.counterSessions)
        patterns.showCounter(week?.corePatterns)
    }

    /// Reads the sample limits once; they never refresh.
    func load() async { await subscriptions.load() }
}
