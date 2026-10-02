import Foundation

/// Fixed, content-free failures. Never surface Apple's localized error or a credential.
public enum NativePasskeyFailure: String, Error, Sendable {
    case invalidOptions = "passkey-options-invalid"
    case incompleteCredential = "passkey-credential-incomplete"
    case unexpectedCredential = "passkey-credential-unexpected"
    case cancelled = "passkey-cancelled"
    case busy = "passkey-busy"
    case presentationUnavailable = "passkey-presentation-unavailable"
    case authorizationFailed = "passkey-authorization-failed"
}

/// Transport encoding only. Rust alone assembles and validates WebAuthn JSON.
public enum NativePasskeyEncoding {
    public static func encode(_ data: Data) -> String {
        data.base64EncodedString().replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "")
    }

    public static func decode(_ value: String, maximumBytes: Int = 65_536) throws -> Data {
        guard !value.isEmpty, value.utf8.count <= maximumBytes * 4 / 3 + 4,
              value.utf8.allSatisfy({ (65...90).contains($0) || (97...122).contains($0)
                  || (48...57).contains($0) || $0 == 45 || $0 == 95 })
        else { throw NativePasskeyFailure.invalidOptions }
        let padded = value.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
            + String(repeating: "=", count: (4 - value.count % 4) % 4)
        guard let data = Data(base64Encoded: padded), !data.isEmpty,
              data.count <= maximumBytes, encode(data) == value
        else { throw NativePasskeyFailure.invalidOptions }
        return data
    }
}

public struct NativePasskeyCredential: Encodable, Sendable, Equatable {
    public enum Kind: Sendable { case registration, assertion }
    public let kind: Kind
    public let credentialID: String
    public let rawClientDataJSON: String
    public let rawAttestationObject: String?
    public let rawAuthenticatorData: String?
    public let signature: String?
    public let userHandle: String?

    enum CodingKeys: String, CodingKey {
        case credentialID = "credential_id", rawClientDataJSON = "raw_client_data_json"
        case rawAttestationObject = "raw_attestation_object", rawAuthenticatorData = "raw_authenticator_data"
        case signature, userHandle = "user_handle"
    }

    public static func registration(credentialID: Data, clientDataJSON: Data, attestationObject: Data?) throws -> Self {
        guard let attestationObject else { throw NativePasskeyFailure.incompleteCredential }
        try validate([credentialID, clientDataJSON, attestationObject])
        return Self(kind: .registration, credentialID: NativePasskeyEncoding.encode(credentialID),
                    rawClientDataJSON: NativePasskeyEncoding.encode(clientDataJSON),
                    rawAttestationObject: NativePasskeyEncoding.encode(attestationObject),
                    rawAuthenticatorData: nil, signature: nil, userHandle: nil)
    }

    public static func assertion(credentialID: Data, clientDataJSON: Data, authenticatorData: Data,
                                 signature: Data, userHandle: Data?) throws -> Self {
        try validate([credentialID, clientDataJSON, authenticatorData, signature])
        if let userHandle { try validate([userHandle]) }
        return Self(kind: .assertion, credentialID: NativePasskeyEncoding.encode(credentialID),
                    rawClientDataJSON: NativePasskeyEncoding.encode(clientDataJSON), rawAttestationObject: nil,
                    rawAuthenticatorData: NativePasskeyEncoding.encode(authenticatorData),
                    signature: NativePasskeyEncoding.encode(signature), userHandle: userHandle.map(NativePasskeyEncoding.encode))
    }

    private static func validate(_ fields: [Data]) throws {
        guard fields.allSatisfy({ !$0.isEmpty && $0.count <= 65_536 })
        else { throw NativePasskeyFailure.incompleteCredential }
    }
}

public enum NativePasskeyAction: String, Sendable { case create, login, add }

/// C3's token-free platform options; never contains server ceremony IDs or bearer tokens.
public struct NativePasskeyBegin: Decodable, Sendable {
    public let ceremony: String
    public let rpID: String
    public let challenge: String
    public let userID: String?
    public let userName: String?
    public let userDisplayName: String?
    public let expiresInSecs: Int
    public let excludeCredentials: [String]?
    public let allowedCredentials: [String]?
    public let userVerification: String
    public let authenticatorAttachment: String?
    public let residentKey: String?
    public let algorithms: [Int]?
    public let attestation: String?

    enum CodingKeys: String, CodingKey {
        case ceremony, challenge, algorithms, attestation
        case rpID = "rp_id", userID = "user_id", userName = "user_name", userDisplayName = "user_display_name"
        case expiresInSecs = "expires_in_secs", excludeCredentials = "exclude_credentials"
        case allowedCredentials = "allowed_credentials", userVerification = "user_verification"
        case authenticatorAttachment = "authenticator_attachment", residentKey = "resident_key"
    }

    public func validated(for action: NativePasskeyAction) throws -> NativePasskeyRequest {
        guard !ceremony.isEmpty, ceremony.utf8.count <= 256,
              rpID == "tracecommons.ai", (1...600).contains(expiresInSecs),
              ["required", "preferred", "discouraged"].contains(userVerification)
        else { throw NativePasskeyFailure.invalidOptions }
        let challengeBytes = try NativePasskeyEncoding.decode(challenge, maximumBytes: 1_024)
        func credentials(_ values: [String]?) throws -> [Data] {
            guard (values?.count ?? 0) <= 100 else { throw NativePasskeyFailure.invalidOptions }
            return try (values ?? []).map { try NativePasskeyEncoding.decode($0, maximumBytes: 1_024) }
        }
        var userBytes: Data?
        if action != .login {
            guard let userID, let userName, !userName.isEmpty, userName.utf8.count <= 256,
                  authenticatorAttachment == "platform", let residentKey,
                  ["required", "preferred", "discouraged"].contains(residentKey),
                  let algorithms, !algorithms.isEmpty, algorithms.count <= 16, algorithms.contains(-7),
                  ["none", "indirect", "direct"].contains(attestation ?? "none"),
                  (userDisplayName?.utf8.count ?? 0) <= 256
            else { throw NativePasskeyFailure.invalidOptions }
            userBytes = try NativePasskeyEncoding.decode(userID, maximumBytes: 64)
        }
        return NativePasskeyRequest(action: action, rpID: rpID, challenge: challengeBytes,
            userID: userBytes, userName: userName, userDisplayName: userDisplayName,
            userVerification: userVerification, attestation: attestation ?? "none",
            excludedCredentials: try credentials(excludeCredentials),
            allowedCredentials: try credentials(allowedCredentials), expiresInSecs: expiresInSecs)
    }
}

/// Decoded, validated request consumed by a platform-only provider on the main actor.
public struct NativePasskeyRequest: Sendable {
    public let action: NativePasskeyAction
    public let rpID: String
    public let challenge: Data
    public let userID: Data?
    public let userName: String?
    public let userDisplayName: String?
    public let userVerification: String
    public let attestation: String
    public let excludedCredentials: [Data]
    public let allowedCredentials: [Data]
    public let expiresInSecs: Int
}

public struct NativeAccountBinding: Decodable, Sendable, Equatable {
    public let bindingState: String
    enum CodingKeys: String, CodingKey { case bindingState = "binding_state" }
}

public struct NativeAccountBindResult: Decodable, Sendable, Equatable {
    public let outcome: String
    public let bindingState: String
    enum CodingKeys: String, CodingKey { case outcome, bindingState = "binding_state" }
}

/// Compatible with C1's optional identity fields. Unknown stays unknown.
public struct NativeAccountSession: Decodable, Sendable, Equatable {
    public let state: String?
    public let signedIn: Bool?
    public let accountID: String?
    public let expiresAt: Date?
    enum CodingKeys: String, CodingKey {
        case state, signedIn = "signed_in", accountID = "account_id", expiresAt = "expires_at"
    }
}

public struct NativePasskeyState: Decodable, Sendable, Equatable {
    public let state: String
    public let passkeyCount: Int?
    public let nearAiConnected: Bool?
    enum CodingKeys: String, CodingKey {
        case state, passkeyCount = "passkey_count", nearAiConnected = "near_ai_connected"
    }
}

public protocol NativePasskeyDaemonCalling: Sendable {
    func begin(_ action: NativePasskeyAction, label: String?, ingestURL: String?) async throws -> NativePasskeyBegin
    func complete(_ action: NativePasskeyAction, ceremony: String, credential: NativePasskeyCredential) async throws -> NativeAccountBinding
    func cancel(ceremony: String) async throws
}

@MainActor
public protocol NativePasskeyProviding: AnyObject, Sendable {
    func authorize(_ request: NativePasskeyRequest) async throws -> NativePasskeyCredential
    func cancel()
}

/// Owns one begin/present/complete sequence. Swift owns no account authority.
@MainActor
public final class NativePasskeyCoordinator {
    private let daemon: any NativePasskeyDaemonCalling
    private let provider: any NativePasskeyProviding
    public private(set) var isRunning = false
    private var explicitlyCancelled = false
    private var runID: UUID?

    public init(daemon: any NativePasskeyDaemonCalling, provider: any NativePasskeyProviding) {
        self.daemon = daemon
        self.provider = provider
    }

    public func cancel() {
        guard isRunning else { return }
        explicitlyCancelled = true
        provider.cancel()
    }

    public func perform(_ action: NativePasskeyAction, label: String? = nil,
                        ingestURL: String? = nil) async throws -> NativeAccountBinding {
        guard !isRunning else { throw NativePasskeyFailure.busy }
        guard (label?.count ?? 0) <= 64, action != .add || ingestURL == nil
        else { throw NativePasskeyFailure.invalidOptions }
        isRunning = true
        explicitlyCancelled = false
        let thisRun = UUID()
        runID = thisRun
        defer { isRunning = false; runID = nil }
        let begin = try await daemon.begin(action, label: label, ingestURL: ingestURL)
        do {
            try checkCancellation()
            let request = try begin.validated(for: action)
            let deadline = ContinuousClock.now.advanced(by: .seconds(request.expiresInSecs))
            let presentedProvider = provider
            let credential = try await withTaskCancellationHandler {
                try await presentedProvider.authorize(request)
            } onCancel: {
                Task { @MainActor [weak self] in
                    // A queued cancellation from an earlier task must not
                    // dismiss a newly started ceremony on the same provider.
                    guard self?.runID == thisRun else { return }
                    presentedProvider.cancel()
                }
            }
            try checkCancellation()
            guard ContinuousClock.now < deadline else { throw NativePasskeyFailure.invalidOptions }
            guard credential.kind == (action == .login ? .assertion : .registration)
            else { throw NativePasskeyFailure.unexpectedCredential }
            return try await daemon.complete(action, ceremony: begin.ceremony, credential: credential)
        } catch {
            // A cleanup error must not replace the ceremony's original failure.
            try? await daemon.cancel(ceremony: begin.ceremony)
            if Task.isCancelled || explicitlyCancelled { throw NativePasskeyFailure.cancelled }
            throw error
        }
    }

    private func checkCancellation() throws {
        guard !Task.isCancelled, !explicitlyCancelled else { throw NativePasskeyFailure.cancelled }
    }
}
