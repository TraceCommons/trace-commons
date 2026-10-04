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

    enum CodingKeys: String, CodingKey {
        case disclosure
        case patternsOnly = "patterns_only"
        case modelScrubbed = "model_scrubbed"
        case noReview = "no_review"
    }

    /// The payload fields this shell decodes, by wire name.
    public static let consumedFields = ["disclosure", "patterns_only", "model_scrubbed", "no_review"]

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
