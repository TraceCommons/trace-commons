import Foundation

/// `near_ai_spend_organizations` as the daemon answers it: the signed-in
/// account's active NEAR AI organizations, which one Private AI spends from,
/// and whether the contributor chose it.
///
/// Most accounts have one organization, and for them this draws nothing:
/// the picker needs two or more, and the Private AI prompt needs the daemon's
/// own `prompt`. A listing that failed, or a payload this build cannot read,
/// is `unreported`, which draws nothing either -- an unknown count is never
/// "several".
///
/// Every word comes from the daemon's `view`, which the shared Rust crate
/// writes; nothing here owns a sentence.
public struct SpendOrganizations: Equatable, Sendable {
    public struct Organization: Equatable, Sendable, Identifiable {
        public let id: String
        public let name: String
        public let current: Bool

        public init(id: String, name: String, current: Bool) {
            self.id = id
            self.name = name
            self.current = current
        }
    }

    /// The daemon's words for this surface (draft wording, awaiting owner
    /// review). All present or the surface draws nothing.
    public struct Words: Equatable, Sendable {
        public let title: String
        public let what: String
        public let picker: String
        public let switching: String
        public let failed: String
        public let sessionExpired: String
        public let refused: String
        public let revokePending: String
        public let promptTitle: String
        public let promptLine: String
        public let promptAction: String
        public let promptActionAccessibility: String
        public let promptDismiss: String
        public let promptDismissAccessibility: String
    }

    public let state: String
    public let organizations: [Organization]
    public let currentOrganizationID: String?
    public let explicit: Bool
    public let prompt: Bool
    public let revokePending: Bool
    public let words: Words?

    public static let stateListed = "listed"

    /// Nothing has been read, or what was read could not be.
    public static let unreported = SpendOrganizations(
        state: "", organizations: [], currentOrganizationID: nil,
        explicit: false, prompt: false, revokePending: false, words: nil)

    public init(
        state: String, organizations: [Organization], currentOrganizationID: String?,
        explicit: Bool, prompt: Bool, revokePending: Bool, words: Words?
    ) {
        self.state = state
        self.organizations = organizations
        self.currentOrganizationID = currentOrganizationID
        self.explicit = explicit
        self.prompt = prompt
        self.revokePending = revokePending
        self.words = words
    }

    /// Whether Settings draws the picker: a complete listing of two or more
    /// organizations, a key spending from one of them, and the words to say
    /// it with. One organization draws nothing new.
    public var showsPicker: Bool {
        state == Self.stateListed && organizations.count >= 2
            && currentOrganizationID != nil && words != nil
    }

    /// Whether the Private AI screen asks the contributor to choose. The
    /// daemon decides (`prompt`: several organizations and no choice ever
    /// recorded for this sign-in); this only refuses to draw it without a
    /// picker to send the contributor to.
    public var showsPrompt: Bool { showsPicker && prompt }

    /// This answer with the prompt ended, for the moment a choice has been
    /// accepted and before the re-read lands: once anything is chosen the
    /// prompt never shows again (owner, on #1590).
    public func withPromptEnded() -> SpendOrganizations {
        SpendOrganizations(
            state: state, organizations: organizations, currentOrganizationID: currentOrganizationID,
            explicit: explicit, prompt: false, revokePending: revokePending, words: words)
    }

    /// The sentence for a refused switch, by the daemon's label. Every
    /// `near_ai_spend_org_*` refusal changed nothing, and the lines say so.
    /// Anything else -- a transport that gave up waiting, say -- is not a
    /// refusal the daemon made, and the switch may have happened, so it gets
    /// no line: the re-read listing shows which organization is current.
    public func failureLine(for label: String) -> String? {
        guard let words, label.hasPrefix("near_ai_spend_org_") else { return nil }
        switch label {
        case "near_ai_spend_org_session_expired", "near_ai_spend_org_no_session":
            return words.sessionExpired
        case "near_ai_spend_org_refused":
            return words.refused
        default:
            return words.failed
        }
    }

    /// From the method's own result object. Anything unreadable degrades to
    /// `unreported`, never to a guess.
    public static func parse(fromJSON json: String) -> SpendOrganizations {
        guard let data = json.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let state = object["state"] as? String
        else { return .unreported }
        let rows = (object["organizations"] as? [[String: Any]]) ?? []
        var organizations: [Organization] = []
        for row in rows {
            guard let id = row["id"] as? String, !id.isEmpty,
                let name = row["name"] as? String, !name.isEmpty
            else { return .unreported }
            organizations.append(Organization(id: id, name: name, current: row["current"] as? Bool ?? false))
        }
        return SpendOrganizations(
            state: state,
            organizations: organizations,
            currentOrganizationID: object["current_organization_id"] as? String,
            explicit: object["explicit"] as? Bool ?? false,
            prompt: object["prompt"] as? Bool ?? false,
            revokePending: object["revoke_pending"] as? Bool ?? false,
            words: (object["view"] as? [String: Any]).flatMap(Self.words))
    }

    private static func words(_ view: [String: Any]) -> Words? {
        func text(_ key: String) -> String? {
            guard let value = view[key] as? String, !value.isEmpty else { return nil }
            return value
        }
        guard let title = text("title"), let what = text("what"), let picker = text("picker"),
            let switching = text("switching"), let failed = text("failed"),
            let sessionExpired = text("session_expired"), let refused = text("refused"),
            let revokePending = text("revoke_pending"), let promptTitle = text("prompt_title"),
            let promptLine = text("prompt_line"), let promptAction = text("prompt_action"),
            let promptActionAccessibility = text("prompt_action_accessibility"),
            let promptDismiss = text("prompt_dismiss"),
            let promptDismissAccessibility = text("prompt_dismiss_accessibility")
        else { return nil }
        return Words(
            title: title, what: what, picker: picker, switching: switching, failed: failed,
            sessionExpired: sessionExpired, refused: refused, revokePending: revokePending,
            promptTitle: promptTitle, promptLine: promptLine, promptAction: promptAction,
            promptActionAccessibility: promptActionAccessibility, promptDismiss: promptDismiss,
            promptDismissAccessibility: promptDismissAccessibility)
    }
}
