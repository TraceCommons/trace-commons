import Foundation

/// The three tables of the disclosure bundle the monitor reads, decoded from
/// `tc_contributor_disclosure_copy_json`
/// (`disclosure_copy::contributor_disclosure_copy`): the verdict and
/// correction words (`outcome`), History's words (`history_ui`), and each
/// folder mode's one name (`folder_mode_labels`). The bundle's other tables
/// are not decoded here.
///
/// Ron's #1146 inspector reads these through `useContributorDisclosureCopy`;
/// this is the same read for macOS (#1241). Decoding is here rather than in
/// `TCBridge` so it can be tested without linking the dylib;
/// `ContributorDisclosureCopyExportTests` checks it against the real export.
public struct ContributorDisclosureCopy: Decodable, Equatable, Sendable {
    /// `outcome_copy::OutcomeCopy`: the verdict question and its three
    /// answers, the correction field, and Submit-all-as.
    public struct Outcome: Decodable, Equatable, Sendable {
        public let verdictQuestion: String
        public let worked: String
        public let partly: String
        public let failed: String
        /// The verdict's empty answer, offered to assistive tech, which
        /// takes an answer back.
        public let verdictNone: String
        public let verdictCaption: String
        public let correctionQuestion: String
        public let correctionPlaceholder: String
        public let correctionCaption: String
        public let correctionCredentialHeadline: String
        public let correctionCredentialBody: String
        public let submitAllAs: String
        public let submitAllAsTooltip: String
        /// The longest correction the daemon accepts, in characters.
        public let maxCorrectionChars: Int

        enum CodingKeys: String, CodingKey {
            case verdictQuestion = "verdict_question"
            case worked, partly, failed
            case verdictNone = "verdict_none"
            case verdictCaption = "verdict_caption"
            case correctionQuestion = "correction_question"
            case correctionPlaceholder = "correction_placeholder"
            case correctionCaption = "correction_caption"
            case correctionCredentialHeadline = "correction_credential_headline"
            case correctionCredentialBody = "correction_credential_body"
            case submitAllAs = "submit_all_as"
            case submitAllAsTooltip = "submit_all_as_tooltip"
            case maxCorrectionChars = "max_correction_chars"
        }

        var words: [String] {
            [
                verdictQuestion, worked, partly, failed, verdictNone, verdictCaption, correctionQuestion,
                correctionPlaceholder, correctionCaption, correctionCredentialHeadline,
                correctionCredentialBody, submitAllAs, submitAllAsTooltip,
            ]
        }
    }

    /// History's words: the held row's body, the status labels by wire
    /// status, and the word for a status this build does not know.
    public struct HistoryUi: Decodable, Equatable, Sendable {
        public let heldRowBody: String
        /// Optional in the core: the label is looked up in its status table.
        public let statusAwaitingPiiBackstop: String?
        public let statusUnavailable: String
        public let statusLabels: [String: String]

        enum CodingKeys: String, CodingKey {
            case heldRowBody = "held_row_body"
            case statusAwaitingPiiBackstop = "status_awaiting_pii_backstop"
            case statusUnavailable = "status_unavailable"
            case statusLabels = "status_labels"
        }
    }

    public let outcome: Outcome
    public let historyUi: HistoryUi
    /// Each folder mode's one name, keyed by wire mode (`notify_only`,
    /// `auto_upload`, `ignore`).
    public let folderModeLabels: [String: String]

    enum CodingKeys: String, CodingKey {
        case outcome
        case historyUi = "history_ui"
        case folderModeLabels = "folder_mode_labels"
    }

    /// Decode the bundle, or nil if it will not parse, a word is empty, or
    /// the correction limit is not positive.
    public static func decode(fromJSON json: String?) -> ContributorDisclosureCopy? {
        guard let data = json?.data(using: .utf8),
            let copy = try? JSONDecoder().decode(ContributorDisclosureCopy.self, from: data)
        else {
            return nil
        }
        var words = copy.outcome.words
        words += [copy.historyUi.heldRowBody, copy.historyUi.statusUnavailable]
        words += Array(copy.historyUi.statusLabels.values) + Array(copy.folderModeLabels.values)
        guard !words.contains(where: \.isEmpty), copy.outcome.maxCorrectionChars > 0 else { return nil }
        return copy
    }
}
