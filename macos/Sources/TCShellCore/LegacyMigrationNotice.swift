import Foundation

/// `status.legacy_invite_migration`: whether moving a legacy invite identity
/// to a NEAR AI account is offered, and the notice after it moved.
///
/// `offered` is the daemon's own answer (`legacy_migration::status_value`):
/// a legacy `tenant-` identity enrolled by an invite, not yet moved. This
/// shell does not work it out from the tenant id. The daemon cannot tell a
/// pooled invite from a personal one before the commons answers, so a
/// pooled device is offered the move too and is refused with its own line.
///
/// The notice is read only to hand it back to the Rust
/// (`TCConsentCopy.legacyMigrationNoticeJSON`), which reads `folders_kept`
/// and returns the words. This shell never reads it to choose a sentence.
///
/// A daemon that predates the field, an `offered` that is not `true`, or a
/// notice that is not an object, reads as nothing to offer or show.
public struct LegacyMigrationWire: Decodable, Equatable, Sendable {
    /// Whether the daemon offers the move.
    public let offered: Bool
    /// The notice object as the daemon sent it, re-encoded, or nil when there
    /// is none.
    public let noticeJSON: String?

    public static let none = LegacyMigrationWire(offered: false, noticeJSON: nil)

    init(offered: Bool = false, noticeJSON: String?) {
        self.offered = offered
        self.noticeJSON = noticeJSON
    }

    public init(from decoder: Decoder) throws {
        let value = try JSONValue(from: decoder)
        guard case .object(let fields) = value else {
            offered = false
            noticeJSON = nil
            return
        }
        if case .bool(true)? = fields["offered"] {
            offered = true
        } else {
            offered = false
        }
        guard case .object? = fields["notice"], let notice = fields["notice"] else {
            noticeJSON = nil
            return
        }
        let data = try JSONEncoder().encode(notice)
        noticeJSON = String(decoding: data, as: UTF8.self)
    }
}

/// The offer to move a legacy invite identity to a NEAR AI account, in the
/// Rust's words (`consent_copy::legacy_migration_offer`, across
/// `TCCoreCopy.legacyMigrationOfferJSON`). Drawn in Settings and, until it
/// is dismissed, as a card in the notices. No property is written in Swift.
public struct LegacyMigrationOffer: Decodable, Equatable, Sendable {
    public let title: String
    public let body: String
    /// The button that starts the move (`legacy_invite_migrate`).
    public let action: String
    /// Shown in place of the button while the move runs.
    public let working: String
    /// The invite field's label, shown only after the daemon refused with
    /// `legacy_migration_invite_needed`.
    public let invitePrompt: String
    /// The move could not be started at all: nothing ran.
    public let startFailed: String
    /// The card's dismiss. Puts the card away for good on this Mac.
    public let dismiss: String
    /// Beside the dismiss: the move stays in Settings.
    public let later: String
    /// Shown in place of the button while the NEAR AI login step runs in
    /// the browser.
    public let nearAILoginWaiting: String
    /// The NEAR AI login step ended without finishing: nothing was moved.
    public let nearAILoginFailed: String
    /// Stops waiting on the NEAR AI login step.
    public let nearAILoginCancel: String
    /// The accessible name of `action` on the card in the notices.
    public let actionNameNotice: String
    /// The accessible name of `action` in Settings.
    public let actionNameSettings: String

    /// Where a Move button is drawn. The two look the same and are named
    /// apart for a screen reader (`docs/ui-copy-guide.md` 5).
    public enum Placement: Sendable {
        case notice, settings
    }

    /// The core's accessible name for the Move button at `placement`.
    public func actionName(for placement: Placement) -> String {
        switch placement {
        case .notice: actionNameNotice
        case .settings: actionNameSettings
        }
    }

    enum CodingKeys: String, CodingKey {
        case title, body, action, working, dismiss, later
        case invitePrompt = "invite_prompt"
        case startFailed = "start_failed"
        // The NEAR AI login step's keys keep their old `passkey_` wire names.
        case nearAILoginWaiting = "passkey_waiting"
        case nearAILoginFailed = "passkey_failed"
        case nearAILoginCancel = "passkey_cancel"
        case actionNameNotice = "action_name_notice"
        case actionNameSettings = "action_name_settings"
    }

    /// The payload fields this shell decodes, by wire name. Compared against
    /// the live export by `TCBridgeTests`.
    public static let consumedFields = [
        "title", "body", "action", "working", "invite_prompt", "start_failed", "dismiss", "later",
        "passkey_waiting", "passkey_failed", "passkey_cancel", "action_name_notice",
        "action_name_settings",
    ]

    /// The daemon's refusal label that asks for the invite, so the shell
    /// shows the invite field and sends what is pasted with the next move.
    public static let inviteNeededLabel = "legacy_migration_invite_needed"

    /// The daemon's refusal label for a move that needs the NEAR AI login
    /// step first (no session to provision the account with). The shell runs the
    /// step itself and then retries the move.
    public static let nearAILoginNeededLabel = "legacy_migration_no_near_ai_session"

    /// Decode the payload, or nil if it will not parse or a field is empty:
    /// shown whole or not at all.
    public static func decode(fromJSON json: String) -> LegacyMigrationOffer? {
        guard let data = json.data(using: .utf8),
            let offer = try? JSONDecoder().decode(LegacyMigrationOffer.self, from: data)
        else {
            return nil
        }
        let words = [
            offer.title, offer.body, offer.action, offer.working, offer.invitePrompt,
            offer.startFailed, offer.dismiss, offer.later, offer.nearAILoginWaiting,
            offer.nearAILoginFailed, offer.nearAILoginCancel, offer.actionNameNotice, offer.actionNameSettings,
        ]
        return words.contains(where: \.isEmpty) ? nil : offer
    }
}

/// The notice the Rust assembled: that the contributor's contributions now go
/// under their NEAR AI account, and whether their automatic folders were
/// kept. Every property is filled from the payload and none is written in
/// Swift.
public struct LegacyMigrationNotice: Decodable, Equatable, Sendable {
    public let title: String
    public let body: String
    /// Whether automatic folders were kept, already chosen by the Rust.
    public let folders: String
    /// The button. It records that the notice was shown
    /// (`acknowledge_legacy_invite_migration`) and does nothing else.
    public let acknowledge: String

    /// The payload fields this shell decodes, by wire name. Compared against
    /// the live export by `TCBridgeTests`.
    public static let consumedFields = ["title", "body", "folders", "acknowledge"]

    /// Decode the payload, or nil if it will not parse or a field is empty:
    /// shown whole or not at all.
    public static func decode(fromJSON json: String) -> LegacyMigrationNotice? {
        guard let data = json.data(using: .utf8),
            let notice = try? JSONDecoder().decode(LegacyMigrationNotice.self, from: data)
        else {
            return nil
        }
        let sentences = [notice.title, notice.body, notice.folders, notice.acknowledge]
        return sentences.contains(where: \.isEmpty) ? nil : notice
    }
}
