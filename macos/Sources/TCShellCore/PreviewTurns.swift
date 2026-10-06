import Foundation

/// The turn index over a redacted preview body, decoded from
/// `tc_preview_turns_json` (`TCPreviewTurns`).
///
/// An overlay of byte ranges on the body `tc_preview_body` returned, never
/// text: each turn is the event type that opens it, the tool it names if
/// any, and where it sits in that exact string. Ron's Look inside reads it
/// as the Turn index tab and as turn separators (#1241).
public struct PreviewTurns: Decodable, Equatable, Sendable {
    public struct Turn: Decodable, Equatable, Sendable {
        public let index: Int
        /// The opening event's wire type: `user_message`, `tool_call`, ...
        public let role: String
        /// Absent when the opening event names no tool.
        public let toolName: String?
        public let byteOffset: Int
        public let byteLen: Int

        enum CodingKeys: String, CodingKey {
            case index, role
            case toolName = "tool_name"
            case byteOffset = "byte_offset"
            case byteLen = "byte_len"
        }
    }

    public let entryId: String
    /// The body the offsets index: `sha256:<hex>` over its UTF-8 bytes.
    public let bodyDigest: String
    public let envelopeDigest: String
    public let turnCount: Int
    public let turns: [Turn]

    enum CodingKeys: String, CodingKey {
        case entryId = "entry_id"
        case bodyDigest = "body_digest"
        case envelopeDigest = "envelope_digest"
        case turnCount = "turn_count"
        case turns
    }

    /// Whether this index is for the body with `bodyDigest`. One for any
    /// other body is not drawn: its offsets would still look like a
    /// transcript.
    public func indexes(bodyDigest: String) -> Bool {
        self.bodyDigest == bodyDigest
    }

    /// Decode the index, or nil if it will not parse.
    public static func decode(fromJSON json: String?) -> PreviewTurns? {
        guard let data = json?.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(PreviewTurns.self, from: data)
    }
}
