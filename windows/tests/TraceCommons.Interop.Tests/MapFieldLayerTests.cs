using System;
using TraceCommons.Interop.Map;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Checklist O3, the map's CPU. The rise at 40-50 s (and again at 80-90 s)
/// is the field's shimmer: it passes 1 s after the map is shown and every
/// 40 s after, for 1.1 s, and while it does the still layer is drawn at 60
/// frames a second. Each of those frames filled every mark of the field's
/// tile twice under a gradient layer, which a GPU-less box (WARP) rasterises
/// on the CPU: about 3.5 s of CPU a pass on tc-win-az11, measured. The field
/// at rest and the marks' mask are now rasterised once per size, theme and
/// DPI, and a pass draws two bitmaps.
/// </summary>
public class MapFieldLayerTests
{
    [Fact]
    public void TheShimmerPassesAtOneSecondAndEveryFortyAfter()
    {
        Assert.Equal(0, MapField.Sweep(0.5, false));
        Assert.True(MapField.Sweep(1.5, false) > 0);
        Assert.Equal(0, MapField.Sweep(2.2, false));
        Assert.Equal(0, MapField.Sweep(40.9, false));
        Assert.True(MapField.Sweep(41.5, false) > 0, "the second pass, inside the 40-50 s window");
        Assert.True(MapField.Sweep(81.5, false) > 0, "the third, inside 80-90 s");
        Assert.Equal(0, MapField.UntilNextPass(41.5));
        Assert.Equal(38.9, MapField.UntilNextPass(2.1), 6);
    }

    [Fact]
    public void ThePassDrawsBitmapsNotTheFieldsGeometry()
    {
        string source = Source();
        string body = Method(source, "private void DrawField(");
        Assert.DoesNotContain("FillGeometry(tile", body, StringComparison.Ordinal);
        Assert.Contains("FieldLayers(", body, StringComparison.Ordinal);
        Assert.Contains("ds.DrawImage(layers.Base", body, StringComparison.Ordinal);
        Assert.Contains("new CanvasImageBrush(device, layers.Mask)", body, StringComparison.Ordinal);

        // Built once per size, theme and DPI, and dropped with the device.
        string build = Method(source, "private (CanvasRenderTarget Base, CanvasRenderTarget Mask) FieldLayers(");
        Assert.Contains("_fieldLayerKey", build, StringComparison.Ordinal);
        Assert.Contains("Dark", build, StringComparison.Ordinal);
        Assert.Contains(".Dpi", build, StringComparison.Ordinal);
        Assert.Contains("FillGeometry(tile", build, StringComparison.Ordinal);
        Assert.Contains("DropFieldLayers();", Method(source, "private void DropResources("), StringComparison.Ordinal);
    }

    private static MapClockInputs Inputs(bool transfer = false, bool pile = false, bool light = false, bool flows = true, bool library = true) => new(
        AnimationsEnabled: true, Loaded: true, WindowVisible: true, PaneShown: true, HasLibrary: library, Flows: flows, Healthy: true,
        TransferPlaying: transfer, PileMoving: pile, LightFading: light, SecondsShown: 41.5, NextDeadlineIn: null);

    [Fact]
    public void APassDoesNotDrawTheArcsAndStationAtSixty()
    {
        // During a pass the clock ticks at 60 for the field; the arcs keep
        // their 30 a second and the station its 20.
        Assert.Equal(MapMotion.TransferFrameInterval, MapMotion.ClockInterval(Inputs()));
        Assert.False(MapMotion.MainLayerDue(Inputs(), sinceLastDraw: 1.0 / 60));
        Assert.True(MapMotion.MainLayerDue(Inputs(), sinceLastDraw: 2.0 / 60));
        Assert.False(MapMotion.MainLayerDue(Inputs(flows: false), sinceLastDraw: 2.0 / 60));
        Assert.True(MapMotion.MainLayerDue(Inputs(flows: false), sinceLastDraw: 3.0 / 60));

        // What moves at 60 in the main layer still draws every tick.
        Assert.True(MapMotion.MainLayerDue(Inputs(transfer: true), sinceLastDraw: 1.0 / 60));
        Assert.True(MapMotion.MainLayerDue(Inputs(pile: true), sinceLastDraw: 1.0 / 60));
        Assert.True(MapMotion.MainLayerDue(Inputs(light: true), sinceLastDraw: 1.0 / 60));

        // A clock that only waits (nothing turns or flows) draws when it fires, as before.
        Assert.True(MapMotion.MainLayerDue(Inputs(flows: false, library: false), sinceLastDraw: 1.0 / 60));
    }

    [Fact]
    public void TheTickAsksBeforeDrawingTheMainLayer()
    {
        string tick = Method(Source(), "private DispatcherQueueTimer CreateTimer(");
        Assert.Contains("MapMotion.MainLayerDue(", tick, StringComparison.Ordinal);
        Assert.DoesNotContain("            _canvas.Invalidate();\n            bool fieldMoves", tick, StringComparison.Ordinal);
    }

    private static string Source()
    {
        string path = System.IO.Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App/Controls/TracesMapView.cs.txt");
        return System.IO.File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }

    private static string Method(string source, string signature)
    {
        int at = source.IndexOf(signature, StringComparison.Ordinal);
        Assert.True(at >= 0, signature + " is gone");
        return source[at..source.IndexOf("\n    }\n", at, StringComparison.Ordinal)];
    }
}
