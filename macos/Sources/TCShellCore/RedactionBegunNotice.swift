import Foundation

/// When to say "Data redaction has begun." (owner, 2026-11-05).
///
/// It is the only thing a person is told about the privacy witness's work,
/// and it is said once per run of the app -- from launch to quit -- on the
/// first trace the person submits themselves that goes to the witness.
/// Background and Automatic-folder submissions say nothing, and they do not
/// use up the once: only a foreground submission is counted.
///
/// Pure and in `TCShellCore` so the rule is unit-tested without a display.
/// The app holds one instance for the life of the process
/// (`RedactionBegunNotice.appRun`); a reattached daemon client is not a new
/// run of the app.
public final class RedactionBegunNotice: @unchecked Sendable {
    /// Who made the submission.
    public enum Submission: Sendable {
        /// The person pressed Submit while the app was open.
        case foreground
        /// Anything the app did not do at a person's press: an
        /// Automatic-folder or other background submission.
        case background
    }

    /// The instance for this run of the app.
    public static let appRun = RedactionBegunNotice()

    private let lock = NSLock()
    private var said = false

    public init() {}

    /// Whether to show the line for this submission, and if so, count it
    /// as said. `witnessReview` is the approve response's `witness_review`:
    /// the submission went to the privacy witness.
    public func claim(_ submission: Submission, witnessReview: Bool) -> Bool {
        guard submission == .foreground, witnessReview else { return false }
        lock.lock()
        defer { lock.unlock() }
        if said { return false }
        said = true
        return true
    }

    /// The line to show for this submission, or nil.
    public func line(_ submission: Submission, witnessReview: Bool, words: MonitorUndoCopy?) -> String? {
        guard let words, claim(submission, witnessReview: witnessReview) else { return nil }
        return words.redactionBegun
    }
}
