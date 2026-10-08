import Foundation

/// The words this shell used to write in Swift -- withdrawal, the public
/// profile, the legacy queue and History words, the scrubbing caveat and
/// the Settings sections' sentences -- decoded from
/// `tc_shell_words_copy_json` (`shell_words_copy::shell_words_copy`).
///
/// Decoding is here so it is testable without the dylib; `TCBridgeTests`
/// checks it against the real export. A table with any empty string is
/// refused whole: a blank sentence on a withdrawal or a public-profile
/// screen is worse than none.
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
        public let onRosterSince: String
        public let goPublicHeadline: String
        public let goPublicDescription: String
        public let goPublic: String
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
        public let credit: String
        public let required: String
        public let nothingPreselected: String
        public let nothingChanged: String
        public let auditActions: [String: String]
        public let auditChanged: String
    }

    public let withdrawal: Withdrawal
    public let publicProfile: PublicProfile
    public let queue: Queue
    public let history: History
    public let scrubbing: Scrubbing
    public let settings: Settings

    /// Fill a string's `{name}` holes, each with its value; adds nothing.
    public static func fill(_ template: String, _ values: [String: String]) -> String {
        values.reduce(template) { text, pair in
            text.replacingOccurrences(of: "{" + pair.key + "}", with: pair.value)
        }
    }

    /// Decode the table, or nil if it will not parse or any string in it is
    /// empty.
    public static func decode(fromJSON json: String?) -> ShellWordsCopy? {
        guard let data = json?.data(using: .utf8),
            let tree = try? JSONSerialization.jsonObject(with: data),
            !hasEmptyString(tree)
        else {
            return nil
        }
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try? decoder.decode(ShellWordsCopy.self, from: data)
    }

    private static func hasEmptyString(_ node: Any) -> Bool {
        switch node {
        case let text as String: return text.isEmpty
        case let list as [Any]: return list.contains(where: hasEmptyString)
        case let map as [String: Any]: return map.values.contains(where: hasEmptyString)
        default: return false
        }
    }
}
