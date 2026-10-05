import Foundation

/// The #1030 first run's words (`tc_first_run_copy_json`,
/// `first_run_copy::first_run_copy`), grouped by screen. Every field is the
/// core's; a shell fills the `{...}` placeholders and adds nothing.
///
/// Keys arrive in snake case and decode by the converting strategy, so this
/// file names no wire key. Decoding is here so it is testable without the
/// dylib; `TCBridgeTests` checks it against the real export.
public struct FirstRunCopy: Decodable, Equatable, Sendable {
    public struct Frame: Decodable, Equatable, Sendable {
        public let quickSetup: String
        public let customSetup: String
        public let stepJoin: String
        public let stepFolders: String
        public let stepTools: String
        public let stepRules: String
        public let stepUses: String
        public let customSetupInstead: String
        public let continueButton: String
        public let answerEveryTool: String
    }

    public struct Join: Decodable, Equatable, Sendable {
        public let titleLight: String
        public let titleBold: String
        public let body: String
        public let bodyEmphasis: String
        public let inviteEyebrow: String
        public let invitePlaceholder: String
        public let lookUp: String
        /// `{host}` and `{pay_range}`.
        public let inviteJoined: String
        public let inviteError: String
        public let passkeyEyebrow: String
        public let passkeyText: String
        /// `{name}`.
        public let passkeyReady: String
        public let passkeyCreate: String
        public let passkeyDone: String
        /// Create passkey chosen, created once the daemon starts.
        public let passkeyChosen: String
        public let passkeyUndo: String
        public let nearAiEyebrow: String
        public let nearAiText: String
        public let nearAiSignIn: String
        /// near.ai chosen, signed in once the daemon starts.
        public let nearAiChosen: String
        public let nearAiUndo: String
        public let signedIn: String
        public let noSharing: String
        public let skipNote: String
        public let skip: String
        public let signedOut: String
    }

    public struct Folders: Decodable, Equatable, Sendable {
        public let titleLight: String
        public let titleBold: String
        public let body: String
        public let loading: String
        public let watch: String
        public let dontUse: String
        /// `{tool}` in each of the next four.
        public let watchQuestion: String
        public let chooseFolder: String
        public let getTool: String
        public let downloadTool: String
        public let notInstalled: String
        /// Discovery returned no row the shell could read.
        public let discoveryFailed: String
        public let retry: String
        /// Enroll was refused after the invite's lookup accepted it.
        public let enrollRefused: String
    }

    public struct Tools: Decodable, Equatable, Sendable {
        public let titleLight: String
        public let titleBold: String
        public let addTool: String
        public let addToolCaption: String
        public let addToolRefused: String
        public let addedByYou: String
        /// `{folder}`: a picked folder matching more than one kind.
        public let whichKind: String
        /// A folder of exported traces, as an option and as its row's name.
        public let trajectoryLabel: String
        /// `{count}`.
        public let sessionCount: String
    }

    public struct Rules: Decodable, Equatable, Sendable {
        public let titleLight: String
        public let titleBold: String
        public let loading: String
        public let empty: String
        /// `{tools}`.
        public let reposFound: String
        /// `{count}`.
        public let sessionCount: String
        /// `{folder}`.
        public let ruleFor: String
        public let pastSessions: String
        /// `{selected}` and `{total}`.
        public let selectedSummary: String
        public let folderSelected: String
        /// `{folder}`.
        public let includeEvery: String
        /// `{count}`.
        public let showAll: String
        public let showFewer: String
        /// `{count}`.
        public let neverCount: String
        /// `{folder}`.
        public let neverLabel: String
        /// The folders could not be read.
        public let unavailable: String
        /// One folder's past sessions could not be read.
        public let sessionsUnavailable: String
    }

    public struct Uses: Decodable, Equatable, Sendable {
        public let titleLight: String
        public let titleBold: String
        public let eyebrow: String
        public let required: String
        public let allOptional: String
        /// `{count}`, and `{selected}` in the third.
        public let optionalAllOn: String
        public let optionalAllOff: String
        public let optionalSomeOn: String
        public let sharing: String
        public let sharingLoading: String
        public let sharingUnavailable: String
        public let baseUseNote: String
        public let start: String
    }

    public struct Passkey: Decodable, Equatable, Sendable {
        public let back: String
        public let close: String
        public let cancel: String
        public let chooseTitle: String
        public let useExisting: String
        public let createNew: String
        public let chooseNote: String
        public let nameTitle: String
        public let nameField: String
        public let clearName: String
        public let defaultName: String
        public let nameWarning: String
        public let nameEmpty: String
        /// `{max}`.
        public let nameTooLong: String
        public let verifyTitle: String
        public let verifyBody: String
        public let verify: String
        public let verifyNote: String
        public let welcomeTitle: String
        public let welcomeBody: String
        public let welcomeSignIn: String
        public let otherOptions: String
    }

    public struct PrivateAi: Decodable, Equatable, Sendable {
        public let loading: String
        public let unavailable: String
        public let toggleLoading: String
    }

    public let frame: Frame
    public let join: Join
    public let folders: Folders
    public let tools: Tools
    public let rules: Rules
    public let uses: Uses
    public let passkey: Passkey
    public let privateAi: PrivateAi

    /// Decode the table, or nil if it will not parse or any string in it is
    /// empty: a first run with a blank control is refused, not shown.
    public static func decode(_ json: String) -> FirstRunCopy? {
        guard let data = json.data(using: .utf8),
            let tree = try? JSONSerialization.jsonObject(with: data),
            !hasEmptyString(tree)
        else {
            return nil
        }
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try? decoder.decode(FirstRunCopy.self, from: data)
    }

    private static func hasEmptyString(_ node: Any) -> Bool {
        switch node {
        case let text as String:
            return text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        case let map as [String: Any]:
            return map.values.contains(where: hasEmptyString)
        case let list as [Any]:
            return list.contains(where: hasEmptyString)
        default:
            return false
        }
    }
}
