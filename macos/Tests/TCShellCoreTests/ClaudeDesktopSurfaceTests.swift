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

/// The setup sheet explains itself: an intro, then the steps in order,
/// from choosing models to starting a session in Claude Desktop.
final class ClaudeDesktopSheetStepsTests: XCTestCase {
    private func status(steps: Bool) throws -> ClaudeDesktopStatus {
        var copy = Dictionary(uniqueKeysWithValues: ClaudeDesktopStatus.copyKeys.map { ($0, $0) })
        if steps { for key in ClaudeDesktopStatus.sheetStepKeys + ["sheet_intro"] { copy[key] = key } }
        let fields: [String: Any] = ["state": "disconnected", "observed": false, "copy": copy]
        return try JSONDecoder().decode(ClaudeDesktopStatus.self, from: JSONSerialization.data(withJSONObject: fields))
    }

    func testTheStepsComeInOrder() throws {
        let s = try status(steps: true)
        XCTAssertEqual(s.sheetSteps, ["sheet_step_models", "sheet_step_copy", "sheet_step_environment", "sheet_step_session"])
        XCTAssertEqual(s.text("sheet_intro"), "sheet_intro")
    }

    func testAnOlderDaemonFallsBackToItsInstructions() throws {
        XCTAssertEqual(try status(steps: false).sheetSteps, [])
    }
}

/// The Claude Desktop sheet lists only the models the daemon says its route
/// accepts (`desktop_models`); an older daemon's catalogue is unchanged.
final class ClaudeDesktopCatalogueTests: XCTestCase {
    private func catalogue(desktop: [String]?, default def: String = "small/model") throws -> ManagedModelCatalogue {
        var value: [String: Any] = [
            "models": ["big/model", "small/model", "huge/model"].map { ["id": $0] },
            "default": def, "favorites": ["small/model", "big/model"],
        ]
        if let desktop { value["desktop_models"] = desktop }
        return try JSONDecoder().decode(ManagedModelCatalogue.self, from: JSONSerialization.data(withJSONObject: value))
    }

    func testTheSheetListsOnlyModelsTheRouteAccepts() throws {
        let cut = try catalogue(desktop: ["big/model", "huge/model"]).forClaudeDesktop()
        XCTAssertEqual(cut.models.map(\.id), ["big/model", "huge/model"])
        XCTAssertEqual(cut.favorites, ["big/model"])
        // The default moves to the first accepted model when it is not one.
        XCTAssertEqual(cut.default, "big/model")
        XCTAssertEqual(try catalogue(desktop: ["big/model", "huge/model"], default: "huge/model").forClaudeDesktop().default, "huge/model")
    }

    func testAnOlderDaemonsCatalogueIsUnchanged() throws {
        let old = try catalogue(desktop: nil)
        XCTAssertEqual(old.forClaudeDesktop(), old)
    }
}

/// The sheet's Open Claude Desktop button uses Claude Desktop's documented
/// deep link for a new Code session.
final class ClaudeDesktopOpenLinkTests: XCTestCase {
    func testTheLinkOpensANewCodeSession() {
        XCTAssertEqual(ClaudeDesktopPrompt.newCodeSessionURL.absoluteString, "claude://code/new")
    }
}
