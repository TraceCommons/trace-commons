// The legacy folder row's words, at the path the wording ratchet
// (`ShellWordingTests`) records them under. The row itself is gone (R15);
// the glass Traces folder row reads these.

/// The folder row's sentences the glass Traces row also draws, held once.
enum QueueFolderWords {
    static func submitAll(_ count: Int) -> String { "Submit all (\(count))" }

    static func submitAllHelp(_ label: String) -> String {
        """
        Submits every session in \(label) that can be sent. Each is \
        scrubbed the same way a single Submit would be, and flagged \
        sessions are included, not held back.
        """
    }
}
