import TCBridge
import XCTest

final class OnboardingCopyExportTests: XCTestCase {
    func testSharedCopyDoesNotInventSourceCoverageOrReminderEditor() throws {
        let copy = try XCTUnwrap(TCOnboardingCopy.load())
        XCTAssertTrue(copy.welcomeBody.contains("according to your source settings"))
        XCTAssertTrue(copy.doneBody.contains("sessions to review"))
        XCTAssertFalse(copy.doneBody.contains("reminder settings"))
        XCTAssertFalse(copy.doneBody.contains("4 hours"))
        XCTAssertFalse(copy.welcomeBody.contains("Claude Code"))
        XCTAssertTrue(copy.notificationPurpose.contains("never submit"))
        XCTAssertNotEqual(copy.notificationUnknown, copy.notificationAllowed)
        XCTAssertFalse(copy.systemSettings.isEmpty)
    }
}
