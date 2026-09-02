import Foundation

/// The routing surface's fixed words, decoded from what the Rust exported.
///
/// Every property here is filled from the payload. None of them has a
/// default and none is written in Swift: a word this shell invented would be
/// a word the Linux and Windows shells do not print, and `word_private` is a
/// privacy claim, so inventing one is inventing a claim.
///
/// Decoding is here rather than in `TCBridge` so it can be tested without
/// linking the dylib; `TCBridgeTests` checks the same properties against the
/// real export.
public struct RoutingCopy: Decodable, Equatable, Sendable {
    public let toolsHeading: String
    /// The one word on this surface that claims privacy.
    public let wordPrivate: String
    /// The not-wired word. Deliberately not "Not private": "Private" is a
    /// substring of that, and a shell matching with `contains` would find
    /// the wrong one.
    public let wordDirect: String
    public let wordUnknown: String
    public let wordNotUsed: String
    public let toolClaude: String
    public let toolCodex: String
    public let toolGemini: String
    public let intro: String
    public let toggle: String
    public let appliesAtOnce: String
    public let portTitle: String
    public let portNote: String
    public let folderTitle: String
    public let folderNote: String
    public let apply: String
    public let checking: String
    public let checkUnavailable: String
    public let probeReachable: String
    public let stateOff: String
    public let stateWaiting: String
    public let stateReading: String

    enum CodingKeys: String, CodingKey {
        case toolsHeading = "tools_heading"
        case wordPrivate = "word_private"
        case wordDirect = "word_direct"
        case wordUnknown = "word_unknown"
        case wordNotUsed = "word_not_used"
        case toolClaude = "tool_claude"
        case toolCodex = "tool_codex"
        case toolGemini = "tool_gemini"
        case intro
        case toggle
        case appliesAtOnce = "applies_at_once"
        case portTitle = "port_title"
        case portNote = "port_note"
        case folderTitle = "folder_title"
        case folderNote = "folder_note"
        case apply
        case checking
        case checkUnavailable = "check_unavailable"
        case probeReachable = "probe_reachable"
        case stateOff = "state_off"
        case stateWaiting = "state_waiting"
        case stateReading = "state_reading"
    }

    /// Decode the payload, or nil if it will not parse.
    ///
    /// Nil, never a partly-filled value with placeholder words: a screen that
    /// renders "" beside a tool name is worse than one that renders nothing,
    /// and a screen that renders a Swift-authored word is worse than both.
    public static func decode(fromJSON json: String) -> RoutingCopy? {
        guard let data = json.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(RoutingCopy.self, from: data)
    }

    /// The four words, in the order the surface uses them.
    public var words: [String] {
        [wordPrivate, wordDirect, wordUnknown, wordNotUsed]
    }
}
