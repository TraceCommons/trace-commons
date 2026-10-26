import XCTest
@testable import TraceCommonsApp

/// The Breakdown card opens on its first five sessions; Show all shows
/// every one (owner, 2026-10-26).
@MainActor
final class InsightsBreakdownFoldTests: XCTestCase {
    func testFoldedShowsTheFirstFiveAndShowAllShowsEvery() {
        let rows = Array(0..<12)
        XCTAssertEqual(InsightsBreakdownFold.shown(rows, all: false), [0, 1, 2, 3, 4])
        XCTAssertEqual(InsightsBreakdownFold.shown(rows, all: true), rows)
        XCTAssertEqual(InsightsBreakdownFold.shown([0, 1], all: false), [0, 1])
    }

    func testBothViewsFoldTheirSessions() throws {
        let overview = try GlassSurfaceRulesTests.text("Views/InsightsOverviewTab.swift")
        XCTAssertTrue(overview.contains("ForEach(InsightsBreakdownFold.shown(sessions.rows, all: showAll))"))
        XCTAssertTrue(overview.contains("ForEach(InsightsBreakdownFold.shown(inputs.sessions, all: showAll))"))
    }
}
