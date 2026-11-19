using System;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// Where the edge rail's window sits (<c>EdgeRailGeometry</c>,
/// macos/Sources/TCDesign/Components/EdgeRail.swift:7-24), from the Mac's
/// tokens (GlassTokens.swift:527-612): a thin hover zone on the right edge
/// while closed, and, open, the icon rail inset from the edge with its peek
/// beside it.
/// </summary>
public static class EdgeRailGeometry
{
    /// <summary>GlassTokens.swift:503, :527-528 (<c>space.s2</c>, <c>edgeRailInset</c>, <c>edgeRailGap</c>).</summary>
    public const double SpaceS2 = 4;
    public const double Inset = 12;
    public const double Gap = 16;

    /// <summary>GlassTokens.swift:606-614 (<c>size.edgeRail*</c>).</summary>
    public const double RailWidth = 56;
    public const double Tile = 44;
    public const double RuleWidth = 28;
    public const double PeekWidth = 380;
    public const double OpenHeight = 680;
    public const double ZoneWidth = 4;
    public const double ZoneHeight = 240;
    public const double HandleWidth = 3;
    public const double HandleHeight = 72;

    /// <summary>EdgeRail.swift:10: room left of the peek for its edge and shadow.</summary>
    public const double ShadowRoom = Gap + SpaceS2;

    /// <summary>EdgeRail.swift:12-15: 484 points.</summary>
    public const double OpenWidth = ShadowRoom + PeekWidth + Gap + RailWidth + Inset;

    /// <summary>
    /// EdgeRail.swift:19-23: the frame on a screen whose work area is
    /// <paramref name="visible"/>, all in one unit (points on the Mac, DIPs
    /// here): against its right edge, centred vertically, never taller than
    /// the work area.
    /// </summary>
    public static LayoutRect Frame(bool open, LayoutRect visible)
    {
        double width = open ? OpenWidth : ZoneWidth;
        double height = Math.Min(open ? OpenHeight : ZoneHeight, visible.Height);
        return new LayoutRect(visible.X + visible.Width - width, visible.Y + visible.Height / 2 - height / 2, width, height);
    }

    /// <summary>
    /// The frame in physical pixels for a work area in physical pixels on a
    /// monitor at <paramref name="scale"/> (its DPI over 96): the Mac's
    /// points are DIPs here, so the rail is the same size at every scale.
    /// Whole pixels, right edge flush.
    /// </summary>
    public static (int X, int Y, int Width, int Height) FramePixels(bool open, (int X, int Y, int Width, int Height) workArea, double scale)
    {
        if (!(scale > 0))
        {
            scale = 1;
        }

        LayoutRect dips = Frame(open, new LayoutRect(0, 0, workArea.Width / scale, workArea.Height / scale));
        int width = (int)Math.Round(dips.Width * scale);
        int height = (int)Math.Round(dips.Height * scale);
        int right = workArea.X + workArea.Width;
        int top = workArea.Y + (int)Math.Round((workArea.Height - height) / 2.0);
        return (right - width, top, width, height);
    }
}

/// <summary>What the rail asks its window to do.</summary>
public enum EdgeRailStep
{
    None,

    /// <summary>Grow the window to the open frame, then slide the rail in (EdgeRailController.swift:142-147).</summary>
    GrowThenOpen,

    /// <summary>Slide the rail out, then shrink the window to the zone once it has gone (:148-155).</summary>
    CloseThenShrink,
}

/// <summary>
/// The rail's hover and keyboard rules, as a clock-free state machine the
/// window drives: the Mac's <c>EdgeRailView.onHover</c> and
/// <c>EdgeRailOpening</c> (Views/EdgeRail/EdgeRailView.swift:62-75,
/// EdgeRailController.swift:110-170), plus the Windows rules the Linux port
/// found it needed (one-pass checklist M2): hover never takes the keyboard,
/// the keyboard is taken only after a click inside, and it is given back as
/// the close starts.
/// </summary>
public sealed class EdgeRailHover
{
    /// <summary>EdgeRailView.swift:68: the rail closes this long after the pointer leaves it.</summary>
    public static readonly TimeSpan CloseDelay = TimeSpan.FromMilliseconds(300);

    /// <summary>The slide (<c>GlassMotion.curve(GlassTokens.Motion.slide)</c>, EdgeRailView.swift:79): 0.25 s on the token curve.</summary>
    public const double SlideSeconds = 0.25;

    private DateTimeOffset? _closeAt;

    /// <summary>Whether the rail is open, or opening.</summary>
    public bool Open { get; private set; }

    /// <summary>Whether the rail holds the keyboard: only after a click inside it.</summary>
    public bool HoldsKeyboard { get; private set; }

    /// <summary>When a pending close falls due, if one is pending.</summary>
    public DateTimeOffset? CloseAt => _closeAt;

    /// <summary>The pointer came over the zone or the open rail.</summary>
    public EdgeRailStep PointerEntered()
    {
        _closeAt = null;
        if (Open)
        {
            return EdgeRailStep.None;
        }

        Open = true;
        return EdgeRailStep.GrowThenOpen;
    }

    /// <summary>The pointer left the rail: a close is scheduled <see cref="CloseDelay"/> later.</summary>
    public void PointerExited(DateTimeOffset now)
    {
        if (Open)
        {
            _closeAt = now + CloseDelay;
        }
    }

    /// <summary>A click inside the open rail: from now it holds the keyboard.</summary>
    public void Clicked()
    {
        if (Open)
        {
            HoldsKeyboard = true;
        }
    }

    /// <summary>Escape, or a control that closes the rail: closes now.</summary>
    public EdgeRailStep CloseNow()
    {
        _closeAt = null;
        if (!Open)
        {
            return EdgeRailStep.None;
        }

        Open = false;
        HoldsKeyboard = false;
        return EdgeRailStep.CloseThenShrink;
    }

    /// <summary>The window's timer: closes once a scheduled close falls due.</summary>
    public EdgeRailStep Tick(DateTimeOffset now) =>
        _closeAt is DateTimeOffset due && now >= due ? CloseNow() : EdgeRailStep.None;

    /// <summary>
    /// The rail's slide at <paramref name="seconds"/> into an open or close:
    /// how far right of its place it is (0 placed, 1 a whole rail width out)
    /// and its opacity. Moves in from the trailing edge with a fade; under
    /// reduce motion it only fades (EdgeRailView.swift:54).
    /// </summary>
    public static (double Offset, double Opacity, bool Done) Slide(bool opening, double seconds, bool reduceMotion)
    {
        double t = Math.Clamp(seconds / SlideSeconds, 0, 1);
        double eased = MonitorMotion.Ease(t);
        double shown = opening ? eased : 1 - eased;
        return (reduceMotion ? 0 : 1 - shown, shown, t >= 1);
    }
}

/// <summary>
/// Where focus goes back when the rail or the flyout lets the keyboard go:
/// only to the window that had it, and only if it is still there, visible and
/// not minimised, without restoring, raising or switching desktops
/// (one-pass checklist M2).
/// </summary>
public static class FocusReturn
{
    public static bool ShouldReturn(bool exists, bool visible, bool minimised, bool sameDesktop) =>
        exists && visible && !minimised && sameDesktop;
}

/// <summary>
/// Where the tray flyout opens: on the monitor that holds the click, in that
/// monitor's work area, against the taskbar's edge (one-pass checklist M3).
/// All in physical pixels; the flyout's size is its DIP size times the
/// monitor's scale.
/// </summary>
public static class TrayFlyoutPlacement
{
    /// <summary>
    /// The flyout's top-left for a click at <paramref name="click"/> on a
    /// monitor whose full bounds are <paramref name="monitor"/> and whose work
    /// area is <paramref name="work"/>. The taskbar is on whichever side the
    /// work area gives up; the flyout sits against it, centred on the click
    /// along that edge, then kept inside the work area with
    /// <paramref name="margin"/> to spare.
    /// </summary>
    public static (int X, int Y) Place(
        (int X, int Y) click,
        (int X, int Y, int Width, int Height) monitor,
        (int X, int Y, int Width, int Height) work,
        (int Width, int Height) size,
        int margin)
    {
        int workRight = work.X + work.Width, workBottom = work.Y + work.Height;
        int monitorRight = monitor.X + monitor.Width, monitorBottom = monitor.Y + monitor.Height;
        int x, y;
        if (workBottom < monitorBottom)
        {
            // Taskbar at the bottom: above it, centred on the click.
            x = click.X - size.Width / 2;
            y = workBottom - size.Height - margin;
        }
        else if (work.Y > monitor.Y)
        {
            // At the top: below it.
            x = click.X - size.Width / 2;
            y = work.Y + margin;
        }
        else if (work.X > monitor.X)
        {
            // On the left: right of it, centred on the click.
            x = work.X + margin;
            y = click.Y - size.Height / 2;
        }
        else if (workRight < monitorRight)
        {
            // On the right: left of it.
            x = workRight - size.Width - margin;
            y = click.Y - size.Height / 2;
        }
        else
        {
            // An auto-hidden taskbar gives up nothing: open above the click.
            x = click.X - size.Width / 2;
            y = click.Y - size.Height - margin;
        }

        x = Math.Clamp(x, work.X + margin, Math.Max(work.X + margin, workRight - size.Width - margin));
        y = Math.Clamp(y, work.Y + margin, Math.Max(work.Y + margin, workBottom - size.Height - margin));
        return (x, y);
    }
}
