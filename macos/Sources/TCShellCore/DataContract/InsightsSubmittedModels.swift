import Foundation

/// `insights_submitted` (product owner, 2026-10-29): every trace the
/// contributor has submitted, whatever became of it (accepted, quarantined,
/// rejected, withdrawn or still pending), joined to what the counter pass
/// stored about its session. Credit is in points as History stores them;
/// money is integer USD micros at API list prices. Every session figure is
/// a sum as far as known beside its `partial` flag, never a zero for
/// unknown, and there is no minimum: one trace answers every part.
extension DaemonData {
    public struct InsightsSubmitted: Codable, Equatable, Sendable {
        public let enabled: Bool
        public let feed: String?
        public let readable: Bool?
        /// Why `readable` is false: a fixed label (`store_unreadable`,
        /// `key_unavailable`, `policy_unreadable`, `history_unreadable`).
        public let reason: String?
        public let updatedAt: String?
        /// `4`, `13`, or `nil` for all time.
        public let weeks: Int?
        public let windowStart: String?
        public let windowEnd: String?
        public let pricing: InsightsSubmittedPricing?
        public let totals: InsightsSubmittedTotals?
        public let byModel: [InsightsSubmittedModel]?
        public let byModelLeftOut: InsightsSubmittedModelLeftOut?
        /// Always eight, in Benchmarks' order: `bug_fix`, `feature`,
        /// `refactor`, `tests`, `docs`, `understand`, `review`, `setup`.
        public let byTask: [InsightsSubmittedTally]?
        /// Submissions with no task: unmatched, or stored without facts.
        public let byTaskUnknown: InsightsSubmittedTally?
        public let code: InsightsSubmittedCode?
        /// Oldest first, empty weeks included.
        public let byWeek: [InsightsSubmittedTally]?
        public let byProject: [InsightsSubmittedTally]?
        public let leftOut: InsightsSubmittedLeftOut?
        public let tracesTotal: Int?
        /// Newest first, at most 500; `tracesTotal` counts them all.
        public let traces: [InsightsSubmittedTrace]?

        public init(enabled: Bool = true, feed: String? = "counter_pass", readable: Bool? = true,
                    reason: String? = nil, updatedAt: String? = nil, weeks: Int? = nil,
                    windowStart: String? = nil, windowEnd: String? = nil,
                    pricing: InsightsSubmittedPricing? = nil, totals: InsightsSubmittedTotals?,
                    byModel: [InsightsSubmittedModel]? = [], byModelLeftOut: InsightsSubmittedModelLeftOut? = nil,
                    byTask: [InsightsSubmittedTally]? = [], byTaskUnknown: InsightsSubmittedTally? = nil,
                    code: InsightsSubmittedCode? = nil, byWeek: [InsightsSubmittedTally]? = [],
                    byProject: [InsightsSubmittedTally]? = [], leftOut: InsightsSubmittedLeftOut? = nil,
                    tracesTotal: Int? = nil, traces: [InsightsSubmittedTrace]? = []) {
            self.enabled = enabled; self.feed = feed; self.readable = readable; self.reason = reason
            self.updatedAt = updatedAt; self.weeks = weeks; self.windowStart = windowStart
            self.windowEnd = windowEnd; self.pricing = pricing; self.totals = totals; self.byModel = byModel
            self.byModelLeftOut = byModelLeftOut; self.byTask = byTask; self.byTaskUnknown = byTaskUnknown
            self.code = code; self.byWeek = byWeek; self.byProject = byProject; self.leftOut = leftOut
            self.tracesTotal = tracesTotal; self.traces = traces
        }

        /// An answer to draw: switched on and read.
        public var isAnswered: Bool { enabled && readable == true && totals != nil }

        public enum CodingKeys: String, CodingKey {
            case enabled, feed, readable, reason, weeks, pricing, totals, code, traces
            case updatedAt = "updated_at"
            case windowStart = "window_start"
            case windowEnd = "window_end"
            case byModel = "by_model"
            case byModelLeftOut = "by_model_left_out"
            case byTask = "by_task"
            case byTaskUnknown = "by_task_unknown"
            case byWeek = "by_week"
            case byProject = "by_project"
            case leftOut = "left_out"
            case tracesTotal = "traces_total"
        }
    }

    public struct InsightsSubmittedPricing: Codable, Equatable, Sendable {
        public let basis: String
        public let tables: [InsightsSubmittedPriceTable]

        public init(basis: String = "api_list", tables: [InsightsSubmittedPriceTable] = []) {
            self.basis = basis; self.tables = tables
        }
    }

    public struct InsightsSubmittedPriceTable: Codable, Equatable, Sendable {
        public let tableId: String
        public let version: Int

        public init(tableId: String, version: Int) { self.tableId = tableId; self.version = version }

        public enum CodingKeys: String, CodingKey {
            case version
            case tableId = "table_id"
        }
    }

    /// Submitted, accepted and credit for one group. `key` names a task,
    /// `weekStart` a local Monday (`YYYY-MM-DD`), `label` a History project
    /// label; each is `nil` where it does not apply.
    public struct InsightsSubmittedTally: Codable, Equatable, Sendable {
        public var key: String? = nil
        public var weekStart: String? = nil
        public var label: String? = nil
        public let submitted: Int
        public let accepted: Int
        public let creditFinal: Double
        public let creditPending: Double
        /// Joined to a stored session.
        public let matched: Int

        public init(key: String? = nil, weekStart: String? = nil, label: String? = nil, submitted: Int,
                    accepted: Int, creditFinal: Double, creditPending: Double, matched: Int) {
            self.key = key; self.weekStart = weekStart; self.label = label; self.submitted = submitted
            self.accepted = accepted; self.creditFinal = creditFinal; self.creditPending = creditPending
            self.matched = matched
        }

        public enum CodingKeys: String, CodingKey {
            case key, label, submitted, accepted, matched
            case weekStart = "week_start"
            case creditFinal = "credit_final"
            case creditPending = "credit_pending"
        }
    }

    public struct InsightsSubmittedTotals: Codable, Equatable, Sendable {
        public let submitted: Int
        public let accepted: Int
        public let creditFinal: Double
        public let creditPending: Double
        public let matched: Int
        public let unmatched: Int
        /// `accepted`, `pending`, `quarantined`, `rejected`, `withdrawn`,
        /// `other`, each always present.
        public let byStatus: [String: Int]
        /// The server's credit outcome label (`policy_mismatch`,
        /// `novelty_failed`, `duplicate`, ...) to its count. Word it through
        /// the core's copy, never raw.
        public let byOutcomeReason: [String: Int]
        /// `no_queue_entry`, `source_not_read`, `no_session_row`.
        public let unmatchedByReason: [String: Int]

        public init(submitted: Int, accepted: Int, creditFinal: Double, creditPending: Double, matched: Int,
                    unmatched: Int, byStatus: [String: Int], byOutcomeReason: [String: Int],
                    unmatchedByReason: [String: Int]) {
            self.submitted = submitted; self.accepted = accepted; self.creditFinal = creditFinal
            self.creditPending = creditPending; self.matched = matched; self.unmatched = unmatched
            self.byStatus = byStatus; self.byOutcomeReason = byOutcomeReason
            self.unmatchedByReason = unmatchedByReason
        }

        public enum CodingKeys: String, CodingKey {
            case submitted, accepted, matched, unmatched
            case creditFinal = "credit_final"
            case creditPending = "credit_pending"
            case byStatus = "by_status"
            case byOutcomeReason = "by_outcome_reason"
            case unmatchedByReason = "unmatched_by_reason"
        }
    }

    /// Edit and line counts, each known part beside its `partial` flag.
    /// An unknown declined count reads as none declined in `editsKeptKnown`.
    public struct InsightsSubmittedEdits: Codable, Equatable, Sendable {
        public let editsKnown: Int64
        public let editsPartial: Bool
        public let editsKeptKnown: Int64
        public let editsKeptPartial: Bool
        public let editsUndoneKnown: Int64
        public let editsUndonePartial: Bool
        public let editsDeniedKnown: Int64
        public let editsDeniedPartial: Bool
        public let linesAddedKnown: Int64
        public let linesRemovedKnown: Int64
        public let linesPartial: Bool

        public init(editsKnown: Int64, editsPartial: Bool = false, editsKeptKnown: Int64,
                    editsKeptPartial: Bool = false, editsUndoneKnown: Int64 = 0, editsUndonePartial: Bool = false,
                    editsDeniedKnown: Int64 = 0, editsDeniedPartial: Bool = false, linesAddedKnown: Int64 = 0,
                    linesRemovedKnown: Int64 = 0, linesPartial: Bool = false) {
            self.editsKnown = editsKnown; self.editsPartial = editsPartial; self.editsKeptKnown = editsKeptKnown
            self.editsKeptPartial = editsKeptPartial; self.editsUndoneKnown = editsUndoneKnown
            self.editsUndonePartial = editsUndonePartial; self.editsDeniedKnown = editsDeniedKnown
            self.editsDeniedPartial = editsDeniedPartial; self.linesAddedKnown = linesAddedKnown
            self.linesRemovedKnown = linesRemovedKnown; self.linesPartial = linesPartial
        }

        public enum CodingKeys: String, CodingKey, CaseIterable {
            case editsKnown = "edits_known"
            case editsPartial = "edits_partial"
            case editsKeptKnown = "edits_kept_known"
            case editsKeptPartial = "edits_kept_partial"
            case editsUndoneKnown = "edits_undone_known"
            case editsUndonePartial = "edits_undone_partial"
            case editsDeniedKnown = "edits_denied_known"
            case editsDeniedPartial = "edits_denied_partial"
            case linesAddedKnown = "lines_added_known"
            case linesRemovedKnown = "lines_removed_known"
            case linesPartial = "lines_partial"
        }
    }

    /// Spend at API list prices. A session any of whose usage went through
    /// NEAR AI is not list-priced: it counts in `nearAiSessions`.
    public struct InsightsSubmittedSpend: Codable, Equatable, Sendable {
        public let usdMicrosKnown: Int64
        public let usdPartial: Bool
        public let pricedSessions: Int
        public let unpricedSessions: Int
        public let nearAiSessions: Int

        public init(usdMicrosKnown: Int64, usdPartial: Bool = false, pricedSessions: Int,
                    unpricedSessions: Int = 0, nearAiSessions: Int = 0) {
            self.usdMicrosKnown = usdMicrosKnown; self.usdPartial = usdPartial; self.pricedSessions = pricedSessions
            self.unpricedSessions = unpricedSessions; self.nearAiSessions = nearAiSessions
        }

        public enum CodingKeys: String, CodingKey, CaseIterable {
            case usdMicrosKnown = "usd_micros_known"
            case usdPartial = "usd_partial"
            case pricedSessions = "priced_sessions"
            case unpricedSessions = "unpriced_sessions"
            case nearAiSessions = "near_ai_sessions"
        }
    }

    /// Distinct edited files per extension class, summed over sessions.
    public struct InsightsSubmittedFiles: Codable, Equatable, Sendable {
        public let source: Int
        public let test: Int
        public let doc: Int
        public let config: Int
        public let total: Int

        public init(source: Int = 0, test: Int = 0, doc: Int = 0, config: Int = 0) {
            self.source = source; self.test = test; self.doc = doc; self.config = config
            self.total = source + test + doc + config
        }
    }

    /// One model's submitted sessions, by the normalized declared label
    /// serving a strict majority of each session's turns.
    public struct InsightsSubmittedModel: Codable, Equatable, Sendable, Identifiable {
        public let label: String
        /// `claude_code` or `codex`.
        public let tool: String
        /// Decoded from the row itself, so its `label` is the model's.
        public let tally: InsightsSubmittedTally
        public let spend: InsightsSubmittedSpend
        public let edits: InsightsSubmittedEdits
        /// Task key to sessions.
        public let tasks: [String: Int]
        public var id: String { "\(tool)/\(label)" }

        public init(label: String, tool: String, tally: InsightsSubmittedTally, spend: InsightsSubmittedSpend,
                    edits: InsightsSubmittedEdits, tasks: [String: Int]) {
            self.label = label; self.tool = tool; self.tally = tally; self.spend = spend
            self.edits = edits; self.tasks = tasks
        }

        private enum Keys: String, CodingKey { case label, tool, tasks }

        public init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: Keys.self)
            label = try c.decode(String.self, forKey: .label)
            tool = try c.decode(String.self, forKey: .tool)
            tasks = try c.decode([String: Int].self, forKey: .tasks)
            tally = try InsightsSubmittedTally(from: decoder)
            spend = try InsightsSubmittedSpend(from: decoder)
            edits = try InsightsSubmittedEdits(from: decoder)
        }

        public func encode(to encoder: Encoder) throws {
            var c = encoder.container(keyedBy: Keys.self)
            try c.encode(label, forKey: .label)
            try c.encode(tool, forKey: .tool)
            try c.encode(tasks, forKey: .tasks)
            try tally.encode(to: encoder)
            try spend.encode(to: encoder)
            try edits.encode(to: encoder)
        }
    }

    public struct InsightsSubmittedModelLeftOut: Codable, Equatable, Sendable {
        public let unmatched: Int
        public let mixedModels: Int
        public let noModel: Int

        public init(unmatched: Int, mixedModels: Int = 0, noModel: Int = 0) {
            self.unmatched = unmatched; self.mixedModels = mixedModels; self.noModel = noModel
        }

        public enum CodingKeys: String, CodingKey {
            case unmatched
            case mixedModels = "mixed_models"
            case noModel = "no_model"
        }
    }

    /// What the matched sessions changed. Never a path or a file name.
    public struct InsightsSubmittedCode: Codable, Equatable, Sendable {
        public let sessions: Int
        public let sessionsWithFiles: Int
        public let files: InsightsSubmittedFiles
        public let filesPartial: Bool
        public let edits: InsightsSubmittedEdits
        public let spend: InsightsSubmittedSpend

        public init(sessions: Int, sessionsWithFiles: Int, files: InsightsSubmittedFiles, filesPartial: Bool = false,
                    edits: InsightsSubmittedEdits, spend: InsightsSubmittedSpend) {
            self.sessions = sessions; self.sessionsWithFiles = sessionsWithFiles; self.files = files
            self.filesPartial = filesPartial; self.edits = edits; self.spend = spend
        }

        private enum Keys: String, CodingKey {
            case sessions, files
            case sessionsWithFiles = "sessions_with_files"
            case filesPartial = "files_partial"
        }

        public init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: Keys.self)
            sessions = try c.decode(Int.self, forKey: .sessions)
            sessionsWithFiles = try c.decode(Int.self, forKey: .sessionsWithFiles)
            files = try c.decode(InsightsSubmittedFiles.self, forKey: .files)
            filesPartial = try c.decode(Bool.self, forKey: .filesPartial)
            edits = try InsightsSubmittedEdits(from: decoder)
            spend = try InsightsSubmittedSpend(from: decoder)
        }

        public func encode(to encoder: Encoder) throws {
            var c = encoder.container(keyedBy: Keys.self)
            try c.encode(sessions, forKey: .sessions)
            try c.encode(sessionsWithFiles, forKey: .sessionsWithFiles)
            try c.encode(files, forKey: .files)
            try c.encode(filesPartial, forKey: .filesPartial)
            try edits.encode(to: encoder)
            try spend.encode(to: encoder)
        }
    }

    public struct InsightsSubmittedLeftOut: Codable, Equatable, Sendable {
        /// Submissions from a folder set to Never: in no figure.
        public let neverFolder: Int

        public init(neverFolder: Int = 0) { self.neverFolder = neverFolder }

        public enum CodingKeys: String, CodingKey {
            case neverFolder = "never_folder"
        }
    }

    /// One submission. Every session figure is `nil` when unmatched or
    /// unknown, never zero.
    public struct InsightsSubmittedTrace: Codable, Equatable, Sendable, Identifiable {
        /// The reference History rows carry.
        public let submissionId: String
        public let submittedAt: String
        public let projectLabel: String
        /// The adapter, as History shows it (`claude-code`, `codex`, ...).
        public let source: String
        /// `accepted`, `pending`, `quarantined`, `rejected`, `withdrawn`
        /// or `other`.
        public let status: String
        public let creditPointsPending: Double
        public let creditPointsFinal: Double?
        public let creditOutcomeReason: String?
        public let matched: Bool
        /// `no_queue_entry`, `source_not_read` or `no_session_row`.
        public let unmatchedReason: String?
        public let tool: String?
        public let model: String?
        public let mixedModels: Bool
        public let task: String?
        public let files: InsightsSubmittedFiles?
        public let edits: Int?
        public let editsKept: Int?
        public let editsUndone: Int?
        public let editsDenied: Int?
        public let usdMicros: Int64?
        public let nearAi: Bool
        public var id: String { submissionId }

        public init(submissionId: String, submittedAt: String, projectLabel: String, source: String,
                    status: String, creditPointsPending: Double = 0, creditPointsFinal: Double? = nil,
                    creditOutcomeReason: String? = nil, matched: Bool = true, unmatchedReason: String? = nil,
                    tool: String? = nil, model: String? = nil, mixedModels: Bool = false, task: String? = nil,
                    files: InsightsSubmittedFiles? = nil, edits: Int? = nil, editsKept: Int? = nil,
                    editsUndone: Int? = nil, editsDenied: Int? = nil, usdMicros: Int64? = nil,
                    nearAi: Bool = false) {
            self.submissionId = submissionId; self.submittedAt = submittedAt; self.projectLabel = projectLabel
            self.source = source; self.status = status; self.creditPointsPending = creditPointsPending
            self.creditPointsFinal = creditPointsFinal; self.creditOutcomeReason = creditOutcomeReason
            self.matched = matched; self.unmatchedReason = unmatchedReason; self.tool = tool
            self.model = model; self.mixedModels = mixedModels; self.task = task; self.files = files
            self.edits = edits; self.editsKept = editsKept; self.editsUndone = editsUndone
            self.editsDenied = editsDenied; self.usdMicros = usdMicros; self.nearAi = nearAi
        }

        public enum CodingKeys: String, CodingKey {
            case source, status, matched, tool, model, task, files, edits
            case submissionId = "submission_id"
            case submittedAt = "submitted_at"
            case projectLabel = "project_label"
            case creditPointsPending = "credit_points_pending"
            case creditPointsFinal = "credit_points_final"
            case creditOutcomeReason = "credit_outcome_reason"
            case unmatchedReason = "unmatched_reason"
            case mixedModels = "mixed_models"
            case editsKept = "edits_kept"
            case editsUndone = "edits_undone"
            case editsDenied = "edits_denied"
            case usdMicros = "usd_micros"
            case nearAi = "near_ai"
        }
    }
}
