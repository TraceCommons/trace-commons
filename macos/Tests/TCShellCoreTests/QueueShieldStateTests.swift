import XCTest

@testable import TCShellCore

/// The nav item's shield. It adds a state the bare count could never carry;
/// it does NOT replace the count -- at 149 waiting sessions, "how many" is
/// the signal a person is actually using.
final class QueueShieldStateTests: XCTestCase {
    func testAnEmptyQueueIsClear() {
        XCTAssertEqual(
            QueueShieldState.state(waiting: 0, nothingMatched: 0, trimmed: 0),
            .clear
        )
    }

    func testAnOrdinaryQueueIsWaiting() {
        XCTAssertEqual(
            QueueShieldState.state(waiting: 12, nothingMatched: 0, trimmed: 0),
            .waiting
        )
    }

    func testASessionWhereNothingMatchedRaisesAttention() {
        XCTAssertEqual(
            QueueShieldState.state(waiting: 12, nothingMatched: 1, trimmed: 0),
            .attention
        )
    }

    func testATrimmedSessionRaisesAttention() {
        XCTAssertEqual(
            QueueShieldState.state(waiting: 12, nothingMatched: 0, trimmed: 1),
            .attention
        )
    }

    func testAnEmptyQueueIsClearEvenWithStaleFlags() {
        // Nothing is waiting, so there is nothing to be attentive about.
        XCTAssertEqual(
            QueueShieldState.state(waiting: 0, nothingMatched: 3, trimmed: 2),
            .clear
        )
    }
}
