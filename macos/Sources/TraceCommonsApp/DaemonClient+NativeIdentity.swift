import Foundation
import TCShellCore

extension DaemonClient {
    func accountSessionStatus() throws -> NativeAccountSession { try identityCall("account_session_status") }

    /// The enrolled browser/PKCE path. Rust owns URL validation, opening and tokens.
    func accountSignIn() throws -> NativeAccountSession {
        try identityCall("account_sign_in")
    }

    /// Answers whether the NEAR AI key was left at NEAR AI: stored, cleared
    /// here, and not revoked there (`CredentialSurface.signOutLeftKeyAtService`).
    @discardableResult
    func accountSignOut() throws -> Bool {
        struct Reply: Decodable { let signedOut: Bool; let nearAiRevoked: Bool?
            enum CodingKeys: String, CodingKey {
                case signedOut = "signed_out", nearAiRevoked = "near_ai_revoked"
            }
        }
        let reply: Reply = try identityCall("account_sign_out")
        guard reply.signedOut else { throw Failure(code: "refused", message: "account-sign-out-failed") }
        return CredentialSurface.signOutLeftKeyAtService(nearAiRevoked: reply.nearAiRevoked)
    }

    /// `unenroll`: the daemon drops this Mac's enrollment, locally only, and
    /// the NEAR AI sign-in with it. Checked against `hello` first, so a daemon
    /// without the call refuses (`native-identity-unsupported`) and the
    /// caller stays as it was. Answers whether the NEAR AI key was left at
    /// NEAR AI, as `accountSignOut` does.
    func unenroll() throws -> Bool {
        struct Reply: Decodable { let unenrolled: Bool; let nearAiRevoked: Bool?
            enum CodingKeys: String, CodingKey {
                case unenrolled, nearAiRevoked = "near_ai_revoked"
            }
        }
        let reply: Reply = try identityCall("unenroll")
        guard reply.unenrolled else { throw Failure(code: "refused", message: "unenroll-failed") }
        return CredentialSurface.signOutLeftKeyAtService(nearAiRevoked: reply.nearAiRevoked)
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
