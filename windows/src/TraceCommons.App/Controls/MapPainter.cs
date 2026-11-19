using System;
using System.Collections.Generic;
using System.Numerics;
using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Brushes;
using Microsoft.Graphics.Canvas.Geometry;
using TraceCommons.Interop.Map;
using TraceCommons.Interop.Shell;
using Color = Windows.UI.Color;

namespace TraceCommons.App.Controls;

/// <summary>
/// The map's Win2D drawing that the render tests replay against the Mac's
/// ImageRenderer output: the station, the orb, the NEAR AI mark and the
/// tool logos. The map control and
/// tests/TraceCommons.RenderTests both compile this file, so a render test
/// draws with exactly the code the window draws with.
/// </summary>
internal static class MapPainter
{
    /// <summary>Path steps as a Win2D geometry, filled non-zero as SVG fills.</summary>
    public static CanvasGeometry Path(ICanvasResourceCreator device, IReadOnlyList<PathStep> steps)
    {
        using var builder = new CanvasPathBuilder(device);
        builder.SetFilledRegionDetermination(CanvasFilledRegionDetermination.Winding);
        bool open = false;
        foreach (PathStep s in steps)
        {
            switch (s.Kind)
            {
                case PathStepKind.Move:
                    if (open)
                    {
                        builder.EndFigure(CanvasFigureLoop.Open);
                    }

                    builder.BeginFigure((float)s.X, (float)s.Y);
                    open = true;
                    break;
                case PathStepKind.Line:
                    if (!open)
                    {
                        builder.BeginFigure((float)s.X, (float)s.Y);
                        open = true;
                    }
                    else
                    {
                        builder.AddLine((float)s.X, (float)s.Y);
                    }

                    break;
                case PathStepKind.Cubic:
                    builder.AddCubicBezier(new Vector2((float)s.X1, (float)s.Y1), new Vector2((float)s.X2, (float)s.Y2), new Vector2((float)s.X, (float)s.Y));
                    break;
                case PathStepKind.Close:
                    if (open)
                    {
                        builder.EndFigure(CanvasFigureLoop.Closed);
                        open = false;
                    }

                    break;
            }
        }

        if (open)
        {
            builder.EndFigure(CanvasFigureLoop.Open);
        }

        return CanvasGeometry.CreatePath(builder);
    }

    /// <summary>The NEAR AI mark in a unit square (<c>GlassBrandMark.unit</c>).</summary>
    public static CanvasGeometry MarkUnit(ICanvasResourceCreator device) => Path(device, NearMarkPath.Unit);

    /// <summary>A logo's paths as one geometry, in its viewBox units (<c>GlassToolLogoID.path</c>).</summary>
    public static CanvasGeometry? Logo(ICanvasResourceCreator device, string name)
    {
        if (!MapArt.Logos.TryGetValue(name, out LogoArtwork? art))
        {
            return null;
        }

        var parts = new List<CanvasGeometry>();
        foreach (string data in art.Paths)
        {
            if (SvgPathData.Parse(data) is { } steps)
            {
                parts.Add(Path(device, steps));
            }
        }

        return CanvasGeometry.CreateGroup(device, parts.ToArray(), CanvasFilledRegionDetermination.Winding);
    }

    /// <summary>
    /// The transform that fits a logo's viewBox in a square of
    /// <paramref name="side"/> centred on <paramref name="centre"/>, aspect
    /// kept (<c>GlassToolLogoShape.path(in:)</c>, GlassToolLogo.swift:41-50).
    /// </summary>
    public static Matrix3x2 LogoFit(LogoArtwork art, Vector2 centre, double side)
    {
        double scale = Math.Min(side / art.Width, side / art.Height);
        return Matrix3x2.CreateTranslation(-(float)(art.X + art.Width / 2), -(float)(art.Y + art.Height / 2))
            * Matrix3x2.CreateScale((float)scale)
            * Matrix3x2.CreateTranslation(centre);
    }

    /// <summary>
    /// The station at <paramref name="angle"/> in a <paramref name="width"/>
    /// by <paramref name="height"/> frame whose top-left is
    /// <paramref name="origin"/>: each face filled and edged 0.6 in its own
    /// colour, far to near (LibraryStation.swift:253-304).
    /// </summary>
    public static void Station(CanvasDrawingSession ds, ICanvasResourceCreator device, Vector2 origin, double width, double height, double angle)
    {
        using var seam = new CanvasStrokeStyle { LineJoin = CanvasLineJoin.Round };
        foreach (DrawnFace face in LibraryStation.Frame(width, height, angle))
        {
            using var builder = new CanvasPathBuilder(device);
            builder.BeginFigure(origin.X + (float)face.Points[0].X, origin.Y + (float)face.Points[0].Y);
            for (int i = 1; i < face.Points.Count; i++)
            {
                builder.AddLine(origin.X + (float)face.Points[i].X, origin.Y + (float)face.Points[i].Y);
            }

            builder.EndFigure(CanvasFigureLoop.Closed);
            using CanvasGeometry polygon = CanvasGeometry.CreatePath(builder);
            Color colour = Color.FromArgb(255, (byte)Math.Round(face.R * 255), (byte)Math.Round(face.G * 255), (byte)Math.Round(face.B * 255));
            ds.FillGeometry(polygon, colour);
            ds.DrawGeometry(polygon, colour, 0.6f, seam);
        }
    }

    /// <summary>
    /// The orb of radius <paramref name="r"/> at <paramref name="c"/>, its
    /// meridians at <paramref name="angle"/>, in the campaign blues
    /// (LibraryOrb.swift:76-120). The caller dims it while unlit.
    /// </summary>
    public static void Orb(CanvasDrawingSession ds, ICanvasResourceCreator device, Vector2 c, double r, double angle, Color blue, Color deep)
    {
        static Color With(Color c, double a) => Color.FromArgb((byte)Math.Round(c.A * a), c.R, c.G, c.B);
        static Color White(double a) => Color.FromArgb((byte)Math.Round(255 * a), 255, 255, 255);
        using (var glass = new CanvasRadialGradientBrush(device, new[]
        {
            new CanvasGradientStop { Position = 0, Color = With(blue, 0.95) },
            new CanvasGradientStop { Position = 0.5f, Color = With(deep, 0.95) },
            new CanvasGradientStop { Position = 1, Color = deep },
        })
        {
            Center = new Vector2((float)(c.X - r * 0.35), (float)(c.Y - r * 0.4)),
            RadiusX = (float)(r * 1.5),
            RadiusY = (float)(r * 1.5),
        })
        {
            ds.FillCircle(c, (float)r, glass);
        }

        using CanvasGeometry sphere = CanvasGeometry.CreateCircle(device, c, (float)r);
        using (ds.CreateLayer(1, sphere))
        {
            Matrix3x2 saved = ds.Transform;
            ds.Transform = Matrix3x2.CreateRotation((float)LibraryOrb.Lean) * Matrix3x2.CreateTranslation(c) * saved;
            foreach (double latitude in new[] { -0.5, 0.0, 0.5 })
            {
                double y = r * latitude, half = r * Math.Sqrt(1 - latitude * latitude);
                ds.DrawEllipse(0, (float)y, (float)half, (float)(half * 0.22), White(0.18), 1);
            }

            foreach ((double width, bool near) in LibraryOrb.MeridianWidths(angle))
            {
                ds.DrawEllipse(0, 0, (float)(r * width), (float)r, White(near ? 0.32 : 0.12), near ? 1.2f : 0.8f);
            }

            ds.Transform = saved;
        }

        double side = r * 0.95;
        using CanvasGeometry unit = MarkUnit(device);
        ds.FillGeometry(unit.Transform(Matrix3x2.CreateScale((float)side) * Matrix3x2.CreateTranslation((float)(c.X - side / 2), (float)(c.Y - side / 2))), White(1));
        using (var shine = new CanvasLinearGradientBrush(device, White(0.45), White(0))
        {
            StartPoint = new Vector2(c.X, (float)(c.Y - r * 0.78)),
            EndPoint = new Vector2(c.X, (float)(c.Y - r * 0.4)),
        })
        {
            // The highlight: an ellipse r*0.7 by r*0.38 whose top-left is (-0.62r, -0.78r).
            ds.FillEllipse((float)(c.X - r * 0.62 + r * 0.35), (float)(c.Y - r * 0.78 + r * 0.19), (float)(r * 0.35), (float)(r * 0.19), shine);
        }

        ds.DrawCircle(c, (float)r, White(0.35), 1);
    }
}
