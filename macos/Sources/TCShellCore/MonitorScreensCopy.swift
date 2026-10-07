import Foundation

/// The glass monitor's other screens' words (the map, Inference, Home,
/// History, Missions and the menu-bar popover), decoded from
/// `tc_monitor_screens_copy_json` (`preview_copy::monitor_screens_copy`).
///
/// They were Swift (`MonitorWords`, `InferenceWords`, `MenuWords`); the core
/// holds them now, as it holds the Traces tab's (`MonitorTracesCopy`), so no
/// shell writes its own. Decoding is here so it is testable without the
/// dylib; `TCBridgeTests` checks it against the real export.
public struct MonitorScreensCopy: Decodable, Equatable, Sendable {
    public let computer: String
    public let commons: String
    public let waiting: String
    public let folders: String
    public let watched: String
    public let off: String
    public let on: String
    public let connected: String
    public let reduce: String
    public let enlarge: String
    public let calls: String
    public let models: String
    public let priced: String
    public let unknown: String
    public let proofVerified: String
    public let proofGatewayOnly: String
    public let proofUnattested: String
    public let proofPending: String
    public let proofUnavailable: String
    public let proofFailed: String
    public let proofOutside: String
    public let proofUnrecorded: String
    public let history: String
    public let contributed: String
    public let watching: String
    public let paused: String
    public let summary: String
    public let week: String
    public let month: String
    public let total: String
    public let held: String
    public let withdrawn: String
    public let credit: String
    public let creditFinal: String
    public let pending: String
    public let community: String
    public let rank: String
    public let window: String
    public let approved: String
    public let unrecorded: String
    public let missions: String
    public let contributionMode: String
    public let mixed: String
    public let shared: String
    public let kept: String
    public let recentActivity: String
    public let flagged: String
    public let manageRules: String
    public let settings: String
    /// The Settings modal's title, subtitle, section list name and close
    /// button (Ron's #1146 `SettingsModal`, #1241 Task 10).
    public let settingsTitle: String
    public let settingsSubtitle: String
    public let settingsSections: String
    public let close: String
    /// Ron's native shell words: the toolbar's View menu and Graph toggle,
    /// the View menu's ignored folders, the Traces graph's focus and
    /// period steps, and Home's pending-credit tile and Traces link.
    public let view: String
    public let graph: String
    public let showIgnoredFolders: String
    public let focus: String
    public let previous: String
    public let next: String
    public let creditPending: String
    public let openTraces: String
    public let quit: String
    public let coreUnreachable: String
    public let requestFailed: String
    public let heldForReview: String
    public let heldExplanation: String
    public let creditNotCurrency: String
    public let historyShownOf: String
    public let historyShown: String
    public let signedOut: String
    public let projected: String
    public let projectedNote: String
    /// `{min}`, `{max}`: a mission's range in `points`.
    public let missionCreditPoints: String
    /// `{min}`.
    public let missionCreditPointsOne: String
    /// Approved 2026-10-06. The window the Inference tab's counts cover;
    /// `{hours}` is replaced with a number. See `windowLine(hours:)`.
    public let windowLastHours: String
    /// History's word for a `submitted`
    /// contribution: waiting to be scored, not done.
    public let historySubmitted: String
    /// The queue's safeguards panel (Ron's #1146 `QueueStatusPanel`, #1241).
    public let safeguards: MonitorSafeguardsCopy
    /// History's refresh and account sign-in controls (Ron's #1146).
    public let historyActions: MonitorHistoryActionsCopy
    /// Ron's #1146 toolbar, Home and History words (owner ruling,
    /// 2026-10-06).
    public let shell: MonitorShellCopy

    enum CodingKeys: String, CodingKey {
        case computer
        case commons
        case waiting
        case folders
        case watched
        case off
        case on
        case connected
        case reduce
        case enlarge
        case calls
        case models
        case priced
        case unknown
        case proofVerified = "proof_verified"
        case proofGatewayOnly = "proof_gateway_only"
        case proofUnattested = "proof_unattested"
        case proofPending = "proof_pending"
        case proofUnavailable = "proof_unavailable"
        case proofFailed = "proof_failed"
        case proofOutside = "proof_outside"
        case proofUnrecorded = "proof_unrecorded"
        case history
        case contributed
        case watching
        case paused
        case summary
        case week
        case month
        case total
        case held
        case withdrawn
        case credit
        case creditFinal = "credit_final"
        case pending
        case community
        case rank
        case window
        case approved
        case unrecorded
        case missions
        case contributionMode = "contribution_mode"
        case mixed
        case shared
        case kept
        case recentActivity = "recent_activity"
        case flagged
        case manageRules = "manage_rules"
        case settings
        case settingsTitle = "settings_title"
        case settingsSubtitle = "settings_subtitle"
        case settingsSections = "settings_sections"
        case close
        case view
        case graph
        case showIgnoredFolders = "show_ignored_folders"
        case focus
        case previous
        case next
        case creditPending = "credit_pending"
        case openTraces = "open_traces"
        case quit
        case coreUnreachable = "core_unreachable"
        case requestFailed = "request_failed"
        case heldForReview = "held_for_review"
        case heldExplanation = "held_explanation"
        case creditNotCurrency = "credit_not_currency"
        case historyShownOf = "history_shown_of"
        case historyShown = "history_shown"
        case signedOut = "signed_out"
        case projected
        case projectedNote = "projected_note"
        case missionCreditPoints = "mission_credit_points"
        case missionCreditPointsOne = "mission_credit_points_one"
        case windowLastHours = "window_last_hours"
        case historySubmitted = "history_submitted"
        case safeguards
        case historyActions = "history_actions"
        case shell
    }

    /// The payload fields this shell decodes, by wire name.
    public static let consumedFields = [
        "computer",
        "commons",
        "waiting",
        "folders",
        "watched",
        "off",
        "on",
        "connected",
        "reduce",
        "enlarge",
        "calls",
        "models",
        "priced",
        "unknown",
        "proof_verified",
        "proof_gateway_only",
        "proof_unattested",
        "proof_pending",
        "proof_unavailable",
        "proof_failed",
        "proof_outside",
        "proof_unrecorded",
        "history",
        "contributed",
        "watching",
        "paused",
        "summary",
        "week",
        "month",
        "total",
        "held",
        "withdrawn",
        "credit",
        "credit_final",
        "pending",
        "community",
        "rank",
        "window",
        "approved",
        "unrecorded",
        "missions",
        "contribution_mode",
        "mixed",
        "shared",
        "kept",
        "recent_activity",
        "flagged",
        "manage_rules",
        "settings",
        "settings_title",
        "settings_subtitle",
        "settings_sections",
        "close",
        "view",
        "graph",
        "show_ignored_folders",
        "focus",
        "previous",
        "next",
        "credit_pending",
        "open_traces",
        "quit",
        "core_unreachable",
        "request_failed",
        "held_for_review",
        "held_explanation",
        "credit_not_currency",
        "history_shown_of",
        "history_shown",
        "signed_out",
        "projected",
        "projected_note",
        "mission_credit_points",
        "mission_credit_points_one",
        "window_last_hours",
        "history_submitted",
        "safeguards",
        "history_actions",
        "shell",
    ]

    /// Each nested table's wire fields, by its wire name.
    public static let consumedTables: [String: [String]] = [
        "safeguards": MonitorSafeguardsCopy.consumedFields,
        "history_actions": MonitorHistoryActionsCopy.consumedFields,
        "shell": MonitorShellCopy.consumedFields,
    ]

    /// Decode the payload, or nil if it will not parse or a field is empty.
    public static func decode(fromJSON json: String?) -> MonitorScreensCopy? {
        guard let data = json?.data(using: .utf8),
            let copy = try? JSONDecoder().decode(MonitorScreensCopy.self, from: data)
        else {
            return nil
        }
        let words = [copy.computer, copy.commons, copy.waiting, copy.folders, copy.watched, copy.off, copy.on, copy.connected, copy.reduce, copy.enlarge, copy.calls, copy.models, copy.priced, copy.unknown, copy.proofVerified, copy.proofGatewayOnly, copy.proofUnattested, copy.proofPending, copy.proofUnavailable, copy.proofFailed, copy.proofOutside, copy.proofUnrecorded, copy.history, copy.contributed, copy.watching, copy.paused, copy.summary, copy.week, copy.month, copy.total, copy.held, copy.withdrawn, copy.credit, copy.creditFinal, copy.pending, copy.community, copy.rank, copy.window, copy.approved, copy.unrecorded, copy.missions, copy.contributionMode, copy.mixed, copy.shared, copy.kept, copy.recentActivity, copy.flagged, copy.manageRules, copy.settings, copy.settingsTitle, copy.settingsSubtitle, copy.settingsSections, copy.close, copy.view, copy.graph, copy.showIgnoredFolders, copy.focus, copy.previous, copy.next, copy.creditPending, copy.openTraces, copy.quit, copy.coreUnreachable, copy.requestFailed, copy.heldForReview, copy.heldExplanation, copy.creditNotCurrency, copy.historyShownOf, copy.historyShown, copy.signedOut, copy.projected, copy.projectedNote, copy.missionCreditPoints, copy.missionCreditPointsOne, copy.windowLastHours, copy.historySubmitted]
        return words.contains(where: \.isEmpty) || !copy.safeguards.isWhole || !copy.historyActions.isWhole
            || !copy.shell.isWhole ? nil : copy
    }

    /// What a screen says for a failed read: the core's line for a core that
    /// does not answer, or for a request that failed. Never the error's own
    /// fixed label, which is for logs.
    public func historyCap(shown: Int, total: Int?) -> String {
        guard let total else { return historyShown.replacingOccurrences(of: "{shown}", with: String(shown)) }
        return historyShownOf
            .replacingOccurrences(of: "{shown}", with: String(shown))
            .replacingOccurrences(of: "{total}", with: String(total))
    }

    /// The window a count covers, in the core's words, from the hours the
    /// core reported for it. A dash when it reported none: an unknown
    /// window is never said as a default one.
    public func windowLine(hours: Int?) -> String {
        guard let hours else { return "\u{2014}" }
        return windowLastHours.replacingOccurrences(of: "{hours}", with: String(hours))
    }

    public func line(for error: DaemonDataError) -> String {
        if case .unreachable = error { return coreUnreachable }
        return requestFailed
    }
}
