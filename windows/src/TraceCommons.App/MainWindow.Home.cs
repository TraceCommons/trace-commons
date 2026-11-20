using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using TraceCommons.App.Controls;
using TraceCommons.Interop;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App;

/// <summary>
/// Home (W11) and the credit arrival (W12): the window's half. Home's reads,
/// its links, the campaign's X, and the one place a live credit arrival is
/// noticed and celebrated on the tray and the taskbar.
/// </summary>
public sealed partial class MainWindow
{
    private readonly HomeView _home = new();
    private readonly ShellPreferences _prefs = ShellPreferences.Shared;
    private readonly BalanceArrival.Tracker _creditTracker = new();
    private readonly CreditCelebration.Player _creditPlayer = new();
    private readonly LocalInsights _homeInsights = new();
    private DispatcherQueueTimer? _creditTimer;
    private TaskbarCredit? _taskbarCredit;
    private bool _homeLoading;
    private string? _insightsHeading;
    private string? _insightsIntro;
    private CommonsCredit? _homeCredit;
    private NearAiBalance? _homeBalance;
    private JsonElement? _homeGlance;
    private ManagedSnapshot? _homeManaged;
    private NetworkInferenceSummary? _homeNetwork;
    private DaemonStatus? _homeStatus;
    private HistoryRollup? _homeRollup;
    private IReadOnlyList<QueueEntry>? _homePending;
    private readonly Campaign? _campaign = Campaign.Compiled();

    /// <summary>Called once from the constructor, after the tray is wired.</summary>
    private void InitializeHome()
    {
        // A sample launch writes nothing to this machine (Program.IsSampleLaunch):
        // its switches and the Balance tile's last reading stay in memory.
        _prefs.ReadOnly = Program.IsSampleLaunch;
        HomePane.Children.Insert(0, _home);

        // The animation and taskbar settings, followed through this window's
        // WM_SETTINGCHANGE: the tray redraws in the taskbar's new ink.
        SystemSettingWatch.Install(WinRT.Interop.WindowNative.GetWindowHandle(this));
        SystemSettingWatch.Changed += () => _tray.RedrawForSettings();
        _home.LinkRequested += OnHomeLink;
        _home.CampaignClosed += () =>
        {
            if (_campaign is Campaign campaign)
            {
                _prefs.ClosedCampaign = HomeFacts.CampaignFingerprint(campaign);
                ShowHome();
            }
        };
        ViewModel.PropertyChanged += (_, change) =>
        {
            if (change.PropertyName == nameof(ViewModels.MainViewModel.ShowingHome))
            {
                _home.Showing = ViewModel.ShowingHome;
                if (ViewModel.ShowingHome)
                {
                    _ = RefreshHomeAsync();
                }
            }
        };
        _home.Showing = ViewModel.ShowingHome;

        // Whether the window is active is W5's _windowActive
        // (MainWindow.Look.cs), which the taskbar flag reads too.
        Activated += (_, _) => SystemSettingWatch.Refresh();

        // Home's own reads follow the daemon while Home is on screen, as the
        // tray's do: the window opens on Home, so no page change would ever
        // ask for them (HomeStore.run on the Mac).
        _host.StatusChanged += RefreshHomeIfShown;
        _host.QueueChanged += RefreshHomeIfShown;

        _creditTimer = Microsoft.UI.Dispatching.DispatcherQueue.GetForCurrentThread().CreateTimer();
        _creditTimer.Tick += (_, _) => CreditFrame();
        _taskbarCredit = new TaskbarCredit(WinRT.Interop.WindowNative.GetWindowHandle(this));

        // Settings' switches take effect at once: the credit switch going
        // off stops a celebration and restores the icons (GeneralSection.swift:41-43).
        _prefs.Changed += () => DispatcherQueue.TryEnqueue(() =>
        {
            if (!_prefs.CreditIconAnimation)
            {
                StopCredit();
            }

            ApplyEdgeRail();

            // Home reads the promotions switch, so it changes at once
            // (GeneralSection.swift:18-20).
            if (ViewModel.ShowingHome)
            {
                ShowHome();
            }
        });
        ApplyEdgeRail();
    }

    private async Task SignInFromHomeAsync()
    {
        ShowPrivateInferencePane();
        OpenTab(MonitorTab.Inference);
        if (PrivateInferencePane.Content is PrivateInferenceView page)
        {
            await page.ViewModel.ContinueCredentialAsync(await page.ViewModel.StartCredentialAsync());
        }
    }

    /// <summary>Whether the commons' catalogue was read and decodes, as main's way in on Home asks (<see cref="ApplyMissionCatalogueEntry"/>).</summary>
    private bool MissionCatalogueRead()
    {
        (MissionRead read, string? json) = _host.MissionCatalogue();
        return MissionPage.Settle(read, json, MonitorShellCopy.Top).Catalogue is not null;
    }

    private void OnHomeLink(HomeLink link)
    {
        switch (link)
        {
            case HomeLink.Traces:
                OpenTab(MonitorTab.Traces);
                break;
            case HomeLink.History:
                OnShowHistory(this, new RoutedEventArgs());
                break;
            case HomeLink.PrivateAI:
                OpenTab(MonitorTab.Inference);
                break;
            case HomeLink.Insights:
                OpenTab(MonitorTab.Insights);
                break;
            case HomeLink.MissionDrafts:
                OnShowMissionDrafts(this, new RoutedEventArgs());
                break;
            case HomeLink.PrivateAISignIn:
                // The Private AI tab's own NEAR AI sign-in, as its managed
                // card begins it (HomeViews.swift:484-487 begins the same
                // sign-in as the tab's notice).
                _ = SignInFromHomeAsync();
                break;
            case HomeLink.MissionCatalogue:
                OnShowMissions(this, new RoutedEventArgs());
                break;
            case HomeLink.Settings:
                // The Mac opens Settings at the data uses; this shell's
                // Settings window has no section to open at yet.
                ShowSettingsWindow();
                break;
            case HomeLink.Campaign:
                if (_campaign is Campaign campaign && Uri.TryCreate(campaign.ActionUrl, UriKind.Absolute, out Uri? url))
                {
                    _ = Windows.System.Launcher.LaunchUriAsync(url);
                }

                break;
        }
    }

    private void RefreshHomeIfShown()
    {
        if (ViewModel.ShowingHome && !_closed)
        {
            _ = RefreshHomeAsync();
        }
    }

    /// <summary>
    /// Home's own reads, beside the status and rollup the tray already holds:
    /// the commons' credit, the NEAR AI balance, today's Insights glance and
    /// the Insights heading. A failed read is null, so Home shows a dash and
    /// never an earlier reply as current.
    /// </summary>
    private async Task RefreshHomeAsync()
    {
        if (_homeLoading)
        {
            return;
        }

        _homeLoading = true;
        try
        {
            Task<DaemonResponse> credit = _host.CallAsync(HomeFacts.CommonsCreditSummaryMethod);
            Task<DaemonResponse> balance = _host.CallAsync(DaemonProtocol.Methods.NearAiBalance);
            Task<DaemonResponse> glance = _host.CallAsync(HomeFacts.InsightsGlanceMethod);
            Task<DaemonResponse> status = _host.CallAsync(DaemonProtocol.Methods.Status);
            Task<DaemonResponse> rollup = _host.CallAsync(DaemonProtocol.Methods.HistoryRollup);
            Task<DaemonResponse> pending = _host.CallAsync(DaemonProtocol.Methods.ListPending);

            // The Private AI section's reads (HomeViews.swift:466-490): the
            // managed snapshot says whether NEAR AI is signed in, the network
            // summary the calls and how many are verified.
            Task<DaemonResponse> managed = _host.CallAsync("managed_snapshot");
            Task<DaemonResponse> network = _host.CallAsync("inference_summary");
            await Task.WhenAll(credit, balance, glance, status, rollup, pending, managed, network).ConfigureAwait(true);
            _homeManaged = managed.Result.IsError || managed.Result.Result is not JsonElement snapshot
                ? null
                : ManagedSnapshot.Parse(snapshot.GetRawText());
            _homeNetwork = network.Result.IsError || network.Result.Result is not JsonElement summary
                ? null
                : InferenceLedger.ParseNetworkSummary(summary) is NetworkInferenceSummary { Readable: true } readable ? readable : null;
            _homeStatus = status.Result.ResultAs<DaemonStatus>();
            _homeRollup = rollup.Result.ResultAs<HistoryRollup>();
            _homePending = pending.Result.ResultAs<PendingList>()?.Pending;
            _homeCredit = credit.Result.IsError ? null : CommonsCredit.Parse(credit.Result.Result);
            _homeGlance = glance.Result.IsError ? null : glance.Result.Result;
            ObserveBalance(balance.Result);

            // Every rollup read notes a settled credit arriving, as the Mac's
            // does (AppModel.recordHistoryRollup, AppModel.swift:2160-2174),
            // not only the tray's.
            if (_homeRollup is HistoryRollup settled)
            {
                ObserveCreditFinal(settled.CreditFinal);
            }

            // A sample launch reads only the recorded set: never the local
            // Insights store (Program.IsSampleLaunch).
            if (_insightsHeading is null && !Program.IsSampleLaunch)
            {
                try
                {
                    JsonElement copy = await _homeInsights.CallAsync(new { type = "copy" }, CancellationToken.None).ConfigureAwait(true);
                    if (copy.TryGetProperty("copy", out JsonElement words))
                    {
                        _insightsHeading = words.TryGetProperty("title", out JsonElement t) ? t.GetString() : null;
                        _insightsIntro = words.TryGetProperty("intro", out JsonElement i) ? i.GetString() : null;
                    }
                }
                catch (Exception)
                {
                    // No heading yet: the Insights hero is not drawn (HomeViews.swift:207).
                }
            }

            ShowHome();
            _ = RefreshSentTrainingLineAsync();
        }
        finally
        {
            _homeLoading = false;
        }
    }

    /// <summary>The tray's reads feed Home too: one status, one rollup, one pending list.</summary>
    private void FeedHome(DaemonStatus? status, HistoryRollup? rollup, IReadOnlyList<QueueEntry>? pending)
    {
        _homeStatus = status;
        _homeRollup = rollup;
        _homePending = pending;
        ShowInsightsCreditTile();
        if (ViewModel.ShowingHome)
        {
            ShowHome();
        }
    }

    /// <summary>
    /// Insights' "Credits earned" tile is Home's, from Home's own reads, so
    /// the two never disagree (InsightsView.swift:177-182); none until the
    /// rollup or the commons' credit came.
    /// </summary>
    private void ShowInsightsCreditTile()
    {
        if (InsightsPane.Content is InsightsView insights)
        {
            insights.ViewModel.CreditTile =
                HomeFacts.InsightsCreditTile(_homeRollup, _homeCredit, HistoryCopy.CreditFigure, HomeFacts.CreditTileWords.FromMonitorTable());
        }
    }

    private void ShowHome()
    {
        ShowInsightsCreditTile();
        string fingerprint = _campaign is Campaign c ? HomeFacts.CampaignFingerprint(c) : string.Empty;
        var reads = new HomeOverviewModel.Reads(
            _homeStatus,
            _homeRollup,
            _homeCredit,
            _homeBalance,
            _homePending,
            _homeGlance,
            _campaign,
            HomeFacts.ShowsCampaign(fingerprint, _prefs.ClosedCampaign, _prefs.ShowsPromotions),
            ViewModel.MissionsShown,
            null,
            _insightsHeading,
            _insightsIntro,
            ViewModel.MissionsShown && MissionCatalogueRead(),
            _homeManaged,
            _homeNetwork);
        PrivateInferenceCopy? copy = PrivateInferenceSurface.Copy();
        _homeModel = HomeOverviewModel.From(
            reads,
            MonitorShellCopy.Top,
            key => MonitorShellCopy.Section("home_history", key),
            copy?.BalanceTitle ?? string.Empty,
            new HomeOverviewModel.Figures(
                count => HistoryCopy.CountFigure((long)count),
                HistoryCopy.CreditFigure,
                NearAiBalanceSurface.Amount));
        _home.Show(_homeModel);
    }

    /// <summary>The last model Home drew; the edge rail's peeks show its tiles.</summary>
    private HomeOverviewModel? _homeModel;

    /// <summary>
    /// A NEAR AI balance read: Home's tile, the Balance tile's flourish from
    /// the reading last shown (kept across launches), and the live tracker
    /// for the tray and taskbar (<c>AppModel.recordBalanceStatus</c>,
    /// AppModel.swift:1036-1041).
    /// </summary>
    private void ObserveBalance(DaemonResponse response)
    {
        if (response.IsError || response.Result is not JsonElement result)
        {
            _homeBalance = null;
            return;
        }

        _homeBalance = NearAiBalanceSurface.Parse(result.GetRawText());
        long? received = ReadNanos(result, "received_total_nanos");
        var now = new BalanceArrival.Reading(received, ReadNanos(result, "remaining_nanos"));
        if (ViewModel.ShowingHome)
        {
            ShowHome();
            _home.CheckBalance(now, _prefs.BalanceShown, _homeBalance?.Scale);
        }

        if (received is not null && ViewModel.ShowingHome)
        {
            _prefs.BalanceShown = now;
        }

        if (_creditTracker.Observe(received))
        {
            Celebrate();
        }
    }

    /// <summary>The tray's balance read, observed when it lands: it never holds the strip.</summary>
    private async void ObserveBalanceWhenRead(Task<DaemonResponse> read)
    {
        DaemonResponse response = await read.ConfigureAwait(true);
        if (!_closed)
        {
            ObserveBalance(response);
        }
    }

    private static long? ReadNanos(JsonElement element, string name) =>
        element.TryGetProperty(name, out JsonElement value) && value.ValueKind == JsonValueKind.Number && value.TryGetInt64(out long n) ? n : null;

    /// <summary>Settled Trace credit (<c>AppModel.recordHistoryRollup</c>, AppModel.swift:2166-2171), even when not signed in to NEAR AI.</summary>
    private void ObserveCreditFinal(double creditFinal)
    {
        if (_creditTracker.ObserveCreditFinal(creditFinal))
        {
            Celebrate();
        }
    }

    /// <summary>
    /// One celebration on the tray and the taskbar, as
    /// <see cref="CreditCelebration.Plan"/> decides it. A new arrival
    /// restarts it from the first frame.
    /// </summary>
    private void Celebrate()
    {
        CreditCelebrationPlan plan = CreditCelebration.Plan(
            _prefs.CreditIconAnimation,
            reduceMotion: !SystemSettingWatch.AnimationsEnabled,
            windowActive: _windowActive,
            hasTaskbarButton: AppWindow.IsVisible);
        if (!plan.Play)
        {
            StopCredit();
            return;
        }

        if (plan.FlashTaskbar)
        {
            _taskbarCredit?.FlashOnce();
        }

        _creditPlayer.Start(plan);
        if (_creditTimer is DispatcherQueueTimer timer)
        {
            timer.Interval = plan.FrameWait;
            timer.Start();
        }

        CreditFrame();
    }

    private void CreditFrame()
    {
        // The switch going off stops it at once and restores the icons
        // (GeneralSection.swift:41-43; MenuBarGlassPanel.swift:692-699).
        double? t = _prefs.CreditIconAnimation ? _creditPlayer.Step() : null;
        _tray.ShowCreditFrame(t);
        _taskbarCredit?.ShowFrame(AppWindow.IsVisible ? t : null, still: !SystemSettingWatch.AnimationsEnabled);
        if (t is null)
        {
            _creditTimer?.Stop();
        }
    }

    /// <summary>Stops a celebration and puts the tray and taskbar icons back.</summary>
    internal void StopCredit()
    {
        _creditPlayer.Stop();
        _creditTimer?.Stop();
        _tray.ShowCreditFrame(null);
        _taskbarCredit?.ShowFrame(null, still: false);
    }

    /// <summary>Settings' credit-arrival switch.</summary>
    internal bool CreditIconAnimation
    {
        get => _prefs.CreditIconAnimation;
        set
        {
            _prefs.CreditIconAnimation = value;
            if (!value)
            {
                StopCredit();
            }
        }
    }
}
