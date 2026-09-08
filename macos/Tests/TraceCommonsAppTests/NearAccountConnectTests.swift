import TCBridge
import XCTest
@testable import TraceCommonsApp

private final class NearRecordingDaemon: DaemonCalling {
    /// Locked pre-emptively, not because a crash was seen here. These tests
    /// drive `DaemonClient` directly and synchronously on the test thread
    /// and never go through `AppModel`, so no `Task.detached` fan-out
    /// reaches this double today. Routing it through `AppModel` would make
    /// it race exactly as CredentialCancelTests did; the guard is free and
    /// removes the trap rather than leaving it for whoever does that.
    private let lock = NSLock()
    private var responseValue = #"{"id":1,"result":{"ready":true}}"#
    private var recorded: [(String, String)] = []

    var response: String {
        get { lock.lock(); defer { lock.unlock() }; return responseValue }
        set { lock.lock(); defer { lock.unlock() }; responseValue = newValue }
    }

    var calls: [(String, String)] {
        lock.lock()
        defer { lock.unlock() }
        return recorded
    }

    func call(_ method: String, params: String) -> String {
        lock.lock()
        defer { lock.unlock() }
        recorded.append((method, params))
        return responseValue
    }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}

final class NearAccountConnectTests: XCTestCase {
    func testMissingCoreViewDoesNotGrantReadiness() {
        let daemon = NearRecordingDaemon()
        XCTAssertThrowsError(try DaemonClient(daemon: daemon).nativeWalletFlow(action: "open", flowID: "", commons: "", account: ""))
        XCTAssertEqual(daemon.calls.map(\.0), ["native_wallet_flow"])
    }
    func testPreparationRequiresExplicitSessionBackendAndConfirmation() throws {
        let daemon = NearRecordingDaemon()
        daemon.response = #"{"id":1,"result":{"status":"ready_for_next_inference","expires_at":123}}"#
        let result = try DaemonClient(daemon: daemon).prepareAdmissionSession(entryID: "selected", backend: "near-funded")
        XCTAssertEqual(result.status, "ready_for_next_inference")
        XCTAssertNil(result.view, "an old raw success is not authoritative readiness")
        XCTAssertEqual(daemon.calls.map(\.0), ["prepare_admission_session"])
        let params = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(daemon.calls[0].1.utf8)) as? [String: Any])
        XCTAssertEqual(Set(params.keys), ["entry_id", "backend", "confirmed"])
        XCTAssertEqual(params["entry_id"] as? String, "selected")
        XCTAssertEqual(params["backend"] as? String, "near-funded")
        XCTAssertEqual(params["confirmed"] as? Bool, true)
    }
}
