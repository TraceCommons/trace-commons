import Foundation
import TCShellCore

// Typed shapes for `trace_commons.daemon.v1_1`, as specified in
// docs/contributor-daemon-ipc-v1_1.md. Nothing here carries a filesystem
// path: the contract keeps `project_key` and `path` off the wire on purpose,
// and this layer has no field to put one in.

// MARK: - Queue

enum QueueState: String, Codable {
    case pending, approved, uploading, uploaded, refused, failed, expired, superseded

    /// Every state except `uploaded`/`uploading` means nothing left this
    /// machine, and the queue view says so in words rather than in a colour.
    var nothingWasSent: Bool {
        switch self {
        case .uploaded, .uploading: return false
        default: return true
        }
    }
}

struct QueueEntry: Decodable, Identifiable, Hashable {
    let entryID: String
    let sessionHash: String
    let source: String
    /// The opaque id `set_project_mode` and `approve`'s `project_id` filter
    /// both accept. Never a path, and never `projectLabel` -- the daemon
    /// refuses a label there (`project-key-unrecognized`), and a label is
    /// not guaranteed unique across two projects in the first place.
    let projectID: String
    let projectLabel: String
    let sizeBytes: Int
    let discoveredAt: Date
    let state: QueueState
    let reasonLabel: String?
    let attempts: Int
    /// How many delegated subagent transcripts this entry's session covers,
    /// and how many were left out because the conversation exceeded the
    /// source's raw byte budget.
    ///
    /// Optional because a daemon predating the fields sends neither, and a
    /// missing count is not a count of zero -- but both read as "nothing to
    /// say" through `subagentLine`, which is the only correct rendering of
    /// silence here. See `TCShellCore.SubagentCopy` for the words.
    let subagentCount: Int?
    let subagentsDropped: Int?

    var id: String { entryID }

    /// The card's extent line, or `nil` when there is nothing to report.
    /// The contract makes surfacing a non-zero `subagents_dropped`
    /// mandatory: a conversation trimmed to fit must say so rather than
    /// presenting as complete.
    var subagentLine: String? {
        SubagentCopy.line(count: subagentCount ?? 0, dropped: subagentsDropped ?? 0)
    }

    /// Whether this card is standing for a deliberately trimmed
    /// conversation. Drives tone only; the sentence says the rest.
    var wasTrimmed: Bool { (subagentsDropped ?? 0) > 0 }

    enum CodingKeys: String, CodingKey {
        case entryID = "entry_id"
        case sessionHash = "session_hash"
        case source
        case projectID = "project_id"
        case projectLabel = "project_label"
        case sizeBytes = "size_bytes"
        case discoveredAt = "discovered_at"
        case state
        case reasonLabel = "reason_label"
        case attempts
        case subagentCount = "subagent_count"
        case subagentsDropped = "subagents_dropped"
    }

    /// "Claude Code" / "Codex", never the raw source token.
    var agentName: String {
        switch source {
        case "claude-code", "claude_code": return "Claude Code"
        case "codex": return "Codex"
        case "gemini-cli", "gemini_cli": return "Gemini CLI"
        case "trajectory", "letta_trajectory": return "Letta trajectory"
        default:
            return source
                .replacingOccurrences(of: "_", with: " ")
                .replacingOccurrences(of: "-", with: " ")
                .capitalized
        }
    }
}

private struct PendingList: Decodable {
    let pending: [QueueEntry]
}

// MARK: - Status

struct DaemonHealth: Decodable, Equatable {
    let lastErrorLabel: String?
    let since: Date?

    enum CodingKeys: String, CodingKey {
        case lastErrorLabel = "last_error_label"
        case since
    }
}

struct DaemonStatus: Decodable, Equatable {
    let schemaVersion: String
    let loggedIn: Bool
    let tenantID: String?
    let consentScopes: [String]
    let paused: Bool
    let queueDepth: Int
    let nextDigestAt: Date?
    let health: DaemonHealth
    /// The daily volume caps and what they are holding back.
    ///
    /// Decoded with a fallback rather than as an optional: a daemon that
    /// predates the field reports an unspent budget blocking nothing, which
    /// is the only safe reading of silence here.
    let dailyBudget: DailyBudget

    enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version"
        case loggedIn = "logged_in"
        case tenantID = "tenant_id"
        case consentScopes = "consent_scopes"
        case paused
        case queueDepth = "queue_depth"
        case nextDigestAt = "next_digest_at"
        case health
        case dailyBudget = "daily_budget"
    }

    init(
        schemaVersion: String,
        loggedIn: Bool,
        tenantID: String?,
        consentScopes: [String],
        paused: Bool,
        queueDepth: Int,
        nextDigestAt: Date?,
        health: DaemonHealth,
        dailyBudget: DailyBudget = .unknown
    ) {
        self.schemaVersion = schemaVersion
        self.loggedIn = loggedIn
        self.tenantID = tenantID
        self.consentScopes = consentScopes
        self.paused = paused
        self.queueDepth = queueDepth
        self.nextDigestAt = nextDigestAt
        self.health = health
        self.dailyBudget = dailyBudget
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        schemaVersion = try c.decodeIfPresent(String.self, forKey: .schemaVersion) ?? ""
        loggedIn = try c.decodeIfPresent(Bool.self, forKey: .loggedIn) ?? false
        tenantID = try c.decodeIfPresent(String.self, forKey: .tenantID)
        consentScopes = try c.decodeIfPresent([String].self, forKey: .consentScopes) ?? []
        paused = try c.decodeIfPresent(Bool.self, forKey: .paused) ?? false
        queueDepth = try c.decodeIfPresent(Int.self, forKey: .queueDepth) ?? 0
        nextDigestAt = try c.decodeIfPresent(Date.self, forKey: .nextDigestAt)
        health = try c.decodeIfPresent(DaemonHealth.self, forKey: .health)
            ?? DaemonHealth(lastErrorLabel: nil, since: nil)
        dailyBudget = try c.decodeIfPresent(DailyBudget.self, forKey: .dailyBudget) ?? .unknown
    }

    static let unknown = DaemonStatus(
        schemaVersion: "",
        loggedIn: false,
        tenantID: nil,
        consentScopes: [],
        paused: false,
        queueDepth: 0,
        nextDigestAt: nil,
        health: DaemonHealth(lastErrorLabel: nil, since: nil)
    )
}

// MARK: - Preview

/// The socket `preview` result: summary only, never the trace body.
struct PreviewSummary: Decodable, Equatable, Sendable {
    let wouldSendBytes: Int
    let rawSessionBytes: Int
    let eventCount: Int
    let openingPrompt: String
    let redactions: [String: Int]
    let piiLabelsPresent: [String]
    let consentScopes: [String]
    let residualRisk: String

    enum CodingKeys: String, CodingKey {
        case wouldSendBytes = "would_send_bytes"
        case rawSessionBytes = "raw_session_bytes"
        case eventCount = "event_count"
        case openingPrompt = "opening_prompt"
        case redactions
        case piiLabelsPresent = "pii_labels_present"
        case consentScopes = "consent_scopes"
        case residualRisk = "residual_risk"
    }

    /// "12 secrets, 4 tokens, 31 paths" -- category labels and counts only;
    /// the contract guarantees neither map ever carries matched text.
    var redactionReceipt: String {
        if redactions.isEmpty { return "scrubbed: nothing matched" }
        let parts = redactions
            .sorted { $0.value == $1.value ? $0.key < $1.key : $0.value > $1.value }
            .map { "\($0.value) \($0.key.replacingOccurrences(of: "_", with: " "))" }
        return "scrubbed: " + parts.joined(separator: ", ")
    }
}

/// The wire shape `preview_request`'s immediate response and the
/// `preview_ready` event both carry, specialized to this app's own
/// `PreviewSummary` -- see `PreviewRequestResult`'s doc in `TCShellCore` for
/// why the generic lives there instead of a second copy of this decoder.
typealias PreviewRequestResult = TCShellCore.PreviewRequestResult<PreviewSummary>

/// A session refused by the daemon's preview scheduler's admission control,
/// before anything was parsed. `rawSessionBytes` is a `stat`; there is no
/// would-send figure, on purpose -- see the design spec's "Admission
/// control by size". A card renders exactly these two numbers and nothing
/// derived from them.
struct PreviewTooLarge: Equatable {
    let rawSessionBytes: Int
    let limitBytes: Int
}

// MARK: - Audit

/// One row of the daemon's local change log (`list_audit`).
///
/// Every field here is a fixed label by contract -- `action` and `detail`
/// are "never free text, a path, or a token", and `project_label` is the
/// daemon-derived display name, never a `project_key` or a path. That is
/// what makes this shape safe to render at all, and it is why nothing in
/// this app may enrich a row with anything more identifying.
///
/// The contract is equally explicit that this log is a VISIBILITY feature
/// for the contributor, not a security control: nothing in this app may
/// gate, permit or refuse anything on the strength of what is in here.
struct AuditEntry: Decodable, Equatable {
    let at: Date
    let action: String
    let projectLabel: String?
    /// Carried because the contract carries it, and deliberately not
    /// rendered: the Linux shell shows the action and the project only, and
    /// inventing a second line of copy for a label neither shell has ever
    /// displayed would put this app's audit surface out of step with it.
    let detail: String?

    enum CodingKeys: String, CodingKey {
        case at
        case action
        case projectLabel = "project_label"
        case detail
    }
}

// MARK: - History

struct HistoryRecord: Decodable, Identifiable, Equatable {
    let submissionID: String
    let submittedAt: Date
    let projectLabel: String
    let source: String
    let status: String
    let consentScopes: [String]
    let creditPointsPending: Double
    let creditPointsFinal: Double?
    let explanations: [String]
    let lastRefreshedAt: Date?

    var id: String { submissionID }

    enum CodingKeys: String, CodingKey {
        case submissionID = "submission_id"
        case submittedAt = "submitted_at"
        case projectLabel = "project_label"
        case source
        case status
        case consentScopes = "consent_scopes"
        case creditPointsPending = "credit_points_pending"
        case creditPointsFinal = "credit_points_final"
        case explanations
        case lastRefreshedAt = "last_refreshed_at"
    }
}

// MARK: - Withdrawal

/// Which of the three withdrawal tiers the server applied.
///
/// The wire names are the SERVER's, pinned in
/// `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`
/// (`TRACE_WITHDRAWAL_REACH_*`) and mirrored by `DistributionReach` in
/// `crates/trace-commons-contributor/src/withdraw.rs`. The two sides were
/// once written in parallel and did NOT agree -- the Rust client expected
/// `in_commons`/`distributed` while the server sends
/// `commons_not_distributed`/`commons_distributed` -- so the exact strings
/// below matter more than they look. `docs/contributor-daemon-ipc-v1_1.md`
/// still documents the old, wrong pair; the Rust is authoritative.
///
/// Deliberately decoded leniently (see `WithdrawalOutcome`): an unrecognized
/// label must leave this `nil` so the UI says it cannot tell which tier
/// applied, rather than throwing away a withdrawal that really happened.
enum WithdrawalReach: String, Decodable {
    /// Never entered the commons. Nothing was distributed.
    case notDistributed = "not_distributed"
    /// In the commons, never published in an export or benchmark.
    case commonsNotDistributed = "commons_not_distributed"
    /// In the commons AND already published. Copies cannot be recalled.
    case commonsDistributed = "commons_distributed"
}

/// The `withdraw` result: `withdrawn: true` plus the tier that applied.
struct WithdrawalOutcome: Decodable, Equatable {
    let withdrawn: Bool
    /// `nil` when the daemon sent a label this build does not know. The
    /// withdrawal still happened; what cannot be stated is how far the trace
    /// had travelled, and the UI says so rather than guessing the gentler
    /// answer.
    let distributionReach: WithdrawalReach?

    enum CodingKeys: String, CodingKey {
        case withdrawn
        case distributionReach = "distribution_reach"
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        withdrawn = try container.decodeIfPresent(Bool.self, forKey: .withdrawn) ?? true
        let label = try container.decodeIfPresent(String.self, forKey: .distributionReach)
        distributionReach = label.flatMap(WithdrawalReach.init(rawValue:))
    }
}

struct HistoryCounts: Decodable, Equatable {
    let submitted: Int
    let accepted: Int
    let quarantined: Int
    let other: Int
}

struct HistoryRollup: Decodable, Equatable {
    let week: HistoryCounts
    let month: HistoryCounts
    let allTime: HistoryCounts
    let creditPending: Double
    let creditFinal: Double
    let quarantined: Int
    let lastRefreshedAt: Date?

    enum CodingKeys: String, CodingKey {
        case week, month
        case allTime = "all_time"
        case creditPending = "credit_pending"
        case creditFinal = "credit_final"
        case quarantined
        case lastRefreshedAt = "last_refreshed_at"
    }
}

// MARK: - Consent, projects, settings

struct ConsentScope: Decodable, Identifiable, Equatable {
    let name: String
    let description: String
    let alwaysOn: Bool
    let grantsDataUse: Bool

    var id: String { name }

    enum CodingKeys: String, CodingKey {
        case name, description
        case alwaysOn = "always_on"
        case grantsDataUse = "grants_data_use"
    }
}

// `ProjectMode` and `ProjectRow` live in TCShellCore, re-exported here by the
// file-level `import TCShellCore`. They moved because both had wire bugs no
// test could reach from an executable target: `ask` was spelled `"ask"` while
// the daemon sends `"notify_only"`, and `addedAt` was non-optional while a
// project the contributor has not decided about carries null. Either one
// fails the whole array, so `list_projects` never decoded and the projects
// screen rendered its empty state on every machine.

/// `get_settings`: the credential and both session roots are reported as
/// configured-or-not booleans, never as values. This type has nowhere to put
/// the values even if the daemon sent them.
struct DaemonSettingsView: Decodable, Equatable {
    let quiescenceSecs: Int
    let digestIntervalSecs: Int
    let localNotifications: Bool
    let queueTtlDays: Int
    let maxQueueEntries: Int
    let maxUploadsPerDay: Int
    let nearAIConfigured: Bool
    let claudeRootConfigured: Bool
    let codexRootConfigured: Bool

    enum CodingKeys: String, CodingKey {
        case quiescenceSecs = "quiescence_secs"
        case digestIntervalSecs = "digest_interval_secs"
        case localNotifications = "local_notifications"
        case queueTtlDays = "queue_ttl_days"
        case maxQueueEntries = "max_queue_entries"
        case maxUploadsPerDay = "max_uploads_per_day"
        case nearAIConfigured = "near_ai_configured"
        case claudeRootConfigured = "claude_root_configured"
        case codexRootConfigured = "codex_root_configured"
    }
}

/// `enroll`'s success shape. `tenant_id`/`device_key_id` are the same
/// already-public identifiers `whoami` prints -- never key material, never a
/// URL. See "### `enroll`" in the contract for what is deliberately absent
/// on failure.
struct EnrollResult: Decodable, Equatable {
    let enrolled: Bool
    let tenantID: String?
    let deviceKeyID: String?
    let consentScopes: [String]?

    enum CodingKeys: String, CodingKey {
        case enrolled
        case tenantID = "tenant_id"
        case deviceKeyID = "device_key_id"
        case consentScopes = "consent_scopes"
    }
}

// MARK: - Events

enum DaemonEvent: Equatable {
    case snapshot(pending: [QueueEntry], status: DaemonStatus)
    case queueChanged
    case statusChanged
    case digestDue(pending: Int, text: String)
    case resyncRequired
    /// The ABI's synthetic frame for a delivery gap. Treated exactly like
    /// `resync_required`: refetch rather than reason about what was missed.
    case lagged(skipped: Int)
    /// A previously `queued`/`running` scheduled preview finished. Never
    /// published for a `preview_request` that was itself answered from
    /// cache -- see the contract's note that a cache hit "no event
    /// follows".
    case previewReady(PreviewRequestResult)
    case unknown(String)
}

// MARK: - Decoding

enum DaemonDecoding {
    /// chrono serializes `DateTime<Utc>` as RFC 3339 with fractional
    /// seconds; `.iso8601` alone rejects those, so both spellings are tried.
    static func decoder() -> JSONDecoder {
        let decoder = JSONDecoder()
        let withFraction = ISO8601DateFormatter()
        withFraction.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        let plain = ISO8601DateFormatter()
        plain.formatOptions = [.withInternetDateTime]
        decoder.dateDecodingStrategy = .custom { decoder in
            let text = try decoder.singleValueContainer().decode(String.self)
            if let d = withFraction.date(from: text) { return d }
            if let d = plain.date(from: text) { return d }
            throw DecodingError.dataCorrupted(
                .init(codingPath: decoder.codingPath, debugDescription: "unparseable timestamp")
            )
        }
        return decoder
    }

    static func pendingEntries(from data: Data) throws -> [QueueEntry] {
        try decoder().decode(PendingList.self, from: data).pending
    }
}
