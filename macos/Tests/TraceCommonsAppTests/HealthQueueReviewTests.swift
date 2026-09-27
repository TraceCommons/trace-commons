import XCTest
import TCBridge
import TCShellCore
@testable import TraceCommonsApp

final class HealthQueueReviewTests: XCTestCase {
    func testUnsupportedOpenCodeVersionUsesSharedRecoveryCopy() throws {
        let shared = try XCTUnwrap(TCSourceChecks.settingsCopy())
        let health = HealthCopy.forLabel("opencode-export-version-unsupported")
        XCTAssertEqual(health.title, shared.opencodeVersionTitle)
        XCTAssertEqual(health.detail, shared.opencodeVersionDetail)
        XCTAssertTrue(health.detail.contains("sessions created with OpenCode 1.18.29"))
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

    func testOnlyQueueFullOffersQueueNavigation() {
        XCTAssertTrue(HealthCopy.forLabel("queue-full").reviewsQueue)
        XCTAssertEqual(HealthCopy.forLabel("queue-full").actionTitle, "Review")
        for label in ["not-logged-in", "near-ai-notice-not-acknowledged", "daily-cap-reached", "future-label"] {
            XCTAssertFalse(HealthCopy.forLabel(label).reviewsQueue)
        }
    }
}
