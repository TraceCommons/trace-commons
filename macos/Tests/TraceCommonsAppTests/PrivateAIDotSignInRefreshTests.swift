import Combine
import os
import TCBridge
import TCDesign
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// A daemon whose NEAR AI key the test gives and takes away. Its Private
/// AI listener says who answers from the key, as the daemon's
/// `label_for` does: answered elsewhere without one, running with one.
private final class KeyDaemon: DaemonCalling {
    private struct State: Sendable {
        var signedIn = false
        var recorded: [String] = []
    }
    private let state = OSAllocatedUnfairLock(initialState: State())

    func setSignedIn(_ signedIn: Bool) { state.withLock { $0.signedIn = signedIn } }
    func count(_ method: String) -> Int { state.withLock { $0.recorded.filter { $0 == method }.count } }

    func call(_ method: String, params paramsJSON: String) -> String {
        state.withLock { state in
            state.recorded.append(method)
            switch method {
            case "near_ai_credential_status":
                // No attempt id: the Sign in button's own flow
                // (`signInToNearAI`) and an outside sign-in hold none.
                return state.signedIn
                    ? #"{"id":1,"result":{"state":"present","session_state":"present"}}"#
                    : #"{"id":1,"result":{"state":"absent","session_state":"absent"}}"#
            case "get_settings":
                let label = state.signedIn ? "running" : "running_answered_elsewhere"
                return """
                    {"id":1,"result":{"quiescence_secs":45,"digest_interval_secs":3600,\
                    "local_notifications":true,"queue_ttl_days":14,"max_queue_entries":500,\
                    "max_uploads_per_day":100,"near_ai_configured":\(state.signedIn),\
                    "claude_root_configured":true,"codex_root_configured":true,\
                    "private_inference":true,"private_inference_offer_seen":true,\
                    "private_inference_state":{"state":"\(label)","port":4000}}}
                    """
            case "near_ai_balance":
                return #"{"id":1,"result":{"state":"no_session","remaining_nanos":null,"spend_limit_nanos":null,"total_spent_nanos":null,"scale":9,"observed_at":null}}"#
            default:
                return #"{"id":1,"error":{"code":"unavailable","message":"synthetic_refusal"}}"#
            }
        }
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}

/// Owner, 2026-10-29: "the color should update if I login to near.ai". A
/// NEAR AI sign-in or sign-out that lands re-reads Private AI's state at
/// once, whichever path it came by, and every dot reads one colour for it.
final class PrivateAIDotSignInRefreshTests: XCTestCase {
    @MainActor
    private func settled(_ model: AppModel, to label: String) -> (XCTestExpectation, AnyCancellable) {
        let expectation = expectation(description: label)
        let subscription = model.$daemonSettings
            .filter { $0?.privateInferenceState?.state == label }
            .prefix(1)
            .sink { _ in expectation.fulfill() }
        return (expectation, subscription)
    }

    @MainActor
    private func credential(_ model: AppModel, _ state: String) -> (XCTestExpectation, AnyCancellable) {
        let expectation = expectation(description: "credential \(state)")
        let subscription = model.$credentialStatus
            .filter { $0.state == state }
            .prefix(1)
            .sink { _ in expectation.fulfill() }
        return (expectation, subscription)
    }

    /// The tab's dot and the heading's dot for the model's current read.
    @MainActor
    private func dots(_ model: AppModel) throws -> (tab: GlassStatus?, heading: GlassStatus, menuBar: GlassStatus) {
        let copy = try XCTUnwrap(model.privateInferenceCopy)
        let view = try XCTUnwrap(model.daemonSettings?.privateInferenceState)
        let tab = MonitorWindowView.inferenceDot(model.privateInferenceState, calls: model.privateInferenceCalls)
        let heading = PrivateAIInspectorView.statusRow(
            DaemonData.PrivateInferenceState(state: view.state, port: view.port.map(Int.init)),
            copy: copy, calls: model.privateInferenceCalls).dot
        let menuBar = MenuPanelStatus.privateAIDot(MenuPanelStatus.privateAI(model))
        return (tab, heading, menuBar)
    }

    @MainActor
    func testSigningInAndOutReReadsPrivateAIAndMovesEveryDotTogether() async throws {
        let daemon = KeyDaemon()
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))

        // Signed out: the listener is on, answered by the tools' own
        // accounts. Amber on the tab, the heading and the menu bar alike.
        let (signedOutRead, a) = settled(model, to: "running_answered_elsewhere")
        model.refreshSettings()
        let (absent, b) = credential(model, "absent")
        model.refreshNearAiCredential()
        await fulfillment(of: [signedOutRead, absent], timeout: 3)
        a.cancel(); b.cancel()
        var now = try dots(model)
        XCTAssertEqual(now.tab, .ask)
        XCTAssertEqual(now.heading, .ask)
        XCTAssertEqual(now.menuBar, .ask)
        let settingsReads = daemon.count("get_settings")

        // The sign-in lands with no attempt held (the Sign in button's own
        // flow ends in this one credential read). Nothing else is pressed.
        daemon.setSignedIn(true)
        let (signedInRead, c) = settled(model, to: "running")
        model.refreshNearAiCredential()
        await fulfillment(of: [signedInRead], timeout: 3)
        c.cancel()
        XCTAssertGreaterThan(daemon.count("get_settings"), settingsReads, "the sign-in did not re-read Private AI")
        now = try dots(model)
        XCTAssertEqual(now.tab, .on)
        XCTAssertEqual(now.heading, .on)
        XCTAssertEqual(now.menuBar, .on)

        // Signing out (Forget, or an expiry) re-reads it too.
        daemon.setSignedIn(false)
        let (signedOutAgain, d) = settled(model, to: "running_answered_elsewhere")
        model.refreshNearAiCredential()
        await fulfillment(of: [signedOutAgain], timeout: 3)
        d.cancel()
        now = try dots(model)
        XCTAssertEqual(now.tab, .ask)
        XCTAssertEqual(now.heading, .ask)
        XCTAssertEqual(now.menuBar, .ask)
    }

    /// Only a key that came or went between two answered reads re-reads:
    /// the first answer after launch, and a read that says the same again,
    /// do not.
    func testOnlyAPresenceChangeBetweenAnsweredReadsCounts() {
        let present = CredentialStatus(state: CredentialSurface.statePresent)
        let absent = CredentialStatus(state: "absent")
        let obtaining = CredentialStatus(state: "obtaining")
        XCTAssertTrue(AppModel.credentialPresenceMoved(from: absent, to: present))
        XCTAssertTrue(AppModel.credentialPresenceMoved(from: obtaining, to: present))
        XCTAssertTrue(AppModel.credentialPresenceMoved(from: present, to: absent))
        XCTAssertFalse(AppModel.credentialPresenceMoved(from: present, to: present))
        XCTAssertFalse(AppModel.credentialPresenceMoved(from: absent, to: obtaining))
        XCTAssertFalse(AppModel.credentialPresenceMoved(from: .unreported, to: present))
        XCTAssertFalse(AppModel.credentialPresenceMoved(from: present, to: .unreported))
    }
}
