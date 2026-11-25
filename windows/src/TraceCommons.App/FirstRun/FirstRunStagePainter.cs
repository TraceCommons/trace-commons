using System;
using System.Collections.Generic;
using System.Numerics;
using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Brushes;
using Microsoft.Graphics.Canvas.Effects;
using Microsoft.Graphics.Canvas.Geometry;
using Microsoft.Graphics.Canvas.Text;
using TraceCommons.Interop.FirstRun;
using TraceCommons.Interop.Shell;
using Windows.Foundation;
using Windows.UI;

namespace TraceCommons.App.FirstRun;

/// <summary>
/// Draws the first run's stage with Win2D: the scene and its breathing
/// glow, the N (marker-drawn on Welcome, with pen tip and shine) and its
/// breathing glow, the tool tiles, and each step's graphic, from a
/// <see cref="StageSnapshot"/> and the model it came from. Every number is
/// the Mac's, through <see cref="FirstRunStageLayout"/> and its siblings;
/// every colour is a generated <see cref="FirstRunTokens"/> value.
/// </summary>
/// <remarks>
/// No XAML here: the render tests compile this file and draw offscreen.
/// Words the stage draws are the core's or a tool's own name; the stage is
/// decorative and hidden from assistive technology, as on the Mac. The mark
/// geometry and the sweep's conic image are built once per device and
/// reused; a frame draws, transforms and blurs (checklist O3).
/// </remarks>
public sealed class FirstRunStagePainter : IDisposable
{
    private readonly Dictionary<string, string> _names;
    private CanvasDevice? _device;
    private CanvasGeometry? _unitMark;
    private CanvasRenderTarget? _sweep;

    /// <param name="names">Each tool's display name, by source; the tile draws the initials when it has no logo.</param>
    public FirstRunStagePainter(Dictionary<string, string> names) => _names = names;

    public static Color Argb(uint argb, double alpha = 1) => Color.FromArgb(
        (byte)Math.Round(((argb >> 24) & 0xFF) * Math.Clamp(alpha, 0, 1)), (byte)(argb >> 16), (byte)(argb >> 8), (byte)argb);

    private static Vector2 V(double x, double y) => new((float)x, (float)y);

    private static Vector2 V(StagePoint p) => V(p.X, p.Y);

    public void Dispose()
    {
        _unitMark?.Dispose();
        _sweep?.Dispose();
    }

    /// <summary>
    /// Draws one frame: the canvas is <paramref name="width"/> by
    /// <paramref name="height"/> device-independent pixels; the design's
    /// 860 by 760 stage is fitted into it (<c>FirstRunStageFit</c>).
    /// </summary>
    /// <summary>
    /// The icon font: Segoe Fluent Icons where installed (Windows 11), else
    /// Segoe MDL2 Assets. Win2D takes one family name, not XAML's fallback
    /// list: "Segoe Fluent Icons, Segoe MDL2 Assets" named no installed font
    /// and drew the Uses thumbs-up as a missing-glyph box (box tc-win-az3,
    /// 2026-11-19).
    /// </summary>
    public static string IconFamily { get; } =
        Array.IndexOf(CanvasTextFormat.GetSystemFontFamilies(), "Segoe Fluent Icons") >= 0 ? "Segoe Fluent Icons" : "Segoe MDL2 Assets";

    /// <summary>A centred icon glyph's format at <paramref name="size"/>.</summary>
    public static CanvasTextFormat IconFormat(double size) => new()
    {
        FontFamily = IconFamily,
        FontSize = (float)size,
        HorizontalAlignment = CanvasHorizontalAlignment.Center,
        VerticalAlignment = CanvasVerticalAlignment.Center,
    };

    public void Draw(CanvasDrawingSession ds, ICanvasResourceCreator rc, FirstRunStageModel model, double now, double width, double height, string? tracesCountText = null, string? tracesCaption = null, string? privateAIPill = null, IReadOnlyList<string>? usesCaptions = null, StageBackdrop backdrop = StageBackdrop.Full)
    {
        Prepare(rc);
        model.Tick(now);
        StageSnapshot s = model.Snapshot(now);
        bool reduce = s.ReduceMotion;
        double scale = FirstRunStageLayout.FitScale(width, height);
        double stageWidth = scale < 1 ? FirstRunStageLayout.CanvasWidth : width;
        double stageHeight = scale < 1 ? FirstRunStageLayout.CanvasHeight : height;
        double left = (stageWidth - FirstRunStageLayout.CanvasWidth) / 2;
        Matrix3x2 fit = scale < 1
            ? Matrix3x2.CreateScale((float)scale) * Matrix3x2.CreateTranslation((float)((width - stageWidth * scale) / 2), (float)((height - stageHeight * scale) / 2))
            : Matrix3x2.Identity;
        if (scale < 1)
        {
            // The scene at the window's size behind the scaled stage, as the
            // Mac's window draws FirstRunBackdrop under FirstRunStageFit
            // (FirstRunViews.swift:28-29): no unpainted bands at the sides.
            ds.Transform = Matrix3x2.Identity;
            Backdrop(ds, rc, width, height, now, reduce, backdrop);
        }

        ds.Transform = fit;
        Backdrop(ds, rc, stageWidth, stageHeight, now, reduce, backdrop);
        Matrix3x2 graphic = Matrix3x2.CreateTranslation((float)left, 0) * fit;
        ds.Transform = graphic;
        double top = s.GraphicTop;
        bool welcome = s.Step == FirstRunStep.Welcome;
        double welcomeElapsed = now - model.WelcomeOpenedAt;
        bool charged = s.Step == FirstRunStep.Tools && model.Watched.Count > 0;
        void DrawHub() => Hub(ds, rc, s.Hub, FirstRunStageLayout.AtRate(now, FirstRunMotion.Thirty), charged, reduce,
            drawn: welcome ? WelcomeMotion.Drawn(welcomeElapsed, reduce) : 1,
            shine: welcome ? WelcomeMotion.Shine(welcomeElapsed, reduce) : null,
            tip: welcome,
            glow: welcome ? WelcomeMotion.Glow(welcomeElapsed, reduce) : 1);
        void DrawTiles()
        {
            // Lit only on Tools (FirstRunToolFace's default elsewhere).
            foreach (StageTile tile in s.Tiles)
            {
                Tile(ds, rc, tile.Source, tile.Frame, false, 1, 0);
            }
        }

        // Each step's drawing order is the Mac's ZStack order.
        switch (s.Step)
        {
            case FirstRunStep.Tools:
                ToolsGraphic(ds, rc, model, s, now, top, reduce, DrawHub);
                break;
            case FirstRunStep.PrivateAI:
                PrivateAIGraphic(ds, rc, model, s, now, top, reduce, privateAIPill);
                DrawTiles();
                DrawHub();
                break;
            case FirstRunStep.Traces:
                TracesGraphicDraw(ds, rc, model, s, now, top, reduce, tracesCountText, tracesCaption);
                DrawTiles();
                DrawHub();
                break;
            case FirstRunStep.Uses:
                UsesGraphic(ds, rc, model, s, now, top, reduce, usesCaptions);
                DrawTiles();
                DrawHub();
                break;
            case FirstRunStep.Join:
                JoinGraphic(ds, rc, model, s, now, top, reduce);
                DrawHub();
                DrawTiles();
                break;
            default:
                DrawTiles();
                DrawHub();
                break;
        }

        ds.Transform = Matrix3x2.Identity;
    }

    private void Prepare(ICanvasResourceCreator rc)
    {
        if (ReferenceEquals(_device, rc.Device))
        {
            return;
        }

        Dispose();
        _device = rc.Device;
        _unitMark = MarkGeometry(rc, NearMarkPath.Unit);
        _sweep = SweepImage(rc);
    }

    /// <summary>
    /// A dashed stroke as the Mac's <c>StrokeStyle(dash:dashPhase:)</c>
    /// draws it: CoreGraphics dashes in points and its phase, converted to
    /// Direct2D's stroke widths, with the Mac's cap on each dash. Win2D's
    /// own default dash cap is square, which lengthens every dash by the
    /// stroke's width (measured on the test box: a 4 pt dash under phase -8
    /// started 6 pt along, not 8); the Mac's default is butt.
    /// </summary>
    public static CanvasStrokeStyle Dashes(IReadOnlyList<double> dash, double width, double phase = 0, bool round = false) => new()
    {
        CustomDashStyle = DashFlow.Direct2DDashes(dash, width),
        DashOffset = DashFlow.Direct2DOffset(phase, width),
        DashCap = round ? CanvasCapStyle.Round : CanvasCapStyle.Flat,
        StartCap = round ? CanvasCapStyle.Round : CanvasCapStyle.Flat,
        EndCap = round ? CanvasCapStyle.Round : CanvasCapStyle.Flat,
    };

    // ---- The scene ---------------------------------------------------------

    /// <summary>
    /// FirstRunBackdrop (FirstRunStage.swift:109-149), dark only. On a
    /// machine that cannot keep up (<see cref="StageBackdropBudget"/>, Windows
    /// only) the blurred breath is left out, then the two ground gradients,
    /// which leave the scene's base colour.
    /// </summary>
    private static void Backdrop(CanvasDrawingSession ds, ICanvasResourceCreator rc, double w, double h, double now, bool reduce, StageBackdrop backdrop)
    {
        var rect = new Rect(0, 0, w, h);
        if (backdrop == StageBackdrop.NoGround)
        {
            ds.FillRectangle(rect, Argb(FirstRunTokens.SceneBase));
            return;
        }

        using (var ground = new CanvasRadialGradientBrush(rc, Argb(FirstRunTokens.SceneWarm), Argb(FirstRunTokens.SceneBase)))
        {
            ground.Center = V(w / 2, 0);
            ground.RadiusX = ground.RadiusY = (float)(Math.Max(w, h) * 0.9);
            ds.FillRectangle(rect, ground);
        }

        uint glow = FirstRunTokens.FirstRunGlow;
        using (var corner = new CanvasRadialGradientBrush(rc, new[]
        {
            new CanvasGradientStop { Position = 0, Color = Argb(glow, SceneMotion.GlowStrong) },
            new CanvasGradientStop { Position = 0.42f, Color = Argb(glow, SceneMotion.GlowSoft) },
            new CanvasGradientStop { Position = 0.75f, Color = Argb(glow, 0) },
        }))
        {
            // A 2.4w by 1.9h ellipse centred on the corner (FirstRunStage.swift:121-128).
            corner.Center = V(0, 0);
            corner.RadiusX = (float)(w * 1.2);
            corner.RadiusY = (float)(h * 0.95);
            ds.FillRectangle(rect, corner);
        }

        if (backdrop != StageBackdrop.Full)
        {
            return;
        }

        // The breath, at the backdrop timeline's 30 a second (:131-142).
        double phase = SceneMotion.BackdropPhase(FirstRunStageLayout.AtRate(now, FirstRunMotion.Thirty), reduce);
        double k = SceneMotion.BreathScale(phase);
        using var list = new CanvasCommandList(rc);
        using (CanvasDrawingSession inner = list.CreateDrawingSession())
        using (var breath = new CanvasRadialGradientBrush(rc, Argb(glow, SceneMotion.GlowStrong), Argb(glow, 0)))
        {
            Vector2 c = V(SceneMotion.BreathOffset + SceneMotion.BreathWidth * k / 2, SceneMotion.BreathOffset + SceneMotion.BreathHeight * k / 2);
            // The gradient reaches 300 in the unscaled 640 by 580 ellipse.
            breath.Center = c;
            breath.RadiusX = breath.RadiusY = (float)(SceneMotion.BreathRadius * k);
            inner.FillEllipse(c, (float)(SceneMotion.BreathWidth * k / 2), (float)(SceneMotion.BreathHeight * k / 2), breath);
        }

        using var blur = new GaussianBlurEffect { Source = list, BlurAmount = (float)SceneMotion.BreathBlur, BorderMode = EffectBorderMode.Soft };
        using (ds.CreateLayer((float)SceneMotion.BreathOpacity(phase)))
        {
            ds.DrawImage(blur);
        }
    }

    // ---- The N -------------------------------------------------------------

    /// <summary>FirstRunHub (FirstRunStage.swift:233-286) and GlassBrandMarkDrawing (GlassBrandMarkDrawing.swift:76-140).</summary>
    private void Hub(CanvasDrawingSession ds, ICanvasResourceCreator rc, StageFrame frame, double breathTime, bool charged, bool reduce, double drawn, double? shine, bool tip, double glow)
    {
        double side = frame.Side;
        double disc = side * SceneMotion.DiscPerSide;
        var (breathScale, breathOpacity) = SceneMotion.Breathing(breathTime, charged, reduce);
        using (var glowBrush = new CanvasRadialGradientBrush(rc, Argb(FirstRunTokens.FirstRunGlow, SceneMotion.DiscAlpha), Argb(FirstRunTokens.FirstRunGlow, 0)))
        {
            glowBrush.Center = V(frame.Center);
            glowBrush.RadiusX = glowBrush.RadiusY = (float)(disc * SceneMotion.DiscReach * breathScale);
            using (ds.CreateLayer((float)Math.Clamp(breathOpacity * glow, 0, 1)))
            {
                ds.FillCircle(V(frame.Center), (float)(disc / 2 * breathScale), glowBrush);
            }
        }

        if (drawn <= 0 || _unitMark is null)
        {
            return;
        }

        // The mark fills its frame's width (GlassBrandMarkDrawing.swift:90-97).
        LayoutRect box = NearMarkPath.ViewBox;
        double markSide = Math.Min(side, side * box.Width / box.Height);
        double markScale = markSide / box.Width;
        Vector2 origin = V(frame.Center.X - markSide / 2, frame.Center.Y - box.Height * markScale / 2);
        Matrix3x2 saved = ds.Transform;
        Matrix3x2 unit = Matrix3x2.CreateScale((float)markSide) * Matrix3x2.CreateTranslation(origin);
        Vector2 Place(StagePoint p) => V((p.X - box.X) * markScale, (p.Y - box.Y) * markScale) + origin;

        CanvasGeometry mark = _unitMark.Transform(unit);
        if (drawn < 1)
        {
            using var builder = new CanvasPathBuilder(rc);
            IReadOnlyList<StagePoint> line = BrandMarkDrawing.MarkerTrim(drawn);
            builder.BeginFigure(Place(line[0]));
            for (int i = 1; i < line.Count; i++)
            {
                builder.AddLine(Place(line[i]));
            }

            builder.EndFigure(CanvasFigureLoop.Open);
            using CanvasGeometry stroke = CanvasGeometry.CreatePath(builder);
            using var style = new CanvasStrokeStyle { StartCap = CanvasCapStyle.Round, EndCap = CanvasCapStyle.Round, LineJoin = CanvasLineJoin.Round };
            using CanvasGeometry inked = stroke.Stroke((float)(BrandMarkDrawing.MarkerWidth * markScale), style);
            using (ds.CreateLayer(1, inked))
            {
                ds.FillGeometry(mark, Argb(FirstRunTokens.FirstRunGlow));
            }
        }
        else
        {
            ds.FillGeometry(mark, Argb(FirstRunTokens.FirstRunGlow));
        }

        if (shine is double sp)
        {
            var band = BrandMarkDrawing.ShineBand(BrandMarkDrawing.ShineX(sp));
            using var builder = new CanvasPathBuilder(rc);
            builder.BeginFigure(Place(band.Corners[0]));
            for (int i = 1; i < 4; i++)
            {
                builder.AddLine(Place(band.Corners[i]));
            }

            builder.EndFigure(CanvasFigureLoop.Closed);
            using CanvasGeometry strip = CanvasGeometry.CreatePath(builder);
            var stops = new CanvasGradientStop[BrandMarkDrawing.ShineStops.Count];
            for (int i = 0; i < stops.Length; i++)
            {
                stops[i] = new CanvasGradientStop { Position = (float)BrandMarkDrawing.ShineStops[i].Location, Color = Argb(FirstRunTokens.HeroInk, BrandMarkDrawing.ShineStops[i].Alpha) };
            }

            using var light = new CanvasLinearGradientBrush(rc, stops) { StartPoint = Place(band.From), EndPoint = Place(band.To) };
            using (ds.CreateLayer(1, mark))
            {
                ds.FillGeometry(strip, light);
            }
        }

        if (tip && BrandMarkDrawing.PenTip(drawn) is { } pen)
        {
            var (center, radius, opacity) = pen;
            float r = (float)(radius * markScale);
            using (ds.CreateLayer((float)opacity))
            {
                Glow(ds, rc, inner => inner.FillCircle(Place(center), r, Argb(FirstRunTokens.FirstRunGlow)), r * BrandMarkDrawing.PenGlow);
                ds.FillCircle(Place(center), r, Argb(FirstRunTokens.HeroInk));
            }
        }

        mark.Dispose();
        ds.Transform = saved;
    }

    // ---- Tiles -------------------------------------------------------------

    /// <summary>FirstRunToolFace (FirstRunStage.swift:289-317): a raised face, its shadow, and the lit glow.</summary>
    private void Tile(CanvasDrawingSession ds, ICanvasResourceCreator rc, string source, StageFrame frame, bool lit, double opacity, double bob)
    {
        double side = frame.Side;
        if (side <= 0.5 || opacity <= 0)
        {
            return;
        }

        var rect = new Rect(frame.Center.X - side / 2, frame.Center.Y - side / 2 + bob, side, side);
        float corner = (float)(side * 14 / 52);
        using (ds.CreateLayer((float)opacity))
        {
            if (lit)
            {
                Glow(ds, rc, inner => inner.FillRoundedRectangle(rect, corner, corner, Argb(FirstRunTokens.FirstRunGlow, 0.35)), side * 11 / 52);
            }

            Rect shadow = rect;
            shadow.Y += side * 8 / 52;
            Glow(ds, rc, inner => inner.FillRoundedRectangle(shadow, corner, corner, Argb(FirstRunTokens.FirstRunTileShadow)), side * 10 / 52);
            ds.FillRoundedRectangle(rect, corner, corner, Argb(FirstRunTokens.FirstRunTile));
            string name = _names.TryGetValue(source, out string? n) ? n : source;
            using var format = new CanvasTextFormat
            {
                FontSize = (float)Math.Max(6, side * 16 / 52),
                FontWeight = Microsoft.UI.Text.FontWeights.Bold,
                HorizontalAlignment = CanvasHorizontalAlignment.Center,
                VerticalAlignment = CanvasVerticalAlignment.Center,
            };
            ds.DrawText(Initials(name), rect, Argb(FirstRunTokens.TextPrimary), format);
        }
    }

    /// <summary>A tool's initials (<c>GlassTool.initials</c>): the first letter of up to two words.</summary>
    public static string Initials(string name)
    {
        string[] words = name.Split(' ', StringSplitOptions.RemoveEmptyEntries);
        string letters = "";
        foreach (string w in words)
        {
            if (letters.Length < 2)
            {
                letters += char.ToUpperInvariant(w[0]);
            }
        }

        return letters;
    }

    /// <summary>A blurred copy of what <paramref name="draw"/> draws: SwiftUI's shadow and blur, as a Gaussian at the radius.</summary>
    private static void Glow(CanvasDrawingSession ds, ICanvasResourceCreator rc, Action<CanvasDrawingSession> draw, double radius)
    {
        using var list = new CanvasCommandList(rc);
        using (CanvasDrawingSession inner = list.CreateDrawingSession())
        {
            draw(inner);
        }

        using var blur = new GaussianBlurEffect { Source = list, BlurAmount = (float)(radius / 2), BorderMode = EffectBorderMode.Soft };
        ds.DrawImage(blur);
    }

    // ---- Tools ------------------------------------------------------------

    /// <summary>ToolsRing (ToolsScreen.swift:411-523) and ToolsBeam (:526-587).</summary>
    private void ToolsGraphic(CanvasDrawingSession ds, ICanvasResourceCreator rc, FirstRunStageModel model, StageSnapshot s, double now, double top, bool reduce, Action drawHub)
    {
        double? elapsed = model.ScanElapsed(now);
        bool settled = elapsed is double e0 ? FirstRunStageLayout.ScanDone(e0) : true;
        // The ring's own timeline: display rate while it scans, 30 after.
        double time = FirstRunStageLayout.AtRate(now, FirstRunMotion.LayerInterval(StageLayer.Tools, elapsed is not null));
        var center = new StagePoint(FirstRunStageLayout.CanvasWidth / 2, ToolsRing.CenterY + top);
        double scanEnd = model.ScanStartedAt is double st ? st + FirstRunStageLayout.ScanLead + FirstRunStageLayout.ScanTurn + 0.25 : double.NegativeInfinity;
        double ringScale = model.ToolsScanned || settled ? FirstRunMotion.RingScale(now - scanEnd, reduce) : 1;
        if (!model.ToolsScanned && elapsed is null)
        {
            ringScale = 1;
        }

        using (var dash = Dashes(ToolsRing.RingDash, ToolsRing.RingLine))
        using (ds.CreateLayer((float)ToolsRing.RingOpacity(elapsed)))
        {
            Matrix3x2 saved = ds.Transform;
            ds.Transform = Matrix3x2.CreateRotation((float)(ToolsRing.Turn(time, reduce) * Math.PI / 180), V(center)) * saved;
            ds.DrawCircle(V(center), (float)(ToolsRing.LooseRadius * ringScale), Argb(FirstRunTokens.FirstRunRule), (float)ToolsRing.RingLine, dash);
            ds.Transform = saved;
        }

        if (elapsed is double e && FirstRunStageLayout.SweepRotation(e) is double rotation && _sweep is not null)
        {
            Matrix3x2 saved = ds.Transform;
            ds.Transform = Matrix3x2.CreateRotation((float)(rotation * Math.PI / 180), V(center)) * saved;
            using var blur = new GaussianBlurEffect { Source = _sweep, BlurAmount = 2, BorderMode = EffectBorderMode.Soft };
            using (ds.CreateLayer((float)FirstRunStageLayout.SweepFade(e)))
            {
                ds.DrawImage(blur, (float)(center.X - 150), (float)(center.Y - 150));
            }

            ds.Transform = saved;
        }

        var exists = new bool[model.Rows.Count];
        for (int i = 0; i < exists.Length; i++)
        {
            exists[i] = model.Rows[i].Exists;
        }

        for (int i = 0; i < model.Rows.Count; i++)
        {
            string source = model.Rows[i].Source;
            if (!model.Rows[i].Exists || !settled)
            {
                continue;
            }

            double angle = ToolsRing.Placement(i, exists, null).Angle;
            double beam = model.BeamOpacity(source, now);
            if (beam > 0)
            {
                Beam(ds, rc, FirstRunStageLayout.Point(center, ToolsRing.BeamFrom, angle), FirstRunStageLayout.Point(center, ToolsRing.BeamTo, angle), time, reduce, beam);
            }
        }

        drawHub();
        foreach (StageTile tile in s.Tiles)
        {
            int index = IndexOf(model.Rows, tile.Source);
            RingPlacement place = ToolsRing.Placement(index, exists, elapsed);
            double bob = reduce ? 0 : ToolsRing.Bob(tile.Source, time);
            var scaled = new StageFrame(tile.Frame.Center, tile.Frame.Side * place.Scale);
            Tile(ds, rc, tile.Source, scaled, tile.Lit, place.Opacity, bob * place.Scale);
            if (ToolsRing.Flash(index, model.Rows.Count, elapsed) is double flash)
            {
                var (k, o) = ToolsRing.FlashLook(flash);
                double side = (ToolsRing.Tile + 14) * k * place.Scale;
                var r = new Rect(tile.Frame.Center.X - side / 2, tile.Frame.Center.Y + bob * place.Scale - side / 2, side, side);
                float corner = (float)(19 * k * place.Scale);
                using (ds.CreateLayer((float)(o * place.Opacity)))
                {
                    ds.DrawRoundedRectangle(r, corner, corner, Argb(FirstRunTokens.FirstRunGlow), 2);
                }
            }
        }
    }

    private static int IndexOf(IReadOnlyList<(string Source, bool Exists)> rows, string source)
    {
        for (int i = 0; i < rows.Count; i++)
        {
            if (rows[i].Source == source)
            {
                return i;
            }
        }

        return -1;
    }

    /// <summary>One watched tile's beam into the N (ToolsScreen.swift:526-587); pulses on <c>dashPhase: -flow</c>.</summary>
    private static void Beam(CanvasDrawingSession ds, ICanvasResourceCreator rc, StagePoint from, StagePoint to, double time, bool reduce, double opacity)
    {
        uint beam = FirstRunTokens.FirstRunBeam, core = FirstRunTokens.FirstRunBeamCore;
        CanvasLinearGradientBrush Body() => new(rc, new[]
        {
            new CanvasGradientStop { Position = 0, Color = Argb(beam, 0.95) },
            new CanvasGradientStop { Position = 0.55f, Color = Argb(beam, 0.4) },
            new CanvasGradientStop { Position = 1, Color = Argb(beam, 0) },
        }) { StartPoint = V(from), EndPoint = V(to) };
        double breath = ToolsRing.BeamBreath(time, reduce);
        using var round = new CanvasStrokeStyle { StartCap = CanvasCapStyle.Round, EndCap = CanvasCapStyle.Round };
        using (ds.CreateLayer((float)opacity))
        {
            void Layer(double blur, double alpha, Action<CanvasDrawingSession> draw)
            {
                using (ds.CreateLayer((float)alpha))
                {
                    Glow(ds, rc, draw, blur * 2);
                }
            }

            Layer(7, 0.34 + 0.16 * breath, inner => { using CanvasLinearGradientBrush b = Body(); inner.DrawLine(V(from), V(to), b, (float)(24 + 6 * breath), round); });
            Layer(2.6, 0.55 + 0.17 * breath, inner => { using CanvasLinearGradientBrush b = Body(); inner.DrawLine(V(from), V(to), b, (float)(9 + 2 * breath), round); });
            Layer(0.9, 0.8, inner => { using CanvasLinearGradientBrush b = Body(); inner.DrawLine(V(from), V(to), b, 2.6f, round); });
            Layer(2.6, 0.95, inner =>
            {
                using var pulses = new CanvasLinearGradientBrush(rc, new[]
                {
                    new CanvasGradientStop { Position = 0, Color = Argb(core) },
                    new CanvasGradientStop { Position = 0.6f, Color = Argb(core, 0.45) },
                    new CanvasGradientStop { Position = 1, Color = Argb(core, 0) },
                }) { StartPoint = V(from), EndPoint = V(to) };
                using CanvasStrokeStyle dash = Dashes(ToolsRing.BeamDash, ToolsRing.BeamPulseWidth, ToolsRing.BeamDashPhase(time, reduce), round: true);
                inner.DrawLine(V(from), V(to), pulses, (float)ToolsRing.BeamPulseWidth, dash);
            });
            Layer(7, 0.4, inner => inner.FillCircle(V(from), 13, Argb(beam)));
        }
    }

    /// <summary>
    /// The sweep's conic gradient (ToolsScreen.swift:470-494), drawn once
    /// into a 300-point image: Win2D has no angular gradient, so it is laid
    /// down in one-degree wedges, then masked clear near the N.
    /// </summary>
    private static CanvasRenderTarget SweepImage(ICanvasResourceCreator rc)
    {
        var target = new CanvasRenderTarget(rc, 300, 300, 96);
        using CanvasDrawingSession ds = target.CreateDrawingSession();
        ds.Clear(Color.FromArgb(0, 0, 0, 0));
        var c = V(150, 150);
        // Clear inside 36, full from 64 (the RadialGradient mask, :487-489),
        // applied as the layer's opacity.
        using var mask = new CanvasRadialGradientBrush(rc, new[]
        {
            new CanvasGradientStop { Position = 36f / 150, Color = Color.FromArgb(0, 0, 0, 0) },
            new CanvasGradientStop { Position = 64f / 150, Color = Color.FromArgb(255, 0, 0, 0) },
        }) { Center = c, RadiusX = 150, RadiusY = 150 };
        using (ds.CreateLayer(mask))
        {
            for (int deg = 0; deg < 360; deg++)
            {
                Color color = SweepColor((deg + 0.5) / 360);
                if (color.A == 0)
                {
                    continue;
                }

                double a0 = (deg - 90) * Math.PI / 180, a1 = (deg + 1.05 - 90) * Math.PI / 180;
                using var builder = new CanvasPathBuilder(rc);
                builder.BeginFigure(c);
                builder.AddLine(V(150 + 150 * Math.Cos(a0), 150 + 150 * Math.Sin(a0)));
                builder.AddLine(V(150 + 150 * Math.Cos(a1), 150 + 150 * Math.Sin(a1)));
                builder.EndFigure(CanvasFigureLoop.Closed);
                using CanvasGeometry wedge = CanvasGeometry.CreatePath(builder);
                ds.FillGeometry(wedge, color);
            }
        }

        return target;
    }

    private static Color SweepColor(double location)
    {
        var stops = ToolsRing.SweepStops;
        for (int i = 1; i < stops.Count; i++)
        {
            if (location <= stops[i].Location)
            {
                var (l0, b0, a0) = stops[i - 1];
                var (l1, b1, a1) = stops[i];
                double t = (location - l0) / (l1 - l0);
                uint c0 = b0 ? FirstRunTokens.FirstRunBeam : FirstRunTokens.FirstRunGlow;
                uint c1 = b1 ? FirstRunTokens.FirstRunBeam : FirstRunTokens.FirstRunGlow;
                byte Mix(int shift) => (byte)Math.Round(((c0 >> shift) & 0xFF) * (1 - t) + ((c1 >> shift) & 0xFF) * t);
                return Color.FromArgb((byte)Math.Round(255 * (a0 + (a1 - a0) * t)), Mix(16), Mix(8), Mix(0));
            }
        }

        return Color.FromArgb(0, 0, 0, 0);
    }

    // ---- Private AI --------------------------------------------------------

    /// <summary>The diagram (PrivateAIScreen.swift:93-133): dashed routes into the N and on to the pill.</summary>
    private static void PrivateAIGraphic(CanvasDrawingSession ds, ICanvasResourceCreator rc, FirstRunStageModel model, StageSnapshot s, double now, double top, bool reduce, string? pill)
    {
        double time = FirstRunStageLayout.AtRate(now, FirstRunMotion.LayerInterval(StageLayer.PrivateAI, false));
        var hub = new StagePoint(PrivateAIDiagram.HubX, PrivateAIDiagram.HubY + top);
        double phase = PrivateAIDiagram.DashPhase(time, reduce);
        using CanvasStrokeStyle dash = Dashes(PrivateAIDiagram.Dash, PrivateAIDiagram.Line, phase);
        Color glow = Argb(FirstRunTokens.FirstRunGlow);
        IReadOnlyList<string> tools = model.Connectable;
        for (int i = 0; i < tools.Count; i++)
        {
            double y = PrivateAIDiagram.RowY(i, tools.Count) + top;
            using var builder = new CanvasPathBuilder(rc);
            builder.BeginFigure(V(PrivateAIDiagram.RouteStart, y));
            builder.AddCubicBezier(V(330, y), V(350, hub.Y), V(hub.X - 36, hub.Y));
            builder.EndFigure(CanvasFigureLoop.Open);
            using CanvasGeometry route = CanvasGeometry.CreatePath(builder);
            ds.DrawGeometry(route, glow, (float)PrivateAIDiagram.Line, dash);
        }

        double half = PrivateAIDiagram.PillWidth / 2;
        ds.DrawLine(V(hub.X + 36, hub.Y), V(PrivateAIDiagram.DestinationX - half, hub.Y), glow, (float)PrivateAIDiagram.Line, dash);
        if (pill is not null)
        {
            var r = new Rect(PrivateAIDiagram.DestinationX - half, hub.Y - PrivateAIDiagram.PillHeight / 2, PrivateAIDiagram.PillWidth, PrivateAIDiagram.PillHeight);
            ds.FillRoundedRectangle(r, (float)GlassTokens.Radius.Tile, (float)GlassTokens.Radius.Tile, Argb(FirstRunTokens.TintAccent));
            using var format = new CanvasTextFormat
            {
                FontSize = 12,
                FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
                HorizontalAlignment = CanvasHorizontalAlignment.Center,
                VerticalAlignment = CanvasVerticalAlignment.Center,
            };
            ds.DrawText(pill, r, Argb(FirstRunTokens.ToggleOn), format);
        }
    }

    // ---- Traces ------------------------------------------------------------

    /// <summary>The Traces graphic (TracesScreen.swift:158-258).</summary>
    private static void TracesGraphicDraw(CanvasDrawingSession ds, ICanvasResourceCreator rc, FirstRunStageModel model, StageSnapshot s, double now, double top, bool reduce, string? countText, string? caption)
    {
        double time = FirstRunStageLayout.AtRate(now, FirstRunMotion.LayerInterval(StageLayer.Traces, false));
        double elapsed = model.TracesCounted ? 1e9 : time - model.StepOpenedAt;
        double counted = TracesGraphic.Counted(elapsed, reduce);
        var center = new StagePoint(TracesGraphic.CenterX, TracesGraphic.CenterY + top);
        IReadOnlyList<string> rows = model.Watched;
        using var dash = Dashes(new[] { 2.0, 5.0 }, 1.5);
        for (int i = 0; i < rows.Count; i++)
        {
            double y = TracesGraphic.RowY(i, rows.Count, center.Y);
            var start = new StagePoint(TracesGraphic.LabelX + TracesGraphic.LabelWidth + 12, y);
            var curve = TracesGraphic.Stream(start, new StagePoint(center.X - 114, center.Y));
            using var builder = new CanvasPathBuilder(rc);
            builder.BeginFigure(V(curve.Item1));
            builder.AddCubicBezier(V(curve.Item2), V(curve.Item3), V(curve.Item4));
            builder.EndFigure(CanvasFigureLoop.Open);
            using CanvasGeometry stream = CanvasGeometry.CreatePath(builder);
            ds.DrawGeometry(stream, Argb(FirstRunTokens.FirstRunRule), 1.5f, dash);
            if (TracesGraphic.Flowing(elapsed, reduce))
            {
                for (int orb = 0; orb < 3; orb++)
                {
                    if (TracesGraphic.Orb(orb, i, elapsed) is { } placed)
                    {
                        var (along, opacity) = placed;
                        using (ds.CreateLayer((float)opacity))
                        {
                            Orb(ds, rc, TracesGraphic.Bezier(curve, along), 14);
                        }
                    }
                }
            }
        }

        ds.DrawCircle(V(center), (float)(TracesGraphic.Ring / 2), Argb(FirstRunTokens.WellFill), (float)TracesGraphic.RingLine);
        if (counted > 0)
        {
            using var builder = new CanvasPathBuilder(rc);
            float r = (float)(TracesGraphic.Ring / 2);
            double sweep = Math.Min(counted, 0.99999) * 2 * Math.PI;
            builder.BeginFigure(V(center.X, center.Y - r));
            builder.AddArc(V(center), r, r, (float)(-Math.PI / 2), (float)sweep);
            builder.EndFigure(CanvasFigureLoop.Open);
            using CanvasGeometry arc = CanvasGeometry.CreatePath(builder);
            using var round = new CanvasStrokeStyle { StartCap = CanvasCapStyle.Round, EndCap = CanvasCapStyle.Round };
            Glow(ds, rc, inner => inner.DrawGeometry(arc, Argb(FirstRunTokens.FirstRunGlow, 0.35), (float)TracesGraphic.RingLine, round), 6);
            ds.DrawGeometry(arc, Argb(FirstRunTokens.FirstRunGlow), (float)TracesGraphic.RingLine, round);
        }

        if (countText is not null)
        {
            using var big = new CanvasTextFormat { FontSize = 26 * 1.5f, HorizontalAlignment = CanvasHorizontalAlignment.Center, VerticalAlignment = CanvasVerticalAlignment.Center };
            ds.DrawText(countText, new Rect(center.X - 110, center.Y + 8 - 24 - 8, 220, 48), Argb(FirstRunTokens.TextPrimary), big);
        }

        if (caption is not null)
        {
            using var small = new CanvasTextFormat { FontSize = 12, HorizontalAlignment = CanvasHorizontalAlignment.Center };
            ds.DrawText(caption, new Rect(center.X - 110, center.Y + 8 + 16, 220, 20), Argb(FirstRunTokens.TextSecondary), small);
        }
    }

    /// <summary>FirstRunOrb (FirstRunStage.swift:320-338).</summary>
    private static void Orb(CanvasDrawingSession ds, ICanvasResourceCreator rc, StagePoint at, double diameter)
    {
        using var brush = new CanvasRadialGradientBrush(rc, new[]
        {
            new CanvasGradientStop { Position = 0, Color = Argb(FirstRunTokens.HeroInk) },
            new CanvasGradientStop { Position = 0.22f, Color = Argb(FirstRunTokens.FirstRunBeamCore) },
            new CanvasGradientStop { Position = 0.46f, Color = Argb(FirstRunTokens.FirstRunBeam, 0.65) },
            new CanvasGradientStop { Position = 0.72f, Color = Argb(FirstRunTokens.FirstRunGlow, 0) },
        }) { Center = V(at), RadiusX = (float)(diameter / 2), RadiusY = (float)(diameter / 2) };
        ds.FillCircle(V(at), (float)(diameter / 2), brush);
    }

    // ---- Uses -------------------------------------------------------------

    /// <summary>The Uses flow (UsesScreen.swift:422-600).</summary>
    private static void UsesGraphic(CanvasDrawingSession ds, ICanvasResourceCreator rc, FirstRunStageModel model, StageSnapshot s, double now, double top, bool reduce, IReadOnlyList<string>? captions)
    {
        double time = FirstRunStageLayout.AtRate(now, FirstRunMotion.LayerInterval(StageLayer.Uses, false));
        double elapsed = time - model.SharingSince;
        SharingPath path = model.Sharing;
        double? t = reduce ? null : UsesFlow.Loop(path, elapsed);
        double y = UsesFlow.LineY + top;
        using (var dash = Dashes(new[] { 4.0, 6.0 }, 2))
        {
            ds.DrawLine(V(UsesFlow.LineStart, y), V(UsesFlow.LineEnd, y), Argb(FirstRunTokens.FirstRunRule), 2, dash);
        }

        // This computer's box.
        var mac = new Rect(UsesFlow.MacX - 56, y - 39, 112, 78);
        ds.FillRoundedRectangle(mac, 16, 16, Argb(FirstRunTokens.FirstRunCard));
        ds.FillRoundedRectangle(new Rect(UsesFlow.MacX - 22, mac.Bottom - 10 - 3, 44, 3), 1.5f, 1.5f, Argb(FirstRunTokens.FirstRunRule));

        // The gate.
        UsesGate look = t is double lt ? UsesFlow.GateLook(path, lt) : UsesGate.Rest;
        var gate = new StagePoint(UsesFlow.GateX, y);
        Glow(ds, rc, inner => inner.FillCircle(V(gate.X, gate.Y + 8), 29, Argb(FirstRunTokens.FirstRunTileShadow)), 10);
        ds.FillCircle(V(gate), 29, Argb(FirstRunTokens.FirstRunCardSolid));
        if (look.Fill > 0)
        {
            using var approve = new CanvasRadialGradientBrush(rc, Argb(FirstRunTokens.FirstRunApprove), Argb(FirstRunTokens.FirstRunApproveDeep))
            {
                Center = V(gate.X, gate.Y - 29 + 58 * 0.38),
                RadiusX = 30,
                RadiusY = 30,
            };
            using (ds.CreateLayer((float)look.Fill))
            {
                ds.FillCircle(V(gate), 29, approve);
            }
        }

        // The thumbs-up: Segoe Fluent's Like glyphs stand for the SF Symbols
        // (W5-D2 table, unverified codepoints), scaled and turned about (0.5, 0.6).
        Matrix3x2 saved = ds.Transform;
        var anchor = V(gate.X, gate.Y - 29 + 58 * 0.6);
        ds.Transform = Matrix3x2.CreateScale((float)look.Scale, anchor) * Matrix3x2.CreateRotation((float)(look.Turn * Math.PI / 180), anchor) * saved;
        using (CanvasTextFormat glyph = IconFormat(22))
        {
            var box = new Rect(gate.X - 29, gate.Y - 29, 58, 58);
            ds.DrawText("", box, Argb(FirstRunTokens.TextSecondary, 0.6 * (1 - look.White)), glyph);
            ds.DrawText("", box, Argb(FirstRunTokens.HeroInk, look.White), glyph);
        }

        ds.Transform = saved;

        // The commons: pulse, the ring of five turning once per 40 s, the N.
        var commons = new StagePoint(UsesFlow.CommonsX, y);
        if (t is double pt)
        {
            var (ps, po) = UsesFlow.Pulse(path, pt);
            if (po > 0)
            {
                using var ring = new CanvasRadialGradientBrush(rc, new[]
                {
                    new CanvasGradientStop { Position = 0.3f, Color = Argb(FirstRunTokens.FirstRunBeam, 0) },
                    new CanvasGradientStop { Position = 0.46f, Color = Argb(FirstRunTokens.FirstRunBeam, 0.6) },
                    new CanvasGradientStop { Position = 0.64f, Color = Argb(FirstRunTokens.FirstRunBeam, 0) },
                }) { Center = V(commons), RadiusX = (float)(60 * ps), RadiusY = (float)(60 * ps) };
                using (ds.CreateLayer((float)po))
                {
                    Glow(ds, rc, inner => inner.FillCircle(V(commons), (float)(60 * ps), ring), 3 * 2);
                }
            }
        }

        double turn = UsesFlow.CommonsTurn(elapsed, reduce);
        saved = ds.Transform;
        ds.Transform = Matrix3x2.CreateRotation((float)(turn * Math.PI / 180), V(commons)) * saved;
        using (var dash = Dashes(new[] { 3.0, 4.0 }, 1))
        {
            ds.DrawCircle(V(commons), 45.5f, Argb(FirstRunTokens.FirstRunRule), 1, dash);
        }

        for (int place = 0; place < 5; place++)
        {
            double k = UsesFlow.Satellite(path, elapsed, place, reduce);
            if (k <= 0)
            {
                continue;
            }

            StagePoint at = FirstRunStageLayout.Point(commons, 46, FirstRunStageLayout.RingAngle(place, 5));
            using (ds.CreateLayer((float)Math.Clamp(k, 0, 1)))
            {
                Orb(ds, rc, at, 14 * k);
            }
        }

        ds.Transform = saved;

        // The orb on the line.
        if (t is double ot)
        {
            var (along, opacity) = UsesFlow.Orb(path, ot);
            using (ds.CreateLayer((float)opacity))
            {
                Orb(ds, rc, new StagePoint(UsesFlow.LineStart + (UsesFlow.LineEnd - UsesFlow.LineStart) * along, y), 18);
            }
        }

        if (captions is { Count: 3 })
        {
            using var format = new CanvasTextFormat { FontSize = 12, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold, HorizontalAlignment = CanvasHorizontalAlignment.Center, WordWrapping = CanvasWordWrapping.Wrap };
            double[] xs = { UsesFlow.MacX, UsesFlow.GateX, UsesFlow.CommonsX };
            for (int i = 0; i < 3; i++)
            {
                ds.DrawText(captions[i], new Rect(xs[i] - UsesFlow.CaptionWidth / 2, UsesFlow.CaptionY + top - 8, UsesFlow.CaptionWidth, 40), Argb(FirstRunTokens.TextSecondary), format);
            }
        }
    }

    // ---- Join -------------------------------------------------------------

    /// <summary>Join's orbit (JoinScreen.swift:519-560).</summary>
    private static void JoinGraphic(CanvasDrawingSession ds, ICanvasResourceCreator rc, FirstRunStageModel model, StageSnapshot s, double now, double top, bool reduce)
    {
        double time = FirstRunStageLayout.AtRate(now, FirstRunMotion.LayerInterval(StageLayer.Join, false));
        var c = new StagePoint(FirstRunStageLayout.CanvasWidth / 2, JoinOrbit.Box / 2 + top);
        Matrix3x2 saved = ds.Transform;
        ds.Transform = Matrix3x2.CreateRotation((float)(JoinOrbit.Turn(time, reduce) * Math.PI / 180), V(c)) * saved;
        using (var dash = Dashes(JoinOrbit.RingDash, 1))
        {
            ds.DrawCircle(V(c), (float)(JoinOrbit.Ring / 2 - 0.5), Argb(FirstRunTokens.FirstRunRule), 1, dash);
        }

        ds.Transform = saved;
        if (model.SignedInAt is double at && JoinOrbit.Pulse(time - at, reduce) is { } pulse)
        {
            var (scale, opacity) = pulse;
            using var ring = new CanvasRadialGradientBrush(rc, new[]
            {
                new CanvasGradientStop { Position = 0.26f, Color = Argb(FirstRunTokens.FirstRunBeam, 0) },
                new CanvasGradientStop { Position = 0.44f, Color = Argb(FirstRunTokens.FirstRunBeam, 0.6) },
                new CanvasGradientStop { Position = 0.64f, Color = Argb(FirstRunTokens.FirstRunBeam, 0) },
            }) { Center = V(c), RadiusX = (float)(46 * scale), RadiusY = (float)(46 * scale) };
            using (ds.CreateLayer((float)opacity))
            {
                Glow(ds, rc, inner => inner.FillCircle(V(c), (float)(46 * scale), ring), 8);
            }
        }
    }

    /// <summary>The mark's unit path as a Win2D geometry.</summary>
    public static CanvasGeometry MarkGeometry(ICanvasResourceCreator rc, IReadOnlyList<PathStep> unit)
    {
        using var builder = new CanvasPathBuilder(rc);
        bool open = false;
        foreach (PathStep s in unit)
        {
            switch (s.Kind)
            {
                case PathStepKind.Move:
                    if (open)
                    {
                        builder.EndFigure(CanvasFigureLoop.Open);
                    }

                    builder.BeginFigure(V(s.X, s.Y));
                    open = true;
                    break;
                case PathStepKind.Line:
                    builder.AddLine(V(s.X, s.Y));
                    break;
                case PathStepKind.Cubic:
                    builder.AddCubicBezier(V(s.X1, s.Y1), V(s.X2, s.Y2), V(s.X, s.Y));
                    break;
                case PathStepKind.Close:
                    builder.EndFigure(CanvasFigureLoop.Closed);
                    open = false;
                    break;
            }
        }

        if (open)
        {
            builder.EndFigure(CanvasFigureLoop.Open);
        }

        return CanvasGeometry.CreatePath(builder);
    }
}
