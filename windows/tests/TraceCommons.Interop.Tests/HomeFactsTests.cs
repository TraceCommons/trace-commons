using System;
using System.Linq;
using System.Text.Json;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>Home's rules, the edge rail's geometry and the flyout's placement, pinned to the Mac and the probe.</summary>
public class HomeFactsTests
{
    private static string? Figure(double v) => v.ToString("0.0", System.Globalization.CultureInfo.InvariantCulture);

    private static string? Count(ulong v) => v.ToString(System.Globalization.CultureInfo.InvariantCulture);

    /// <summary>HomeViews.swift:2014-2018: unknown is a dash, zero is a number.</summary>
    [Fact]
    public void UnknownIsADashAndZeroIsANumber()
    {
        Assert.Equal("—", HomeFacts.Count(null, Count));
        Assert.Equal("0", HomeFacts.Count(0, Count));
        Assert.Equal("—", HomeFacts.Points(null, Figure));
    }

    /// <summary>
    /// HomeViews.swift:2120-2136 (D6): pending credit is a figure only beside
    /// the commons' statement; received is 0 once History answered with
    /// nothing decided.
    /// </summary>
    [Fact]
    public void PendingCreditIsAFigureOnlyBesideTheCommonsStatement()
    {
        var silent = new CommonsCredit(PostureKnown: false, SettlementExplanation: "x", PointsKnown: false, PointsLifetimeEarned: null, AccountSessionRequired: true);
        var stated = silent with { PostureKnown = true, SettlementExplanation = "Settles monthly." };
        Assert.Equal(("—", "—"), HomeFacts.CreditTiles(null, null, stated, Figure));
        Assert.Equal(("0.0", "—"), HomeFacts.CreditTiles(null, 4.5, silent, Figure));
        Assert.Equal(("0.0", "4.5"), HomeFacts.CreditTiles(null, 4.5, stated, Figure));
        Assert.Equal(("12.0", "4.5"), HomeFacts.CreditTiles(12, 4.5, stated, Figure));
        Assert.Null(HomeFacts.PendingCondition(stated with { SettlementExplanation = "" }));
    }

    private static (string Value, string Detail) Tile(double? final, double? pending, int? accepted, int? submitted, CommonsCredit? credit) =>
        HomeFacts.CreditTile(final, pending, accepted, submitted, credit, Figure, TileWords);

    private static readonly HomeFacts.CreditTileWords TileWords = new("Pending", "Final", "{amount} credit pending");

    /// <summary>
    /// HomeViews.swift:2074-2104 and HomeViewTests.swift:164-217: one credit
    /// tile, History's Final figure once any credit is decided with what is
    /// still pending under it, and before that the pending figure under D6,
    /// marked Pending. Insights' "Credits earned" tile is this one.
    /// </summary>
    [Fact]
    public void TheCreditTileShowsFinalOnceDecided()
    {
        var known = new CommonsCredit(PostureKnown: true, SettlementExplanation: "Scored when the commons reviews it.", PointsKnown: false, PointsLifetimeEarned: null, AccountSessionRequired: false);

        (string Value, string Detail) undecided = Tile(0, 50.35, 3, 0, known);
        Assert.Equal(HomeFacts.Points(50.35, Figure), undecided.Value);
        Assert.Equal("Pending", undecided.Detail);
        Assert.Equal("—", Tile(null, 50.35, null, 0, null).Value);

        (string Value, string Detail) decided = Tile(48, 2.35, 3, 0, known);
        Assert.Equal("48.0", decided.Value);
        Assert.Equal("2.4 credit pending", decided.Detail);

        Assert.Equal(("48.0", "Final"), Tile(48, 0, 3, 0, known));
        // Every accepted trace decided with no award reads Final 0, never Pending.
        Assert.Equal(("0.0", "Final"), Tile(0, 0, 22, 0, known));
        // With nothing accepted yet it is still the pending figure.
        Assert.Equal("Pending", Tile(0, 0, 0, 0, known).Detail);
        // Without the commons' statement, pending is not shown as a figure.
        Assert.Equal("Final", Tile(48, 2.35, 3, 0, null).Detail);
        // A trace still waiting to be scored is not decided; an unknown count is not zero.
        Assert.Equal(("48.0", "Pending"), Tile(48, 0, 56, 1, known));
        Assert.Equal("Pending", Tile(0, 0, 22, 1, known).Detail);
        Assert.Equal("Pending", Tile(0, 0, 22, null, known).Detail);
        Assert.Equal(("48.0", "Pending"), Tile(48, 0, 3, null, known));

        Assert.True(HomeFacts.ShowsFinalFigure(48, 0, 3, 0));
        Assert.True(HomeFacts.ShowsFinalFigure(48, 2, 3, 1));
        Assert.True(HomeFacts.ShowsFinalFigure(0, 0, 22, 0));
        Assert.False(HomeFacts.ShowsFinalFigure(0, 3, 22, 0));
        Assert.False(HomeFacts.ShowsFinalFigure(null, null, null, null));
        Assert.Null(HomeFacts.CreditPendingAccessory(0, "said", "{amount} credit pending", Figure));
        Assert.Null(HomeFacts.CreditPendingAccessory(2, null, "{amount} credit pending", Figure));
        Assert.Null(HomeFacts.CreditPendingAccessory(2, "said", "", Figure));
    }

    /// <summary>
    /// InsightsView.swift:177-182: Insights draws Home's tile from Home's own
    /// reads, the rollup and the commons' credit, and only once either came.
    /// </summary>
    [Fact]
    public void InsightsDrawsHomesTileOnceHomesReadsCame()
    {
        Assert.Null(HomeFacts.InsightsCreditTile(null, null, Figure, TileWords));
        var rollup = new HistoryRollup { CreditFinal = 48, CreditPending = 0, AllTime = new HistoryCounts { Accepted = 3, Submitted = 0 } };
        Assert.Equal(("48.0", "Final"), HomeFacts.InsightsCreditTile(rollup, null, Figure, TileWords));
        var known = new CommonsCredit(true, "Settles monthly.", false, null, false);
        Assert.Equal(("—", "Pending"), HomeFacts.InsightsCreditTile(null, known, Figure, TileWords));
        Assert.Equal(HomeFacts.CreditTile(48, 0, 3, 0, known, Figure, TileWords), HomeFacts.InsightsCreditTile(rollup, known, Figure, TileWords));
    }

    /// <summary>The recorded normalDay reply: the commons has not said, so pending is a dash, and the account wants a session.</summary>
    [Fact]
    public void TheRecordedCreditSummaryDecodes()
    {
        using JsonDocument doc = JsonDocument.Parse("""
            {"posture_state":"unknown","commons_settlement":null,"commons_settlement_explanation":null,"points_state":"unknown","points_unknown_reason":"account-session-required","commons_points_lifetime_earned":null}
            """);
        CommonsCredit credit = CommonsCredit.Parse(doc.RootElement)!;
        Assert.False(credit.PostureKnown);
        Assert.True(credit.AccountSessionRequired);
        Assert.Null(HomeFacts.CreditAccountTotal(credit, "{amount} across your account", Figure));
        Assert.Equal("7.0 across your account", HomeFacts.CreditAccountTotal(credit with { PointsKnown = true, PointsLifetimeEarned = 7 }, "{amount} across your account", Figure));
        Assert.Null(CommonsCredit.Parse(null));
    }

    /// <summary>HomeViews.swift:138-147: a closed campaign comes back when what it shows changes; promotions off hides it.</summary>
    [Fact]
    public void AClosedCampaignStaysClosedUntilItChanges()
    {
        Campaign campaign = Campaign.Compiled()!;
        string print = HomeFacts.CampaignFingerprint(campaign);
        Assert.True(HomeFacts.ShowsCampaign(print, "", promotionsOn: true));
        Assert.False(HomeFacts.ShowsCampaign(print, print, promotionsOn: true));
        Assert.False(HomeFacts.ShowsCampaign(print, "", promotionsOn: false));
        Assert.NotEqual(print, HomeFacts.CampaignFingerprint(campaign with { Title = campaign.Title + "!" }));
    }

    /// <summary>The compiled campaign is read over the ABI and passes the Mac's checks; its symbol is one a hero draws.</summary>
    [Fact]
    public void TheCompiledCampaignComesFromTheCore()
    {
        Campaign campaign = Campaign.Compiled()!;
        Assert.Equal("cpu", campaign.Symbol);
        Assert.StartsWith("https://near.ai/", campaign.ActionUrl, StringComparison.Ordinal);
        Assert.NotNull(HeroGlyph.HeroMask(campaign.Symbol));
        foreach (string symbol in Campaign.Symbols)
        {
            Assert.NotNull(HeroGlyph.HeroMask(symbol));
        }
    }

    /// <summary>PrivateAICampaign.swift:28-59: what the Mac refuses, this refuses.</summary>
    [Theory]
    [InlineData("{\"schema_version\":2,\"id\":\"a\",\"title\":\"t\",\"body\":\"b\",\"action_title\":\"a\",\"action_url\":\"https://near.ai/x\",\"symbol\":\"cpu\"}")]
    [InlineData("{\"schema_version\":1,\"id\":\"a\",\"title\":\"t\",\"body\":\"b\",\"action_title\":\"a\",\"action_url\":\"http://near.ai/x\",\"symbol\":\"cpu\"}")]
    [InlineData("{\"schema_version\":1,\"id\":\"a\",\"title\":\"t\",\"body\":\"b\",\"action_title\":\"a\",\"action_url\":\"https://evil.example/x\",\"symbol\":\"cpu\"}")]
    [InlineData("{\"schema_version\":1,\"id\":\"a\",\"title\":\"<b>t\",\"body\":\"b\",\"action_title\":\"a\",\"action_url\":\"https://near.ai/x\",\"symbol\":\"cpu\"}")]
    [InlineData("{\"schema_version\":1,\"id\":\"a\",\"title\":\"t\",\"body\":\"b\",\"action_title\":\"a\",\"action_url\":\"https://near.ai/x\",\"symbol\":\"scope\"}")]
    [InlineData("{\"schema_version\":1,\"id\":\"a\",\"title\":\"t\",\"body\":\"b\",\"action_title\":\"a\",\"action_url\":\"https://near.ai/x\",\"symbol\":\"cpu\",\"image_url\":\"https://near.ai/a.gif\"}")]
    public void ACampaignTheMacRefusesIsRefused(string json) => Assert.Null(Campaign.Parse(json));

    /// <summary>HomeInsightsVisual.swift:51-59, 75-76: known positive tokens, most first, four at most; none from a disabled glance.</summary>
    [Fact]
    public void TheInsightsBarsAreTheMacs()
    {
        using JsonDocument doc = JsonDocument.Parse("""
            {"enabled":true,"tools":[{"tool":"a","tokens":5},{"tool":"b","tokens":50},{"tool":"c"},{"tool":"d","tokens":0},{"tool":"e","tokens":7},{"tool":"f","tokens":6},{"tool":"g","tokens":9}]}
            """);
        Assert.Equal(new[] { "b", "g", "e", "f" }, HomeFacts.Bars(doc.RootElement).Select(b => b.Tool));
        using JsonDocument off = JsonDocument.Parse("""{"enabled":false,"tools":[{"tool":"b","tokens":50}]}""");
        Assert.Empty(HomeFacts.Bars(off.RootElement));
        Assert.Equal(190, HomeFacts.BarHeight(50, 50));
        Assert.Equal(8, HomeFacts.BarHeight(1, 1000));
    }

    /// <summary>EdgeRail.swift:12-23, the probe on a 1920x1080 screen: (1436, 200, 484, 680) open, (1916, 420, 4, 240) closed.</summary>
    [Fact]
    public void TheRailsFramesAreTheMacs()
    {
        Assert.Equal(484, EdgeRailGeometry.OpenWidth);
        Assert.Equal(new LayoutRect(1436, 200, 484, 680), EdgeRailGeometry.Frame(true, new LayoutRect(0, 0, 1920, 1080)));
        Assert.Equal(new LayoutRect(1916, 420, 4, 240), EdgeRailGeometry.Frame(false, new LayoutRect(0, 0, 1920, 1080)));
        Assert.Equal(new LayoutRect(956, 25, 484, 600), EdgeRailGeometry.Frame(true, new LayoutRect(0, 25, 1440, 600)));

        // At 150% the same DIPs are 1.5 times the pixels, flush right, on a
        // monitor that starts left of the primary.
        Assert.Equal((-2560 + 2560 - 726, 0 + (1400 - 1020) / 2, 726, 1020), EdgeRailGeometry.FramePixels(true, (-2560, 0, 2560, 1400), 1.5));
    }

    /// <summary>EdgeRailView.swift:62-75: hover opens, leaving closes 300 ms later, a return cancels; the keyboard only after a click.</summary>
    [Fact]
    public void TheRailOpensOnHoverAndClosesAfterLeaving()
    {
        var rail = new EdgeRailHover();
        var t0 = new DateTimeOffset(2026, 11, 19, 0, 0, 0, TimeSpan.Zero);
        Assert.Equal(EdgeRailStep.GrowThenOpen, rail.PointerEntered());
        Assert.False(rail.HoldsKeyboard);
        rail.PointerExited(t0);
        Assert.Equal(EdgeRailStep.None, rail.Tick(t0.AddMilliseconds(299)));
        Assert.Equal(EdgeRailStep.None, rail.PointerEntered());
        Assert.Equal(EdgeRailStep.None, rail.Tick(t0.AddMilliseconds(400)));
        rail.Clicked();
        Assert.True(rail.HoldsKeyboard);
        rail.PointerExited(t0.AddSeconds(1));
        Assert.Equal(EdgeRailStep.CloseThenShrink, rail.Tick(t0.AddMilliseconds(1300)));
        Assert.False(rail.Open);
        Assert.False(rail.HoldsKeyboard);
    }

    /// <summary>EdgeRailView.swift:54, :79: moves in from the trailing edge over 0.25 s on the token curve; under reduce motion it only fades.</summary>
    [Fact]
    public void TheRailSlidesFromTheTrailingEdge()
    {
        (double offset, double opacity, bool done) = EdgeRailHover.Slide(opening: true, 0, reduceMotion: false);
        Assert.Equal((1.0, 0.0, false), (offset, opacity, done));
        (offset, opacity, done) = EdgeRailHover.Slide(opening: true, 0.25, reduceMotion: false);
        Assert.Equal((0.0, 1.0, true), (offset, opacity, done));
        (offset, _, _) = EdgeRailHover.Slide(opening: true, 0.1, reduceMotion: false);
        Assert.InRange(offset, 0.0, 0.5);
        (offset, opacity, _) = EdgeRailHover.Slide(opening: false, 0.1, reduceMotion: true);
        Assert.Equal(0.0, offset);
        Assert.InRange(opacity, 0.0, 0.99);
    }

    /// <summary>Checklist M3: the flyout opens on the clicked monitor, against its taskbar, inside its work area.</summary>
    [Fact]
    public void TheFlyoutOpensOnTheClickedMonitorAgainstTheTaskbar()
    {
        // A secondary monitor left of the primary, taskbar at the bottom.
        (int x, int y) = TrayFlyoutPlacement.Place((-300, 1420), (-2560, 0, 2560, 1440), (-2560, 0, 2560, 1392), (484, 680), 12);
        Assert.Equal((-300 - 242, 1392 - 680 - 12), (x, y));

        // Near the right edge it is kept inside the work area.
        (x, _) = TrayFlyoutPlacement.Place((1910, 1060), (0, 0, 1920, 1080), (0, 0, 1920, 1032), (484, 680), 12);
        Assert.Equal(1920 - 484 - 12, x);

        // A taskbar on the left: right of it.
        (x, y) = TrayFlyoutPlacement.Place((20, 500), (0, 0, 1920, 1080), (48, 0, 1872, 1080), (484, 680), 12);
        Assert.Equal((60, 500 - 340), (x, y));

        // Low on that edge it is held inside the work area.
        (_, y) = TrayFlyoutPlacement.Place((20, 1000), (0, 0, 1920, 1080), (48, 0, 1872, 1080), (484, 680), 12);
        Assert.Equal(1080 - 680 - 12, y);
    }

    [Fact]
    public void FocusGoesBackOnlyToAWindowStillThere() =>
        Assert.Equal(
            new[] { true, false, false, false, false },
            new[]
            {
                FocusReturn.ShouldReturn(true, true, false, true),
                FocusReturn.ShouldReturn(false, true, false, true),
                FocusReturn.ShouldReturn(true, false, false, true),
                FocusReturn.ShouldReturn(true, true, true, true),
                FocusReturn.ShouldReturn(true, true, false, false),
            });
}

/// <summary>
/// The flyout's and the rail's window styles (W13). On tc-win-az5,
/// 2026-11-19, the presenter left both with a 3 px classic frame: the rail's
/// 4 px zone became a 6 px window with no client area, so its XAML never
/// drew and the pointer never reached it, and the rail could not be opened.
/// </summary>
public class ToolWindowStyleTests
{
    // An overlapped window as the presenter leaves it: caption, thick frame,
    // system menu and the minimise and maximise boxes.
    private const long Overlapped = 0x00CF0000;

    [Fact]
    public void AToolWindowHasNoFrameAtAll()
    {
        long style = ToolWindowStyle.Frameless(Overlapped | 0x10000000 /* WS_VISIBLE */);

        Assert.Equal(0, style & ToolWindowStyle.FrameBits);
        Assert.Equal(ToolWindowStyle.Popup, style & ToolWindowStyle.Popup);

        // Visibility is the window's own business, not the style's.
        Assert.Equal(0x10000000, style & 0x10000000);
    }

    [Fact]
    public void TheFrameBitsAreCaptionBorderDialogFrameAndThickFrame()
    {
        // WS_CAPTION is WS_BORDER | WS_DLGFRAME; WS_THICKFRAME is the sizing frame.
        Assert.Equal(0x00C00000 | 0x00800000 | 0x00400000 | 0x00040000, ToolWindowStyle.FrameBits);
    }

    [Fact]
    public void TheExtendedStyleKeepsItOutOfTheSwitchersAndOnTopWithoutActivating()
    {
        long ex = ToolWindowStyle.Extended(0x00000100 /* WS_EX_WINDOWEDGE */ | 0x00000200 /* WS_EX_CLIENTEDGE */ | 0x00000001 /* WS_EX_DLGMODALFRAME */);

        Assert.Equal(0x08000000 | 0x00000080 | 0x00000008, ex & (0x08000000 | 0x00000080 | 0x00000008));

        // No edge of any kind: those draw a frame too.
        Assert.Equal(0, ex & (0x00000100 | 0x00000200 | 0x00000001 | 0x00020000));
    }
}
