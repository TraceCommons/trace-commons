namespace TraceCommons.Interop;

/// <summary>
/// What the verdict control says. Word for word what the Linux and macOS
/// shells print -- <c>crates/trace-commons-contributor-gtk/src/copy.rs</c> is
/// where these originate -- so a contributor who has seen this question on
/// one machine recognises it on another rather than reading a second,
/// differently-worded version of it.
/// </summary>
public static class VerdictCopy
{
    /// <summary>
    /// The question. Answering it is optional and never gates Contribute.
    /// </summary>
    public const string Question = "Did this session do what you asked?";

    public const string Worked = "Worked";
    public const string Partly = "Partly";
    public const string Failed = "Failed";

    /// <summary>
    /// Load-bearing, not decoration: the spec exempts the outcome fields
    /// from the "the preview above is exactly what would be sent"
    /// guarantee, and this sentence is where that exemption is disclosed to
    /// the contributor. Do not reword, shorten or drop it.
    /// </summary>
    public const string Caption =
        "Optional. This is recorded as the trace outcome; the preview above does not show it.";

    /// <summary>
    /// The bulk verdict menu beside "Submit all". The plain button stays a
    /// one-click unanswered submit; this is the opt-in path for answering
    /// once for the whole group, never a step in front of it.
    /// </summary>
    public const string SubmitAllAs = "Submit all as...";

    public const string SubmitAllAsTooltip =
        "Record the same outcome for every session in this group.";
}
