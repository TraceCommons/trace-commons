import XCTest

@testable import TCShellCore
@testable import TraceCommonsApp

/// AI usage this week and its drill-down (product owner, 2026-10-28): the
/// usage cards that lead Overview, and Details' By project, By task and By
/// tool views, from `insights_week`. A call with no charge is the dash,
/// never `$0.00`.
final class InsightsCreditTests: XCTestCase {
    private typealias Group = DaemonData.InsightsCreditGroup
    private typealias Window = DaemonData.InsightsCreditWindow
    private let copy = [
        "analytics_unavailable": "—", "analytics_task_bug_fix": "Bug fix", "analytics_task_docs": "Docs",
        "analytics_task_other": "Other", "analytics_unknown_label": "Unknown",
        "analytics_folder_unknown": "Unknown folder", "analytics_credit_tool_codex": "Codex",
        "analytics_credit_tool_claude_code": "Claude Code",
        "analytics_credit_unknown_note": "Some calls had no charge, so their totals are unknown.",
        "analytics_projects_traces": "{n} traces",
    ]

    private func empty() -> Window { Window(usdMicros: 0, sessions: 0, calls: 0, callsWithoutCharge: 0) }

    private func credit(week: Window, last31: Window? = nil, byDay: [Window]? = nil) -> DaemonData.InsightsNearAiCredit {
        .init(week: week, today: empty(), byDay: byDay, last31Days: last31 ?? week)
    }

    func testGroupsNameEachKeyWithItsAmountAndDashForUnknown() {
        let lines = InsightsCreditWords.lines([
            Group(key: "bug_fix", usdMicros: 1_250_000),
            Group(key: "docs", usdMicros: nil, callsWithoutCharge: 1),
            Group(key: nil, usdMicros: 500),
        ], dimension: .task, copy: copy)
        XCTAssertEqual(lines.map(\.label), ["Bug fix", "Docs", "Unknown"])
        XCTAssertEqual(lines[0].amount, "$1.25")
        XCTAssertEqual(lines[1].amount, "—", "unknown is the dash, never $0.00")
        let tools = InsightsCreditWords.lines([Group(key: "codex", usdMicros: 10), Group(key: "claude_code", usdMicros: 5)],
                                              dimension: .tool, copy: copy)
        XCTAssertEqual(tools.map(\.label), ["Codex", "Claude Code"])
        XCTAssertEqual(InsightsCreditWords.unknownNote(
            Window(usdMicros: nil, sessions: 1, calls: 2, callsWithoutCharge: 1), copy: copy),
            "Some calls had no charge, so their totals are unknown.")
        XCTAssertNil(InsightsCreditWords.unknownNote(Window(usdMicros: 10, sessions: 1, calls: 1, callsWithoutCharge: 0),
                                                     copy: copy))
    }

    /// The NEAR AI card shows only while there was NEAR AI spend in the
    /// last 31 days, and its 7 bars leave an unknown day without a bar.
    func testTheNearAiCardShowsOnlyWithSpendAndItsUnknownDaysAreDashes() {
        XCTAssertFalse(InsightsUsageWords.showsNearAi(credit(week: empty())))
        let days = (0..<7).map { index in
            Window(date: "2026-10-\(22 + index)", usdMicros: index == 3 ? nil : Int64(index) * 100_000,
                   sessions: 1, calls: 1, callsWithoutCharge: index == 3 ? 1 : 0)
        }
        let shown = credit(week: Window(usdMicros: 5, sessions: 1, calls: 1, callsWithoutCharge: 0), byDay: days)
        XCTAssertTrue(InsightsUsageWords.showsNearAi(shown))
        let bars = InsightsUsageWords.days(shown, copy: copy)
        XCTAssertEqual(bars.count, 7)
        XCTAssertEqual(bars.map(\.id).first, "2026-10-22")
        XCTAssertNil(bars[3].dollars, "an unknown day draws no bar")
        XCTAssertEqual(bars[3].amount, "—")
        XCTAssertEqual(bars[1].amount, "$0.10")
        XCTAssertTrue(InsightsUsageWords.days(credit(week: empty()), copy: copy).isEmpty, "an older daemon has no bars")
    }

    /// By project: one row per folder, Claude Code and Codex traces and
    /// tokens beside NEAR AI spend, the unknown folder last.
    func testByProjectJoinsTokensAndSpendPerFolder() throws {
        var object = try InsightsSessionRouteTests.recorded()
        var rollup = try XCTUnwrap(object["rollup"] as? [String: Any])
        var rows = try XCTUnwrap(rollup["sessions"] as? [[String: Any]])
        rows[0]["folder"] = "app"   // Claude Code, 37235 tokens
        rows[1]["folder"] = NSNull() // Codex, 110 tokens
        var extra = rows[0]
        extra["tokens"] = NSNull()  // a second Claude trace with an unknown count
        rows.append(extra)
        rollup["sessions"] = rows
        object["rollup"] = rollup
        var credit = try XCTUnwrap(object["near_ai_credit"] as? [String: Any])
        var week = try XCTUnwrap(credit["week"] as? [String: Any])
        week["by_folder"] = [
            ["key": "site", "usd_micros": 900, "sessions": 1, "calls": 1, "calls_without_charge": 0],
            ["key": NSNull(), "usd_micros": 12_500, "sessions": 1, "calls": 1, "calls_without_charge": 0],
        ]
        credit["week"] = week
        object["near_ai_credit"] = credit
        let decoded = try InsightsSessionRouteTests.decode(object)

        let table = InsightsProjectRows.rows(decoded, copy: copy)
        XCTAssertEqual(table.map(\.label), ["site", "app", "Unknown folder"])
        let app = table[1]
        XCTAssertEqual(app.claude, .init(traces: 2, tokens: nil), "one unknown count makes the sum unknown")
        XCTAssertNil(app.codex)
        XCTAssertEqual(InsightsProjectRows.tokens(app.claude, copy: copy), "—")
        XCTAssertEqual(InsightsProjectRows.traces(app.claude, copy: copy), "2 traces")
        XCTAssertEqual(InsightsProjectRows.tokens(app.codex, copy: copy), "—")
        XCTAssertEqual(app.nearAi, "—", "no NEAR AI spend there")
        let unknown = table[2]
        XCTAssertEqual(unknown.codex, .init(traces: 1, tokens: 110))
        XCTAssertEqual(unknown.nearAi, "$0.0125")
        XCTAssertEqual(table[0].nearAi, "$0.0009")
    }

    /// The wiring: Overview leads with the usage cards and has no balance,
    /// limits table or subscription card; "By project" opens Details on By
    /// project; the window no longer hands Insights the balance.
    func testOverviewLeadsWithTheUsageCardsAndDetailsWithTheDrillDown() throws {
        let overview = try GlassSurfaceRulesTests.text("Views/InsightsOverviewTab.swift")
        XCTAssertTrue(overview.contains("InsightsUsageCards(subscriptions: subscriptions, nearAiCredit: nearAiCredit, copy: copy,"))
        for gone in ["InsightsLimitsTable", "InsightsSubscriptionsCard(", "InsightsCreditCard", "var credit: AnyView?"] {
            XCTAssertFalse(overview.contains(gone), gone)
        }
        let window = try GlassSurfaceRulesTests.text("Views/InsightsView.swift")
        XCTAssertTrue(window.contains("showProjects: { detailsView = .folder; tab = .patterns }"))
        XCTAssertTrue(window.contains("week: model.counterWeek, detailsView: $detailsView"))
        XCTAssertFalse(window.contains("func credit("))
        XCTAssertFalse(window.contains("Insights shows no money"), "owner, 2026-10-28: Insights may show money")
        let details = try GlassSurfaceRulesTests.text("Views/InsightsPatternsTab.swift")
        XCTAssertTrue(details.contains("if let week { InsightsDetailsTable(week: week, copy: copy, dimension: detailsView) }"))
        let monitor = try GlassSurfaceRulesTests.text("Views/MonitorWindowView.swift")
        XCTAssertFalse(monitor.contains(".credit(AnyView(InferenceBalanceSection()))"))
        XCTAssertFalse(monitor.contains(".creditLeft("))
        let cards = try GlassSurfaceRulesTests.text("Views/InsightsUsageCards.swift")
        XCTAssertTrue(cards.contains("Button(text(\"analytics_usage_by_project\")) { showProjects() }"))
        XCTAssertEqual(cards.components(separatedBy: "InsightsUsageCard(title:").count - 1, 2,
                       "the login cards and the NEAR AI card are the one component")
    }
}
