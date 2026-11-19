using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using TraceCommons.Interop.Map;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Every arc's strokes against the Mac's own <c>drawArcs</c>, run by a probe
/// (W3 "Tests that pin the Mac": arc dash 5/5; flowing dash 4/8 at width
/// x1.3). Review finding 4 (#2215): these were read, never tested.
/// </summary>
public class MapArcStrokeTests
{
    private static readonly Lazy<JsonElement> Mac = new(() =>
    {
        using JsonDocument doc = JsonDocument.Parse(File.ReadAllText(
            Path.Combine(AppContext.BaseDirectory, "Fixtures", "Map", "mac-arc-strokes.json")));
        return doc.RootElement.GetProperty("strokes").Clone();
    });

    private static MapArcStyle Style(string name) => name switch
    {
        "quiet" => MapArcStyle.Quiet,
        "dashed" => MapArcStyle.Dashed,
        "flowing" => MapArcStyle.Flowing,
        "shared" => MapArcStyle.Shared,
        "unknown" => MapArcStyle.Unknown,
        _ => throw new ArgumentException(name),
    };

    [Fact]
    public void EveryArcIsStrokedAsTheMacStrokesIt()
    {
        int checkedRows = 0;
        foreach (JsonElement row in Mac.Value.EnumerateArray())
        {
            string name = row.GetProperty("style").GetString()!;
            bool healthy = row.GetProperty("healthy").GetBoolean();
            bool strong = row.GetProperty("strong").GetBoolean();
            IReadOnlyList<MapStroke> windows = MapGeometry.ArcStrokes(Style(name), healthy, strong, scale: 1);
            JsonElement[] mac = row.GetProperty("strokes").EnumerateArray().ToArray();
            string at = $"{name} healthy={healthy} strong={strong}";
            Assert.True(mac.Length == windows.Count, $"{at}: Mac {mac.Length} strokes, Windows {windows.Count}");
            for (int i = 0; i < mac.Length; i++)
            {
                JsonElement m = mac[i];
                MapStroke w = windows[i];
                Assert.True(m.GetProperty("ink").GetString() == w.Ink, $"{at} #{i} ink: Mac {m.GetProperty("ink")}, Windows {w.Ink}");
                Assert.True(Math.Abs(m.GetProperty("alpha").GetDouble() - w.Alpha) < 1e-9, $"{at} #{i} alpha: Mac {m.GetProperty("alpha")}, Windows {w.Alpha}");
                Assert.True(Math.Abs(m.GetProperty("width").GetDouble() - w.Width) < 1e-9, $"{at} #{i} width: Mac {m.GetProperty("width")}, Windows {w.Width}");
                double[] dash = m.GetProperty("dash").EnumerateArray().Select(d => d.GetDouble()).ToArray();
                double[] wdash = w.Dash ?? Array.Empty<double>();
                Assert.True(dash.SequenceEqual(wdash), $"{at} #{i} dash: Mac [{string.Join(",", dash)}], Windows [{string.Join(",", wdash)}]");
                Assert.True(m.GetProperty("round").GetBoolean() == w.Round, $"{at} #{i} round: Mac {m.GetProperty("round")}, Windows {w.Round}");
                bool macMoves = m.TryGetProperty("dashPhase", out JsonElement phase) && phase.GetDouble() != 0;
                Assert.True(macMoves == w.Moves, $"{at} #{i} moves: Mac {macMoves}, Windows {w.Moves}");
            }

            checkedRows++;
        }

        Assert.Equal(20, checkedRows);
    }

    [Fact]
    public void TheStrokesScaleWithTheMap()
    {
        MapStroke flowing = MapGeometry.ArcStrokes(MapArcStyle.Flowing, healthy: true, strong: false, scale: 2)[1];
        Assert.Equal(1.4 * 1.3 * 2, flowing.Width, 9);
        Assert.Equal(new[] { 8.0, 16.0 }, flowing.Dash);
    }

    // The window's drawing of these strokes is read from its pixels by the
    // render test ArcsDrawTheMacsStrokes (TraceCommons.RenderTests), which
    // draws through MapPainter.DrawArc, the call TracesMapView makes for
    // every arc. That the window calls it at all is seen on the box's
    // captures, where a map with no arcs would show.
}
