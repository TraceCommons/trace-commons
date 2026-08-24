import Foundation

/// Copy for declining a whole project from the Waiting screen.
///
/// A tested unit rather than string interpolation at the call site: this
/// text is written three times across three shells, and plural agreement is
/// the first thing to drift.
public enum ProjectIgnoreCopy {
    public static let buttonLabel = "Ignore project"

    public static func confirmationTitle(project: String) -> String {
        "Ignore \(project)?"
    }

    /// The removal clause is dropped entirely when nothing is waiting: a
    /// group can render with every card approved or uploading, and
    /// "removes 0 waiting traces" would be both wrong and alarming.
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
        return "This removes \(pendingCount) waiting \(noun). It also stops this project "
            + "being offered. \(tail)"
    }
}
