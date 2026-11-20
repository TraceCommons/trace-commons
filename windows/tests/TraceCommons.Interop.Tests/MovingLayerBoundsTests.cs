using System;
using System.Collections.Generic;
using TraceCommons.Interop.Map;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The map's moving layer at rest: the arcs, the library's turning frame
/// and the rocket at its rest. Its canvas used to cover the whole pane, so
/// every frame (30 a second while the dashes flow) cleared, drew and
/// composed every pixel of the pane, on the CPU on a GPU-less machine
/// (WARP). It is now sized to the box those parts draw in, snapped to whole
/// device pixels so each part lands on the same pixels as before.
/// </summary>
public class MovingLayerBoundsTests
{
    private static readonly MapSize Pane = new(360, 640);

    [Fact]
    public void ARunOrAMovingPileTakesTheWholePane()
    {
        var parts = new[] { new LayoutRect(100, 100, 50, 50) };
        Assert.Null(MovingLayerBounds.Resting(Pane, 1, whole: true, parts));
        Assert.NotNull(MovingLayerBounds.Resting(Pane, 1, whole: false, parts));
    }

    [Fact]
    public void NothingToDrawTakesTheWholePane()
    {
        Assert.Null(MovingLayerBounds.Resting(Pane, 1, whole: false, Array.Empty<LayoutRect>()));
    }

    [Fact]
    public void TheBoxHoldsEveryPartWithRoomForItsEdges()
    {
        var parts = new[] { new LayoutRect(100, 300, 60, 20), new LayoutRect(200, 120, 90, 80) };
        LayoutRect box = MovingLayerBounds.Resting(Pane, 1, whole: false, parts)!.Value;
        foreach (LayoutRect p in parts)
        {
            Assert.True(box.X <= p.X - MovingLayerBounds.Slack, $"left {box.X}");
            Assert.True(box.Y <= p.Y - MovingLayerBounds.Slack, $"top {box.Y}");
            Assert.True(box.X + box.Width >= p.X + p.Width + MovingLayerBounds.Slack, $"right {box.X + box.Width}");
            Assert.True(box.Y + box.Height >= p.Y + p.Height + MovingLayerBounds.Slack, $"bottom {box.Y + box.Height}");
        }

        Assert.Equal(new LayoutRect(98, 118, 194, 204), box);
    }

    [Theory]
    [InlineData(1.0)]
    [InlineData(1.25)]
    [InlineData(1.5)]
    [InlineData(1.75)]
    public void EveryEdgeIsOnAWholeDevicePixel(double scale)
    {
        var parts = new[] { new LayoutRect(100.3, 300.7, 60.1, 20.9), new LayoutRect(201.13, 120.41, 89.9, 80.2) };
        LayoutRect box = MovingLayerBounds.Resting(Pane, scale, whole: false, parts)!.Value;
        static bool Whole(double v) => Math.Abs(v - Math.Round(v)) < 1e-6;
        Assert.True(Whole(box.X * scale), $"x {box.X * scale}");
        Assert.True(Whole(box.Y * scale), $"y {box.Y * scale}");
        Assert.True(Whole(box.Width * scale), $"w {box.Width * scale}");
        Assert.True(Whole(box.Height * scale), $"h {box.Height * scale}");
        Assert.True(box.X <= 100.3 - MovingLayerBounds.Slack && box.Y <= 120.41 - MovingLayerBounds.Slack);
        Assert.True(box.X + box.Width >= 291.03 + MovingLayerBounds.Slack && box.Y + box.Height >= 321.6 + MovingLayerBounds.Slack);
    }

    [Fact]
    public void TheBoxStaysInsideThePane()
    {
        var parts = new[] { new LayoutRect(-20, 600, 120, 80) };
        LayoutRect box = MovingLayerBounds.Resting(Pane, 1, whole: false, parts)!.Value;
        Assert.Equal(0, box.X);
        Assert.Equal(Pane.Height, box.Y + box.Height);
    }

    [Fact]
    public void ABoxNearlyThePaneTakesTheWholePane()
    {
        var parts = new[] { new LayoutRect(4, 4, 352, 630) };
        Assert.Null(MovingLayerBounds.Resting(Pane, 1, whole: false, parts));
    }

    [Fact]
    public void ACurvesBoxHoldsEveryPointOnItAndItsStroke()
    {
        MapPoint a = new(10, 200), c1 = new(80, 90), c2 = new(160, 330), b = new(240, 150);
        const double half = 1.3;
        LayoutRect box = MovingLayerBounds.Curve(a, c1, c2, b, half);
        for (int i = 0; i <= 200; i++)
        {
            double t = i / 200.0, u = 1 - t;
            double x = u * u * u * a.X + 3 * u * u * t * c1.X + 3 * u * t * t * c2.X + t * t * t * b.X;
            double y = u * u * u * a.Y + 3 * u * u * t * c1.Y + 3 * u * t * t * c2.Y + t * t * t * b.Y;
            Assert.True(x - half >= box.X - 1e-9 && x + half <= box.X + box.Width + 1e-9, $"x {x} at {t}");
            Assert.True(y - half >= box.Y - 1e-9 && y + half <= box.Y + box.Height + 1e-9, $"y {y} at {t}");
        }
    }

    [Fact]
    public void ACentredBoxIsCentred()
    {
        Assert.Equal(new LayoutRect(90, 180, 20, 40), MovingLayerBounds.Around(new MapPoint(100, 200), 20, 40));
    }

    [Fact]
    public void TheMovingCanvasIsSizedToTheBoxAndDrawnThroughItsOffset()
    {
        string source = Source();
        string draw = Method(source, "private void DrawMoving(");
        Assert.Contains("sender.ActualOffset", draw, StringComparison.Ordinal);
        Assert.Contains("ds.Transform = Matrix3x2.CreateTranslation(-offset.X, -offset.Y)", draw, StringComparison.Ordinal);

        // Every part the resting layer draws is in the box: the arcs and
        // the rocket at rest; a run or a moving pile takes the whole pane.
        // The library is on its own layer at rest (#2268), in its own box,
        // and comes back to this one only while the whole pane is taken.
        string parts = Method(source, "private LayoutRect? MovingBox(");
        Assert.Contains("_scene.Arcs", parts, StringComparison.Ordinal);
        Assert.Contains("MovingLayerBounds.Curve(", parts, StringComparison.Ordinal);
        Assert.DoesNotContain("LibraryStation.FrameWidth", parts, StringComparison.Ordinal);
        Assert.Contains("LibraryStation.FrameWidth", Method(source, "private LayoutRect? LibraryBox("), StringComparison.Ordinal);
        Assert.Contains("if (LibraryOnMovingLayer(now)", draw, StringComparison.Ordinal);
        string library = Method(source, "private void DrawLibraryLayer(");
        Assert.Contains("sender.ActualOffset", library, StringComparison.Ordinal);
        Assert.Contains("ds.Transform = Matrix3x2.CreateTranslation(-offset.X, -offset.Y)", library, StringComparison.Ordinal);
        Assert.Contains("!LibraryOnMovingLayer(now)", library, StringComparison.Ordinal);
        Assert.Contains("TransferDrawing.RocketRest(", parts, StringComparison.Ordinal);
        Assert.Contains("bool whole = LibraryOnMovingLayer(now);", parts, StringComparison.Ordinal);
        Assert.Contains(
            "private bool LibraryOnMovingLayer(double now) =>\n        (Motion && WindowClocks.Running && _player.Run is not null) || PileMoves(now);",
            source, StringComparison.Ordinal);
        Assert.Contains("PlaceLibraryCanvas(now);", Method(source, "private void PlaceMovingCanvas("), StringComparison.Ordinal);

        // One tick source: the moving tick and the station's own both turn
        // the library layer, which is drawn again only when its frame changes.
        Assert.Contains("TurnLibrary();", Method(source, "private DispatcherQueueTimer CreateTimer("), StringComparison.Ordinal);
        Assert.Contains("TurnLibrary();", Method(source, "private DispatcherQueueTimer CreateLibraryTimer("), StringComparison.Ordinal);
        string turn = Method(source, "private void TurnLibrary(");
        Assert.Contains("MapMotion.LibraryFrameIndex(", turn, StringComparison.Ordinal);
        Assert.Contains("_libraryCanvas.Invalidate();", turn, StringComparison.Ordinal);
        Assert.Contains("MapMotion.LibraryFrameIndex(", Method(source, "private void DrawLibrary("), StringComparison.Ordinal);

        // The box is checked on every tick and on every redraw.
        Assert.Contains("PlaceMovingCanvas();", Method(source, "private DispatcherQueueTimer CreateTimer("), StringComparison.Ordinal);
        Assert.Contains("PlaceMovingCanvas();", Method(source, "private void Redraw("), StringComparison.Ordinal);
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
