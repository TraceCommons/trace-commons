import XCTest

@testable import TCShellCore
@testable import TraceCommonsApp

/// Where NEAR AI credit went (product owner, 2026-10-28): the Overview
/// card's top tasks, the Details table's groups, and the limits table's
/// NEAR AI row, from `insights_week`'s `near_ai_credit`. A call with no
/// charge is the dash, never `$0.00`.
final class InsightsCreditTests: XCTestCase {
    private typealias Group = DaemonData.InsightsCreditGroup
    private typealias Window = DaemonData.InsightsCreditWindow
    private let copy = [
        "analytics_unavailable": "—", "analytics_task_bug_fix": "Bug fix", "analytics_task_docs": "Docs",
        "analytics_task_other": "Other", "analytics_unknown_label": "Unknown",
        "analytics_folder_unknown": "Unknown folder", "analytics_credit_tool_codex": "Codex",
        "analytics_credit_tool_claude_code": "Claude Code",
        "analytics_credit_unknown_note": "Some calls had no charge, so their totals are unknown.",
        "analytics_limits_near_ai": "NEAR AI", "analytics_subscriptions_provider_codex": "Codex",
    ]

    private func credit(week: Window, today: Window? = nil, last31: Window? = nil) -> DaemonData.InsightsNearAiCredit {
        let empty = Window(usdMicros: 0, sessions: 0, calls: 0, callsWithoutCharge: 0)
        return .init(week: week, today: today ?? empty, last31Days: last31 ?? empty)
    }

    func testTopTasksNameEachTaskWithItsAmountAndDashForUnknown() {
        let week = Window(usdMicros: nil, sessions: 3, calls: 4, callsWithoutCharge: 1, byTask: [
            Group(key: "bug_fix", usdMicros: 1_250_000),
            Group(key: "docs", usdMicros: nil, callsWithoutCharge: 1),
            Group(key: nil, usdMicros: 500),
        ])
        let lines = InsightsCreditWords.lines(week.byTask ?? [], dimension: .task, copy: copy)
        XCTAssertEqual(lines.map(\.label), ["Bug fix", "Docs", "Unknown"])
        XCTAssertEqual(lines[0].amount, "$1.25")
        XCTAssertEqual(lines[1].amount, "—", "unknown is the dash, never $0.00")
        XCTAssertEqual(InsightsCreditWords.topTasks(credit(week: week), copy: copy, limit: 2).map(\.label),
                       ["Bug fix", "Docs"])
        XCTAssertEqual(InsightsCreditWords.unknownNote(week, copy: copy),
                       "Some calls had no charge, so their totals are unknown.")
        XCTAssertNil(InsightsCreditWords.unknownNote(
            Window(usdMicros: 10, sessions: 1, calls: 1, callsWithoutCharge: 0), copy: copy))
    }

    func testToolAndFolderGroupsUseTheirOwnWordsAndUnknownLabels() {
        let tools = InsightsCreditWords.lines([Group(key: "codex", usdMicros: 10), Group(key: "claude_code", usdMicros: 5)],
                                              dimension: .tool, copy: copy)
        XCTAssertEqual(tools.map(\.label), ["Codex", "Claude Code"])
        let folders = InsightsCreditWords.lines([Group(key: "app", usdMicros: 10), Group(key: nil, usdMicros: 5)],
                                                dimension: .folder, copy: copy)
        XCTAssertEqual(folders.map(\.label), ["app", "Unknown folder"])
        XCTAssertEqual(folders[0].traces, "1")
    }

    func testNoCallThisWeekIsEmptyNotAZeroList() {
        let week = Window(usdMicros: 0, sessions: 0, calls: 0, callsWithoutCharge: 0, byTask: [])
        XCTAssertTrue(InsightsCreditWords.isEmpty(week))
        XCTAssertTrue(InsightsCreditWords.topTasks(credit(week: week), copy: copy).isEmpty)
    }

    /// One row per account: Codex and Claude keep their windows and show
    /// the dash for spend; NEAR AI has no windows and fills the spend.
    func testTheLimitsTableHasANearAiRowWithSpendAndCreditLeft() {
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        let codex = DaemonData.SubscriptionUsageProvider(provider: "codex", enabled: true, logins: [
            .init(id: "a", label: "me", defaultLogin: true, state: "observed", windows: [
                .init(lane: "weekly", usedPermille: 250, resetsAt: now.addingTimeInterval(84 * 3_600), durationMins: 10_080),
            ]),
        ])
        let near = InsightsLimitsRows.NearAi(
            credit: credit(week: Window(usdMicros: 1, sessions: 1, calls: 1, callsWithoutCharge: 0),
                           today: Window(usdMicros: 420_000, sessions: 1, calls: 2, callsWithoutCharge: 0),
                           last31: Window(usdMicros: nil, sessions: 4, calls: 9, callsWithoutCharge: 1)),
            creditLeft: "$12.00")
        let rows = InsightsLimitsRows.rows([codex], nearAi: near, now: now, copy: copy)
        XCTAssertEqual(rows.map(\.account), ["Codex", "NEAR AI"])
        XCTAssertEqual(rows[0].spentToday, "—")
        XCTAssertEqual(rows[0].spent31Days, "—")
        XCTAssertEqual(rows[0].creditLeft, "—")
        let nearRow = rows[1]
        XCTAssertEqual(nearRow.session, "—")
        XCTAssertEqual(nearRow.weekly, "—")
        XCTAssertEqual(nearRow.reset, "—")
        XCTAssertEqual(nearRow.pace, "—")
        XCTAssertFalse(nearRow.paceWarns)
        XCTAssertEqual(nearRow.spentToday, "$0.42")
        XCTAssertEqual(nearRow.spent31Days, "—", "a call with no charge makes it unknown")
        XCTAssertEqual(nearRow.creditLeft, "$12.00")
    }

    /// Without any NEAR AI figure there is no NEAR AI row, and the table
    /// keeps its header in step with its cells.
    func testNoNearAiRowWithoutAFigure() {
        let rows = InsightsLimitsRows.rows([], nearAi: .init(credit: nil, creditLeft: nil), now: Date(), copy: copy)
        XCTAssertTrue(rows.isEmpty)
        XCTAssertEqual(InsightsLimitsRows.headerKeys.count, 8)
        XCTAssertEqual(Array(InsightsLimitsRows.headerKeys.suffix(3)), [
            "analytics_limits_spent_today", "analytics_limits_spent_31_days", "analytics_limits_credit_left",
        ])
    }

    /// The view wiring: Overview draws the card under the week's header,
    /// Details leads with the table, and the window hands both the week's
    /// credit and the balance's figure.
    func testTheViewsDrawTheCreditWhereTheOwnerAsked() throws {
        let overview = try GlassSurfaceRulesTests.text("Views/InsightsOverviewTab.swift")
        XCTAssertTrue(overview.contains("InsightsCreditCard(credit: nearAiCredit, copy: copy)"))
        XCTAssertTrue(overview.contains("InsightsLimitsTable(model: subscriptions, nearAi: limitsNearAi, copy: copy)"))
        let details = try GlassSurfaceRulesTests.text("Views/InsightsPatternsTab.swift")
        XCTAssertTrue(details.contains("if let nearAiCredit { InsightsCreditTable(credit: nearAiCredit, copy: copy) }"))
        let window = try GlassSurfaceRulesTests.text("Views/InsightsView.swift")
        XCTAssertTrue(window.contains("nearAiCredit: model.counterWeek?.nearAiCredit"))
        XCTAssertFalse(window.contains("Insights shows no money"), "owner, 2026-10-28: Insights may show money")
        let monitor = try GlassSurfaceRulesTests.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(monitor.contains(".creditLeft("))
    }
}
