using System.Collections.Generic;
using System.Linq;

namespace TraceCommons.Interop.Shell;

/// <summary>A rectangle in window coordinates, without a WinUI type.</summary>
public readonly record struct CoverRect(double X, double Y, double Width, double Height)
{
    public double Right => X + Width;

    public double Bottom => Y + Height;

    public bool IsEmpty => Width <= 0 || Height <= 0;
}

/// <summary>Where focus goes from a control the card has come to cover.</summary>
public enum CoverFocusMove
{
    /// <summary>On to the next Tab stop.</summary>
    Forward,

    /// <summary>Back to the previous Tab stop.</summary>
    Back,

    /// <summary>To the card's X, the control drawn in its place.</summary>
    ToCard,
}

/// <summary>What changes when the notice stack's cover is re-read.</summary>
public sealed record NoticeCoverChange<T>(IReadOnlyList<T> Hide, IReadOnlyList<T> Restore);

/// <summary>
/// Which controls the notice stack covers (#2275). The stack is drawn over
/// the top of the tab and pushes nothing down, as on the Mac; a control it
/// covers cannot be reached by the pointer, so it must not be reached by
/// Tab or UI Automation either while it is covered.
/// </summary>
/// <remarks>
/// Covered means wholly inside the front card. A control with any part
/// showing can still be clicked there, so it stays reachable. The plates
/// behind the front card are not counted: they take no hits.
/// </remarks>
public static class NoticeCover
{
    /// <summary>Whether <paramref name="card"/> covers all of <paramref name="control"/>.</summary>
    public static bool Covers(CoverRect card, CoverRect control) =>
        !card.IsEmpty
        && !control.IsEmpty
        && control.X >= card.X
        && control.Y >= card.Y
        && control.Right <= card.Right
        && control.Bottom <= card.Bottom;

    /// <summary>
    /// The controls to take out (covered now, not before) and to put back
    /// (covered before, not now). One still covered is in neither, so the
    /// state saved when it was taken out is kept.
    /// </summary>
    public static NoticeCoverChange<T> Change<T>(IEnumerable<T> hidden, IEnumerable<T> covered)
    {
        var before = hidden.ToList();
        var now = covered.Distinct().ToList();
        return new NoticeCoverChange<T>(
            now.Where(c => !before.Contains(c)).ToArray(),
            before.Where(c => !now.Contains(c)).ToArray());
    }

    /// <summary>
    /// Where focus goes when the control holding it is covered: on the way a
    /// Tab press was going (Tab brings a control scrolled out of view in under
    /// the card), so no press is spent on a covered control; else to the card.
    /// </summary>
    public static CoverFocusMove FocusMove(bool? tabbedForward) => tabbedForward switch
    {
        true => CoverFocusMove.Forward,
        false => CoverFocusMove.Back,
        null => CoverFocusMove.ToCard,
    };
}
