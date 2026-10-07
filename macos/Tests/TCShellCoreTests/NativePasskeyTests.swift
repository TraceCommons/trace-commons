import Foundation
import XCTest
@testable import TCShellCore

final class NativePasskeyTests: XCTestCase {
    func testAppleRegistrationFieldsRemainRawBytesInBase64URLTransport() throws {
        let credential = try NativePasskeyCredential.registration(
            credentialID: Data([0xfb, 0xff]), clientDataJSON: Data([0, 1, 2]),
            attestationObject: Data([3, 4]))
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(credential)) as? [String: Any])
        XCTAssertEqual(object["credential_id"] as? String, "-_8")
        XCTAssertEqual(object["raw_client_data_json"] as? String, "AAEC")
        XCTAssertEqual(object["raw_attestation_object"] as? String, "AwQ")
        XCTAssertNil(object["credential"])
        XCTAssertNil(object["access_token"])
    }

    func testIncompleteAppleFieldsRefuseBeforeComplete() {
        XCTAssertThrowsError(try NativePasskeyCredential.registration(
            credentialID: Data([1]), clientDataJSON: Data([2]), attestationObject: nil))
        XCTAssertThrowsError(try NativePasskeyCredential.assertion(
            credentialID: Data([1]), clientDataJSON: Data([2]), authenticatorData: Data(),
            signature: Data([4]), userHandle: Data([5])))
    }

    func testDiscoverableAssertionRequiresAndPreservesUserHandle() throws {
        for handle in [nil, Data()] as [Data?] {
            XCTAssertThrowsError(try NativePasskeyCredential.assertion(
                credentialID: Data([1]), clientDataJSON: Data([2]), authenticatorData: Data([3]),
                signature: Data([0xff]), userHandle: handle))
        }
        let credential = try NativePasskeyCredential.assertion(
            credentialID: Data([1]), clientDataJSON: Data([2]), authenticatorData: Data([3]),
            signature: Data([0xff]), userHandle: Data([4]))
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(credential)) as? [String: Any])
        XCTAssertEqual(object["signature"] as? String, "_w")
        XCTAssertEqual(object["user_handle"] as? String, "BA")
        XCTAssertEqual(object["raw_authenticator_data"] as? String, "Aw")
    }

    func testBase64URLRefusesPaddingNoncanonicalAndEmptyOptions() throws {
        XCTAssertEqual(try NativePasskeyEncoding.decode("-_8"), Data([0xfb, 0xff]))
        for value in ["", "YQ==", "a+", "a/", "AB", "A"] {
            XCTAssertThrowsError(try NativePasskeyEncoding.decode(value), value)
        }
    }

    func testCredentialListAcceptsRustLimitAndRefusesOverflow() throws {
        var object: [String: Any] = ["ceremony": "local", "rp_id": "tracecommons.ai",
            "challenge": "AQID", "expires_in_secs": 180, "user_verification": "required",
            "allowed_credentials": Array(repeating: "BAUG", count: 128)]
        let accepted = try JSONDecoder().decode(NativePasskeyBegin.self,
            from: JSONSerialization.data(withJSONObject: object))
        XCTAssertEqual(try accepted.validated(for: .login).allowedCredentials.count, 128)
        object["allowed_credentials"] = Array(repeating: "BAUG", count: 129)
        let overflow = try JSONDecoder().decode(NativePasskeyBegin.self,
            from: JSONSerialization.data(withJSONObject: object))
        XCTAssertThrowsError(try overflow.validated(for: .login))
    }
}

@MainActor
final class NativePasskeyCoordinatorTests: XCTestCase {
    func testCreatePresentsOnlyValidatedOptionsThenCompletes() async throws {
        let daemon = IdentityDaemonFixture()
        let provider = IdentityProviderFixture()
        let coordinator = NativePasskeyCoordinator(daemon: daemon, provider: provider)
        let result = try await coordinator.perform(.create, label: "My Mac")
        XCTAssertEqual(result.bindingState, "unbound")
        let events = await daemon.events
        XCTAssertEqual(events, ["begin:create", "complete:create"])
        XCTAssertEqual(provider.requests.first?.challenge, Data([1, 2, 3]))
        XCTAssertEqual(provider.requests.first?.excludedCredentials, [Data([7, 8, 9])])
        XCTAssertEqual(provider.requests.first?.userVerification, "required")
    }

    func testNativeCancellationConsumesCeremonyWithoutComplete() async throws {
        let daemon = IdentityDaemonFixture()
        let provider = IdentityProviderFixture()
        provider.failure = .cancelled
        let coordinator = NativePasskeyCoordinator(daemon: daemon, provider: provider)
        do { _ = try await coordinator.perform(.login); XCTFail("Cancellation must refuse") }
        catch { XCTAssertEqual(error as? NativePasskeyFailure, .cancelled) }
        let events = await daemon.events
        XCTAssertEqual(events, ["begin:login", "cancel:local-handle"])
    }

    func testInvalidOptionsCancelBeforePresentation() async throws {
        for modification in [#""rp_id":"attacker.example""#, #""challenge":"AA==""#,
                             #""algorithms":[-257]"#, #""user_verification":"unknown""#,
                             #""authenticator_attachment":"cross-platform""#] {
            let daemon = IdentityDaemonFixture()
            await daemon.setModification(modification)
            let provider = IdentityProviderFixture()
            let coordinator = NativePasskeyCoordinator(daemon: daemon, provider: provider)
            do { _ = try await coordinator.perform(.create); XCTFail("Malformed options must refuse") }
            catch { XCTAssertEqual(error as? NativePasskeyFailure, .invalidOptions) }
            XCTAssertTrue(provider.requests.isEmpty)
            let events = await daemon.events
            XCTAssertEqual(events, ["begin:create", "cancel:local-handle"])
        }
    }

    func testUnexpectedNativeCredentialNeverCompletes() async throws {
        let daemon = IdentityDaemonFixture()
        let provider = IdentityProviderFixture()
        provider.returnAssertion = true
        let coordinator = NativePasskeyCoordinator(daemon: daemon, provider: provider)
        do { _ = try await coordinator.perform(.add); XCTFail("Mismatched credential must refuse") }
        catch { XCTAssertEqual(error as? NativePasskeyFailure, .unexpectedCredential) }
        let events = await daemon.events
        XCTAssertEqual(events, ["begin:add", "cancel:local-handle"])
    }

    func testUnknownAccountStorageDoesNotBecomeSignedOutOrZeroPasskeys() throws {
        let account = try JSONDecoder().decode(NativeAccountSession.self,
            from: Data(#"{"state":"unknown","signed_in":null,"account_id":null,"expires_at":null}"#.utf8))
        XCTAssertNil(account.signedIn)
        let passkeys = try JSONDecoder().decode(NativePasskeyState.self,
            from: Data(#"{"state":"unknown","passkey_count":null,"near_ai_connected":null}"#.utf8))
        XCTAssertNil(passkeys.passkeyCount)
        XCTAssertNil(passkeys.nearAiConnected)
    }

    /// The passkeys this Mac remembers, for P-7; an older daemon sends no
    /// `remembered_name` and still decodes.
    func testRememberedPasskeysDecodeAndOlderDaemonsStillDecode() throws {
        let remembered = try JSONDecoder().decode(NativePasskeyState.self,
            from: Data(#"{"state":"none","passkey_count":2,"remembered_name":"Home","near_ai_connected":null}"#.utf8))
        XCTAssertEqual(remembered.passkeyCount, 2)
        XCTAssertEqual(remembered.rememberedName, "Home")
        let older = try JSONDecoder().decode(NativePasskeyState.self,
            from: Data(#"{"state":"none","passkey_count":null,"near_ai_connected":null}"#.utf8))
        XCTAssertNil(older.rememberedName)
        XCTAssertNil(older.passkeyCount)
    }

    func testTaskCancellationAndBusyCeremonyNeverComplete() async throws {
        let daemon = IdentityDaemonFixture()
        let provider = IdentityProviderFixture()
        provider.suspend = true
        let coordinator = NativePasskeyCoordinator(daemon: daemon, provider: provider)
        let task = Task { try await coordinator.perform(.create) }
        while provider.requests.isEmpty { await Task.yield() }
        do { _ = try await coordinator.perform(.login); XCTFail("Concurrent ceremony must refuse") }
        catch { XCTAssertEqual(error as? NativePasskeyFailure, .busy) }
        task.cancel()
        do { _ = try await task.value; XCTFail("Cancelled task must refuse") }
        catch { XCTAssertEqual(error as? NativePasskeyFailure, .cancelled) }
        let events = await daemon.events
        XCTAssertEqual(events, ["begin:create", "cancel:local-handle"])
        XCTAssertFalse(coordinator.isRunning)
    }

    func testExplicitCancellationDuringNativeUIRefusesEvenIfProviderReturnsSuccess() async throws {
        let daemon = IdentityDaemonFixture()
        let provider = IdentityProviderFixture()
        provider.suspend = true
        let coordinator = NativePasskeyCoordinator(daemon: daemon, provider: provider)
        let task = Task { try await coordinator.perform(.create) }
        while provider.requests.isEmpty { await Task.yield() }
        coordinator.cancel()
        do { _ = try await task.value; XCTFail("Explicit cancellation must refuse") }
        catch { XCTAssertEqual(error as? NativePasskeyFailure, .cancelled) }
        let events = await daemon.events
        XCTAssertEqual(events, ["begin:create", "cancel:local-handle"])
    }
}

private actor IdentityDaemonFixture: NativePasskeyDaemonCalling {
    var events: [String] = []
    var modification: String?
    func setModification(_ value: String) { modification = value }
    func begin(_ action: NativePasskeyAction, label: String?) async throws -> NativePasskeyBegin {
        events.append("begin:\(action.rawValue)")
        var json = #"{"ceremony":"local-handle","rp_id":"tracecommons.ai","challenge":"AQID","user_id":"BAUG","user_name":"My Mac","expires_in_secs":180,"exclude_credentials":["BwgJ"],"allowed_credentials":["BwgJ"],"user_verification":"required","authenticator_attachment":"platform","resident_key":"preferred","algorithms":[-7]}"#
        if let modification {
            let key = String(modification.split(separator: ":", maxSplits: 1)[0])
            let data = try JSONSerialization.jsonObject(with: Data(("{" + modification + "}").utf8)) as! [String: Any]
            var object = try JSONSerialization.jsonObject(with: Data(json.utf8)) as! [String: Any]
            object[key.replacingOccurrences(of: "\"", with: "")] = data.values.first!
            json = String(decoding: try JSONSerialization.data(withJSONObject: object), as: UTF8.self)
        }
        return try JSONDecoder().decode(NativePasskeyBegin.self, from: Data(json.utf8))
    }
    func complete(_ action: NativePasskeyAction, ceremony: String, credential: NativePasskeyCredential) async throws -> NativeAccountBinding {
        events.append("complete:\(action.rawValue)")
        return try JSONDecoder().decode(NativeAccountBinding.self, from: Data(#"{"binding_state":"unbound"}"#.utf8))
    }
    func cancel(ceremony: String) async throws { events.append("cancel:\(ceremony)") }
}

@MainActor
private final class IdentityProviderFixture: NativePasskeyProviding {
    var requests: [NativePasskeyRequest] = []
    var failure: NativePasskeyFailure?
    var returnAssertion = false
    var suspend = false
    var suspended: CheckedContinuation<NativePasskeyCredential, Error>?
    func authorize(_ request: NativePasskeyRequest) async throws -> NativePasskeyCredential {
        XCTAssertTrue(Thread.isMainThread)
        requests.append(request)
        if let failure { throw failure }
        if suspend { return try await withCheckedThrowingContinuation { suspended = $0 } }
        if request.action == .login || returnAssertion {
            return try .assertion(credentialID: Data([1]), clientDataJSON: Data([2]),
                authenticatorData: Data([3]), signature: Data([4]), userHandle: Data([5]))
        }
        return try .registration(credentialID: Data([1]), clientDataJSON: Data([2]), attestationObject: Data([3]))
    }
    func cancel() {
        let result = try! NativePasskeyCredential.registration(credentialID: Data([1]),
            clientDataJSON: Data([2]), attestationObject: Data([3]))
        suspended?.resume(returning: result)
        suspended = nil
    }
}
