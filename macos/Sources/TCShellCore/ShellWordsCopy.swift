import Foundation

/// The words this shell used to write in Swift -- withdrawal, the public
/// profile, the legacy queue and History words, the scrubbing caveat and
/// the Settings sections' sentences -- decoded from
/// `tc_shell_words_copy_json` (`shell_words_copy::shell_words_copy`).
///
/// Decoding is here so it is testable without the dylib; `TCBridgeTests`
/// checks it against the real export. A table with any empty string is
/// refused whole: a blank sentence on a withdrawal or a public-profile
/// screen is worse than none. The only exceptions are the lines the owner
/// cut (`ownerCut`), which arrive "" and are drawn as nothing.
public struct ShellWordsCopy: Decodable, Equatable, Sendable {
    public struct Withdrawal: Decodable, Equatable, Sendable {
        public let withdraw: String
        public let confirmTitle: String
        public let confirmDescription: String
        public let keep: String
        public let confirm: String
        public let withdrawing: String
        public let disclosureUnavailable: String
        public let ambiguity: String
        public let notDistributed: String
        public let commonsNotDistributed: String
        public let commonsDistributed: String
        public let creditNote: String
        public let resultHeading: String
        public let resultNotDistributed: String
        public let resultCommonsNotDistributed: String
        public let resultCommonsDistributed: String
        public let resultUnknown: String
        public let accountSessionRequired: String
        public let notFound: String
        public let notFoundLabels: [String]
        public let failed: String
        public let tryAgain: String
        public let noBulkAction: String
        public let wordingDefect: String
    }

    public struct PublicProfile: Decodable, Equatable, Sendable {
        public let heading: String
        public let listHandlePublicly: String
        public let footnote: String
        public let handle: String
        public let bio: String
        public let updateProfile: String
        public let withdraw: String
        /// Withdraw's accessible name, naming the profile (rule 5).
        public let withdrawAccessibilityName: String
        public let onRosterSince: String
        public let goPublicHeadline: String
        public let goPublicDescription: String
        public let goPublic: String
        /// The card's button that opens the dialog; `goPublic` confirms it.
        public let goPublicOpen: String
        public let goingPublic: String
        public let notNow: String
        public let publishedHeading: String
        public let publishedLines: [String]
        public let neverHeading: String
        public let neverLines: [String]
        public let acknowledgement: String
        public let goPublicFootnote: String
        public let goPublicHandle: String
        public let goPublicBio: String
        public let published: String
        public let publishedNotCached: String
        public let leftRoster: String
        public let leftRosterNotCached: String
        public let failure: String
        public let failureDefault: String
        public let failureReasons: [String: String]
        public let leaveFailure: String
        public let leaveNotConnected: String
        public let wordingDefect: String
    }

    public struct Queue: Decodable, Equatable, Sendable {
        public let nothingWaiting: String
        public let nothingWaitingDetail: String
        public let undoWillSend: String
        public let closeNoticeStillSends: String
        public let closeNotice: String
        public let notOfferedScope: String
        public let undo: String
        public let agentSetup: String
        public let approvedAgo: String
        public let approvedAgoCeiling: String
        public let noLongerWaiting: String
    }

    public struct History: Decodable, Equatable, Sendable {
        public let typicalWait: String
    }

    public struct Scrubbing: Decodable, Equatable, Sendable {
        public let canonical: String
        public let nothingMatched: String
        public let removed: String
        public let beforeYouContribute: String
    }

    public struct Settings: Decodable, Equatable, Sendable {
        public let consentHeading: String
        public let connected: String
        public let notConnected: String
        public let queuedNothingSent: String
        public let extraScanConfigured: String
        public let sessionFinishedAfter: String
        public let atMostOneNotification: String
        public let atMostOneNotificationOne: String
        public let undecidedDropped: String
        public let stateYes: String
        public let stateNo: String
        public let waitingOnApproval: String
        public let turnOnInSystemSettings: String
        public let couldNotTurnOn: String
        public let couldNotTurnOff: String
        public let version: String
        public let checkNow: String
        public let copy: String
        public let checksDaily: String
        public let checksAutomatically: String
        public let managedByHomebrew: String
        public let homebrewReplaces: String
        public let updatesUnavailable: String
        public let notCheckedYet: String
        public let lastChecked: String
        public let noFeed: String
        public let insecureFeed: String
        public let updatesOff: String
        public let notificationsRenderedHere: String
        public let pausedNothingSent: String
        public let appliesFromNow: String
        public let alwaysIncluded: String
        public let optionalDataUse: String
        public let rewardOpportunities: String?
        /// The hover tip beside `rewardOpportunities`: what credit depends
        /// on. Optional so an older core still decodes.
        /// Approved by the owner 2026-11-04 (Settings copy pass 2, P137).
        public let rewardOpportunitiesTooltip: String?
        public let credit: String
        public let required: String
        public let nothingPreselected: String
        public let nothingChanged: String
        /// A refused folder mode change. Optional so an older core still
        /// decodes. Approved by the owner 2026-11-04 (Settings copy pass 2, P136).
        public let projectModeFailed: String?
        public let auditActions: [String: String]
        public let auditChanged: String
    }

    public let withdrawal: Withdrawal
    public let publicProfile: PublicProfile
    public let queue: Queue
    public let history: History
    public let scrubbing: Scrubbing
    public let settings: Settings
    public let pausedNotice: PausedNotice
    /// Settings > Projects' summary and Manage sheet (approved by the owner
    /// 2026-11-04, #1952). Optional so an older core still decodes; the
    /// card and the sheet draw nothing that needs a word it lacks.
    public let projectsManager: ProjectsManager?

    /// `shell_words_copy::ProjectsManagerWords`.
    public struct ProjectsManager: Decodable, Equatable, Sendable {
        public let manage: String
        public let search: String
        public let all: String
        /// `{title} {count}`: one mode's name and how many projects are in it.
        public let modeCount: String
        /// `{count} selected`.
        public let selected: String
        public let setTo: String
        /// `{count} projects`: several projects as one name.
        public let projectsCount: String
        public let noMatches: String
        /// `{title}, {count} projects`: a group header's accessible name.
        public let groupAccessibility: String
        /// `Select {title}`: a project's checkbox's accessible name.
        public let selectAccessibility: String
    }

    /// The Traces tab's resume notice (approved 2026-11-04): Settings'
    /// paused sentence, Resume and Not now. When it shows is
    /// `PausedResumeNotice`'s.
    public struct PausedNotice: Decodable, Equatable, Sendable {
        public let body: String
        public let resume: String
        public let notNow: String
    }

    /// Fill a string's `{name}` holes, each with its value; adds nothing.
    public static func fill(_ template: String, _ values: [String: String]) -> String {
        values.reduce(template) { text, pair in
            text.replacingOccurrences(of: "{" + pair.key + "}", with: pair.value)
        }
    }

    /// The lines the owner cut 2026-11-04 (Settings copy pass 2, P142,
    /// P144, P235, P236, P244), by wire path: each arrives "", the only
    /// strings in the table allowed to, and a view draws them as nothing.
    public static let ownerCut: Set<String> = [
        "public_profile.footnote",
        "public_profile.go_public_footnote",
        "settings.at_most_one_notification",
        "settings.at_most_one_notification_one",
        "settings.notifications_rendered_here",
    ]

    /// Decode the table, or nil if it will not parse or any string in it
    /// other than an `ownerCut` line is empty.
    public static func decode(fromJSON json: String?) -> ShellWordsCopy? {
        guard let data = json?.data(using: .utf8),
            let tree = try? JSONSerialization.jsonObject(with: data),
            !hasEmptyString(tree, path: "")
        else {
            return nil
        }
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try? decoder.decode(ShellWordsCopy.self, from: data)
    }

    private static func hasEmptyString(_ node: Any, path: String) -> Bool {
        switch node {
        case let text as String: return text.isEmpty && !ownerCut.contains(path)
        case let list as [Any]: return list.contains { hasEmptyString($0, path: path + "[]") }
        case let map as [String: Any]:
            return map.contains { key, value in
                hasEmptyString(value, path: path.isEmpty ? key : path + "." + key)
            }
        default: return false
        }
    }
}
