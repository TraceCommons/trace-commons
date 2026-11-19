using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The decisions the window plays: the credit celebration's plan and player,
/// the shell's switches, the tray strip from a recorded status, Home's model
/// from recorded reads, and the core's words those surfaces show.
/// </summary>
public class HomeWiringTests
{
    private static string Recorded(string set, string method) => File.ReadAllText(Path.Combine(
        AppContext.BaseDirectory, "..", "..", "..", "..", "..", "..",
        "macos", "Sources", "TCShellCore", "DataContract", "RecordedSamples", set, method + ".json"));

    /// <summary>DockCreditAnimator.swift:93-130: off plays nothing; reduce motion one still frame held 1.6 s and no attention.</summary>
    [Fact]
    public void TheCelebrationFollowsTheMacsRules()
    {
        CreditCelebrationPlan off = CreditCelebration.Plan(switchOn: false, reduceMotion: false, windowActive: false, hasTaskbarButton: true);
        Assert.False(off.Play);
        Assert.False(off.FlashTaskbar);

        CreditCelebrationPlan moving = CreditCelebration.Plan(true, false, windowActive: false, hasTaskbarButton: true);
        Assert.True(moving.FlashTaskbar);
        Assert.Equal(41, moving.FrameTimes.Count);
        Assert.Equal(TimeSpan.FromMilliseconds(42), moving.FrameWait);

        CreditCelebrationPlan still = CreditCelebration.Plan(true, reduceMotion: true, windowActive: false, hasTaskbarButton: true);
        Assert.False(still.FlashTaskbar);
        Assert.Equal(new[] { 0.45 }, still.FrameTimes);
        Assert.Equal(TimeSpan.FromMilliseconds(1600), still.FrameWait);

        // The flash is for an app in the background, with a button to flash.
        Assert.False(CreditCelebration.Plan(true, false, windowActive: true, hasTaskbarButton: true).FlashTaskbar);
        Assert.False(CreditCelebration.Plan(true, false, windowActive: false, hasTaskbarButton: false).FlashTaskbar);
    }

    /// <summary>A new arrival restarts from the first frame; stopping goes to rest at once.</summary>
    [Fact]
    public void ThePlayerRestartsAndStops()
    {
        var player = new CreditCelebration.Player();
        player.Start(CreditCelebration.Plan(true, false, false, true));
        Assert.Equal(0.0, player.Step());
        Assert.Equal(1 / 24.0, player.Step()!.Value, 9);
        player.Start(CreditCelebration.Plan(true, false, false, true));
        Assert.Equal(0.0, player.Step());
        player.Stop();
        Assert.Null(player.Step());
        Assert.False(player.Playing);

        player.Start(CreditCelebration.Plan(true, reduceMotion: true, false, true));
        Assert.Equal(0.45, player.Step());
        Assert.Null(player.Step());
    }

    /// <summary>The Mac's defaults: credit switch on, rail off, promotions on; choices survive a restart; a broken file reads as defaults.</summary>
    [Fact]
    public void TheSwitchesKeepTheMacsDefaultsAndPersist()
    {
        string dir = Path.Combine(Path.GetTempPath(), "tc-prefs-" + Guid.NewGuid().ToString("N"));
        string path = Path.Combine(dir, "shell-preferences.json");
        try
        {
            var prefs = new ShellPreferences(path);
            Assert.True(prefs.CreditIconAnimation);
            Assert.False(prefs.EdgeRail);
            Assert.True(prefs.ShowsPromotions);
            Assert.Equal(string.Empty, prefs.ClosedCampaign);
            Assert.Null(prefs.BalanceShown);
            int changes = 0;
            prefs.Changed += () => changes++;
            prefs.CreditIconAnimation = false;
            prefs.EdgeRail = true;
            prefs.BalanceShown = new BalanceArrival.Reading(5, 3);
            prefs.EdgeRail = true;
            Assert.Equal(3, changes);

            var again = new ShellPreferences(path);
            Assert.False(again.CreditIconAnimation);
            Assert.True(again.EdgeRail);
            Assert.Equal(new BalanceArrival.Reading(5, 3), again.BalanceShown);

            File.WriteAllText(path, "{ not json");
            Assert.True(new ShellPreferences(path).CreditIconAnimation);
        }
        finally
        {
            if (Directory.Exists(dir))
            {
                Directory.Delete(dir, recursive: true);
            }
        }
    }

    /// <summary>The recorded normalDay status: two owed, the core's "ready" mark, so the halo rings the badge and the core's words are spoken.</summary>
    [Fact]
    public void TheTrayStripComesFromTheRecordedStatus()
    {
        DaemonStatus status = JsonSerializer.Deserialize<DaemonStatus>(Recorded("normalDay", "status"))!;
        (Strip strip, StripMarkWords? words) = TrayStrip.FromStatus(status);
        Assert.Equal(StripMark.Ready, strip.Mark);
        Assert.Equal(StripMark.Ready, TrayStrip.ShownMark(strip.Mark, strip.Badge, strip.Condition));
        Assert.Equal("2 of them have been idle for 3 days or more.", words!.Value.Accessibility);
        Assert.Equal("2 idle for 3 days or more.", words.Value.Tooltip);

        (Strip down, StripMarkWords? none) = TrayStrip.FromStatus(null);
        Assert.Equal(StripCondition.Unavailable, down.Condition);
        Assert.Null(none);
    }

    /// <summary>The words the Settings switch and Home show are the core's, verbatim, read over the ABI.</summary>
    [Fact]
    public void TheSwitchAndHomeWordsAreTheCores()
    {
        Assert.Equal("Credit arrivals", MonitorShellCopy.Section("home_history", "credit_icon_animation_title"));
        Assert.Equal("Animate Dock and menu bar icons", MonitorShellCopy.Section("home_history", "credit_icon_animation_toggle"));
        Assert.Equal("Show the edge rail", MonitorShellCopy.Section("edge_rail", "setting_toggle"));
        Assert.Equal("Traces", MonitorShellCopy.Section("home_history", "section_traces"));
        Assert.Equal(string.Empty, MonitorShellCopy.Section("home_history", "no_such_word"));
        Assert.Equal(string.Empty, MonitorShellCopy.Section("no_such_table", "section_traces"));
    }

    private static HomeOverviewModel.Figures Figures => new(
        count => count.ToString(System.Globalization.CultureInfo.InvariantCulture),
        credit => credit.ToString("0.0", System.Globalization.CultureInfo.InvariantCulture),
        (nanos, scale) => nanos is long n && scale is not null ? "$" + (n / 1e9).ToString("0.00", System.Globalization.CultureInfo.InvariantCulture) : string.Empty);

    /// <summary>
    /// Home from the recorded normalDay replies: the Traces and Account tiles
    /// in the core's words, the campaign while it is not closed, Missions only
    /// while missions are on, Insights only once its heading has loaded.
    /// </summary>
    [Fact]
    public void HomeIsDrawnFromTheRecordedReads()
    {
        DaemonStatus status = JsonSerializer.Deserialize<DaemonStatus>(Recorded("normalDay", "status"))!;
        HistoryRollup rollup = JsonSerializer.Deserialize<HistoryRollup>(Recorded("normalDay", "history_rollup"))!;
        using JsonDocument credit = JsonDocument.Parse(Recorded("normalDay", "commons_credit_summary"));
        Campaign campaign = Campaign.Compiled()!;
        var reads = new HomeOverviewModel.Reads(
            status, rollup, CommonsCredit.Parse(credit.RootElement), null, Array.Empty<QueueEntry>(), null,
            campaign, CampaignShown: true, MissionsOn: false, MissionDrafts: null, InsightsHeading: null, InsightsIntro: null);
        HomeOverviewModel model = HomeOverviewModel.From(reads, MonitorShellCopy.Top, k => MonitorShellCopy.Section("home_history", k), "NEAR AI balance", Figures);

        Assert.Equal(new[] { MonitorShellCopy.Top("waiting"), MonitorShellCopy.Top("contributed") }, model.TraceTiles.Select(t => t.Label));
        Assert.Equal(status.DecisionsOwed!.Value.ToString(System.Globalization.CultureInfo.InvariantCulture), model.TraceTiles[0].Value);
        Assert.Equal(3, model.AccountTiles.Count);

        // The commons has not said what pending credit waits on: a dash (D6).
        Assert.Equal(HomeFacts.Dash, model.AccountTiles[1].Value);
        Assert.Equal(HomeFacts.Dash, model.AccountTiles[2].Value);
        Assert.Equal(new[] { HomeGround.Campaign }, model.Heroes.Select(h => h.Ground));
        Assert.Equal("cpu", model.Heroes[0].Symbol);
        Assert.True(model.Heroes[0].Closes);

        HomeOverviewModel more = HomeOverviewModel.From(
            reads with { CampaignShown = false, MissionsOn = true, InsightsHeading = "Insights", InsightsIntro = "Intro." },
            MonitorShellCopy.Top, k => MonitorShellCopy.Section("home_history", k), "NEAR AI balance", Figures);
        Assert.Equal(new[] { HomeGround.Missions, HomeGround.Insights }, more.Heroes.Select(h => h.Ground));
        Assert.Equal("scope", more.Heroes[0].Symbol);
        Assert.Equal("chart.bar.xaxis", more.Heroes[1].Symbol);
        Assert.Equal("Intro.", more.Heroes[1].Subtitle);
        Assert.Equal(MonitorShellCopy.Section("home_history", "insights_teaser"), more.InsightsLine);
        Assert.All(more.Heroes, h => Assert.False(h.Closes));
    }

    /// <summary>Not contributed shows only while something waits that has not left (HomeViews.swift:1886-1897).</summary>
    [Fact]
    public void NotContributedCountsOnlyWhatHasNotLeft()
    {
        var pending = new List<QueueEntry>
        {
            new() { State = "pending" },
            new() { State = "uploading" },
            new() { State = "uploaded" },
            new() { State = "held" },
        };
        var reads = new HomeOverviewModel.Reads(null, null, null, null, pending, null, null, false, false, null, null, null);
        HomeOverviewModel model = HomeOverviewModel.From(reads, MonitorShellCopy.Top, k => MonitorShellCopy.Section("home_history", k), string.Empty, Figures);
        Assert.Equal(3, model.TraceTiles.Count);
        Assert.Equal("Not contributed", model.TraceTiles[2].Label);
        Assert.Equal("2", model.TraceTiles[2].Value);
        Assert.Equal(HomeFacts.Dash, model.TraceTiles[0].Value);
        Assert.Equal(2, model.AccountTiles.Count);
    }

    /// <summary>The compact figure today's tokens are written in (<c>.notation(.compactName)</c>).</summary>
    [Theory]
    [InlineData(999UL, "999")]
    [InlineData(1000UL, "1K")]
    [InlineData(1250UL, "1.2K")]
    [InlineData(1350UL, "1.4K")]
    [InlineData(12_345UL, "12K")]
    [InlineData(999_999UL, "1M")]
    [InlineData(2_500_000UL, "2.5M")]
    public void TodaysTotalIsCompact(ulong value, string text) => Assert.Equal(text, HomeOverviewModel.Compact(value));
}
