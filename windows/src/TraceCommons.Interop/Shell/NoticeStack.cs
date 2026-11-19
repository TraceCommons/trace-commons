using System;
using System.Collections.Generic;
using System.Linq;

namespace TraceCommons.Interop.Shell;

/// <summary>The side a notice stack hangs from (<c>GlassPopoverStack.Anchor</c>, PopoverNotice.swift:185).</summary>
public enum StackAnchor
{
    Top,
    Bottom,
}

/// <summary>
/// One plate drawn behind the front card: the front card's size, a little
/// narrower, moved out by <see cref="NoticeStack.Peek"/> a level
/// (<c>plate(level:)</c>, PopoverNotice.swift:251-261).
/// </summary>
public readonly record struct NoticePlate(int Level, double ScaleX, double OffsetY, double Opacity);

/// <summary>
/// How the window's notice stack is laid out: a port of the Mac's
/// <c>GlassPopoverStack</c> (macos/Sources/TCDesign/Components/PopoverNotice.swift:182-262)
/// and of the pure parts of <c>MonitorNotices</c> (MonitorNotices.swift:78-118).
/// </summary>
/// <remarks>
/// The last card is in front and up to two earlier ones peek out behind it.
/// A new card slides in over the one it covers; a card revealed by a close
/// fades in place. Each card's X closes it and the one behind comes forward.
/// </remarks>
public static class NoticeStack
{
    /// <summary>How far each card behind shows past the one in front (PopoverNotice.swift:201).</summary>
    public const double Peek = 8;

    /// <summary>How many cards are drawn (PopoverNotice.swift:202).</summary>
    public const int Drawn = 3;

    /// <summary>The narrowing per level behind (PopoverNotice.swift:258).</summary>
    public const double NarrowingPerLevel = 0.04;

    /// <summary>The arrival spring's response, seconds (PopoverNotice.swift:238).</summary>
    public const double SpringResponse = 0.36;

    /// <summary>The arrival spring's damping fraction (PopoverNotice.swift:238).</summary>
    public const double SpringDamping = 0.86;

    /// <summary>The ids of the cards drawn, the front one first (PopoverNotice.swift:205-207).</summary>
    public static IReadOnlyList<string> Visible(IReadOnlyList<string> ids) =>
        ids.Skip(Math.Max(0, ids.Count - Drawn)).Reverse().ToArray();

    /// <summary>Which way a card behind moves from the front one (PopoverNotice.swift:211).</summary>
    public static double Direction(StackAnchor anchor) => anchor == StackAnchor.Bottom ? -1 : 1;

    /// <summary>How many plates are drawn behind the front card.</summary>
    public static int Behind(IReadOnlyList<string> ids) => Math.Max(0, Visible(ids).Count - 1);

    /// <summary>The plates behind the front card, nearest first.</summary>
    public static IReadOnlyList<NoticePlate> Plates(IReadOnlyList<string> ids, StackAnchor anchor)
    {
        int behind = Behind(ids);
        var plates = new NoticePlate[behind];
        for (int index = 0; index < behind; index++)
        {
            int level = index + 1;
            plates[index] = new NoticePlate(
                level,
                1 - NarrowingPerLevel * level,
                Direction(anchor) * Peek * level,
                level == 1 ? 0.9 : 0.7);
        }

        return plates;
    }

    /// <summary>
    /// The room the stack keeps past the front card for the plates
    /// (PopoverNotice.swift:236).
    /// </summary>
    public static double PlateRoom(IReadOnlyList<string> ids) => Peek * Behind(ids);

    /// <summary>
    /// Whether the card coming to the front slides in (it has just arrived)
    /// or fades in place (a close revealed it) (PopoverNotice.swift:244-247).
    /// </summary>
    public static bool Slides(string front, IReadOnlySet<string> seen) => !seen.Contains(front);

    /// <summary>The items not closed for now, in the stack's order (MonitorNotices.swift:78-80).</summary>
    public static IReadOnlyList<string> Shown(IReadOnlyList<string> items, IReadOnlySet<string> closed) =>
        items.Where(id => !closed.Contains(id)).ToArray();

    /// <summary>
    /// <paramref name="closed"/> without the closed health notices no longer
    /// shown, so one whose cause has gone comes back when it recurs
    /// (MonitorNotices.swift:97-99).
    /// </summary>
    public static IReadOnlySet<string> PrunedClosed(IReadOnlySet<string> closed, IReadOnlySet<string> currentHealth) =>
        closed.Where(id => !id.StartsWith("health:", StringComparison.Ordinal) || currentHealth.Contains(id))
            .ToHashSet(StringComparer.Ordinal);
}
