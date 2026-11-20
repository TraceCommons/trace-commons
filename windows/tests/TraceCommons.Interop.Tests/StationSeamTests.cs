using System;
using System.Collections.Generic;
using System.Linq;
using TraceCommons.Interop.Map;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Checklist O3: the station's 506 faces were each filled and then stroked
/// with a 0.6-wide round-joined seam, which on a GPU-less box cost about
/// 7 of a frame's 19 ms of CPU (probe on tc-win-az11, 2026-11-20). The seam
/// only widens each face by half its width, so each face is now filled once
/// through its outline pushed out by that half (<see cref="StationSeam.Outset"/>).
/// </summary>
public class StationSeamTests
{
    private static void Near(MapPoint expected, MapPoint actual) =>
        Assert.True(Math.Abs(expected.X - actual.X) < 1e-9 && Math.Abs(expected.Y - actual.Y) < 1e-9, $"expected {expected}, was {actual}");

    [Fact]
    public void ASquareGrowsByTheHalfWidthOnEverySide()
    {
        var square = new List<MapPoint> { new(0, 0), new(10, 0), new(10, 10), new(0, 10) };
        IReadOnlyList<MapPoint> grown = StationSeam.Outset(square, 0.3);
        Near(new MapPoint(-0.3, -0.3), grown[0]);
        Near(new MapPoint(10.3, -0.3), grown[1]);
        Near(new MapPoint(10.3, 10.3), grown[2]);
        Near(new MapPoint(-0.3, 10.3), grown[3]);

        // Either winding grows outward.
        IReadOnlyList<MapPoint> reversed = StationSeam.Outset(square.AsEnumerable().Reverse().ToList(), 0.3);
        Near(new MapPoint(-0.3, 10.3), reversed[0]);
        Near(new MapPoint(-0.3, -0.3), reversed[3]);
    }

    [Fact]
    public void ASharpCornerIsClampedNotSpiked()
    {
        // A sliver: the miter at its 2-degree tip would reach ~17 px; the
        // round join it stands in for reaches 0.3.
        var sliver = new List<MapPoint> { new(0, 0), new(100, 1.75), new(100, -1.75) };
        IReadOnlyList<MapPoint> grown = StationSeam.Outset(sliver, 0.3);
        double reach = Math.Sqrt(grown[0].X * grown[0].X + grown[0].Y * grown[0].Y);
        Assert.InRange(reach, 0.3, StationSeam.MiterLimit * 0.3 + 1e-9);
    }

    [Fact]
    public void ADegenerateFaceIsReturnedAsItIs()
    {
        var line = new List<MapPoint> { new(0, 0), new(5, 0), new(10, 0) };
        Assert.Equal(line, StationSeam.Outset(line, 0.3));
        Assert.Equal(new List<MapPoint> { new(1, 1) }, StationSeam.Outset(new List<MapPoint> { new(1, 1) }, 0.3));
    }

    [Fact]
    public void TheStationIsFilledOnceAFaceAndNeverStroked()
    {
        string path = System.IO.Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App/Controls/MapPainter.cs.txt");
        string source = System.IO.File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
        int at = source.IndexOf("public static void Station(", StringComparison.Ordinal);
        Assert.True(at >= 0, "Station is gone");
        string body = source[at..source.IndexOf("\n    }\n", at, StringComparison.Ordinal)];
        Assert.DoesNotContain("DrawGeometry", body, StringComparison.Ordinal);
        Assert.Contains("StationSeam.Outset(", body, StringComparison.Ordinal);
    }
}
