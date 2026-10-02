import Foundation

/// C1 of #1173: the typed models every screen reads through
/// `DaemonDataClient`.
///
/// Shapes come from `docs/contributor-daemon-ipc-v1_1.md` and
/// `daemon/ipc.rs` on main. Three rules hold for every type here:
///
/// - **Absent is unknown.** Every field the daemon may omit, or that an
///   older daemon does not send, is an optional decoded with
///   `decodeIfPresent`. Nothing here substitutes a number for a value the
///   daemon did not give: `decisions_owed` absent is `nil`, never `0` and
///   never `queue_depth`.
/// - **Labels stay strings.** A state or reason label is carried as the
///   daemon's own `String`, with a typed reading beside it, so a label a
///   later daemon adds decodes instead of failing the whole reply.
/// - **Explicit `CodingKeys`.** The wire is snake_case; each key is spelled
///   out so a reviewer can check it against the doc, and so these decode
///   with the same plain `JSONDecoder` the existing `ProjectRow` and
///   `HarnessList` already use (`DaemonDataDecoding.decoder()`).
///
/// Types marked `// PROVISIONAL: shape owned by Zaki's C3` describe network
/// methods that do not exist on main yet; C3 writes their real shape into
/// the IPC doc and these follow it.
public enum DaemonData {}

// MARK: - Decoding

public enum DaemonDataDecoding {
    /// chrono writes RFC 3339 with or without fractional seconds and with
    /// either `Z` or `+00:00`; `.iso8601` alone rejects fractions.
    public static func decoder() -> JSONDecoder {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .custom { decoder in
            let text = try decoder.singleValueContainer().decode(String.self)
            if let date = parseDate(text) { return date }
            throw DecodingError.dataCorrupted(
                .init(codingPath: decoder.codingPath, debugDescription: "unparseable timestamp"))
        }
        return decoder
    }

    static func parseDate(_ text: String) -> Date? {
        let withFraction = ISO8601DateFormatter()
        withFraction.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = withFraction.date(from: text) { return date }
        let plain = ISO8601DateFormatter()
        plain.formatOptions = [.withInternetDateTime]
        return plain.date(from: text)
    }
}

// MARK: - Errors

/// Every way a `DaemonDataClient` call can fail, as one type a screen
/// switches on to draw its error states.
public enum DaemonDataError: Error, Equatable, Sendable, CustomStringConvertible {
    /// The core is not answering: stopped, never started, or the attached
    /// transport failed. A screen draws its core-down state, and never a
    /// healthy one, from this.
    case unreachable
    /// The method has no implementation on this daemon yet. Thrown by the
    /// real client for the provisional network methods (Zaki's C3) and for
    /// the few local ones the real client (K1) still has to route.
    case notAvailableYet(method: String)
    /// The daemon answered with an IPC error. `code` and `message` are the
    /// fixed labels the contract defines, safe to show.
    case daemon(code: String, message: String)
    /// The daemon answered, but not in a shape this build reads.
    case undecodable(method: String)

    public var description: String {
        switch self {
        case .unreachable: return "unreachable"
        case .notAvailableYet(let method): return "not-available-yet: \(method)"
        case .daemon(let code, let message): return "\(code): \(message)"
        case .undecodable(let method): return "undecodable: \(method)"
        }
    }
}

// MARK: - Queue entries

extension DaemonData {
    /// One queue entry, as `list_pending`, `list_kept`, `snapshot` and a
    /// preview's `entry` publish it (`ipc::entry_value`).
    public struct QueueEntry: Codable, Equatable, Hashable, Sendable, Identifiable {
        public let entryId: String
        public let sessionHash: String?
        /// The adapter name (`claude-code`, `codex`, ...).
        public let source: String
        public let declaredSource: String?
        public let projectId: String
        public let projectLabel: String
        public let projectPath: String?
        public let sessionPath: String?
        public let sizeBytes: Int?
        public let discoveredAt: Date?
        /// `pending`, `approved`, `uploading`, `uploaded`, `refused`,
        /// `failed`, `expired`, `superseded`. See `queueState`.
        public let state: String
        public let reasonLabel: String?
        public let attempts: Int?
        public let retryAfter: Date?
        public let submissionId: String?
        public let subagentCount: Int?
        public let subagentsDropped: Int?

        // when the session ran and how many prompts it had. All nil for
        // an entry queued before the daemon recorded them.
        public let startedAt: Date?
        public let endedAt: Date?
        public let durationSecs: Int?
        public let userTurns: Int?

        /// Absent when eligibility does not apply to this contributor.
        public let eligibility: String?
        public let eligibilityReason: String?
        public let holdsCertificate: Bool?
        public let attestation: String?
        public let attestationReason: String?

        // the scrub state. `marks`, `contentMarks` and `unsureSpans` are
        // absent -- nil here, never 0 -- until the entry is scrubbed.
        public let scrub: String?
        public let marks: Int?
        public let contentMarks: Int?
        public let unsureSpans: Int?
        /// Fixed reasons: `nothing-matched`, `looks-unsure`, `trimmed-to-fit`.
        /// Empty is an all-clear only when `scrub` is `scrubbed`.
        public let secondLook: [String]?

        public var id: String { entryId }

        public enum CodingKeys: String, CodingKey, CaseIterable {
            case entryId = "entry_id"
            case sessionHash = "session_hash"
            case source
            case declaredSource = "declared_source"
            case projectId = "project_id"
            case projectLabel = "project_label"
            case projectPath = "project_path"
            case sessionPath = "session_path"
            case sizeBytes = "size_bytes"
            case discoveredAt = "discovered_at"
            case state
            case reasonLabel = "reason_label"
            case attempts
            case retryAfter = "retry_after"
            case submissionId = "submission_id"
            case subagentCount = "subagent_count"
            case subagentsDropped = "subagents_dropped"
            case startedAt = "started_at"
            case endedAt = "ended_at"
            case durationSecs = "duration_secs"
            case userTurns = "user_turns"
            case eligibility
            case eligibilityReason = "eligibility_reason"
            case holdsCertificate = "holds_certificate"
            case attestation
            case attestationReason = "attestation_reason"
            case scrub, marks
            case contentMarks = "content_marks"
            case unsureSpans = "unsure_spans"
            case secondLook = "second_look"
        }

        public var queueState: QueueStateLabel? { QueueStateLabel(rawValue: state) }
        public var scrubState: ScrubState? { scrub.flatMap(ScrubState.init(rawValue:)) }

        /// kept on this Mac (`refused` with `kept-on-this-mac`), the only
        /// refusal a contributor can undo.
        public var isKept: Bool { reasonLabel == ReasonLabel.keptOnThisMac }
        /// back from Keep, waiting for a person even in an armed folder.
        public var returnedFromKeep: Bool { reasonLabel == ReasonLabel.returnedFromKeep }
        /// held by Automatic Scrub check for a second look.
        public var heldForSecondLook: Bool { reasonLabel == ReasonLabel.secondLookReviewRequired }
        /// held because Scrub check is Manual.
        public var heldByManualScrubCheck: Bool { reasonLabel == ReasonLabel.scrubCheckManual }
    }

    public enum ReasonLabel {
        public static let keptOnThisMac = "kept-on-this-mac"
        public static let returnedFromKeep = "returned-from-keep"
        public static let secondLookReviewRequired = "second-look-review-required"
        public static let scrubCheckManual = "scrub-check-manual"
    }

    public enum QueueStateLabel: String, Sendable, CaseIterable {
        case pending, approved, uploading, uploaded, refused, failed, expired, superseded
    }

    public enum ScrubState: String, Sendable {
        case scrubbed
        case notYetScrubbed = "not-yet-scrubbed"
    }

    struct PendingList: Decodable { let pending: [QueueEntry] }
    struct KeptList: Decodable { let kept: [QueueEntry] }
}

// MARK: - Status

extension DaemonData {
    /// `status`, and the `status` inside a `snapshot` event.
    public struct Status: Codable, Equatable, Sendable {
        public let schemaVersion: String?
        public let loggedIn: Bool?
        public let tenantId: String?
        public let consentScopes: [String]?
        public let paused: Bool?
        /// Every `Pending` entry. NOT the badge: never draw a count from it.
        public let queueDepth: Int?
        /// the badge's exact count. `nil` is unknown (an older daemon, or
        /// a reply without it): draw "—", never a fallback number.
        public let decisionsOwed: Int?
        public let nextDigestAt: Date?
        public let health: Health?
        public let dailyBudget: DailyBudget?
        public let routing: Routing?
        public let privateInferenceState: PrivateInferenceState?
        public let witnessCapacity: WitnessCapacity?
        public let armingRewordings: [ArmingRewording]?
        public let automaticContributionHeld: AutomaticContributionHeld?

        public enum CodingKeys: String, CodingKey, CaseIterable {
            case schemaVersion = "schema_version"
            case loggedIn = "logged_in"
            case tenantId = "tenant_id"
            case consentScopes = "consent_scopes"
            case paused
            case queueDepth = "queue_depth"
            case decisionsOwed = "decisions_owed"
            case nextDigestAt = "next_digest_at"
            case health
            case dailyBudget = "daily_budget"
            case routing
            case privateInferenceState = "private_inference_state"
            case witnessCapacity = "witness_capacity"
            case armingRewordings = "arming_rewordings"
            case automaticContributionHeld = "automatic_contribution_held"
        }
    }

    public struct Health: Codable, Equatable, Sendable {
        /// `nil` when healthy; one of the health-precedence labels otherwise.
        public let lastErrorLabel: String?
        public let since: Date?

        public enum CodingKeys: String, CodingKey {
            case lastErrorLabel = "last_error_label"
            case since
        }
    }

    public struct DailyBudget: Codable, Equatable, Sendable {
        public let bytesToday: Int?
        public let maxBytesPerDay: Int?
        public let bytesRemaining: Int?
        public let uploadsToday: Int?
        public let maxUploadsPerDay: Int?
        public let uploadsRemaining: Int?
        public let resetsAt: Date?
        public let blocked: Bool?
        public let blockedEntries: Int?
        public let blockedBytes: Int?

        public enum CodingKeys: String, CodingKey {
            case bytesToday = "bytes_today"
            case maxBytesPerDay = "max_bytes_per_day"
            case bytesRemaining = "bytes_remaining"
            case uploadsToday = "uploads_today"
            case maxUploadsPerDay = "max_uploads_per_day"
            case uploadsRemaining = "uploads_remaining"
            case resetsAt = "resets_at"
            case blocked
            case blockedEntries = "blocked_entries"
            case blockedBytes = "blocked_bytes"
        }
    }

    public struct Routing: Codable, Equatable, Sendable {
        /// `not_declared`, `awaiting_rows`, `rows_seen`, `token_unreadable`, `unknown`.
        public let state: String
        public let derived: Bool?
        public let lastRefreshAt: Date?

        public enum CodingKeys: String, CodingKey {
            case state, derived
            case lastRefreshAt = "last_refresh_at"
        }
    }

    public struct PrivateInferenceState: Codable, Equatable, Sendable {
        /// The label `tool_destinations.private_ai` repeats (`running`, ...).
        public let state: String
        public let port: Int?
    }

    public struct WitnessCapacity: Codable, Equatable, Sendable {
        public let waitingSessions: Int?
        public let nextRetryAt: Date?

        public enum CodingKeys: String, CodingKey {
            case waitingSessions = "waiting_sessions"
            case nextRetryAt = "next_retry_at"
        }
    }

    public struct ArmingRewording: Codable, Equatable, Sendable, Identifiable {
        public let id: Int
        public let rewordedAt: Date?
        public let projectId: String?
        public let projectLabel: String?
        public let was: String?
        public let now: String?
        public let scrubCheckDefaulted: Bool?

        public enum CodingKeys: String, CodingKey {
            case id, was, now
            case rewordedAt = "reworded_at"
            case projectId = "project_id"
            case projectLabel = "project_label"
            case scrubCheckDefaulted = "scrub_check_defaulted"
        }
    }

    public struct AutomaticContributionHeld: Codable, Equatable, Sendable {
        public let heldSessions: Int?
        public let reasons: [String]?
        public let projects: [HeldProject]?

        public enum CodingKeys: String, CodingKey {
            case heldSessions = "held_sessions"
            case reasons, projects
        }

        public struct HeldProject: Codable, Equatable, Sendable {
            public let projectId: String
            public let projectLabel: String?
            public let heldSessions: Int?

            public enum CodingKeys: String, CodingKey {
                case projectId = "project_id"
                case projectLabel = "project_label"
                case heldSessions = "held_sessions"
            }
        }
    }
}

// MARK: - Preview summaries

extension DaemonData {
    /// `preview`, and the `summary` of a ready `preview_request` /
    /// `preview_ready`.
    public struct PreviewSummary: Codable, Equatable, Sendable {
        public let entry: QueueEntry?
        /// the first line of the redacted opening prompt, cut to 60
        /// characters; `nil` when there is no task description. Served only
        /// in a summary, never in `list_pending`.
        public let title: String?
        public let wouldSendBytes: Int?
        public let rawSessionBytes: Int?
        public let eventCount: Int?
        public let openingPrompt: String?
        public let redactions: [String: Int]?
        public let piiLabelsPresent: [String]?
        public let consentScopes: [String]?
        public let residualRisk: String?
        public let envelopeDigest: String?
        public let inputFingerprint: String?
        public let enrolled: Bool?
        public let subagentCount: Int?
        public let subagentsDropped: Int?
        public let scrub: String?
        public let marks: Int?
        public let contentMarks: Int?
        /// how many spans `preview_unsure_spans` would report.
        public let unsureSpans: Int?
        public let secondLook: [String]?

        public enum CodingKeys: String, CodingKey, CaseIterable {
            case entry, title, redactions, enrolled, scrub, marks
            case wouldSendBytes = "would_send_bytes"
            case rawSessionBytes = "raw_session_bytes"
            case eventCount = "event_count"
            case openingPrompt = "opening_prompt"
            case piiLabelsPresent = "pii_labels_present"
            case consentScopes = "consent_scopes"
            case residualRisk = "residual_risk"
            case envelopeDigest = "envelope_digest"
            case inputFingerprint = "input_fingerprint"
            case subagentCount = "subagent_count"
            case subagentsDropped = "subagents_dropped"
            case contentMarks = "content_marks"
            case unsureSpans = "unsure_spans"
            case secondLook = "second_look"
        }
    }

    /// `preview_unsure_spans`: offsets and labels into the body
    /// `preview_body` returned, never the text.
    public struct UnsureSpans: Codable, Equatable, Sendable {
        public let entryId: String
        public let bodyDigest: String
        public let envelopeDigest: String?
        /// Always the total, even when `spans` was cut.
        public let spanCount: Int
        public let spans: [UnsureSpan]
        /// `true` when `spans` is not the whole list: never present it as whole.
        public let spansTruncated: Bool

        public enum CodingKeys: String, CodingKey {
            case entryId = "entry_id"
            case bodyDigest = "body_digest"
            case envelopeDigest = "envelope_digest"
            case spanCount = "span_count"
            case spans
            case spansTruncated = "spans_truncated"
        }
    }

    public struct UnsureSpan: Codable, Equatable, Sendable {
        /// `looks-like-email`, `looks-like-phone`, `looks-like-key`.
        public let label: String
        /// UTF-8 byte offsets, half-open.
        public let byteOffset: Int
        public let byteLen: Int

        public enum CodingKeys: String, CodingKey {
            case label
            case byteOffset = "byte_offset"
            case byteLen = "byte_len"
        }
    }
}

// MARK: - Settings

extension DaemonData {
    /// `get_settings`, and what `set_settings` answers. Only the keys a
    /// screen draws; credential presence stays in the existing models.
    public struct Settings: Codable, Equatable, Sendable {
        public let quiescenceSecs: Int?
        public let approvalHoldSecs: Int?
        // digest and notifications.
        public let digestIntervalSecs: Int?
        public let digestSchedule: DigestSchedule?
        public let localNotifications: Bool?
        // `automatic` or `manual`; see `scrubCheckMode`.
        public let scrubCheck: String?
        public let maxUploadsPerDay: Int?
        public let maxBytesPerDay: Int?
        public let privateInference: Bool?
        /// `unset`, `off` or `watch` per tool. `unset` is never drawn as off.
        public let claudeSourceMode: String?
        public let codexSourceMode: String?
        public let geminiSourceMode: String?
        public let clineSourceMode: String?
        public let opencodeSourceMode: String?

        public enum CodingKeys: String, CodingKey, CaseIterable {
            case quiescenceSecs = "quiescence_secs"
            case approvalHoldSecs = "approval_hold_secs"
            case digestIntervalSecs = "digest_interval_secs"
            case digestSchedule = "digest_schedule"
            case localNotifications = "local_notifications"
            case scrubCheck = "scrub_check"
            case maxUploadsPerDay = "max_uploads_per_day"
            case maxBytesPerDay = "max_bytes_per_day"
            case privateInference = "private_inference"
            case claudeSourceMode = "claude_source_mode"
            case codexSourceMode = "codex_source_mode"
            case geminiSourceMode = "gemini_source_mode"
            case clineSourceMode = "cline_source_mode"
            case opencodeSourceMode = "opencode_source_mode"
        }

        public var scrubCheckMode: ScrubCheckMode? { scrubCheck.flatMap(ScrubCheckMode.init(rawValue:)) }
    }

    public enum ScrubCheckMode: String, Codable, Sendable, CaseIterable {
        case automatic, manual
    }

    /// `{"mode":"interval"}` or `{"mode":"evening","hour":H}`.
    public struct DigestSchedule: Codable, Equatable, Sendable {
        public let mode: String
        /// Local hour 0-23, `evening` only.
        public let hour: Int?

        public init(mode: String, hour: Int? = nil) {
            self.mode = mode
            self.hour = hour
        }

        public static let interval = DigestSchedule(mode: "interval")
        public static func evening(hour: Int) -> DigestSchedule { DigestSchedule(mode: "evening", hour: hour) }
    }
}

// MARK: - Projects

extension DaemonData {
    /// `list_projects`: the rows plus the top-level `unpurposed_traces`.
    public struct ProjectList: Decodable, Equatable, Sendable {
        public let projects: [ProjectRow]
        /// Previewed Ask-me sessions nobody has decided. `nil` from an older daemon.
        public let unpurposedTraces: Int?

        public enum CodingKeys: String, CodingKey {
            case projects
            case unpurposedTraces = "unpurposed_traces"
        }
    }

    /// `set_project_mode`.
    public struct ProjectModeResult: Codable, Equatable, Sendable {
        public let ok: Bool?
        /// Waiting cards removed. Compare against this, never `retracted`.
        public let purged: Int?
        public let retracted: Int?
        public let fromNow: Bool?

        public enum CodingKeys: String, CodingKey {
            case ok, purged, retracted
            case fromNow = "from_now"
        }
    }
}

// MARK: - Queue actions

extension DaemonData {
    /// `approve` for one entry. Only what a screen draws; `skipped[]` and
    /// the group fields stay in `ApproveResponse`.
    public struct ApproveResult: Codable, Equatable, Sendable {
        public let approved: Int?
        public let holdSecs: Int?
        /// `nil` when the hold is off: offer no undo.
        public let holdUntil: Date?

        public enum CodingKeys: String, CodingKey {
            case approved
            case holdSecs = "hold_secs"
            case holdUntil = "hold_until"
        }
    }

    /// `keep` (`kept: true`) and `undo_keep` (`kept: false`).
    public struct KeepResult: Codable, Equatable, Sendable {
        public let kept: Bool
    }
}

// MARK: - History

extension DaemonData {
    /// One `list_history` row.
    public struct HistoryRow: Codable, Equatable, Sendable, Identifiable {
        public let submissionId: String
        public let submittedAt: Date?
        public let projectId: String?
        public let projectLabel: String?
        public let source: String?
        public let status: String?
        public let consentScopes: [String]?
        public let creditPointsPending: Double?
        public let creditPointsFinal: Double?
        public let explanations: [String]?
        public let lastRefreshedAt: Date?
        public let withdrawnAt: Date?
        /// Provenance: `true` armed, `false` a person, `nil` NOT RECORDED.
        public let approvedUnattended: Bool?
        /// `worked` / `partly` / `failed`, or `nil` when none was given.
        public let approvedVerdict: String?

        public var id: String { submissionId }

        public enum CodingKeys: String, CodingKey, CaseIterable {
            case submissionId = "submission_id"
            case submittedAt = "submitted_at"
            case projectId = "project_id"
            case projectLabel = "project_label"
            case source, status, explanations
            case consentScopes = "consent_scopes"
            case creditPointsPending = "credit_points_pending"
            case creditPointsFinal = "credit_points_final"
            case lastRefreshedAt = "last_refreshed_at"
            case withdrawnAt = "withdrawn_at"
            case approvedUnattended = "approved_unattended"
            case approvedVerdict = "approved_verdict"
        }

        /// Render `.notRecorded` as "not recorded", never as "you approved".
        public var provenance: Provenance {
            switch approvedUnattended {
            case .some(true): return .armed
            case .some(false): return .personApproved
            case .none: return .notRecorded
            }
        }
    }

    public enum Provenance: Sendable, Equatable {
        case armed, personApproved, notRecorded
    }

    struct HistoryList: Decodable { let history: [HistoryRow] }

    /// `history_rollup`.
    public struct HistoryRollup: Codable, Equatable, Sendable {
        public let week: HistoryCounts?
        public let month: HistoryCounts?
        public let allTime: HistoryCounts?
        public let creditPending: Double?
        public let creditFinal: Double?
        /// Held for privacy review, NOT rejected; render separately.
        public let quarantined: Int?
        /// the taken-back count.
        public let takenBack: Int?
        public let lastRefreshedAt: Date?
        /// Absent means no standing: draw no community section.
        public let community: Community?

        public enum CodingKeys: String, CodingKey {
            case week, month, quarantined, community
            case allTime = "all_time"
            case creditPending = "credit_pending"
            case creditFinal = "credit_final"
            case takenBack = "taken_back"
            case lastRefreshedAt = "last_refreshed_at"
        }
    }

    public struct HistoryCounts: Codable, Equatable, Sendable {
        public let submitted: Int?
        public let accepted: Int?
        public let quarantined: Int?
        public let withdrawn: Int?
        public let other: Int?
    }

    public struct Community: Codable, Equatable, Sendable {
        /// May be `nil` inside a present object: draw a dash.
        public let rank: Int?
        public let noveltyCredit: Double?
        public let acceptedInWindow: Int?
        public let acceptRate: Double?
        public let windowLabel: String?
        public let publicSince: Date?
        public let snapshotAt: Date?
        public let analyticsWithheld: Bool?

        public enum CodingKeys: String, CodingKey {
            case rank
            case noveltyCredit = "novelty_credit"
            case acceptedInWindow = "accepted_in_window"
            case acceptRate = "accept_rate"
            case windowLabel = "window_label"
            case publicSince = "public_since"
            case snapshotAt = "snapshot_at"
            case analyticsWithheld = "analytics_withheld"
        }
    }

    /// `commons_credit_summary`: the commons' own figures, never near.ai's.
    /// Every field of an `unknown` half is `nil`, which is never a zero
    /// balance and never "settlement is off".
    public struct CommonsCreditSummary: Codable, Equatable, Sendable {
        /// `known` or `unknown`.
        public let postureState: String
        /// Settlement posture: `http`, `dry_run`, `disabled`.
        public let commonsSettlement: String?
        public let commonsSettlementExplanation: String?
        public let commonsGraded: Bool?
        public let pointsState: String
        public let commonsPointsEarnedThisPeriod: Int?
        public let commonsPointsLifetimeEarned: Int?
        public let commonsPendingReview: Int?
        public let commonsCurrencyCode: String?
        public let commonsCurrencyEarnedThisPeriod: String?
        public let commonsPeriodStart: Date?
        public let commonsPeriodEnd: Date?
        public let observedAt: Date?
        public let credentialWarning: String?

        public enum CodingKeys: String, CodingKey, CaseIterable {
            case postureState = "posture_state"
            case commonsSettlement = "commons_settlement"
            case commonsSettlementExplanation = "commons_settlement_explanation"
            case commonsGraded = "commons_graded"
            case pointsState = "points_state"
            case commonsPointsEarnedThisPeriod = "commons_points_earned_this_period"
            case commonsPointsLifetimeEarned = "commons_points_lifetime_earned"
            case commonsPendingReview = "commons_pending_review"
            case commonsCurrencyCode = "commons_currency_code"
            case commonsCurrencyEarnedThisPeriod = "commons_currency_earned_this_period"
            case commonsPeriodStart = "commons_period_start"
            case commonsPeriodEnd = "commons_period_end"
            case observedAt = "observed_at"
            case credentialWarning = "credential_warning"
        }

        public var postureKnown: Bool { postureState == "known" }
        public var pointsKnown: Bool { pointsState == "known" }
    }
}

// MARK: - The map and the Inference tab

extension DaemonData {
    /// `tool_destinations`.
    public struct ToolDestinations: Codable, Equatable, Sendable {
        public let privateAi: String?
        /// `witness`, `local`, `witness_refusing`, `not_enrolled`, `settings_unreadable`.
        public let sessionsRoute: String?
        public let folders: FolderCounts?
        public let tools: [ToolDestination]

        public enum CodingKeys: String, CodingKey {
            case folders, tools
            case privateAi = "private_ai"
            case sessionsRoute = "sessions_route"
        }
    }

    public struct FolderCounts: Codable, Equatable, Sendable {
        public let armed: Int?
        public let askFirst: Int?
        public let ignored: Int?

        public enum CodingKeys: String, CodingKey {
            case armed, ignored
            case askFirst = "ask_first"
        }
    }

    public struct ToolDestination: Codable, Equatable, Sendable, Identifiable {
        public let tool: String
        public let name: String?
        public let sessions: SessionRoute?
        public let modelCalls: ModelCallRoute?

        public var id: String { tool }

        public enum CodingKeys: String, CodingKey {
            case tool, name, sessions
            case modelCalls = "model_calls"
        }
    }

    public struct SessionRoute: Codable, Equatable, Sendable {
        /// `watched`, `off`, `not_watched`.
        public let watch: String
        public let to: [String]
    }

    public struct ModelCallRoute: Codable, Equatable, Sendable {
        /// `near_ai`, a vendor label, or `unknown`.
        public let to: String
        /// `observed`, `configured` (NOT confirmed), `answered_elsewhere`,
        /// `tool_default`, `unknown`.
        public let basis: String
    }

    /// One page of `inference_calls`.
    public struct InferenceCallPage: Codable, Equatable, Sendable {
        /// `false`: no ledger answered. Not evidence of no calls; never an
        /// empty table.
        public let readable: Bool
        public let windowHours: Int?
        public let calls: [InferenceCall]
        public let nextCursor: String?

        public enum CodingKeys: String, CodingKey {
            case readable, calls
            case windowHours = "window_hours"
            case nextCursor = "next_cursor"
        }
    }

    public struct InferenceCall: Codable, Equatable, Sendable, Identifiable {
        public let id: Int64
        public let at: Date
        public let tool: String
        public let family: String
        /// Free text from a proxy the contributor can patch.
        public let model: String
        /// `routed`, `outside`, `unknown`.
        public let route: String
        public let cost: PricedCost?
        /// IronWire's proof label, passed through. See `proofLabel`.
        public let proof: String

        public var proofLabel: ProofLabel { ProofLabel(rawValue: proof) ?? .unrecorded }
    }

    public struct PricedCost: Codable, Equatable, Sendable {
        /// `false` is not zero.
        public let known: Bool
        /// Priced, not billed: never money spent.
        public let pricedMicros: Int64?

        public enum CodingKeys: String, CodingKey {
            case known
            case pricedMicros = "priced_micros"
        }
    }

    /// The proof labels `inference_calls` passes through. Only `verified`
    /// is proof, and `failed` is kept distinct.
    public enum ProofLabel: String, Sendable, CaseIterable {
        case verified
        case gatewayOnly = "gateway_only"
        case unattested, pending, unavailable, failed, outside, unrecorded

        public var isProof: Bool { self == .verified }
    }
}

// MARK: - Network methods that do not exist yet (Zaki's C3)

extension DaemonData {
    /// Z1.1: per-model calls, cost and proof counts from IronWire's
    /// `/_ironwire/summary`, which nothing reads today.
    // PROVISIONAL: shape owned by Zaki's C3
    public struct InferenceSummary: Codable, Equatable, Sendable {
        /// Same meaning as `InferenceCallPage.readable`.
        public let readable: Bool
        public let windowHours: Int?
        public let models: [ModelSummary]

        public enum CodingKeys: String, CodingKey {
            case readable, models
            case windowHours = "window_hours"
        }
    }

    // PROVISIONAL: shape owned by Zaki's C3
    public struct ModelSummary: Codable, Equatable, Sendable, Identifiable {
        public let model: String
        public let family: String?
        public let calls: Int?
        /// Priced, not billed (see `PricedCost`).
        public let pricedMicros: Int64?
        /// Calls per `ProofLabel` raw value.
        public let proofCounts: [String: Int]?

        public var id: String { model }

        public enum CodingKeys: String, CodingKey {
            case model, family, calls
            case pricedMicros = "priced_micros"
            case proofCounts = "proof_counts"
        }
    }

    /// Z1.2: `inference_call_proof`, a proof's detail.
    // PROVISIONAL: shape owned by Zaki's C3
    public struct InferenceProofDetail: Codable, Equatable, Sendable {
        public let callId: Int64
        public let proof: String
        public let checkedAt: Date?
        /// Fixed labels describing what was checked; the copy comes from the core.
        public let checks: [String]?

        public enum CodingKeys: String, CodingKey {
            case proof, checks
            case callId = "call_id"
            case checkedAt = "checked_at"
        }
    }

    /// Z1.3: billed spend per model. Today only `harness_list.spend` (a day's
    /// total) is billed.
    // PROVISIONAL: shape owned by Zaki's C3
    public struct ModelSpend: Codable, Equatable, Sendable {
        /// `false` is not zero.
        public let known: Bool
        public let since: Date?
        public let models: [ModelBilled]
    }

    // PROVISIONAL: shape owned by Zaki's C3
    public struct ModelBilled: Codable, Equatable, Sendable {
        public let model: String
        public let billedMicros: Int64?

        public enum CodingKeys: String, CodingKey {
            case model
            case billedMicros = "billed_micros"
        }
    }

    /// Z1.5: the Private AI on/off switch, with its disclosure from the core.
    // PROVISIONAL: shape owned by Zaki's C3
    public struct PrivateAISwitch: Codable, Equatable, Sendable {
        /// `nil` when the daemon cannot say.
        public let on: Bool?
        /// The `private_inference_state.state` label.
        public let state: String?
        public let disclosure: String?
    }

    /// Z2.2: the mission catalogue, the same request for every contributor.
    // PROVISIONAL: shape owned by Zaki's C3
    public struct MissionCatalogue: Codable, Equatable, Sendable {
        public let missions: [Mission]
        public let fetchedAt: Date?
        /// Settlement posture, so pending credit carries its condition (D8).
        public let posture: CreditPosture?

        public enum CodingKeys: String, CodingKey {
            case missions, posture
            case fetchedAt = "fetched_at"
        }
    }

    // PROVISIONAL: shape owned by Zaki's C3
    public struct Mission: Codable, Equatable, Sendable, Identifiable {
        public let id: String
        public let title: String
        public let summary: String?
        /// Always pending, never earned (D8).
        public let creditRange: CreditRange?

        public enum CodingKeys: String, CodingKey {
            case id, title, summary
            case creditRange = "credit_range"
        }
    }

    /// Mirrors `trace_commons_protocol::CreditPosture`.
    // PROVISIONAL: shape owned by Zaki's C3
    public struct CreditPosture: Codable, Equatable, Sendable {
        /// `http`, `dry_run`, or `disabled`.
        public let settlement: String
        public let graded: Bool
        public let explanation: String
    }

    /// Mirrors `trace_commons_protocol::invite_lookup::CreditRange`.
    // PROVISIONAL: shape owned by Zaki's C3
    public struct CreditRange: Codable, Equatable, Sendable {
        public let min: Int
        public let max: Int
        /// `points` today.
        public let unit: String
    }

    /// Z3.1: invite lookup, mirroring the server's `InviteLookupResponse`.
    /// The pay range is pending credit (D8).
    // PROVISIONAL: shape owned by Zaki's C3
    public struct InviteLookup: Codable, Equatable, Sendable {
        public let valid: Bool
        public let issuerDisplayName: String?
        public let creditRange: CreditRange?
        public let reasonLabel: String?

        public enum CodingKeys: String, CodingKey {
            case valid
            case issuerDisplayName = "issuer_display_name"
            case creditRange = "credit_range"
            case reasonLabel = "reason_label"
        }
    }

    /// Z3.2: passkey binding state.
    // PROVISIONAL: shape owned by Zaki's C3
    public struct PasskeyState: Codable, Equatable, Sendable {
        /// A fixed label, for example `none`, `bound`.
        public let state: String
        public let passkeyCount: Int?
        public let nearAiConnected: Bool?

        public enum CodingKeys: String, CodingKey {
            case state
            case passkeyCount = "passkey_count"
            case nearAiConnected = "near_ai_connected"
        }
    }

    /// Z3.4: `account_session_status`.
    // PROVISIONAL: shape owned by Zaki's C3
    public struct AccountState: Codable, Equatable, Sendable {
        public let signedIn: Bool?
        public let accountId: String?

        public enum CodingKeys: String, CodingKey {
            case signedIn = "signed_in"
            case accountId = "account_id"
        }
    }
}
