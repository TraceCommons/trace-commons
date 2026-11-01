import os
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// Records what `DaemonClient` sent and answers with whatever the test set,
/// so the method name and the parameter object are assertable without a
/// running daemon. The same shape as the double in `SetSettingsTests`.
private final class GrantRecordingDaemon: DaemonCalling {
    private struct State: Sendable {
        var recorded: [(method: String, params: String)] = []
        var methodResponses: [String: String] = [:]
        var response = #"{"id":1,"result":{}}"#
    }
    private let state = OSAllocatedUnfairLock(initialState: State())

    var methodResponses: [String: String] {
        get { state.withLock { $0.methodResponses } }
        set { state.withLock { $0.methodResponses = newValue } }
    }

    var calls: [(method: String, params: String)] {
        state.withLock { $0.recorded }
    }

    var response: String {
        get { state.withLock { $0.response } }
        set { state.withLock { $0.response = newValue } }
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        state.withLock {
            $0.recorded.append((method: method, params: paramsJSON))
            return $0.methodResponses[method] ?? $0.response
        }
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }

    func openPreview(entryID: String) throws -> TCPreview {
        throw TCDaemon.TCError.daemonGone
    }

    /// The parsed parameter object of the last call: what the daemon reads
    /// is the object, not its spelling.
    var lastParams: [String: Any]? {
        guard let last = calls.last,
              let data = last.params.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        return object
    }
}

/// The Flow 1 grant and the invite lookup, as the macOS client sends them.
final class DaemonClientFirstRunGrantTests: XCTestCase {
    private let grantedFrame =
        #"{"id":1,"result":{"granted":true,"granted_at":"2026-10-04T12:00:00.123456Z","on_disk_recorded":false,"include_existing":true}}"#

    func test_grantAutomaticSendsConfirmedAndTheWitness() throws {
        let daemon = GrantRecordingDaemon()
        daemon.response = grantedFrame
        let client = DaemonClient(daemon: daemon)

        let grant = try client.grantAutomatic(witnessSigningAddress: nil)

        XCTAssertEqual(daemon.calls.map(\.method), ["grant_automatic_including_existing"])
        let params = try XCTUnwrap(daemon.lastParams)
        XCTAssertEqual(Set(params.keys), ["confirmed", "witness_signing_address", "include_existing", "keep_folder_asks"])
        XCTAssertEqual(params["include_existing"] as? Bool, true)
        // A folder's own Ask is kept under the first run's grant (#1753).
        XCTAssertEqual(params["keep_folder_asks"] as? Bool, true)
        // A JSON true, not 1: the daemon only takes a boolean confirmation.
        XCTAssertTrue(daemon.calls[0].params.contains(#""confirmed":true"#))
        // No witness is a JSON null, never an omitted key: an omitted key is
        // refused as `automatic-grant-witness-required`.
        XCTAssertTrue(params["witness_signing_address"] is NSNull)

        XCTAssertTrue(grant.granted)
        XCTAssertNotNil(grant.grantedAt)
        XCTAssertEqual(grant.onDiskRecorded, false)
        XCTAssertEqual(grant.includeExisting, true)
    }

    func test_grantAutomaticPassesTheWitnessShown() throws {
        let daemon = GrantRecordingDaemon()
        daemon.response = grantedFrame
        let client = DaemonClient(daemon: daemon)

        _ = try client.grantAutomatic(witnessSigningAddress: "witness-address")

        let params = try XCTUnwrap(daemon.lastParams)
        XCTAssertEqual(params["witness_signing_address"] as? String, "witness-address")
        XCTAssertEqual(params["include_existing"] as? Bool, true)
    }

    func test_aRefusedGrantThrowsTheDaemonsLabel() {
        let daemon = GrantRecordingDaemon()
        daemon.response =
            #"{"id":1,"error":{"code":"bad-params","message":"automatic-grant-witness-changed"}}"#
        let client = DaemonClient(daemon: daemon)

        XCTAssertThrowsError(try client.grantAutomatic(witnessSigningAddress: nil)) { error in
            XCTAssertEqual(
                (error as? DaemonClient.Failure)?.message, "automatic-grant-witness-changed")
        }
    }

    func test_anUngrantedAnswerDecodesWithoutATime() throws {
        let daemon = GrantRecordingDaemon()
        daemon.response = #"{"id":1,"result":{"granted":false}}"#
        let client = DaemonClient(daemon: daemon)

        let grant = try client.grantAutomatic(witnessSigningAddress: nil)
        XCTAssertFalse(grant.granted)
        XCTAssertNil(grant.grantedAt)
        XCTAssertNil(grant.onDiskRecorded)
    }

    func test_anOlderDaemonCannotConfirmTheBroaderAutomaticScope() throws {
        for extra in ["", #","include_existing":false"#] {
            let daemon = GrantRecordingDaemon()
            daemon.response = #"{"id":1,"result":{"granted":true"# + extra + "}}"
            let client = DaemonClient(daemon: daemon)
            XCTAssertThrowsError(try client.grantAutomatic(witnessSigningAddress: nil))
        }
    }

    /// Ask each time withdraws the grant and returns the folders it armed
    /// to Ask; it sets no global Ask override, which no screen could clear
    /// and under which a folder set to Automatic never sent (#1753 A2).
    func test_askWithdrawsTheGrantAndSetsNoOverride() throws {
        let daemon = GrantRecordingDaemon()
        daemon.response = #"{"id":1,"result":{"withdrawn":true}}"#
        XCTAssertTrue(try DaemonClient(daemon: daemon).setAskEachTime())
        XCTAssertEqual(daemon.calls.map(\.method), ["withdraw_automatic_grant"])
        let params = try XCTUnwrap(daemon.lastParams)
        XCTAssertEqual(params["return_folders_to_ask"] as? Bool, true)
    }

    func test_aRefusedAskThrowsTheDaemonsLabel() {
        let daemon = GrantRecordingDaemon()
        daemon.response = #"{"id":1,"error":{"code":"unavailable","message":"policy-write-failed"}}"#
        XCTAssertThrowsError(try DaemonClient(daemon: daemon).setAskEachTime()) { error in
            XCTAssertEqual((error as? DaemonClient.Failure)?.message, "policy-write-failed")
        }
    }

    func test_inviteLookupSendsTheInviteAsItsCode() throws {
        let daemon = GrantRecordingDaemon()
        daemon.response =
            #"{"id":1,"result":{"valid":true,"issuer_display_name":"Issuer","credit_range":{"min":10,"max":40,"unit":"points_per_accepted_trace"}}}"#
        let client = DaemonClient(daemon: daemon)
        let invite = "https://issuer.tracecommons.ai/onboard#VQWWPGYSG8Y4LTP6"

        let lookup = try client.inviteLookup(invite)

        XCTAssertEqual(daemon.calls.map(\.method), ["invite_lookup"])
        let params = try XCTUnwrap(daemon.lastParams)
        XCTAssertEqual(Set(params.keys), ["code"])
        XCTAssertEqual(params["code"] as? String, invite)
        XCTAssertTrue(lookup.valid)
        XCTAssertEqual(lookup.issuerDisplayName, "Issuer")
        XCTAssertEqual(lookup.creditRange?.max, 40)
    }
}
