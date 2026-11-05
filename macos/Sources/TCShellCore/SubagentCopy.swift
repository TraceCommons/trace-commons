import Foundation
import TCLocalization

/// What one queue card actually covers, when the answer is more than the
/// conversation itself.
///
/// A Claude Code conversation is not one file: each delegated subagent's
/// turns are written beside the session, and one probed machine had 114 of
/// them under a single conversation. The card offers all of it as one
/// decision, so its extent belongs in the description -- see
/// `docs/contributor-daemon-ipc-v1_1.md`, which asks a client to say how
/// many delegated transcripts an entry covers and **requires** it to
/// surface a non-zero dropped count.
///
/// The second sentence is the one that has to be exactly right. A dropped
/// transcript is a normal consequence of a very large conversation, not an
/// error, and it is never the conversation itself -- the parent file is
/// always kept, and only delegated transcripts, largest first, are left out
/// to bring the group under the byte budget. So the line states what was
/// left out, why, and what that does not mean, in that order, and it does
/// it without a word that reads as a failure.
///
/// Lives here rather than in the app target for the reason `ProjectRow`
/// moved: this is plural agreement and case analysis over two wire fields,
/// which is exactly the kind of code that has shipped wrong before, and
/// nothing in an executable target can be tested. The Linux shell's
/// `copy::subagent_line` and the Windows shell's `SubagentCopy.Line` are the
/// same sentences word for word; three clients describing the same trim
/// three ways is three different claims about what was sent.
public enum SubagentCopy {
    /// The card's extent line, or `nil` when there is nothing to say.
    ///
    /// An entry covering no delegated transcripts and dropping none renders
    /// no row at all rather than a line of zeroes: a line that is always
    /// present is a line nobody reads, and the one case that matters would
    /// be lost inside it.
    public static func line(count: Int, dropped: Int) -> String? {
        let count = max(count, 0)
        let dropped = max(dropped, 0)
        guard count > 0 || dropped > 0 else { return nil }
        if count == 0 {
            return Localization.current.text("app.subagent.dropped", count: Int64(dropped))
        }
        let included = Localization.current.text("app.subagent.includes", count: Int64(count))
        guard dropped > 0 else { return included }
        return Localization.current.text("app.subagent.includes_dropped", parameters: ["included": included], count: Int64(dropped))
    }
}
