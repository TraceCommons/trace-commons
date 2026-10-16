import TCShellCore

// The legacy queue's words. The screen itself is gone (R15): the Traces tab
// draws on glass from `Views/Monitor/`. What it still reads of the old
// queue's is the core's now (#1146 parity, 2026-10-07).

/// The queue's sentences the glass Traces tab draws (`TracesOffersBar`,
/// the session inspector), read from the core's table
/// (`shell_words_copy::queue_words`, through `ShellWords`).
enum QueueLegacyWords {
    private static var words: ShellWordsCopy.Queue? { ShellWords.table?.queue }

    static var nothingWaiting: String { words?.nothingWaiting ?? "" }
    static var nothingWaitingDetail: String { words?.nothingWaitingDetail ?? "" }
    static var undoWillSend: String { words?.undoWillSend ?? "" }
    static var closeNoticeStillSends: String { words?.closeNoticeStillSends ?? "" }
    static var closeNotice: String { words?.closeNotice ?? "" }
    static var notOfferedScope: String { words?.notOfferedScope ?? "" }
    static var undo: String { words?.undo ?? "" }
    static var agentSetup: String { words?.agentSetup ?? "" }

    /// Counts up from a real instant and stops at the ceiling: the deadline
    /// is the daemon's next upload sweep, which nothing here can observe.
    static func approvedAgo(_ seconds: Int) -> String {
        guard let words else { return "" }
        return seconds >= AppModel.Undo.tickCeiling
            ? ShellWords.fill(words.approvedAgoCeiling, ["seconds": String(AppModel.Undo.tickCeiling)])
            : ShellWords.fill(words.approvedAgo, ["seconds": String(seconds)])
    }

    static func noLongerWaiting(_ count: Int) -> String {
        ShellWords.fill(words?.noLongerWaiting ?? "", ["count": String(count)])
    }
}
