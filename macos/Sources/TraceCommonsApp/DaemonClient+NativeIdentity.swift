import Foundation
import TCShellCore

extension DaemonClient {
    func accountSessionStatus() throws -> NativeAccountSession { try identityCall("account_session_status") }

    /// The enrolled browser/PKCE path. Rust owns URL validation, opening and tokens.
    func accountSignIn() throws -> NativeAccountSession {
        try identityCall("account_sign_in")
    }

    /// Answers what happened to the NEAR AI sign-in that goes with it
    /// (`CredentialSurface.signOutNearAi`).
    @discardableResult
    func accountSignOut() throws -> CredentialSurface.SignOutNearAi {
        struct Reply: Decodable { let signedOut: Bool; let nearAiCleared: Bool?; let nearAiRevoked: Bool?
            enum CodingKeys: String, CodingKey {
                case signedOut = "signed_out", nearAiCleared = "near_ai_cleared"
                case nearAiRevoked = "near_ai_revoked"
            }
        }
        let reply: Reply = try identityCall("account_sign_out")
        guard reply.signedOut else { throw Failure(code: "refused", message: "account-sign-out-failed") }
        return CredentialSurface.signOutNearAi(
            nearAiCleared: reply.nearAiCleared, nearAiRevoked: reply.nearAiRevoked)
    }

    /// `unenroll`: the daemon drops this Mac's enrollment, locally only, and
    /// the NEAR AI sign-in with it. Checked against `hello` first, so a daemon
    /// without the call refuses (`native-identity-unsupported`) and the
    /// caller stays as it was. Answers what happened to the NEAR AI sign-in,
    /// as `accountSignOut` does.
    func unenroll() throws -> CredentialSurface.SignOutNearAi {
        struct Reply: Decodable { let unenrolled: Bool; let nearAiCleared: Bool?; let nearAiRevoked: Bool?
            enum CodingKeys: String, CodingKey {
                case unenrolled, nearAiCleared = "near_ai_cleared"
                case nearAiRevoked = "near_ai_revoked"
            }
        }
        let reply: Reply = try identityCall("unenroll")
        guard reply.unenrolled else { throw Failure(code: "refused", message: "unenroll-failed") }
        return CredentialSurface.signOutNearAi(
            nearAiCleared: reply.nearAiCleared, nearAiRevoked: reply.nearAiRevoked)
    }

    private func identityCall<T: Decodable>(_ method: String, params: [String: Any] = [:]) throws -> T {
        try requireIdentityMethods([method])
        return try call(method, params: params, as: T.self)
    }

    // Calls may reconnect to a restarted/replaced daemon. Re-check capabilities
    // until the transport exposes a connection generation for safe caching.
    private func requireIdentityMethods(_ required: [String]) throws {
        let hello = try call("hello", as: DaemonHello.self)
        guard required.allSatisfy(hello.serves) else {
            throw Failure(code: "unknown_method", message: "native-identity-unsupported")
        }
    }
}
