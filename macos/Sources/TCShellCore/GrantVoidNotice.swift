import Foundation

/// One element of `status.grant_voids`: a grant the daemon voided because the
/// terms it was given under widened (R6 of the connect-and-forget design),
/// not yet shown to the contributor.
///
/// Only `id` is read here -- it is what `acknowledge_grant_voids` takes. The
/// rest of the element is kept verbatim in `json` and handed back to the
/// Rust (`TCConsentCopy.voidNoticeJSON`), which reads `kind` and `reasons`
/// and returns the words. This shell never reads either to choose a
/// sentence; a branch kept in four shells drifts the way words do.
///
/// Decodes any element that carries an id, so a shape this build does not
/// recognise is still a void to report rather than a status that fails to
/// decode.
public struct GrantVoidWire: Decodable, Equatable, Sendable {
    public let id: UInt64
    /// The element as the daemon sent it, re-encoded.
    public let json: String
    /// The project a project void names, for "Turn back on". Read only
    /// through `GrantVoidNotice.rearmTarget(for:)`, which returns it only
    /// when the Rust offered the button.
    public let projectId: String?

    public init(from decoder: Decoder) throws {
        let value = try JSONValue(from: decoder)
        guard case .object(let fields) = value, case .number(let n)? = fields["id"],
            n >= 0, n.rounded() == n, n <= Double(UInt64.max)
        else {
            throw DecodingError.dataCorrupted(
                .init(codingPath: decoder.codingPath, debugDescription: "grant_voids.id"))
        }
        id = UInt64(n)
        if case .string(let project)? = fields["project_id"], !project.isEmpty {
            projectId = project
        } else {
            projectId = nil
        }
        let data = try JSONEncoder().encode(value)
        json = String(decoding: data, as: UTF8.self)
    }
}

/// A JSON value, kept whole so an element can go back to the core as it came.
private enum JSONValue: Codable, Equatable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() {
            self = .null
        } else if let b = try? c.decode(Bool.self) {
            self = .bool(b)
        } else if let n = try? c.decode(Double.self) {
            self = .number(n)
        } else if let s = try? c.decode(String.self) {
            self = .string(s)
        } else if let a = try? c.decode([JSONValue].self) {
            self = .array(a)
        } else {
            self = .object(try c.decode([String: JSONValue].self))
        }
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .null: try c.encodeNil()
        case .bool(let b): try c.encode(b)
        case .number(let n):
            // Ids and counts are integers; write them as integers so the
            // core reads `4`, not `4.0`.
            if n.rounded() == n, abs(n) < 1e15 { try c.encode(Int64(n)) } else { try c.encode(n) }
        case .string(let s): try c.encode(s)
        case .array(let a): try c.encode(a)
        case .object(let o): try c.encode(o)
        }
    }
}

/// The notice the Rust assembled for one void. Every property is filled from
/// the payload and none is written in Swift.
public struct GrantVoidNotice: Decodable, Equatable, Sendable {
    public let title: String
    public let body: String
    public let reasonsHeading: String
    /// One sentence per reason, in the daemon's order.
    public let reasons: [String]
    /// That automatic contributing can be turned back on, and that doing so
    /// agrees to the new settings.
    public let rearm: String
    /// The button. It records that the notice was shown and does nothing
    /// else.
    public let acknowledge: String
    /// "Turn back on", or nil when the notice has no project to arm (the
    /// automatic grant's, and an unplaced one).
    public let rearmAction: String?
    /// Shown when the daemon refuses the re-arm; nil exactly when
    /// `rearmAction` is.
    public let rearmFailed: String?

    enum CodingKeys: String, CodingKey {
        case title, body
        case reasonsHeading = "reasons_heading"
        case reasons, rearm, acknowledge
        case rearmAction = "rearm_action"
        case rearmFailed = "rearm_failed"
    }

    /// The payload fields this shell decodes, by wire name. Compared against
    /// the live export by `TCBridgeTests`.
    public static let consumedFields = [
        "title", "body", "reasons_heading", "reasons", "rearm", "acknowledge",
        "rearm_action", "rearm_failed",
    ]

    /// The project "Turn back on" arms, or nil when there is no button. The
    /// button sends `set_project_mode` with this id and `auto_upload`, the
    /// same call as arming a project in Settings.
    public func rearmTarget(for void: GrantVoidWire) -> String? {
        rearmAction == nil ? nil : void.projectId
    }

    /// Decode the payload, or nil if it will not parse, a field is empty, or
    /// there is no reason. Shown whole or not at all: a notice that says
    /// automatic contributing stopped without saying why is the silent void
    /// this exists to prevent, one layer down.
    public static func decode(fromJSON json: String) -> GrantVoidNotice? {
        guard let data = json.data(using: .utf8),
            let notice = try? JSONDecoder().decode(GrantVoidNotice.self, from: data)
        else {
            return nil
        }
        let sentences = [notice.title, notice.body, notice.reasonsHeading, notice.rearm, notice.acknowledge]
            + notice.reasons
        if notice.reasons.isEmpty || sentences.contains(where: \.isEmpty) { return nil }
        // The button and its refusal line travel together, and neither is
        // ever an empty string.
        if (notice.rearmAction == nil) != (notice.rearmFailed == nil) { return nil }
        if notice.rearmAction?.isEmpty == true || notice.rearmFailed?.isEmpty == true { return nil }
        return notice
    }
}
