import Foundation
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

final class NativePasskeyClientTests: XCTestCase {
    func testCompleteSendsOnlyLocalCeremonyAndRawAppleFields() throws {
        let daemon = NativeIdentityWireFixture()
        let client = DaemonClient(daemon: daemon)
        let credential = try NativePasskeyCredential.assertion(credentialID: Data([1]),
            clientDataJSON: Data([2]), authenticatorData: Data([3]), signature: Data([4]), userHandle: Data([5]))
        _ = try client.passkeyComplete(.login, ceremony: "local-handle", credential: credential)
        let call = try XCTUnwrap(daemon.calls.last)
        XCTAssertEqual(call.0, "passkey_login_complete")
        let params = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(call.1.utf8)) as? [String: Any])
        XCTAssertEqual(Set(params.keys), ["ceremony", "credential_id", "raw_client_data_json", "raw_authenticator_data", "signature", "user_handle"])
        XCTAssertEqual(params["user_handle"] as? String, "BQ")
        XCTAssertEqual(params["ceremony"] as? String, "local-handle")
        XCTAssertEqual(params["signature"] as? String, "BA")
    }

    func testUnavailableMethodsNeverStartNativeCeremony() throws {
        let daemon = NativeIdentityWireFixture()
        daemon.methods = []
        let client = DaemonClient(daemon: daemon)
        XCTAssertThrowsError(try client.passkeyBegin(.create, label: nil)) { error in
            XCTAssertEqual((error as? DaemonClient.Failure)?.code, "unknown_method")
        }
        XCTAssertEqual(daemon.calls.map { $0.0 }, ["hello"])
    }

    func testUnknownDaemonStatusRetainsNullableIdentity() throws {
        let client = DaemonClient(daemon: NativeIdentityWireFixture())
        let status = try client.accountSessionStatus()
        XCTAssertEqual(status.state, "unknown")
        XCTAssertNil(status.signedIn)
    }

    func testCapabilityChangeAfterReconnectIsCheckedAgain() throws {
        let daemon = NativeIdentityWireFixture()
        let client = DaemonClient(daemon: daemon)
        _ = try client.accountSessionStatus()
        daemon.methods = []
        XCTAssertThrowsError(try client.accountSessionStatus()) { error in
            XCTAssertEqual((error as? DaemonClient.Failure)?.code, "unknown_method")
        }
        XCTAssertEqual(daemon.calls.map { $0.0 }, ["hello", "account_session_status", "hello"])
    }

    func testUnenrollIsSentOnlyToADaemonThatAdvertisesIt() throws {
        let daemon = NativeIdentityWireFixture()
        daemon.methods.append("unenroll")
        let client = DaemonClient(daemon: daemon)
        try client.unenroll()
        XCTAssertEqual(daemon.calls.map { $0.0 }, ["hello", "unenroll"])

        let old = NativeIdentityWireFixture()
        XCTAssertThrowsError(try DaemonClient(daemon: old).unenroll())
        XCTAssertEqual(old.calls.map { $0.0 }, ["hello"])
    }

    func testCoreDownRefusesWithoutManufacturingAccountState() {
        let daemon = NativeIdentityWireFixture()
        daemon.coreDown = true
        let client = DaemonClient(daemon: daemon)
        XCTAssertThrowsError(try client.accountSessionStatus()) { error in
            XCTAssertEqual((error as? DaemonClient.Failure)?.code, "unavailable")
        }
        XCTAssertEqual(daemon.calls.map { $0.0 }, ["hello"])
    }
}

private final class NativeIdentityWireFixture: DaemonCalling {
    var methods = ["passkey_create_begin", "passkey_create_complete", "passkey_login_begin", "passkey_login_complete", "passkey_add_begin", "passkey_add_complete", "passkey_cancel", "account_bind", "account_binding", "passkey_state", "account_sign_in", "account_session_status", "account_sign_out"]
    var calls: [(String, String)] = []
    var coreDown = false
    func call(_ method: String, params: String) -> String {
        calls.append((method, params))
        if coreDown { return #"{"error":{"code":"unavailable","message":"handle-freed"}}"# }
        let result: [String: Any]
        switch method {
        case "hello": result = ["methods": methods]
        case "account_session_status": result = ["state": "unknown", "signed_in": NSNull(), "expires_at": NSNull()]
        case "unenroll": result = ["unenrolled": true, "removed": true, "approvals_returned": 0]
        default: result = ["binding_state": "unbound"]
        }
        return String(decoding: try! JSONSerialization.data(withJSONObject: ["result": result]), as: UTF8.self)
    }
    func openPreview(entryID: String) throws -> TCPreview { fatalError("No preview in identity tests") }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
}
