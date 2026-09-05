import TCBridge
import XCTest
@testable import TraceCommonsApp

private final class NearRecordingDaemon: DaemonCalling {
    var response = #"{"id":1,"result":{"ready":true}}"#
    var calls: [(String, String)] = []
    func call(_ method: String, params: String) -> String { calls.append((method, params)); return response }
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
