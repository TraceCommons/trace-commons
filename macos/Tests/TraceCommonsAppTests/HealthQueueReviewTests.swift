import XCTest
import TCBridge
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

    func testOnlyQueueFullOffersQueueNavigation() {
        XCTAssertTrue(HealthCopy.forLabel("queue-full").reviewsQueue)
        XCTAssertEqual(HealthCopy.forLabel("queue-full").actionTitle, "Review")
        for label in ["not-logged-in", "near-ai-notice-not-acknowledged", "daily-cap-reached", "future-label"] {
            XCTAssertFalse(HealthCopy.forLabel(label).reviewsQueue)
        }
    }
}
