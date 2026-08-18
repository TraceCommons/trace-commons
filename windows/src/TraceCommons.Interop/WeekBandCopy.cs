using System;

namespace TraceCommons.Interop;

/// <summary>
/// The queue's week band: three figures under the list, transcribed from the
/// Linux shell's <c>ui::queue::render_week</c> so the two shells count the
/// same things and call them the same names.
/// </summary>
/// <remarks>
/// The two labels this band shares with History are taken from
/// <see cref="HistoryCopy"/> rather than restated, because a contributor who
/// reads "Held for privacy review" under the queue and something else under
/// History has been told two different things about one state.
/// </remarks>
public static class WeekBandCopy
{
    /// <summary>The band's heading.</summary>
    public const string ThisWeek = "This week";

    /// <summary>The eyebrow over the count of what did go out this week.</summary>
    public const string Contributed = "Contributed";

    /// <summary>Held for privacy review, this week.</summary>
    public const string Held = HistoryCopy.QuarantineHeading;

    /// <summary>
    /// In the commons -- and this one is all-time, not this week.
    /// </summary>
    /// <remarks>
    /// "In the commons" is a standing total. A weekly slice of it would read
    /// as the commons shrinking every Monday, which is both untrue and
    /// discouraging in exactly the place a contributor looks for evidence
    /// that their work went somewhere.
    /// </remarks>
    public const string InTheCommons = HistoryCopy.InTheCommons;
}
