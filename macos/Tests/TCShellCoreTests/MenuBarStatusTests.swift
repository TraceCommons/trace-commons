import XCTest

@testable import TCShellCore

/// The status item's precedence and its badge text are the only two things
/// about it that can be checked without drawing it, so they are checked.
final class MenuBarStatusTests: XCTestCase {
    func testNoDecisionsMeansNoBadge() {
        XCTAssertNil(MenuBarStatus.badgeText(decisionsOwed: 0))
        XCTAssertNil(MenuBarStatus.badgeText(decisionsOwed: -1))
    }

    func testTheBadgeStatesTheExactCountUpToTheCap() {
        XCTAssertEqual(MenuBarStatus.badgeText(decisionsOwed: 1), "1")
        XCTAssertEqual(MenuBarStatus.badgeText(decisionsOwed: 12), "12")
        XCTAssertEqual(MenuBarStatus.badgeText(decisionsOwed: 99), "99")
    }

    func testPastTheCapTheBadgeSaysNinetyNinePlus() {
        XCTAssertEqual(MenuBarStatus.badgeText(decisionsOwed: 100), "99+")
        XCTAssertEqual(MenuBarStatus.badgeText(decisionsOwed: 120), "99+")
    }

    func testACountOutranksEverything() {
        XCTAssertEqual(
            MenuBarStatus.state(decisionsOwed: 3, unhealthy: true, paused: true),
            .count("3")
        )
    }

    func testUnhealthyOutranksPaused() {
        XCTAssertEqual(
            MenuBarStatus.state(decisionsOwed: 0, unhealthy: true, paused: true),
            .attention
        )
    }

    func testPausedOutranksIdle() {
        XCTAssertEqual(
            MenuBarStatus.state(decisionsOwed: 0, unhealthy: false, paused: true),
            .paused
        )
    }

    func testNothingAtAllIsIdle() {
        XCTAssertEqual(
            MenuBarStatus.state(decisionsOwed: 0, unhealthy: false, paused: false),
            .idle
        )
    }
}
