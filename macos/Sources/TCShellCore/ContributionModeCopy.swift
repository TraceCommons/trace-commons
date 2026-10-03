import Foundation

/// The menu-bar Contribution mode pill's words (#1173, #1208), decoded from
/// `tc_contribution_mode_copy_json` (`project_copy::contribution_mode_copy`).
/// Decoding is here so it is testable without the dylib; `TCBridgeTests`
/// checks it against the real export.
public struct ContributionModeCopy: Decodable, Equatable, Sendable {
    public struct Choice: Decodable, Equatable, Sendable {
        /// The `mode` `set_contribution_override` takes: `notify_only`,
        /// `auto_upload` or `ignore`.
        public let mode: String
        public let label: String
        public let line: String
    }

    public let title: String
    public let mixed: String
    /// Ask me, Auto contribute, Never, in that order.
    public let choices: [Choice]
    /// Said while a contribution override is in force.
    public let overrideActive: String
    public let clear: String
    /// Under the Auto contribute label exactly when
    /// `status.contribution_mode_partial` is true.
    public let autoPartial: String

    enum CodingKeys: String, CodingKey {
        case title, mixed, choices, clear
        case overrideActive = "override_active"
        case autoPartial = "auto_partial"
    }

    /// The payload fields this shell decodes, by wire name.
    public static let consumedFields = ["auto_partial", "choices", "clear", "mixed", "override_active", "title"]

    /// Decode the payload, or nil if it will not parse or a word is empty.
    public static func decode(fromJSON json: String?) -> ContributionModeCopy? {
        guard let data = json?.data(using: .utf8),
            let copy = try? JSONDecoder().decode(ContributionModeCopy.self, from: data)
        else {
            return nil
        }
        let words = [copy.title, copy.mixed, copy.overrideActive, copy.clear, copy.autoPartial]
            + copy.choices.flatMap { [$0.mode, $0.label, $0.line] }
        return words.contains(where: \.isEmpty) || copy.choices.isEmpty ? nil : copy
    }

    /// The choice for a `status.contribution_mode` value, nil for `mixed`,
    /// an unknown value or none.
    public func choice(for mode: String?) -> Choice? {
        choices.first { $0.mode == mode }
    }
}

/// One contribution override's confirmation (#1173, #1208), decoded from
/// `tc_contribution_override_confirm_json`
/// (`project_copy::contribution_override_confirm_copy`). Every word of it is
/// the core's, including the Auto contribute arming disclosure (`arming`),
/// which a shell renders whole beside `body` and never words itself.
public struct ContributionOverrideConfirmCopy: Decodable, Equatable, Sendable {
    /// The `mode` `set_contribution_override` takes.
    public let mode: String
    public let title: String
    public let body: String
    public let confirm: String
    public let cancel: String
    /// The arming disclosure: present for `auto_upload`, and only for it.
    public let arming: AutomaticGrantCopy?

    /// The payload fields this shell decodes, by wire name.
    public static let consumedFields = ["arming", "body", "cancel", "confirm", "mode", "title"]

    /// Decode the payload, or nil if it will not parse, a word is empty, or
    /// an `auto_upload` confirmation arrives without its arming disclosure:
    /// arming is never confirmed from a dialog that did not show it.
    public static func decode(fromJSON json: String?) -> ContributionOverrideConfirmCopy? {
        guard let data = json?.data(using: .utf8),
            let copy = try? JSONDecoder().decode(ContributionOverrideConfirmCopy.self, from: data)
        else {
            return nil
        }
        if [copy.mode, copy.title, copy.body, copy.confirm, copy.cancel].contains(where: \.isEmpty) {
            return nil
        }
        if copy.mode == "auto_upload" {
            guard let arming = copy.arming, !arming.lines.isEmpty, !arming.lines.contains(where: \.isEmpty) else {
                return nil
            }
        }
        return copy
    }

    /// The body's paragraphs, then the arming disclosure's lines, in order.
    public var paragraphs: [String] {
        body.components(separatedBy: "\n\n") + (arming?.lines ?? [])
    }
}
