import TCShellCore
import XCTest

/// When the Traces tab's resume notice shows (owner, 2026-11-04).
final class PausedResumeNoticeTests: XCTestCase {
    private func shows(
        paused: Bool = true, loggedIn: Bool = true, needsAttention: Bool = false,
        contributionHeld: Bool = false, dismissed: Bool = false
    ) -> Bool {
        PausedResumeNotice.shows(
            paused: paused, loggedIn: loggedIn, needsAttention: needsAttention,
            contributionHeld: contributionHeld, dismissedThisVisit: dismissed)
    }

    func testPausedShows() {
        XCTAssertTrue(shows())
    }

    func testRunningDoesNotShow() {
        XCTAssertFalse(shows(paused: false))
    }

    func testPausedButHeldForAnotherReasonDoesNotShow() {
        XCTAssertFalse(shows(needsAttention: true), "the account needs attention")
        XCTAssertFalse(shows(contributionHeld: true), "automatic contribution is held")
        XCTAssertFalse(shows(loggedIn: false), "this device is not enrolled")
    }

    func testDismissedThisVisitDoesNotShow() {
        XCTAssertFalse(shows(dismissed: true))
    }

    func testReopeningTheTracesTabShowsItAgain() {
        var closed: Set<String> = ["other"]
        closed.insert(PausedResumeNotice.id)  // Not now
        XCTAssertFalse(shows(dismissed: closed.contains(PausedResumeNotice.id)))
        // Another tab: still hidden.
        closed = PausedResumeNotice.closed(closed, afterOpeningTraces: false)
        XCTAssertTrue(closed.contains(PausedResumeNotice.id))
        // Traces again: a new visit.
        closed = PausedResumeNotice.closed(closed, afterOpeningTraces: true)
        XCTAssertFalse(closed.contains(PausedResumeNotice.id))
        XCTAssertTrue(closed.contains("other"), "other closed notices are left alone")
        XCTAssertTrue(shows(dismissed: closed.contains(PausedResumeNotice.id)))
    }
}
