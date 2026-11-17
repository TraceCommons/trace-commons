import XCTest

import TCShellCore
@testable import TraceCommonsApp

/// `status.terms_hold` on the app's own status: read when sent, false from
/// a daemon that predates it, and false for a value of the wrong type.
final class TermsStatusTests: XCTestCase {
    private func status(_ keys: String) throws -> DaemonStatus {
        let json = """
            {"schema_version":"v","logged_in":true,"paused":false,"queue_depth":0,
             "health":{"last_error_label":null,"since":null}\(keys)}
            """
        return try JSONDecoder().decode(DaemonStatus.self, from: Data(json.utf8))
    }

    func testStatusDecodesTermsHoldDefaultFalse() throws {
        XCTAssertFalse(try status("").termsHold)
        XCTAssertTrue(try status(#","terms_hold":true,"terms_version_owed":"v2""#).termsHold)
        XCTAssertFalse(try status(#","terms_hold":"yes""#).termsHold)
        // Separate from the user's pause.
        XCTAssertFalse(try status(#","terms_hold":true"#).paused)
    }

    /// An unenroll keeps the hold: it belongs to the person, not the org.
    func testUnenrolledKeepsTheHold() throws {
        XCTAssertTrue(try status(#","terms_hold":true"#).unenrolled().termsHold)
    }
}
