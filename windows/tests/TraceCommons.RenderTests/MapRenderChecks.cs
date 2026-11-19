using System;
using System.IO;
using System.Numerics;
using System.Threading.Tasks;
using Microsoft.Graphics.Canvas;
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
