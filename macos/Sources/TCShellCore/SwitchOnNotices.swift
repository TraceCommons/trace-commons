import Foundation

// The two notices the connect-and-forget design requires before the
// automatic-contribution gate is enforced: a folder armed under the old
// "will be scrubbed" wording, told what its arming now means
// (`status.arming_rewordings`, K5), and armed folders the gate is holding
// (`status.automatic_contribution_held`). Every word comes from the Rust
// (`consent_copy`) across the ABI; these types only carry the wire and
// decode the payload.

/// One element of `status.arming_rewordings`. Only `id` is read -- it is what
/// `acknowledge_arming_rewordings` takes. The rest is kept verbatim in `json`
/// and handed back to the Rust, which words the notice.
public struct ArmingRewordingWire: Decodable, Equatable, Sendable {
    public let id: UInt64
    /// The element as the daemon sent it, re-encoded.
    public let json: String
    /// The project "Ask me first" switches. Read only through
    /// `ArmingRewordedNotice.askFirstTarget(for:)`.
    public let projectId: String?

    public init(from decoder: Decoder) throws {
        let value = try JSONValue(from: decoder)
        guard case .object(let fields) = value, case .number(let n)? = fields["id"],
            n >= 0, n.rounded() == n, n <= Double(UInt64.max)
        else {
            throw DecodingError.dataCorrupted(
                .init(codingPath: decoder.codingPath, debugDescription: "arming_rewordings.id"))
        }
        id = UInt64(n)
        if case .string(let project)? = fields["project_id"], !project.isEmpty {
            projectId = project
        } else {
            projectId = nil
        }
        json = String(decoding: try JSONEncoder().encode(value), as: UTF8.self)
    }
}

/// The notice the Rust assembled for one rewording. Every property is filled
/// from the payload and none is written in Swift.
public struct ArmingRewordedNotice: Decodable, Equatable, Sendable {
    public let title: String
    /// That the folder is still armed, and why it is being told anything.
    public let body: String
    public let nowHeading: String
    /// The patterns-only disclosure, word for word.
    public let scope: String
    public let limit: String
    public let noReview: String
    /// Records that the notice was shown, and does nothing else.
    public let acknowledge: String
    /// "Ask me first", or nil when the element names no project.
    public let askFirstAction: String?
    /// Shown when the daemon refuses the switch; nil exactly when
    /// `askFirstAction` is.
    public let askFirstFailed: String?

    enum CodingKeys: String, CodingKey {
        case title, body
        case nowHeading = "now_heading"
        case scope, limit
        case noReview = "no_review"
        case acknowledge
        case askFirstAction = "ask_first_action"
        case askFirstFailed = "ask_first_failed"
    }

    /// The payload fields this shell decodes, by wire name. Compared against
    /// the live export by `TCBridgeTests`.
    public static let consumedFields = [
        "title", "body", "now_heading", "scope", "limit", "no_review", "acknowledge",
        "ask_first_action", "ask_first_failed",
    ]

    /// The project "Ask me first" switches to ask-first, or nil when there is
    /// no button. The button sends `set_project_mode` with this id and
    /// `notify_only`, the same call as Settings.
    public func askFirstTarget(for rewording: ArmingRewordingWire) -> String? {
        askFirstAction == nil ? nil : rewording.projectId
    }

    /// Decode the payload, or nil if it will not parse or a sentence is
    /// empty: shown whole or not at all.
    public static func decode(fromJSON json: String) -> ArmingRewordedNotice? {
        guard let data = json.data(using: .utf8),
            let notice = try? JSONDecoder().decode(ArmingRewordedNotice.self, from: data)
        else {
            return nil
        }
        let sentences = [
            notice.title, notice.body, notice.nowHeading, notice.scope, notice.limit,
            notice.noReview, notice.acknowledge,
        ]
        if sentences.contains(where: \.isEmpty) { return nil }
        if (notice.askFirstAction == nil) != (notice.askFirstFailed == nil) { return nil }
        if notice.askFirstAction?.isEmpty == true || notice.askFirstFailed?.isEmpty == true {
            return nil
        }
        return notice
    }
}

/// `status.automatic_contribution_held`: what the automatic-contribution gate
/// held at the daemon's last full pass. Beside `health` rather than in it,
/// because a higher label can mask `automatic-contribution-held`. A daemon
/// that predates it holds nothing.
public struct GateHeld: Decodable, Equatable, Sendable {
    public let heldSessions: Int
    /// The object as the daemon sent it, re-encoded, for the Rust to word.
    public let json: String

    public init(from decoder: Decoder) throws {
        let value = try JSONValue(from: decoder)
        guard case .object(let fields) = value, case .number(let n)? = fields["held_sessions"],
            n >= 0, n.rounded() == n, n <= Double(Int.max)
        else {
            throw DecodingError.dataCorrupted(
                .init(
                    codingPath: decoder.codingPath,
                    debugDescription: "automatic_contribution_held.held_sessions"))
        }
        heldSessions = Int(n)
        json = String(decoding: try JSONEncoder().encode(value), as: UTF8.self)
    }

    private init() {
        heldSessions = 0
        json = #"{"held_sessions":0,"reasons":[],"projects":[]}"#
    }

    public static let none = GateHeld()

    public var held: Bool { heldSessions > 0 }

    /// The health label the daemon sets from the same pass.
    public static let label = "automatic-contribution-held"
}

/// One held folder in the Rust's notice.
public struct GateHeldProjectNotice: Decodable, Equatable, Sendable {
    /// For the "Ask me first" button's `set_project_mode`. Never shown.
    public let projectId: String?
    public let line: String
    public let askFirstAction: String?
    public let askFirstFailed: String?

    enum CodingKeys: String, CodingKey {
        case projectId = "project_id"
        case line
        case askFirstAction = "ask_first_action"
        case askFirstFailed = "ask_first_failed"
    }
}

/// The notice the Rust assembled for what the gate holds. It is never
/// acknowledged: it goes when the hold does.
public struct GateHeldNotice: Decodable, Equatable, Sendable {
    public let title: String
    public let body: String
    public let reasons: [String]
    public let release: String
    public let askFirst: String
    public let projects: [GateHeldProjectNotice]

    enum CodingKeys: String, CodingKey {
        case title, body, reasons, release
        case askFirst = "ask_first"
        case projects
    }

    public static let consumedFields = ["title", "body", "reasons", "release", "ask_first", "projects"]

    /// Decode the payload, or nil if it will not parse, a sentence is empty,
    /// there is no reason, or a folder's button comes without its refusal
    /// line: shown whole or not at all.
    public static func decode(fromJSON json: String) -> GateHeldNotice? {
        guard let data = json.data(using: .utf8),
            let notice = try? JSONDecoder().decode(GateHeldNotice.self, from: data)
        else {
            return nil
        }
        let sentences = [notice.title, notice.body, notice.release, notice.askFirst] + notice.reasons
        if notice.reasons.isEmpty || sentences.contains(where: \.isEmpty) { return nil }
        for project in notice.projects {
            if project.line.isEmpty { return nil }
            if (project.askFirstAction == nil) != (project.askFirstFailed == nil) { return nil }
            if (project.askFirstAction == nil) != (project.projectId == nil) { return nil }
        }
        return notice
    }
}
