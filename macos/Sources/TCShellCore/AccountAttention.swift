import Foundation

/// `status.account_state`, `account_state_reason` and
/// `near_ai_login_unconfirmed` (#1836 F1): whether this app's account is
/// locally broken in a way signing in again does not fix.
///
/// Read only to hand back to the Rust (`TCConsentCopy.accountAttentionJSON`),
/// which chooses the words. This shell never reads the reason to choose a
/// sentence. A daemon that predates the keys reads as nothing to show.
public struct AccountAttentionWire: Equatable, Sendable {
    public let state: String?
    public let reason: String?
    public let nearAILoginUnconfirmed: Bool

    public static let none = AccountAttentionWire(state: nil, reason: nil, nearAILoginUnconfirmed: false)

    public init(state: String?, reason: String?, nearAILoginUnconfirmed: Bool) {
        self.state = state
        self.reason = reason
        self.nearAILoginUnconfirmed = nearAILoginUnconfirmed
    }

    /// The three keys as the ABI takes them, or nil when the daemon did not
    /// say (an older daemon).
    public var wireJSON: String? {
        guard let state else { return nil }
        var object: [String: Any] = [
            "account_state": state,
            "near_ai_login_unconfirmed": nearAILoginUnconfirmed,
        ]
        object["account_state_reason"] = reason ?? NSNull()
        guard let data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]) else {
            return nil
        }
        return String(decoding: data, as: UTF8.self)
    }
}

/// What the account section shows while the account needs attention, in
/// the Rust's words (`consent_copy::account_attention_for_wire`). A status
/// in the section, never a banner or notification.
public struct AccountAttention: Decodable, Equatable, Sendable {
    public let title: String
    /// The reason's sentence; nil for a reason this build does not know.
    public let body: String?
    /// Only while a NEAR AI login is stored.
    public let privateAI: String?
    /// "Set up again": opens the confirmation below; nothing runs yet.
    public let action: String
    /// The confirmation. Only `confirmAction` unenrolls, then the first run
    /// opens.
    public let confirmTitle: String
    public let confirmBody: String
    public let confirmAction: String
    public let confirmCancel: String
    /// Shown under the button when the daemon refused to set up again.
    public let failed: String

    enum CodingKeys: String, CodingKey {
        case title
        case body
        case privateAI = "private_ai"
        case action
        case confirmTitle = "confirm_title"
        case confirmBody = "confirm_body"
        case confirmAction = "confirm_action"
        case confirmCancel = "confirm_cancel"
        case failed
    }

    /// The payload fields this shell decodes, by wire name. Compared against
    /// the live export by `TCBridgeTests`.
    public static let consumedFields = [
        "title", "body", "private_ai", "action", "confirm_title", "confirm_body", "confirm_action",
        "confirm_cancel", "failed",
    ]

    /// Decode the payload, or nil if it will not parse or any word other than
    /// the two optional lines is empty: shown whole or not at all.
    public static func decode(fromJSON json: String) -> AccountAttention? {
        guard let data = json.data(using: .utf8),
            let copy = try? JSONDecoder().decode(AccountAttention.self, from: data)
        else {
            return nil
        }
        let words = [
            copy.title, copy.action, copy.confirmTitle, copy.confirmBody, copy.confirmAction,
            copy.confirmCancel, copy.failed,
        ]
        return words.contains(where: \.isEmpty) ? nil : copy
    }
}
