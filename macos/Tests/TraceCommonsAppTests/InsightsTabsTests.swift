import XCTest

import TCBridge
@testable import TraceCommonsApp

/// Insights' tab bar (product owner, 2026-10-29): Overview and Breakdown
/// only. Sessions and Analyze are off the bar.
final class InsightsTabsTests: XCTestCase {
    func testTheBarShowsOverviewThenBreakdownOnly() {
        XCTAssertEqual(InsightsView.shownTabs, [.overview, .patterns])
        XCTAssertEqual(InsightsView.shownTabs.map(\.copyKey), ["analytics_tab_overview", "analytics_tab_patterns"])
    }

    func testBreakdownIsTheSecondTabsName() throws {
        let copy = try XCTUnwrap(TCInsights.copy())
        XCTAssertEqual(copy["analytics_tab_patterns"], "Breakdown")
    }
}
