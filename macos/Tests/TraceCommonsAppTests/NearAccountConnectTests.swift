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
    func testCapabilitiesNeverStartSignupAndMissingReadinessRefuses() throws {
        let daemon = NearRecordingDaemon()
        let client = DaemonClient(daemon: daemon)
        XCTAssertTrue(try client.nearAccountCapabilities(commons: "https://commons.example").ready)
        XCTAssertEqual(daemon.calls.map(\.0), ["near_account_capabilities"])
        daemon.response = #"{"id":1,"result":{}}"#
        XCTAssertThrowsError(try client.nearAccountCapabilities(commons: "https://commons.example"))
    }
    func testWalletHandoffCarriesOnlySelectedServiceAndAccount() throws {
        let daemon = NearRecordingDaemon()
        daemon.response = #"{"id":1,"result":{"status":"waiting_for_wallet","attempt_id":"test-attempt","browser_url":"https://commons.example/account/near/provision/wallet#opaque"}}"#
        let client = DaemonClient(daemon: daemon)
        let progress = try client.nearAccountStart(commons: "https://commons.example", account: "alice.near")
        XCTAssertEqual(progress.attemptID, "test-attempt")
        XCTAssertNotNil(progress.browserURLFor(commons: "https://commons.example"))
        let params = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(daemon.calls[0].1.utf8)) as? [String: String])
        XCTAssertEqual(params, ["ingest_url": "https://commons.example", "account_id": "alice.near"])
    }
    func testBrowserHandoffRejectsDifferentOriginsAndCredentials() {
        for target in ["http://commons.example/wallet", "https://other.example/wallet", "https://commons.example:444/wallet", "https://user@commons.example/wallet"] {
            let progress = NearAccountProgress(status: "waiting_for_wallet", attemptID: "id", browserURL: target)
            XCTAssertNil(progress.browserURLFor(commons: "https://commons.example"))
        }
    }
    func testStatusAndCancellationReferOnlyToTheLocalAttempt() throws {
        let daemon = NearRecordingDaemon()
        daemon.response = #"{"id":1,"result":{"status":"complete","attempt_id":"local"}}"#
        let client = DaemonClient(daemon: daemon)
        XCTAssertEqual(try client.nearAccountStatus(attemptID: "local").status, "complete")
        try client.nearAccountCancel(attemptID: "local")
        XCTAssertEqual(daemon.calls.map(\.0), ["near_account_status", "near_account_cancel"])
        for (_, payload) in daemon.calls {
            let fields = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(payload.utf8)) as? [String: String])
            XCTAssertEqual(fields, ["attempt_id": "local"])
        }
    }
}
