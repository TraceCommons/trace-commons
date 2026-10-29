import XCTest
@testable import TraceCommonsApp

/// The open Sessions trace's details are the inspector's, to the right of
/// the list, as a Traces session's card is (Ron's feedback, Insights PM
/// 2026-10-26); without the window's inspector they open under the row.
@MainActor
final class InsightsSessionDetailPaneTests: XCTestCase {
    private struct Unreachable: Error {}

    private func sessions() -> InsightsSessionsModel {
        InsightsSessionsModel(service: { _ in throw Unreachable() })
    }

    /// The window's demand keys as `MonitorWindowView.demandKeys` adds the
    /// inspector's: on every tab.
    private func keys(_ inspector: InsightsInspector) -> Set<String> {
        inspector.demandKey.map { [$0] } ?? []
    }

    func testTheInspectorMayOpenOnInsightsOnlyWhileSessionsHandsItATrace() {
        XCTAssertFalse(MonitorWindowView.inspectorAvailable(.insights, homePage: .overview))
        XCTAssertFalse(MonitorWindowView.inspectorAvailable(.insights, homePage: .overview, insightsDetail: false))
        XCTAssertTrue(MonitorWindowView.inspectorAvailable(.insights, homePage: .overview, insightsDetail: true))
        XCTAssertFalse(MonitorWindowView.inspectorAvailable(.inference, homePage: .overview, insightsDetail: true))
    }

    /// Opening a trace is a new demand each time, so the inspector opens.
    func testATraceThePersonOpensOpensTheInspector() {
        let model = sessions()
        let inspector = InsightsInspector()
        inspector.show(model) { InsightsSessionDetail(model: model, copy: [:]) }
        var before = keys(inspector)
        XCTAssertTrue(before.isEmpty, "nothing opened, nothing asked")
        inspector.personOpened()
        XCTAssertTrue(InspectorDemand.opens(previous: before, current: keys(inspector)))
        before = keys(inspector)
        inspector.personOpened()
        XCTAssertTrue(InspectorDemand.opens(previous: before, current: keys(inspector)), "a second trace asks again")
    }

    /// Leaving Sessions (or Insights) and coming back keeps the same key,
    /// so an inspector the person closed stays closed.
    func testShowingSessionsAgainDoesNotReopenTheInspector() {
        let model = sessions()
        let inspector = InsightsInspector()
        inspector.show(model) { InsightsSessionDetail(model: model, copy: [:]) }
        inspector.personOpened()
        let opened = keys(inspector)
        inspector.clear()
        XCTAssertNil(inspector.detail)
        XCTAssertEqual(keys(inspector), opened, "away, the key stays")
        inspector.show(model) { InsightsSessionDetail(model: model, copy: [:]) }
        XCTAssertFalse(InspectorDemand.opens(previous: opened, current: keys(inspector)))
    }

    /// The saved feed picks the newest snapshot itself: no demand.
    func testATracePickedForThePersonDoesNotOpenTheInspector() {
        let model = sessions()
        let inspector = InsightsInspector()
        inspector.show(model) { InsightsSessionDetail(model: model, copy: [:]) }
        model.sync(snapshotIDs: ["newest", "older"])
        XCTAssertEqual(model.selected, "newest")
        XCTAssertNil(inspector.demandKey)
        model.select("older")
        XCTAssertNil(inspector.demandKey, "only the tab's explicit opens count")
    }

    func testTheWindowDrawsTheDetailsAndTheTabStopsExpandingRows() throws {
        let window = try MonitorNavigationTests.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("} else if shownTab == .insights, let detail = insightsInspector.detail {"))
        XCTAssertTrue(window.contains(".union(insightsInspector.demandKey.map { [$0] } ?? [])"))
        XCTAssertTrue(window.contains("inspector: insightsInspector, home: home)"))
        let view = try MonitorNavigationTests.text("Views/InsightsView.swift")
        XCTAssertTrue(view.contains("detailInInspector: inspector != nil,"))
        XCTAssertTrue(view.contains("opened: { inspector?.personOpened() })"))
        XCTAssertTrue(view.contains(".onChange(of: tab, initial: true) { _, shown in showSessionDetail(shown) }"))
        XCTAssertTrue(view.contains("inspector?.clear()"), "closing Insights empties the inspector")
        let tab = try MonitorNavigationTests.text("Views/InsightsSessionsTab.swift")
        XCTAssertTrue(tab.contains("if !detailInInspector && model.selected == row.sessionRef { inlineDetail }"))
        XCTAssertTrue(tab.contains("if !detailInInspector { inlineDetail }"))
        XCTAssertTrue(tab.contains("if keyRow == row.sessionRef || open {"), "the open row is ringed")
        XCTAssertTrue(tab.contains("if !open { opened() }"), "a click that closes the row is no open")
        XCTAssertTrue(tab.contains("set: { model.select($0); opened() }"))
        // Nothing open: the tab's own explainers, no new words.
        XCTAssertTrue(tab.contains("Text(text(\"analytics_sessions_explainer_watched\"))"))
        XCTAssertTrue(tab.contains("Text(text(\"analytics_sessions_explainer_saved\"))"))
    }
}
