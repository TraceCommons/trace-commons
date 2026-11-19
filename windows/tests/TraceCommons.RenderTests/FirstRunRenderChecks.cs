using System;
using System.Collections.Generic;
using System.IO;
using System.Numerics;
using System.Threading.Tasks;
using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Geometry;
using Microsoft.Graphics.Canvas.Text;
using TraceCommons.App.FirstRun;
using TraceCommons.Interop.FirstRun;
using Windows.UI;

namespace TraceCommons.RenderTests;

/// <summary>The first-run stage's render tests (W8).</summary>
public static partial class RenderChecks
{
    /// <summary>
    /// Direct2D's dash offset read the way the stage relies on: under the
    /// Mac's <c>dashPhase: -flow</c> (ToolsScreen.swift:574,
    /// PrivateAIScreen.swift:97) the dashes move forward along the line, from
    /// its start to its end. Draws one dashed line at flow 0 and flow 8 and
    /// finds the first dash's left edge in each. Red before the stage set
    /// butt dash caps: Win2D's default square cap put it 6 along, not 8.
    /// </summary>
    [RenderTest]
    public static string FirstRunDashesMoveForwardUnderANegativePhase(CanvasDevice device, string outDir)
    {
        double[] dash = { 4, 36 };
        const double width = 4;
        int FirstDash(double flow)
        {
            using var target = new CanvasRenderTarget(device, 220, 20, 96f);
            using (CanvasDrawingSession ds = target.CreateDrawingSession())
            {
                ds.Clear(Color.FromArgb(0, 0, 0, 0));
                // The stage's own stroke style (butt dashes, as the Mac's default).
                using CanvasStrokeStyle style = FirstRunStagePainter.Dashes(dash, width, -flow);
                ds.DrawLine(new Vector2(10, 10), new Vector2(210, 10), Color.FromArgb(255, 255, 255, 255), (float)width, style);
            }

            byte[] px = target.GetPixelBytes();
            for (int x = 10; x < 210; x++)
            {
                if (px[(10 * 220 + x) * 4 + 3] >= 128)
                {
                    return x;
                }
            }

            return -1;
        }

        int at0 = FirstDash(0), at8 = FirstDash(8);
        string numbers = $"firstDashAtFlow0={at0} firstDashAtFlow8={at8}";
        // The first dash starts at the line's start at flow 0; under -8 it
        // starts 8 further along (the 4 pt dash before it wraps in front).
        if (at0 != 10 || Math.Abs(at8 - at0 - 8) > 1)
        {
            throw new InvalidOperationException(numbers + " (want the dash at 10, then 8 further along)");
        }

        return numbers;
    }

    /// <summary>
    /// Welcome's final frame (animations off) draws the scene and the N in
    /// firstRunGlow at the hub's place, 132 across, the glow brightest in the
    /// top-left corner (FirstRunStage.swift:109-149, WelcomeScreen.swift:111-118).
    /// Measures drawn pixels; a render diff against the Mac's ImageRenderer
    /// render is still to come (no Mac fixture yet).
    /// </summary>
    [RenderTest]
    public static string FirstRunWelcomeFinalFrameDrawsTheNOnTheScene(CanvasDevice device, string outDir)
    {
        var model = new FirstRunStageModel();
        model.SetReduceMotion(true);
        model.SetGraphicTop(FirstRunStep.Welcome, 200);
        using var painter = new FirstRunStagePainter(new Dictionary<string, string>());
        using var target = new CanvasRenderTarget(device, 860, 760, 96f);
        using (CanvasDrawingSession ds = target.CreateDrawingSession())
        {
            painter.Draw(ds, device, model, 100, 860, 760);
        }

        Task.Run(async () => await target.SaveAsync(Path.Combine(outDir, "win-first-run-welcome-final.png"))).GetAwaiter().GetResult();
        byte[] px = target.GetPixelBytes();
        Color At(int x, int y)
        {
            int i = (y * 860 + x) * 4;
            return Color.FromArgb(px[i + 3], px[i + 2], px[i + 1], px[i]);
        }

        Color glow = FirstRunStagePainter.Argb(FirstRunTokens.FirstRunGlow);
        int markPixels = 0;
        for (int y = 200; y < 332; y++)
        {
            for (int x = 430 - 66; x < 430 + 66; x++)
            {
                Color c = At(x, y);
                if (Math.Abs(c.R - glow.R) + Math.Abs(c.G - glow.G) + Math.Abs(c.B - glow.B) < 24)
                {
                    markPixels++;
                }
            }
        }

        double coverage = markPixels / (132.0 * 132.0);
        Color corner = At(20, 20), far = At(840, 740);
        int Sum(Color c) => c.R + c.G + c.B;
        string numbers = $"markCoverage={coverage:F4} corner={corner.R},{corner.G},{corner.B} far={far.R},{far.G},{far.B}";
        if (coverage < 0.3 || Sum(corner) <= Sum(far))
        {
            throw new InvalidOperationException(numbers + " (want the N over 30% of its frame and the corner glow brighter than the far corner)");
        }

        return numbers;
    }

    /// <summary>
    /// A window smaller than the stage scales the stage down and still paints
    /// the scene to every edge, as the Mac's first-run window draws
    /// FirstRunBackdrop at the window's size behind the scaled stage
    /// (FirstRunViews.swift:28-29, OnboardingCoordinatorView.swift:205-221).
    /// Red before the painter drew the ground at the window's size: at 700 by
    /// 560 the stage letterboxed to 634 across and left 33-pixel bands at
    /// the sides unpainted (box tc-win-az3, 2026-11-19: white bands either
    /// side of every step).
    /// </summary>
    [RenderTest]
    public static string FirstRunScaledStagePaintsToEveryEdge(CanvasDevice device, string outDir)
    {
        const int w = 700, h = 560;
        var model = new FirstRunStageModel();
        model.SetReduceMotion(true);
        model.SetStep(FirstRunStep.Tools, 0);
        model.SetGraphicTop(FirstRunStep.Tools, 120);
        using var painter = new FirstRunStagePainter(new Dictionary<string, string>());
        using var target = new CanvasRenderTarget(device, w, h, 96f);
        using (CanvasDrawingSession ds = target.CreateDrawingSession())
        {
            ds.Clear(Color.FromArgb(0, 0, 0, 0));
            painter.Draw(ds, device, model, 100, w, h);
        }

        Task.Run(async () => await target.SaveAsync(Path.Combine(outDir, "win-first-run-scaled-700x560.png"))).GetAwaiter().GetResult();
        byte[] px = target.GetPixelBytes();
        int Alpha(int x, int y) => px[(y * w + x) * 4 + 3];
        var points = new (int X, int Y)[] { (2, 2), (w - 3, 2), (2, h - 3), (w - 3, h - 3), (2, h / 2), (w - 3, h / 2) };
        var alphas = new List<string>();
        bool all = true;
        foreach (var (x, y) in points)
        {
            int a = Alpha(x, y);
            alphas.Add($"{x},{y}={a}");
            all &= a == 255;
        }

        string numbers = "edgeAlpha " + string.Join(" ", alphas);
        if (!all)
        {
            throw new InvalidOperationException(numbers + " (want the ground opaque at every edge)");
        }

        return numbers;
    }

    /// <summary>
    /// The Uses gate's thumbs-up (U+E8E1, Like) draws from an installed icon
    /// font, not as a missing-glyph box: the ink spans at least 15 of the
    /// 22-point em both ways (18 by 22 drawn). Red before the painter named one family: Win2D
    /// read "Segoe Fluent Icons, Segoe MDL2 Assets" as one unknown name and
    /// drew an 11 by 16 box (box tc-win-az3, 2026-11-19).
    /// </summary>
    [RenderTest]
    public static string FirstRunThumbDrawsFromTheIconFont(CanvasDevice device, string outDir)
    {
        const int size = 58;
        using var target = new CanvasRenderTarget(device, size, size, 96f);
        using (CanvasDrawingSession ds = target.CreateDrawingSession())
        using (CanvasTextFormat glyph = FirstRunStagePainter.IconFormat(22))
        {
            ds.Clear(Color.FromArgb(0, 0, 0, 0));
            ds.DrawText("\uE8E1", new Windows.Foundation.Rect(0, 0, size, size), Color.FromArgb(255, 255, 255, 255), glyph);
        }

        byte[] px = target.GetPixelBytes();
        int minX = size, maxX = -1, minY = size, maxY = -1;
        for (int y = 0; y < size; y++)
        {
            for (int x = 0; x < size; x++)
            {
                if (px[(y * size + x) * 4 + 3] >= 128)
                {
                    minX = Math.Min(minX, x);
                    maxX = Math.Max(maxX, x);
                    minY = Math.Min(minY, y);
                    maxY = Math.Max(maxY, y);
                }
            }
        }

        int w = maxX - minX + 1, h = maxY - minY + 1;
        string numbers = $"family=\"{FirstRunStagePainter.IconFamily}\" ink={w}x{h}";
        if (maxX < 0 || w < 15 || h < 15)
        {
            throw new InvalidOperationException(numbers + " (want the thumb at least 15 by 15; a missing-glyph box is 11 by 16)");
        }

        return numbers;
    }
}
