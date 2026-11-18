import XCTest
import TCBridge
import TCShellCore
@testable import TraceCommonsApp

final class HealthQueueReviewTests: XCTestCase {
    func testUnsupportedOpenCodeVersionUsesSharedRecoveryCopy() throws {
        let shared = try XCTUnwrap(TCSourceChecks.settingsCopy())
        let health = HealthCopy.core(label: "opencode-export-version-unsupported", maxQueueEntries: nil)
        XCTAssertEqual(health.title, shared.opencodeVersionTitle)
        XCTAssertEqual(health.detail, shared.opencodeVersionDetail)
        XCTAssertTrue(health.detail.contains("OpenCode 1.14.28 to 1.14.29, 1.18.0 to 1.18.35 and 1.19.0 to 1.19.5"))
        XCTAssertFalse(health.title.lowercased().contains("session"))
        XCTAssertFalse(health.detail.contains("database"))
        XCTAssertFalse(health.reviewsQueue)
        XCTAssertNil(health.actionTitle)
    }

    /// Sessions held on a busy witness get the Rust's notice, with the count
    /// and the next try, as a waiting banner with no action.
    func testWitnessCapacityBannerIsTheRustsNoticeWithTheNextTry() throws {
        let at = Date(timeIntervalSince1970: 1_893_456_060)
        let capacity = WitnessCapacity(waitingSessions: 2, nextRetryAt: at)
        let notice = try XCTUnwrap(
            TCConsentCopy.witnessCapacityNoticeJSON(forCapacity: capacity.wireJSON)
                .flatMap(WitnessCapacityNotice.decode(fromJSON:)))
        let banner = try XCTUnwrap(HealthCopy.forWitnessCapacity(capacity))
        XCTAssertEqual(banner.title, notice.title)
        XCTAssertTrue(banner.detail.hasPrefix(notice.body), banner.detail)
        XCTAssertTrue(banner.detail.contains(notice.nextCheck), banner.detail)
        XCTAssertEqual(banner.severity, .waiting)
        XCTAssertNil(banner.actionTitle)
        XCTAssertFalse(banner.reviewsQueue)
    }

    func testNoWitnessCapacityBannerWhenNothingIsWaiting() {
        XCTAssertNil(HealthCopy.forWitnessCapacity(.none))
    }

    /// The export-failure fallback is the core's own on-hold line, verbatim,
    /// so a label the core cannot word is never drawn as healthy.
    func testTheExportFailureFallbackIsTheCoresOnHoldLine() {
        XCTAssertEqual(
            HealthCopy.onHoldFallback,
            HealthCopy.core(label: "a-label-from-the-future", maxQueueEntries: nil))
        XCTAssertEqual(HealthCopy.onHoldFallback.severity, .waiting)
    }

    func testQueueFullNamesTheConfiguredLimit() {
        XCTAssertTrue(HealthCopy.core(label: "queue-full", maxQueueEntries: 250).detail.contains("250"))
    }

    func testOnlyQueueFullOffersQueueNavigation() {
        XCTAssertTrue(HealthCopy.core(label: "queue-full", maxQueueEntries: nil).reviewsQueue)
        XCTAssertEqual(HealthCopy.core(label: "queue-full", maxQueueEntries: nil).actionTitle, "Review")
        for label in ["not-logged-in", "future-label"] {
            XCTAssertFalse(HealthCopy.core(label: label, maxQueueEntries: nil).reviewsQueue)
        }
    }
}
