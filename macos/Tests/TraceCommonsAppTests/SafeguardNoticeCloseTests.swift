import XCTest
import TCDesign
@testable import TraceCommonsApp

/// The Traces tab's safeguard notices (daily upload limit, a busy witness,
/// gate holds) can be closed; a closed one comes back when its cause recurs.
/// A core that is down or unread is never closable.
@MainActor
final class SafeguardNoticeCloseTests: XCTestCase {
    private func banner(_ title: String, _ tone: GlassStatus, index: Int = 0) -> TracesHealth.Banner {
        .init(title: title, detail: "detail", tone: tone, index: index)
    }

    func testAWaitingSafeguardIsClosableByAStableID() {
        let first = MonitorNotices.healthCloseID(for: banner("Today's upload limit is used up.", .ask, index: 0))
        let moved = MonitorNotices.healthCloseID(for: banner("Today's upload limit is used up.", .ask, index: 2))
        XCTAssertNotNil(first)
        // Another safeguard appearing above it must not reopen a closed card.
        XCTAssertEqual(first, moved)
    }

    func testPersistentSafeguardIdentityUsesConditionInsteadOfCopyOrStackPosition() {
        let before = TracesHealth.Banner(title: "Earlier title", detail: "500 waiting", tone: .ask, condition: "queue-full")
        let after = TracesHealth.Banner(title: "Updated title", detail: "600 waiting", tone: .ask, index: 2, condition: "queue-full")
        XCTAssertEqual(MonitorNotices.healthCloseID(for: before), "health:queue-full")
        XCTAssertEqual(MonitorNotices.healthCloseID(for: before), MonitorNotices.healthCloseID(for: after))
        XCTAssertNil(MonitorNotices.healthCloseID(for: .init(title: "Core down", detail: nil, tone: .outside, condition: "queue-full")))
    }

    func testACoreThatIsDownOrUnreadCannotBeClosed() {
        XCTAssertNil(MonitorNotices.healthCloseID(for: banner("Trace Commons is not running.", .outside)))
    }

    func testAClosedSafeguardReturnsOnceItsCauseHasGone() {
        let id = MonitorNotices.healthCloseID(for: banner("Today's upload limit is used up.", .ask))!
        let closed: Set<String> = [id, "folder-notice:x"]
        // Still current: stays closed.
        XCTAssertEqual(MonitorNotices.prunedClosed(closed, currentHealth: [id]), closed)
        // Gone (the limit reset): forgotten, so the next block shows again.
        // Other closed cards are untouched.
        XCTAssertEqual(MonitorNotices.prunedClosed(closed, currentHealth: []), ["folder-notice:x"])
    }
}
