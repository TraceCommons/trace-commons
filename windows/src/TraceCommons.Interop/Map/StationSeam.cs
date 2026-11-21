using System;
using System.Collections.Generic;

namespace TraceCommons.Interop.Map;

/// <summary>
/// The station's seams (LibraryStation.swift: each face filled and stroked
/// 0.6 wide with round joins, so neighbouring faces meet without a hairline
/// gap). A stroke centred on the outline widens the face by half its width,
/// so the face is filled once through its outline pushed out by that half:
/// one fill instead of a fill and a stroke, the costliest part of a station
/// frame on a GPU-less box (checklist O3).
/// </summary>
public static class StationSeam
{
    /// <summary>The seam's width (LibraryStation.swift; MapPainter's 0.6).</summary>
    public const double Width = 0.6;

    /// <summary>
    /// How far a corner may reach, in half-widths: a round join reaches one;
    /// a miter at a sharp corner would spike far past it.
    /// </summary>
    public const double MiterLimit = 2;

    /// <summary>
    /// <paramref name="points"/> pushed out by <paramref name="by"/> along
    /// each corner's bisector, whichever way the face winds, each corner's
    /// reach held to <see cref="MiterLimit"/> times <paramref name="by"/>.
    /// A face with no area is returned as it is.
    /// </summary>
    public static IReadOnlyList<MapPoint> Outset(IReadOnlyList<MapPoint> points, double by)
    {
        int n = points.Count;
        var all = new MapPoint[n];
        var corners = new int[n];
        for (int i = 0; i < n; i++)
        {
            all[i] = points[i];
            corners[i] = i;
        }

        if (!HasArea(all, corners, 0, n))
        {
            return points;
        }

        var result = new MapPoint[n];
        Outset(all, corners, 0, n, by, result);
        return result;
    }

    /// <summary>
    /// <see cref="Outset(IReadOnlyList{MapPoint}, double)"/> for the face
    /// whose corners are <c>projected[corners[first..first + count]]</c>,
    /// written into <paramref name="into"/> (its first
    /// <paramref name="count"/> places): the painter's path, which keeps
    /// its buffers from face to face. A face with no area is copied as it is.
    /// </summary>
    public static void Outset(MapPoint[] projected, int[] corners, int first, int count, double by, MapPoint[] into)
    {
        int n = count;
        MapPoint At(int i) => projected[corners[first + i]];
        if (!HasArea(projected, corners, first, n))
        {
            for (int i = 0; i < n; i++)
            {
                into[i] = At(i);
            }

            return;
        }

        double area = SignedArea(projected, corners, first, n);
        double side = area > 0 ? 1 : -1;
        (double X, double Y) Normal(MapPoint a, MapPoint b)
        {
            double dx = b.X - a.X, dy = b.Y - a.Y, length = Math.Sqrt(dx * dx + dy * dy);
            return length < 1e-12 ? (0, 0) : (side * dy / length, -side * dx / length);
        }

        for (int i = 0; i < n; i++)
        {
            MapPoint previous = At((i + n - 1) % n), here = At(i), next = At((i + 1) % n);
            (double X, double Y) n1 = Normal(previous, here), n2 = Normal(here, next);
            double sx = n1.X + n2.X, sy = n1.Y + n2.Y, s = Math.Sqrt(sx * sx + sy * sy);
            double mx, my;
            if (s < 1e-9)
            {
                // A corner that doubles back: push out along one edge's normal.
                (mx, my) = (n1.X * by, n1.Y * by);
            }
            else
            {
                double ux = sx / s, uy = sy / s;
                double cos = ux * n1.X + uy * n1.Y;
                double reach = cos > 1e-9 ? Math.Min(by / cos, MiterLimit * by) : MiterLimit * by;
                (mx, my) = (ux * reach, uy * reach);
            }

            into[i] = new MapPoint(here.X + mx, here.Y + my);
        }
    }

    private static bool HasArea(MapPoint[] projected, int[] corners, int first, int n) =>
        n >= 3 && Math.Abs(SignedArea(projected, corners, first, n)) >= 1e-9;

    private static double SignedArea(MapPoint[] projected, int[] corners, int first, int n)
    {
        double area = 0;
        for (int i = 0; i < n; i++)
        {
            MapPoint a = projected[corners[first + i]], b = projected[corners[first + (i + 1) % n]];
            area += a.X * b.Y - b.X * a.Y;
        }

        return area;
    }
}
