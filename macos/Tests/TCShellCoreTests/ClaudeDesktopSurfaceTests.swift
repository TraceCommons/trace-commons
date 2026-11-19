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

/// The Private AI tab's notice offering Claude Desktop setup.
final class ClaudeDesktopPromptTests: XCTestCase {
    private func status(_ state: String, promptCopy: Bool = true) throws -> ClaudeDesktopStatus {
        var copy = Dictionary(uniqueKeysWithValues: ClaudeDesktopStatus.copyKeys.map { ($0, $0) })
        if promptCopy { for key in ClaudeDesktopStatus.promptCopyKeys { copy[key] = key } }
        let fields: [String: Any] = ["state": state, "observed": false, "copy": copy]
        return try JSONDecoder().decode(ClaudeDesktopStatus.self, from: JSONSerialization.data(withJSONObject: fields))
    }

    func testShownOnlyWhileInstalledFlaggedUndismissedAndNotSetUp() throws {
        let off = try status("disconnected")
        XCTAssertTrue(ClaudeDesktopPrompt.shows(featureOn: true, desktopInstalled: true, dismissed: false, status: off))
        XCTAssertFalse(ClaudeDesktopPrompt.shows(featureOn: false, desktopInstalled: true, dismissed: false, status: off))
        XCTAssertFalse(ClaudeDesktopPrompt.shows(featureOn: true, desktopInstalled: false, dismissed: false, status: off))
        XCTAssertFalse(ClaudeDesktopPrompt.shows(featureOn: true, desktopInstalled: true, dismissed: true, status: off))
        XCTAssertFalse(ClaudeDesktopPrompt.shows(featureOn: true, desktopInstalled: true, dismissed: false, status: nil))
    }

    func testNotShownOnceSetUpOrWhenSetupCannotRun() throws {
        for state in ["ready", "observed", "unavailable", "sign_in_required", "disabled", "later_state"] {
            XCTAssertFalse(ClaudeDesktopPrompt.shows(featureOn: true, desktopInstalled: true, dismissed: false,
                                                     status: try status(state)), state)
        }
    }

    func testADaemonWithoutThePromptWordsDrawsNoPrompt() throws {
        let status = try status("disconnected", promptCopy: false)
        XCTAssertFalse(ClaudeDesktopPrompt.shows(featureOn: true, desktopInstalled: true, dismissed: false, status: status))
    }
}
