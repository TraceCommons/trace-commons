using System;
using System.Collections.Generic;
using System.Linq;
using TraceCommons.Interop.Map;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// #2268's fourth change, on Windows: a station frame turns and projects
/// each distinct corner once (each ring corner is shared by four faces) and
/// orders faces by index, instead of allocating arrays per face. The faces,
/// their order, their corners and their colours must be exactly what the
/// face-by-face frame gave.
/// </summary>
public class StationIndexedMeshTests
{
    [Fact]
    public void TheIndexedMeshSharesCornersAndRebuildsEveryFace()
    {
        LibraryStation.IndexedMesh m = LibraryStation.Indexed;
        Assert.Equal(LibraryStation.Mesh.Count, m.Starts.Length - 1);
        Assert.True(m.Vertices.Length < m.Corners.Length / 2, $"{m.Vertices.Length} distinct of {m.Corners.Length}");
        for (int f = 0; f < LibraryStation.Mesh.Count; f++)
        {
            var rebuilt = Enumerable.Range(m.Starts[f], m.Starts[f + 1] - m.Starts[f]).Select(c => m.Vertices[m.Corners[c]]);
            Assert.Equal(LibraryStation.Mesh[f].Points, rebuilt);
        }
    }

    [Theory]
    [InlineData(150, 130)]
    [InlineData(112.5, 97.5)]
    [InlineData(300, 260)]
    public void AFrameIsExactlyTheFaceByFaceFrame(double width, double height)
    {
        // The turn's own frames, then square angles, where mirrored faces
        // lie at equal depths and only the mesh's order can break the tie.
        var angles = Enumerable.Range(0, 390).Where(i => i % 7 == 0).Select(i => LibraryStation.RestAngle + i / 390.0 * 2 * Math.PI)
            .Concat(new[] { 0, Math.PI / 2, Math.PI, 3 * Math.PI / 2 });
        foreach (double angle in angles)
        {
            IReadOnlyList<DrawnFace> got = LibraryStation.Frame(width, height, angle);
            IReadOnlyList<DrawnFace> want = Reference(width, height, angle);
            Assert.Equal(want.Count, got.Count);
            for (int f = 0; f < want.Count; f++)
            {
                Assert.Equal(want[f].Points, got[f].Points);
                Assert.Equal((want[f].R, want[f].G, want[f].B), (got[f].R, got[f].G, got[f].B));
            }
        }
    }

    /// <summary>
    /// The painter's path: one buffer kept across frames and sizes, read
    /// face by face through the mesh's indices, gives the face-by-face
    /// frame, and a frame after the first allocates nothing per face.
    /// </summary>
    [Fact]
    public void ABufferKeptAcrossFramesGivesTheSameFacesAndAllocatesNothingPerFace()
    {
        var into = new StationFrame();
        LibraryStation.IndexedMesh m = LibraryStation.Indexed;
        foreach ((double width, double height) in new[] { (150.0, 130.0), (300.0, 260.0), (112.5, 97.5) })
        {
            foreach (double angle in new[] { LibraryStation.RestAngle, 1.3, 0, Math.PI / 2, 4.1 })
            {
                LibraryStation.FrameInto(into, width, height, angle);
                IReadOnlyList<DrawnFace> want = Reference(width, height, angle);
                Assert.Equal(want.Count, into.Count);
                for (int i = 0; i < into.Count; i++)
                {
                    int f = into.Order[i];
                    var points = Enumerable.Range(m.Starts[f], m.Starts[f + 1] - m.Starts[f]).Select(c => into.Projected[m.Corners[c]]);
                    Assert.Equal(want[i].Points, points);
                    Assert.Equal((want[i].R, want[i].G, want[i].B), (into.R[f], into.G[f], into.B[f]));

                    // The seam's outset into a kept buffer is the outset.
                    int n = m.Starts[f + 1] - m.Starts[f];
                    var outset = new MapPoint[n];
                    StationSeam.Outset(into.Projected, m.Corners, m.Starts[f], n, StationSeam.Width / 2, outset);
                    Assert.Equal(StationSeam.Outset(want[i].Points, StationSeam.Width / 2), outset);
                }
            }
        }

        LibraryStation.FrameInto(into, 150, 130, 2.0);
        long before = GC.GetAllocatedBytesForCurrentThread();
        LibraryStation.FrameInto(into, 150, 130, 2.1);
        long allocated = GC.GetAllocatedBytesForCurrentThread() - before;
        Assert.True(allocated < 1024, $"{allocated} bytes for {into.Count} faces");
    }

    /// <summary>The frame as it was drawn face by face before this change.</summary>
    private static IReadOnlyList<DrawnFace> Reference(double width, double height, double angle)
    {
        double scale = Math.Min(width / 140, height / 120);
        MapPoint Project(Vec3 q)
        {
            double s = scale * LibraryStation.CameraDistance / (LibraryStation.CameraDistance - q.Z);
            return new MapPoint(width / 2 + q.X * s, height / 2 - q.Y * s);
        }

        double cy = Math.Cos(angle), sy = Math.Sin(angle);
        var drawn = new List<(Vec3[] Points, Vec3 Color, double Shade, double Spec, double Depth, bool Cap)>();
        double markNear = double.NegativeInfinity, markFar = double.PositiveInfinity;
        foreach (StationFace face in LibraryStation.Mesh)
        {
            Vec3[] q = face.Points.Select(p => LibraryStation.Turned(p, cy, sy)).ToArray();
            if (q.Length < 3)
            {
                continue;
            }

            Vec3 n = Vec3.Cross(q[1] - q[0], q[2] - q[0]);
            double length = n.Length;
            n = length > 0 ? n / length : new Vec3(0, 0, 1);
            if (n.Z < 0)
            {
                n = -n;
            }

            double lambert = Math.Max(0, Vec3.Dot(n, LibraryStation.Light));
            double spec = Math.Pow(Math.Max(0, n.Z * 0.5 + lambert * 0.5), 18) * 0.35;
            double depth = q.Sum(p => p.Z) / q.Length;
            if (face.Mark)
            {
                foreach (Vec3 p in q)
                {
                    markNear = Math.Max(markNear, p.Z);
                    markFar = Math.Min(markFar, p.Z);
                }
            }

            drawn.Add((q, face.Color, 0.3 + 0.7 * lambert, spec, depth, face.MarkCap));
        }

        double markMiddle = (markNear + markFar) / 2;
        for (int i = 0; i < drawn.Count; i++)
        {
            if (drawn[i].Cap)
            {
                var d = drawn[i];
                d.Depth = d.Depth > markMiddle ? markNear + 0.01 : markFar - 0.01;
                drawn[i] = d;
            }
        }

        return drawn.OrderBy(d => d.Depth).Select(d =>
        {
            Vec3 c = d.Color * d.Shade + new Vec3(d.Spec, d.Spec, d.Spec);
            return new DrawnFace(d.Points.Select(Project).ToList(), Math.Min(1, c.X), Math.Min(1, c.Y), Math.Min(1, c.Z));
        }).ToList();
    }
}
