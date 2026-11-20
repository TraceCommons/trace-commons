using System;
using System.Collections.Generic;
using System.Linq;
using TraceCommons.Interop.Generated;

namespace TraceCommons.Interop.Shell;

/// <summary>Which ground a hero card takes (<c>GlassTokens.Gradient.hero*</c>).</summary>
public enum HeroGround
{
    Campaign,
    Missions,
    Insights,
}

/// <summary>The parts of a hero card that take a colour of their own.</summary>
public enum HeroPart
{
    Ground,
    Ink,
    PillFill,
    PillInk,
}

/// <summary>
/// A colour a hero part takes: the Mac's literal (0xAARRGGBB), or, when
/// <see cref="ThemeKey"/> is set, the app's theme brush of that name.
/// </summary>
public readonly record struct HeroInk(uint Argb, string? ThemeKey);

/// <summary>
/// The rules the pixel grid moves by (<c>GlassPixelField</c>,
/// macos/Sources/TCDesign/Components/PixelField.swift). The hero glyph takes
/// its dither, ripple and colour drift from here. The field itself, as a
/// card's background, is drawn on the Mac only by the Home walkthrough card,
/// which Windows leaves out (owner decision W11-D2 (a)).
/// </summary>
public static class PixelField
{
    /// <summary>PixelField.swift:51: a pixel grid reads as stepped anyway, so it runs at 10 fps.</summary>
    public const double FramesPerSecond = 10;

    /// <summary>PixelField.swift:69: the ordered dither's thresholds (Bayer, 4 by 4).</summary>
    public static readonly int[,] Bayer = { { 0, 8, 2, 10 }, { 12, 4, 14, 6 }, { 3, 11, 1, 9 }, { 15, 7, 13, 5 } };

    /// <summary>PixelField.swift:72-74: a cell's threshold in (0, 1).</summary>
    public static double Threshold(int column, int row) => (Bayer[row & 3, column & 3] + 0.5) / 16;

    /// <summary>PixelField.swift:78-83: from the bottom trailing corner, in units of the field's reach.</summary>
    public static double Distance(double x, double y, double width, double height, double reachX, double reachY)
    {
        if (width <= 0 || height <= 0 || reachX <= 0 || reachY <= 0)
        {
            return double.PositiveInfinity;
        }

        double u = (width - x) / (width * reachX);
        double v = (height - y) / (height * reachY);
        return Math.Sqrt(u * u + v * v);
    }

    /// <summary>PixelField.swift:87-93: a smooth falloff from the corner with a ripple travelling outward.</summary>
    public static double Level(double distance, double time)
    {
        if (!(distance < 1))
        {
            return 0;
        }

        double falloff = 1 - distance;
        double smooth = falloff * falloff * (3 - 2 * falloff);
        double ripple = 0.5 + 0.5 * Math.Sin(distance * 14 - time * 1.6);
        return smooth * (0.72 + 0.28 * ripple);
    }

    /// <summary>PixelField.swift:112-119: which of <paramref name="count"/> colours a cell takes at <paramref name="time"/>.</summary>
    public static int Shade(int column, int row, double time, int count)
    {
        if (count <= 1)
        {
            return 0;
        }

        double wave = Math.Sin(column * 0.31 + row * 0.23 + time * 0.45) + Math.Sin((column - row) * 0.17 - time * 0.3);
        double unit = Math.Min(Math.Max((wave + 2) / 4, 0), 0.999_999);
        return (int)(unit * count);
    }

    /// <summary>
    /// The clock's time the Mac draws at: seconds since its reference date
    /// (2001-01-01 UTC) modulo an hour (PixelField.swift:57, PixelGlyph.swift:62),
    /// or 0 when still.
    /// </summary>
    public static double Time(DateTimeOffset now, bool still)
    {
        if (still)
        {
            return 0;
        }

        double seconds = (now - new DateTimeOffset(2001, 1, 1, 0, 0, 0, TimeSpan.Zero)).TotalSeconds;
        return seconds % 3600;
    }
}

/// <summary>
/// A hero card's pixel glyph (<c>GlassPixelGlyph</c> and <c>GlassHeroSymbol</c>,
/// macos/Sources/TCDesign/Components/PixelGlyph.swift and HeroCard.swift):
/// a symbol as lit cells on a visible grid, thinning toward its top leading
/// corner, moving as the field does.
/// </summary>
public static class HeroGlyph
{
    /// <summary>HeroCard.swift:258-269: the main symbol's size, its bleed past the card's trailing and bottom edges, the tilt, the pitch, and the cells' strengths.</summary>
    public const double SymbolSize = 190;
    public const double BleedX = 46;
    public const double BleedY = 34;
    public const double Angle = -12;
    public const double Cell = 6;
    public const double BrightCells = 0.46;
    public const double DimCells = 0.32;

    /// <summary>
    /// The tilt as the matrix the glyph is drawn with (M11, M12, M21, M22),
    /// in XAML's convention: y down, a point (x, y) maps to
    /// (x M11 + y M21, x M12 + y M22), and a positive angle turns clockwise.
    /// <see cref="Angle"/> is negative, so the glyph's trailing side rises,
    /// as SwiftUI's <c>rotationEffect(.degrees(-12))</c> draws it.
    /// </summary>
    public static (double M11, double M12, double M21, double M22) Rotation()
    {
        double radians = Angle * Math.PI / 180;
        double cos = Math.Cos(radians), sin = Math.Sin(radians);
        return (cos, sin, -sin, cos);
    }

    /// <summary>PixelGlyph.swift:49: how far the canvas runs past the symbol.</summary>
    public const double Overscan = 0.3;

    /// <summary>PixelGlyph.swift:75: how much of a cell the source must cover to light.</summary>
    public const double MinimumCoverage = 0.5;

    /// <summary>PixelGlyph.swift:90-91: where along the diagonal the fade starts and how much it takes.</summary>
    public const double FadeStart = 0.55;
    public const double FadeDepth = 0.92;

    /// <summary>The level past which a lit cell is a bright one (PixelGlyph.swift:180).</summary>
    public const double BrightLevel = 0.28;

    /// <summary>PixelGlyph.swift:52: the canvas's side, a whole number of cells (252 points, 42 cells).</summary>
    public static double CanvasSide => Math.Ceiling(SymbolSize * (1 + Overscan) / Cell) * Cell;

    /// <summary>The canvas's cells along a side.</summary>
    public static int Columns => (int)Math.Round(CanvasSide / Cell);

    /// <summary>PixelGlyph.swift:79-83: all of the glyph toward the bottom trailing corner, thinning toward the top leading one.</summary>
    public static double Presence(double across, double down)
    {
        double far = 1 - (across + down) / 2;
        double into = Math.Min(Math.Max((far - FadeStart) / (1 - FadeStart), 0), 1);
        return 1 - FadeDepth * into * into * (3 - 2 * into);
    }

    /// <summary>PixelGlyph.swift:94-99: covered, and present past its dither threshold.</summary>
    public static bool IsLit(double coverage, int column, int row, int columns, int rows)
    {
        if (!(coverage >= MinimumCoverage) || columns <= 1 || rows <= 1)
        {
            return false;
        }

        double across = (double)column / (columns - 1);
        double down = (double)row / (rows - 1);
        return Presence(across, down) > PixelField.Threshold(column, row);
    }

    /// <summary>How much of each cell the source covers, 0 to 1, row by row from the top.</summary>
    public sealed record Mask(int Columns, IReadOnlyList<double> Coverage);

    private readonly record struct Run(int X, int Y, int W, int H);

    /// <summary>
    /// The symbols the Mac places by hand (PixelGlyph.swift:117-139), run for
    /// run: the grid's side, how many cells right of centre, then each layer
    /// lighting or cutting back its runs.
    /// </summary>
    private static readonly Dictionary<string, (int Grid, int Shift, (bool Lit, Run[] Runs)[] Layers)> Drawn = new()
    {
        ["cpu"] = (38, 2, new[]
        {
            (true, new[]
            {
                new Run(11, 0, 2, 7), new Run(18, 0, 2, 7), new Run(25, 0, 2, 7),
                new Run(11, 31, 2, 7), new Run(18, 31, 2, 7), new Run(25, 31, 2, 7),
                new Run(0, 11, 7, 2), new Run(0, 18, 7, 2), new Run(0, 25, 7, 2),
                new Run(31, 11, 7, 2), new Run(31, 18, 7, 2), new Run(31, 25, 7, 2),
                new Run(6, 6, 26, 26),
            }),
            (false, new[] { new Run(6, 6, 1, 1), new Run(31, 6, 1, 1), new Run(6, 31, 1, 1), new Run(31, 31, 1, 1), new Run(9, 9, 20, 20) }),
            (true, new[] { new Run(13, 13, 12, 12) }),
            (false, new[] { new Run(15, 15, 8, 8) }),
        }),
        ["chart.bar.xaxis"] = (32, 0, new[]
        {
            (true, new[] { new Run(-2, 12, 7, 20), new Run(8, 0, 7, 32), new Run(18, 7, 7, 25) }),
        }),
    };

    /// <summary>PixelGlyph.swift:142-159: a drawn symbol centred on a canvas of <paramref name="columns"/> cells.</summary>
    public static Mask? DrawnMask(string symbol, int columns)
    {
        if (!Drawn.TryGetValue(symbol, out var drawn))
        {
            return null;
        }

        int inset = (columns - drawn.Grid) / 2;
        var coverage = new double[columns * columns];
        foreach ((bool lit, Run[] runs) in drawn.Layers)
        {
            foreach (Run run in runs)
            {
                for (int row = run.Y; row < run.Y + run.H; row++)
                {
                    for (int column = run.X; column < run.X + run.W; column++)
                    {
                        int y = row + inset, x = column + inset + drawn.Shift;
                        if (y < 0 || y >= columns || x < 0 || x >= columns)
                        {
                            continue;
                        }

                        coverage[y * columns + x] = lit ? 1 : 0;
                    }
                }
            }
        }

        return new Mask(columns, coverage);
    }

    /// <summary>
    /// Symbols the Mac samples from Apple's SF Symbols, which Windows does
    /// not have: drawn here from this file's own geometry, on the same
    /// 42-cell canvas, to look like the symbol (owner decision W11-D1 (a),
    /// 2026-11-18; the shapes are the Linux port's, #2187). A test at a point
    /// in cell units.
    /// </summary>
    private static bool? HandDrawnInside(string symbol, double x, double y)
    {
        double dx = x - 21, dy = y - 21;
        double r = Math.Sqrt(dx * dx + dy * dy);
        return symbol switch
        {
            // Missions' "scope": a ring, and a crosshair's four arms crossing it.
            "scope" => (r >= 12.5 && r <= 16.5)
                || (Math.Abs(dy) <= 1.5 && Math.Abs(dx) >= 6.5 && Math.Abs(dx) <= 19)
                || (Math.Abs(dx) <= 1.5 && Math.Abs(dy) >= 6.5 && Math.Abs(dy) <= 19),

            // A campaign's "globe": a ring, an inner meridian ellipse, the
            // axis, the equator and two parallels.
            "globe" => r <= 16.5 && (r >= 14.3
                || (Ellipse(dx, dy, 7.6) <= 1 && Ellipse(dx, dy, 5.4) >= 1)
                || Math.Abs(dx) <= 1.1
                || Math.Abs(dy) <= 1.2
                || Math.Abs(Math.Abs(dy) - 8.5) <= 1.0),

            // A campaign's "bolt": a lightning bolt's outline.
            "bolt" => BoltInside(x, y),

            // A campaign's "sparkles": three four-pointed stars, the largest low and right.
            "sparkles" => Stars.Any(s => Math.Pow(Math.Abs(x - s.Cx) / s.Rx, 0.6) + Math.Pow(Math.Abs(y - s.Cy) / s.Ry, 0.6) <= 1),
            _ => null,
        };
    }

    private static double Ellipse(double dx, double dy, double rx) => (dx / rx) * (dx / rx) + (dy / 16) * (dy / 16);

    private static readonly (double X, double Y)[] Bolt = { (27, 3), (10, 24.5), (21, 24.5), (15.5, 39), (33, 16.5), (22, 16.5) };

    private static readonly (double Cx, double Cy, double Rx, double Ry)[] Stars = { (24, 27, 12, 13), (12, 13, 6, 6.5), (21.5, 5, 3.5, 4) };

    private static bool BoltInside(double x, double y)
    {
        for (int i = 0; i < Bolt.Length; i++)
        {
            (double ax, double ay) = Bolt[i];
            (double bx, double by) = Bolt[(i + 1) % Bolt.Length];
            double vx = bx - ax, vy = by - ay;
            double length = vx * vx + vy * vy;
            double t = length > 0 ? Math.Clamp(((x - ax) * vx + (y - ay) * vy) / length, 0, 1) : 0;
            double cx = ax + t * vx, cy = ay + t * vy;
            if (Math.Sqrt((x - cx) * (x - cx) + (y - cy) * (y - cy)) <= 1.35)
            {
                return true;
            }
        }

        return false;
    }

    /// <summary>Samples per cell along each side, as the Mac samples a symbol (PixelGlyph.swift:191).</summary>
    private const int Samples = 4;

    /// <summary>A hand-drawn symbol sampled onto a canvas <paramref name="columns"/> cells square, scaled from the 42-cell design.</summary>
    public static Mask? HandDrawnMask(string symbol, int columns)
    {
        if (HandDrawnInside(symbol, 0, 0) is null)
        {
            return null;
        }

        double scale = 42.0 / columns;
        var coverage = new double[columns * columns];
        for (int row = 0; row < columns; row++)
        {
            for (int column = 0; column < columns; column++)
            {
                int inside = 0;
                for (int sy = 0; sy < Samples; sy++)
                {
                    for (int sx = 0; sx < Samples; sx++)
                    {
                        double x = (column + (sx + 0.5) / Samples) * scale;
                        double y = (row + (sy + 0.5) / Samples) * scale;
                        if (HandDrawnInside(symbol, x, y) == true)
                        {
                            inside++;
                        }
                    }
                }

                coverage[row * columns + column] = (double)inside / (Samples * Samples);
            }
        }

        return new Mask(columns, coverage);
    }

    /// <summary>The hero canvas's mask for <paramref name="symbol"/>; null for a symbol neither the Mac nor this file draws by hand.</summary>
    public static Mask? HeroMask(string symbol) => DrawnMask(symbol, Columns) ?? HandDrawnMask(symbol, Columns);

    /// <summary>One lit cell: where, which colour, and whether it is a bright one.</summary>
    public readonly record struct Lit(int Column, int Row, int Shade, bool Bright);

    /// <summary>
    /// PixelGlyph.swift:162-185: the cells lit at <paramref name="time"/>, each
    /// with its colour (the field's slow drift) and brightness (the field's
    /// ripple from the glyph's bottom trailing corner).
    /// </summary>
    public static List<Lit> LitCells(Mask mask, double time, int colours)
    {
        int n = mask.Columns;
        double extent = n * Cell;
        var lit = new List<Lit>();
        for (int row = 0; row < n; row++)
        {
            for (int column = 0; column < n; column++)
            {
                if (!IsLit(mask.Coverage[row * n + column], column, row, n, n))
                {
                    continue;
                }

                double x = column * Cell, y = row * Cell;
                double distance = PixelField.Distance(x + Cell / 2, y + Cell / 2, extent, extent, 1, 1);
                lit.Add(new Lit(column, row, PixelField.Shade(column, row, time, colours), PixelField.Level(distance, time) > BrightLevel));
            }
        }

        return lit;
    }

    /// <summary>The glyph's three blues for one appearance (HeroCard.swift:265-267).</summary>
    public static IReadOnlyList<uint> Tints(bool dark) => dark ? HomeTrayInks.Dark.HeroTints : HomeTrayInks.Light.HeroTints;

    /// <summary>
    /// The glyph at <paramref name="time"/>, unturned, on a canvas of
    /// <paramref name="pixelsPerPoint"/> pixels a point: one-point gaps
    /// between cells (PixelGlyph.swift:172), bright cells at
    /// <see cref="BrightCells"/> and the others at <see cref="DimCells"/>.
    /// The tilt is applied by whoever shows it, as the Mac tilts the whole
    /// grid (HeroCard.swift:249-251).
    /// </summary>
    public static PixelCanvas Render(Mask mask, double time, bool dark, double pixelsPerPoint)
    {
        int side = Math.Max(1, (int)Math.Round(mask.Columns * Cell * pixelsPerPoint));
        var canvas = new PixelCanvas(side, side);
        IReadOnlyList<uint> tints = Tints(dark);
        double cell = Math.Max(Cell - 1, 1);
        foreach (Lit lit in LitCells(mask, time, tints.Count))
        {
            Rgba ink = Rgba.FromArgb(tints[lit.Shade]).WithOpacity(lit.Bright ? BrightCells : DimCells);
            canvas.Fill(
                CanvasShape.Rect(lit.Column * Cell * pixelsPerPoint, lit.Row * Cell * pixelsPerPoint, cell * pixelsPerPoint, cell * pixelsPerPoint),
                ink);
        }

        return canvas;
    }

    /// <summary>
    /// Where the glyph's centre sits on a card of <paramref name="width"/> by
    /// <paramref name="height"/>: the middle of the graphic's 210-point column
    /// at the trailing edge, pushed past the edges by the bleed
    /// (HeroCard.swift:150-157, :253-254).
    /// </summary>
    public static (double X, double Y) Centre(double width, double height) =>
        (width - HeroCard.GraphicWidth / 2 + BleedX, height / 2 + BleedY);
}

/// <summary>The hero card's measures (<c>GlassHeroCard</c>, macos/Sources/TCDesign/Components/HeroCard.swift).</summary>
public static class HeroCard
{
    /// <summary>HeroCard.swift:75-82.</summary>
    public const double MinHeight = 156;
    public const double GraphicReserve = 90;
    public const double GraphicWidth = 210;

    /// <summary>HeroCard.swift:146: heroInk over the ground under the pointer.</summary>
    public const double HoverWash = 0.06;

    /// <summary>HeroCard.swift:186, :216: the line under the title, and the X at rest and under the pointer.</summary>
    public const double SubtitleInk = 0.88;
    public const double ContentInk = 0.92;
    public const double CloseInk = 0.75;
    public const double CloseInkHover = 1.0;

    /// <summary>HeroCard.swift:203-207, :204: the X's glyph and its inset.</summary>
    public const double CloseGlyph = 12;

    /// <summary>HeroCard.swift:198-200: the pill's padding.</summary>
    public const double PillPaddingX = 14;
    public const double PillPaddingY = 7;

    /// <summary>The ground's colour: each is one solid colour, the same in both appearances.</summary>
    public static uint Ground(HeroGround ground) => ground switch
    {
        HeroGround.Campaign => HomeTrayInks.HeroCampaignCard,
        HeroGround.Missions => HomeTrayInks.HeroMissions,
        _ => HomeTrayInks.HeroInsights,
    };

    /// <summary>The theme brush a card's ground takes (the Home tiles' own).</summary>
    public const string HighContrastGroundKey = "TcSurfaceCardBrush";

    /// <summary>The theme brush a card's words take (the Home tiles' own).</summary>
    public const string HighContrastInkKey = "TcInkPrimaryBrush";

    /// <summary>
    /// A part's colour: the Mac's literal (GlassTokens.swift:405-409,
    /// HeroCard.swift:186-200), the same in both appearances. Under a high
    /// contrast theme the ground and the pill take the card ground and the
    /// words the ink, as every other Home card does, so the system's colours
    /// reach the hero too (checklist D2).
    /// </summary>
    public static HeroInk Ink(HeroPart part, HeroGround ground, bool highContrast) => (part, highContrast) switch
    {
        (HeroPart.Ground or HeroPart.PillFill, true) => new HeroInk(0, HighContrastGroundKey),
        (_, true) => new HeroInk(0, HighContrastInkKey),
        (HeroPart.Ground, false) => new HeroInk(Ground(ground), null),
        (HeroPart.PillInk, false) => new HeroInk(HomeTrayInks.HeroButtonInk, null),
        (HeroPart.PillFill, false) => new HeroInk(HomeTrayInks.HeroButtonFill, null),
        _ => new HeroInk(HomeTrayInks.HeroInk, null),
    };

    /// <summary>
    /// Whether the glyph or bars are drawn behind the words: not under a
    /// high contrast theme, where they would be decoration in fixed blues
    /// over the system's ground.
    /// </summary>
    public static bool DrawsArt(bool highContrast) => !highContrast;

    /// <summary>The Missions hero's symbol (HomeViews.swift:291).</summary>
    public const string MissionsSymbol = "scope";

    /// <summary>The Insights hero's symbol without a readable glance (HomeInsightsVisual.swift:59).</summary>
    public const string InsightsSymbol = "chart.bar.xaxis";
}

/// <summary>
/// When Home's glyph clock wakes: on screen with animations on, at the
/// field's 10 fps; otherwise not at all. Animations coming back on reach
/// the window as WM_SETTINGCHANGE (<c>SystemSettingWatch</c>), which starts
/// the clock again, so nothing polls while they are off (checklist P1, O3).
/// </summary>
public static class GlyphClock
{
    public static readonly TimeSpan Frame = TimeSpan.FromSeconds(1 / PixelField.FramesPerSecond);

    public static TimeSpan? Interval(bool onScreen, bool hasGlyphs, bool animations) =>
        onScreen && hasGlyphs && animations ? Frame : null;
}
