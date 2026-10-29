import TCBridge
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Home's Traces hero (owner, 2026-10-28): the macOS MVP is "contribute
/// traces, earn credit", so Home leads with one large card that says so
/// and opens Traces.
@MainActor
final class HomeTracesHeroTests: XCTestCase {
    func test_theHeroSaysWhatContributingIsAndOpensTraces() throws {
        let words = try XCTUnwrap(MonitorWords.table).homeHistory
        XCTAssertEqual(words.tracesHeroTitle, "Contribute traces, earn credit, use Private AI")
        XCTAssertEqual(
            words.tracesHeroBody,
            "Share the sessions from your AI coding tools with Trace Commons. Each accepted trace earns you credit.")
        XCTAssertEqual(words.tracesHeroAction, "Start earning")
        let home = try String(
            contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent("Views/Monitor/HomeViews.swift"), encoding: .utf8)
        let hero = try XCTUnwrap(home.range(of: "private var tracesHero: some View {"))
        let body = home[hero.upperBound...].prefix(600)
        XCTAssertTrue(body.contains("GlassHeroCard("))
        XCTAssertTrue(body.contains("actionTitle: words.tracesHeroAction, action: openTraces"))
    }
}
