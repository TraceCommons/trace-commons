using System;
using System.Collections.Generic;
using System.Linq;
using System.Numerics;
using TraceCommons.Interop.Shell;

namespace TraceCommons.Interop.Map;

/// <summary>A 3D point of the station's mesh.</summary>
public readonly record struct Vec3(double X, double Y, double Z)
{
    public static Vec3 operator +(Vec3 a, Vec3 b) => new(a.X + b.X, a.Y + b.Y, a.Z + b.Z);

    public static Vec3 operator -(Vec3 a, Vec3 b) => new(a.X - b.X, a.Y - b.Y, a.Z - b.Z);

    public static Vec3 operator *(Vec3 a, double k) => new(a.X * k, a.Y * k, a.Z * k);

    public static Vec3 operator /(Vec3 a, double k) => new(a.X / k, a.Y / k, a.Z / k);

    public static Vec3 operator -(Vec3 a) => new(-a.X, -a.Y, -a.Z);

    public double Length => Math.Sqrt(X * X + Y * Y + Z * Z);

    public static double Dot(Vec3 a, Vec3 b) => a.X * b.X + a.Y * b.Y + a.Z * b.Z;

    public static Vec3 Cross(Vec3 a, Vec3 b) => new(a.Y * b.Z - a.Z * b.Y, a.Z * b.X - a.X * b.Z, a.X * b.Y - a.Y * b.X);
}

/// <summary>One flat face of the station: its corners, colour (0..1) and whether it is the mark.</summary>
public sealed record StationFace(IReadOnlyList<Vec3> Points, Vec3 Color, bool MarkCap = false, bool Mark = false);

/// <summary>A face ready to fill: its projected corners and its shaded colour, far to near.</summary>
public readonly record struct DrawnFace(IReadOnlyList<MapPoint> Points, double R, double G, double B);

/// <summary>
/// The library on the rocket map: a space station round the NEAR AI mark,
/// a port of <c>GlassLibraryStation</c>
/// (macos/Sources/TCDesign/Components/LibraryStation.swift). The mesh is built
/// once; a frame turns it, projects it, sorts it and shades it.
/// </summary>
public static class LibraryStation
{
    /// <summary>One turn (:33).</summary>
    public const double TurnSeconds = 26;

    /// <summary>The resting angle (:35).</summary>
    public const double RestAngle = 0.6;

    /// <summary>The station's own frame rate, 20 frames a second (:39).</summary>
    public const double FrameInterval = 1.0 / 20;

    /// <summary>The station's frame on the rocket map, in design units (FlowMapView.swift:290).</summary>
    public const double FrameWidth = 150, FrameHeight = 130;

    /// <summary>The glow's strength (:57).</summary>
    public static double Glow(bool lit) => lit ? 0.35 : 0.25;

    /// <summary>The glow's radius (:51).</summary>
    public const double GlowRadius = 6;

    /// <summary>The angle at <paramref name="seconds"/> past the reference date (:60-64).</summary>
    public static double Angle(double seconds, bool reduceMotion) =>
        reduceMotion ? RestAngle : RestAngle + (seconds % TurnSeconds) / TurnSeconds * 2 * Math.PI;

    public static readonly double Tilt = 42 * Math.PI / 180;
    public static readonly double Lean = -14 * Math.PI / 180;
    public const double CameraDistance = 240;
    public static readonly Vec3 Light = Normalized(new Vec3(-0.42, 0.62, 0.66));

    private static readonly double Cx = Math.Cos(Tilt), Sx = Math.Sin(Tilt), Cz = Math.Cos(Lean), Sz = Math.Sin(Lean);

    private static readonly Lazy<IReadOnlyList<StationFace>> MeshValue = new(Build);

    /// <summary>The mesh, built once (:89).</summary>
    public static IReadOnlyList<StationFace> Mesh => MeshValue.Value;

    private static Vec3 Normalized(Vec3 v) => v / v.Length;

    /// <summary>
    /// The mark's outline from the brand path, curves flattened in five steps,
    /// centred, scaled to <paramref name="height"/> and turned y-up (:93-133).
    /// </summary>
    public static IReadOnlyList<(double X, double Y)> MarkOutline(double height)
    {
        IReadOnlyList<PathStep>? path = SvgPathData.Parse(NearMarkPath.Data);
        if (path is null)
        {
            return Array.Empty<(double, double)>();
        }

        var points = new List<(double X, double Y)>();
        (double X, double Y) current = (0, 0);
        foreach (PathStep e in path)
        {
            switch (e.Kind)
            {
                case PathStepKind.Move or PathStepKind.Line:
                    current = (e.X, e.Y);
                    points.Add(current);
                    break;
                case PathStepKind.Cubic:
                    var curve = new[] { current, (e.X1, e.Y1), (e.X2, e.Y2), (e.X, e.Y) };
                    for (int step = 1; step <= 5; step++)
                    {
                        points.Add(Bezier(curve, step / 5.0));
                    }

                    current = (e.X, e.Y);
                    break;
            }
        }

        LayoutRect box = NearMarkPath.ViewBox;
        double k = height / box.Height;
        double midX = box.X + box.Width / 2, midY = box.Y + box.Height / 2;
        var outline = new List<(double X, double Y)>();
        foreach ((double x, double y) in points)
        {
            (double X, double Y) q = ((x - midX) * k, -(y - midY) * k);
            if (outline.Count > 0 && Distance(q, outline[^1]) <= 0.05)
            {
                continue;
            }

            outline.Add(q);
        }

        if (outline.Count > 1 && Distance(outline[0], outline[^1]) < 0.05)
        {
            outline.RemoveAt(outline.Count - 1);
        }

        return outline;
    }

    private static double Distance((double X, double Y) a, (double X, double Y) b) => Math.Sqrt((a.X - b.X) * (a.X - b.X) + (a.Y - b.Y) * (a.Y - b.Y));

    /// <summary>De Casteljau (:142-155).</summary>
    public static (double X, double Y) Bezier(IReadOnlyList<(double X, double Y)> controls, double t)
    {
        var level = controls.ToList();
        while (level.Count > 1)
        {
            var next = new List<(double X, double Y)>();
            for (int i = 0; i < level.Count - 1; i++)
            {
                next.Add((level[i].X + (level[i + 1].X - level[i].X) * t, level[i].Y + (level[i + 1].Y - level[i].Y) * t));
            }

            level = next;
        }

        return level.Count > 0 ? level[0] : (0, 0);
    }

    /// <summary>The mesh (:157-222).</summary>
    private static IReadOnlyList<StationFace> Build()
    {
        var faces = new List<StationFace>();
        Vec3 Rgb(double r, double g, double b) => new Vec3(r, g, b) / 255;
        void Box(Vec3 c, Vec3 s, Vec3 color)
        {
            Vec3 a = c - s / 2, b = c + s / 2;
            Vec3 P(double x, double y, double z) => new(x, y, z);
            faces.Add(new StationFace(new[] { P(a.X, a.Y, b.Z), P(b.X, a.Y, b.Z), P(b.X, b.Y, b.Z), P(a.X, b.Y, b.Z) }, color));
            faces.Add(new StationFace(new[] { P(b.X, a.Y, a.Z), P(a.X, a.Y, a.Z), P(a.X, b.Y, a.Z), P(b.X, b.Y, a.Z) }, color));
            faces.Add(new StationFace(new[] { P(a.X, b.Y, a.Z), P(a.X, b.Y, b.Z), P(b.X, b.Y, b.Z), P(b.X, b.Y, a.Z) }, color));
            faces.Add(new StationFace(new[] { P(a.X, a.Y, b.Z), P(a.X, a.Y, a.Z), P(b.X, a.Y, a.Z), P(b.X, a.Y, b.Z) }, color));
            faces.Add(new StationFace(new[] { P(b.X, a.Y, b.Z), P(b.X, a.Y, a.Z), P(b.X, b.Y, a.Z), P(b.X, b.Y, b.Z) }, color));
            faces.Add(new StationFace(new[] { P(a.X, a.Y, a.Z), P(a.X, a.Y, b.Z), P(a.X, b.Y, b.Z), P(a.X, b.Y, a.Z) }, color));
        }

        IReadOnlyList<(double X, double Y)> mark = MarkOutline(30);
        const double H = 3.2;
        Vec3 blue = Rgb(0, 145, 253);
        faces.Add(new StationFace(mark.Select(p => new Vec3(p.X, H, -p.Y)).ToList(), blue, MarkCap: true, Mark: true));
        faces.Add(new StationFace(mark.Reverse().Select(p => new Vec3(p.X, -H, -p.Y)).ToList(), blue, MarkCap: true, Mark: true));
        for (int j = 0; j < mark.Count; j++)
        {
            (double X, double Y) p = mark[j], q = mark[(j + 1) % mark.Count];
            faces.Add(new StationFace(
                new[] { new Vec3(p.X, H, -p.Y), new Vec3(q.X, H, -q.Y), new Vec3(q.X, -H, -q.Y), new Vec3(p.X, -H, -p.Y) },
                Rgb(0, 98, 176), Mark: true));
        }

        const double RingRadius = 36.0, Tube = 3.6;
        const int Around = 40, Across = 8;
        Vec3 Torus(int u, int v)
        {
            double a = (double)u / Around * 2 * Math.PI, b = (double)v / Across * 2 * Math.PI;
            double r = RingRadius + Tube * Math.Cos(b);
            return new Vec3(r * Math.Cos(a), Tube * Math.Sin(b), r * Math.Sin(a));
        }

        for (int u = 0; u < Around; u++)
        {
            for (int v = 0; v < Across; v++)
            {
                faces.Add(new StationFace(new[] { Torus(u, v), Torus(u + 1, v), Torus(u + 1, v + 1), Torus(u, v + 1) }, Rgb(172, 196, 230)));
            }
        }

        Vec3 spoke = Rgb(140, 164, 200);
        const double Hub = 17.0, Length = RingRadius - Hub, Mid = (Hub + RingRadius) / 2;
        Box(new Vec3(0, 0, Mid), new Vec3(1.8, 1.8, Length), spoke);
        Box(new Vec3(0, 0, -Mid), new Vec3(1.8, 1.8, Length), spoke);
        Box(new Vec3(Mid, 0, 0), new Vec3(Length, 1.8, 1.8), spoke);
        Box(new Vec3(-Mid, 0, 0), new Vec3(Length, 1.8, 1.8), spoke);
        foreach (double degrees in new[] { 45.0, 135, 225, 315 })
        {
            double a = degrees * Math.PI / 180;
            Box(new Vec3(RingRadius * Math.Cos(a), 0, RingRadius * Math.Sin(a)), new Vec3(6, 5, 6), Rgb(226, 236, 248));
        }

        Vec3 truss = Rgb(120, 134, 156), panel = Rgb(44, 96, 176);
        foreach (double side in new[] { 1.0, -1.0 })
        {
            Box(new Vec3(side * (RingRadius + 9), 0, 0), new Vec3(14, 1.6, 1.6), truss);
            Box(new Vec3(side * (RingRadius + 20), 0, 0), new Vec3(16, 0.9, 20), panel);
            Box(new Vec3(side * (RingRadius + 20), 0, 0), new Vec3(0.9, 1.1, 20.4), truss);
        }

        return faces;
    }

    /// <summary>A mesh point turned about the station's axis, tilted and leaned (:238-251).</summary>
    public static Vec3 Turned(Vec3 p, double angle) => Turned(p, Math.Cos(angle), Math.Sin(angle));

    public static Vec3 Turned(Vec3 p, double cy, double sy)
    {
        double x = p.X * cy + p.Z * sy, z = -p.X * sy + p.Z * cy, y = p.Y;
        double y2 = y * Cx - z * Sx, z2 = y * Sx + z * Cx;
        return new Vec3(x * Cz - y2 * Sz, x * Sz + y2 * Cz, z2);
    }

    /// <summary>
    /// One frame in a <paramref name="width"/> by <paramref name="height"/>
    /// frame: every face turned, projected, shaded and sorted far to near
    /// (:253-304). The caller fills each and strokes it 0.6 wide in its own
    /// colour, round joins, to close the seams.
    /// </summary>
    public static IReadOnlyList<DrawnFace> Frame(double width, double height, double angle)
    {
        double scale = Math.Min(width / 140, height / 120);
        MapPoint Project(Vec3 q)
        {
            double s = scale * CameraDistance / (CameraDistance - q.Z);
            return new MapPoint(width / 2 + q.X * s, height / 2 - q.Y * s);
        }

        double cy = Math.Cos(angle), sy = Math.Sin(angle);
        var drawn = new List<(Vec3[] Points, Vec3 Color, double Shade, double Spec, double Depth, bool Cap)>(Mesh.Count);
        double markNear = double.NegativeInfinity, markFar = double.PositiveInfinity;
        foreach (StationFace face in Mesh)
        {
            Vec3[] q = face.Points.Select(p => Turned(p, cy, sy)).ToArray();
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

            double lambert = Math.Max(0, Vec3.Dot(n, Light));
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

        // Swift's sort is stable for equal depths; OrderBy is too.
        return drawn.OrderBy(d => d.Depth).Select(d =>
        {
            Vec3 c = d.Color * d.Shade + new Vec3(d.Spec, d.Spec, d.Spec);
            return new DrawnFace(d.Points.Select(Project).ToList(), Math.Min(1, c.X), Math.Min(1, c.Y), Math.Min(1, c.Z));
        }).ToList();
    }
}

/// <summary>
/// The library on the standard map: a glass orb round the NEAR AI mark, its
/// meridians turning about a leaning axis, a port of <c>GlassLibraryOrb</c>
/// (macos/Sources/TCDesign/Components/LibraryOrb.swift).
/// </summary>
public static class LibraryOrb
{
    public const double TurnSeconds = 18;
    public const double RestAngle = 0.4;
    public const double Inset = 0.29;
    public const int Meridians = 6;
    public static readonly double Lean = -14 * Math.PI / 180;

    /// <summary>The orb's frame rate, the station's (:36).</summary>
    public const double FrameInterval = LibraryStation.FrameInterval;

    /// <summary>The glow: brand blue at 0.5 lit, 0.15 not, radius 10 (:47).</summary>
    public static double Glow(bool lit) => lit ? 0.5 : 0.15;

    public const double GlowRadius = 10;

    /// <summary>The orb drawn dimmer while nothing is contributed (:86).</summary>
    public static double Opacity(bool lit) => lit ? 1 : 0.6;

    /// <summary>The angle at <paramref name="seconds"/> (:52-55).</summary>
    public static double Angle(double seconds, bool reduceMotion) =>
        reduceMotion ? RestAngle : RestAngle + (seconds % TurnSeconds) / TurnSeconds * 2 * Math.PI;

    /// <summary>Each meridian's half-width share and whether it is near (:63-68).</summary>
    public static IReadOnlyList<(double Width, bool Near)> MeridianWidths(double angle) =>
        Enumerable.Range(0, Meridians).Select(i =>
        {
            double phase = angle + i * Math.PI / Meridians;
            return (Math.Abs(Math.Cos(phase)), Math.Sin(phase) >= 0);
        }).ToList();

    /// <summary>The sphere's radius in a frame (:71-74).</summary>
    public static double Radius(double width, double height) => Math.Min(width, height) / 2 * (1 - Inset);
}

/// <summary>
/// The map's field: the mark tile, its dots, the light under the pointer
/// and the shimmer (<c>GlassMapField</c>, <c>GlassMapMarks</c>,
/// macos/Sources/TCDesign/Components/Indicators.swift:563-885).
/// </summary>
public static class MapField
{
    /// <summary>The field's ellipse radii as fractions, and its centre (:573-574).</summary>
    public const double RadiusX = 0.8, RadiusY = 0.6, CentreX = 0.5, CentreY = 0.55;

    public const double Pitch = 28;
    public const double MarkSize = 5.6;
    public const double DotSize = 1.6;

    /// <summary>The shimmer: first pass 1 s after shown, then every 40 s, 1.1 s each (:666-668).</summary>
    public const double ShimmerFirst = 1;
    public const double ShimmerEvery = 40;
    public const double ShimmerDuration = 1.1;

    /// <summary>The shimmer's timing curve (:672-674).</summary>
    public const double ShimmerX1 = 0.3, ShimmerY1 = 0, ShimmerX2 = 0.15, ShimmerY2 = 1;

    public const double ShimmerHalfWidth = 0.1;
    public const double ShimmerSlant = 20;

    /// <summary>How far the pointer's light reaches (:677).</summary>
    public const double Reach = 140;

    /// <summary>
    /// The marks' inks in the flat theme, dark then light: at rest, lit and
    /// at the shimmer's crest (:647-665), and the bloom (:668).
    /// </summary>
    public static (MapRgba Resting, MapRgba Lit, MapRgba Crest, MapRgba Bloom) Inks(bool dark) => dark
        ? (new MapRgba(255, 255, 255, 0.096), new MapRgba(255, 255, 255, 0.2), new MapRgba(255, 255, 255, 0.24), new MapRgba(255, 255, 255, 0.035))
        : (new MapRgba(255, 255, 255, 0.4), new MapRgba(255, 255, 255, 0.49), new MapRgba(255, 255, 255, 0.49), new MapRgba(255, 255, 255, 0.2));

    /// <summary>The tile's fade: stops (location, opacity), centre and radius fraction (:681-690).</summary>
    public static readonly IReadOnlyList<(double At, double Opacity)> FadeStops = new[] { (0.0, 1.0), (0.55, 0.85), (1.0, 0.0) };

    public const double FadeCentreX = 0.5, FadeCentreY = 0.52, FadeRadius = 0.62;

    /// <summary>Every mark's top-left corner from the cells that touch <paramref name="near"/> (:694-712).</summary>
    public static IReadOnlyList<MapPoint> Origins(MapSize size, (double X, double Y, double W, double H)? near = null)
    {
        if (Area(size, near) is not { } area)
        {
            return Array.Empty<MapPoint>();
        }

        var result = new List<MapPoint>();
        for (int row = (int)Math.Floor(area.Y / Pitch); row <= (int)Math.Floor((area.Y + area.H) / Pitch); row++)
        {
            for (int column = (int)Math.Floor(area.X / Pitch); column <= (int)Math.Floor((area.X + area.W) / Pitch); column++)
            {
                foreach (double quarter in new[] { 0.25, 0.75 })
                {
                    double offset = Pitch * quarter - MarkSize / 2;
                    result.Add(new MapPoint(column * Pitch + offset, row * Pitch + offset));
                }
            }
        }

        return result;
    }

    /// <summary>The dots' centres (:727-749).</summary>
    public static IReadOnlyList<MapPoint> Dots(MapSize size, (double X, double Y, double W, double H)? near = null)
    {
        if (Area(size, near) is not { } area)
        {
            return Array.Empty<MapPoint>();
        }

        var result = new List<MapPoint>();
        for (int row = (int)Math.Floor(area.Y / Pitch); row <= (int)Math.Floor((area.Y + area.H) / Pitch); row++)
        {
            for (int column = (int)Math.Floor(area.X / Pitch); column <= (int)Math.Floor((area.X + area.W) / Pitch); column++)
            {
                double x = column * Pitch, y = row * Pitch;
                result.Add((row + column) % 2 == 0 ? new MapPoint(x + Pitch * 0.75, y + Pitch * 0.25) : new MapPoint(x + Pitch * 0.25, y + Pitch * 0.75));
            }
        }

        return result;
    }

    private static (double X, double Y, double W, double H)? Area(MapSize size, (double X, double Y, double W, double H)? near)
    {
        (double X, double Y, double W, double H) rect = near ?? (0, 0, size.Width, size.Height);
        double x0 = Math.Max(rect.X, 0), y0 = Math.Max(rect.Y, 0);
        double x1 = Math.Min(rect.X + rect.W, size.Width), y1 = Math.Min(rect.Y + rect.H, size.Height);
        return x1 - x0 > 0 && y1 - y0 > 0 ? (x0, y0, x1 - x0, y1 - y0) : null;
    }

    /// <summary>How lit a mark is at <paramref name="distance"/> from the pointer (:768-773).</summary>
    public static double Light(double distance)
    {
        if (distance >= Reach)
        {
            return 0;
        }

        double t = 1 - distance / Reach * (distance / Reach);
        return t * t;
    }

    /// <summary>
    /// The shimmer band at <paramref name="sweep"/>: the gradient's start and
    /// end, leaning 20 degrees, crossing left to right (:719-722,
    /// ShimmerBand.swift:13-27).
    /// </summary>
    public static (MapPoint Start, MapPoint End) ShimmerBand(double sweep, MapSize size)
    {
        double progress = sweep - Math.Floor(sweep);
        double angle = ShimmerSlant * Math.PI / 180;
        double ux = Math.Cos(angle), uy = Math.Sin(angle);
        double[] along = { 0, size.Width * ux, size.Height * uy, size.Width * ux + size.Height * uy };
        double half = ShimmerHalfWidth * Math.Max(Math.Sqrt(size.Width * size.Width + size.Height * size.Height), 1);
        double from = along.Min() - half, to = along.Max() + half;
        double centre = from + (to - from) * progress;
        return (new MapPoint(ux * (centre - half), uy * (centre - half)), new MapPoint(ux * (centre + half), uy * (centre + half)));
    }

    /// <summary>
    /// When the shimmer passes: the seconds since the map was shown to the
    /// sweep it draws (0 off the field), passes 1 s after showing and every
    /// 40 s after, each 1.1 s on its curve (:787-807). None under reduce motion.
    /// </summary>
    public static double Sweep(double secondsShown, bool reduceMotion)
    {
        if (reduceMotion || secondsShown < ShimmerFirst)
        {
            return 0;
        }

        double since = (secondsShown - ShimmerFirst) % ShimmerEvery;
        return since >= ShimmerDuration ? 0 : Shell.MonitorMotion.CubicBezier(ShimmerX1, ShimmerY1, ShimmerX2, ShimmerY2, since / ShimmerDuration);
    }

    /// <summary>The seconds until the next shimmer frame is needed, from <paramref name="secondsShown"/>.</summary>
    public static double UntilNextPass(double secondsShown)
    {
        if (secondsShown < ShimmerFirst)
        {
            return ShimmerFirst - secondsShown;
        }

        double since = (secondsShown - ShimmerFirst) % ShimmerEvery;
        return since < ShimmerDuration ? 0 : ShimmerEvery - since;
    }
}
