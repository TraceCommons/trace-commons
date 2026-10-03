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
        for label in ["not-logged-in", "near-ai-notice-not-acknowledged", "daily-cap-reached", "future-label"] {
            XCTAssertFalse(HealthCopy.core(label: label, maxQueueEntries: nil).reviewsQueue)
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
    /// so there is no "still loading" interval to pin -- `HealthCopy.core`
    /// either reads the recovery sentence straight out of `PrivacyScanCopy`
    /// (`ready`), or -- if the dylib's own table somehow failed to decode --
    /// falls back to the generic on-hold sentence (`onHoldFallback`), which is
    /// what Tauri's `unavailable` means: offer nothing rather than a
    /// half-read prompt.
    func testOnlyTheNearAiNoticeLabelOffersTheSharedRecoveryCopy() throws {
        let copy = try XCTUnwrap(PrivacyScanCopy.decode(fromJSON: TCCoreCopy.privacyScanCopyJSON()))
        let recovery = HealthCopy.core(label: "near-ai-notice-not-acknowledged", maxQueueEntries: nil)
        XCTAssertEqual(recovery.title, copy.recoveryTitle)
        XCTAssertEqual(recovery.detail, copy.recoveryDetail)
        XCTAssertEqual(recovery.actionTitle, copy.recoveryAction)
        XCTAssertEqual(recovery.severity, .actionable)

        // No other label reads the recovery copy -- it is the core's
        // explanation for exactly this hold, not a generic sentence. The two
        // other privacy holds are listed because they are the likeliest to
        // be wired to it by mistake.
        for label in [
            "not-logged-in", "queue-full", "daily-cap-reached",
            "privacy-filter-canary-failed", "pii-filter-unavailable",
            "a-status-from-the-future",
        ] {
            let other = HealthCopy.core(label: label, maxQueueEntries: nil)
            XCTAssertNotEqual(other.title, copy.recoveryTitle, label)
            XCTAssertNotEqual(other.actionTitle, copy.recoveryAction, label)
        }
    }
}
