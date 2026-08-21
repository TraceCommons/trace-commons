using System.Globalization;

namespace TraceCommons.Interop;

/// <summary>
/// What one queue card actually covers, when the answer is more than the
/// conversation itself.
/// </summary>
/// <remarks>
/// <para>
/// A Claude Code conversation is not one file: each delegated subagent's
/// turns are written beside the session, and one probed machine had 114 of
/// them under a single conversation. The card offers all of it as one
/// decision, so its extent belongs in the description -- see
/// <c>docs/contributor-daemon-ipc-v1_1.md</c>, which asks a client to say how
/// many delegated transcripts an entry covers and <b>requires</b> it to
/// surface a non-zero dropped count.
/// </para>
/// <para>
/// The second sentence is the one that has to be exactly right. A dropped
/// transcript is a normal consequence of a very large conversation, not an
/// error, and it is never the conversation itself -- the parent file is
/// always kept, and only delegated transcripts, largest first, are left out
/// to bring the group under the byte budget. So the line states what was left
/// out, why, and what that does not mean, in that order, and it does it
/// without a word that reads as a failure.
/// </para>
/// <para>
/// The sentences are the Linux shell's <c>copy::subagent_line</c> and the
/// macOS shell's <c>SubagentCopy.line</c>, word for word. Three clients
/// describing the same trim three ways is three different claims about what
/// was sent.
/// </para>
/// </remarks>
public static class SubagentCopy
{
    private const string Trimmed =
        "left out to keep this session within its size limit; the conversation itself is complete.";

    /// <summary>
    /// The card's extent line, or an empty string when there is nothing to
    /// say. An entry covering no delegated transcripts and dropping none
    /// renders no row at all rather than a line of zeroes: a line that is
    /// always present is a line nobody reads, and the one case that matters
    /// would be lost inside it.
    /// </summary>
    public static string Line(int subagentCount, int subagentsDropped)
    {
        int count = subagentCount > 0 ? subagentCount : 0;
        int dropped = subagentsDropped > 0 ? subagentsDropped : 0;

        if (count == 0 && dropped == 0)
        {
            return string.Empty;
        }

        if (count == 0)
        {
            return dropped == 1
                ? string.Format(
                    CultureInfo.CurrentCulture,
                    "1 delegated subagent transcript was {0}",
                    Trimmed)
                : string.Format(
                    CultureInfo.CurrentCulture,
                    "{0} delegated subagent transcripts were {1}",
                    dropped,
                    Trimmed);
        }

        string includes = string.Format(
            CultureInfo.CurrentCulture,
            "Includes {0} {1}.",
            count,
            Transcripts(count));

        if (dropped == 0)
        {
            return includes;
        }

        return dropped == 1
            ? string.Format(
                CultureInfo.CurrentCulture,
                "{0} The largest was {1}",
                includes,
                Trimmed)
            : string.Format(
                CultureInfo.CurrentCulture,
                "{0} The {1} largest were {2}",
                includes,
                dropped,
                Trimmed);
    }

    private static string Transcripts(int n) =>
        n == 1 ? "delegated subagent transcript" : "delegated subagent transcripts";
}
