import Foundation

/// One row of the first-run past-session picker: the daemon's
/// `list_past_sessions` row (`crates/trace-commons-contributor/src/daemon/
/// past_sessions.rs`, `PastSessionRow`). Display fields only; the session's
/// identity on the wire is the opaque `session_id`, never a path.
///
/// A `not_queued` row was never opened, so it carries a date and a size and
/// nothing else: `title` and `durationSecs` are nil there by design.
public struct PastSession: Decodable, Identifiable, Equatable, Sendable {
    public enum State: String, Sendable {
        case pending
        case approved
        case expired
        case notQueued = "not_queued"
        case never
        case stillActive = "still_active"
        /// Held for a person's review of that one session; an include
        /// skips it, so it is never selectable.
        case heldForReview = "held_for_review"
        /// One a group approval leaves out for this contributor; an include
        /// skips it, so it is never selectable.
        case ineligible
    }

    /// The daemon's `session_id`.
    public let id: String
    public let entryID: String?
    public let state: State
    /// Whether the picker may tick this row. Always false for a state this
    /// shell does not know (fail closed), whatever the wire said.
    public let selectable: Bool
    public let startedAt: Date?
    public let durationSecs: Int?
    public let title: String?
    public let sizeBytes: Int
    public let source: String

    public init(
        id: String,
        entryID: String?,
        state: State,
        selectable: Bool,
        startedAt: Date?,
        durationSecs: Int?,
        title: String?,
        sizeBytes: Int,
        source: String
    ) {
        self.id = id
        self.entryID = entryID
        self.state = state
        self.selectable = selectable
        self.startedAt = startedAt
        self.durationSecs = durationSecs
        self.title = title
        self.sizeBytes = sizeBytes
        self.source = source
    }

    enum CodingKeys: String, CodingKey {
        case id = "session_id"
        case entryID = "entry_id"
        case state
        case selectable
        case startedAt = "started_at"
        case durationSecs = "duration_secs"
        case title
        case sizeBytes = "size_bytes"
        case source
    }

    /// Decodes its own timestamp rather than relying on the caller's
    /// decoder: this module carries none of the app's date plumbing, and
    /// chrono writes nanoseconds, which `ISO8601DateFormatter` refuses.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        entryID = try c.decodeIfPresent(String.self, forKey: .entryID)
        let known = State(rawValue: try c.decode(String.self, forKey: .state))
        state = known ?? .never
        selectable = known == nil ? false : try c.decode(Bool.self, forKey: .selectable)
        startedAt = try c.decodeIfPresent(String.self, forKey: .startedAt)
            .flatMap(SourceCandidate.Wire.parseTimestamp)
        durationSecs = try c.decodeIfPresent(Int.self, forKey: .durationSecs)
        title = try c.decodeIfPresent(String.self, forKey: .title)
        sizeBytes = try c.decode(Int.self, forKey: .sizeBytes)
        source = try c.decode(String.self, forKey: .source)
    }

}

/// The whole `list_past_sessions` answer for one folder.
public struct PastSessionList: Decodable, Equatable, Sendable {
    /// The folder's newest sessions, at most the 500 one include may name.
    public let sessions: [PastSession]
    public let total: Int
    /// How many older sessions the folder has beyond `sessions`; 0 from a
    /// daemon that does not say.
    public let notListed: Int
    /// The folder's resolved rule: `notify_only`, `auto_upload` or `ignore`.
    public let projectMode: String?

    enum CodingKeys: String, CodingKey {
        case sessions
        case total
        case notListed = "not_listed"
        case projectMode = "project_mode"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        sessions = try c.decode([PastSession].self, forKey: .sessions)
        total = try c.decode(Int.self, forKey: .total)
        notListed = try c.decodeIfPresent(Int.self, forKey: .notListed) ?? 0
        projectMode = try c.decodeIfPresent(String.self, forKey: .projectMode)
    }
}

/// What `include_past_sessions` came to: every distinct id asked for is
/// counted in `approved` or listed in `skipped` with a fixed wire label.
public struct IncludeOutcome: Decodable, Equatable, Sendable {
    public struct Skip: Decodable, Equatable, Sendable {
        public let sessionID: String
        public let label: String

        enum CodingKeys: String, CodingKey {
            case sessionID = "session_id"
            case label
        }
    }

    public let approved: Int
    public let skipped: [Skip]
    /// The queue entries approved, for `cancel` (Undo).
    public let approvedEntryIDs: [String]
    /// When the approvals' undo window ends, as the daemon wrote it; nil
    /// when nothing was approved or the hold is off.
    public let holdUntil: String?

    enum CodingKeys: String, CodingKey {
        case approved
        case skipped
        case approvedEntryIDs = "approved_entry_ids"
        case holdUntil = "hold_until"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        approved = try c.decode(Int.self, forKey: .approved)
        skipped = try c.decode([Skip].self, forKey: .skipped)
        approvedEntryIDs = try c.decodeIfPresent([String].self, forKey: .approvedEntryIDs) ?? []
        holdUntil = try c.decodeIfPresent(String.self, forKey: .holdUntil)
    }
}
