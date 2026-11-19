using System;
using System.IO;
using System.Numerics;
using System.Threading.Tasks;
using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Geometry;
using TraceCommons.App.Controls;
using TraceCommons.Interop.Map;
using Windows.UI;

namespace TraceCommons.RenderTests;

/// <summary>
/// The map art (slice W6) drawn by the window's own painter
/// (<see cref="MapPainter"/>) against the Mac's ImageRenderer renders of
/// the same art at the same size, flat theme, printed by the W3 probe at
/// 841d81ba5 (Fixtures/mac-station-150x130.png from
/// <c>GlassLibraryStation.draw(in:size:angle: 0.6)</c>, mac-orb-104.png from
/// <c>GlassLibraryOrb.draw(in:size:angle: 0.4, lit: true)</c>, and
/// mac-logo-*-48.png from <c>GlassToolLogoShape</c> filled black). The
/// station's glow and the orb's are the caller's shadows and are not in
/// either render. Text is not drawn. Thresholds sit just outside what the
/// box measured on 2026-11-19 (station iou 0.9892 mean 0.0016, orb 0.9933 /
/// 0.0045, logos iou 0.9838 to 1.0, mean at most 0.0013).
/// </summary>
public static partial class RenderChecks
{
    [RenderTest]
    public static string StationMatchesTheMacsRender(CanvasDevice device, string outDir)
    {
        const int W = 150, H = 130;
        using var target = new CanvasRenderTarget(device, W, H, 96f);
        using (CanvasDrawingSession ds = target.CreateDrawingSession())
        {
            ds.Clear(Color.FromArgb(0, 0, 0, 0));
            MapPainter.Station(ds, device, Vector2.Zero, W, H, 0.6);
        }

        return Compare(device, target, "mac-station-150x130.png", outDir, "win-station-150x130.png", minIou: 0.98, maxMean: 0.005);
    }

    [RenderTest]
    public static string OrbMatchesTheMacsRender(CanvasDevice device, string outDir)
    {
        const int Side = 104;
        using var target = new CanvasRenderTarget(device, Side, Side, 96f);
        IReadOnlyDictionaryHolder inks = new(MapArt.Light);
        using (CanvasDrawingSession ds = target.CreateDrawingSession())
        {
            ds.Clear(Color.FromArgb(0, 0, 0, 0));
            MapPainter.Orb(ds, device, new Vector2(Side / 2f, Side / 2f), LibraryOrb.Radius(Side, Side), 0.4,
                inks.Color("heroCampaignBright"), inks.Color("heroCampaignDeep"));
        }

        return Compare(device, target, "mac-orb-104.png", outDir, "win-orb-104.png", minIou: 0.985, maxMean: 0.01);
    }

    [RenderTest]
    public static string ToolLogosMatchTheMacsRenders(CanvasDevice device, string outDir)
    {
        const int Side = 48;
        var lines = new System.Collections.Generic.List<string>();
        foreach ((string name, LogoArtwork art) in MapArt.Logos)
        {
            using var target = new CanvasRenderTarget(device, Side, Side, 96f);
            using (CanvasDrawingSession ds = target.CreateDrawingSession())
            {
                ds.Clear(Color.FromArgb(0, 0, 0, 0));
                using var logo = MapPainter.Logo(device, name)!;
                ds.FillGeometry(logo.Transform(MapPainter.LogoFit(art, new Vector2(Side / 2f, Side / 2f), Side)), Color.FromArgb(255, 0, 0, 0));
            }

            lines.Add(name + " " + Compare(device, target, $"mac-logo-{name}-48.png", outDir, $"win-logo-{name}-48.png", minIou: 0.975, maxMean: 0.004));
        }

        return string.Join("; ", lines);
    }

    /// <summary>
    /// The flowing arc's dashes, drawn by the window's painter on a straight
    /// line at 0 s and 1/6 s, move 3 points from the arc's start toward its
    /// end, as the Mac's do (FlowMapView.swift:84 and :294, phase 18 points a
    /// second, <c>dashPhase: -phase * scale</c>). Direct2D's dash offset is
    /// in stroke widths and its sign is not CoreGraphics'; this reads the
    /// drawn pixels, not the number.
    /// </summary>
    [RenderTest]
    public static string FlowingDashesMoveFromTheArcsStart(CanvasDevice device, string outDir)
    {
        bool[] At(double seconds)
        {
            using var target = new CanvasRenderTarget(device, 200, 20, 96f);
            using (CanvasDrawingSession ds = target.CreateDrawingSession())
            {
                ds.Clear(Color.FromArgb(0, 0, 0, 0));
                using var builder = new CanvasPathBuilder(device);
                builder.BeginFigure(10, 10);
                builder.AddCubicBezier(new Vector2(70, 10), new Vector2(130, 10), new Vector2(190, 10));
                builder.EndFigure(CanvasFigureLoop.Open);
                using CanvasGeometry line = CanvasGeometry.CreatePath(builder);
                MapPainter.FlowingDash(ds, line, 1, seconds, reduceMotion: false, Color.FromArgb(255, 0, 0, 0));
            }

            byte[] px = target.GetPixelBytes();
            var on = new bool[200];
            for (int x = 0; x < 200; x++)
            {
                on[x] = px[(10 * 200 + x) * 4 + 3] >= 128;
            }

            return on;
        }

        bool[] before = At(0), after = At(1.0 / 6);
        int Matches(int shift)
        {
            int n = 0;
            for (int x = 30; x < 170; x++)
            {
                n += after[x] == before[x - shift] ? 1 : 0;
            }

            return n;
        }

        int forward = Matches(3), backward = Matches(-3);
        string numbers = $"forward={forward} backward={backward} of 140";
        if (forward < 134 || forward <= backward + 20)
        {
            throw new InvalidOperationException(numbers + " (want the dashes 3 px further along, toward the end)");
        }

        return numbers;
    }

    /// <summary>
    /// The arcs as the window draws them (<see cref="MapPainter.DrawArc"/>),
    /// read from the drawn pixels against the Mac's numbers
    /// (FlowMapView.swift:268-294): at scale 4 a healthy flowing arc's moving
    /// dash repeats every (4 + 8) x 4 = 48 px and is 1.4 x 1.3 x 4 = 7.28 px
    /// thick; a dashed arc repeats every (5 + 5) x 4 = 40 px and is
    /// 1.4 x 4 = 5.6 px thick. This replaces a check that only read the
    /// window's source for the call (B2 of #2215).
    /// </summary>
    [RenderTest]
    public static string ArcsDrawTheMacsStrokes(CanvasDevice device, string outDir)
    {
        (double Period, int Thick) Measure(MapArcStyle style, byte threshold)
        {
            const int w = 400, h = 40;
            using var target = new CanvasRenderTarget(device, w, h, 96f);
            using (CanvasDrawingSession ds = target.CreateDrawingSession())
            {
                ds.Clear(Color.FromArgb(0, 0, 0, 0));
                using var builder = new CanvasPathBuilder(device);
                builder.BeginFigure(10, 20);
                builder.AddCubicBezier(new Vector2(140, 20), new Vector2(260, 20), new Vector2(390, 20));
                builder.EndFigure(CanvasFigureLoop.Open);
                using CanvasGeometry line = CanvasGeometry.CreatePath(builder);
                MapPainter.DrawArc(ds, line, style, healthy: true, strong: false, 4, 0, reduceMotion: true,
                    // Opaque, except a flowing arc's line under its dash.
                    stroke => Color.FromArgb(
                        style == MapArcStyle.Flowing && !stroke.Moves ? (byte)Math.Round(255 * stroke.Alpha) : (byte)255, 0, 0, 0));
            }

            byte[] px = target.GetPixelBytes();
            bool On(int x, int y) => px[(y * w + x) * 4 + 3] >= threshold;
            var rises = new System.Collections.Generic.List<int>();
            for (int x = 21; x < w - 20; x++)
            {
                if (On(x, 20) && !On(x - 1, 20))
                {
                    rises.Add(x);
                }
            }

            if (rises.Count < 3)
            {
                throw new InvalidOperationException($"{style}: {rises.Count} dashes drawn");
            }

            double period = (double)(rises[^1] - rises[0]) / (rises.Count - 1);
            int column = rises[1] + 2, thick = 0;
            for (int y = 0; y < h; y++)
            {
                thick += On(column, y) ? 1 : 0;
            }

            return (period, thick);
        }

        // The flowing arc's line is drawn at 0.35 under its dash: read the dash alone.
        (double flowPeriod, int flowThick) = Measure(MapArcStyle.Flowing, 200);
        (double dashPeriod, int dashThick) = Measure(MapArcStyle.Dashed, 128);
        string numbers = $"flowing period={flowPeriod:F2} thick={flowThick}; dashed period={dashPeriod:F2} thick={dashThick}";
        if (Math.Abs(flowPeriod - 48) > 1 || Math.Abs(flowThick - 7.28) > 1.5
            || Math.Abs(dashPeriod - 40) > 1 || Math.Abs(dashThick - 5.6) > 1.5)
        {
            throw new InvalidOperationException(numbers + " (want 48 and ~7.3; 40 and ~5.6)");
        }

        return numbers;
    }

    /// <summary>
    /// Alpha IoU (covered at 50%) and the mean absolute difference over every
    /// channel of the premultiplied pixels, against a Mac fixture.
    /// </summary>
    private static string Compare(CanvasDevice device, CanvasRenderTarget ours, string fixture, string outDir, string saveAs, double minIou, double maxMean)
    {
        Task.Run(async () => await ours.SaveAsync(Path.Combine(outDir, saveAs))).GetAwaiter().GetResult();
        using CanvasBitmap mac = Task.Run(async () => await CanvasBitmap.LoadAsync(
            device, Path.Combine(AppContext.BaseDirectory, "Fixtures", fixture), 96f)).GetAwaiter().GetResult();
        if (mac.SizeInPixels.Width != ours.SizeInPixels.Width || mac.SizeInPixels.Height != ours.SizeInPixels.Height)
        {
            throw new InvalidOperationException($"{fixture} is {mac.SizeInPixels.Width}x{mac.SizeInPixels.Height}, ours {ours.SizeInPixels.Width}x{ours.SizeInPixels.Height}");
        }

        byte[] a = ours.GetPixelBytes(), b = mac.GetPixelBytes();
        double sum = 0;
        int both = 0, either = 0, max = 0;
        for (int i = 0; i < a.Length; i++)
        {
            int d = Math.Abs(a[i] - b[i]);
            sum += d;
            max = Math.Max(max, d);
            if (i % 4 == 3)
            {
                bool inA = a[i] >= 128, inB = b[i] >= 128;
                both += inA && inB ? 1 : 0;
                either += inA || inB ? 1 : 0;
            }
        }

        double mean = sum / a.Length / 255.0;
        double iou = either == 0 ? 0 : (double)both / either;
        string numbers = $"meanAbs={mean:F4} max={max} iou={iou:F4}";
        if (iou < minIou || mean > maxMean)
        {
            throw new InvalidOperationException($"{fixture}: {numbers} (want iou >= {minIou}, mean <= {maxMean})");
        }

        return numbers;
    }

    private readonly struct IReadOnlyDictionaryHolder
    {
        private readonly System.Collections.Generic.IReadOnlyDictionary<string, MapRgba> _inks;

        public IReadOnlyDictionaryHolder(System.Collections.Generic.IReadOnlyDictionary<string, MapRgba> inks) => _inks = inks;

        public Color Color(string token)
        {
            MapRgba c = _inks[token];
            return Windows.UI.Color.FromArgb((byte)Math.Round(255 * c.A), c.R, c.G, c.B);
        }
    }
}
