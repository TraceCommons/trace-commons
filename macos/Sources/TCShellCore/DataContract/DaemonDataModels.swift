import CryptoKit
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
/// The network methods (C3, #1187) decode into the shapes the daemon
/// serves. Three of them -- `inference_summary`, `mission_catalogue` and
/// `private_ai` -- live under `Network*` names beside the earlier shapes the
/// Inference and Missions screens were written against (`InferenceSummary`,
/// `MissionCatalogue`, and the settings-backed `PrivateAISwitch`). Types
/// still marked `// PROVISIONAL: shape owned by Zaki's C3` are those earlier
/// shapes; moving the screens to the `Network*` types retires them.
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
    /// The method is not served by this client: the live client has not
    /// routed it, and sends nothing. Today that is the provisional
    /// `inferenceSummary()` and `missionCatalogue()`, whose real replies are
    /// `networkInferenceSummary()` and `networkMissionCatalogue()`. An older
    /// attached daemon that predates a method it is asked for answers
    /// `unknown_method` instead, carried as `.daemon` (see `isNotServed`).
    case notAvailableYet(method: String)
    /// The daemon answered with an IPC error. `code` and `message` are the
    /// fixed labels the contract defines, safe to show.
    case daemon(code: String, message: String)
    /// A single-entry `approve` the daemon answered OK but did not act on:
    /// `approved` is 0 and `skipped` names the entry. `reasonLabel` is the
    /// daemon's fixed `skipped[].reason_label` (`not-pending`,
    /// `not-enrolled`, `witness-review-stale`, ...), safe to show and turned
    /// into words by `SubmitToast.reasonLabel`; `nil` only if the daemon
    /// approved nothing and named no reason. Never drawn as success.
    case notApproved(reasonLabel: String?)
    /// The daemon answered, but not in a shape this build reads.
    /// `codingPath` is where decoding stopped, as `DecodingError` reported
    /// it: keys and array indices only (`pending[2].started_at`), never a
    /// value, so it is safe to log. Empty when the frame itself was not
    /// JSON, or the failure was at the top level.
    case undecodable(method: String, codingPath: String = "")

    public var description: String {
        switch self {
        case .unreachable: return "unreachable"
        case .notAvailableYet(let method): return "not-available-yet: \(method)"
        case .daemon(let code, let message): return "\(code): \(message)"
        case .notApproved(let reason): return "not-approved: \(reason ?? "unknown")"
        case .undecodable(let method, let path):
            return path.isEmpty ? "undecodable: \(method)" : "undecodable: \(method): \(path)"
        }
    }

    /// The method is not there to ask: this client has not routed it
    /// (`notAvailableYet`), or an older attached daemon predates it
    /// (`unknown_method`). Neither is a failure of a method that exists, so
    /// a screen draws it as absent, never as an error.
    public var isNotServed: Bool {
        switch self {
        case .notAvailableYet: return true
        case .daemon(let code, _): return code == "unknown_method"
        default: return false
        }
    }

    /// `.undecodable` for `error`, keeping the coding path a
    /// `DecodingError` carries: the keys and indices down to the field that
    /// failed, never the value it held. Any other error keeps no path.
    public static func undecodable(method: String, from error: any Error) -> DaemonDataError {
        guard let decoding = error as? DecodingError else { return .undecodable(method: method) }
        var path: [any CodingKey]
        switch decoding {
        case .typeMismatch(_, let context), .valueNotFound(_, let context), .dataCorrupted(let context):
            path = context.codingPath
        case .keyNotFound(let key, let context):
            path = context.codingPath + [key]
        @unknown default:
            path = []
        }
        return .undecodable(method: method, codingPath: renderCodingPath(path))
    }

    /// `pending[2].started_at`: string keys joined with dots, integer keys
    /// (array positions) in brackets.
    static func renderCodingPath(_ path: [any CodingKey]) -> String {
        var out = ""
        for key in path {
            if let index = key.intValue {
                out += "[\(index)]"
            } else {
                out += out.isEmpty ? key.stringValue : ".\(key.stringValue)"
            }
        }
        return out
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
        /// The session's title (K9): the first line of its opening prompt,
        /// through the deterministic redaction pass only. On the queue only:
        /// receipts and history rows never carry it. `nil` for an entry
        /// queued before titles existed, or a task with no description.
        public let title: String?
        public let projectPath: String?
        public let sessionPath: String?
        public let sizeBytes: Int?
        /// The pinned preview's measured envelope size (K10), or `nil` when
        /// nothing is pinned. Measured before scope stamping, so it can be a
        /// few bytes short of what an upload sends.
        public let wouldSendBytes: Int?
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
        /// Whether attested inference was inside the witness certificate
        /// `holdsCertificate` says is held. Absent is "not known" (no review,
        /// a review that predates the record, a local preview): draw
        /// nothing, never either answer. Not `attestation`, which is about
        /// the session's last model call.
        public let attestedInference: AttestedInference?
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
        /// The sentence for each `secondLook` reason, index for index (R6/R7;
        /// DRAFT wording). Never shorter than `secondLook`.
        public let secondLookLines: [String]?

        public var id: String { entryId }

        public enum CodingKeys: String, CodingKey, CaseIterable {
            case entryId = "entry_id"
            case sessionHash = "session_hash"
            case source
            case declaredSource = "declared_source"
            case projectId = "project_id"
            case projectLabel = "project_label"
            case title
            case projectPath = "project_path"
            case sessionPath = "session_path"
            case sizeBytes = "size_bytes"
            case wouldSendBytes = "would_send_bytes"
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
            case attestedInference = "attested_inference"
            case attestation
            case attestationReason = "attestation_reason"
            case scrub, marks
            case contentMarks = "content_marks"
            case unsureSpans = "unsure_spans"
            case secondLook = "second_look"
            case secondLookLines = "second_look_lines"
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
        /// waiting for a person even in an armed folder, and left out of a
        /// folder approve: the daemon's `held_for_review`. Manual Scrub check
        /// is not one of these.
        public var heldForReview: Bool { reasonLabel.map(ReasonLabel.needingAPerson.contains) ?? false }
    }

    /// A queue entry's `attested_inference`
    /// (`witness::inference_record::InferenceAttestationRecord`). Labels
    /// only: no model, provider, digest or key.
    public struct AttestedInference: Codable, Equatable, Hashable, Sendable {
        /// `certified` or `uncertified`.
        public let state: String
        /// Why it is uncertified; absent for `certified`.
        public let reason: String?
    }

    public enum ReasonLabel {
        public static let keptOnThisMac = "kept-on-this-mac"
        public static let returnedFromKeep = "returned-from-keep"
        public static let secondLookReviewRequired = "second-look-review-required"
        public static let scrubCheckManual = "scrub-check-manual"
        public static let tokenDistributionReviewRequired = "token-distribution-review-required"
        public static let witnessRiskReviewRequired = "witness-risk-review-required"
        public static let privacyFilterTransientExhausted = "privacy-filter-transient-exhausted"

        /// `queue.rs` `REASONS_NEEDING_A_PERSON`, in its order.
        public static let needingAPerson: Set<String> = [
            tokenDistributionReviewRequired,
            witnessRiskReviewRequired,
            privacyFilterTransientExhausted,
            secondLookReviewRequired,
        ]
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
        /// Opaque account binding; present whenever the daemon can derive a
        /// scope from its store, signed in or not. Not proof of sign-in: read
        /// `loggedIn` for that.
        public let accountScope: String?
        public let consentScopes: [String]?
        /// `consent-scopes-not-chosen` while the daemon sends nothing under
        /// its enrolment because the scopes were saved by enrolment and never
        /// chosen; `nil` otherwise, and from a daemon predating it. A label,
        /// never shown as text.
        public let consentHold: String?
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
        /// Grants the daemon voided (R6) that no shell has shown yet: a ship
        /// condition. `[]` is "nothing to show"; `nil` is a daemon too old
        /// to say, which is not the same and must not be drawn as it. Each
        /// element is kept verbatim for `TCConsentCopy.voidNoticeJSON`.
        public let grantVoids: [GrantVoidWire]?
        /// Whether moving a legacy invite identity is offered, and the
        /// notice after it moved. `nil` when the daemon did not say.
        public let legacyInviteMigration: LegacyInviteMigration?
        /// The menu-bar pill's global override (#1173): `nil` when none is in
        /// force, or from a daemon predating it.
        public let contributionOverride: ContributionOverride?
        /// The pill's roll-up: `notify_only`, `auto_upload`, `ignore` or
        /// `mixed`. Computed by the daemon; a shell never derives it.
        public let contributionMode: String?
        /// `true` when `contributionMode` is `auto_upload` but a Never folder
        /// or the unidentified bucket does not upload (#1208): draw the core's
        /// `auto_partial` line under the label.
        public let contributionModePartial: Bool?
        /// K2 (#1173): `true` while a debug daemon runs a developer dry
        /// run. A release daemon never sends it. The app's notice reads it
        /// rather than the environment.
        public let devDryRun: Bool?

        public enum CodingKeys: String, CodingKey, CaseIterable {
            case schemaVersion = "schema_version"
            case loggedIn = "logged_in"
            case tenantId = "tenant_id"
            case accountScope = "account_scope"
            case consentScopes = "consent_scopes"
            case consentHold = "consent_hold"
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
            case grantVoids = "grant_voids"
            case legacyInviteMigration = "legacy_invite_migration"
            case contributionOverride = "contribution_override"
            case contributionMode = "contribution_mode"
            case contributionModePartial = "contribution_mode_partial"
            case devDryRun = "dev_dry_run"
        }
    }

    /// `status.contribution_override` while one is in force.
    public struct ContributionOverride: Codable, Equatable, Sendable {
        public let mode: String?
        public let since: Date?
    }

    /// `status.legacy_invite_migration` (`legacy_migration::status_value`).
    public struct LegacyInviteMigration: Codable, Equatable, Sendable {
        public let offered: Bool?
        /// The notice after the identity moved, until a shell acknowledges
        /// it. `nil` when there is none to show.
        public let notice: Notice?

        public struct Notice: Codable, Equatable, Sendable {
            public let foldersKept: Bool?
            public let automaticGrantKept: Bool?

            public enum CodingKeys: String, CodingKey {
                case foldersKept = "folders_kept"
                case automaticGrantKept = "automatic_grant_kept"
            }
        }

        /// The notice as `TCConsentCopy.legacyMigrationNoticeJSON` takes it,
        /// re-encoded with the wire's keys, or `nil` when there is none.
        public var noticeJSON: String? {
            guard let notice else { return nil }
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.sortedKeys]
            return (try? encoder.encode(notice)).map { String(decoding: $0, as: UTF8.self) }
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
        /// Ledger rows the daemon could not read at the last refresh.
        public let unreadableRows: Int?

        public enum CodingKeys: String, CodingKey {
            case state, derived
            case lastRefreshAt = "last_refresh_at"
            case unreadableRows = "unreadable_rows"
        }
    }

    public struct PrivateInferenceState: Codable, Equatable, Sendable {
        /// The label `tool_destinations.private_ai` repeats (`running`, ...).
        public let state: String
        public let port: Int?

        public init(state: String, port: Int?) {
            self.state = state
            self.port = port
        }
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
        /// Times each pattern fired, by category.
        public let redactions: [String: Int]?
        /// Distinct VALUES removed, by category: one email address seen ten
        /// times is 10 in `redactions` and 1 here. `{}` when nothing was
        /// removed. Sent only with the FULL summary -- `preview` of an entry
        /// that holds a witness certificate, and `tc_preview_summary_json`
        /// -- never with a card summary (`preview` of any other entry, and
        /// `preview_request` / `preview_ready`), where it is `nil`: unknown,
        /// not zero.
        public let redactionsDistinct: [String: Int]?
        /// Present only on the full preview, never on a card.
        public let tokenDistributionSummary: String?
        public let piiLabelsPresent: [String]?
        public let consentScopes: [String]?
        public let residualRisk: String?
        public let envelopeDigest: String?
        public let inputFingerprint: String?
        /// `false`: this device is not enrolled, the summary describes a
        /// placeholder-identity build, and Contribute will be skipped as
        /// `not-enrolled`. The check before Contribute (R7), with the
        /// entry's `eligibility` (present only when eligibility applies).
        public let enrolled: Bool?
        public let subagentCount: Int?
        public let subagentsDropped: Int?
        public let scrub: String?
        public let marks: Int?
        public let contentMarks: Int?
        /// how many spans `preview_unsure_spans` would report.
        public let unsureSpans: Int?
        public let secondLook: [String]?
        /// The sentence for each `secondLook` reason, index for index (R6/R7;
        /// DRAFT wording).
        public let secondLookLines: [String]?

        public enum CodingKeys: String, CodingKey, CaseIterable {
            case entry, title, redactions, enrolled, scrub, marks
            case wouldSendBytes = "would_send_bytes"
            case rawSessionBytes = "raw_session_bytes"
            case eventCount = "event_count"
            case openingPrompt = "opening_prompt"
            case redactionsDistinct = "redactions_distinct"
            case tokenDistributionSummary = "token_distribution_summary"
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
            case secondLookLines = "second_look_lines"
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
    /// screen draws; the rest are named, with the reason each is left out,
    /// in `DaemonDataKeyCoverageTests.notModeled`, so a key the daemon adds
    /// fails that test until someone decides.
    public struct Settings: Codable, Equatable, Sendable {
        public let quiescenceSecs: Int?
        public let approvalHoldSecs: Int?
        // digest and notifications.
        public let digestIntervalSecs: Int?
        public let digestSchedule: DigestSchedule?
        public let localNotifications: Bool?
        // `automatic` or `manual`; see `scrubCheckMode`.
        public let scrubCheck: String?
        /// The stored Scrub check was unset when Automatic became the
        /// default, so already-armed folders changed behaviour: announce it.
        public let scrubCheckDefaultedOnUpgrade: Bool?
        /// Only on the `set_settings` reply that switched Scrub check to
        /// Manual: how many unsent unattended approvals went back to waiting.
        public let scrubCheckReturnedToWaiting: Int?
        /// Whether attested model-call bodies are sent with a session.
        public let ironwireAttestedBodies: Bool?
        public let maxUploadsPerDay: Int?
        public let maxBytesPerDay: Int?
        public let privateInference: Bool?
        /// Whether the first-run Private AI question has been answered.
        public let privateInferenceOfferSeen: Bool?
        /// The listener's own report; never assumed running.
        public let privateInferenceState: PrivateInferenceState?
        /// `unset`, `off` or `watch` per tool. `unset` is never drawn as off.
        public let claudeSourceMode: String?
        public let codexSourceMode: String?
        public let geminiSourceMode: String?
        public let clineSourceMode: String?
        public let opencodeSourceMode: String?
        /// The declared trajectory folder's mode; the path is never sent.
        public let trajectorySourceMode: String?

        public enum CodingKeys: String, CodingKey, CaseIterable {
            case quiescenceSecs = "quiescence_secs"
            case approvalHoldSecs = "approval_hold_secs"
            case digestIntervalSecs = "digest_interval_secs"
            case digestSchedule = "digest_schedule"
            case localNotifications = "local_notifications"
            case scrubCheck = "scrub_check"
            case scrubCheckDefaultedOnUpgrade = "scrub_check_defaulted_on_upgrade"
            case scrubCheckReturnedToWaiting = "scrub_check_returned_to_waiting"
            case ironwireAttestedBodies = "ironwire_attested_bodies"
            case maxUploadsPerDay = "max_uploads_per_day"
            case maxBytesPerDay = "max_bytes_per_day"
            case privateInference = "private_inference"
            case privateInferenceOfferSeen = "private_inference_offer_seen"
            case privateInferenceState = "private_inference_state"
            case claudeSourceMode = "claude_source_mode"
            case codexSourceMode = "codex_source_mode"
            case geminiSourceMode = "gemini_source_mode"
            case clineSourceMode = "cline_source_mode"
            case opencodeSourceMode = "opencode_source_mode"
            case trajectorySourceMode = "trajectory_source_mode"
        }

        public var scrubCheckMode: ScrubCheckMode? { scrubCheck.flatMap(ScrubCheckMode.init(rawValue:)) }
    }

    /// What `setSource` throws for a choice that is not an answer, sending
    /// nothing: the label the daemon's settings validator gives the same
    /// input (`daemon::settings::ERR_SETTINGS_INVALID_VALUE`).
    static let unansweredSource = DaemonDataError.daemon(code: "bad_params", message: "settings-invalid-value")

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
        /// The contribution override's mode when it still decides this
        /// folder, so the change is saved but not yet in effect (#1208).
        /// `nil` when the folder now resolves to what was set.
        public let overriddenBy: String?

        public enum CodingKeys: String, CodingKey {
            case ok, purged, retracted
            case fromNow = "from_now"
            case overriddenBy = "overridden_by"
        }
    }

    /// `set_contribution_override` (#1173, #1208).
    public struct ContributionOverrideResult: Codable, Equatable, Sendable {
        /// `false` when that override was already in force: nothing was
        /// recorded, and an Automatic override kept its hold.
        public let changed: Bool
        /// The override now in force, `{mode, since}`.
        public let contributionOverride: ContributionOverride?
        /// Unattended approvals not yet sent that went back to waiting.
        public let returned: Int

        public init(changed: Bool, contributionOverride: ContributionOverride?, returned: Int) {
            self.changed = changed
            self.contributionOverride = contributionOverride
            self.returned = returned
        }

        public enum CodingKeys: String, CodingKey {
            case changed, returned
            case contributionOverride = "contribution_override"
        }
    }

    /// `clear_contribution_override`.
    public struct ContributionOverrideClearResult: Codable, Equatable, Sendable {
        /// `false` when no override was in force.
        public let cleared: Bool
        /// Unattended approvals not yet sent that went back to waiting.
        public let returned: Int

        public init(cleared: Bool, returned: Int) {
            self.cleared = cleared
            self.returned = returned
        }
    }
}

// MARK: - Queue actions

extension DaemonData {
    // `approve` answers with `TCShellCore.ApproveResponse`, the shape the
    // existing shell already decodes: `approved`, `flagged`, `redactions`,
    // `skipped[]`, `hold_secs`, `hold_until`, and on a group call
    // `excluded_ineligible` / `excluded_held`. See
    // `DaemonDataClient.approve(entryId:)` for the single-entry rule.

    /// `preview_request`'s answer and the `preview_ready` event's payload:
    /// `entry_id`, `state`, and whatever that state carries.
    public typealias PreviewRequestOutcome = PreviewRequestResult<PreviewSummary>

    /// `preview_cancel`.
    public struct PreviewCancelResult: Codable, Equatable, Sendable {
        public let entryId: String
        /// `false` is "nothing to drop" (finished, cancelled, never asked),
        /// not an error.
        public let dropped: Bool

        public enum CodingKeys: String, CodingKey {
            case entryId = "entry_id"
            case dropped
        }
    }

    /// `preview_visible`: how many ids the daemon now treats as on screen.
    struct PreviewVisibleResult: Decodable {
        let visible: Int
    }

    /// `cancel` for one entry (and other methods that answer only `ok`).
    struct OkReply: Decodable {
        let ok: Bool?
    }

    /// `cancel` with `project_id`.
    struct CancelFolderResult: Decodable {
        let canceled: Int
    }

    /// `keep` (`kept: true`) and `undo_keep` (`kept: false`).
    public struct KeepResult: Codable, Equatable, Sendable {
        public let kept: Bool
    }
}

// MARK: - Events

extension DaemonData {
    /// The `digest_due` event's payload. Every field is optional because
    /// an older daemon sends only `pending` and `text`: absent is unknown,
    /// never 0. `text` is the core's own sentence; a shell that words its
    /// notification itself does so from the counts and labels here.
    public struct DigestDue: Codable, Equatable, Sendable {
        public let pending: Int?
        /// Sessions that went without the contributor since the last digest.
        public let contributed: Int?
        /// The folders they went from, as labels, never keys.
        public let contributedProjects: [String]?
        /// Pending credit for them; pending, never earned.
        public let creditPending: Double?
        public let text: String?

        public init(
            pending: Int?, contributed: Int? = nil, contributedProjects: [String]? = nil,
            creditPending: Double? = nil, text: String?
        ) {
            self.pending = pending
            self.contributed = contributed
            self.contributedProjects = contributedProjects
            self.creditPending = creditPending
            self.text = text
        }

        public enum CodingKeys: String, CodingKey {
            case pending, contributed, text
            case contributedProjects = "contributed_projects"
            case creditPending = "credit_pending"
        }
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
        /// When a web withdrawal was first seen (K12). `nil` when not revoked,
        /// revoked before the field existed, or withdrawn from this device.
        public let revokedAt: Date?
        /// The bytes this submission actually sent (K10), or `nil`.
        public let uploadedBytes: Int?
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
            case revokedAt = "revoked_at"
            case uploadedBytes = "uploaded_bytes"
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
        /// The window `counts` cover, in hours (K14): the same 24 hours
        /// `inference_calls` uses.
        public let windowHours: Int?
        /// The window's listed calls no tool can be named for (K14). `nil`
        /// when no ledger answered -- not zero.
        public let unattributedCalls: Int?
        /// Whether the ledger answered. Missing on older daemons is unknown,
        /// not a readable ledger with zero destinations.
        public let ledgerReadable: Bool?
        public let observedDestinations: [ObservedDestination]?
        public let hub: InferenceHub?

        public enum CodingKeys: String, CodingKey {
            case folders, tools, hub
            case privateAi = "private_ai"
            case sessionsRoute = "sessions_route"
            case windowHours = "window_hours"
            case unattributedCalls = "unattributed_calls"
            case ledgerReadable = "ledger_readable"
            case observedDestinations = "observed_destinations"
        }
    }

    /// A route observed in the ledger. `to` can remain `unknown` even when
    /// the route and local proxy are known; never infer a provider from it.
    public struct ObservedDestination: Codable, Equatable, Sendable {
        public let route: String
        public let to: String
        public let via: String
        public let basis: String
    }

    /// The local proxy's reported state. Configuration is not proof that a
    /// provider answered a call, and nullable ownership remains unknown.
    public struct InferenceHub: Codable, Equatable, Sendable {
        public let kind: String
        /// Missing or null when settings could not be read.
        public let state: String?
        public let port: Int?
        public let owned: Bool?
        public let basis: String
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
        /// The map's per-tool node counts over `windowHours` (K14).
        public let counts: ToolCounts?

        public var id: String { tool }

        public enum CodingKeys: String, CodingKey {
            case tool, name, sessions, counts
            case modelCalls = "model_calls"
        }
    }

    /// One tool's counts for the window. `nil` is "could not be read",
    /// never zero.
    public struct ToolCounts: Codable, Equatable, Sendable {
        /// Sessions, once per session hash, under the tool each reads as.
        public let sessions: Int?
        /// Exactly the `inference_calls` rows naming this tool.
        public let inferenceCalls: Int?

        public enum CodingKeys: String, CodingKey {
            case sessions
            case inferenceCalls = "inference_calls"
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

    /// An `inference_call_added` event (`inference_map::call_added`). A
    /// pulse, not a row: it carries no time, family, route or cost, so a
    /// screen re-reads `inference_calls` and `tool_destinations` rather
    /// than adding it to what it holds.
    public struct InferenceCallAdded: Codable, Equatable, Sendable {
        public let id: Int64
        public let tool: String
        public let model: String
        public let proof: String

        public init(id: Int64, tool: String, model: String, proof: String) {
            self.id = id
            self.tool = tool
            self.model = model
            self.proof = proof
        }

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

// MARK: - Network methods (C3, #1187)

extension DaemonData {
    /// `inference_summary` as #1187 serves it: IronWire's upstream grouped
    /// summary in a `readable` wrapper. Unreadable is never an observed empty
    /// window. Beside the provisional `InferenceSummary`, which the Inference
    /// screen still reads; moving the screen here is a follow-up.
    public struct NetworkInferenceSummary: Codable, Equatable, Sendable {
        public let readable: Bool
        public let windowHours: Int?
        public let observedAt: Date?
        public let summary: InferenceSummaryView?

        public enum CodingKeys: String, CodingKey {
            case readable, summary
            case windowHours = "window_hours"
            case observedAt = "observed_at"
        }
    }

    /// The Cargo-pinned IronWire `/_ironwire/summary` reply, unchanged.
    public struct InferenceSummaryView: Codable, Equatable, Sendable {
        /// Disabled capture is not an observed empty window.
        public let enabled: Bool
        /// The receipts setting, not a claim that every call is verified.
        public let receipts: Bool
        public let since: Date
        public let groups: [InferenceSummaryGroup]
        public let routed: InferenceRouteTotal
        public let outside: InferenceRouteTotal
        public let unknown: InferenceRouteTotal
    }

    public struct InferenceSummaryGroup: Codable, Equatable, Sendable, Identifiable {
        /// Digest of the original source grouping tuple, before label sanitization.
        /// Absent only for older C3 daemon replies.
        public let groupId: String?
        public let model: String?
        /// A backend label, not verified provider identity.
        public let backend: String
        public let route: String
        /// No classification source exists today; this remains nil.
        public let workKind: String?
        public let calls: UInt64
        public let pricedCalls: UInt64
        /// Registry-priced USD over priced calls only, never billed spend.
        public let costUSD: Double
        public let proof: InferenceProofCounts

        public enum GroupID: Hashable, Sendable {
            case source(String)
            case legacy(model: String?, backend: String, route: String, workKind: String?)
        }

        public var id: GroupID {
            if let groupId { return .source(groupId) }
            return .legacy(model: model, backend: backend, route: route, workKind: workKind)
        }

        public init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            groupId = try values.decodeIfPresent(String.self, forKey: .groupId)
            if let groupId {
                guard groupId.hasPrefix("sha256:"), groupId.utf8.count == 71,
                      groupId.dropFirst(7).utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) })
                else { throw DecodingError.dataCorruptedError(forKey: .groupId, in: values, debugDescription: "summary-group-id-invalid") }
            }
            model = try values.decodeIfPresent(String.self, forKey: .model)
            backend = try values.decode(String.self, forKey: .backend)
            route = try values.decode(String.self, forKey: .route)
            workKind = try values.decodeIfPresent(String.self, forKey: .workKind)
            calls = try values.decode(UInt64.self, forKey: .calls)
            pricedCalls = try values.decode(UInt64.self, forKey: .pricedCalls)
            costUSD = try values.decode(Double.self, forKey: .costUSD)
            proof = try values.decode(InferenceProofCounts.self, forKey: .proof)
        }

        public enum CodingKeys: String, CodingKey {
            case model, backend, route, calls, proof
            case groupId = "group_id"
            case workKind = "work_kind"
            case pricedCalls = "priced_calls"
            case costUSD = "cost_usd"
        }
    }

    public struct InferenceRouteTotal: Codable, Equatable, Sendable {
        public let calls: UInt64
        public let pricedCalls: UInt64
        /// Registry-priced, not billed. `pricedCalls < calls` is incomplete pricing.
        public let costUSD: Double
        public let proof: InferenceProofCounts

        public enum CodingKeys: String, CodingKey {
            case calls, proof
            case pricedCalls = "priced_calls"
            case costUSD = "cost_usd"
        }
    }

    /// Only `verified` is model proof; failed and gateway-only remain distinct.
    public struct InferenceProofCounts: Codable, Equatable, Sendable {
        public let verified: UInt64
        public let gatewayOnly: UInt64
        public let unattested: UInt64
        public let pending: UInt64
        public let unavailable: UInt64
        public let failed: UInt64
        public let outside: UInt64
        public let unrecorded: UInt64

        public enum CodingKeys: String, CodingKey {
            case verified, unattested, pending, unavailable, failed, outside, unrecorded
            case gatewayOnly = "gateway_only"
        }
    }

    /// `inference_call_proof`: a stored label, not a new verification.
    public struct InferenceProofDetail: Codable, Equatable, Sendable {
        public let callId: Int64
        public let proof: String
        /// The current source records neither timestamp nor detailed checks.
        public let checkedAt: Date?
        public let checks: [String]?
        public let readable: Bool
        public let found: Bool

        public enum CodingKeys: String, CodingKey {
            case proof, checks, readable, found
            case callId = "call_id"
            case checkedAt = "checked_at"
        }
    }

    /// Provider-reported usage cost for the entire NEAR AI organization, including
    /// other devices. It is never this Mac's registry-priced inference cost.
    public struct ModelSpend: Codable, Equatable, Sendable {
        public let known: Bool
        public let scope: String?
        public let source: String?
        public let currency: String?
        public let scale: Int?
        public let windowHours: Int?
        public let since: Date?
        public let observedAt: Date?
        public let models: [ModelBilled]
        public let reasonLabel: String?
        public let state: String?

        public enum CodingKeys: String, CodingKey {
            case known, scope, source, currency, scale, since, models, state
            case windowHours = "window_hours"
            case observedAt = "observed_at"
            case reasonLabel = "reason_label"
        }

        public init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            known = try values.decode(Bool.self, forKey: .known)
            scope = try values.decodeIfPresent(String.self, forKey: .scope)
            source = try values.decodeIfPresent(String.self, forKey: .source)
            currency = try values.decodeIfPresent(String.self, forKey: .currency)
            scale = try values.decodeIfPresent(Int.self, forKey: .scale)
            windowHours = try values.decodeIfPresent(Int.self, forKey: .windowHours)
            since = try values.decodeIfPresent(Date.self, forKey: .since)
            observedAt = try values.decodeIfPresent(Date.self, forKey: .observedAt)
            models = try values.decode([ModelBilled].self, forKey: .models)
            reasonLabel = try values.decodeIfPresent(String.self, forKey: .reasonLabel)
            state = try values.decodeIfPresent(String.self, forKey: .state)
            if known {
                guard scope == "near_ai_organization", source == "near_ai_usage_by_model",
                      currency == "USD", scale == 9, windowHours == 24,
                      let since, let observedAt, since <= observedAt,
                      models.allSatisfy({ row in
                          row.billedNanos >= 0 && row.billedMicros >= 0 && row.calls >= 0
                              && row.rounding == "nearest_micro_half_up"
                              && row.billedMicros == row.billedNanos / 1_000 + (row.billedNanos % 1_000 >= 500 ? 1 : 0)
                      })
                else { throw DecodingError.dataCorruptedError(forKey: .known, in: values, debugDescription: "model-spend-source-invalid") }
            } else if !models.isEmpty {
                throw DecodingError.dataCorruptedError(forKey: .models, in: values, debugDescription: "unknown-model-spend-has-rows")
            }
        }
    }

    public struct ModelBilled: Codable, Equatable, Sendable {
        public let model: String
        /// Exact nonnegative USD nanos from the provider, not a registry estimate.
        public let billedNanos: Int64
        /// Explicit nearest-micro half-up projection for display only.
        public let billedMicros: Int64
        public let calls: Int64
        public let rounding: String

        public enum CodingKeys: String, CodingKey {
            case model, calls, rounding
            case billedNanos = "billed_nanos"
            case billedMicros = "billed_micros"
        }
    }

    /// `private_ai` as #1187 serves it, with the core's disclosure. Requested
    /// setting and actual owned-proxy state are separate facts. Beside the
    /// settings-backed `PrivateAISwitch` the screens use today.
    public struct NetworkPrivateAISwitch: Codable, Equatable, Sendable {
        public let on: Bool?
        public let state: String?
        public let port: Int?
        /// Supplied by the Rust core; Swift does not author release consent copy.
        public let disclosure: String
    }

    /// What `setNetworkPrivateAI(on: true, ...)` needs: an acknowledgement that
    /// can only be built from a `NetworkPrivateAISwitch` the core answered, so a
    /// caller cannot enable Private AI without first holding the switch
    /// whose `disclosure` it shows. Carries the SHA-256 of that disclosure,
    /// lowercase hex. The daemon does not check the digest yet (#1187's
    /// `set_private_ai` takes `confirmed` only); binding it there is a
    /// protocol change left to a follow-up.
    public struct PrivateAIConsent: Equatable, Sendable {
        public let disclosureSHA256: String

        /// `nil` for a switch with no disclosure: there is nothing to
        /// acknowledge, so there is no consent.
        public init?(acknowledging shown: NetworkPrivateAISwitch) {
            guard !shown.disclosure.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
            disclosureSHA256 = SHA256.hash(data: Data(shown.disclosure.utf8))
                .map { String(format: "%02x", $0) }.joined()
        }
    }

    /// `mission_catalogue` as #1187 serves it: public skill-evaluation
    /// packages, with no daily rewards. Beside the provisional
    /// `MissionCatalogue`, which the Missions screen still reads; moving the
    /// screen here is a follow-up.
    public struct NetworkMissionCatalogue: Codable, Equatable, Sendable {
        public let kind: String
        public let catalogue: MissionCatalogPage
        public let disclosure: String
    }

    /// Mirrors protocol `MissionCatalogPage`; entries grant no execution or consent.
    public struct MissionCatalogPage: Codable, Equatable, Sendable {
        public let schemaVersion: UInt32
        public let entries: [MissionCatalogEntry]
        public let nextCursor: String?

        public enum CodingKeys: String, CodingKey {
            case entries
            case schemaVersion = "schema_version"
            case nextCursor = "next_cursor"
        }
    }

    public struct MissionCatalogEntry: Codable, Equatable, Sendable, Identifiable {
        public let missionId: String
        public let programId: String
        public let packageSHA256: String
        public let offerVersionHash: String
        /// Bounded plaintext, not HTML.
        public let taskPreview: String
        public let publishedAt: Date

        public var id: String { missionId }

        public enum CodingKeys: String, CodingKey {
            case missionId = "mission_id"
            case programId = "program_id"
            case packageSHA256 = "package_sha256"
            case offerVersionHash = "offer_version_hash"
            case taskPreview = "task_preview"
            case publishedAt = "published_at"
        }
    }

    /// Mirrors protocol `InviteLookupResponse`.
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

    public struct PasskeyState: Codable, Equatable, Sendable {
        /// `none`, `unknown`, or binding labels `unbound|bound|closed|legacy`.
        public let state: String
        /// The passkeys this Mac remembers, not the account's on the server;
        /// null when the daemon cannot read its list. Connection is never
        /// inferred from a missing binding row.
        public let passkeyCount: Int?
        /// The most recently used remembered passkey's name, if it has one.
        public let rememberedName: String?
        /// The name remembered for the signed-in account's own record, never
        /// the most recent record's; nil when signed out or unnamed.
        public let signedInName: String?
        public let nearAiConnected: Bool?

        public enum CodingKeys: String, CodingKey {
            case state
            case passkeyCount = "passkey_count"
            case rememberedName = "remembered_name"
            case signedInName = "signed_in_name"
            case nearAiConnected = "near_ai_connected"
        }
    }

    /// `account_session_status`: unreadable storage is unknown, never signed out.
    public struct AccountState: Codable, Equatable, Sendable {
        public let state: String
        public let signedIn: Bool?
        public let accountId: String?
        public let expiresAt: Date?

        public enum CodingKeys: String, CodingKey {
            case state
            case signedIn = "signed_in"
            case accountId = "account_id"
            case expiresAt = "expires_at"
        }
    }
}

// MARK: - Shapes the screens still read
//
// The Inference and Missions screens (and `HomeStore` / `InferenceStore`)
// were written against these before #1187 fixed the wire.
// `InferenceSummary` and `MissionCatalogue` are provisional: the live client
// answers their methods with `notAvailableYet`, and the real replies decode
// into `NetworkInferenceSummary` and `NetworkMissionCatalogue` above.
// `PrivateAISwitch` is K1's real switch over `get_settings`/`set_settings`;
// `NetworkPrivateAISwitch` is the same switch over #1187's `private_ai`.
// Moving the screens across and retiring the duplicates is a follow-up.

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

    /// Z1.5: the Private AI on/off switch, as the daemon's settings echo it.
    /// The disclosure words are the core's (`PrivateInferenceCopy.offerExposure`),
    /// read through the bridge in the app target, not carried here.
    public struct PrivateAISwitch: Codable, Equatable, Sendable {
        /// `nil` when the daemon cannot say.
        public let on: Bool?
        /// Whether the first-run question has been answered; a write sets it.
        public let offerSeen: Bool?
        /// The listener's own report (`state`, `port`).
        public let state: PrivateInferenceState?

        public init(on: Bool?, offerSeen: Bool?, state: PrivateInferenceState?) {
            self.on = on
            self.offerSeen = offerSeen
            self.state = state
        }

        public init(settings: Settings) {
            self.init(
                on: settings.privateInference, offerSeen: settings.privateInferenceOfferSeen,
                state: settings.privateInferenceState)
        }
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

    /// Mirrors `trace_commons_protocol::invite_lookup::CreditRange`: an
    /// invite operator's estimated credit per accepted trace, not yet settled.
    public struct CreditRange: Codable, Equatable, Sendable {
        public let min: Int
        public let max: Int
        /// `points_per_accepted_trace`.
        public let unit: String
    }
}
