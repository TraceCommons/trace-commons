import Foundation

/// The consent surface's fixed sentences, decoded from what the Rust
/// exported.
///
/// Every property here is filled from the payload and none is written in
/// Swift: a sentence this shell invented would be one the Linux and Windows
/// shells do not print, and `gateStatement` is the claim a contributor reads
/// immediately above an irreversible button, so inventing one is inventing a
/// claim.
///
/// Decoding is here rather than in `TCBridge` so it can be tested without
/// linking the dylib; `TCBridgeTests` checks the same properties against the
/// real export.
public struct ConsentCopy: Decodable, Equatable, Sendable {
    /// The claim that replaced the acknowledgement checkbox.
    public let gateStatement: String
    /// The tooltip on an armed `Contribute`.
    public let readyHelp: String
    /// The tooltip on a `Contribute` with nothing to bind to. Never chosen
    /// here: `TCConsentCopy.gateHelp(pinned:)` asks the ABI which of the two
    /// applies, because a branch kept in three shells drifts the same way
    /// words do.
    public let notPinnedHelp: String
    /// What runs on every automatically contributed session, for the Flow 1
    /// grant screen. Not rendered yet; decoded so this shell has it before
    /// that screen exists, rather than writing its own.
    public let autoScrubScope: String
    /// The limit of both halves of the scrubbing.
    public let autoScrubLimit: String
    /// That no one reviews a session before it is sent. The sentence the
    /// product will most want to soften; this shell never rewrites it.
    public let autoNoReview: String

    enum CodingKeys: String, CodingKey {
        case gateStatement = "gate_statement"
        case readyHelp = "ready_help"
        case notPinnedHelp = "not_pinned_help"
        case autoScrubScope = "auto_scrub_scope"
        case autoScrubLimit = "auto_scrub_limit"
        case autoNoReview = "auto_no_review"
    }

    /// The payload fields this shell decodes, by wire name. Compared against
    /// the live export by `TCBridgeTests`.
    public static let consumedFields = [
        "gate_statement", "ready_help", "not_pinned_help",
        "auto_scrub_scope", "auto_scrub_limit", "auto_no_review",
    ]

    /// Every sentence, for the refuse-on-any-empty-field check.
    public var sentences: [String] {
        [gateStatement, readyHelp, notPinnedHelp, autoScrubScope, autoScrubLimit, autoNoReview]
    }

    /// Decode the payload, or nil if it will not parse or a field is empty.
    ///
    /// Nil, never a partly-filled value: a screen that renders "" where a
    /// safety claim goes is worse than one that renders nothing, and one
    /// that renders a Swift-authored claim is worse than both.
    public static func decode(fromJSON json: String) -> ConsentCopy? {
        guard let data = json.data(using: .utf8),
            let copy = try? JSONDecoder().decode(ConsentCopy.self, from: data)
        else {
            return nil
        }
        return copy.sentences.contains(where: \.isEmpty) ? nil : copy
    }
}
