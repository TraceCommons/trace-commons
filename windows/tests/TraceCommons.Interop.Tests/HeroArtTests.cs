using System;
using System.Linq;
using System.Text;
using TraceCommons.Interop.Generated;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Home's hero art pinned to the Mac: the pixel field's rules, the glyph's
/// drawn masks cell for cell, the card's measures and the grounds. Expected
/// values were printed by the throwaway Swift probe (see
/// <see cref="CreditMotionTests"/>); the masks are the probe's own drawing
/// of <c>GlassPixelGlyph.drawnMask</c> through <c>isLit</c>, '#' lit, '+'
/// covered but dithered out, '.' empty.
/// </summary>
public class HeroArtTests
{
    private const double Tight = 1e-6;

    /// <summary>PixelField.swift:69-74: the Bayer thresholds, row by row.</summary>
    [Fact]
    public void TheDitherIsTheMacsBayerMatrix()
    {
        double[] expected =
        {
            0.03125, 0.53125, 0.15625, 0.65625,
            0.78125, 0.28125, 0.90625, 0.40625,
            0.21875, 0.71875, 0.09375, 0.59375,
            0.96875, 0.46875, 0.84375, 0.34375,
        };
        for (int row = 0; row < 4; row++)
        {
            for (int column = 0; column < 4; column++)
            {
                Assert.Equal(expected[row * 4 + column], PixelField.Threshold(column, row), Tight);
            }
        }
    }

    /// <summary>PixelField.swift:78-119, the probe's values: the ripple's sign runs outward with time.</summary>
    [Fact]
    public void TheFieldsRulesAreTheMacs()
    {
        Assert.Equal(0.951917681, PixelField.Distance(100, 50, 300, 156, 1, 1), Tight);
        Assert.Equal(0.766599855, PixelField.Level(0.3, 2), Tight);
        Assert.Equal(0.024211802, PixelField.Level(0.9, 0), Tight);
        Assert.Equal(0.0, PixelField.Level(1, 0), Tight);
        Assert.Equal(0, PixelField.Shade(5, 7, 3, 3));
        Assert.Equal(1, PixelField.Shade(20, 1, 0, 3));
        Assert.Equal(0, PixelField.Shade(0, 30, 10, 3));
        Assert.Equal(10, PixelField.FramesPerSecond);
    }

    /// <summary>PixelGlyph.swift:79-91 and HeroCard.swift:258-269.</summary>
    [Fact]
    public void TheGlyphsFadeAndMeasuresAreTheMacs()
    {
        Assert.Equal(0.08, HeroGlyph.Presence(0, 0), Tight);
        Assert.Equal(1.0, HeroGlyph.Presence(1, 1), Tight);
        Assert.Equal(0.883895748, HeroGlyph.Presence(0.3, 0.4), Tight);
        Assert.Equal(252, HeroGlyph.CanvasSide, Tight);
        Assert.Equal(42, HeroGlyph.Columns);
        Assert.Equal((190.0, 46.0, 34.0, -12.0, 6.0, 0.46, 0.32), (HeroGlyph.SymbolSize, HeroGlyph.BleedX, HeroGlyph.BleedY, HeroGlyph.Angle, HeroGlyph.Cell, HeroGlyph.BrightCells, HeroGlyph.DimCells));
        Assert.Equal((156.0, 90.0, 210.0), (HeroCard.MinHeight, HeroCard.GraphicReserve, HeroCard.GraphicWidth));
    }

    /// <summary>HeroCard.swift:265-267 under the flat theme: dataKept, chartSeries1, firstRunBeamCore, per appearance.</summary>
    [Fact]
    public void TheGlyphsBluesAndTheGroundsAreTheMacs()
    {
        Assert.Equal(new uint[] { 0xFF3A7BD5, 0xFF0091FD, 0xFFE2F6FF }, HeroGlyph.Tints(dark: true));
        Assert.Equal(new uint[] { 0xFF2F6BC4, 0xFF2A78D6, 0xFF9FDCFF }, HeroGlyph.Tints(dark: false));

        // GlassTokens.swift:405-409: solid navy grounds since fc4ecc1c6, not the old gradients.
        Assert.Equal(0xFF081538u, HeroCard.Ground(HeroGround.Campaign));
        Assert.Equal(0xFF07152Fu, HeroCard.Ground(HeroGround.Missions));
        Assert.Equal(0xFF0A2546u, HeroCard.Ground(HeroGround.Insights));
        Assert.Equal(0xFFFFFFFFu, HomeTrayInks.HeroInk);
        Assert.Equal(0xFF16161Au, HomeTrayInks.HeroButtonInk);
    }

    [Fact]
    public void TheCpuGlyphIsTheMacsCellForCell() => AssertMask("cpu", CpuRows, covered: 496, lit: 466);

    [Fact]
    public void TheChartGlyphIsTheMacsCellForCell() => AssertMask("chart.bar.xaxis", ChartRows, covered: 539, lit: 511);

    /// <summary>
    /// Missions' "scope" is an SF Symbol the Mac samples; Windows draws it by
    /// hand (owner decision W11-D1 (a)). Its ring and arms sit where the Mac's
    /// symbol has them: the centre is empty, the ring is lit, the arms reach
    /// out past it.
    /// </summary>
    [Fact]
    public void TheScopeIsDrawnByHandAsARingAndCrosshair()
    {
        HeroGlyph.Mask mask = HeroGlyph.HeroMask("scope")!;
        double At(int column, int row) => mask.Coverage[row * mask.Columns + column];
        Assert.Equal(42, mask.Columns);
        Assert.Equal(0, At(21, 21));
        Assert.Equal(1, At(21 + 14, 21));
        Assert.Equal(1, At(21, 21 - 14));
        Assert.Equal(1, At(21 + 18, 21));
        Assert.Equal(0, At(21 + 5, 21 + 5));
        Assert.Equal(1, At(21 + 10, 21 + 10));
        Assert.Null(HeroGlyph.HeroMask("questionmark"));
    }

    /// <summary>PixelGlyph.swift:162-185: a still glyph lights the same cells every frame; time moves only colour and brightness.</summary>
    [Fact]
    public void TimeMovesColoursNotCells()
    {
        HeroGlyph.Mask mask = HeroGlyph.HeroMask("cpu")!;
        var at0 = HeroGlyph.LitCells(mask, 0, 3);
        var at7 = HeroGlyph.LitCells(mask, 7.3, 3);
        Assert.Equal(at0.Select(c => (c.Column, c.Row)), at7.Select(c => (c.Column, c.Row)));
        Assert.NotEqual(at0.Select(c => (c.Shade, c.Bright)), at7.Select(c => (c.Shade, c.Bright)));
    }

    /// <summary>The render draws a lit cell in its tint at the cell's strength, with the one-point gap.</summary>
    [Fact]
    public void TheRenderDrawsLitCellsWithTheGap()
    {
        HeroGlyph.Mask mask = HeroGlyph.HeroMask("cpu")!;
        PixelCanvas canvas = HeroGlyph.Render(mask, 0, dark: true, pixelsPerPoint: 1);
        Assert.Equal(252, canvas.Width);
        HeroGlyph.Lit cell = HeroGlyph.LitCells(mask, 0, 3).First();
        Rgba inside = canvas.Pixel(cell.Column * 6 + 2, cell.Row * 6 + 2);
        Assert.Equal(cell.Bright ? 0.46 : 0.32, inside.A, 2);
        Rgba gap = canvas.Pixel(cell.Column * 6 + 5, cell.Row * 6 + 2);
        Assert.Equal(0, gap.A, 3);
    }

    private static void AssertMask(string symbol, string[] rows, int covered, int lit)
    {
        HeroGlyph.Mask mask = HeroGlyph.HeroMask(symbol)!;
        int n = mask.Columns;
        var drawn = new string[n];
        int c = 0, l = 0;
        for (int row = 0; row < n; row++)
        {
            var line = new StringBuilder();
            for (int column = 0; column < n; column++)
            {
                double coverage = mask.Coverage[row * n + column];
                bool isLit = HeroGlyph.IsLit(coverage, column, row, n, n);
                c += coverage >= 0.5 ? 1 : 0;
                l += isLit ? 1 : 0;
                line.Append(isLit ? '#' : coverage >= 0.5 ? '+' : '.');
            }

            drawn[row] = line.ToString();
        }

        Assert.Equal(rows, drawn);
        Assert.Equal(covered, c);
        Assert.Equal(lit, l);
    }

    private static readonly string[] CpuRows =
    {
        "..........................................",
        "..........................................",
        "...............+#.....##.....##...........",
        "...............#+.....+#.....##...........",
        "...............+#.....##.....##...........",
        "...............#+.....+#.....##...........",
        "...............##.....##.....##...........",
        "...............#+.....##.....##...........",
        "...........+#######################.......",
        "..........+#+#+#+#+#################......",
        "..........###+######################......",
        "..........+#+....................###......",
        "..........###....................###......",
        "....+#+#+#+#+....................#########",
        "....#+###+###....................#########",
        "..........+#+....############....###......",
        "..........###....############....###......",
        "..........+##....##........##....###......",
        "..........###....##........##....###......",
        "..........##+....##........##....###......",
        "....#########....##........##....#########",
        "....+#+######....##........##....#########",
        "..........###....##........##....###......",
        "..........###....##........##....###......",
        "..........###....##........##....###......",
        "..........###....############....###......",
        "..........###....############....###......",
        "....+########....................#########",
        "....#########....................#########",
        "..........###....................###......",
        "..........###....................###......",
        "..........##########################......",
        "..........##########################......",
        "...........########################.......",
        "...............##.....##.....##...........",
        "...............##.....##.....##...........",
        "...............##.....##.....##...........",
        "...............##.....##.....##...........",
        "...............##.....##.....##...........",
        "...............##.....##.....##...........",
        "..........................................",
        "..........................................",
    };

    private static readonly string[] ChartRows =
    {
        "..........................................",
        "..........................................",
        "..........................................",
        "..........................................",
        "..........................................",
        ".............#+#+#+#......................",
        ".............+###+##......................",
        ".............#+#+#+#......................",
        ".............#######......................",
        ".............#+#+#+#......................",
        ".............+######......................",
        ".............#+#+###......................",
        ".............#######...#######............",
        ".............#+#####...#######............",
        ".............#######...#######............",
        ".............###+###...#######............",
        ".............#######...#######............",
        "...#+#+#+#...#######...#######............",
        "...##+####...#######...#######............",
        "...#+#+#+#...#######...#######............",
        "...#######...#######...#######............",
        "...#+#+###...#######...#######............",
        "...#######...#######...#######............",
        "...#+###+#...#######...#######............",
        "...#######...#######...#######............",
        "...#######...#######...#######............",
        "...#######...#######...#######............",
        "...#+#####...#######...#######............",
        "...#######...#######...#######............",
        "...#######...#######...#######............",
        "...#######...#######...#######............",
        "...#######...#######...#######............",
        "...#######...#######...#######............",
        "...#######...#######...#######............",
        "...#######...#######...#######............",
        "...#######...#######...#######............",
        "...#######...#######...#######............",
        "..........................................",
        "..........................................",
        "..........................................",
        "..........................................",
        "..........................................",
    };

}
