namespace TraceCommons.Interop;

/// <summary>
/// Copy for declining a whole project from the Waiting screen. A tested unit
/// rather than inline interpolation: this text exists in three shells and
/// plural agreement is the first thing to drift between them.
/// </summary>
public static class ProjectIgnoreCopy
{
    public const string ButtonLabel = "Ignore project";

    /// <summary>
    /// Word for word what GTK and macOS put on the same button:
    /// <c>copy::IGNORE_PROJECT_TOOLTIP</c> and <c>ProjectIgnoreCopy.tooltip</c>.
    /// Here rather than in the XAML for the same reason the body is here: a
    /// tooltip nobody tests is the first thing to drift between three shells,
    /// and these three had drifted already.
    /// </summary>
    public const string Tooltip =
        "Stops this project being offered and clears what it has waiting. "
        + "Anything already submitted is unaffected, and you can undo this in Settings.";

    public static string ConfirmationTitle(string project) => $"Ignore {project}?";

    /// <summary>
    /// The removal clause is dropped when nothing is waiting.
    ///
    /// No group renders that way today: all three shells build their groups
    /// from the pending list alone, so a group that renders has at least one
    /// waiting session in it. The branch is kept because this method is handed
    /// a number and must be right about whatever number it is handed --
    /// "removes 0 waiting traces" would be both wrong and alarming -- not
    /// because a caller is known to produce zero.
    /// </summary>
    public static string ConfirmationBody(string project, int pendingCount)
    {
        const string tail =
            "Nothing already submitted is affected. You can undo this in Settings.";
        if (pendingCount <= 0)
        {
            return $"Stops this project being offered. {tail}";
        }
        var noun = pendingCount == 1 ? "trace" : "traces";
        return $"This removes {pendingCount} waiting {noun} and stops this project "
             + $"being offered. {tail}";
    }

    /// <summary>
    /// What is said afterwards when the daemon removed a different number than
    /// the confirmation named, and <c>null</c> when the two agree.
    ///
    /// The dialog has to state a count before the call is made, so it states
    /// the one this shell can see. The queue is live: a poll between the render
    /// and the click adds waiting sessions, an approval elsewhere removes one,
    /// and the daemon acts on what is there when it gets the message.
    /// <paramref name="purged"/> is that number and it is the authority; the
    /// promise was an estimate. Silent when they agree -- a line that appears
    /// every time to say nothing happened is noise, and noise is how a line
    /// that matters gets skipped.
    /// </summary>
    public static string? Reconciliation(string project, int promised, int purged)
    {
        if (purged == promised)
        {
            return null;
        }
        var clause = purged == 1
            ? "1 waiting trace was removed"
            : $"{purged} waiting traces were removed";
        return $"Ignored {project}. The queue changed while you were deciding: "
             + $"{clause}, not {promised}.";
    }
}
