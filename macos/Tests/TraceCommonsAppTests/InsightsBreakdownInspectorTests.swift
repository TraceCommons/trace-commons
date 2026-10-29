import XCTest
@testable import TraceCommonsApp

/// Breakdown's details are the Monitor window's inspector's (owner,
/// 2026-10-29): a picked day's sessions and a card's sessions show there,
/// and Breakdown no longer draws them under the chart or the card.
@MainActor
final class InsightsBreakdownInspectorTests: XCTestCase {
    private struct Unreachable: Error {}

    private func patterns() -> InsightsPatternsModel {
        InsightsPatternsModel(service: { _ in throw Unreachable() })
    }

    private func detail(_ inspector: InsightsBreakdownInspector, _ patterns: InsightsPatternsModel) -> InsightsBreakdownDetail {
        InsightsBreakdownDetail(inspector: inspector, patterns: patterns, overview: nil, copy: [:])
    }

    /// The window's demand keys as `MonitorWindowView.demandKeys` adds the
    /// inspector's: on every tab.
    private func keys(_ inspector: InsightsBreakdownInspector) -> Set<String> {
        inspector.demandKey.map { [$0] } ?? []
    }

    func testTheInspectorMayOpenOnInsightsOnlyWhileBreakdownHandsItDetails() {
        XCTAssertFalse(MonitorWindowView.inspectorAvailable(.insights, homePage: .overview))
        XCTAssertFalse(MonitorWindowView.inspectorAvailable(.insights, homePage: .overview, insightsDetail: false))
        XCTAssertTrue(MonitorWindowView.inspectorAvailable(.insights, homePage: .overview, insightsDetail: true))
        XCTAssertFalse(MonitorWindowView.inspectorAvailable(.inference, homePage: .overview, insightsDetail: true))
    }

    /// Nothing picked is the empty state; a card's sessions win over a day;
    /// a day shows only when it is in the week on screen and has sessions.
    func testWhatTheInspectorShows() {
        let day = InsightsDayPick(index: 1, weekday: "Tuesday")
        XCTAssertEqual(InsightsBreakdownDetail.shown(patternSessions: false, day: nil, daySessions: true), .empty)
        XCTAssertEqual(InsightsBreakdownDetail.shown(patternSessions: false, day: day, daySessions: true), .day)
        XCTAssertEqual(InsightsBreakdownDetail.shown(patternSessions: false, day: day, daySessions: false), .empty,
                       "a week with no rows draws no empty card")
        XCTAssertEqual(InsightsBreakdownDetail.shown(patternSessions: true, day: day, daySessions: true), .pattern)
        XCTAssertEqual(InsightsBreakdownDetail.shown(patternSessions: true, day: nil, daySessions: false), .pattern)
    }

    /// Picking a day is a demand each time, so the inspector opens; putting
    /// the week back asks nothing.
    func testPickingADayOpensTheInspectorAndTheWeekDoesNot() {
        let inspector = InsightsBreakdownInspector()
        let model = patterns()
        inspector.show { self.detail(inspector, model) }
        XCTAssertNotNil(inspector.detail)
        var before = keys(inspector)
        XCTAssertTrue(before.isEmpty, "nothing picked, nothing asked")
        inspector.pick(day: "2026-10-20", patterns: model)
        XCTAssertEqual(inspector.pickedDay, "2026-10-20")
        XCTAssertTrue(InspectorDemand.opens(previous: before, current: keys(inspector)))
        before = keys(inspector)
        inspector.pick(day: "2026-10-21", patterns: model)
        XCTAssertTrue(InspectorDemand.opens(previous: before, current: keys(inspector)), "a second day asks again")
        before = keys(inspector)
        inspector.pick(day: nil, patterns: model)
        XCTAssertNil(inspector.pickedDay)
        XCTAssertFalse(InspectorDemand.opens(previous: before, current: keys(inspector)), "the week is no demand")
    }

    /// A card's sessions replace a picked day, and asking for them is a
    /// demand.
    func testACardsSessionsReplaceAPickedDay() {
        let inspector = InsightsBreakdownInspector()
        let model = patterns()
        inspector.pick(day: "2026-10-20", patterns: model)
        let before = keys(inspector)
        inspector.toggleSessions("repeated_reads", patterns: model)
        XCTAssertNil(inspector.pickedDay)
        XCTAssertTrue(InspectorDemand.opens(previous: before, current: keys(inspector)))
    }

    /// Leaving Breakdown (or Insights) and coming back keeps the same key,
    /// so an inspector the person closed stays closed.
    func testShowingBreakdownAgainDoesNotReopenTheInspector() {
        let inspector = InsightsBreakdownInspector()
        let model = patterns()
        inspector.show { self.detail(inspector, model) }
        inspector.pick(day: "2026-10-20", patterns: model)
        let picked = keys(inspector)
        inspector.clear()
        XCTAssertNil(inspector.detail)
        XCTAssertEqual(keys(inspector), picked, "away, the key stays")
        inspector.show { self.detail(inspector, model) }
        XCTAssertFalse(InspectorDemand.opens(previous: picked, current: keys(inspector)))
    }

    /// Breakdown hands its details to the inspector and draws none inline;
    /// the window draws them while Insights is shown, and the empty state is
    /// existing copy.
    func testBreakdownsDetailsAreTheInspectorsNotInline() throws {
        let tab = try InsightsGlassConventionsTests.text("Views/InsightsPatternsTab.swift")
        XCTAssertFalse(tab.contains("InsightsPatternSessionsView(found: sessions"), "a card's sessions are inline again")
        XCTAssertTrue(tab.contains("inspector.toggleSessions(card.kind, patterns: model)"))
        let byDay = try InsightsGlassConventionsTests.text("Views/InsightsTokensByDay.swift")
        XCTAssertFalse(byDay.contains("InsightsCounterSessionsView("), "a day's sessions are inline again")
        XCTAssertFalse(byDay.contains("InsightsCardInputsView("), "a day's sessions are inline again")
        let insights = try InsightsGlassConventionsTests.text("Views/InsightsView.swift")
        XCTAssertTrue(insights.contains(".onChange(of: tab, initial: true) { _, shown in showBreakdownDetail(shown) }"))
        XCTAssertTrue(insights.contains("guard shown == .patterns else { inspector.clear(); return }"))
        let window = try InsightsGlassConventionsTests.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("} else if shownTab == .insights, let detail = insightsInspector.detail {"))
        XCTAssertTrue(window.contains(".union(insightsInspector.demandKey.map { [$0] } ?? [])"))
        let detail = try InsightsGlassConventionsTests.text("Views/InsightsBreakdownInspector.swift")
        XCTAssertTrue(detail.contains("InsightsBreakdownTitle(day: nil, copy: copy, showWeek: {})\n"
            + "                    Text(text(\"analytics_unavailable\")).insightsNote()"))
    }
}
