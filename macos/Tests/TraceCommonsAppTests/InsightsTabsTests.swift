import XCTest

import TCBridge
@testable import TraceCommonsApp

/// Insights' tab bar (product owner, 2026-10-29): Overview, Breakdown and
/// Benchmarks. Sessions and Analyze are off the bar.
final class InsightsTabsTests: XCTestCase {
    func testTheBarShowsOverviewBreakdownAndBenchmarks() {
        XCTAssertEqual(InsightsView.shownTabs, [.overview, .patterns, .benchmarks])
        XCTAssertEqual(InsightsView.shownTabs.map(\.copyKey),
                       ["analytics_tab_overview", "analytics_tab_patterns", "analytics_tab_benchmarks"])
    }

    func testBreakdownIsTheSecondTabsName() throws {
        let copy = try XCTUnwrap(TCInsights.copy())
        XCTAssertEqual(copy["analytics_tab_patterns"], "Breakdown")
    }
}
