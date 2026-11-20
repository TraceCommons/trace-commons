using System;
using System.Collections.Generic;

namespace TraceCommons.Interop.FirstRun;

/// <summary>A row of a step's column.</summary>
public enum FirstRunFrameSlot
{
    Gap,
    Graphic,
    Title,
    Card,
    Notice,
}

/// <summary>One row: what it holds, whether it takes a share of the free height, and its least height.</summary>
public readonly record struct FirstRunFrameRow(FirstRunFrameSlot Slot, bool Fills, double MinHeight);

/// <summary>The gaps a step's free height becomes, and what the card gains.</summary>
public readonly record struct FirstRunFrameSplit(double AboveGraphic, double BelowGraphic, double BelowCard, double CardExtra);

/// <summary>
/// Where a step's column sits (FirstRunFrame.swift:186-226,
/// FirstRunFrameLayout): below the thread's 44, the free height is split
/// equally three ways -- above the graphic, between the graphic and the
/// title, below the card -- none under <see cref="MinimumGap"/> (Ron,
/// 2026-10-21). On Uses and Join the card is a ScrollView
/// (UsesScreen.swift:647, JoinScreen.swift:494), which takes all the free
/// height before the gaps do: every gap rests at its minimum and the column
/// starts under the thread. The window lays a step out as a grid of
/// <see cref="Rows"/>; <see cref="Split"/> is the same rule in numbers.
/// </summary>
public static class FirstRunFrameLayout
{
    /// <summary>No gap is ever smaller than this.</summary>
    public const double MinimumGap = 24;

    /// <summary>Between a title and its card.</summary>
    public const double TitleGap = 20;

    /// <summary>Where the free height starts: below the step thread.</summary>
    public const double ThreadBottom = 44;

    /// <summary>Whether the step's card scrolls and so takes the free height.</summary>
    public static bool CardScrolls(FirstRunStep step) => step is FirstRunStep.Uses or FirstRunStep.Join;

    /// <summary>The column's rows, top to bottom; those that fill share the free height equally.</summary>
    public static IReadOnlyList<FirstRunFrameRow> Rows(FirstRunStep step)
    {
        bool scrolls = CardScrolls(step);
        var gap = new FirstRunFrameRow(FirstRunFrameSlot.Gap, !scrolls, MinimumGap);
        return new[]
        {
            gap,
            new FirstRunFrameRow(FirstRunFrameSlot.Graphic, false, 0),
            gap,
            new FirstRunFrameRow(FirstRunFrameSlot.Title, false, 0),
            new FirstRunFrameRow(FirstRunFrameSlot.Card, scrolls, 0),
            new FirstRunFrameRow(FirstRunFrameSlot.Notice, false, 0),
            gap,
        };
    }

    /// <summary>
    /// The gaps <paramref name="free"/> becomes: the height left once the
    /// thread, graphic, title and card at their own heights are placed.
    /// </summary>
    public static FirstRunFrameSplit Split(FirstRunStep step, double free)
    {
        double rest = Math.Max(0, free - 3 * MinimumGap);
        if (CardScrolls(step))
        {
            return new FirstRunFrameSplit(MinimumGap, MinimumGap, MinimumGap, rest);
        }

        double third = MinimumGap + rest / 3;
        return new FirstRunFrameSplit(third, third, third, 0);
    }
}
