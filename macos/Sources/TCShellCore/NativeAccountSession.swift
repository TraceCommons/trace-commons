import Foundation

/// Compatible with C1's optional identity fields. Unknown stays unknown.
public struct NativeAccountSession: Decodable, Sendable, Equatable {
    public let state: String?
    public let signedIn: Bool?
    public let expiresAt: Date?
    /// `renewing` while the daemon renews an expired session in the
    /// background, `failed` after a refused renewal; nil otherwise and from
    /// a daemon that predates it.
    public let renewal: String?
    /// The refused renewal's class, e.g. `near_ai_sign_in_required`.
    public let renewalFailure: String?
    /// `near_ai` when Sign in again is `near_ai_account_sign_in` (a NEAR AI
    /// account); nil otherwise, when Sign in again is `account_sign_in`.
    public let signIn: String?
    enum CodingKeys: String, CodingKey {
        case state, signedIn = "signed_in", expiresAt = "expires_at"
        case renewal, renewalFailure = "renewal_failure", signIn = "sign_in"
    }

    public init(state: String?, signedIn: Bool?, expiresAt: Date?,
                renewal: String? = nil, renewalFailure: String? = nil, signIn: String? = nil) {
        self.state = state
        self.signedIn = signedIn
        self.expiresAt = expiresAt
        self.renewal = renewal
        self.renewalFailure = renewalFailure
        self.signIn = signIn
    }

    /// Whether Sign in again runs the NEAR AI path (`near_ai_account_sign_in`).
    public var signsInWithNearAI: Bool { signIn == "near_ai" }
}
