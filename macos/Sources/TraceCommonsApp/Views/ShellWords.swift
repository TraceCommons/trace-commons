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

    static func fill(_ template: String, _ values: [String: String]) -> String {
        ShellWordsCopy.fill(template, values)
    }
}
