import TCBridge
import TCShellCore

/// The core's table of the words this shell used to write itself
/// (`ShellWordsCopy`, `tc_shell_words_copy_json`), decoded once.
///
/// `WithdrawalCopy`, `PublicProfileCopy`, `QueueLegacyWords`,
/// `HistoryLegacyWords`, `ScrubbingCaveat` and `SettingsLegacyWords` read
/// it; none of them holds a sentence. With no table a word is empty, and
/// the surfaces that must not run on empty words (withdrawal, going public)
/// refuse instead.
enum ShellWords {
    static let table: ShellWordsCopy? = ShellWordsCopy.decode(fromJSON: TCCoreCopy.shellWordsCopyJSON())

    /// A wording-defect notice's title when this table did not decode,
    /// which is exactly when the notice shows: the core's "Status
    /// unavailable", from its disclosure table, so the notice is never
    /// titled with an empty string. Nil only when neither table decoded.
    static var unavailableTitle: String? {
        TracesStore.disclosureCopy?.historyUi.statusUnavailable
    }

    /// A defect notice's title: the table's own words, else
    /// `unavailableTitle`; never "".
    static func defectTitle(_ word: String?) -> String? {
        word.flatMap { $0.isEmpty ? nil : $0 } ?? unavailableTitle
    }

    static func fill(_ template: String, _ values: [String: String]) -> String {
        ShellWordsCopy.fill(template, values)
    }
}
