import AuthenticationServices
import Foundation
import TCShellCore
import XCTest
@testable import TraceCommonsApp

@MainActor
final class PlatformPasskeyProviderTests: XCTestCase {
    func testRegistrationUsesPlatformProviderAndPreservesServerPreferences() throws {
        let begin = try options()
        let request = try XCTUnwrap(PlatformPasskeyProvider.authorizationRequest(for: begin.validated(for: .add))
            as? ASAuthorizationPlatformPublicKeyCredentialRegistrationRequest)
        XCTAssertEqual(request.relyingPartyIdentifier, "tracecommons.ai")
        XCTAssertEqual(request.challenge, Data([1, 2, 3]))
        XCTAssertEqual(request.userID, Data([4, 5, 6]))
        XCTAssertEqual(request.excludedCredentials?.map(\.credentialID), [Data([7, 8, 9])])
        XCTAssertEqual(request.userVerificationPreference, .required)
        XCTAssertEqual(request.attestationPreference, .direct)
        XCTAssertEqual(request.displayName, "My Mac")
    }

    func testAssertionCarriesAllowedCredentialsAndUserVerification() throws {
        let request = try XCTUnwrap(PlatformPasskeyProvider.authorizationRequest(for: options().validated(for: .login))
            as? ASAuthorizationPlatformPublicKeyCredentialAssertionRequest)
        XCTAssertEqual(request.allowedCredentials.map(\.credentialID), [Data([7, 8, 9])])
        XCTAssertEqual(request.userVerificationPreference, .required)
    }

    func testMissingPresentationAnchorRefusesWithoutApplePrompt() async throws {
        let provider = PlatformPasskeyProvider(presentationAnchor: { nil })
        do { _ = try await provider.authorize(options().validated(for: .create)); XCTFail("A window is required") }
        catch { XCTAssertEqual(error as? NativePasskeyFailure, .presentationUnavailable) }
    }

    private func options() throws -> NativePasskeyBegin {
        try JSONDecoder().decode(NativePasskeyBegin.self, from: Data(
            #"{"ceremony":"local-handle","rp_id":"tracecommons.ai","challenge":"AQID","user_id":"BAUG","user_name":"My Mac","user_display_name":"My Mac","expires_in_secs":180,"exclude_credentials":["BwgJ"],"allowed_credentials":["BwgJ"],"user_verification":"required","authenticator_attachment":"platform","resident_key":"required","algorithms":[-7],"attestation":"direct"}"#.utf8))
    }
}
