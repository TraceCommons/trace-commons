#if DEBUG
import SwiftUI
import TCBridge
import TCDesign
import XCTest

@testable import TCShellCore
@testable import TraceCommonsApp

/// The product owner's 2026-10-29 rulings on the staging regression audit
/// (#1753) that land in the macOS app: Home's Credit earned tile shows the
/// account's whole total without the change since yesterday; Insights loses
/// Show week, the higher-than-usual card switch and its figures'
/// drill-downs; the menu-bar popover's graph greys while paused; and the
/// edge rail's unused calls graph goes.
@MainActor
final class DesignRulings20261029Tests: XCTestCase {
    private static func text(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    private func creditSummary(pointsState: String, lifetime: Int?) throws -> DaemonData.CommonsCreditSummary {
        let reply = try XCTUnwrap(SampleDaemonData.reply("commons_credit_summary", in: .normalDay))
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(reply.utf8)) as? [String: Any])
        object["points_state"] = pointsState
        object["commons_points_lifetime_earned"] = lifetime as Any? ?? NSNull()
        return try DaemonDataDecoding.decoder().decode(
            DaemonData.CommonsCreditSummary.self, from: JSONSerialization.data(withJSONObject: object))
    }

    // MARK: Home's Credit earned tile

    /// The account's whole total, in the core's words, only while the
    /// commons knows its points: unknown is no line, never a zero, and a
    /// known zero is a real one.
    func test_theCreditTileShowsTheAccountTotalOnlyWhileKnown() throws {
        let words = try XCTUnwrap(MonitorWords.table?.homeHistory)
        XCTAssertTrue(words.creditAccountTotal.contains("{amount}"))
        let known = try creditSummary(pointsState: "known", lifetime: 45)
        XCTAssertEqual(HomeFormat.creditAccountTotal(known, words: words),
                       words.creditAccountTotal.replacingOccurrences(of: "{amount}", with: HomeFormat.points(45)))
        XCTAssertNil(HomeFormat.creditAccountTotal(try creditSummary(pointsState: "unknown", lifetime: 45), words: words))
        XCTAssertNil(HomeFormat.creditAccountTotal(try creditSummary(pointsState: "known", lifetime: nil), words: words))
        XCTAssertNotNil(HomeFormat.creditAccountTotal(try creditSummary(pointsState: "known", lifetime: 0), words: words))
        XCTAssertNil(HomeFormat.creditAccountTotal(nil, words: words))
        XCTAssertNil(HomeFormat.creditAccountTotal(known, words: nil))
    }

    /// The total goes under this Mac's figure, in the sign-in line's place
    /// (the two never meet: one needs known points, the other unknown).
    /// The change since yesterday, its formatter and the Mac's own store of
    /// daily totals are gone.
    func test_theCreditTileDrawsTheTotalAndNoChangeSinceYesterday() throws {
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains(
            "let creditDetail = HomeFormat.creditSignIn(freshCredit, line: HomeFormat.creditSignInLine)\n"
                + "            ?? HomeFormat.creditAccountTotal(freshCredit, words: MonitorWords.table?.homeHistory)"))
        XCTAssertTrue(home.contains("detail: creditDetail, systemImage: HomeStatIcons.credit, action: openPrivateAI)"))
        XCTAssertFalse(home.contains("creditDelta"))
        XCTAssertFalse(home.contains("creditTotal("))
        XCTAssertFalse(try Self.text("Views/Monitor/HomeStore.swift").contains("CreditDailyTotals"))
        XCTAssertFalse(FileManager.default.fileExists(
            atPath: GlassSurfaceRulesTests.root.appendingPathComponent("Views/Monitor/CreditDailyTotals.swift").path))
    }

    // MARK: Insights

    /// No Show week beside a picked day's title, and none in the core's
    /// words. A picked day still clears on Tokens by day: clicking it
    /// again, Escape, stepping past either end, and a new week; and a
    /// cleared day withdraws the inspector's panel.
    func test_insightsHasNoShowWeekAndAPickedDayStillClears() throws {
        for file in ["Views/InsightsOverviewTab.swift", "Views/InsightsBreakdownInspector.swift"] {
            let source = try Self.text(file)
            XCTAssertFalse(source.contains("showWeek"), file)
            XCTAssertFalse(source.contains("analytics_drill_day_week"), file)
        }
        XCTAssertNil(TCInsights.copy()?["analytics_drill_day_week"])

        let chart = try Self.text("Views/InsightsTokensByDay.swift")
        XCTAssertTrue(chart.contains("pick(pickedDay == day.id ? nil : day.id)"))
        XCTAssertTrue(chart.contains(".onExitCommand { if pickedDay != nil { pick(nil) } }"))
        XCTAssertTrue(chart.contains(".onChange(of: overview.week_start) { _, _ in if pickedDay != nil { pick(nil) } }"))
        XCTAssertNil(InsightsTokensByDay.step(["mon", "tue"], from: "tue", by: 1))
        XCTAssertNil(InsightsTokensByDay.step(["mon", "tue"], from: "mon", by: -1))

        let inspector = InsightsBreakdownInspector()
        inspector.show { InsightsBreakdownDetail(inspector: inspector, overview: nil, copy: [:]) }
        inspector.pick(day: "2026-10-27")
        XCTAssertTrue(inspector.offered)
        inspector.pick(day: nil)
        XCTAssertFalse(inspector.offered, "clearing the day withdraws the panel")
    }

    /// The higher-than-usual card's switch is gone from Settings, and its
    /// label from the core's words.
    func test_theHigherThanUsualSwitchIsGone() throws {
        let card = try Self.text("Views/Settings/InsightsSettingsCard.swift")
        XCTAssertFalse(card.contains("lever"))
        XCTAssertNil(TCInsights.copy()?["analytics_setting_lever"])
        let bridge = try String(
            contentsOf: GlassSurfaceRulesTests.root.deletingLastPathComponent()
                .appendingPathComponent("TCBridge/TCInsights.swift"),
            encoding: .utf8)
        XCTAssertFalse(bridge.contains("lever_card`"), "the request keeps no field for the retired operation")
    }

    /// The tokens and cache share figures are plain: they drill nowhere,
    /// and the model asks for no drill-down a view never drew.
    func test_theFallbackFiguresArePlainText() throws {
        let overview = try Self.text("Views/InsightsOverviewTab.swift")
        XCTAssertFalse(overview.contains("drillButton("))
        XCTAssertFalse(overview.contains("model.showInputs("))
        let model = try Self.text("InsightsModel.swift")
        XCTAssertFalse(model.contains("func showInputs("))
        XCTAssertFalse(model.contains("var openCard"))
    }

    // MARK: Menu-bar popover and edge rail

    /// The popover's bars grey while watching is paused, as the menu-bar
    /// item's do; the legend keeps its colours.
    func test_thePopoverGraphGreysWhilePaused() throws {
        let panel = try Self.text("Views/Monitor/MenuBarGlassPanel.swift")
        XCTAssertTrue(panel.contains("scaleFloor: 5, compact: true, paused: model.status.paused, hovered: $hovered)"))
        XCTAssertEqual(panel.components(separatedBy: "GlassBarGraph(").count - 1, 1)
    }

    /// The edge rail's Private AI calls graph had no caller and is gone.
    func test_theEdgeRailHasNoCallsGraph() throws {
        XCTAssertFalse(try Self.text("Views/EdgeRail/EdgeRailView.swift").contains("EdgeRailCallsGraph"))
    }
}
#endif
