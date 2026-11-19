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
    private bool _awaitingLeave;

    /// <summary>Whether the rail is open, or opening.</summary>
    public bool Open { get; private set; }

    /// <summary>Whether the rail holds the keyboard: only after a click inside it.</summary>
    public bool HoldsKeyboard { get; private set; }

    /// <summary>When a pending close falls due, if one is pending.</summary>
    public DateTimeOffset? CloseAt => _closeAt;

    /// <summary>The pointer came over the zone or the open rail.</summary>
    /// <remarks>
    /// <paramref name="cursorInside"/> is whether the system cursor is on the
    /// window: WinUI can report an enter from a stale pointer position as the
    /// window resizes under it, and that opens nothing. After a close by
    /// Escape or a control, a pointer still resting on the zone opens nothing
    /// until it has left (tc-win-az5: the rail reopened 120-190 ms after
    /// Escape); a cursor seen elsewhere ends that wait.
    /// </remarks>
    public EdgeRailStep PointerEntered(bool cursorInside = true)
    {
        if (!cursorInside)
        {
            _awaitingLeave = false;
            return EdgeRailStep.None;
        }

        if (_awaitingLeave)
        {
            return EdgeRailStep.None;
        }

        _closeAt = null;
        if (Open)
        {
            return EdgeRailStep.None;
        }

        Open = true;
        return EdgeRailStep.GrowThenOpen;
    }

    /// <summary>The system cursor was seen off the window: a wait for the pointer to leave is over.</summary>
    public void CursorAway() => _awaitingLeave = false;

    /// <summary>The pointer left the rail: a close is scheduled <see cref="CloseDelay"/> later.</summary>
    /// <remarks>
    /// An exit with the system cursor still on the window is not a leave:
    /// WinUI reports one as the rail slides out or resizes under a resting
    /// cursor, and taking it reopened the rail after Escape (tc-win-az10).
    /// </remarks>
    public void PointerExited(DateTimeOffset now, bool cursorInside = false)
    {
        if (cursorInside)
        {
            return;
        }

        _awaitingLeave = false;
        if (Open)
        {
            _closeAt = now + CloseDelay;
        }
    }

    /// <summary>
    /// A press on the rail: a closed rail opens (a touch or pen tap has no
    /// hover, and WinUI sends no new enter while it still counts the pointer
    /// inside), and from now it holds the keyboard.
    /// </summary>
    public EdgeRailStep Pressed()
    {
        EdgeRailStep step = EdgeRailStep.None;
        if (!Open)
        {
            _awaitingLeave = false;
            step = PointerEntered(cursorInside: true);
        }

        Clicked();
        return step;
    }

    /// <summary>A click inside the open rail: from now it holds the keyboard.</summary>
    public void Clicked()
    {
        if (Open)
        {
            HoldsKeyboard = true;
        }
    }

    /// <summary>Escape, or a control that closes the rail: closes now, and stays closed until the pointer has left.</summary>
    public EdgeRailStep CloseNow()
    {
        EdgeRailStep step = Close();
        _awaitingLeave = step == EdgeRailStep.CloseThenShrink;
        return step;
    }

    /// <summary>The window's timer: closes once a scheduled close falls due.</summary>
    public EdgeRailStep Tick(DateTimeOffset now) =>
        _closeAt is DateTimeOffset due && now >= due ? Close() : EdgeRailStep.None;

    private EdgeRailStep Close()
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

    /// <summary>
    /// The rail's slide at <paramref name="seconds"/> into an open or close:
    /// how far right of its place it is (0 placed, 1 a whole rail width out)
    /// and its opacity. Moves in from the trailing edge with a fade; under
    /// reduce motion it has no offset, and the window asks for its last
    /// frame at once: the Mac requests no animation then (EdgeRailView.swift:82).
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
/// Which window the rail gives the keyboard back to (one-pass checklist M2):
/// the last window other than the rail that was in the foreground, followed
/// through the system's foreground events. A press on the closed zone
/// activates the rail before any handler runs, so the foreground window at
/// that moment is the rail itself, and on tc-win-az5 the keyboard was handed
/// to nothing (0x0) and stayed on the 4 px zone.
/// </summary>
public sealed class EdgeRailFocus
{
    private readonly IntPtr _rail;
    private IntPtr _lastOther;
    private IntPtr _noted;

    public EdgeRailFocus(IntPtr rail) => _rail = rail;

    /// <summary>The system's foreground changed to <paramref name="hwnd"/>.</summary>
    public void Foreground(IntPtr hwnd)
    {
        if (hwnd != _rail && hwnd != IntPtr.Zero)
        {
            _lastOther = hwnd;
        }
    }

    /// <summary>The rail opens, or takes the keyboard; <paramref name="foregroundNow"/> is the foreground window at that moment.</summary>
    public void Opening(IntPtr foregroundNow)
    {
        Foreground(foregroundNow);
        _noted = _lastOther;
    }

    /// <summary>The window to give the keyboard back to as the rail closes, once.</summary>
    public IntPtr HandBack()
    {
        IntPtr back = _noted;
        _noted = IntPtr.Zero;
        return back;
    }
}

/// <summary>
/// The window messages after which the rail is placed again (one-pass
/// checklist M3; the Mac re-places on <c>didChangeScreenParameters</c>,
/// EdgeRailController.swift:67): a display arriving, leaving or changing
/// resolution, a DPI change, and the work area moving (the taskbar resized
/// or moved).
/// </summary>
public static class EdgeRailReplace
{
    private const uint WM_SETTINGCHANGE = 0x001A;
    private const uint WM_DISPLAYCHANGE = 0x007E;
    private const uint WM_DPICHANGED = 0x02E0;
    private const long SPI_SETWORKAREA = 0x002F;

    public static bool After(uint message, long wParam) =>
        message is WM_DISPLAYCHANGE or WM_DPICHANGED
        || (message == WM_SETTINGCHANGE && wParam == SPI_SETWORKAREA);
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

/// <summary>
/// The window styles the flyout and the rail end with (one-pass checklist
/// M1): a popup with no frame of any kind, a tool window (out of Alt-Tab and
/// the taskbar), topmost, and never activated by being shown. The presenter
/// alone leaves a 3 px classic frame, which on the rail's 4 px zone left no
/// client area to draw in or to hover.
/// </summary>
public static class ToolWindowStyle
{
    /// <summary>WS_POPUP.</summary>
    public const long Popup = 0x80000000L;

    /// <summary>WS_CAPTION (WS_BORDER | WS_DLGFRAME) and WS_THICKFRAME.</summary>
    public const long FrameBits = 0x00C00000L | 0x00040000L;

    /// <summary>WS_SYSMENU, WS_MINIMIZEBOX, WS_MAXIMIZEBOX: they come with a caption and go with it.</summary>
    private const long CaptionExtras = 0x00080000L | 0x00020000L | 0x00010000L;

    /// <summary>WS_EX_DLGMODALFRAME, WS_EX_WINDOWEDGE, WS_EX_CLIENTEDGE, WS_EX_STATICEDGE.</summary>
    private const long EdgeBits = 0x00000001L | 0x00000100L | 0x00000200L | 0x00020000L;

    /// <summary>WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST.</summary>
    private const long ToolBits = 0x08000000L | 0x00000080L | 0x00000008L;

    public static long Frameless(long style) => (style & ~(FrameBits | CaptionExtras)) | Popup;

    public static long Extended(long exStyle) => (exStyle & ~EdgeBits) | ToolBits;
}

/// <summary>
/// Where a tray click or key press says the icon is (one-pass checklist M3).
/// Under NOTIFYICON_VERSION_4 a NIN_SELECT, NIN_KEYSELECT or WM_CONTEXTMENU
/// callback carries the icon's anchor in <c>wParam</c>, in physical screen
/// pixels, as two signed 16-bit halves: a monitor left of or above the
/// primary has negative coordinates. The cursor is not the anchor: after a
/// keyboard select it can be on another monitor.
/// </summary>
public static class TrayAnchor
{
    /// <summary>The anchor in <paramref name="wParam"/>; the cursor only when the callback carried none.</summary>
    public static (int X, int Y) From(long wParam, (int X, int Y) cursor) =>
        wParam == 0 ? cursor : ((short)(wParam & 0xFFFF), (short)((wParam >> 16) & 0xFFFF));
}
