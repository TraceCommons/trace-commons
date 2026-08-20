import Foundation

/// The submit toast: the daemon's counts, said in one sentence.
///
/// One-click submit sends a session nobody previewed, so the toast is the
/// only place a contributor learns what happened -- what went, what
/// scrubbing did to it, what was held, and what never left. That makes the
/// wording a contract rather than a presentation detail, and it is fixed in
/// `docs/superpowers/specs/2026-08-20-one-click-submit-design.md` under
/// "The toast: normative copy". The strings below are transcribed from that
/// section, not paraphrased from it.
///
/// Three shells render this sentence and they must render it identically.
/// The Linux copy is `crates/trace-commons-contributor-gtk/src/toast.rs`
/// (with its clause strings in `copy.rs`), the Windows copy is
/// `windows/src/TraceCommons.Interop/SubmitToast.cs`, and all three assert
/// the spec's four worked examples for exactly one reason: a sentence
/// reworded in one client is the drift that section exists to prevent.
///
/// This lives in `TCShellCore` rather than in a view for the same reason
/// `StateDirectory` does -- the app target links the FFI dylib, so nothing
/// in it is a unit test. Deliberately pure: counts in, a string and a Bool
/// out, no I/O and no UI types, so the assertion that the three shells
/// agree runs without a display.
public struct SubmitToast: Equatable, Sendable {
    /// The whole sentence, ready to display.
    public let line: String

    /// Whether to offer Undo alongside it.
    ///
    /// True only when something was actually sent. Offering Undo on any
    /// successful response was correct while every approval succeeded and
    /// is wrong now that entries can be skipped: a skipped entry with an
    /// undo timer behind it reads as sent.
    public let offerUndo: Bool

    public init(line: String, offerUndo: Bool) {
        self.line = line
        self.offerUndo = offerUndo
    }

    /// Render the toast from an `approve` response.
    ///
    /// `redactions` is the sum of the response's `redactions` map -- the
    /// toast names a count, never a category, because the preview sheet is
    /// where a contributor sees which detector fired. `skipped` is the
    /// response's wire reason labels, in response order; the rendered
    /// sentence uses the human labels, distinct, in the spec table's order,
    /// so neither a wire label nor an entry id ever reaches a contributor.
    public static func render(
        approved: UInt64,
        redactions: UInt64,
        flagged: UInt64,
        skipped: [String]
    ) -> SubmitToast {
        var clauses = [sentClause(approved), scrubClause(redactions)]

        if flagged > 0 {
            clauses.append(flaggedClause(flagged))
        }

        if !skipped.isEmpty {
            clauses.append(skippedClause(skipped))
        }

        return SubmitToast(line: clauses.joined(separator: " "), offerUndo: approved > 0)
    }

    // --- The clauses ---------------------------------------------------

    /// Clause 1: what was sent.
    static func sentClause(_ approved: UInt64) -> String {
        switch approved {
        case 0: return "Nothing sent."
        case 1: return "Sent."
        default: return "Sent \(approved) sessions."
        }
    }

    /// Clause 2: what scrubbing did.
    ///
    /// Always present, including when it did nothing. A count of zero is a
    /// fact the contributor is owed, not an absence to omit -- and it is
    /// the case worth weighing, which is why it is never silently dropped.
    ///
    /// The count is the sum of the response's `redactions` map. Categories
    /// are deliberately not named here; the preview sheet is where a
    /// contributor sees which detector fired.
    static func scrubClause(_ totalRedactions: UInt64) -> String {
        switch totalRedactions {
        case 0: return "Scrubbing matched nothing."
        case 1: return "Scrubbing removed 1 thing."
        default: return "Scrubbing removed \(totalRedactions) things."
        }
    }

    /// Clause 3: what was flagged. Rendered only when `flagged > 0`.
    static func flaggedClause(_ flagged: UInt64) -> String {
        "\(flagged) flagged."
    }

    /// Clause 4: what was not sent. Rendered only when something was
    /// skipped.
    ///
    /// The count is entries; the list is distinct reasons. Those are
    /// different numbers whenever several entries were skipped for the same
    /// reason, and the sentence says both because a contributor needs the
    /// first to know how much is still queued and the second to know what
    /// to do about it.
    static func skippedClause(_ skipped: [String]) -> String {
        var reasons: [String] = []
        for (_, human) in skipReasons where skipped.contains(where: { reasonLabel($0) == human }) {
            reasons.append(human)
        }
        if skipped.contains(where: { reasonLabel($0) == skipReasonUnknown }) {
            reasons.append(skipReasonUnknown)
        }

        return "\(skipped.count) not sent: \(reasons.joined(separator: ", "))."
    }

    // --- The reason table ----------------------------------------------

    /// The human label for each wire reason an entry can be skipped for, in
    /// the spec table's order -- which is also the order they are listed in
    /// when several apply.
    ///
    /// The wire spellings are the daemon's and belong to the protocol; the
    /// human halves belong to the contributor. Nothing here ever shows the
    /// left-hand column, and nothing here shows an entry id: an id in a
    /// toast is noise a contributor cannot act on.
    ///
    /// An array of pairs rather than a dictionary, because the order is
    /// part of the contract and a dictionary has none.
    public static let skipReasons: [(wire: String, human: String)] = [
        ("not-enrolled", "not connected to a commons"),
        ("not-pending", "already decided"),
        ("not-pinned", "could not be prepared"),
        ("envelope-too-large", "too large to send"),
        ("session-file-vanished", "the session file is gone"),
        ("preview-failed", "could not be read"),
    ]

    /// What an unrecognised wire label is called instead of itself.
    ///
    /// The spec's table is closed today, but a daemon newer than the shell
    /// can send a label this build has never been taught, and the one thing
    /// that must not then happen is the shell echoing protocol vocabulary
    /// at a contributor. So an unknown label degrades to the least specific
    /// true statement available, and is listed last.
    public static let skipReasonUnknown = "could not be sent"

    /// Translate one wire reason label. Never returns its argument.
    public static func reasonLabel(_ wire: String) -> String {
        skipReasons.first { $0.wire == wire }?.human ?? skipReasonUnknown
    }
}
