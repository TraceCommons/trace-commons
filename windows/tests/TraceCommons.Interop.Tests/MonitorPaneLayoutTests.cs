using System.Collections.Generic;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Monitor window's pane arithmetic against the Mac's own.
/// </summary>
/// <remarks>
/// Every expected number here was printed by a throwaway SwiftPM probe that
/// compiles macos/Sources/TCDesign at 3372425f4 (ThreePane.swift's
/// <c>GlassPaneLayout</c>, GlassTokens.swift) and calls it with the same
/// arguments. None is computed by the C# under test. The probe is not
/// committed (spec, "How parity is checked").
/// </remarks>
public class MonitorPaneLayoutTests
{
    private static readonly PaneVisibility None = new(false, false);
    private static readonly PaneVisibility InspectorOnly = new(false, true);
    private static readonly PaneVisibility MapOnly = new(true, false);
    private static readonly PaneVisibility Both = new(true, true);

    [Fact]
    public void ThePaneWidthsAreTheMacs()
    {
        // ThreePane.swift:44 (paneLeftWidth 400 + 40), :48 (round(440 * 2/3)),
        // :54; GlassTokens.swift:512-513, 584-594.
        Assert.Equal(440, MonitorPaneLayout.MainWidth);
        Assert.Equal(293, MonitorPaneLayout.InspectorWidth);
        Assert.Equal(4, MonitorPaneLayout.DividerReach);
        Assert.Equal(10, MonitorPaneLayout.PaneGap);
        Assert.Equal(0, MonitorPaneLayout.WindowPadding);
        Assert.Equal(440, MonitorPaneLayout.DefaultMapWidth);
        Assert.Equal(360, MonitorPaneLayout.MapMinWidth);
        Assert.Equal(614, MonitorPaneLayout.WindowHeight);
        Assert.Equal(560, MonitorPaneLayout.WindowMinHeight);
        Assert.Equal(1100, MonitorPaneLayout.MapBreakpoint);
        Assert.Equal(900, MonitorPaneLayout.InspectorBreakpoint);
        Assert.Equal(1193, MonitorPaneLayout.DefaultWidth);
    }

    [Theory]
    // ThreePane.swift:143-146 and :122-128, probe lines minimumWindowWidth / windowWidth.
    [InlineData(false, false, 440, 440)]
    [InlineData(false, true, 743, 743)]
    [InlineData(true, false, 810, 890)]
    [InlineData(true, true, 1113, 1193)]
    public void MinimumAndDefaultWidthsPerComposition(bool map, bool inspector, double minimum, double @default)
    {
        Assert.Equal(minimum, MonitorPaneLayout.MinimumWindowWidth(map, inspector));
        Assert.Equal(@default, MonitorPaneLayout.WindowWidth(map, inspector));
    }

    [Fact]
    public void AKeptMapAndMainWidenTheDefault()
    {
        // probe: windowWidth(map:true,insp:false,mapWidth:300) = 810 (held to the
        // map's minimum); windowWidth(map:true,insp:true,mapWidth:600,main:500) = 1413.
        Assert.Equal(810, MonitorPaneLayout.WindowWidth(true, false, mapWidth: 300));
        Assert.Equal(1413, MonitorPaneLayout.WindowWidth(true, true, mapWidth: 600, main: 500));
    }

    public static IEnumerable<object?[]> Layouts() => new[]
    {
        // windowWidth, map, inspector, main -> main, map, inspector (probe "layout(...)")
        new object?[] { 440.0, false, false, 440.0, 440.0, null, null },
        new object?[] { 900.0, false, false, 440.0, 900.0, null, null },
        new object?[] { 743.0, false, true, 440.0, 440.0, null, 293.0 },
        new object?[] { 890.0, true, false, 440.0, 440.0, 440.0, null },
        new object?[] { 1193.0, true, true, 440.0, 440.0, 440.0, 293.0 },
        new object?[] { 1000.0, true, true, 440.0, 440.0, 247.0, 293.0 },
        new object?[] { 800.0, true, false, 440.0, 440.0, 350.0, null },
        new object?[] { 1400.0, true, true, 440.0, 440.0, 647.0, 293.0 },
        new object?[] { 1400.0, true, true, 560.0, 560.0, 527.0, 293.0 },
        new object?[] { 1100.0, true, false, 700.0, 700.0, 390.0, null },
        new object?[] { 600.0, true, true, 440.0, 440.0, 0.0, 293.0 },
    };

    [Theory]
    [MemberData(nameof(Layouts))]
    public void ThePanesAtAWidthAreTheMacs(
        double width, bool map, bool inspector, double main, double wantMain, double? wantMap, double? wantInspector)
    {
        // ThreePane.swift:59-71.
        MonitorPaneLayout layout = MonitorPaneLayout.For(width, map, inspector, main);
        Assert.Equal(wantMain, layout.Main);
        Assert.Equal(wantMap, layout.Map);
        Assert.Equal(wantInspector, layout.Inspector);
        Assert.Equal(10, layout.MapGap);
    }

    public static IEnumerable<object[]> Growth() => new[]
    {
        // current, from, to, main, mapWidth -> width (probe "windowWidth(current:...)")
        new object[] { 440.0, None, InspectorOnly, 440.0, 440.0, 743.0 },
        new object[] { 743.0, InspectorOnly, None, 440.0, 440.0, 440.0 },
        new object[] { 440.0, None, MapOnly, 440.0, 440.0, 890.0 },
        new object[] { 890.0, MapOnly, None, 440.0, 440.0, 440.0 },
        new object[] { 890.0, MapOnly, Both, 440.0, 440.0, 1193.0 },
        new object[] { 1193.0, Both, None, 440.0, 440.0, 440.0 },
        new object[] { 1000.0, MapOnly, None, 440.0, 440.0, 440.0 },
        new object[] { 440.0, None, Both, 440.0, 520.0, 1273.0 },
        new object[] { 500.0, None, MapOnly, 440.0, 200.0, 870.0 },
        new object[] { 1300.0, Both, MapOnly, 560.0, 440.0, 997.0 },
    };

    [Theory]
    [MemberData(nameof(Growth))]
    public void OpeningAPaneGrowsTheWindowByThatPaneAndItsGap(
        double current, PaneVisibility from, PaneVisibility to, double main, double mapWidth, double want)
    {
        // ThreePane.swift:166-182.
        Assert.Equal(want, MonitorPaneLayout.WindowWidth(current, from, to, main, mapWidth));
    }

    public static IEnumerable<object[]> Frames() => new[]
    {
        // frame, width, visible -> frame (probe "windowFrame(...)"), with the
        // Mac's bottom-left y carried through unchanged: the frame keeps its y
        // either way, so the top-left reading gives the same answer.
        new object[] { new LayoutRect(100, 50, 440, 614), 743.0, new LayoutRect(0, 0, 1440, 875), new LayoutRect(100, 50, 743, 614) },
        new object[] { new LayoutRect(900, 50, 440, 614), 743.0, new LayoutRect(0, 0, 1440, 875), new LayoutRect(697, 50, 743, 614) },
        new object[] { new LayoutRect(900, 50, 743, 614), 440.0, new LayoutRect(0, 0, 1440, 875), new LayoutRect(900, 50, 440, 614) },
        new object[] { new LayoutRect(1500, 50, 440, 614), 1193.0, new LayoutRect(1440, 0, 1280, 800), new LayoutRect(1500, 50, 1193, 614) },
        new object[] { new LayoutRect(100, 50, 440, 614), 2000.0, new LayoutRect(0, 0, 1440, 875), new LayoutRect(0, 50, 1440, 614) },
        new object[] { new LayoutRect(-50, 50, 440, 614), 743.0, new LayoutRect(0, 0, 1440, 875), new LayoutRect(0, 50, 743, 614) },
    };

    [Theory]
    [MemberData(nameof(Frames))]
    public void AWindowAtTheScreenEdgeSlidesLeftByThePaneItOpens(LayoutRect frame, double width, LayoutRect visible, LayoutRect want)
    {
        // ThreePane.swift:194-202, including a second monitor whose work area
        // starts at x = 1440 (its origin is part of the clamp).
        Assert.Equal(want, MonitorPaneLayout.WindowFrame(frame, width, visible));
    }

    [Theory]
    // ThreePane.swift:208-210 with GlassTokens.swift:591-592, probe "firstLaunch(...)".
    [InlineData(899, false, false)]
    [InlineData(900, false, true)]
    [InlineData(1099, false, true)]
    [InlineData(1100, true, true)]
    [InlineData(1440, true, true)]
    public void TheFirstLaunchSeedsTheMapFrom1100AndTheInspectorFrom900(double width, bool map, bool inspector)
    {
        Assert.Equal(new PaneVisibility(map, inspector), MonitorPaneLayout.FirstLaunch(width));
    }

    [Theory]
    // ThreePane.swift:134-138, probe "draggedMain(...)".
    [InlineData(440, 100, 1193, true, 520)]
    [InlineData(440, 1000, 1193, true, 520)]
    [InlineData(440, -100, 1193, true, 440)]
    [InlineData(560, 50, 1100, false, 610)]
    public void TheDividerMovesRoomBetweenMainAndMap(double start, double dx, double width, bool inspector, double want)
    {
        Assert.Equal(want, MonitorPaneLayout.DraggedMain(start, dx, width, inspector));
    }

    public static IEnumerable<object?[]> Transitions() => new[]
    {
        // width, from, to, mapWidth -> main, map, inspector, gap, lower limit (probe "transition(...)")
        new object?[] { 600.0, None, MapOnly, null, 440.0, 150.0, null, 10.0, 440.0 },
        new object?[] { 900.0, MapOnly, None, 440.0, 440.0, 450.0, null, 10.0, 440.0 },
        new object?[] { 743.0, MapOnly, Both, 440.0, 440.0, 440.0, 293.0, 10.0, 810.0 },
        new object?[] { 1000.0, InspectorOnly, Both, null, 440.0, 247.0, 293.0, 10.0, 743.0 },
        new object?[] { 450.0, None, MapOnly, null, 440.0, 0.0, null, 10.0, 440.0 },
    };

    [Theory]
    [MemberData(nameof(Transitions))]
    public void ThePanesWhileTheWindowMoves(
        double width, PaneVisibility from, PaneVisibility to, double? mapWidth,
        double main, double? map, double? inspector, double gap, double lower)
    {
        // ThreePane.swift:86-104 and :151-156.
        var transition = new PaneTransition(from, to, mapWidth, 440);
        MonitorPaneLayout layout = MonitorPaneLayout.For(width, transition);
        Assert.Equal(main, layout.Main);
        Assert.Equal(map, layout.Map);
        Assert.Equal(inspector, layout.Inspector);
        Assert.Equal(gap, layout.MapGap);
        Assert.Equal(lower, MonitorPaneLayout.WindowWidthLowerLimit(transition));
    }

    [Fact]
    public void TheClientWidthComesFromTheWindowAndIsZeroAtAScaleThatIsNotAPositiveNumber()
    {
        // A minimised window can report a scale of 0; an infinite width would
        // reach GridLength, which throws inside AppWindow.Changed.
        Assert.Equal(880, MonitorPaneLayout.ClientWidth(1100, 1.25), 9);
        Assert.Equal(440, MonitorPaneLayout.ClientWidth(440, 1.0), 9);
        Assert.Equal(0, MonitorPaneLayout.ClientWidth(500, 0));
        Assert.Equal(0, MonitorPaneLayout.ClientWidth(500, double.NaN));
        Assert.Equal(0, MonitorPaneLayout.ClientWidth(-32000, 1.0));
    }

    [Fact]
    public void APaneToggleFlipsWhereThePanesAreGoingNotWhereTheyWere()
    {
        // Review finding 10 (#2215): "hide inspector, hide map, show
        // inspector, hide inspector" left the inspector open, because each
        // toggle read the panes as presented, and those change only when
        // the window's move ends. A click mid-move answers the move's end.
        var shown = new PaneVisibility(Map: true, Inspector: true);
        var closingInspector = new PaneVisibility(Map: true, Inspector: false);
        Assert.False(PaneToggles.Wanted(PaneToggle.Inspector, shown, movingTo: null));
        Assert.True(PaneToggles.Wanted(PaneToggle.Inspector, shown, movingTo: closingInspector));
        Assert.False(PaneToggles.Wanted(PaneToggle.Map, shown, movingTo: closingInspector));
        Assert.True(PaneToggles.Wanted(PaneToggle.Map, closingInspector with { Map = false }, movingTo: null));

        string shell = ShellSourceText("TraceCommons.App/MainWindow.Shell.cs");
        string map = ShellSourceText("TraceCommons.App/MainWindow.Map.cs");
        Assert.Contains("PaneToggles.Wanted(PaneToggle.Inspector, _presented, _transition?.To)", shell, System.StringComparison.Ordinal);
        Assert.Contains("PaneToggles.Wanted(PaneToggle.Map, _presented, _transition?.To)", map, System.StringComparison.Ordinal);
    }

    private static string ShellSourceText(string relativePath)
    {
        string path = System.IO.Path.Combine(System.AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(System.IO.File.Exists(path), path);
        return System.IO.File.ReadAllText(path);
    }
}
