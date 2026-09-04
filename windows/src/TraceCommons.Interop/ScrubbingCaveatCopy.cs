namespace TraceCommons.Interop;

/// <summary>
/// What the app says about the limits of scrubbing.
/// </summary>
/// <remarks>
/// Here rather than in the markup because the same two sentences are printed
/// by three shells in several places each, and a copy of one sitting in XAML
/// is outside everything that keeps them from drifting. A person who read the
/// caveat under the queue has to recognise it above Contribute, rather than
/// reading a second, weaker message.
/// </remarks>
public static class ScrubbingCaveatCopy
{
    /// <summary>
    /// The caveat, word for word as the queue, the scrubbing panel and the
    /// preview footer all print it. Do not reword it in one place.
    /// </summary>
    public const string Sentence =
        "Scrubbing is pattern-based. It misses things it hasn't seen before.";

    /// <summary>
    /// What a session where no pattern fired says, and what to do about it.
    /// </summary>
    /// <remarks>
    /// The chip is correct and stays gold: a session where nothing matched is
    /// the one worth slowing down on. What it lacked was a next step, which is
    /// an affordance complaint rather than a complaint about the tone. The
    /// clause pointing at search is that next step, and the chip itself is now
    /// a control that opens it.
    /// </remarks>
    public const string NothingMatchedLine =
        "Nothing matched. On a session that touched credentials, that is itself worth a "
        + "second look. Search it for anything you would not want to send.";

    /// <summary>
    /// The line printed beside a session's removal count.
    /// </summary>
    /// <param name="removals">
    /// Occurrences removed, which is <c>RedactionLabels.Total</c> and never
    /// the raw map: a session whose only count is a surviving secret removed
    /// nothing, and reads as the zero case here.
    /// </param>
    public static string RowLine(int removals) =>
        removals == 0 ? NothingMatchedLine : Sentence;
}
