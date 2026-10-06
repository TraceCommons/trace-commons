// The legacy queue's words, at the path the wording ratchet
// (`ShellWordingTests`) records them under.
//
// The screen itself is gone (R15): the Traces tab draws on glass from
// `Views/Monitor/`, and those files author no sentence. What it still reads
// of the old queue's is held here, verbatim, until the core exports it.

/// The queue's sentences the glass Traces tab draws (`TracesOffersBar`,
/// the session inspector), held once.
enum QueueLegacyWords {
    static let nothingWaiting = "Nothing is waiting."
    static let nothingWaitingDetail = """
    When a session finishes and goes quiet, it shows up here. \
    Nothing is sent unless you say so.
    """
    static let undoWillSend = """
    Approved sessions will send automatically. You can undo until uploading starts.
    """
    static let closeNoticeStillSends = "Close this notice. Approved sessions will still send automatically."
    static let closeNotice = "Close this notice."
    static let notOfferedScope = """
    This covers sessions that reached the queue. Sessions that were \
    never queued at all are not counted here.
    """
    static let undo = "Undo"
    static let lookInside = "Look inside"
    static let agentSetup = "Agent setup"

    /// Counts up from a real instant and stops at the ceiling: the deadline
    /// is the daemon's next upload sweep, which nothing here can observe.
    static func approvedAgo(_ seconds: Int) -> String {
        seconds >= AppModel.Undo.tickCeiling
            ? "Approved \(AppModel.Undo.tickCeiling)s+ ago"
            : "Approved \(seconds)s ago"
    }

    static func noLongerWaiting(_ count: Int) -> String {
        "Sessions no longer waiting (\(count))"
    }
}
