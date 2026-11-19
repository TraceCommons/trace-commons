using System;
using TraceCommons.Interop.Generated;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The tray mark (<c>GlassMenuBarStrip</c> and <c>MenuPanelStatus</c>):
/// the conditions, then the drawn pixels. Lengths are MenuBarPanel.swift's;
/// the halo's reach, 3.5, was printed by the Swift probe.
/// </summary>
public class TrayStripTests
{
    [Fact]
    public void TheLengthsAreTheMacs()
    {
        Assert.Equal((22.0, 14.0), (TrayStrip.Height, TrayStrip.MarkSize));
        Assert.Equal((11.0, 7.0, 2.5), (TrayStrip.BadgeSize, TrayStrip.BadgeGlyph, TrayStrip.BadgeInset));
        Assert.Equal((2.5, 2.0, 3.5), (TrayStrip.HaloInset, TrayStrip.HaloLine, TrayStrip.HaloReach));
        Assert.Equal((9.0, 1.5, 2.0), (TrayStrip.RingSize, TrayStrip.RingLine, TrayStrip.MarkOnlyGap));
    }

    /// <summary>MenuBarGlassPanel.swift:840-849 (the Mac's <c>test_theStripIsDownWhenTheCoreIs</c>).</summary>
    [Fact]
    public void TheStripIsDownWhenTheCoreIs()
    {
        Assert.Equal(StripCondition.Unavailable, TrayStrip.Condition(2, false, false, available: false, stale: false, statusAnswered: true));
        Assert.Equal(StripCondition.Unavailable, TrayStrip.Condition(2, false, false, available: true, stale: true, statusAnswered: true));
        Assert.Equal(StripCondition.Attention, TrayStrip.Condition(2, unhealthy: true, false, true, false, true));
        Assert.Equal(StripCondition.Attention, TrayStrip.Condition(null, false, false, true, false, true));
        Assert.Equal(StripCondition.Paused, TrayStrip.Condition(0, false, paused: true, true, false, true));
        Assert.Equal(StripCondition.Live, TrayStrip.Condition(0, false, false, true, false, true));
        Assert.Equal(StripCondition.Loading, TrayStrip.Condition(null, false, false, true, false, statusAnswered: false));
        Assert.Equal(StripCondition.Attention, TrayStrip.Condition(null, unhealthy: true, false, true, false, statusAnswered: false));
    }

    /// <summary>MenuBarPanel.swift:427-436, MenuBarStatus.swift:31: 99+ past 99, and never a number it cannot vouch for.</summary>
    [Fact]
    public void TheBadgeStopsAtNinetyNineAndNeverGuesses()
    {
        Assert.Equal("99", TrayStrip.BadgeText(99));
        Assert.Equal("99+", TrayStrip.BadgeText(100));
        Assert.Null(TrayStrip.Badge(0));
        Assert.Null(TrayStrip.Badge(null));
        Assert.Null(TrayStrip.ShownBadge(5, StripCondition.Unavailable));
        Assert.Null(TrayStrip.ShownBadge(5, StripCondition.Loading));
        Assert.Equal(5, TrayStrip.ShownBadge(5, StripCondition.Attention));
    }

    /// <summary>MenuBarPanel.swift:394-401, the probe's five cases.</summary>
    [Fact]
    public void TheMarkIsDrawnOnlyWhereItsSlotIs()
    {
        Assert.Equal(StripMark.News, TrayStrip.ShownMark(StripMark.News, null, StripCondition.Live));
        Assert.Equal(StripMark.None, TrayStrip.ShownMark(StripMark.News, 3, StripCondition.Live));
        Assert.Equal(StripMark.Ready, TrayStrip.ShownMark(StripMark.Ready, 3, StripCondition.Attention));
        Assert.Equal(StripMark.None, TrayStrip.ShownMark(StripMark.Ready, 3, StripCondition.Unavailable));
        Assert.Equal(StripMark.None, TrayStrip.ShownMark(StripMark.News, null, StripCondition.Paused));
    }

    /// <summary>NudgeSurface.mark: only the core's news or ready, and nothing while unavailable; the words follow what is drawn.</summary>
    [Fact]
    public void TheMarkIsTheCoresAndItsWordsFollowWhatIsDrawn()
    {
        Assert.Equal(StripMark.Ready, TrayStrip.LitMark("ready", available: true));
        Assert.Equal(StripMark.News, TrayStrip.LitMark("news", available: true));
        Assert.Equal(StripMark.None, TrayStrip.LitMark("loud", available: true));
        Assert.Equal(StripMark.None, TrayStrip.LitMark("ready", available: false));
        var words = new StripMarkWords("2 of them have been idle for 3 days or more.", "2 idle for 3 days or more.");
        Assert.Equal(words, TrayStrip.MarkWords("ready", words, true, 2, StripCondition.Live));
        Assert.Null(TrayStrip.MarkWords("ready", words, true, null, StripCondition.Live));
        Assert.Equal("Base. 2 of them have been idle for 3 days or more.", TrayStrip.MarkAccessibility("Base.", words));
        Assert.Equal("Base.", TrayStrip.MarkAccessibility("Base.", null));
    }

    private static Rgba At(Strip strip, bool dark, double px22, double py22, int size = 44, double? burst = null)
    {
        PixelCanvas canvas = TrayStrip.Render(strip, dark, size, burst);
        double unit = size / 22.0;
        return canvas.Pixel((int)(px22 * unit), (int)(py22 * unit));
    }

    /// <summary>The plain icon is the N in the kit's ink for the taskbar: white on dark, black on light (menuItemMark).</summary>
    [Fact]
    public void ThePlainIconIsTheNInTheKitsInk()
    {
        var plain = new Strip(StripCondition.Live, null, StripMark.None);

        // The N's left stem: 4 + 14 * (90/234) across, half way down.
        Rgba dark = At(plain, true, 4 + 14 * 0.1, 11);
        Rgba light = At(plain, false, 4 + 14 * 0.1, 11);
        Assert.Equal(1.0, dark.A, 2);
        Assert.Equal((1.0, 1.0, 1.0), (dark.R, dark.G, dark.B));
        Assert.Equal((0.0, 0.0, 0.0), (Math.Round(light.R, 3), Math.Round(light.G, 3), Math.Round(light.B, 3)));

        // Outside the 14-point mark the square is clear.
        Assert.Equal(0, At(plain, true, 1, 1).A, 3);
    }

    /// <summary>The badge is the mark's ink, never red, with its figure cut out (MenuBarPanel.swift:686-693).</summary>
    [Fact]
    public void TheBadgeIsTheMarksInkWithItsFigureCutOut()
    {
        var strip = new Strip(StripCondition.Live, 7, StripMark.None);
        PixelCanvas canvas = TrayStrip.Render(strip, true, 44);
        TrayStrip.Slot slot = TrayStrip.SlotOf(strip, 1);
        (double x, double y, double w, double h) = slot.Badge!.Value;
        Assert.Equal((11.0, 11.0, 11.0, 11.0), (x, y, w, h));

        // A corner of the badge, inside the capsule, outside the figure.
        Rgba body = canvas.Pixel((int)((x + w / 2) * 2), (int)((y + 1.5) * 2));
        Assert.Equal(1.0, body.A, 2);
        Assert.Equal((1.0, 1.0, 1.0), (body.R, body.G, body.B));

        // Every opaque pixel is the mark's white: nothing red anywhere.
        for (int py = 0; py < 44; py++)
        {
            for (int px = 0; px < 44; px++)
            {
                Rgba p = canvas.Pixel(px, py);
                if (p.A > 0.5)
                {
                    Assert.True(p.R > 0.99 && p.G > 0.99 && p.B > 0.99, $"({px},{py}) is not the mark's ink");
                }
            }
        }

        // The figure "7" is cut: its top bar's middle cell is clear.
        Rgba cut = canvas.Pixel((int)((x + w / 2) * 2), (int)((y + (h - 5) / 2 + 0.5) * 2));
        Assert.Equal(0, cut.A, 2);
    }

    /// <summary>The halo rings a drawn badge at inset 2.5, line 2, in purpleText as the flat theme resolves it.</summary>
    [Fact]
    public void TheHaloRingsTheBadgeAtTheMacsInset()
    {
        var strip = new Strip(StripCondition.Live, 3, StripMark.Ready);
        TrayStrip.Slot slot = TrayStrip.SlotOf(strip, 1);
        Assert.True(slot.Halo);
        (double x, double y, double w, double h) = slot.Badge!.Value;

        // The halo's line is centred 2.5 points out from the badge's top
        // edge, at the badge's horizontal middle.
        Rgba halo = At(strip, true, x + w / 2, y - TrayStrip.HaloInset);
        Assert.Equal(Rgba.FromArgb(HomeTrayInks.Dark.PurpleText).B, halo.B, 2);
        Assert.True(halo.A > 0.9);

        // Inside the inset, between halo and badge, it is clear: the mark was cut past the halo too.
        Rgba between = At(strip, true, x + w / 2, y - 1);
        Assert.True(between.A < 0.1, $"alpha {between.A}");
    }

    /// <summary>The news ring sits in the empty slot of a live strip, hollow, 9 points, line 1.5.</summary>
    [Fact]
    public void TheNewsRingSitsInTheEmptySlot()
    {
        var strip = new Strip(StripCondition.Live, null, StripMark.News);
        TrayStrip.Slot slot = TrayStrip.SlotOf(strip, 1);
        (double cx, double cy) = slot.Ring!.Value;
        Assert.Equal((22 - 5.25, 22 - 5.25), (cx, cy));
        Assert.True(At(strip, false, cx + 4.5, cy).A > 0.9);
        Assert.True(At(strip, false, cx, cy).A < 0.1);
    }

    /// <summary>The attention capsule takes the slot when there is no badge (MenuBarPanel.swift:704-712).</summary>
    [Fact]
    public void AttentionDrawsTheCapsuleInTheSlot()
    {
        var strip = new Strip(StripCondition.Attention, null, StripMark.None);
        TrayStrip.Slot slot = TrayStrip.SlotOf(strip, 1);
        Assert.NotNull(slot.Attention);
        (double x, double y, double w, double h) = slot.Attention!.Value;
        Rgba fill = At(strip, true, x + 1.5, y + h / 2);
        Assert.Equal(Rgba.FromArgb(HomeTrayInks.Dark.MenuModeNever).R, fill.R, 2);
        Assert.Null(TrayStrip.SlotOf(new Strip(StripCondition.Loading, null, StripMark.None), 1).Attention);
    }

    /// <summary>At the gleam's hold the mark is gold over its ink; at rest it is the ink.</summary>
    [Fact]
    public void TheBurstGleamsTheMarkGold()
    {
        var plain = new Strip(StripCondition.Live, null, StripMark.None);
        Rgba gold = At(plain, true, 4 + 14 * 0.1, 11, burst: 0.5);
        Rgba rest = At(plain, true, 4 + 14 * 0.1, 11, burst: null);
        Rgba coin = Rgba.FromArgb(HomeTrayInks.CoinGold);
        Assert.Equal(coin.R, gold.R, 2);
        Assert.Equal(coin.G, gold.G, 2);
        Assert.Equal(coin.B, gold.B, 2);
        Assert.Equal((1.0, 1.0, 1.0), (rest.R, rest.G, rest.B));

        // Past the burst it is at rest again.
        Rgba after = At(plain, true, 4 + 14 * 0.1, 11, burst: 1.7);
        Assert.Equal((1.0, 1.0, 1.0), (after.R, after.G, after.B));
    }

    /// <summary>The hop lifts the mark: at its peak the N's foot has moved up two points.</summary>
    [Fact]
    public void TheHopLiftsTheMarkUp()
    {
        var plain = new Strip(StripCondition.Live, null, StripMark.None);

        // The N's left stem ends at y = 4 + 14 * 0.99; a point just above
        // its foot is ink at rest, and at the hop's peak the mark has left
        // it: only the flash's faint glow is there.
        double foot = 4 + 14 * 0.93;
        Assert.True(At(plain, true, 4 + 14 * 0.1, foot, burst: null).A > 0.9);
        Assert.True(At(plain, true, 4 + 14 * 0.1, foot, burst: StripBurst.HopDuration / 2).A < 0.5);

        // A point 1.5 points above the mark's resting top is clear at rest
        // and ink while the mark is up: the hop is upward.
        double over = 4 - 1.5;
        Assert.True(At(plain, true, 4 + 14 * 0.1, over, burst: null).A < 0.1);
        Assert.True(At(plain, true, 4 + 14 * 0.1, over, burst: StripBurst.HopDuration / 2).A > 0.5);
    }

    [Fact]
    public void EveryIconSizeIsSquareAndWhole()
    {
        foreach (int size in new[] { 16, 20, 24, 32, 40, 48 })
        {
            PixelCanvas canvas = TrayStrip.Render(new Strip(StripCondition.Live, 120, StripMark.Ready), true, size, 0.2);
            Assert.Equal((size, size), (canvas.Width, canvas.Height));
            Assert.Equal(size * size * 4, canvas.ToBgra().Length);
        }
    }
}
