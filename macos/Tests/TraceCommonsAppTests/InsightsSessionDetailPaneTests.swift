import XCTest
@testable import TraceCommonsApp

/// The open Sessions trace's details are the inspector's, to the right of
/// the list, as a Traces session's card is (Ron's feedback, Insights PM
/// 2026-10-26); without the window's inspector they open under the row.
@MainActor
final class InsightsSessionDetailPaneTests: XCTestCase {
    private struct Unreachable: Error {}

    func testTheInspectorMayOpenOnInsightsOnlyWhileSessionsHandsItATrace() {
        XCTAssertFalse(MonitorWindowView.inspectorAvailable(.insights, homePage: .overview))
        XCTAssertFalse(MonitorWindowView.inspectorAvailable(.insights, homePage: .overview, insightsDetail: false))
        XCTAssertTrue(MonitorWindowView.inspectorAvailable(.insights, homePage: .overview, insightsDetail: true))
        XCTAssertFalse(MonitorWindowView.inspectorAvailable(.inference, homePage: .overview, insightsDetail: true))
    }

    /// An opened trace is a new demand, so the inspector opens; leaving
    /// Sessions takes the details and the demand back.
    func testAnOpenedTraceIsTheInspectorsDemandUntilSessionsIsLeft() {
        let sessions = InsightsSessionsModel(service: { _ in throw Unreachable() })
        let inspector = InsightsInspector()
        XCTAssertNil(inspector.detail)
        XCTAssertNil(inspector.demandKey)
        inspector.show(sessions) { InsightsSessionDetail(model: sessions, copy: [:]) }
        XCTAssertNotNil(inspector.detail)
        XCTAssertNil(inspector.demandKey, "nothing open, nothing asked")
        sessions.select("t1.a")
        XCTAssertEqual(inspector.demandKey, "insights-session:t1.a")
        XCTAssertTrue(InspectorDemand.opens(previous: [], current: [inspector.demandKey!]))
        inspector.clear()
        XCTAssertNil(inspector.detail)
        XCTAssertNil(inspector.demandKey)
    }

    func testTheWindowDrawsTheDetailsAndTheTabStopsExpandingRows() throws {
        let window = try MonitorNavigationTests.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("} else if shownTab == .insights, let detail = insightsInspector.detail {\n            // Insights' Sessions: the open trace's details, beside the list.\n            detail()\n"))
        XCTAssertTrue(window.contains(".union(shownTab == .insights ? insightsInspector.demandKey.map { [$0] } ?? [] : [])"))
        XCTAssertTrue(window.contains("inspector: insightsInspector)"))
        let view = try MonitorNavigationTests.text("Views/InsightsView.swift")
        XCTAssertTrue(view.contains("detailInInspector: inspector != nil)"))
        XCTAssertTrue(view.contains(".onChange(of: tab, initial: true) { _, shown in showSessionDetail(shown) }"))
        XCTAssertTrue(view.contains("inspector?.clear()"), "closing Insights empties the inspector")
        let tab = try MonitorNavigationTests.text("Views/InsightsSessionsTab.swift")
        XCTAssertTrue(tab.contains("if !detailInInspector && model.selected == row.sessionRef { inlineDetail }"))
        XCTAssertTrue(tab.contains("if !detailInInspector { inlineDetail }"))
        XCTAssertTrue(tab.contains("if keyRow == row.sessionRef || open {"), "the open row is ringed")
        // Nothing open: the tab's own explainer, no new words.
        XCTAssertTrue(tab.contains("} else if !inline {\n                    Text(text(\"analytics_sessions_explainer_watched\"))"))
    }
}
