import XCTest

import TCBridge
@testable import TraceCommonsApp

/// Insights' tab bar (product owner, 2026-10-29): Overview, Breakdown and
/// Submitted, in place of Benchmarks.
@MainActor
final class InsightsTabsTests: XCTestCase {
    func testTheBarShowsOverviewBreakdownAndSubmitted() {
        XCTAssertEqual(InsightsView.shownTabs, [.overview, .patterns, .submitted])
        XCTAssertEqual(InsightsView.shownTabs.map(\.copyKey),
                       ["analytics_tab_overview", "analytics_tab_patterns", "analytics_tab_submitted"])
    }

    func testBreakdownIsTheSecondTabsName() throws {
        let copy = try XCTUnwrap(TCInsights.copy())
        XCTAssertEqual(copy["analytics_tab_patterns"], "Breakdown")
    }
}
