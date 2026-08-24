namespace TraceCommons.Interop;

/// <summary>
/// Copy for declining a whole project from the Waiting screen. A tested unit
/// rather than inline interpolation: this text exists in three shells and
/// plural agreement is the first thing to drift between them.
/// </summary>
public static class ProjectIgnoreCopy
{
    public const string ButtonLabel = "Ignore project";

    public static string ConfirmationTitle(string project) => $"Ignore {project}?";

    /// <summary>
    /// The removal clause is dropped when nothing is waiting: a group can
    /// render with every card approved or uploading, and "removes 0 waiting
    /// traces" would be both wrong and alarming.
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
}
