import Foundation

/// Copy for declining a whole project from the Waiting screen.
///
/// A tested unit rather than string interpolation at the call site: this
/// text is written three times across three shells, and plural agreement is
/// the first thing to drift.
public enum ProjectIgnoreCopy {
    public static let buttonLabel = "Ignore project"

    /// Word for word what GTK and Windows put on the same button:
    /// `copy::IGNORE_PROJECT_TOOLTIP` and `ProjectIgnoreCopy.Tooltip`. It
    /// lives here rather than at the call site for the same reason the body
    /// does — a tooltip nobody tests is the first thing to drift.
    public static let tooltip = "Stops this project being offered and clears what it has waiting. "
        + "Anything already submitted is unaffected, and you can undo this in Settings."

    public static func confirmationTitle(project: String) -> String {
        "Ignore \(project)?"
    }

    /// The removal clause is dropped entirely when nothing is waiting.
    ///
    /// No group renders that way today: all three shells build their groups
    /// from the pending list alone, so a group that renders has at least one
    /// waiting session in it. The branch is kept because this function is
    /// handed a number and must be right about whatever number it is handed
    /// — "removes 0 waiting traces" would be both wrong and alarming — not
    /// because a caller is known to produce zero.
    ///
    /// The last two sentences are load-bearing. One bounds the blast radius,
    /// the other names the way back — which is what lets the action itself
    /// be quiet.
    public static func confirmationBody(project: String, pendingCount: Int) -> String {
        let tail = "Nothing already submitted is affected. You can undo this in Settings."
        if pendingCount <= 0 {
            return "Stops this project being offered. \(tail)"
        }
        let noun = pendingCount == 1 ? "trace" : "traces"
        return "This removes \(pendingCount) waiting \(noun) and stops this project "
            + "being offered. \(tail)"
    }

    /// What is said afterwards when the daemon removed a different number
    /// than the confirmation named.
    ///
    /// The dialog has to state a count before the call is made, so it states
    /// the one this shell can see. The queue is live: a poll between the
    /// render and the click adds waiting sessions, an approval elsewhere
    /// removes one, and the daemon acts on what is there when it gets the
    /// message. `purged` is that number and it is the authority; the promise
    /// was an estimate.
    ///
    /// `nil` when the two agree, which is the ordinary case — a line that
    /// appears every time to say nothing happened is noise, and noise is how
    /// a line that matters gets skipped.
    public static func reconciliation(project: String, promised: Int, purged: Int) -> String? {
        if purged == promised {
            return nil
        }
        let clause = purged == 1
            ? "1 waiting trace was removed"
            : "\(purged) waiting traces were removed"
        return "Ignored \(project). The queue changed while you were deciding: "
            + "\(clause), not \(promised)."
    }
}
