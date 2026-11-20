using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using TraceCommons.Interop.Map;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// #2268's glow, on Windows: drawn once per size, scale, lit state and
/// colour from the ring's outline, which is the same at every angle of the
/// turn, not blurred again from the turning mesh on every frame.
/// </summary>
public class StationGlowTests
{
    /// <summary>LibraryStationTests.test_theRingsOutlineIsTheSameAtEveryAngle.</summary>
    [Fact]
    public void TheRingsOutlineIsTheSameAtEveryAngle()
    {
        const double width = 150, height = 130;
        double radius = LibraryStation.RingRadius;
        var outline = Enumerable.Range(0, 720).Select(step =>
        {
            double b = step / 720.0 * 2 * Math.PI;
            return LibraryStation.Project(LibraryStation.Turned(new Vec3(radius * Math.Cos(b), 0, radius * Math.Sin(b)), 0), width, height);
        }).ToList();
        // A real ring: wider than it is deep, about the station's middle.
        Assert.True(outline.Max(q => q.X) - outline.Min(q => q.X) > 60, "the outline has no width");
        Assert.True(outline.Max(q => q.Y) - outline.Min(q => q.Y) > 5, "the outline has no depth");
        for (double a = 0; a < 2 * Math.PI; a += 0.37)
        {
            MapPoint p = LibraryStation.Project(LibraryStation.Turned(new Vec3(radius * Math.Cos(a), 0, radius * Math.Sin(a)), 1.3), width, height);
            Assert.True(outline.Any(q => Math.Sqrt((q.X - p.X) * (q.X - p.X) + (q.Y - p.Y) * (q.Y - p.Y)) < 0.5), "a turned point of the ring left the ring's outline");
        }
    }

    [Fact]
    public void TheOutlineIsTheRingsOuterAndInnerEdgesClosed()
    {
        const double width = 150, height = 130;
        IReadOnlyList<MapPoint[]> loops = LibraryStation.RingOutline(width, height);
        Assert.Equal(2, loops.Count);
        foreach (MapPoint[] loop in loops)
        {
            Assert.Equal(97, loop.Length);
            Assert.Equal(loop[0], loop[^1]);
        }

        // The outer edge is outside the inner one, step for step.
        double Reach(MapPoint p) => Math.Sqrt((p.X - width / 2) * (p.X - width / 2) + (p.Y - height / 2) * (p.Y - height / 2));
        for (int i = 0; i < 97; i++)
        {
            Assert.True(Reach(loops[0][i]) > Reach(loops[1][i]), $"step {i}");
        }

        Assert.Equal(LibraryStation.Project(LibraryStation.Turned(new Vec3(LibraryStation.RingRadius + LibraryStation.RingTube, 0, 0), 0), width, height), loops[0][0]);
        Assert.Equal(LibraryStation.Project(LibraryStation.Turned(new Vec3(LibraryStation.RingRadius - LibraryStation.RingTube, 0, 0), 0), width, height), loops[1][0]);
    }

    /// <summary>The outline's ring is the mesh's ring: its 320 tube corners lie on it.</summary>
    [Fact]
    public void TheRingsMeasuresAreTheMeshs()
    {
        int onTube = LibraryStation.Indexed.Vertices.Count(v =>
        {
            double r = Math.Sqrt(v.X * v.X + v.Z * v.Z) - LibraryStation.RingRadius;
            return Math.Abs(Math.Sqrt(r * r + v.Y * v.Y) - LibraryStation.RingTube) < 1e-9;
        });
        Assert.True(onTube >= 40 * 8, $"{onTube} corners on the tube");
    }

    /// <summary>
    /// The map draws the station's glow from the ring once per look and
    /// keeps it; the frames carry only the art. The orb keeps its own glow.
    /// </summary>
    [Fact]
    public void TheMapDrawsTheStationsGlowOnceFromTheRing()
    {
        string source = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App/Controls/TracesMapView.cs.txt")).Replace("\r\n", "\n", StringComparison.Ordinal);
        string glow = Method(source, "private CanvasRenderTarget StationGlow(");
        Assert.Contains("LibraryStation.RingOutline(", glow, StringComparison.Ordinal);
        Assert.Contains("CanvasFilledRegionDetermination.Alternate", glow, StringComparison.Ordinal);
        Assert.Contains("DrawGlow(", glow, StringComparison.Ordinal);
        Assert.Contains("LibraryStation.Glow(lit)", glow, StringComparison.Ordinal);
        Assert.Contains("LibraryStation.GlowRadius", glow, StringComparison.Ordinal);
        // Keyed by size, scale, lit state and colour.
        Assert.Contains("_stationGlow.Look($\"{side}|{tall}|{dpi}|{lit}|{brand}\")", glow, StringComparison.Ordinal);

        string library = Method(source, "private void DrawLibrary(");
        Assert.Contains("StationGlow(", library, StringComparison.Ordinal);
        Assert.Contains("if (!launch)", library, StringComparison.Ordinal);
        Assert.Contains("_stationGlow.Clear();", Method(source, "private void DropResources("), StringComparison.Ordinal);
    }

    private static string Method(string source, string signature)
    {
        int at = source.IndexOf(signature, StringComparison.Ordinal);
        Assert.True(at >= 0, signature + " is gone");
        return source[at..source.IndexOf("\n    }\n", at, StringComparison.Ordinal)];
    }
}
