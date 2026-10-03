import TCBridge
import TCShellCore
import XCTest

/// The health banner's words cross the ABI (R6/R7): nothing for a healthy
/// daemon, the core-down banner for an unreachable one, and an on-hold line
/// -- never the raw label -- for a label this build has never heard of.
final class HealthCopyExportTests: XCTestCase {
    func testAReachableDaemonThatReportsNothingHasNoBanner() {
        XCTAssertNil(TCCoreCopy.healthCopyJSON(reachable: true, label: nil, maxQueueEntries: nil))
        XCTAssertNil(TCCoreCopy.healthCopyJSON(reachable: true, label: "", maxQueueEntries: nil))
    }

    func testAnUnreachableDaemonGetsTheCoreDownBanner() throws {
        let line = try XCTUnwrap(HealthLineCopy.decode(
            fromJSON: TCCoreCopy.healthCopyJSON(reachable: false, label: nil, maxQueueEntries: nil)))
        XCTAssertFalse(line.title.isEmpty)
        XCTAssertEqual(line.severity, .actionable)
        XCTAssertNil(line.action)
        XCTAssertNil(line.actionKind)
    }

    func testOnlyQueueFullReviewsTheQueueAndAnUnknownLabelIsOnHold() throws {
        let full = try XCTUnwrap(HealthLineCopy.decode(
            fromJSON: TCCoreCopy.healthCopyJSON(reachable: true, label: "queue-full", maxQueueEntries: 500)))
        XCTAssertEqual(full.actionKind, .reviewQueue)
        XCTAssertTrue(full.detail.contains("500"))
        let future = try XCTUnwrap(HealthLineCopy.decode(
            fromJSON: TCCoreCopy.healthCopyJSON(reachable: true, label: "a-label-from-the-future", maxQueueEntries: nil)))
        XCTAssertFalse(future.title.contains("a-label-from-the-future"))
        XCTAssertFalse(future.detail.contains("a-label-from-the-future"))
        XCTAssertEqual(future.severity, .waiting)
    }

    func testADecodeFailureIsNilNeverAnEmptyBanner() {
        XCTAssertNil(HealthLineCopy.decode(fromJSON: nil))
        XCTAssertNil(HealthLineCopy.decode(fromJSON: "not json"))
        XCTAssertNil(HealthLineCopy.decode(
            fromJSON: #"{"title":"","detail":"x","action":null,"action_kind":null,"severity":"waiting"}"#))
    }
}
