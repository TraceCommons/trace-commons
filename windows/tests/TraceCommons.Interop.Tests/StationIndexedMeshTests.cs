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
        for (int i = 0; i < 390; i += 7)
        {
            double angle = LibraryStation.RestAngle + i / 390.0 * 2 * Math.PI;
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
