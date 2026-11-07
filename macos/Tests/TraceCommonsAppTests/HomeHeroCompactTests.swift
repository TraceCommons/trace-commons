import XCTest
@testable import TraceCommonsApp

/// Home's 3-steps card goes compact when the full card would push the
/// stats out of view, and its X is the person's choice, kept under its own
/// key (product owner, 2026-11-07).
@MainActor
final class HomeHeroCompactTests: XCTestCase {
    typealias Hero = HomeTracesHero

    func test_aNarrowCardIsCompact() {
        XCTAssertTrue(Hero.isCompact(width: 400, visibleHeight: 2000, fullHeight: 360))
        XCTAssertFalse(Hero.isCompact(width: 900, visibleHeight: 2000, fullHeight: 360))
    }

    func test_aShortHomeIsCompactWhenTheStatsWouldNotShow() {
        // 360 of card and 200 for the first stats: 560 is just enough.
        XCTAssertFalse(Hero.isCompact(width: 900, visibleHeight: 560, fullHeight: 360))
        XCTAssertTrue(Hero.isCompact(width: 900, visibleHeight: 559, fullHeight: 360))
    }

    func test_beforeAnythingIsMeasuredTheCardIsFull() {
        XCTAssertFalse(Hero.isCompact(width: 0, visibleHeight: 0, fullHeight: 360))
    }

    func test_closingIsKeptUnderItsOwnKey() throws {
        XCTAssertEqual(Hero.closedKey, "home.stepsCardClosed")
        XCTAssertNotEqual(Hero.closedKey, "home.tracesHeroClosed", "a card closed in an older version stays shown")
        let source = try String(
            contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent("Views/Monitor/HomeTracesHero.swift"),
            encoding: .utf8)
        XCTAssertTrue(source.contains("closeLabel: MonitorWords.close, onClose: { closed = true },"))
        XCTAssertTrue(source.contains("compact: compact"))
    }
}
