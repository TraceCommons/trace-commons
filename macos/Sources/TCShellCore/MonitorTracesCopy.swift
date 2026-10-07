import Foundation

/// The glass monitor's Traces words, decoded from
/// `tc_monitor_traces_copy_json` (`preview_copy::monitor_traces_copy`).
///
/// Every property comes from the payload: the inspector's row labels, the
/// review's actions, and what the tab says when the core does not answer.
/// They were written in Swift (`MonitorWords`); the core now holds them so
/// no shell writes its own.
///
/// Decoding is here rather than in `TCBridge` so it can be tested without
/// linking the dylib; `TCBridgeTests` checks it against the real export.
public struct MonitorTracesCopy: Decodable, Equatable, Sendable {
    public let review: String
    public let tool: String
    public let folder: String
    public let started: String
    public let length: String
    public let prompts: String
    public let size: String
    public let sends: String
    public let marks: String
    public let unsure: String
    public let eligibility: String
    public let attestation: String
    public let held: String
    public let sample: String
    public let residualRisk: String
    public let personalInformation: String
    public let secondLookWaiting: String
    public let contribute: String
    public let keep: String
    public let dismiss: String
    public let undoContribute: String
    public let undoKeep: String
    public let coreUnreachable: String
    public let requestFailed: String
    /// Ron's plain Dismiss (#1146): the undo bar's, the Dismiss-session
    /// confirmation's and the review card's. Not `dismiss` ("Not this one").
    public let dismissAction: String
    /// The arming offer's eyebrow.
    public let optionalAutomation: String
    /// Ron's #1146 inspector words (#1241), by the part that shows them.
    public let tree: MonitorTreeCopy
    public let counts: MonitorCountsCopy
    public let inspector: MonitorInspectorCopy
    public let summaryPanel: MonitorSummaryCopy
    public let sessionReview: MonitorSessionReviewCopy
    public let lookInside: MonitorLookInsideCopy
    public let undo: MonitorUndoCopy

    enum CodingKeys: String, CodingKey {
        case review, tool, folder, started, length, prompts, size, sends, marks, unsure
        case eligibility, attestation, held, sample
        case residualRisk = "residual_risk"
        case personalInformation = "personal_information"
        case secondLookWaiting = "second_look_waiting"
        case contribute, keep, dismiss
        case undoContribute = "undo_contribute"
        case undoKeep = "undo_keep"
        case coreUnreachable = "core_unreachable"
        case requestFailed = "request_failed"
        case dismissAction = "dismiss_action"
        case optionalAutomation = "optional_automation"
        case tree, counts, inspector
        case summaryPanel = "summary_panel"
        case sessionReview = "session_review"
        case lookInside = "look_inside"
        case undo
    }

    /// The payload fields this shell decodes, by wire name.
    public static let consumedFields = [
        "review", "tool", "folder", "started", "length", "prompts", "size", "sends", "marks", "unsure",
        "eligibility", "attestation", "held", "sample", "residual_risk", "personal_information",
        "second_look_waiting", "contribute", "keep", "dismiss", "undo_contribute", "undo_keep", "core_unreachable", "request_failed",
        "dismiss_action", "optional_automation", "tree", "counts", "inspector", "summary_panel", "session_review",
        "look_inside", "undo",
    ]

    /// Each nested table's wire fields, by its wire name.
    public static let consumedTables: [String: [String]] = [
        "tree": MonitorTreeCopy.consumedFields,
        "counts": MonitorCountsCopy.consumedFields,
        "inspector": MonitorInspectorCopy.consumedFields,
        "summary_panel": MonitorSummaryCopy.consumedFields,
        "session_review": MonitorSessionReviewCopy.consumedFields,
        "look_inside": MonitorLookInsideCopy.consumedFields,
        "undo": MonitorUndoCopy.consumedFields,
    ]

    /// Decode the payload, or nil if it will not parse or a field is empty.
    public static func decode(fromJSON json: String?) -> MonitorTracesCopy? {
        guard let data = json?.data(using: .utf8),
            let copy = try? JSONDecoder().decode(MonitorTracesCopy.self, from: data)
        else {
            return nil
        }
        let words = [
            copy.review, copy.tool, copy.folder, copy.started, copy.length, copy.prompts, copy.size,
            copy.sends, copy.marks, copy.unsure, copy.eligibility, copy.attestation, copy.held, copy.sample,
            copy.residualRisk, copy.personalInformation, copy.secondLookWaiting,
            copy.contribute, copy.keep, copy.dismiss,
            copy.undoContribute, copy.undoKeep, copy.coreUnreachable, copy.requestFailed,
            copy.dismissAction, copy.optionalAutomation,
        ]
        let tables: [Bool] = [
            copy.tree.isWhole, copy.counts.isWhole, copy.inspector.isWhole, copy.summaryPanel.isWhole,
            copy.sessionReview.isWhole, copy.lookInside.isWhole, copy.undo.isWhole,
        ]
        return words.contains(where: \.isEmpty) || tables.contains(false) ? nil : copy
    }

    /// What the tab says for a failed request: the core's line for a core
    /// that does not answer, or for one that refused. Never the error's own
    /// fixed label, which is for logs.
    public func line(for error: DaemonDataError) -> String {
        if case .unreachable = error { return coreUnreachable }
        return requestFailed
    }
}

/// Ron's #1146 inspector words (#1241), nested in the monitor tables by the
/// part of the inspector that shows them. Decoded from
/// `preview_copy::Monitor*Copy`; every property comes from the payload and
/// none is written here. A `{name}` in a word is a hole the shell fills
/// (`FirstRunCopy.fill`) and adds nothing to.

/// A nested word table: decoded by wire name, and whole only when every
/// word in it is non-empty.
public protocol MonitorWordTable: Decodable, Equatable, Sendable {
    /// The payload fields this shell decodes, by wire name.
    static var consumedFields: [String] { get }
}

extension MonitorWordTable {
    /// Every word in the table, so a decode can refuse an empty one.
    var words: [String] { Mirror(reflecting: self).children.compactMap { $0.value as? String } }

    /// Whether every word is there: an empty one is a table this shell
    /// cannot render, never a blank label.
    var isWhole: Bool { !words.contains(where: \.isEmpty) }
}

/// The Traces tree's rows and the Dismiss-session confirmation (`traces-tree.tsx`).
public struct MonitorTreeCopy: MonitorWordTable {
    public let treeLabel: String
    public let readingQueue: String
    public let submit: String
    public let submitCount: String
    public let submitting: String
    public let submitTip: String
    public let eligibleCount: String
    public let reviewing: String
    public let reviewTip: String
    public let ignoredFolder: String
    public let ignoreFolder: String
    public let watchFolder: String
    public let watchTool: String
    public let dismissSession: String
    public let dismissSessionTitle: String
    public let dismissSessionBody: String
    public let dismissSessionKeep: String
    public let dismissing: String
    public let dismissSessionFailed: String

    enum CodingKeys: String, CodingKey, CaseIterable {
        case treeLabel = "tree_label"
        case readingQueue = "reading_queue"
        case submit
        case submitCount = "submit_count"
        case submitting
        case submitTip = "submit_tip"
        case eligibleCount = "eligible_count"
        case reviewing
        case reviewTip = "review_tip"
        case ignoredFolder = "ignored_folder"
        case ignoreFolder = "ignore_folder"
        case watchFolder = "watch_folder"
        case watchTool = "watch_tool"
        case dismissSession = "dismiss_session"
        case dismissSessionTitle = "dismiss_session_title"
        case dismissSessionBody = "dismiss_session_body"
        case dismissSessionKeep = "dismiss_session_keep"
        case dismissing
        case dismissSessionFailed = "dismiss_session_failed"
    }

    public static var consumedFields: [String] { CodingKeys.allCases.map(\.rawValue) }
}

/// Counted lines the tree, the inspectors and the summary share. A singular is its own line.
public struct MonitorCountsCopy: MonitorWordTable {
    public let sessionsWaitingOne: String
    public let sessionsWaiting: String
    public let waitingCount: String
    public let contributedCount: String
    public let projectCountOne: String
    public let projectCount: String

    enum CodingKeys: String, CodingKey, CaseIterable {
        case sessionsWaitingOne = "sessions_waiting_one"
        case sessionsWaiting = "sessions_waiting"
        case waitingCount = "waiting_count"
        case contributedCount = "contributed_count"
        case projectCountOne = "project_count_one"
        case projectCount = "project_count"
    }

    public static var consumedFields: [String] { CodingKeys.allCases.map(\.rawValue) }
}

/// The tool, folder and session inspectors' headings, and Submit-all-as's sentence.
public struct MonitorInspectorCopy: MonitorWordTable {
    public let decisions: String
    public let notSet: String
    public let sessionsFolder: String
    public let project: String
    public let projectOf: String
    public let sessionOf: String
    public let path: String
    public let contributionRule: String
    public let noRule: String
    public let applyOutcomeOne: String
    public let applyOutcome: String
    public let cancel: String

    enum CodingKeys: String, CodingKey, CaseIterable {
        case decisions
        case notSet = "not_set"
        case sessionsFolder = "sessions_folder"
        case project
        case projectOf = "project_of"
        case sessionOf = "session_of"
        case path
        case contributionRule = "contribution_rule"
        case noRule = "no_rule"
        case applyOutcomeOne = "apply_outcome_one"
        case applyOutcome = "apply_outcome"
        case cancel
    }

    public static var consumedFields: [String] { CodingKeys.allCases.map(\.rawValue) }
}

/// The summary inspector shown when nothing is selected.
public struct MonitorSummaryCopy: MonitorWordTable {
    public let toolsWatched: String
    public let waitingForYou: String
    public let worthASecondLook: String
    public let uploadsToday: String
    public let statistics: String
    public let topProjects: String
    public let topTools: String
    public let noLongerWaiting: String
    public let noLongerWaitingScope: String

    enum CodingKeys: String, CodingKey, CaseIterable {
        case toolsWatched = "tools_watched"
        case waitingForYou = "waiting_for_you"
        case worthASecondLook = "worth_a_second_look"
        case uploadsToday = "uploads_today"
        case statistics
        case topProjects = "top_projects"
        case topTools = "top_tools"
        case noLongerWaiting = "no_longer_waiting"
        case noLongerWaitingScope = "no_longer_waiting_scope"
    }

    public static var consumedFields: [String] { CodingKeys.allCases.map(\.rawValue) }
}

/// The session review card (Ron's `WaitingReview`). The verdict and correction words are `ContributorDisclosureCopy.outcome`.
public struct MonitorSessionReviewCopy: MonitorWordTable {
    public let eyebrow: String
    public let heading: String
    public let enrolled: String
    public let notEnrolled: String
    public let noOpeningPrompt: String
    public let redactedPayload: String
    public let events: String
    public let buildingPreview: String
    public let cannotShowTitle: String
    public let cannotShowBody: String
    public let preparingRedactions: String
    public let redactionsUnavailable: String
    public let removed: String
    public let nothingRemoved: String
    public let stillPresent: String
    public let residualUnavailable: String
    public let residualLoading: String
    public let consentScopes: String
    public let eligibilityFailed: String
    public let eligibilityChecking: String
    public let outcomeUnavailable: String
    public let outcomeLoading: String
    public let lookInside: String
    public let enrollToApprove: String
    public let notEligible: String
    public let checkingEligibility: String

    enum CodingKeys: String, CodingKey, CaseIterable {
        case eyebrow
        case heading
        case enrolled
        case notEnrolled = "not_enrolled"
        case noOpeningPrompt = "no_opening_prompt"
        case redactedPayload = "redacted_payload"
        case events
        case buildingPreview = "building_preview"
        case cannotShowTitle = "cannot_show_title"
        case cannotShowBody = "cannot_show_body"
        case preparingRedactions = "preparing_redactions"
        case redactionsUnavailable = "redactions_unavailable"
        case removed
        case nothingRemoved = "nothing_removed"
        case stillPresent = "still_present"
        case residualUnavailable = "residual_unavailable"
        case residualLoading = "residual_loading"
        case consentScopes = "consent_scopes"
        case eligibilityFailed = "eligibility_failed"
        case eligibilityChecking = "eligibility_checking"
        case outcomeUnavailable = "outcome_unavailable"
        case outcomeLoading = "outcome_loading"
        case lookInside = "look_inside"
        case enrollToApprove = "enroll_to_approve"
        case notEligible = "not_eligible"
        case checkingEligibility = "checking_eligibility"
    }

    public static var consumedFields: [String] { CodingKeys.allCases.map(\.rawValue) }
}

/// Look inside, read-only (Ron's `PreviewInspector`), with the native review actions and the witness consent line.
public struct MonitorLookInsideCopy: MonitorWordTable {
    public let eyebrow: String
    public let title: String
    public let description: String
    public let wouldSend: String
    public let onDisk: String
    public let loadTranscript: String
    public let loadingTranscript: String
    public let searchOriginal: String
    public let turnIndex: String
    public let transcriptCaption: String
    public let loadMore: String
    public let addTurnSeparators: String
    public let searchCaption: String
    public let searchLabel: String
    public let searchPlaceholder: String
    public let checkCount: String
    public let originalMatchOne: String
    public let originalMatches: String
    public let turnsNeedFullRead: String
    public let loadTurnIndex: String
    public let turnIndexEyebrow: String
    public let turnEvent: String
    public let turnBytes: String
    public let close: String
    public let nativeReview: String
    public let nativeReviewCaption: String
    public let prepareAdmission: String
    public let requestWitnessReview: String
    public let witnessConfirmLine: String
    public let witnessConfirmLabel: String
    public let witnessReviewing: String

    enum CodingKeys: String, CodingKey, CaseIterable {
        case eyebrow
        case title
        case description
        case wouldSend = "would_send"
        case onDisk = "on_disk"
        case loadTranscript = "load_transcript"
        case loadingTranscript = "loading_transcript"
        case searchOriginal = "search_original"
        case turnIndex = "turn_index"
        case transcriptCaption = "transcript_caption"
        case loadMore = "load_more"
        case addTurnSeparators = "add_turn_separators"
        case searchCaption = "search_caption"
        case searchLabel = "search_label"
        case searchPlaceholder = "search_placeholder"
        case checkCount = "check_count"
        case originalMatchOne = "original_match_one"
        case originalMatches = "original_matches"
        case turnsNeedFullRead = "turns_need_full_read"
        case loadTurnIndex = "load_turn_index"
        case turnIndexEyebrow = "turn_index_eyebrow"
        case turnEvent = "turn_event"
        case turnBytes = "turn_bytes"
        case close
        case nativeReview = "native_review"
        case nativeReviewCaption = "native_review_caption"
        case prepareAdmission = "prepare_admission"
        case requestWitnessReview = "request_witness_review"
        case witnessConfirmLine = "witness_confirm_line"
        case witnessConfirmLabel = "witness_confirm_label"
        case witnessReviewing = "witness_reviewing"
    }

    public static var consumedFields: [String] { CodingKeys.allCases.map(\.rawValue) }
}

/// The undo bar after an approve (Ron's `UndoBar`). Undo itself is `MonitorTracesCopy.undoContribute`.
public struct MonitorUndoCopy: MonitorWordTable {
    public let approvalSaved: String
    public let approved: String
    public let unavailable: String
    public let within: String
    public let mayHaveStarted: String
    public let undoing: String

    enum CodingKeys: String, CodingKey, CaseIterable {
        case approvalSaved = "approval_saved"
        case approved
        case unavailable
        case within
        case mayHaveStarted = "may_have_started"
        case undoing
    }

    public static var consumedFields: [String] { CodingKeys.allCases.map(\.rawValue) }
}

/// The queue's safeguards panel labels that `HealthCopy`, `DailyBudgetCopy` and the routing copy do not already hold.
public struct MonitorSafeguardsCopy: MonitorWordTable {
    public let eyebrow: String
    public let heading: String
    public let dailyLimit: String
    public let inferenceRouting: String
    public let daemonOwned: String
    public let rowsUnavailableOne: String
    public let rowsUnavailable: String
    public let remaining: String
    public let heldByLimitOne: String
    public let heldByLimit: String
    public let capacityUnreadable: String

    enum CodingKeys: String, CodingKey, CaseIterable {
        case eyebrow
        case heading
        case dailyLimit = "daily_limit"
        case inferenceRouting = "inference_routing"
        case daemonOwned = "daemon_owned"
        case rowsUnavailableOne = "rows_unavailable_one"
        case rowsUnavailable = "rows_unavailable"
        case remaining
        case heldByLimitOne = "held_by_limit_one"
        case heldByLimit = "held_by_limit"
        case capacityUnreadable = "capacity_unreadable"
    }

    public static var consumedFields: [String] { CodingKeys.allCases.map(\.rawValue) }
}

/// History's refresh and account sign-in controls: the controls Ron's
/// #1146 `history-refresh-control.tsx` and `account-sign-in-control.tsx`
/// draw, in the core's native words rather than his.
public struct MonitorHistoryActionsCopy: MonitorWordTable {
    public let requestRefresh: String
    public let requesting: String
    public let refreshRequested: String
    public let refreshFailed: String
    public let checkingAccount: String
    public let signInToWithdraw: String
    public let waitingForSignIn: String
    public let completeSignIn: String
    public let signInInactive: String
    public let signInUnverified: String
    public let signInFailed: String

    enum CodingKeys: String, CodingKey, CaseIterable {
        case requestRefresh = "request_refresh"
        case requesting
        case refreshRequested = "refresh_requested"
        case refreshFailed = "refresh_failed"
        case checkingAccount = "checking_account"
        case signInToWithdraw = "sign_in_to_withdraw"
        case waitingForSignIn = "waiting_for_sign_in"
        case completeSignIn = "complete_sign_in"
        case signInInactive = "sign_in_inactive"
        case signInUnverified = "sign_in_unverified"
        case signInFailed = "sign_in_failed"
    }

    public static var consumedFields: [String] { CodingKeys.allCases.map(\.rawValue) }
}

/// Ron's #1146 words for the monitor's toolbar, the Traces graph's focus
/// button, Home and History (`preview_copy::MonitorShellCopy`; owner
/// ruling, 2026-10-06). Numbers are `{name}` holes; a singular is its own
/// line.
public struct MonitorShellCopy: MonitorWordTable {
    public let showGraph: String
    public let hideGraph: String
    public let showMap: String
    public let hideMap: String
    public let showInspector: String
    public let hideInspector: String
    public let focusNeedsSelection: String
    public let focusWholeMap: String
    public let focusTool: String
    public let watchingToolsOne: String
    public let watchingTools: String
    public let waitingForYouOne: String
    public let waitingForYou: String
    public let worthASecondLook: String
    public let nothingWaiting: String
    public let nothingContributed: String
    public let creditPendingAmount: String
    public let filterAll: String
    public let filterAccepted: String
    public let filterSubmitted: String
    public let filterQuarantined: String
    public let filterWithdrawn: String
    public let historyEmpty: String
    public let historyFilterEmpty: String
    public let open: String

    enum CodingKeys: String, CodingKey, CaseIterable {
        case showGraph = "show_graph"
        case hideGraph = "hide_graph"
        case showMap = "show_map"
        case hideMap = "hide_map"
        case showInspector = "show_inspector"
        case hideInspector = "hide_inspector"
        case focusNeedsSelection = "focus_needs_selection"
        case focusWholeMap = "focus_whole_map"
        case focusTool = "focus_tool"
        case watchingToolsOne = "watching_tools_one"
        case watchingTools = "watching_tools"
        case waitingForYouOne = "waiting_for_you_one"
        case waitingForYou = "waiting_for_you"
        case worthASecondLook = "worth_a_second_look"
        case nothingWaiting = "nothing_waiting"
        case nothingContributed = "nothing_contributed"
        case creditPendingAmount = "credit_pending_amount"
        case filterAll = "filter_all"
        case filterAccepted = "filter_accepted"
        case filterSubmitted = "filter_submitted"
        case filterQuarantined = "filter_quarantined"
        case filterWithdrawn = "filter_withdrawn"
        case historyEmpty = "history_empty"
        case historyFilterEmpty = "history_filter_empty"
        case open
    }

    public static var consumedFields: [String] { CodingKeys.allCases.map(\.rawValue) }

    /// Watching N tools: the singular is its own line.
    public func watching(tools: Int) -> String {
        tools == 1 ? watchingToolsOne : watchingTools.replacingOccurrences(of: "{count}", with: String(tools))
    }

    /// Home's second status line: sessions waiting, then how many of them
    /// are worth a second look, or nothing waiting.
    public func waiting(_ waiting: Int, secondLook: Int) -> String {
        guard waiting > 0 else { return nothingWaiting }
        let head = waiting == 1
            ? waitingForYouOne : waitingForYou.replacingOccurrences(of: "{count}", with: String(waiting))
        guard secondLook > 0 else { return head }
        return head + " \u{00B7} " + worthASecondLook.replacingOccurrences(of: "{count}", with: String(secondLook))
    }

    /// The focus button's tip: select first, back to the whole map, or
    /// show the selected tool.
    public func focusTip(tool: String?, focused: Bool) -> String {
        guard let tool else { return focusNeedsSelection }
        return focused ? focusWholeMap : focusTool.replacingOccurrences(of: "{tool}", with: tool)
    }
}
