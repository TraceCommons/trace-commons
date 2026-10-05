import AppKit
import AuthenticationServices
import Foundation
import TCShellCore

/// Only platform passkeys. Does not instantiate a security-key provider, hold
/// bearer tokens, assemble WebAuthn JSON or choose a tenant/account identity.
@MainActor
final class PlatformPasskeyProvider: NSObject, NativePasskeyProviding,
    ASAuthorizationControllerDelegate, ASAuthorizationControllerPresentationContextProviding {
    private let anchorProvider: @MainActor () -> NSWindow?
    private var presentationWindow: NSWindow?
    private var controller: ASAuthorizationController?
    private var continuation: CheckedContinuation<NativePasskeyCredential, Error>?
    private var expiryTask: Task<Void, Never>?

    init(presentationAnchor: @escaping @MainActor () -> NSWindow?) {
        anchorProvider = presentationAnchor
        super.init()
    }

    func authorize(_ request: NativePasskeyRequest) async throws -> NativePasskeyCredential {
        guard continuation == nil else { throw NativePasskeyFailure.busy }
        guard !Task.isCancelled else { throw NativePasskeyFailure.cancelled }
        guard let window = anchorProvider() else { throw NativePasskeyFailure.presentationUnavailable }
        let authorizationRequest = try Self.authorizationRequest(for: request)
        presentationWindow = window
        return try await withCheckedThrowingContinuation { continuation in
            self.continuation = continuation
            let controller = ASAuthorizationController(authorizationRequests: [authorizationRequest])
            self.controller = controller
            controller.delegate = self
            controller.presentationContextProvider = self
            expiryTask = Task { @MainActor [weak self] in
                do { try await Task.sleep(for: .seconds(request.expiresInSecs)) }
                catch { return }
                self?.cancel()
            }
            controller.performRequests()
        }
    }

    /// Pure request construction is exercised without showing an Apple prompt.
    static func authorizationRequest(for request: NativePasskeyRequest) throws -> ASAuthorizationRequest {
        let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(relyingPartyIdentifier: request.rpID)
        let verification: ASAuthorizationPublicKeyCredentialUserVerificationPreference
        switch request.userVerification {
        case "required": verification = .required
        case "preferred": verification = .preferred
        case "discouraged": verification = .discouraged
        default: throw NativePasskeyFailure.invalidOptions
        }
        if request.action == .login {
            let assertion = provider.createCredentialAssertionRequest(challenge: request.challenge)
            assertion.userVerificationPreference = verification
            assertion.allowedCredentials = request.allowedCredentials.map {
                ASAuthorizationPlatformPublicKeyCredentialDescriptor(credentialID: $0)
            }
            return assertion
        }
        guard let userID = request.userID, let name = request.userName
        else { throw NativePasskeyFailure.invalidOptions }
        let registration = provider.createCredentialRegistrationRequest(challenge: request.challenge, name: name, userID: userID)
        registration.displayName = request.userDisplayName
        registration.userVerificationPreference = verification
        registration.excludedCredentials = request.excludedCredentials.map {
            ASAuthorizationPlatformPublicKeyCredentialDescriptor(credentialID: $0)
        }
        switch request.attestation {
        case "none": registration.attestationPreference = .none
        case "indirect": registration.attestationPreference = .indirect
        case "direct": registration.attestationPreference = .direct
        default: throw NativePasskeyFailure.invalidOptions
        }
        return registration
    }

    func cancel() {
        let activeController = controller
        finish(.failure(NativePasskeyFailure.cancelled))
        activeController?.cancel()
    }

    func presentationAnchor(for controller: ASAuthorizationController) -> ASPresentationAnchor {
        // Set before performRequests and retained through callbacks. A cancelled
        // controller has its presentation delegate removed before its UI is cancelled.
        presentationWindow!
    }

    func authorizationController(controller: ASAuthorizationController, didCompleteWithAuthorization authorization: ASAuthorization) {
        guard controller === self.controller else { return }
        do {
            if let registration = authorization.credential as? ASAuthorizationPlatformPublicKeyCredentialRegistration {
                finish(.success(try .registration(credentialID: registration.credentialID,
                    clientDataJSON: registration.rawClientDataJSON, attestationObject: registration.rawAttestationObject)))
            } else if let assertion = authorization.credential as? ASAuthorizationPlatformPublicKeyCredentialAssertion {
                finish(.success(try .assertion(credentialID: assertion.credentialID,
                    clientDataJSON: assertion.rawClientDataJSON, authenticatorData: assertion.rawAuthenticatorData,
                    signature: assertion.signature, userHandle: assertion.userID.isEmpty ? nil : assertion.userID)))
            } else {
                finish(.failure(NativePasskeyFailure.unexpectedCredential))
            }
        } catch { finish(.failure(error)) }
    }

    func authorizationController(controller: ASAuthorizationController, didCompleteWithError error: Error) {
        guard controller === self.controller else { return }
        let cancelled = (error as? ASAuthorizationError)?.code == .canceled
        finish(.failure(cancelled ? NativePasskeyFailure.cancelled : NativePasskeyFailure.authorizationFailed))
    }

    private func finish(_ result: Result<NativePasskeyCredential, Error>) {
        guard let continuation else { return }
        self.continuation = nil
        expiryTask?.cancel()
        expiryTask = nil
        controller?.delegate = nil
        controller?.presentationContextProvider = nil
        controller = nil
        // Retain the anchor until this provider's lifetime ends so an already
        // scheduled framework callback can never observe a missing window.
        continuation.resume(with: result)
    }
}
