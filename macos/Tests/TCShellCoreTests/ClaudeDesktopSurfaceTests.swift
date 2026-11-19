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

/// The Private AI tab's Claude Desktop card: an offer while it is not set
/// up, and its status with Disconnect while it is.
final class ClaudeDesktopPromptTests: XCTestCase {
    private func status(_ state: String, observed: Bool = false, model: String? = nil,
                        promptCopy: Bool = true) throws -> ClaudeDesktopStatus {
        var copy = Dictionary(uniqueKeysWithValues: ClaudeDesktopStatus.copyKeys.map { ($0, $0) })
        if promptCopy { for key in ClaudeDesktopStatus.promptCopyKeys { copy[key] = key } }
        var fields: [String: Any] = ["state": state, "observed": observed, "copy": copy]
        fields["model"] = model
        return try JSONDecoder().decode(ClaudeDesktopStatus.self, from: JSONSerialization.data(withJSONObject: fields))
    }

    private func mode(featureOn: Bool = true, installed: Bool = true, dismissed: Bool = false,
                      _ status: ClaudeDesktopStatus?) -> ClaudeDesktopPrompt.Mode {
        ClaudeDesktopPrompt.mode(featureOn: featureOn, desktopInstalled: installed, dismissed: dismissed, status: status)
    }

    func testOfferedOnlyWhileInstalledFlaggedUndismissedAndNotSetUp() throws {
        let off = try status("disconnected")
        XCTAssertEqual(mode(off), .offer)
        XCTAssertEqual(mode(featureOn: false, off), .hidden)
        XCTAssertEqual(mode(installed: false, off), .hidden)
        XCTAssertEqual(mode(dismissed: true, off), .hidden)
        XCTAssertEqual(mode(nil), .hidden)
    }

    func testASetUpConnectionShowsItsStatusWithDisconnect() throws {
        for set in [try status("ready", model: "main"), try status("observed", observed: true, model: "main")] {
            XCTAssertEqual(mode(set), .connected)
            // Dismissing the offer, or Claude Desktop since removed, never
            // hides a live connection's way to turn it off.
            XCTAssertEqual(mode(dismissed: true, set), .connected)
            XCTAssertEqual(mode(installed: false, set), .connected)
            XCTAssertTrue(set.canDisconnect)
        }
        XCTAssertEqual(mode(featureOn: false, try status("ready", model: "main")), .hidden)
    }

    func testNothingWhenSetupCannotRunHere() throws {
        for state in ["unavailable", "sign_in_required", "disabled", "later_state"] {
            XCTAssertEqual(mode(try status(state)), .hidden, state)
        }
    }

    func testADaemonWithoutThePromptWordsMakesNoOffer() throws {
        XCTAssertEqual(mode(try status("disconnected", promptCopy: false)), .hidden)
        XCTAssertEqual(mode(try status("ready", model: "main", promptCopy: false)), .connected)
    }
}
