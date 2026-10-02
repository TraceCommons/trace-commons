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

    /// The counterpart of the Tauri frontend's `health-recovery.test.mjs`:
    /// "only the NEAR AI notice label offers the notice recovery", and "the
    /// confirmation is offered only with the shared notice loaded".
    ///
    /// Tauri's version has `loading` and `unavailable` states besides `ready`,
    /// because its copy arrives over an async IPC round trip to the daemon
    /// and can still be in flight, or fail, when the health label already
    /// has. This shell's copy is compiled into the same dylib the health
    /// label itself came from (`TCCoreCopy.privacyScanCopyJSON()`, no IPC),
    /// so there is no "still loading" interval to pin -- `HealthCopy.forLabel`
    /// either reads the recovery sentence straight out of `PrivacyScanCopy`
    /// (`ready`), or -- if the dylib's own table somehow failed to decode --
    /// falls back to the generic on-hold sentence (`forLabel("")`), which is
    /// what Tauri's `unavailable` means: offer nothing rather than a
    /// half-read prompt.
    func testOnlyTheNearAiNoticeLabelOffersTheSharedRecoveryCopy() throws {
        let copy = try XCTUnwrap(PrivacyScanCopy.decode(fromJSON: TCCoreCopy.privacyScanCopyJSON()))
        let recovery = HealthCopy.forLabel("near-ai-notice-not-acknowledged")
        XCTAssertEqual(recovery.title, copy.recoveryTitle)
        XCTAssertEqual(recovery.detail, copy.recoveryDetail)
        XCTAssertEqual(recovery.actionTitle, copy.recoveryAction)
        XCTAssertEqual(recovery.severity, .actionable)

        // No other label reads the recovery copy -- it is the core's
        // explanation for exactly this hold, not a generic sentence.
        for label in ["not-logged-in", "queue-full", "daily-cap-reached", "a-status-from-the-future"] {
            let other = HealthCopy.forLabel(label)
            XCTAssertNotEqual(other.title, copy.recoveryTitle, label)
            XCTAssertNotEqual(other.actionTitle, copy.recoveryAction, label)
        }
    }
}
