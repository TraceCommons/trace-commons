using System;
using System.Collections.Generic;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// The Monitor window's motion numbers (<c>GlassTokens.Motion</c>,
/// GlassTokens.swift:693-701) and the pure halves of the pane move
/// (<c>GlassThreePane.move(to:)</c>, ThreePane.swift:519-566).
/// </summary>
public static class MonitorMotion
{
    /// <summary><c>Motion.fast</c> (GlassTokens.swift:693).</summary>
    public const double Fast = 0.15;

    /// <summary><c>Motion.standard</c>: a pane opening or closing (GlassTokens.swift:694, ThreePane.swift:547).</summary>
    public const double Standard = 0.22;

    /// <summary>The token curve's control points (GlassTokens.swift:698-701).</summary>
    public const double EaseX1 = 0.2, EaseY1 = 0.8, EaseX2 = 0.2, EaseY2 = 1;

    /// <summary>
    /// The token curve at <paramref name="t"/> in [0, 1]: a cubic Bezier from
    /// (0,0) to (1,1) through (EaseX1, EaseY1) and (EaseX2, EaseY2), solved
    /// for x by Newton's method then bisection, as CoreAnimation does.
    /// </summary>
    public static double Ease(double t) => CubicBezier(EaseX1, EaseY1, EaseX2, EaseY2, t);

    /// <summary>A unit cubic Bezier's y at x = <paramref name="t"/>.</summary>
    public static double CubicBezier(double x1, double y1, double x2, double y2, double t)
    {
        if (t <= 0)
        {
            return 0;
        }

        if (t >= 1)
        {
            return 1;
        }

        double cx = 3 * x1, bx = 3 * (x2 - x1) - cx, ax = 1 - cx - bx;
        double cy = 3 * y1, by = 3 * (y2 - y1) - cy, ay = 1 - cy - by;
        double SampleX(double s) => ((ax * s + bx) * s + cx) * s;
        double SampleY(double s) => ((ay * s + by) * s + cy) * s;
        double SampleDx(double s) => (3 * ax * s + 2 * bx) * s + cx;

        double u = t;
        for (int i = 0; i < 8; i++)
        {
            double x = SampleX(u) - t;
            if (Math.Abs(x) < 1e-7)
            {
                return SampleY(u);
            }

            double d = SampleDx(u);
            if (Math.Abs(d) < 1e-6)
            {
                break;
            }

            u -= x / d;
        }

        double lo = 0, hi = 1;
        u = t;
        while (lo < hi)
        {
            double x = SampleX(u);
            if (Math.Abs(x - t) < 1e-7)
            {
                break;
            }

            if (t > x)
            {
                lo = u;
            }
            else
            {
                hi = u;
            }

            u = (lo + hi) / 2;
            if (hi - lo < 1e-9)
            {
                break;
            }
        }

        return SampleY(u);
    }

    /// <summary>
    /// SwiftUI's <c>.spring(response:dampingFraction:)</c> at
    /// <paramref name="seconds"/> after it starts, from 0 towards 1: a damped
    /// spring whose undamped period is <paramref name="response"/>
    /// (stiffness (2 pi / response) squared, unit mass). Checked against
    /// SwiftUI's own <c>Spring(response:dampingRatio:).value(target:time:)</c>
    /// on this Mac: for 0.36 / 0.86 it read 0.23117 at 0.05 s, 0.56773 at
    /// 0.1 s, 0.92849 at 0.2 s and overshot to 1.00395 at 0.4 s.
    /// </summary>
    public static double Spring(double response, double damping, double seconds)
    {
        if (seconds <= 0)
        {
            return 0;
        }

        double omega = 2 * Math.PI / response;
        double decay = Math.Exp(-damping * omega * seconds);
        if (damping < 1)
        {
            double omegaD = omega * Math.Sqrt(1 - damping * damping);
            return 1 - decay * (Math.Cos(omegaD * seconds) + damping * omega / omegaD * Math.Sin(omegaD * seconds));
        }

        return 1 - decay * (1 + omega * seconds);
    }

    /// <summary>
    /// <paramref name="value"/> sampled every 1/<paramref name="perSecond"/>
    /// of a second across <paramref name="duration"/> seconds, as key frames
    /// (the fraction of the duration, the value), the last at exactly 1. The
    /// compositor interpolates between them at the display's own rate, which
    /// is the rate SwiftUI runs its animations at.
    /// </summary>
    public static IReadOnlyList<(double Fraction, double Value)> KeyFrames(Func<double, double> value, double duration, double perSecond = 60)
    {
        int steps = Math.Max(1, (int)Math.Ceiling(duration * perSecond));
        var frames = new (double, double)[steps + 1];
        for (int i = 0; i <= steps; i++)
        {
            double fraction = (double)i / steps;
            frames[i] = (fraction, value(fraction * duration));
        }

        return frames;
    }

    /// <summary>
    /// Whether a pane move animates (ThreePane.swift:539-545): not under
    /// reduce motion, not when the window's frame does not change, and not
    /// when the window was held to the work area so the main pane changes
    /// width; those change at once.
    /// </summary>
    public static bool AnimatesPaneMove(bool animationsEnabled, bool frameChanges, bool mainChanges) =>
        animationsEnabled && frameChanges && !mainChanges;
}

/// <summary>
/// What a pane opening or closing does to the window, in physical pixels: its
/// new outer frame, the main pane's width it keeps, and whether it moves on
/// the standard curve or at once.
/// </summary>
public sealed record PaneMove(LayoutRect Frame, double Main, bool Animates);

/// <summary>
/// The window's growth at the screen edge (spec W2-D3 (a)): the Mac's
/// <c>move(to:)</c> (ThreePane.swift:519-566) over Windows' window
/// geometry. The Mac's arithmetic is in points over the window's content;
/// Windows positions the outer frame in physical pixels and reports the
/// client area separately, so the change is computed in device-independent
/// units over the client width and applied to the outer frame.
/// </summary>
public static class MonitorWindowGrowth
{
    /// <summary>
    /// The move from <paramref name="old"/> to <paramref name="next"/>.
    /// </summary>
    /// <param name="outer">The window's outer frame, physical pixels.</param>
    /// <param name="clientWidth">The client area's width, device-independent units.</param>
    /// <param name="workArea">The monitor's work area, physical pixels, origin included.</param>
    /// <param name="scale">The monitor's DPI over 96.</param>
    /// <param name="main">The main pane's kept width beside the map.</param>
    /// <param name="mapWidth">The map's kept width.</param>
    /// <param name="animationsEnabled">Whether the system lets things move.</param>
    public static PaneMove Plan(
        LayoutRect outer, double clientWidth, LayoutRect workArea, double scale,
        PaneVisibility old, PaneVisibility next, double main, double mapWidth, bool animationsEnabled)
    {
        MonitorPaneLayout before = MonitorPaneLayout.For(clientWidth, old.Map, old.Inspector, main);
        double kept = before.Main;
        double wanted = MonitorPaneLayout.WindowWidth(clientWidth, old, next, kept, mapWidth);
        // The change, not each width, in whole physical pixels: rounding the
        // two widths up separately added a pixel on every open and close at
        // a fractional scale.
        double outerWidth = outer.Width + Math.Round((wanted - clientWidth) * scale, MidpointRounding.AwayFromZero);
        LayoutRect frame = MonitorPaneLayout.WindowFrame(outer, outerWidth, workArea);
        double clientAfter = wanted - (outerWidth - frame.Width) / scale;
        MonitorPaneLayout after = MonitorPaneLayout.For(clientAfter, next.Map, next.Inspector, kept);
        bool frameChanges = frame != outer;
        bool mainChanges = Math.Abs(after.Main - before.Main) > 1e-6;
        return new PaneMove(frame, kept, MonitorMotion.AnimatesPaneMove(animationsEnabled, frameChanges, mainChanges));
    }

    /// <summary>
    /// The outer frame <paramref name="progress"/> of the way from
    /// <paramref name="from"/> to <paramref name="to"/> on the token curve,
    /// whole pixels.
    /// </summary>
    public static LayoutRect Interpolate(LayoutRect from, LayoutRect to, double progress)
    {
        double e = MonitorMotion.Ease(Math.Clamp(progress, 0, 1));
        static double Lerp(double a, double b, double e) => Math.Round(a + (b - a) * e);
        return new LayoutRect(Lerp(from.X, to.X, e), Lerp(from.Y, to.Y, e), Lerp(from.Width, to.Width, e), Lerp(from.Height, to.Height, e));
    }
}

/// <summary>One frame of a pane move: the window's outer frame and each pane's opacity.</summary>
public readonly record struct PaneMoveFrame(LayoutRect Frame, double MapOpacity, double InspectorOpacity, bool Done);

/// <summary>
/// A pane move under way (ThreePane.swift:538-566): the window's frame on the
/// token curve over <see cref="MonitorMotion.Standard"/>, and the panes that
/// open or close crossfading on the same curve and clock (ThreePane.swift:366-374:
/// a pane that closes goes from 1 to 0, one that opens is inserted with an
/// opacity transition, 0 to 1; a pane that stays is at 1). The window drives
/// it once a frame; hiding the window finishes it at its end state.
/// </summary>
public sealed class PaneMoveRun
{
    public PaneMoveRun(LayoutRect from, LayoutRect to, PaneVisibility old, PaneVisibility next)
    {
        From = from;
        To = to;
        Old = old;
        Next = next;
    }

    public LayoutRect From { get; }

    public LayoutRect To { get; }

    public PaneVisibility Old { get; }

    public PaneVisibility Next { get; }

    /// <summary>The move <paramref name="seconds"/> after it started.</summary>
    public PaneMoveFrame At(double seconds)
    {
        double progress = Math.Clamp(seconds / MonitorMotion.Standard, 0, 1);
        double eased = MonitorMotion.Ease(progress);
        return new PaneMoveFrame(
            MonitorWindowGrowth.Interpolate(From, To, progress),
            Opacity(Old.Map, Next.Map, eased),
            Opacity(Old.Inspector, Next.Inspector, eased),
            progress >= 1);
    }

    /// <summary>The move's last frame, for a move cut short by hiding the window.</summary>
    public PaneMoveFrame End => At(MonitorMotion.Standard);

    /// <summary>
    /// A pane's opacity <paramref name="eased"/> of the way through the move:
    /// rising for one that opens, falling for one that closes, 1 for one that
    /// stays.
    /// </summary>
    public static double Opacity(bool before, bool after, double eased) =>
        before == after ? 1 : after ? eased : 1 - eased;
}

/// <summary>
/// Whether a window's clocks may tick: only while it is shown, not hidden to
/// the tray and not minimised. <c>AppWindow.Hide</c> raises no
/// visibility change on the XAML window, so hiding to the tray is reported
/// here explicitly, beside the presenter's own minimised state.
/// </summary>
public sealed class WindowClockGate
{
    private bool _hiddenToTray;
    private bool _minimised;
    private bool _closed;

    /// <summary>Raised with the new answer whenever <see cref="MayTick"/> changes.</summary>
    public event Action<bool>? Changed;

    public bool MayTick => !_hiddenToTray && !_minimised && !_closed;

    public void SetHiddenToTray(bool hidden) => Set(ref _hiddenToTray, hidden);

    public void SetMinimised(bool minimised) => Set(ref _minimised, minimised);

    public void SetClosed() => Set(ref _closed, true);

    private void Set(ref bool field, bool value)
    {
        bool before = MayTick;
        field = value;
        if (MayTick != before)
        {
            Changed?.Invoke(MayTick);
        }
    }
}

/// <summary>
/// The title bar's interactive regions in physical pixels: the tabs, the
/// toolbar toggles and the gear take the pointer; the rest of the row drags
/// the window. WinUI's <c>InputNonClientPointerSource.SetRegionRects</c>
/// takes physical pixels relative to the window's client area.
/// </summary>
public static class TitleBarRegions
{
    /// <summary>
    /// A caption-button inset reported by the title bar (physical pixels) as a
    /// column width in device-independent units. A reading that is not a
    /// usable number keeps <paramref name="previousDips"/>: while a minimised
    /// window is restored, AppWindow.Changed reports RightInset -8, and no
    /// later size change reads it again.
    /// </summary>
    public static double InsetDips(double physicalInset, double scale, double previousDips) =>
        IsPositive(scale) && double.IsFinite(physicalInset) && physicalInset >= 0 ? physicalInset / scale : previousDips;

    /// <summary>
    /// When a size change may read the caption inset and lay the title bar
    /// out: never while minimised, and not inside the event that restores a
    /// minimised window, whose inset is transient (RightInset -8 at 100
    /// percent, 29 at 125 percent on the test box, against 144 and 180), so it
    /// is read once the restore has been dispatched.
    /// </summary>
    public static TitleBarRead WhenToRead(bool wasMinimised, bool minimised) =>
        minimised ? TitleBarRead.Skip : wasMinimised ? TitleBarRead.AfterDispatch : TitleBarRead.Now;

    /// <summary>Whether a rasterization scale can convert between units.</summary>
    public static bool IsUsableScale(double scale) => IsPositive(scale);

    private static bool IsPositive(double value) => double.IsFinite(value) && value > 0;

    /// <summary>
    /// Each element's bounds, given in device-independent units relative to
    /// the window's content, as whole physical pixels: the left and top edges
    /// rounded down and the right and bottom edges rounded up, so the region
    /// always covers the element. Empty elements are left out.
    /// </summary>
    public static IReadOnlyList<(int X, int Y, int Width, int Height)> Passthrough(IEnumerable<LayoutRect> dips, double scale)
    {
        var rects = new List<(int, int, int, int)>();
        if (!IsPositive(scale))
        {
            return rects;
        }

        foreach (LayoutRect r in dips)
        {
            if (r.Width <= 0 || r.Height <= 0)
            {
                continue;
            }

            int left = (int)Math.Floor(r.X * scale + 1e-9);
            int top = (int)Math.Floor(r.Y * scale + 1e-9);
            int right = (int)Math.Ceiling((r.X + r.Width) * scale - 1e-9);
            int bottom = (int)Math.Ceiling((r.Y + r.Height) * scale - 1e-9);
            rects.Add((left, top, right - left, bottom - top));
        }

        return rects;
    }
}

/// <summary>When the title bar's inset and regions are read (<see cref="TitleBarRegions.WhenToRead"/>).</summary>
public enum TitleBarRead
{
    Now,
    AfterDispatch,
    Skip,
}
