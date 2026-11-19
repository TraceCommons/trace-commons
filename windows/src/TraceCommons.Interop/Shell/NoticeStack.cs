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

    /// <summary>
    /// The cards the stack has held, after it shows <paramref name="shown"/>:
    /// every one of them added (PopoverNotice.swift:241), and none at all once
    /// the stack is empty, because the Mac's stack, and its <c>seen</c> with
    /// it, is taken down when nothing is shown (MonitorNotices.swift:71-77).
    /// </summary>
    public static IReadOnlySet<string> SeenAfter(IReadOnlySet<string> seen, IReadOnlyList<string> shown) =>
        shown.Count == 0
            ? new HashSet<string>(StringComparer.Ordinal)
            : seen.Concat(shown).ToHashSet(StringComparer.Ordinal);

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

/// <summary>How the card coming to the front enters (PopoverNotice.swift:244-247).</summary>
public enum NoticeEntrance
{
    /// <summary>It is already in front, or nothing moves: it is simply drawn.</summary>
    None,
    /// <summary>It has just arrived: it slides in from the side the stack hangs from and fades in.</summary>
    Slide,
    /// <summary>A close revealed it: it fades in where it is.</summary>
    Fade,
}

/// <summary>
/// What changes at the front of the stack: the card that leaves the front
/// (it fades where it is, PopoverNotice.swift:247 <c>removal: .opacity</c>)
/// and how the new front card enters.
/// </summary>
public readonly record struct NoticeChange(string? Outgoing, NoticeEntrance Entrance);

/// <summary>
/// The notice stack's motion: one spring for all of it, response 0.36 s and
/// damping 0.86 (PopoverNotice.swift:238, <c>.animation(..., value: front.id)</c>),
/// which carries the arriving card's slide and fade, the covered card's fade
/// and the revealed card's fade alike. None under reduce motion (the same
/// line). The Linux port hid the covered card at once; here it fades.
/// </summary>
/// <remarks>
/// The whole stack's own <c>.transition(.opacity)</c> (MonitorNotices.swift:76)
/// has no animation in its scope: the stack's spring is keyed on its front
/// card inside it, the window's one <c>.animation</c> is keyed on the graph
/// (MonitorWindowView.swift:1034), and closing a card inserts into
/// <c>closed</c> with no <c>withAnimation</c> (MonitorNotices.swift:167). So
/// on the Mac the stack appears and goes at once unless the change that
/// caused it was itself animated, and of the cards Windows shows only the
/// arming offer's exit is (AppModel.swift:2255, spring 0.42/0.9), which is
/// W9's. The first card and the last close therefore change at once here too.
/// </remarks>
public static class NoticeMotion
{
    /// <summary>
    /// How long the spring runs: SwiftUI's own <c>settlingDuration</c> for
    /// <c>Spring(response: 0.36, dampingRatio: 0.86)</c>, printed on this Mac.
    /// </summary>
    public const double SpringSettling = 0.6449744783708491;

    /// <summary>
    /// What a change of the front card from <paramref name="previous"/> to
    /// <paramref name="next"/> does, given the cards the stack has already
    /// held. Nothing moves with animations off, when the front card is the
    /// same, or when the stack appears or empties (see the remarks).
    /// </summary>
    public static NoticeChange Plan(string? previous, string? next, IReadOnlySet<string> seen, bool animationsEnabled)
    {
        if (!animationsEnabled || previous is null || next is null || previous == next)
        {
            return new NoticeChange(null, NoticeEntrance.None);
        }

        return new NoticeChange(previous, NoticeStack.Slides(next, seen) ? NoticeEntrance.Slide : NoticeEntrance.Fade);
    }

    /// <summary>The spring's progress <paramref name="seconds"/> in.</summary>
    public static double Progress(double seconds) =>
        MonitorMotion.Spring(NoticeStack.SpringResponse, NoticeStack.SpringDamping, seconds);

    /// <summary>
    /// An arriving card's offset across the stack <paramref name="seconds"/> in:
    /// from a whole card's height off the side the stack hangs from (above it,
    /// negative, for a stack at the top; <c>.move(edge: .top)</c>) to its place.
    /// </summary>
    public static double ArrivalOffset(StackAnchor anchor, double height, double seconds) =>
        -NoticeStack.Direction(anchor) * height * (1 - Progress(seconds));

    /// <summary>The opacity of a card fading in (arriving or revealed), clamped where the spring overshoots.</summary>
    public static double FadeIn(double seconds) => Math.Clamp(Progress(seconds), 0, 1);

    /// <summary>The opacity of the card that leaves the front, where it is.</summary>
    public static double FadeOut(double seconds) => 1 - FadeIn(seconds);
}
