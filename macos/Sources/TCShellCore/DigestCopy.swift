import Foundation

/// The digest's words.
///
/// The digest could once say only one thing -- how many sessions are waiting
/// for review -- which was complete while every upload passed through review.
/// It stopped being complete when a project could be armed to contribute
/// without asking: an armed project queues nothing, so the waiting count is
/// permanently zero and the contributor who most wanted to stop supervising
/// was told nothing at all.
///
/// `contributionLine` is the half that says what went without them. The
/// daemon composes the same sentence for its own local notifier
/// (`crates/trace-commons-contributor/src/daemon/notify.rs`,
/// `contribution_text`), as do Windows (`DigestText.ContributionLine`) and
/// Linux (`notify::contribution_body`); the four must not drift. All name a
/// project only when exactly one distinct, non-blank label is present (the
/// WYSIWYG design's evening-digest examples, K9 #1118), and each pins the
/// design's two example sentences verbatim.
public enum DigestCopy {
    /// Nil when nothing was contributed -- the caller then has only the
    /// waiting half, or nothing to say at all. A line reading "0 sessions
    /// contributed" is worse than no line.
    public static func contributionLine(
        count: Int,
        projects: [String],
        creditPending: Double
    ) -> String? {
        guard count > 0 else { return nil }
        let noun = count == 1 ? "session" : "sessions"
        var line = "\(count) \(noun) contributed"
        // Labels only, never a path: a notification is rendered by the
        // desktop environment and may be logged by it.
        let named = Set(projects.filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty })
        if named.count == 1, let only = named.first {
            line += " from \(only)"
        }
        line += "."
        // Stated only when there is some. "0 credit pending" reads as a
        // failure rather than as a fresh start, and the first digest after
        // arming a project is exactly when that would show. Always
        // "pending", never "earned": settlement is off on every deployment
        // shipped so far, so a bare figure would be read as money.
        if creditPending > 0 {
            // Rounded half away from zero before formatting, matching the
            // daemon and the other two shells. `%.1f` alone rounds half to
            // even while .NET's "0.0" rounds half away from zero, so 4.25
            // would read as 4.2 here and 4.3 on Windows -- the same
            // contribution, a different figure depending which machine the
            // contributor read it on.
            let rounded = (creditPending * 10).rounded() / 10
            line += String(format: " %.1f credit pending.", rounded)
        }
        return line
    }
}
