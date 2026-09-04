using System;
using System.Globalization;

namespace TraceCommons.Interop;

/// <summary>
/// What a search found, once the pre-redaction count is taken into account.
/// </summary>
/// <remarks>
/// <para>
/// <c>tc_preview_search</c> scans the REDACTED body, by an absolute stated
/// rule. Searching it for a value that was removed correctly returns zero
/// matches, which is indistinguishable from the value never having been there
/// -- and those are the two answers a worried contributor most needs to tell
/// apart. <c>tc_search_original</c> counts the same needle in the
/// pre-redaction session text, and this is what turns the pair of numbers into
/// a sentence.
/// </para>
/// <para>
/// Four cases, and the two that matter are the ones about not knowing.
/// </para>
/// </remarks>
public abstract record OriginalSearchOutcome
{
    private OriginalSearchOutcome()
    {
    }

    /// <summary>Not in this session at all, before or after scrubbing.</summary>
    public static OriginalSearchOutcome Absent { get; } = new AbsentOutcome();

    /// <summary>
    /// The check could not be made, and nothing was found in the redacted
    /// body either.
    /// </summary>
    /// <remarks>
    /// Distinct from <see cref="Absent"/> on purpose. Reporting "not in this
    /// session" because a call failed would be the single most dangerous wrong
    /// answer this tab can give.
    /// </remarks>
    public static OriginalSearchOutcome Unknown { get; } = new UnknownOutcome();

    /// <summary>Present, and every occurrence was taken out.</summary>
    public sealed record AllRemoved(int Removed) : OriginalSearchOutcome;

    /// <summary>
    /// Still in what would be sent. The alarming case, and the only one drawn
    /// in the attention tone.
    /// </summary>
    /// <param name="Remaining">Matches in the redacted body.</param>
    /// <param name="Total">
    /// Occurrences before scrubbing, which is <paramref name="Remaining"/>
    /// itself whenever the pre-redaction count is missing or impossible.
    /// </param>
    public sealed record SomeRemain(int Remaining, int Total) : OriginalSearchOutcome;

    private sealed record AbsentOutcome : OriginalSearchOutcome;

    private sealed record UnknownOutcome : OriginalSearchOutcome;

    /// <summary>
    /// Classifies a search from the redacted match count and the
    /// pre-redaction one.
    /// </summary>
    /// <param name="remaining">Matches in the redacted body.</param>
    /// <param name="original">
    /// Occurrences before scrubbing, or null when the check could not be made.
    /// </param>
    /// <remarks>
    /// With no original count, fail toward what is CERTAIN: the redacted body
    /// is in hand, so matches in it are known, and the absence of a check must
    /// never render as a clean result. An original count BELOW the remaining
    /// count is impossible from a correct daemon, and falls back the same way
    /// rather than reporting a negative number of removals.
    /// </remarks>
    public static OriginalSearchOutcome Classify(int remaining, int? original)
    {
        if (original is not int total || total < remaining)
        {
            return remaining == 0 ? Unknown : new SomeRemain(remaining, remaining);
        }

        if (remaining > 0)
        {
            return new SomeRemain(remaining, total);
        }

        return total == 0 ? Absent : new AllRemoved(total);
    }

    /// <summary>The line the search tab prints.</summary>
    public string Sentence => this switch
    {
        AbsentOutcome => "Not in this session, before or after scrubbing.",
        UnknownOutcome =>
            "0 matches in what would be sent. This build could not check the session as it was "
            + "recorded, so that is not the same as saying it was never there.",
        AllRemoved removed => removed.Removed == 1
            ? "1 match, and it was removed."
            : string.Format(
                CultureInfo.CurrentCulture,
                "{0} matches, and all {0} were removed.",
                removed.Removed),
        SomeRemain remain => remain.Remaining == remain.Total
            ? string.Format(
                CultureInfo.CurrentCulture,
                "{0} still in what would be sent.",
                Matches(remain.Remaining))
            : string.Format(
                CultureInfo.CurrentCulture,
                "{0} of {1} still in what would be sent.",
                Matches(remain.Remaining),
                remain.Total),
        _ => throw new InvalidOperationException("unreachable: the case set is closed"),
    };

    /// <summary>
    /// Whether this is the answer that should be drawn in the attention tone.
    /// </summary>
    /// <remarks>
    /// True only for <see cref="SomeRemain"/>. <see cref="Unknown"/> is not
    /// alarming, it is unproven, and its sentence says so; colouring it would
    /// spend the tone that a value actually still in the payload needs.
    /// </remarks>
    public bool IsAlarming => this is SomeRemain;

    private static string Matches(int count) => count == 1
        ? "1 match"
        : string.Format(CultureInfo.CurrentCulture, "{0} matches", count);
}
