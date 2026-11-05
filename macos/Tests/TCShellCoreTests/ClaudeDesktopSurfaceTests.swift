import Foundation
import XCTest
@testable import TCShellCore

final class ClaudeDesktopSurfaceTests: XCTestCase {
    private func decode(_ fields: [String: Any]) throws -> ClaudeDesktopStatus {
        var fields = fields
        fields["copy"] = Dictionary(uniqueKeysWithValues: ClaudeDesktopStatus.copyKeys.map { ($0, $0) })
        return try JSONDecoder().decode(ClaudeDesktopStatus.self,
                                       from: JSONSerialization.data(withJSONObject: fields))
    }

    func testReadyIsConfiguredWithoutObservedClaim() throws {
        let status = try decode(["state": "ready", "observed": false, "model": "main",
                                 "background_model": "background", "environment": "preset"])
        XCTAssertEqual(status.state, .ready)
        XCTAssertFalse(status.observed)
        XCTAssertEqual(status.statusLine, "status_ready")
        XCTAssertEqual(status.backgroundModel, "background")
        XCTAssertEqual(status.environment, "preset")
        XCTAssertNil(status.withoutEnvironment().environment)
    }

    func testOnlyMatchingDaemonEvidenceCanSayObserved() throws {
        XCTAssertTrue(try decode(["state": "observed", "observed": true]).observed)
        let inconsistent = try decode(["state": "observed", "observed": false])
        XCTAssertEqual(inconsistent.state, .ready)
        XCTAssertFalse(inconsistent.observed)
        XCTAssertFalse(try decode(["state": "ready", "observed": true]).observed)
    }

    func testUnknownStateFailsClosedAndDropsEnvironment() throws {
        let status = try decode(["state": "future_state", "observed": true, "environment": "preset", "model": "main"])
        XCTAssertEqual(status.state, .unavailable)
        XCTAssertFalse(status.canConfigure)
        XCTAssertFalse(status.canDisconnect)
        XCTAssertFalse(status.withoutEnvironment().canConfigure)
        XCTAssertFalse(status.withoutEnvironment().canDisconnect)
        XCTAssertFalse(status.observed)
        XCTAssertNil(status.environment)
        XCTAssertEqual(status.statusLine, "status_unavailable")
    }

    func testKnownUnavailableAllowsRetryAndResetWithoutReadableBinding() throws {
        let status = try decode(["state": "unavailable", "observed": false])
        XCTAssertNil(status.model)
        XCTAssertTrue(status.canConfigure)
        XCTAssertTrue(status.canDisconnect)
        XCTAssertTrue(status.withoutEnvironment().canConfigure)
        XCTAssertTrue(status.withoutEnvironment().canDisconnect)
        let unknown = try decode(["state": "future_state", "observed": false])
        XCTAssertFalse(unknown.canConfigure)
        XCTAssertFalse(unknown.canDisconnect)
    }

    func testUnavailableSignInAndDisabledNeverKeepEnvironment() throws {
        for state in ["disconnected", "unavailable", "sign_in_required", "disabled"] {
            let status = try decode(["state": state, "observed": false, "environment": "preset"])
            XCTAssertNil(status.environment)
            XCTAssertEqual(status.statusLine, "status_" + state)
        }
    }

    func testMissingSharedCopyCannotCreatePartialUI() throws {
        let data = Data(#"{"state":"disconnected","observed":false,"copy":{"title":"title"}}"#.utf8)
        XCTAssertThrowsError(try JSONDecoder().decode(ClaudeDesktopStatus.self, from: data))
    }
}
