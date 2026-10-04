import AppKit
import Foundation
import TCShellCore

extension DaemonClient {
    func passkeyBegin(_ action: NativePasskeyAction, label: String? = nil) throws -> NativePasskeyBegin {
        guard (label?.count ?? 0) <= 64
        else { throw NativePasskeyFailure.invalidOptions }
        let prefix = "passkey_\(action.rawValue)"
        try requireIdentityMethods(["\(prefix)_begin", "\(prefix)_complete", "passkey_cancel"])
        var params: [String: Any] = [:]
        if let label, action != .login { params["label"] = label }
        return try call("\(prefix)_begin", params: params, as: NativePasskeyBegin.self)
    }

    func passkeyComplete(_ action: NativePasskeyAction, ceremony: String,
                         credential: NativePasskeyCredential) throws -> NativeAccountBinding {
        guard credential.kind == (action == .login ? .assertion : .registration)
        else { throw NativePasskeyFailure.unexpectedCredential }
        let method = "passkey_\(action.rawValue)_complete"
        try requireIdentityMethods([method])
        guard var params = try JSONSerialization.jsonObject(with: JSONEncoder().encode(credential)) as? [String: Any]
        else { throw NativePasskeyFailure.incompleteCredential }
        params["ceremony"] = ceremony
        return try call(method, params: params, as: NativeAccountBinding.self)
    }

    func passkeyCancel(ceremony: String) throws {
        struct Reply: Decodable { let cancelled: Bool }
        let reply: Reply = try identityCall("passkey_cancel", params: ["ceremony": ceremony])
        guard reply.cancelled else { throw NativePasskeyFailure.authorizationFailed }
    }

    func accountBind() throws -> NativeAccountBindResult { try identityCall("account_bind") }
    func accountBinding() throws -> NativeAccountBinding { try identityCall("account_binding") }
    func passkeyState() throws -> NativePasskeyState { try identityCall("passkey_state") }
    func accountSessionStatus() throws -> NativeAccountSession { try identityCall("account_session_status") }

    /// The enrolled browser/PKCE path. Rust owns URL validation, opening and tokens.
    func accountSignIn() throws -> NativeAccountSession {
        try identityCall("account_sign_in")
    }

    func accountSignOut() throws {
        struct Reply: Decodable { let signedOut: Bool
            enum CodingKeys: String, CodingKey { case signedOut = "signed_out" }
        }
        let reply: Reply = try identityCall("account_sign_out")
        guard reply.signedOut else { throw NativePasskeyFailure.authorizationFailed }
    }

    @MainActor
    func nativePasskeyCoordinator(presentationAnchor: @escaping @MainActor () -> NSWindow?) -> NativePasskeyCoordinator {
        NativePasskeyCoordinator(daemon: NativeIdentityTransport(client: self),
                                provider: PlatformPasskeyProvider(presentationAnchor: presentationAnchor))
    }

    private func identityCall<T: Decodable>(_ method: String, params: [String: Any] = [:]) throws -> T {
        try requireIdentityMethods([method])
        return try call(method, params: params, as: T.self)
    }

    // Calls may reconnect to a restarted/replaced daemon. Re-check capabilities
    // until the transport exposes a connection generation for safe caching.
    private func requireIdentityMethods(_ required: [String]) throws {
        struct Hello: Decodable { let methods: [String] }
        let hello = try call("hello", as: Hello.self)
        guard required.allSatisfy(hello.methods.contains) else {
            throw Failure(code: "unknown_method", message: "native-identity-unsupported")
        }
    }
}

/// The underlying live TCDaemon gates concurrent calls and teardown. This
/// immutable adapter carries it to blocking worker tasks; UI work stays on MainActor.
final class NativeIdentityTransport: NativePasskeyDaemonCalling, @unchecked Sendable {
    private let client: DaemonClient
    init(client: DaemonClient) { self.client = client }
    func begin(_ action: NativePasskeyAction, label: String?) async throws -> NativePasskeyBegin {
        try await Task.detached { [self] in try client.passkeyBegin(action, label: label) }.value
    }
    func complete(_ action: NativePasskeyAction, ceremony: String, credential: NativePasskeyCredential) async throws -> NativeAccountBinding {
        try await Task.detached { [self] in try client.passkeyComplete(action, ceremony: ceremony, credential: credential) }.value
    }
    func cancel(ceremony: String) async throws {
        try await Task.detached { [self] in try client.passkeyCancel(ceremony: ceremony) }.value
    }
    func connectNearAI() async throws -> NativeAccountBindResult {
        try await Task.detached { [self] in try client.accountBind() }.value
    }
    func binding() async throws -> NativeAccountBinding {
        try await Task.detached { [self] in try client.accountBinding() }.value
    }
    func status() async throws -> NativeAccountSession {
        try await Task.detached { [self] in try client.accountSessionStatus() }.value
    }
    func passkeys() async throws -> NativePasskeyState {
        try await Task.detached { [self] in try client.passkeyState() }.value
    }
    func signInWithBrowser() async throws -> NativeAccountSession {
        try await Task.detached { [self] in try client.accountSignIn() }.value
    }
    func signOut() async throws {
        try await Task.detached { [self] in try client.accountSignOut() }.value
    }
}
