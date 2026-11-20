using System;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// Which panes beside the main pane are drawn.
/// </summary>
public readonly record struct PaneVisibility(bool Map, bool Inspector);

/// <summary>What a pane toggle asks for.</summary>
public static class PaneToggles
{
    /// <summary>
    /// Whether <paramref name="pane"/> is wanted after its toggle is
    /// pressed: the opposite of where the panes are going
    /// (<paramref name="movingTo"/>, while the window moves), else of where
    /// they are. The Mac's toggle flips the wanted state, never the drawn one.
    /// </summary>
    public static bool Wanted(PaneToggle pane, PaneVisibility presented, PaneVisibility? movingTo)
    {
        PaneVisibility current = movingTo ?? presented;
        return pane == PaneToggle.Map ? !current.Map : !current.Inspector;
    }
}

/// <summary>
/// A move from one composition to another while the window resizes
/// (<c>GlassPaneLayout.Transition</c>, ThreePane.swift:236-254).
/// </summary>
public readonly record struct PaneTransition(PaneVisibility From, PaneVisibility To, double? MapWidth, double MainWidth);

/// <summary>
/// A rectangle in one coordinate space: device-independent units for the
/// layout, physical pixels for <see cref="MonitorPaneLayout.WindowFrame"/>.
/// The origin is the top-left corner, as Windows has it.
/// </summary>
public readonly record struct LayoutRect(double X, double Y, double Width, double Height)
{
    public double Right => X + Width;
}

/// <summary>
/// The Monitor window's pane widths for one window width: a port of the Mac's
/// <c>GlassPaneLayout</c> (macos/Sources/TCDesign/Components/ThreePane.swift:30-258)
/// with the glass tokens it reads (TCDesign/Generated/GlassTokens.swift).
/// </summary>
/// <remarks>
/// <para>
/// The main pane opens at <see cref="MainWidth"/> and never goes below it.
/// The inspector is a fixed two thirds of it. Without the map the main pane
/// takes a resize; with the map, the map takes it first, down to its minimum,
/// then the main pane gives up room down to its default. Opening a pane grows
/// the window on its right by that pane and its gap, closing shrinks it by
/// the same, so no other pane moves.
/// </para>
/// <para>
/// Pure and in device-independent units, so it is tested on any machine
/// against numbers printed by a probe compiled from the Mac's own Swift.
/// </para>
/// </remarks>
public sealed record MonitorPaneLayout(double Main, double? Map, double? Inspector, double MapGap)
{
    /// <summary><c>GlassTokens.Size.paneLeftWidth</c> (GlassTokens.swift:582).</summary>
    public const double PaneLeftWidth = 400;

    /// <summary><c>GlassTokens.Size.mapWidth</c> (GlassTokens.swift:584).</summary>
    public const double DefaultMapWidth = 440;

    /// <summary><c>GlassTokens.Size.mapMinWidth</c> (GlassTokens.swift:585).</summary>
    public const double MapMinWidth = 360;

    /// <summary><c>GlassTokens.Space.paneGap</c> (GlassTokens.swift:512).</summary>
    public const double PaneGap = 10;

    /// <summary><c>GlassTokens.Space.windowPadding</c> (GlassTokens.swift:513).</summary>
    public const double WindowPadding = 0;

    /// <summary><c>GlassTokens.Size.windowHeight</c> (GlassTokens.swift:588).</summary>
    public const double WindowHeight = 614;

    /// <summary><c>GlassTokens.Size.windowMinHeight</c> (GlassTokens.swift:594).</summary>
    public const double WindowMinHeight = 560;

    /// <summary><c>GlassTokens.Size.mapBreakpoint</c> (GlassTokens.swift:591).</summary>
    public const double MapBreakpoint = 1100;

    /// <summary><c>GlassTokens.Size.inspectorBreakpoint</c> (GlassTokens.swift:592).</summary>
    public const double InspectorBreakpoint = 900;

    /// <summary>
    /// <c>GlassPaneLayout.mainWidth</c>: <c>paneLeftWidth + 40</c> (ThreePane.swift:44).
    /// </summary>
    public const double MainWidth = PaneLeftWidth + 40;

    /// <summary>
    /// <c>GlassPaneLayout.dividerReach</c> (ThreePane.swift:54): how far into
    /// the map the divider takes the pointer.
    /// </summary>
    public const double DividerReach = 4;

    /// <summary>
    /// <c>GlassPaneLayout.inspectorWidth</c>: two thirds of the main pane,
    /// rounded (ThreePane.swift:48). The 300 token is not used.
    /// </summary>
    public static double InspectorWidth => Math.Round(MainWidth * 2 / 3, MidpointRounding.AwayFromZero);

    /// <summary>
    /// The panes at <paramref name="windowWidth"/> (ThreePane.swift:59-71).
    /// <paramref name="main"/> is the main pane's kept width beside the map.
    /// </summary>
    public static MonitorPaneLayout For(double windowWidth, bool showsMap, bool showsInspector, double main = MainWidth)
    {
        double room = windowWidth - OthersWidth(showsInspector);
        double? inspector = showsInspector ? InspectorWidth : null;
        if (showsMap)
        {
            double shared = room - PaneGap;
            double kept = Math.Min(Math.Max(main, MainWidth), Math.Max(MainWidth, shared - MapMinWidth));
            return new MonitorPaneLayout(kept, Math.Max(0, shared - kept), inspector, PaneGap);
        }

        return new MonitorPaneLayout(Math.Max(MainWidth, room), null, inspector, PaneGap);
    }

    /// <summary>
    /// The panes while the window moves between compositions
    /// (ThreePane.swift:86-104).
    /// </summary>
    public static MonitorPaneLayout For(double windowWidth, PaneTransition transition)
    {
        bool showsInspector = transition.From.Inspector || transition.To.Inspector;
        double? inspector = showsInspector ? InspectorWidth : null;
        if (transition.From.Map && transition.To.Map)
        {
            return new MonitorPaneLayout(transition.MainWidth, transition.MapWidth ?? DefaultMapWidth, inspector, PaneGap);
        }

        if (!transition.From.Map && !transition.To.Map)
        {
            return new MonitorPaneLayout(transition.MainWidth, null, inspector, PaneGap);
        }

        double room = Math.Max(0, windowWidth - OthersWidth(showsInspector) - transition.MainWidth);
        double gap = Math.Min(PaneGap, room);
        return new MonitorPaneLayout(transition.MainWidth, room - gap, inspector, gap);
    }

    public PaneVisibility Visibility => new(Map is not null, Inspector is not null);

    /// <summary><c>othersWidth</c> (ThreePane.swift:108-110).</summary>
    internal static double OthersWidth(bool showsInspector) =>
        WindowPadding * 2 + (showsInspector ? PaneGap + InspectorWidth : 0);

    /// <summary><c>fixedWidth</c> (ThreePane.swift:114-116).</summary>
    internal static double FixedWidth(bool showsInspector) => OthersWidth(showsInspector) + MainWidth;

    /// <summary>
    /// The window's width with every shown pane at its default, the map at
    /// <paramref name="mapWidth"/> (ThreePane.swift:122-128).
    /// </summary>
    public static double WindowWidth(bool showsMap, bool showsInspector, double mapWidth = DefaultMapWidth, double main = MainWidth) =>
        FixedWidth(showsInspector) + (Math.Max(main, MainWidth) - MainWidth)
        + (showsMap ? PaneGap + Math.Max(MapMinWidth, mapWidth) : 0);

    /// <summary>
    /// The main pane's width while the divider is dragged (ThreePane.swift:134-138).
    /// </summary>
    public static double DraggedMain(double start, double dx, double windowWidth, bool showsInspector) =>
        For(windowWidth, showsMap: true, showsInspector, main: start + dx).Main;

    /// <summary>The narrowest window for these panes (ThreePane.swift:143-146).</summary>
    public static double MinimumWindowWidth(bool showsMap, bool showsInspector) =>
        FixedWidth(showsInspector) + (showsMap ? PaneGap + MapMinWidth : 0);

    /// <summary>The smallest window any composition fits (ThreePane.swift:254-256).</summary>
    public static double MinimumWindowWidthAny => MinimumWindowWidth(showsMap: false, showsInspector: false);

    /// <summary>
    /// The lower width limit while the window moves between two compositions
    /// (ThreePane.swift:151-156); there is no upper one.
    /// </summary>
    public static double WindowWidthLowerLimit(PaneTransition transition) =>
        Math.Min(
            MinimumWindowWidth(transition.From.Map, transition.From.Inspector),
            MinimumWindowWidth(transition.To.Map, transition.To.Inspector));

    /// <summary>
    /// The window's new width when panes open or close (ThreePane.swift:166-182).
    /// </summary>
    public static double WindowWidth(
        double current, PaneVisibility from, PaneVisibility to, double main = MainWidth, double mapWidth = DefaultMapWidth)
    {
        MonitorPaneLayout before = For(current, from.Map, from.Inspector, main);
        double width = current;
        if (from.Map != to.Map)
        {
            double opened = Math.Max(MapMinWidth, mapWidth) + PaneGap;
            width += to.Map ? opened : -((before.Map ?? 0) + PaneGap);
        }

        if (from.Inspector != to.Inspector)
        {
            width += (to.Inspector ? 1 : -1) * (InspectorWidth + PaneGap);
        }

        return Math.Max(MinimumWindowWidth(to.Map, to.Inspector), width);
    }

    /// <summary>
    /// The window's frame at <paramref name="width"/>, kept on the work area
    /// (ThreePane.swift:194-202): the top and the leading edge stay, the
    /// window grows or shrinks on its right, and one that would run off the
    /// work area moves left only as far as it must. One wider than the work
    /// area is held to it.
    /// </summary>
    /// <remarks>
    /// Windows' origin is the top-left corner, so keeping the top keeps
    /// <c>Y</c> (AppKit keeps <c>maxY</c> for the same edge). Every argument
    /// and the result are in the same unit: the caller passes physical pixels
    /// from <c>AppWindow</c> and <c>DisplayArea.WorkArea</c>, with the width
    /// already scaled by the monitor's DPI (<see cref="ToPhysical"/>).
    /// </remarks>
    public static LayoutRect WindowFrame(LayoutRect frame, double width, LayoutRect workArea)
    {
        width = Math.Min(width, workArea.Width);
        double x = frame.X;
        if (x + width > workArea.Right)
        {
            x = workArea.Right - width;
        }

        x = Math.Max(workArea.X, x);
        return new LayoutRect(x, frame.Y, width, frame.Height);
    }

    /// <summary>
    /// A width in device-independent units as whole physical pixels at
    /// <paramref name="scale"/> (the monitor's DPI over 96), rounded up so a
    /// pane never loses its last pixel column to truncation.
    /// </summary>
    public static int ToPhysical(double dips, double scale) => (int)Math.Ceiling(dips * scale - 1e-9);

    /// <summary>
    /// The window's client width in device-independent units, the width the
    /// panes are laid out in. Read from the window, never from the pane row:
    /// the row is arranged from the columns this layout sets, so once they are
    /// wider than the window the row never reports the window narrowing.
    /// </summary>
    public static double ClientWidth(int clientPixels, double scale) =>
        double.IsFinite(scale) && scale > 0 && clientPixels > 0 ? clientPixels / scale : 0;

    /// <summary>
    /// The panes a window opens with before the person has chosen
    /// (ThreePane.swift:208-210): the map from 1100, the inspector from 900,
    /// measured on the work area's width in device-independent units.
    /// </summary>
    public static PaneVisibility FirstLaunch(double workAreaWidth) =>
        new(workAreaWidth >= MapBreakpoint, workAreaWidth >= InspectorBreakpoint);

    /// <summary>
    /// The window's default width: every pane at its default (ThreePane.swift:569-571).
    /// </summary>
    public static double DefaultWidth => WindowWidth(showsMap: true, showsInspector: true);
}
