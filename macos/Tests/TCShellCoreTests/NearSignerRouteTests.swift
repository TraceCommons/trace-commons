import Foundation
import TCShellCore
import XCTest

/// The route between the NEAR signer app and the browser. The fakes record
/// every call so "did not happen" is asserted on what was called, not on a
/// return value.
final class NearSignerRouteTests: XCTestCase {
    private actor Daemon: NearSignerDaemon {
        private(set) var calls: [String] = []
        func nativePayload(flowID: String) async throws -> NearSignerPayload {
            calls.append("native_payload")
            return NearSignerPayload(message: "m", recipient: "cloud.near.ai", nonce: Data(count: 32))
        }
        func nativeComplete(flowID: String, proof: NearSignerProof) async throws {
            calls.append("native_complete")
        }
    }

    private struct Signer: NearSignerSigning {
        let outcome: Result<NearSignerProof, NearSignerRefusal>
        func signIn(_ payload: NearSignerPayload) async throws -> NearSignerProof {
            try outcome.get()
        }
    }

    func testRouteTable() {
        XCTAssertEqual(chooseRoute(settingOn: false, signerInstalled: false), .browser)
        XCTAssertEqual(chooseRoute(settingOn: false, signerInstalled: true), .browser)
        XCTAssertEqual(chooseRoute(settingOn: true, signerInstalled: false), .browser)
        XCTAssertEqual(chooseRoute(settingOn: true, signerInstalled: true), .nativeSigner)
    }

    func testRequirementStringPinsTeamAndBundle() {
        XCTAssertEqual(
            nearSignerCodeRequirement,
            "anchor apple generic and identifier \"com.iqlusion.near-signer\" and certificate leaf[subject.OU] = \"KXSWJN7WY8\"")
        XCTAssertEqual(nearSignerBundleID, "com.iqlusion.near-signer")
    }

    func testSuccessHandsTheSignatureToTheDaemon() async {
        let daemon = Daemon()
        let proof = NearSignerProof(accountID: "a.near", publicKey: "ed25519:k", signature: "c2ln")
        let result = await runNativeSigner(flowID: "f", daemon: daemon, signer: Signer(outcome: .success(proof)))
        if case .failure(let refusal) = result { XCTFail("refused: \(refusal.label)") }
        let calls = await daemon.calls
        XCTAssertEqual(calls, ["native_payload", "native_complete"])
    }

    func testRefusalNeverAutoFallsBack() async {
        let daemon = Daemon()
        let signer = Signer(outcome: .failure(NearSignerRefusal(label: "near_refused")))
        let result = await runNativeSigner(flowID: "f", daemon: daemon, signer: signer)
        guard case .failure(let refusal) = result else { return XCTFail("a refusal must be a failure") }
        XCTAssertEqual(refusal.label, "near_refused")
        // The browser route is the daemon's "start"; a refusal must not reach
        // it, and nothing is completed either.
        let calls = await daemon.calls
        XCTAssertEqual(calls, ["native_payload"])
    }
}
