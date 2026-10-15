import Foundation

/// How far a contributor has come toward the Flow 1 grant, in the shape of
/// the core's `flow1::Flow1Progress`, so the core decides what still blocks
/// it (`TCFlow1.grantRequestJSON`).
///
/// `connected` is set from the daemon's status every time, never from a
/// step the shell remembers.
public struct Flow1Progress: Encodable, Equatable, Sendable {
    /// The path question's two answers, as the core spells them.
    public enum Path: String, Encodable, Equatable, Sendable {
        case automatic
        case askFirst = "ask_first"
    }

    public var connected: Bool
    /// The scopes the contributor chose and the daemon saved. Nil or empty
    /// is not a choice, and blocks the grant.
    public var scopesSaved: [String]?
    public var path: Path?
    public var scrubDisclosureSeen: Bool
    public var witnessDisclosureSeen: Bool
    /// The signing address of the witness the disclosure screen showed, nil
    /// for none.
    public var witnessShown: String?

    public init(
        connected: Bool = false,
        scopesSaved: [String]? = nil,
        path: Path? = nil,
        scrubDisclosureSeen: Bool = false,
        witnessDisclosureSeen: Bool = false,
        witnessShown: String? = nil
    ) {
        self.connected = connected
        self.scopesSaved = scopesSaved
        self.path = path
        self.scrubDisclosureSeen = scrubDisclosureSeen
        self.witnessDisclosureSeen = witnessDisclosureSeen
        self.witnessShown = witnessShown
    }

    enum CodingKeys: String, CodingKey {
        case connected
        case scopesSaved = "scopes_saved"
        case path
        case scrubDisclosureSeen = "scrub_disclosure_seen"
        case witnessDisclosureSeen = "witness_disclosure_seen"
        case witnessShown = "witness_shown"
    }

    /// Every key is written, a nil one as JSON null, so what the core reads
    /// does not depend on its defaults for a missing key.
    public func encode(to encoder: any Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(connected, forKey: .connected)
        try container.encode(scopesSaved, forKey: .scopesSaved)
        try container.encode(path, forKey: .path)
        try container.encode(scrubDisclosureSeen, forKey: .scrubDisclosureSeen)
        try container.encode(witnessDisclosureSeen, forKey: .witnessDisclosureSeen)
        try container.encode(witnessShown, forKey: .witnessShown)
    }

    /// The JSON `tc_flow1_grant_request_json` takes, or nil if it could not
    /// be encoded.
    public func jsonString() -> String? {
        guard let data = try? JSONEncoder().encode(self) else { return nil }
        return String(data: data, encoding: .utf8)
    }
}

/// The core's answer to whether the grant may be asked for now. `ready`
/// exactly when `blockers` is empty; only then is `witnessSigningAddress`
/// the witness to pass to `grant_automatic` (nil for none).
public struct Flow1GrantRequest: Decodable, Equatable, Sendable {
    public let ready: Bool
    /// Fixed labels, in step order: `connect`, `scope`, `path`,
    /// `scrub_disclosure`, `witness_disclosure`.
    public let blockers: [String]
    public let witnessSigningAddress: String?

    enum CodingKeys: String, CodingKey {
        case ready, blockers
        case witnessSigningAddress = "witness_signing_address"
    }

    /// Nil for a NULL answer or one that does not decode: not ready.
    public static func decode(fromJSON json: String?) -> Flow1GrantRequest? {
        guard let json else { return nil }
        return try? JSONDecoder().decode(Flow1GrantRequest.self, from: Data(json.utf8))
    }
}

/// The daemon's `grant_automatic` answer. An ungranted answer carries only
/// `granted`, so the other two are optional.
public struct AutomaticGrant: Decodable, Equatable, Sendable {
    public let granted: Bool
    public let grantedAt: Date?
    /// A source has been recorded under the grant (not every source).
    public let onDiskRecorded: Bool?

    enum CodingKeys: String, CodingKey {
        case granted
        case grantedAt = "granted_at"
        case onDiskRecorded = "on_disk_recorded"
    }
}
