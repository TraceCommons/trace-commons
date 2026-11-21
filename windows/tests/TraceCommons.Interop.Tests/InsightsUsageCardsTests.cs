using System;
using System.Collections.Concurrent;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using TraceCommons.App.ViewModels;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The usage cards' wiring in the Insights window (InsightsView.swift:253-282,
/// InsightsOverviewTab.swift:66-67, InsightsUsageCards.swift): the limits are
/// read while Overview is on screen, silently on open; "By project" opens
/// Breakdown on its By project view; the credit tile is Home's; the sample
/// has a Claude and a Codex login.
/// </summary>
public sealed class InsightsUsageCardsTests
{
    private static async Task Settle(Func<bool> done)
    {
        var clock = Stopwatch.StartNew();
        while (!done() && clock.ElapsedMilliseconds < 5_000) await Task.Delay(1);
    }

    private static (InsightsViewModel Model, ConcurrentQueue<(bool Refresh, bool Interactive, string? Login)> Calls) Window()
    {
        var calls = new ConcurrentQueue<(bool, bool, string?)>();
        var subscriptions = new SubscriptionUsageModel((refresh, interactive, _, login) =>
        {
            calls.Enqueue((refresh, interactive, login));
            return Task.FromResult(new SubscriptionUsage(new[]
            {
                new SubscriptionUsageProvider("claude", true, Logins: new[] { new SubscriptionUsageLogin("claude-default", "Claude", "unknown", Reason: "not_checked") }),
            }));
        });
        var service = new RefusingInsights();
        return (new InsightsViewModel(service, subscriptions: subscriptions), calls);
    }

    private sealed class RefusingInsights : ILocalInsights
    {
        public Task<System.Text.Json.JsonElement> CallAsync(object request, CancellationToken cancellationToken) =>
            throw new InvalidOperationException("no store in this test");
    }

    /// <summary>Overview reads the limits on open, silently, and leaving it stops the reads.</summary>
    [Fact]
    public async Task OverviewReadsTheLimitsWhileItIsShown()
    {
        (InsightsViewModel model, var calls) = Window();
        using var _ = model;
        await model.SelectTabAsync(InsightsViewModel.OverviewTab).ContinueWith(_ => { });
        await Settle(() => calls.Count >= 2);
        Assert.Equal((false, false, null), calls.ElementAt(0));
        Assert.Equal((true, false, "claude-default"), calls.ElementAt(1));
        Assert.True(model.Subscriptions.IsVisible);
        await model.SelectTabAsync(InsightsViewModel.PatternsTab).ContinueWith(_ => { });
        Assert.False(model.SubscriptionsObserved);
        await model.SelectTabAsync(InsightsViewModel.OverviewTab).ContinueWith(_ => { });
        Assert.True(model.SubscriptionsObserved);
        model.SetClocksRunning(false);
        Assert.False(model.SubscriptionsObserved);
        model.SetClocksRunning(true);
        Assert.True(model.SubscriptionsObserved);
        model.Cancel();
        Assert.False(model.SubscriptionsObserved);
    }

    /// <summary>Another tab never reads the limits.</summary>
    [Fact]
    public async Task AnotherTabNeverReadsTheLimits()
    {
        (InsightsViewModel model, var calls) = Window();
        using var _ = model;
        await model.SelectTabAsync(InsightsViewModel.BenchmarksTab).ContinueWith(_ => { });
        await Task.Delay(50);
        Assert.Empty(calls);
        Assert.False(model.SubscriptionsObserved);
    }

    /// <summary>"By project": Breakdown, on its By project view, whichever view it was left on.</summary>
    [Fact]
    public void ByProjectOpensBreakdownOnByProject()
    {
        using var model = new InsightsViewModel(new RefusingInsights());
        model.DetailsView = InsightsDetailsWords.Dimension.Model;
        Assert.Equal(InsightsViewModel.PatternsTab, model.ShowProjects());
        Assert.Equal(InsightsDetailsWords.Dimension.Folder, model.DetailsView);
    }

    /// <summary>The credit tile is Home's and says so when it changes.</summary>
    [Fact]
    public void TheCreditTileIsHomesAndAnnouncesItself()
    {
        using var model = new InsightsViewModel(new RefusingInsights());
        Assert.Null(model.CreditTile);
        int changes = 0;
        model.PropertyChanged += (_, _) => changes++;
        model.CreditTile = ("12.5", "Final");
        Assert.Equal(("12.5", "Final"), model.CreditTile);
        Assert.Equal(1, changes);
        model.CreditTile = ("12.5", "Final");
        Assert.Equal(1, changes);
    }

    /// <summary>
    /// The Mac's sample limits (<c>InsightsSample.subscriptions</c>,
    /// InsightsSample.swift:177-205): a Claude login with both windows and a
    /// Codex login with a weekly one, resetting a few days on.
    /// </summary>
    [Fact]
    public void TheSampleHasAClaudeAndACodexLogin()
    {
        DateTimeOffset now = DateTimeOffset.FromUnixTimeSeconds(1_800_000_000);
        SubscriptionUsage usage = InsightsSample.Subscriptions(now);
        Assert.Equal(SubscriptionUsage.Schema, usage.SchemaVersion);
        Assert.Equal(new[] { "claude", "codex" }, usage.Providers.Select(p => p.Provider));
        SubscriptionUsageLogin claude = usage.Providers[0].Logins.Single();
        Assert.Equal(("sample-claude", "observed", true), (claude.Id, claude.State, claude.DefaultLogin));
        Assert.Equal(new[] { ("session", 340, 300), ("weekly", 410, 10_080) },
            claude.Windows.Select(w => (w.Lane, w.UsedPermille!.Value, w.DurationMins!.Value)));
        Assert.Equal(now.AddHours(2.4), claude.Windows[0].ResetsAt);
        Assert.Equal(now.AddDays(3.2), claude.Windows[1].ResetsAt);
        SubscriptionUsageLogin codex = usage.Providers[1].Logins.Single();
        Assert.Equal(("weekly", 220, 10_080), (codex.Windows.Single().Lane, codex.Windows.Single().UsedPermille!.Value, codex.Windows.Single().DurationMins!.Value));
        Assert.Equal(now.AddDays(4.6), codex.Windows.Single().ResetsAt);
        Assert.Equal(2, InsightsLimitsRows.Rows(usage.Providers, now, key => key).Count);
    }

    private static string Shell(string relative) => File.ReadAllText(
        Path.Combine(AppContext.BaseDirectory, "shell-source", relative + ".txt")).Replace("\r\n", "\n", StringComparison.Ordinal);

    /// <summary>
    /// The view draws the Mac's cards behind the Mac's gates
    /// (InsightsUsageCards.swift:43-67, :141-182): one heading over the login
    /// cards and the NEAR AI card, the refresh hint only beside a Refresh not
    /// spinning, every card ending in its bars and "By project", Refresh
    /// scoped to its login; and Breakdown leads with the drill-down under
    /// feed T (InsightsPatternsTab.swift:49).
    /// </summary>
    [Fact]
    public void TheViewDrawsTheCardsBehindTheMacsGates()
    {
        string view = Shell("TraceCommons.App/Controls/InsightsView.UsageCards.cs");
        Assert.Contains("InsightsLimitsRows.Rows(ViewModel.Subscriptions.SupportedProviders, DateTimeOffset.Now, copy)", view);
        Assert.Contains("if (logins.Count > 0 || nearAi is not null)", view);
        Assert.Contains("if (logins.Count > 0 && !ViewModel.Subscriptions.IsRefreshing)", view);
        Assert.Contains("Text(copy, \"analytics_subscriptions_refresh_hint\")", view);
        Assert.Contains("if (ViewModel.Subscriptions.FailureKey is { } failure)", view);
        Assert.Contains("_ = ViewModel.Subscriptions.Refresh(row.Provider, row.LoginId);", view);
        Assert.Contains("IsEnabled = ViewModel.Subscriptions.CanRefresh(row.Provider, row.LoginId)", view);
        Assert.Contains("\"analytics_subscriptions_refresh_login_accessibility\"", view);
        Assert.Contains("if (!row.Measured) panel.Children.Add(Caption(Text(copy, \"analytics_limits_refresh_to_see\")));", view);
        Assert.Contains("InsightsUsageWords.TokenBars(week?.UsageByDay, row.Provider == \"claude\" ? \"claude_code\" : row.Provider, copy)", view);
        Assert.Contains("Text(copy, \"analytics_usage_by_project\")", view);
        Assert.Contains("\"analytics_usage_by_project_a11y\"", view);
        Assert.Contains("OnTabPressed(ViewModel.ShowProjects());", view);
        Assert.Equal(1, CountOf(view, "private UIElement UsageCard("));
        Assert.Equal(2, CountOf(view, "UsageCard(title"));
        Assert.DoesNotContain("SetEnabledAsync", view);
        string extras = Shell("TraceCommons.App/Controls/InsightsView.WeekExtras.cs");
        Assert.Contains("InsightsWorkWords.Tiles(week?.Work, ViewModel.CreditTile, copy)", extras);
        Assert.Contains("DrawUsageCards(week, copy);", extras);
        Assert.Contains("if (week is not null) PatternsDetails.Children.Add(DetailsTable(week, copy));", extras);
        string xaml = Shell("TraceCommons.App/Controls/InsightsView.xaml");
        Assert.Contains("<StackPanel x:Name=\"PatternsDetails\"", xaml);
        Assert.True(xaml.IndexOf("x:Name=\"PatternsDetails\"", StringComparison.Ordinal) < xaml.IndexOf("Patterns.Status", StringComparison.Ordinal),
            "the drill-down leads Breakdown");
    }

    /// <summary>The window hands Insights Home's reads and the daemon's limits; a sample never reaches a live daemon.</summary>
    [Fact]
    public void TheWindowWiresHomesTileAndTheLimits()
    {
        string window = Shell("TraceCommons.App/MainWindow.xaml.cs");
        Assert.Contains("subscriptions: new SubscriptionUsageModel(InsightsViewModel.SampleSubscriptionsReader)", window);
        Assert.Contains("subscriptions: new SubscriptionUsageModel(SubscriptionUsageClient.Reader((method, parameters) => _host.CallAsync(method, parameters)),\n"
            + "                SubscriptionUsageClient.Writer((method, parameters) => _host.CallAsync(method, parameters)))", window);
        string home = Shell("TraceCommons.App/MainWindow.Home.cs");
        Assert.Contains("HomeFacts.InsightsCreditTile(_homeRollup, _homeCredit, HistoryCopy.CreditFigure, HomeFacts.CreditTileWords.FromMonitorTable())", home);
        Assert.True(CountOf(home, "ShowInsightsCreditTile();") >= 2, "both of Home's read paths hand Insights the tile");
    }

    private static int CountOf(string text, string part)
    {
        int count = 0;
        for (int at = text.IndexOf(part, StringComparison.Ordinal); at >= 0; at = text.IndexOf(part, at + part.Length, StringComparison.Ordinal)) count++;
        return count;
    }
}
