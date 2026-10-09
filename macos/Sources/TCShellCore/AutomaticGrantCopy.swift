import Foundation

/// What an armed folder is told happens to its sessions, decoded from
/// `tc_automatic_grant_copy_json` (`consent_copy::automatic_grant_copy_named`).
///
/// The daemon chooses the disclosure for each armed folder and names it on
/// the folder's `list_projects` row (`automatic_disclosure`); the core words
/// that answer. Exactly one of `patternsOnly` and `modelScrubbed` arrives,
/// the one `disclosure` names, so this shell never holds the model-scrub
/// wording for a folder that did not earn it.
///
/// Decoding is here rather than in `TCBridge` so it can be tested without
/// linking the dylib; `TCBridgeTests` checks it against the real export.
public struct AutomaticGrantCopy: Decodable, Equatable, Sendable {
    public struct Scrub: Decodable, Equatable, Sendable {
        public let scope: String
        public let limit: String
    }

    public let disclosure: String
    public let patternsOnly: Scrub?
    public let modelScrubbed: Scrub?
    public let noReview: String
    /// The path question's two answers (`path_automatic`, `path_ask_first`),
    /// read by the first run's Sharing card. Optional because an armed
    /// folder's confirmation does not need them; a screen that does treats
    /// nil as the copy being unavailable.
    public let pathAutomatic: String?
    public let pathAskFirst: String?
    /// Each answer split into the line shown and the rest, shown behind an
    /// info button (owner, 2026-10-08). The Sharing card treats nil as the
    /// copy being unavailable, as for the whole answers.
    public let pathAutomaticTitle: String?
    public let pathAutomaticDetail: String?
    public let pathAskFirstTitle: String?
    public let pathAskFirstDetail: String?

    enum CodingKeys: String, CodingKey {
        case disclosure
        case patternsOnly = "patterns_only"
        case modelScrubbed = "model_scrubbed"
        case noReview = "no_review"
        case pathAutomatic = "path_automatic"
        case pathAskFirst = "path_ask_first"
        case pathAutomaticTitle = "path_automatic_title"
        case pathAutomaticDetail = "path_automatic_detail"
        case pathAskFirstTitle = "path_ask_first_title"
        case pathAskFirstDetail = "path_ask_first_detail"
    }

    /// The payload fields this shell decodes, by wire name.
    public static let consumedFields = [
        "disclosure", "patterns_only", "model_scrubbed", "no_review", "path_automatic", "path_ask_first",
        "path_automatic_title", "path_automatic_detail", "path_ask_first_title", "path_ask_first_detail",
    ]

    /// The scrub wording for the disclosure named, and only that one.
    public var scrub: Scrub? {
        switch disclosure {
        case "patterns_only": patternsOnly
        case "model_scrubbed": modelScrubbed
        default: nil
        }
    }

    /// What an armed folder says, in order: what the scrub removes, where it
    /// stops, and that nothing waits for review.
    public var lines: [String] {
        guard let scrub else { return [] }
        return [scrub.scope, scrub.limit, noReview]
    }

    /// Decode the payload, or nil if it will not parse, names no scrub
    /// wording it carries, or a line is empty.
    public static func decode(fromJSON json: String?) -> AutomaticGrantCopy? {
        guard let data = json?.data(using: .utf8),
            let copy = try? JSONDecoder().decode(AutomaticGrantCopy.self, from: data),
            !copy.lines.isEmpty, !copy.lines.contains(where: \.isEmpty)
        else {
            return nil
        }
        return copy
    }
}
