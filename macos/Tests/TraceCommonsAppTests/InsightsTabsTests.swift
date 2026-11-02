import XCTest

import TCBridge
@testable import TraceCommonsApp

/// Insights' tab bar (product owner, 2026-10-29): Overview, Breakdown and
/// Benchmarks, then Economizer (2026-11-01).
@MainActor
final class InsightsTabsTests: XCTestCase {
    func testTheBarShowsOverviewBreakdownBenchmarksAndEconomizer() {
        XCTAssertEqual(InsightsView.shownTabs, [.overview, .patterns, .benchmarks, .economizer])
        XCTAssertEqual(InsightsView.shownTabs.map(\.copyKey),
                       ["analytics_tab_overview", "analytics_tab_patterns", "analytics_tab_benchmarks",
                        "analytics_tab_economizer"])
    }

    func testBreakdownIsTheSecondTabsName() throws {
        let copy = try XCTUnwrap(TCInsights.copy())
        XCTAssertEqual(copy["analytics_tab_patterns"], "Breakdown")
    }
}
