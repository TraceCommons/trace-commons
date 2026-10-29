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
        "analytics_credit_tool_claude_code": "Claude",
        "analytics_projects_traces": "{n} traces", "analytics_projects_traces_one": "{n} trace",
        "analytics_usage_tokens": "{t} tokens", "analytics_usage_at_least": "at least {x}",
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
        XCTAssertEqual(tools.map(\.label), ["Codex", "Claude"])
    }

    /// The NEAR AI card shows only while there was NEAR AI spend in the
    /// last 31 days, and its 7 bars leave an unknown day without a bar.
    func testTheNearAiCardShowsOnlyWithSpendAndItsUnknownDaysAreDashes() {
        XCTAssertFalse(InsightsUsageWords.showsNearAi(credit(week: empty())))
        let days: [Window] = (0..<7).map { (index: Int) -> Window in
            let usdMicros: Int64? = index == 3 ? nil : Int64(index) * 100_000
            return Window(date: "2026-10-\(22 + index)", usdMicros: usdMicros,
                          sessions: 1, calls: 1, callsWithoutCharge: index == 3 ? 1 : 0)
        }
        let shown = credit(week: Window(usdMicros: 5, sessions: 1, calls: 1, callsWithoutCharge: 0), byDay: days)
        XCTAssertTrue(InsightsUsageWords.showsNearAi(shown))
        let bars = InsightsUsageWords.days(shown, copy: copy)
        XCTAssertEqual(bars.count, 7)
        XCTAssertEqual(bars.first?.id, "2026-10-22")
        XCTAssertNil(bars[3].value, "an unknown day draws no bar")
        XCTAssertEqual(bars[3].amount, "—")
        XCTAssertEqual(bars[1].amount, "$0.10")
        XCTAssertTrue(InsightsUsageWords.days(credit(week: empty()), copy: copy).isEmpty, "an older daemon has no bars")
    }

    /// Owner, 2026-10-28: every card has the same 7 bars. Claude's and
    /// Codex's are tokens per day; an unknown day is the dash, no bar.
    func testClaudeAndCodexBarsAreTokensPerDayWithUnknownDaysAsDashes() {
        let days = [
            DaemonData.InsightsUsageDay(date: "2026-10-27", claudeCode: 1_500, codex: nil),
            DaemonData.InsightsUsageDay(date: "2026-10-28", claudeCode: 0, codex: 2_000_000),
        ]
        let claude = InsightsUsageWords.tokenBars(days, tool: "claude_code", copy: copy)
        let codex = InsightsUsageWords.tokenBars(days, tool: "codex", copy: copy)
        XCTAssertEqual(claude.map(\.value), [1_500, 0])
        XCTAssertEqual(codex.map(\.value), [nil, 2_000_000])
        XCTAssertEqual(codex[0].amount, "—")
        XCTAssertEqual(codex[1].amount, "2M")
        XCTAssertTrue(InsightsUsageWords.tokenBars(nil, tool: "codex", copy: copy).isEmpty)
    }

    /// Owner, 2026-10-28: some calls with no charge show the known sum as
    /// a minimum, "at least $5.21"; the dash only when nothing is known.
    func testAPartlyKnownFigureReadsAtLeastItsKnownSum() {
        var owner = Window(usdMicros: nil, sessions: 0, calls: 421, callsWithoutCharge: 1, tokens: nil)
        owner.usdMicrosKnown = 5_210_000
        owner.usdPartial = true
        owner.tokensKnown = 2_000_000
        owner.tokensPartial = true
        XCTAssertEqual(InsightsCreditWords.spendAndTokens(owner, copy: copy), "at least $5.21 · at least 2M tokens")
        var nothing = Window(usdMicros: nil, sessions: 0, calls: 1, callsWithoutCharge: 1, tokens: nil)
        nothing.usdPartial = false
        XCTAssertEqual(InsightsCreditWords.spend(nil, known: nil, partial: false, copy: copy), "—")
        XCTAssertEqual(InsightsCreditWords.spendAndTokens(nothing, copy: copy), "— · —")
        var group = Group(key: "docs", usdMicros: nil, callsWithoutCharge: 1)
        group.usdMicrosKnown = 50
        group.usdPartial = true
        XCTAssertEqual(InsightsCreditWords.lines([group], dimension: .task, copy: copy)[0].amount, "at least $0.00005")
        // A partly known day draws its known part.
        var day = Window(date: "2026-10-26", usdMicros: nil, sessions: 0, calls: 2, callsWithoutCharge: 1)
        day.usdMicrosKnown = 250_000
        day.usdPartial = true
        let bars = InsightsUsageWords.days(.init(week: owner, today: owner, byDay: [day], last31Days: owner), copy: copy)
        XCTAssertEqual(bars[0].value, 0.25)
        XCTAssertEqual(bars[0].amount, "at least $0.25")
    }

    /// Owner, 2026-10-28: the NEAR AI card shows spend and tokens together.
    func testNearAiFiguresPairSpendWithTokens() {
        let window = Window(usdMicros: 4_120_000, sessions: 1, calls: 3, callsWithoutCharge: 0, tokens: 1_200_000)
        XCTAssertEqual(InsightsCreditWords.spendAndTokens(window, copy: copy), "$4.12 · 1.2M tokens")
        let unknown = Window(usdMicros: nil, sessions: 1, calls: 3, callsWithoutCharge: 1, tokens: nil)
        XCTAssertEqual(InsightsCreditWords.spendAndTokens(unknown, copy: copy), "— · —")
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
            ["key": "site", "usd_micros": 900, "sessions": 1, "calls": 1, "calls_without_charge": 0, "tokens": 4_200],
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
        XCTAssertEqual(InsightsProjectRows.traces(unknown.codex, copy: copy), "1 trace", "one, not \"1 traces\"")
        XCTAssertEqual(unknown.nearAi, "$0.0125")
        XCTAssertEqual(table[0].nearAi, "$0.0009")
        XCTAssertEqual(table[0].nearAiTokens, "4.2K tokens")
        XCTAssertEqual(unknown.nearAiTokens, "—", "tokens unknown, never 0")
        XCTAssertNil(app.nearAiTokens, "no NEAR AI call there")
    }

    /// The wiring: Overview leads with the usage cards and has no balance,
    /// limits table or subscription card; "By project" opens Details on By
    /// project; the window no longer hands Insights the balance.
    func testOverviewLeadsWithTheUsageCardsAndDetailsWithTheDrillDown() throws {
        let overview = try GlassSurfaceRulesTests.text("Views/InsightsOverviewTab.swift")
        XCTAssertTrue(overview.contains("InsightsUsageCards(subscriptions: subscriptions, nearAiCredit: nearAiCredit, usageByDay: usageByDay,"))
        // Owner, 2026-10-28: no Models card, and Tokens by day is on Details.
        for gone in ["InsightsLimitsTable", "InsightsSubscriptionsCard(", "InsightsCreditCard", "var credit: AnyView?",
                     "InsightsModelsCard", "byDay(", "pickedDay"] {
            XCTAssertFalse(overview.contains(gone), gone)
        }
        let window = try GlassSurfaceRulesTests.text("Views/InsightsView.swift")
        XCTAssertTrue(window.contains("showProjects: { detailsView = .folder; tab = .patterns }"))
        XCTAssertTrue(window.contains("week: model.counterWeek, detailsView: $detailsView"))
        XCTAssertFalse(window.contains("func credit("))
        XCTAssertFalse(window.contains("Insights shows no money"), "owner, 2026-10-28: Insights may show money")
        let details = try GlassSurfaceRulesTests.text("Views/InsightsPatternsTab.swift")
        XCTAssertTrue(details.contains("if let week { InsightsDetailsTable(week: week, copy: copy, dimension: detailsView) }"))
        XCTAssertTrue(details.contains("InsightsTokensByDay(model: overview, copy: copy, pickedDay: inspector.pickedDay,"))
        XCTAssertFalse(window.contains("modelAnswers"))
        let monitor = try GlassSurfaceRulesTests.text("Views/MonitorWindowView.swift")
        XCTAssertFalse(monitor.contains(".credit(AnyView(InferenceBalanceSection()))"))
        XCTAssertFalse(monitor.contains(".creditLeft("))
        let cards = try GlassSurfaceRulesTests.text("Views/InsightsUsageCards.swift")
        XCTAssertTrue(cards.contains("Button(text(\"analytics_usage_by_project\")) { showProjects() }"))
        // Owner, 2026-10-28: no unknown, empty or folders-stay notes.
        let drill = try GlassSurfaceRulesTests.text("Views/InsightsCreditViews.swift")
        for gone in ["analytics_credit_unknown_note", "analytics_credit_empty", "analytics_credit_folders_local"] {
            XCTAssertFalse(cards.contains(gone), gone)
            XCTAssertFalse(drill.contains(gone), gone)
        }
        XCTAssertTrue(cards.contains("if !row.measured { Text(text(\"analytics_limits_refresh_to_see\")).insightsCaption() }"))
        XCTAssertEqual(cards.components(separatedBy: "InsightsUsageCard(title:").count - 1, 2,
                       "the login cards and the NEAR AI card are the one component")
        XCTAssertEqual(cards.components(separatedBy: "InsightsUsageBars(").count - 1, 1,
                       "one bar component, drawn by the card itself")
    }
}
