using System;
using System.Collections.Generic;
using System.Collections.Specialized;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.UI;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using TraceCommons.App.Controls;
using TraceCommons.App.ViewModels;
using TraceCommons.Interop;

namespace TraceCommons.App;

/// <summary>
/// The main window. Owns the <see cref="DaemonHost"/> for the app's lifetime
/// and hands it to the view model.
///
/// The window, not <see cref="App"/>, owns the daemon so that a daemon which
/// fails to start leaves a window standing that can say so. An app that exits
/// at launch because another instance holds the lock tells the contributor
/// nothing.
/// </summary>
public sealed partial class MainWindow : Window
{
    private readonly DaemonHost _host;

    /// <summary>
    /// The notification-area presence, and the only thing in this app that
    /// may interrupt.
    /// </summary>
    private readonly TrayIcon _tray = new();

    /// <summary>
    /// The model-calls wording, read once. The tray entry is rebuilt on every
    /// settings change and the payload does not change under a running
    /// process, so re-reading it across the ABI each time would be a call per
    /// daemon event for a constant.
    /// </summary>
    private readonly PrivateInferenceCopy? _privateInferenceCopy = PrivateInferenceSurface.Copy();

    /// <summary>
    /// The interruption budget. See <see cref="DigestCadence"/> for why the
    /// shell gates this a second time when the daemon already does.
    /// </summary>
    private readonly DigestCadence _digestCadence = new();

    private readonly SemaphoreSlim _trayRefreshGate = new(1, 1);
    private IReadOnlyList<QueueEntry> _trayPending = Array.Empty<QueueEntry>();
    private IReadOnlyList<ProjectSetting> _trayProjects = Array.Empty<ProjectSetting>();
    private HistoryRollup _trayRollup = new();

    /// <summary>
    /// Which queue entry each currently realized SESSION row shows, keyed by
    /// the realized element itself.
    ///
    /// Reads the INNER ItemsRepeater's own ElementPrepared / ElementClearing,
    /// not the outer ListView's per-project realization -- see the doc
    /// comment on the queue ListView in MainWindow.xaml for why that
    /// distinction matters now. Before the inner list virtualized
    /// (ItemsControl, replaced by #354), "every entry under a realized
    /// project" WAS every entry actually on screen, because ItemsControl
    /// realized a project's rows in full the moment the project scrolled
    /// into view. Now a project with hundreds of sessions windows its own
    /// rows independently, and tracking at the outer, per-project level
    /// would report most of a large project as visible -- safe (visibility
    /// only ever affects build order, never membership), but close to
    /// useless as a priority signal for exactly the case this effort exists
    /// to fix. Tracking each ItemsRepeater's own realized elements instead
    /// keeps the reported set to what is actually windowed into view, plus
    /// only WinUI's own small look ahead buffer around it.
    /// </summary>
    private readonly Dictionary<UIElement, string> _visibleEntryIdsByElement = new();

    /// <summary>
    /// The queue ListView's own scroll surface, found once its template is
    /// realized. WinUI does not expose a ListView's ScrollViewer directly;
    /// <see cref="FindDescendant{T}"/> is the standard visual-tree walk for
    /// it. The inner ItemsRepeater has no ScrollViewer of its own -- it
    /// virtualizes against whichever ancestor ScrollViewer's effective
    /// viewport reaches it, which is this one -- so this remains the right
    /// surface to watch for a scroll settling even though the per-row
    /// realization signal below now comes from the inner control.
    /// </summary>
    private ScrollViewer? _queueScrollViewer;

    /// <summary>
    /// Coalesces every visibility-worthy signal -- a row realizing or being
    /// recycled away, a scroll settling -- into one recompute a short
    /// interval after the last one, so a burst of either produces exactly
    /// one <c>preview_visible</c> call rather than one per signal.
    /// </summary>
    private readonly DispatcherQueueTimer _visibilityDebounceTimer;

    private OnboardingWindow? _onboarding;
    private readonly PendingInviteActivation _redirectedInvite = new();
    private bool _activationDaemonAvailable;
    private bool _activationReady;
    private bool _closed;

    private bool _quitConfirmed;

    /// <summary>
    /// Whether the quit confirmation is already on screen.
    /// </summary>
    /// <remarks>
    /// A second close request while the dialog is up would try to open a
    /// second <c>ContentDialog</c>, which throws. The close button is a
    /// system caption button and stays live behind a dialog, so this is a
    /// click away rather than a theoretical race.
    /// </remarks>
    private bool _quitDialogOpen;

    public MainWindow()
    {
        InitializeComponent();

        // The mark and the app name live in the title bar, which means the
        // window has to own that bar rather than let the system draw it. The
        // caption buttons stay the system's: only their background is cleared,
        // so the chrome colour runs behind them and they keep snap layouts,
        // the window menu and their own accessibility behaviour.
        ExtendsContentIntoTitleBar = true;
        SetTitleBar(AppTitleBar);
        AppWindow.TitleBar.ButtonBackgroundColor = Colors.Transparent;
        AppWindow.TitleBar.ButtonInactiveBackgroundColor = Colors.Transparent;

        // DispatcherQueue.GetForCurrentThread() on the UI thread is the queue
        // every event hop targets.
        _host = new DaemonHost(Microsoft.UI.Dispatching.DispatcherQueue.GetForCurrentThread());
        ViewModel = new MainViewModel(_host, new AppUpdater(_host));
        ViewModel.PropertyChanged += (_, change) =>
        {
            if (change.PropertyName == nameof(MainViewModel.ShowingInsights) && !ViewModel.ShowingInsights)
                (InsightsPane.Content as InsightsView)?.Deactivate();
            if (change.PropertyName == nameof(MainViewModel.ShowingMissionDrafts) && !ViewModel.ShowingMissionDrafts)
                (MissionDraftsPane.Content as MissionDraftsView)?.Deactivate();
        };

        // Found once the template is realized, not here: the ScrollViewer
        // inside a ListView's default template does not exist before Loaded.
        QueueListView.Loaded += OnQueueListViewLoaded;

        // The tray reflects daemon state and never drives it: it re-reads
        // status on the same events the queue does rather than being told
        // what to show by the view model, so the two cannot drift.
        _host.QueueChanged += OnTrayWorthyChange;
        _host.StatusChanged += OnTrayWorthyChange;
        _host.DigestDue += OnDigestDue;
        _tray.OpenRequested += OnTrayOpenRequested;
        _tray.ReviewRequested += OnTrayReviewRequested;
        _tray.SettingsRequested += OnTraySettingsRequested;
        _tray.PrivateInferenceRequested += OnTrayPrivateInferenceRequested;
        _tray.PrivateInferenceStopRequested += OnTrayPrivateInferenceStopRequested;
        _tray.PauseRequested += OnTrayPauseRequested;
        _tray.ResumeRequested += OnTrayResumeRequested;
        _tray.QuitRequested += OnTrayQuitRequested;

        AppWindow.Closing += OnAppWindowClosing;
        Closed += OnClosed;
        Activated += OnFirstActivated;

        // One read, three surfaces. The window forwards whatever the daemon
        // last said about the listener to the destination page and to the
        // tray, so the rail badge, the page and the tray glyph cannot come to
        // disagree about the same fact.
        ViewModel.PrivateInferenceSettingsChanged += OnPrivateInferenceSettingsChanged;

        _visibilityDebounceTimer = _host.Dispatcher.CreateTimer();
        _visibilityDebounceTimer.Interval = TimeSpan.FromMilliseconds(200);
        _visibilityDebounceTimer.IsRepeating = false;
        _visibilityDebounceTimer.Tick += (_, _) => RecomputeVisiblePreviews();
    }

    /// <summary>
    /// What quitting costs, said before it happens.
    /// </summary>
    /// <remarks>
    /// Transcribed from the shared design spec, which gives two wordings and
    /// is explicit that picking the wrong one "is a lie about whether the
    /// machine is still watching". This app HOSTS the daemon in-process --
    /// <see cref="DaemonHost"/> owns it and <see cref="OnClosed"/> tears it
    /// down -- so the hosting wording is the true one. The Linux shell says
    /// the other thing, correctly, because there a systemd unit keeps
    /// running.
    /// </remarks>
    private const string QuitBody =
        "Quitting stops Trace Commons watching for finished sessions. Nothing is queued or "
        + "sent until you open it again. Anything already waiting stays waiting.";

    /// <summary>Hide to a reachable tray; otherwise confirm before stopping the daemon.</summary>
    private async void OnAppWindowClosing(
        Microsoft.UI.Windowing.AppWindow sender,
        Microsoft.UI.Windowing.AppWindowClosingEventArgs args)
    {
        var outcome = CloseBehavior.OnWindowClose(_quitConfirmed, _tray.IsPresent, _quitDialogOpen);
        if (outcome == CloseRequestOutcome.Quit)
        {
            return;
        }

        // Cancel synchronously before awaiting any confirmation.
        args.Cancel = true;
        if (outcome == CloseRequestOutcome.HideToTray)
        {
            AppWindow.Hide();
        }
        else if (outcome == CloseRequestOutcome.AskToQuit)
        {
            await ConfirmQuitAsync();
        }
    }

    private async Task ConfirmQuitAsync()
    {
        if (_quitDialogOpen)
        {
            return;
        }

        var dialog = new Microsoft.UI.Xaml.Controls.ContentDialog
        {
            XamlRoot = Content.XamlRoot,
            Title = "Quit Trace Commons?",
            // The extra sentence is the Rust's, not this file's, and it is
            // added only while the switch is on: with the listener inside
            // this process, quitting stops answering model calls as well as
            // stopping the watcher, and QuitBody above does not cover that.
            // A contributor who never turned it on is not warned about
            // losing it.
            Content = ViewModel.PrivateInferenceQuitDetail is { } stopsRouting
                ? QuitBody + "\n\n" + stopsRouting
                : QuitBody,
            PrimaryButtonText = "Quit",
            CloseButtonText = "Cancel",
            DefaultButton = Microsoft.UI.Xaml.Controls.ContentDialogButton.Close,
        };

        Microsoft.UI.Xaml.Controls.ContentDialogResult result;

        _quitDialogOpen = true;
        try
        {
            // WinUI allows one ContentDialog per XamlRoot, and this window now
            // has three other callers into it -- WithdrawDialog, from
            // History; GoPublicDialog, from Settings; and, since this task,
            // the "Ignore project" confirmation from this same window's own
            // queue header. Going through DialogGuard is what actually
            // prevents the crash: it serializes every ShowAsync in the app
            // behind one semaphore, so a dialog opened while this one (or any
            // other) is already up now WAITS its turn instead of throwing.
            // _quitDialogOpen above is a narrower, same-class guard kept for
            // its own reason -- see below -- not the thing preventing the
            // cross-class race.
            //
            // DialogGuard swallows a ShowAsync failure itself and answers
            // None rather than throwing, so the catch below is defense in
            // depth for anything else in this block (Gate.WaitAsync, in
            // principle) rather than the primary safeguard it used to be.
            result = await DialogGuard.ShowOnceAsync(dialog);
        }
        catch (Exception)
        {
            // The window-close path canceled synchronously; the tray path
            // has not attempted to close. Any failure refuses the quit.
            //
            // The window then refuses to close without saying so, and that
            // is right rather than merely tolerable. This runs precisely
            // BECAUSE a dialog is already up, so at the moment of failure the
            // explanation is already on screen and is the thing the
            // contributor has to deal with. Clicking the owner window while a
            // modal is open doing nothing is the platform's own convention,
            // not a missing message.
            //
            // Recorded because both obvious places to add one are wrong, and
            // that is not obvious. MainViewModel.Notice renders inside the
            // QUEUE pane, and this catch can in principle be reached from any
            // of History, Settings, or this window's own queue header -- so
            // the pane carrying the message would not reliably be the pane on
            // screen. The health banner is above all three panes and would be
            // seen, but it is single-writer over the daemon's own health
            // label, and every sentence in it states what happened to the
            // data; a transient UI collision is neither, and an unrecognised
            // label there renders "Contributions are on hold.", which would
            // be false in the direction that makes people quit.
            return;
        }
        finally
        {
            _quitDialogOpen = false;
        }

        // ContentDialogResult.None -- DialogGuard's answer when it could not
        // show the dialog at all -- falls through to the same place a
        // deliberate Cancel does: neither equals Primary, so quit is not
        // confirmed. A dialog that never appeared must not be read as a yes.

        if (result == Microsoft.UI.Xaml.Controls.ContentDialogResult.Primary)
        {
            _quitConfirmed = true;
            Close();
        }
    }

    public MainViewModel ViewModel { get; }

    /// <summary>
    /// Starts with local Insights. Contribution startup remains an explicit
    /// action and retains the existing source declaration and enrollment gates.
    /// </summary>
    private void OnFirstActivated(object sender, WindowActivatedEventArgs args)
    {
        Activated -= OnFirstActivated;
        ShowInsightsPane();
    }

    private bool _contributionStartupAttempted;
    private bool _contributionStartupBusy;
    private async Task<bool> StartContributionsAsync()
    {
        if (_contributionStartupBusy) return false;
        _contributionStartupBusy = true;
        try
        {
            if (!_contributionStartupAttempted)
            {
                await ViewModel.InitializeAsync();
                _contributionStartupAttempted = _host.IsRunning || ViewModel.NeedsSessionRoots;
                if (_closed) return false;
                _activationReady = ViewModel.NeedsSessionRoots;
                OfferNextRedirectedInvite();
                if (_host.IsRunning) await ContinueStartupAsync();
            }
            if (ViewModel.NeedsSessionRoots)
            {
                await ShowSessionRootsAsync();
                return false;
            }
            return !_closed && _host.IsRunning;
        }
        finally { _contributionStartupBusy = false; }
    }

    /// <summary>
    /// The startup work that requires a running daemon.
    /// </summary>
    private async Task ContinueStartupAsync()
    {
        // After the queue is on screen, not before. The update check is a
        // network round trip through the deployment service and nothing
        // about it should stand between a contributor and the sessions they
        // opened the app to review.
        await ViewModel.CheckForUpdateAsync();

        await RefreshTrayAsync();
        await ShowOnboardingIfNeededAsync();
        OfferNextRedirectedInvite();
    }

    /// <summary>
    /// Asks which session folders to watch, then resumes startup.
    /// </summary>
    /// <remarks>
    /// <para>
    /// A window rather than a page: until this is answered there is no daemon,
    /// so the queue behind it has nothing to show and no menu it offers would
    /// work.
    /// </para>
    /// <para>
    /// Discovery runs on a background thread. It counts session files
    /// recursively under both stores, which on a working developer's machine
    /// is thousands of them, and that is a visible hang if it happens on the
    /// UI thread. Same reasoning as the daemon start above.
    /// </para>
    /// </remarks>
    private async Task ShowSessionRootsAsync()
    {
        IReadOnlyList<SourceCandidate> candidates = await Task
            .Run(SourceDiscovery.ProbeThisMachine)
            .ConfigureAwait(true);

        var roots = new SessionRootsWindow(_host, candidates);
        roots.Declared += OnSessionRootsDeclared;
        roots.Activate();
    }

    private async void OnSessionRootsDeclared()
    {
        // The daemon is up now, so the queue window needs the first snapshot
        // it could not load, and then the startup it did not finish.
        _activationReady = false;
        ViewModel.SessionRootsDeclared();
        await ViewModel.RefreshAsync();
        await ContinueStartupAsync();
    }

    /// <summary>
    /// Re-reads the daemon projections the complete tray needs and hands over
    /// one reduced, path-free menu model.
    /// </summary>
    /// <remarks>
    /// The status remains authoritative for icon state and decisions owed.
    /// The other calls only fill the menu's per-project, week, and armed
    /// readouts. A failed ancillary read keeps its previous value rather than
    /// replacing a real contribution count with zero.
    /// </remarks>
    private async Task RefreshTrayAsync()
    {
        if (!_tray.IsPresent)
        {
            return;
        }

        await _trayRefreshGate.WaitAsync().ConfigureAwait(true);

        try
        {
            Task<DaemonResponse> statusTask = _host.CallAsync(DaemonProtocol.Methods.Status);
            Task<DaemonResponse> pendingTask = _host.CallAsync(DaemonProtocol.Methods.ListPending);
            Task<DaemonResponse> rollupTask = _host.CallAsync(DaemonProtocol.Methods.HistoryRollup);
            Task<DaemonResponse> projectsTask = _host.CallAsync(DaemonProtocol.Methods.ListProjects);

            await Task.WhenAll(statusTask, pendingTask, rollupTask, projectsTask)
                .ConfigureAwait(true);

            if (statusTask.Result.ResultAs<DaemonStatus>() is not DaemonStatus status)
            {
                return;
            }

            if (pendingTask.Result.ResultAs<PendingList>() is { } pending)
            {
                _trayPending = pending.Pending;
            }

            if (rollupTask.Result.ResultAs<HistoryRollup>() is { } rollup)
            {
                _trayRollup = rollup;
            }

            if (projectsTask.Result.ResultAs<ProjectSettingsPayload>() is { } projects)
            {
                _trayProjects = projects.Projects;
            }

            TrayMenuModel menu = TrayMenuModel.Compute(
                status,
                _trayPending,
                _trayRollup,
                _trayProjects);
            _tray.Update(menu, status.IsHealthy);
        }
        finally
        {
            _trayRefreshGate.Release();
        }
    }

    private async void OnTrayWorthyChange()
    {
        await RefreshTrayAsync();
    }

    /// <summary>
    /// The configured digest.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Two gates stand between a <c>digest_due</c> event and an interruption,
    /// and both must pass: the daemon's, which is the shared policy every
    /// shell obeys, and <see cref="DigestCadence"/>, which is this process's
    /// own backstop. Neither can cause a notification; each can only suppress
    /// one. Empty events and duplicate deliveries do not create extra digests.
    /// </para>
    /// <para>
    /// Project labels come from the queue the window already holds, which the
    /// daemon has already reduced from paths to labels. No path, and no line
    /// of transcript, reaches a notification.
    /// </para>
    /// </remarks>
    private void OnDigestDue(DigestFacts facts)
    {
        if (!_digestCadence.TryClaim(facts.PendingCount, facts.ContributedCount, DateTimeOffset.UtcNow))
        {
            return;
        }

        // Two sentences, either of which may be absent: what is waiting for
        // you, and what went without you. Separate lines because they are
        // about different things and a contributor acts on only one of them.
        var lines = new List<string>();

        if (facts.PendingCount > 0)
        {
            var labels = new List<string>();
            foreach (QueueEntryViewModel entry in ViewModel.Pending)
            {
                if (entry.ProjectLabel.Length > 0 && !labels.Contains(entry.ProjectLabel))
                {
                    labels.Add(entry.ProjectLabel);
                }
            }

            labels.Sort(StringComparer.Ordinal);
            lines.Add(DigestText.Body(facts.PendingCount, labels));
        }

        // The contributed labels come off the frame, not off ViewModel.Pending:
        // an armed project's traces were never in that list.
        if (DigestText.ContributionLine(
                facts.ContributedCount,
                facts.ContributedProjects,
                facts.CreditPending) is { } contributed)
        {
            lines.Add(contributed);
        }

        if (lines.Count > 0)
        {
            _tray.ShowDigest(string.Join("\n", lines));
        }
    }

    /// <summary>
    /// Brings the window forward from the tray or from a digest.
    /// </summary>
    /// <remarks>
    /// Raising a window is the entire vocabulary of every surface outside it.
    /// Nothing reachable from the tray or from a notification approves,
    /// dismisses or sends anything -- the read gate is the only route to an
    /// approval, and it lives behind the preview sheet.
    /// </remarks>
    private void OnTrayOpenRequested()
    {
        // Marshalled onto the UI thread: the tray's window procedure runs on
        // whichever thread pumped its message, which is not this one.
        _host.Dispatcher.TryEnqueue(() =>
        {
            BringForward();
        });
    }

    private void OnTrayReviewRequested()
    {
        _host.Dispatcher.TryEnqueue(() =>
        {
            ViewModel.ShowQueue();
            BringForward();
        });
    }

    private void OnTraySettingsRequested()
    {
        _host.Dispatcher.TryEnqueue(() =>
        {
            ShowSettingsPane();
            BringForward();
        });
    }

    private void OnTrayPauseRequested(PauseDuration duration)
    {
        _host.Dispatcher.TryEnqueue(async () => await ViewModel.PauseAsync(duration));
    }

    private void OnTrayResumeRequested()
    {
        _host.Dispatcher.TryEnqueue(async () => await ViewModel.ResumeAsync());
    }

    internal void BringForward()
    {
        AppWindow.Show();
        Activate();
    }

    internal void ReceiveActivation(string? invite)
    {
        if (_closed) return;
        BringForward();
        _redirectedInvite.Receive(invite);
        OfferNextRedirectedInvite();
    }

    private void OfferNextRedirectedInvite()
    {
        if (_closed)
        {
            return;
        }
        var decision = _redirectedInvite.Take(_activationReady, ViewModel.NeedsSessionRoots,
            _activationDaemonAvailable, _onboarding is not null);
        if (decision.Notice is string notice)
        {
            // Notices live on the queue pane, including a link opened from Settings.
            ViewModel.ShowQueue();
            ViewModel.ShowNotice(notice);
        }
        if (decision.Invite is not string invite)
        {
            return;
        }
        var onboarding = OpenOnboarding(OnboardingState.Default());
        onboarding.OfferInvite(invite);
        onboarding.Activate();
    }

    private OnboardingWindow OpenOnboarding(OnboardingState state)
    {
        if (_onboarding is not null) return _onboarding;
        var onboarding = new OnboardingWindow(_host, state);
        _onboarding = onboarding;
        onboarding.Closed += async (_, _) =>
        {
            await onboarding.CloseCompletion;
            if (ReferenceEquals(_onboarding, onboarding)) _onboarding = null;
            DispatcherQueue.TryEnqueue(OfferNextRedirectedInvite);
        };
        return onboarding;
    }

    private void OnTrayQuitRequested()
    {
        _host.Dispatcher.TryEnqueue(async () =>
        {
            BringForward();
            await ConfirmQuitAsync();
        });
    }

    /// <summary>
    /// Opens onboarding when this device has not finished it.
    /// </summary>
    /// <remarks>
    /// The gate is deliberately NOT status.logged_in. enroll succeeds on the
    /// Connect screen and flips logged_in there, three screens before
    /// consent is chosen, so resuming on it would drop a contributor who
    /// quit mid flow into this window carrying enroll's floor only scope
    /// default: silently narrower consent than the one they were in the
    /// middle of choosing. OnboardingState records the end of the flow, per
    /// tenant, and that is what is asked here.
    ///
    /// Both halves of the question are the daemon's to answer, so this runs
    /// after the first status read rather than in the constructor.
    /// </remarks>
    private async Task ShowOnboardingIfNeededAsync()
    {
        var state = OnboardingState.Default();

        DaemonResponse status = await _host
            .CallAsync(DaemonProtocol.Methods.Status)
            .ConfigureAwait(true);

        // No daemon means this process lost the race for the state
        // directory's lock, which is what happens when the app is already
        // running and a second copy is launched -- exactly what clicking an
        // invite link does, since the scheme handler starts a new process.
        //
        // Onboarding must NOT open here. Every call it made would fail, and
        // enroll failing shows the one fixed sentence the invite path has:
        // "This invite link is no longer valid." That sentence would be a
        // lie. The invite is fine; this copy of the app simply cannot reach
        // a daemon. Blaming the contributor's invite for our own state is
        // worse than saying nothing, so this says the true thing instead.
        if (status.IsError)
        {
            _activationDaemonAvailable = false;
            _activationReady = true;
            ViewModel.ReportAlreadyRunning(App.PendingInvite is not null);
            OfferNextRedirectedInvite();
            return;
        }

        _activationReady = true;
        _activationDaemonAvailable = true;
        string? tenantId = null;
        bool loggedIn = false;
        if (status.Result is JsonElement element)
        {
            if (element.TryGetProperty("tenant_id", out JsonElement tenant)
                && tenant.ValueKind == JsonValueKind.String)
            {
                tenantId = tenant.GetString();
            }

            loggedIn = element.TryGetProperty("logged_in", out JsonElement flag)
                       && flag.ValueKind == JsonValueKind.True;
        }

        if (loggedIn && state.IsComplete(tenantId))
        {
            return;
        }

        var onboarding = OpenOnboarding(state);
        if (App.PendingInvite is string invite)
        {
            onboarding.OfferInvite(invite);
        }

        onboarding.Activate();
    }

    private async void OnRefreshClick(object sender, RoutedEventArgs e)
    {
        await ViewModel.RefreshAsync();
    }

    private async void OnPauseForHour(object sender, RoutedEventArgs e)
    {
        await ViewModel.PauseAsync(PauseDuration.OneHour);
    }

    private async void OnPauseUntilTomorrow(object sender, RoutedEventArgs e)
    {
        await ViewModel.PauseAsync(PauseDuration.TomorrowMorning);
    }

    private async void OnPauseUntilResumed(object sender, RoutedEventArgs e)
    {
        await ViewModel.PauseAsync(PauseDuration.UntilResumed);
    }

    private async void OnResumeWatching(object sender, RoutedEventArgs e)
    {
        await ViewModel.ResumeAsync();
    }

    private async void OnShowQueue(object sender, RoutedEventArgs e)
    {
        if (await StartContributionsAsync()) ViewModel.ShowQueue();
    }

    /// <summary>
    /// Switches to History, creating the view the first time and keeping it
    /// thereafter.
    /// </summary>
    /// <remarks>
    /// Kept rather than rebuilt because the pane holds what a withdrawal
    /// actually did, per submission. <c>list_history</c> reports a record's
    /// status and never the tier a withdrawal resolved to, so a rebuilt pane
    /// would replace "this trace had already been included in a published
    /// export" with a bare chip -- the contract's first withdrawal rule,
    /// broken by a nav click rather than by any copy change.
    ///
    /// Created lazily rather than in the constructor because it makes three
    /// IPC calls as soon as it loads, and a contributor who never opens
    /// History should not pay for them at launch.
    /// </remarks>
    private async void OnShowHistory(object sender, RoutedEventArgs e)
    {
        if (!await StartContributionsAsync()) return;
        HistoryPane.Content ??= new HistoryView(_host);
        ViewModel.ShowHistory();
    }

    /// <summary>
    /// The health banner's action, for the two conditions that have one.
    /// </summary>
    /// <remarks>
    /// Both resolve on the same screen, which is why one handler serves both.
    /// <c>not-logged-in</c> is answered by the Connect step, and
    /// <c>near-ai-notice-not-acknowledged</c> by the privacy step, whose
    /// choice is the only thing in this app that calls
    /// <c>acknowledge_near_ai_notice</c> -- without that call the daemon
    /// refuses the filter forever and the contributor experiences unexplained
    /// paralysis, which is precisely the state this banner is reporting.
    ///
    /// Onboarding is opened directly rather than through
    /// <see cref="ShowOnboardingIfNeededAsync"/>: that method's job is to
    /// decide whether to interrupt someone at launch, and its "already
    /// complete, do nothing" answer is the right one there and the wrong one
    /// here. A contributor who has just clicked the banner's only button has
    /// asked for the screen, and returning them silently to the queue would
    /// make it the dead button this banner must never have.
    /// </remarks>
    /// <summary>
    /// "Not now" against the arming offer.
    /// </summary>
    /// <remarks>
    /// The daemon silences the offer for thirty days and remembers that
    /// across relaunches and across shells; this is not a local dismissal.
    /// The card is cleared here as well so it does not linger for a round
    /// trip, and the daemon's next answer will agree.
    /// </remarks>
    /// <summary>
    /// Declines the offer to answer model calls on this computer.
    /// </summary>
    /// <remarks>
    /// The daemon remembers the answer -- across relaunches and across shells
    /// -- so this is not a local dismissal. Declining writes the marker
    /// ALONE; writing the switch as false would make a refusal
    /// indistinguishable from a change.
    /// </remarks>
    private async void OnDeclinePrivateInference(object sender, RoutedEventArgs e)
    {
        await AnswerPrivateInferenceOfferAsync(accepted: false);
    }

    /// <summary>
    /// Accepts it. No confirmation sheet: this card IS the confirmation, and
    /// it already names what turning the switch on exposes.
    /// </summary>
    private async void OnAcceptPrivateInference(object sender, RoutedEventArgs e)
    {
        await AnswerPrivateInferenceOfferAsync(accepted: true);
    }

    /// <summary>
    /// Writes one answer and renders from the daemon's echo.
    /// </summary>
    /// <remarks>
    /// Never optimistic. An answer the daemon refused is not an answer, and
    /// the card staying up is the honest outcome: the next refresh asks the
    /// shared table again with what the daemon really holds.
    /// </remarks>
    private async Task AnswerPrivateInferenceOfferAsync(bool accepted)
    {
        try
        {
            DaemonResponse response = await _host
                .CallAsync(
                    DaemonProtocol.Methods.SetSettings,
                    PrivateInferenceSurface.SerializeOfferAnswer(accepted))
                .ConfigureAwait(true);
            var settings = response.ResultAs<DaemonSettingsSnapshot>();
            if (PrivateInferenceSurface.WriteConfirmed(accepted ? true : (bool?)null, settings))
            {
                ViewModel.SetPrivateInference(settings);
                ViewModel.ClearPrivateInferenceWriteFailure();
            }
            else
                ViewModel.ReportPrivateInferenceWriteFailure();
        }
        catch
        {
            // A transport failure must not escape an async-void click handler
            // and close the app. Keep the last confirmed answer; a later
            // settings refresh can confirm whether the write reached disk.
            System.Diagnostics.Trace.TraceWarning(nameof(AnswerPrivateInferenceOfferAsync));
            ViewModel.ReportPrivateInferenceWriteFailure();
        }
    }

    private async void OnDeclineArming(object sender, RoutedEventArgs e)
    {
        string projectId = ViewModel.ArmingOfferProjectId;
        if (projectId.Length == 0)
        {
            return;
        }

        ViewModel.SetArmingOffer(null);
        await _host
            .CallAsync(
                DaemonProtocol.Methods.DeclineArming,
                $$"""{"project_id":{{JsonSerializer.Serialize(projectId)}}}""")
            .ConfigureAwait(true);
    }

    /// <summary>
    /// Arms the offered project.
    /// </summary>
    /// <remarks>
    /// No confirmation sheet: this card IS the confirmation. It names the
    /// project, states the evidence, and asks the question outright, so a
    /// second dialog saying the same thing would be a step rather than a
    /// safeguard. Settings, where arming is picked from a list rather than
    /// offered, does confirm.
    /// </remarks>
    private async void OnAcceptArming(object sender, RoutedEventArgs e)
    {
        string projectId = ViewModel.ArmingOfferProjectId;
        if (projectId.Length == 0)
        {
            return;
        }

        ViewModel.SetArmingOffer(null);
        await _host
            .CallAsync(
                DaemonProtocol.Methods.SetProjectMode,
                $$"""{"project_id":{{JsonSerializer.Serialize(projectId)}},"mode":"auto_upload"}""")
            .ConfigureAwait(true);
        await ViewModel.RefreshAsync().ConfigureAwait(true);
    }

    private void OnHealthAction(object sender, RoutedEventArgs e)
    {
        var target = ViewModel.HealthDestination;
        if (target == HealthNavigationTarget.Waiting)
        {
            ViewModel.ShowQueue();
            return;
        }
        if (target == HealthNavigationTarget.None)
        {
            return;
        }
        // Raising a live flow must not reset an invite or consent decision.
        bool alreadyOpen = _onboarding is not null;
        var onboarding = OpenOnboarding(OnboardingState.Default());
        if (!alreadyOpen && target == HealthNavigationTarget.Connect)
        {
            onboarding.ViewModel.GetStarted();
        }

        onboarding.Activate();
    }

    /// <summary>
    /// Switches to Settings, creating the view the first time and keeping it
    /// thereafter.
    /// </summary>
    /// <remarks>
    /// Kept rather than rebuilt for the same reason History is, and for one
    /// more: the handle and bio boxes hold what the contributor has typed and
    /// not yet saved, and a rebuilt pane would discard that edit without
    /// saying so. It also carries the sentence reporting what the last claim
    /// or withdrawal did -- the only report of an outward-facing act this
    /// window gives -- and a nav click is not a reason to take it away.
    ///
    /// Created lazily rather than in the constructor because it reads the
    /// profile as soon as it loads, and a contributor who never opens
    /// Settings should not pay for that at launch.
    /// </remarks>
    private void OnShowInsights(object sender, RoutedEventArgs e) => ShowInsightsPane();

    private async void OnShowMissionDrafts(object sender, RoutedEventArgs e)
    {
        if (MissionDraftsPane.Content is not MissionDraftsView)
            MissionDraftsPane.Content = new MissionDraftsView(WinRT.Interop.WindowNative.GetWindowHandle(this));
        ViewModel.ShowMissionDrafts();
        await ((MissionDraftsView)MissionDraftsPane.Content).ActivateAsync();
    }

    private async void ShowInsightsPane()
    {
        if (InsightsPane.Content is not InsightsView)
        {
            var page = new InsightsView(WinRT.Interop.WindowNative.GetWindowHandle(this));
            page.ContributionSetupRequested += async (_, _) =>
            {
                if (await StartContributionsAsync()) ViewModel.ShowQueue();
            };
            InsightsPane.Content = page;
        }
        ViewModel.ShowInsights();
        await ((InsightsView)InsightsPane.Content).ActivateAsync();
    }

    private async void OnShowSettings(object sender, RoutedEventArgs e)
    {
        if (!await StartContributionsAsync()) return;
        ShowSettingsPane();
    }

    private void ShowSettingsPane()
    {
        if (SettingsPane.Content is null)
        {
            var settings = new SettingsView(_host);
            // Settings no longer holds the model-calls switch; it holds a
            // pointer at the destination that does, and this is what turns
            // that pointer into navigation.
            settings.OpenPrivateInferenceRequested += (_, _) => ShowPrivateInferencePane();
            SettingsPane.Content = settings;
        }

        ViewModel.ShowSettings();
    }

    /// <summary>
    /// Switches to the model-calls destination, creating the page the first
    /// time and keeping it thereafter.
    /// </summary>
    /// <remarks>
    /// Kept rather than rebuilt for the reason History and Settings are: it
    /// holds the daemon's last report about the listener, and a rebuilt page
    /// would replace the sentence naming a refusal with a blank until the
    /// next read landed -- which is the shape that reads as "nothing is
    /// wrong".
    ///
    /// Created lazily because it makes an IPC call as soon as it loads.
    /// </remarks>
    private async void OnShowPrivateInference(object sender, RoutedEventArgs e)
    {
        if (!await StartContributionsAsync()) return;
        ShowPrivateInferencePane();
    }

    private void ShowPrivateInferencePane()
    {
        PrivateInferencePane.Content ??= new PrivateInferenceView(_host);
        ViewModel.ShowPrivateInference();
    }

    // The rail's four destinations on Ctrl-1 through Ctrl-4, in rail order.
    // In-app only: these fire while this window has focus and take nothing
    // away from any other application. See the accelerators in the markup.

    private void OnInsightsAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        ShowInsightsPane();
    }

    private async void OnQueueAccelerator(
        KeyboardAccelerator sender,
        KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        if (!await StartContributionsAsync()) return;
        ViewModel.ShowQueue();
    }

    private async void OnHistoryAccelerator(
        KeyboardAccelerator sender,
        KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        if (!await StartContributionsAsync()) return;
        HistoryPane.Content ??= new HistoryView(_host);
        ViewModel.ShowHistory();
    }

    private async void OnPrivateInferenceAccelerator(
        KeyboardAccelerator sender,
        KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        if (!await StartContributionsAsync()) return;
        ShowPrivateInferencePane();
    }

    private async void OnSettingsAccelerator(
        KeyboardAccelerator sender,
        KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        if (!await StartContributionsAsync()) return;
        ShowSettingsPane();
    }

    /// <summary>
    /// Stops answering from the keyboard, or opens the destination.
    /// </summary>
    /// <remarks>
    /// Asymmetric, and it asks <see cref="PrivateInferenceTrayEntry.ActionFor"/>
    /// rather than deciding here -- the same table the tray row asks.
    ///
    /// This chord fires with nothing on screen. Turning answering OFF only
    /// reduces what this computer will answer, so it may act. Turning it ON
    /// opens a listener to everything running on this machine, charged to the
    /// contributor's accounts, and that must not happen with
    /// <c>offer_exposure</c> unread. Showing the page in the same breath as
    /// the write puts the sentence on screen simultaneously with the change,
    /// not before it, which is not consent.
    ///
    /// It also SETS rather than inverts. An inverted press against a stale
    /// cached position is an enable.
    /// </remarks>
    private async void OnTogglePrivateInferenceAccelerator(
        KeyboardAccelerator sender,
        KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        ShowPrivateInferencePane();
        if (PrivateInferencePane.Content is PrivateInferenceView page
            && PrivateInferenceTrayEntry.ActionFor(page.IsAnswering)
                == PrivateInferenceTrayAction.StopAnswering)
        {
            await page.TurnOffAsync();
        }
    }

    private void OnTrayPrivateInferenceRequested()
    {
        _host.Dispatcher.TryEnqueue(() =>
        {
            ShowPrivateInferencePane();
            BringForward();
        });
    }

    /// <summary>
    /// The tray's one write: stop answering model calls.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Off and not a flip. The tray cannot ask for the other direction -- see
    /// <see cref="TrayIcon.PrivateInferenceStopRequested"/> -- and this
    /// handler must not be the place that quietly restores it, so it writes
    /// the value rather than inverting whatever it finds. A flip here would
    /// turn a stale menu into an enable nobody asked for.
    /// </para>
    /// <para>
    /// The page is created and shown, not merely written to. Everything else
    /// the tray can do either opens a window or is reversible without a
    /// sentence; this one changes what this machine answers, and the only
    /// place the daemon's answer to it is rendered is the page.
    /// </para>
    /// </remarks>
    private void OnTrayPrivateInferenceStopRequested()
    {
        _host.Dispatcher.TryEnqueue(async () =>
        {
            ShowPrivateInferencePane();
            BringForward();
            if (PrivateInferencePane.Content is PrivateInferenceView page)
            {
                await page.TurnOffAsync();
            }
        });
    }

    /// <summary>
    /// Hands one settings read to the page and to the tray.
    /// </summary>
    private void OnPrivateInferenceSettingsChanged(DaemonSettingsSnapshot? settings)
    {
        if (PrivateInferencePane.Content is PrivateInferenceView page)
        {
            page.Fill(settings);
        }

        _tray.UpdatePrivateInference(PrivateInferenceTrayEntry.For(
            _privateInferenceCopy,
            PrivateInferenceState.From(settings?.PrivateInferenceReport),
            settings?.PrivateInferenceOn ?? false));
    }

    /// <summary>
    /// Opens the preview sheet for a row.
    /// </summary>
    /// <remarks>
    /// This is the route to approving something after reading it. Submit
    /// (<see cref="OnSubmitEntry"/>) is the other one now: the daemon builds
    /// and pins an envelope inside <c>approve</c> itself for anything
    /// unpreviewed, so a row submit no longer sends bytes nobody was shown.
    /// See docs/superpowers/specs/2026-08-20-one-click-submit-design.md.
    /// </remarks>
    private void OnLookInside(object sender, RoutedEventArgs e) => OpenPreview(EntryOf(sender));

    /// <summary>
    /// A tap anywhere on the card body: the same thing "Look inside" does.
    /// </summary>
    /// <remarks>
    /// <para>
    /// A second route to "Look inside", never a replacement for it. That
    /// button is the card's one accented control and "Submit" is secondary:
    /// one-click submit added AVAILABILITY, and accent styling is a
    /// RECOMMENDATION, which is to read first. What this adds is that the
    /// obvious gesture on a card does the obvious thing.
    /// </para>
    /// <para>
    /// The three footer buttons handle their own pointer input, so a WinUI
    /// Button does not raise Tapped on an ancestor. That is checked rather
    /// than assumed: routed events bubble, and being wrong about which ones
    /// stop here would silently turn "Not this one" into "Look inside" on the
    /// one screen where what a click means must never be ambiguous.
    /// </para>
    /// </remarks>
    private void OnCardTapped(object sender, TappedRoutedEventArgs e)
    {
        if (e.OriginalSource is DependencyObject source && IsInsideButton(source))
        {
            return;
        }

        OpenPreview(EntryOf(sender));
        e.Handled = true;
    }

    /// <summary>
    /// Whether <paramref name="source"/> is a button, or sits inside one.
    /// </summary>
    /// <remarks>
    /// Walks the visual tree rather than testing the element itself, because
    /// what raises the event is whichever piece of a button's template was
    /// under the pointer, never the button.
    /// </remarks>
    private static bool IsInsideButton(DependencyObject source)
    {
        for (DependencyObject? node = source; node is not null; node = VisualTreeHelper.GetParent(node))
        {
            if (node is ButtonBase)
            {
                return true;
            }
        }

        return false;
    }

    private void OpenPreview(QueueEntryViewModel? entry)
    {
        if (entry is null)
        {
            return;
        }

        var sheet = new PreviewWindow(_host, entry, ViewModel.LiveEntry);
        sheet.Decided += OnSheetDecided;

        // The sheet's gate reads the LIVE entry, but the properties it feeds
        // are pull-bound: without a nudge the footer keeps drawing what it
        // last read, and a session downgraded while the sheet is open would go
        // on offering Contribute for as long as the sheet stayed up.
        //
        // Pending is cleared and refilled on every refresh, so its
        // CollectionChanged is the queue's own signal that the rows have been
        // replaced. Unsubscribed on close: a sheet that has gone must not keep
        // the window alive through an event handler.
        void OnQueueChanged(object? _, NotifyCollectionChangedEventArgs __) =>
            sheet.QueueChanged();

        ViewModel.Pending.CollectionChanged += OnQueueChanged;
        sheet.Closed += (_, _) => ViewModel.Pending.CollectionChanged -= OnQueueChanged;

        sheet.Activate();
    }

    /// <summary>
    /// "Not this one" from the row: skips this session only.
    /// </summary>
    /// <remarks>
    /// Dismissing without a preview is deliberate and is not the inverse of
    /// the read gate. Declining to send something is safe in the direction
    /// that matters -- nothing leaves the machine -- so requiring a contributor
    /// to read a transcript before refusing it would only push them towards
    /// approving to make the row go away.
    /// </remarks>
    private async void OnNotThisOne(object sender, RoutedEventArgs e)
    {
        if (EntryOf(sender) is QueueEntryViewModel entry)
        {
            await ViewModel.DismissAsync(entry);
        }
    }

    /// <summary>
    /// "Submit" from the row: one click, no preview.
    /// </summary>
    /// <remarks>
    /// Every decision about what the daemon's response means -- the toast
    /// wording, whether Undo is offered -- is made in
    /// <see cref="MainViewModel.SubmitEntryAsync"/> and the interop assembly
    /// it calls into; this handler only routes the click to the entry it
    /// came from, the same pattern <see cref="OnNotThisOne"/> follows.
    /// </remarks>
    private async void OnSubmitEntry(object sender, RoutedEventArgs e)
    {
        if (EntryOf(sender) is QueueEntryViewModel entry)
        {
            await ViewModel.SubmitEntryAsync(entry);
        }
    }

    /// <summary>
    /// "Submit all" from a project's group header: one <c>approve</c> call
    /// for every pending entry in that project, not a loop over its rows.
    /// </summary>
    /// <remarks>
    /// Sends <see cref="QueueGroupViewModel.ProjectId"/>, the id
    /// <c>entry_value</c> publishes, never <see cref="QueueGroupViewModel.ProjectLabel"/>,
    /// which is display text only. Shown only on a multi-entry group (see
    /// <see cref="QueueGroupViewModel.ShowSubmitAll"/>), so this handler does
    /// not need to guard against being reachable from a single-entry one.
    /// <see cref="MainViewModel.SubmitProjectAsync"/> does everything else:
    /// building the request, decoding the response, arming Undo.
    /// </remarks>
    private async void OnSubmitAll(object sender, RoutedEventArgs e)
    {
        if (GroupOf(sender) is QueueGroupViewModel group)
        {
            await ViewModel.SubmitProjectAsync(group.ProjectId);
        }
    }

    /// <summary>
    /// "Submit all as..." from a project's group header: the same
    /// <c>approve</c> call "Submit all" makes, carrying the verdict the
    /// contributor picked from the menu.
    /// </summary>
    /// <remarks>
    /// One handler per verdict rather than one reading a Tag, for the same
    /// reason the preview sheet has three: which of the three was chosen is
    /// the whole content of the event, and every entry this call covers is
    /// recorded with it. The plain "Submit all" beside this is untouched and
    /// still sends no <c>outcome</c> -- an unanswered bulk submit stays one
    /// click.
    /// </remarks>
    private async void OnSubmitAllWorked(object sender, RoutedEventArgs e) =>
        await SubmitAllAsAsync(sender, Verdict.Worked);

    private async void OnSubmitAllPartly(object sender, RoutedEventArgs e) =>
        await SubmitAllAsAsync(sender, Verdict.Partly);

    private async void OnSubmitAllFailed(object sender, RoutedEventArgs e) =>
        await SubmitAllAsAsync(sender, Verdict.Failed);

    private async Task SubmitAllAsAsync(object sender, string outcome)
    {
        if (GroupOf(sender) is QueueGroupViewModel group)
        {
            await ViewModel.SubmitProjectAsync(group.ProjectId, outcome);
        }
    }

    /// <summary>
    /// "Ignore project" from a project's group header: confirms, then hands
    /// off to <see cref="MainViewModel.IgnoreProjectAsync"/>.
    /// </summary>
    /// <remarks>
    /// The confirmation is built here, from <see cref="ProjectIgnoreCopy"/>,
    /// the same word-for-word text macOS and GTK show -- and shown through
    /// <see cref="DialogGuard"/> rather than a raw ShowAsync, because this is
    /// now a third caller into the one <see cref="XamlRoot"/> this window
    /// owns, alongside <see cref="Controls.WithdrawDialog"/> and
    /// <see cref="Controls.GoPublicDialog"/>.
    /// </remarks>
    private async void OnIgnoreProject(object sender, RoutedEventArgs e)
    {
        if (GroupOf(sender) is not QueueGroupViewModel group)
        {
            return;
        }

        var dialog = new ContentDialog
        {
            XamlRoot = Content.XamlRoot,
            Title = ProjectIgnoreCopy.ConfirmationTitle(group.ProjectLabel),
            Content = ProjectIgnoreCopy.ConfirmationBody(group.ProjectLabel, group.PendingCount),
            PrimaryButtonText = ProjectIgnoreCopy.ButtonLabel,
            CloseButtonText = "Cancel",

            // Keeping the project offered is what Enter and Escape both do.
            // This purges every one of its waiting sessions server-side; it
            // does not get to be the thing a stray keypress commits.
            DefaultButton = ContentDialogButton.Close,
        };

        // Three outcomes, not two. Primary is a yes; Close (and Escape) is a
        // no and needs nothing said, because the person who cancelled knows
        // they cancelled. None means the dialog never appeared -- see
        // DialogGuard -- and folding that into the cancel branch leaves a
        // contributor who pressed a button with no dialog, no change, and no
        // word about either. The quit path can fold them, because there the
        // safe reading is "do not quit" and the window is still there to
        // press again; here the only feedback surface is Notice.
        ContentDialogResult outcome = await DialogGuard.ShowOnceAsync(dialog);
        if (outcome == ContentDialogResult.None)
        {
            ViewModel.ShowNotice("That couldn't be asked just now. Nothing has changed.");
            return;
        }

        if (outcome != ContentDialogResult.Primary)
        {
            return;
        }

        await ViewModel.IgnoreProjectAsync(
            group.ProjectId,
            group.ProjectLabel,
            group.PendingCount);
    }

    /// <summary>
    /// Which queue row a click came from.
    /// </summary>
    /// <remarks>
    /// Tag first, DataContext second. Both are set by the row template, and
    /// the pair is deliberate rather than defensive habit: the entry a click
    /// refers to is the one thing on this card that must never be ambiguous,
    /// because acting on the wrong row means previewing one session and
    /// refusing another.
    /// </remarks>
    private static QueueEntryViewModel? EntryOf(object sender) =>
        sender is FrameworkElement element
            ? element.Tag as QueueEntryViewModel ?? element.DataContext as QueueEntryViewModel
            : null;

    /// <summary>
    /// Which project's group header a "Submit all" click came from. Same
    /// Tag-first, DataContext-second pattern as <see cref="EntryOf"/>, for
    /// the same reason: which project a click means must never be ambiguous.
    /// </summary>
    private static QueueGroupViewModel? GroupOf(object sender) =>
        sender is FrameworkElement element
            ? element.Tag as QueueGroupViewModel ?? element.DataContext as QueueGroupViewModel
            : null;

    /// <summary>
    /// A folder row: shows that project's sessions.
    /// </summary>
    /// <remarks>
    /// The location itself lives on the view model and is re-resolved through
    /// <see cref="QueueNavigation.Resolve"/> on every queue snapshot, so a
    /// folder that empties underneath the contributor returns them to the
    /// list rather than leaving them on an empty pane. This handler only
    /// routes the click to the folder it came from, the same Tag-first
    /// pattern every other queue handler follows.
    /// </remarks>
    private void OnOpenFolder(object sender, RoutedEventArgs e)
    {
        if (GroupOf(sender) is QueueGroupViewModel group)
        {
            ViewModel.OpenFolder(group.ProjectId);
        }
    }

    /// <summary>Back to the folder list.</summary>
    private void OnCloseFolder(object sender, RoutedEventArgs e) => ViewModel.CloseFolder();

    /// <summary>
    /// The detail pane's scroll, watched for the same settle the folder
    /// list's is.
    /// </summary>
    /// <remarks>
    /// Declared in the markup rather than found by walking a template, which
    /// is what <see cref="OnQueueListViewLoaded"/> has to do for a ListView:
    /// this ScrollViewer is an element of the page, so it can say who handles
    /// its own event. It matters because the session rows live here now, so
    /// this is the scroller whose settling changes what is on screen.
    /// </remarks>
    private void OnQueueDetailScrolled(object sender, ScrollViewerViewChangedEventArgs e) =>
        OnQueueScrollViewChanged(sender, e);

    /// <summary>
    /// Finds the ScrollViewer once <see cref="QueueListView"/>'s template is
    /// realized, so its <c>ViewChanged</c> can be watched for a scroll
    /// settling. Runs at most once: a second Loaded (a theme change, a
    /// re-parent) would otherwise double-subscribe the handler.
    /// </summary>
    private void OnQueueListViewLoaded(object sender, RoutedEventArgs e)
    {
        if (_queueScrollViewer is not null)
        {
            return;
        }

        _queueScrollViewer = FindDescendant<ScrollViewer>(QueueListView);
        if (_queueScrollViewer is not null)
        {
            _queueScrollViewer.ViewChanged += OnQueueScrollViewChanged;
        }
    }

    /// <summary>
    /// One signal a settle may have happened. <c>IsIntermediate</c> is true
    /// for every frame of an active scroll or manipulation and false for the
    /// final event once it stops, which is the scroll-settled signal the
    /// design spec asks <c>preview_visible</c> to follow -- but this still
    /// routes through the debounce timer rather than recomputing inline,
    /// because a fling can still produce several "final" events in quick
    /// succession as inertia settles in stages.
    /// </summary>
    private void OnQueueScrollViewChanged(object? sender, ScrollViewerViewChangedEventArgs e)
    {
        if (e.IsIntermediate)
        {
            return;
        }

        ScheduleVisibilityRecompute();
    }

    /// <summary>
    /// Records that a session row has been realized (freshly created, or
    /// recycled and rebound to a different entry) by ITS OWN project's
    /// ItemsRepeater.
    /// </summary>
    /// <remarks>
    /// This fires once per project's ItemsRepeater instance, for whichever
    /// rows that instance currently has windowed into view -- not once for
    /// every entry in that project, which is exactly the distinction that
    /// makes this the right signal now that the inner list virtualizes
    /// (see the doc comment on the queue ListView in MainWindow.xaml).
    /// <paramref name="args"/>'s index is read against the SAME
    /// ItemsRepeater's own <c>ItemsSourceView</c> rather than the element's
    /// <c>DataContext</c>, so this does not depend on whatever WinUI does or
    /// does not set on a compiled x:Bind template's root element.
    /// </remarks>
    private void OnSessionElementPrepared(ItemsRepeater sender, ItemsRepeaterElementPreparedEventArgs args)
    {
        if (sender.ItemsSourceView?.GetAt(args.Index) is QueueEntryViewModel entry)
        {
            _visibleEntryIdsByElement[args.Element] = entry.EntryId;
        }

        ScheduleVisibilityRecompute();
    }

    /// <summary>
    /// The mirror of <see cref="OnSessionElementPrepared"/>: a row is being
    /// recycled away, whether because it scrolled out of its own project's
    /// window or because its project's ItemsRepeater is being torn down
    /// with the rest of the outer container. Removing by element identity
    /// keeps <see cref="_visibleEntryIdsByElement"/> correct regardless of
    /// which reason applies.
    /// </summary>
    private void OnSessionElementClearing(ItemsRepeater sender, ItemsRepeaterElementClearingEventArgs args)
    {
        _visibleEntryIdsByElement.Remove(args.Element);
        ScheduleVisibilityRecompute();
    }

    private void ScheduleVisibilityRecompute()
    {
        _visibilityDebounceTimer.Stop();
        _visibilityDebounceTimer.Start();
    }

    /// <summary>
    /// Tells the view model which entries are on screen right now, from
    /// whichever session rows are currently realized across every project's
    /// ItemsRepeater.
    /// </summary>
    /// <remarks>
    /// Still not a pixel-precise viewport test: a realized row carries
    /// WinUI's own small look-ahead buffer above and below the actual
    /// visible area, so the reported set can be a little wider than what a
    /// contributor's eye actually sees. That remains the safe direction --
    /// the design spec is explicit that visibility decides preview build
    /// ORDER and never membership -- but it is now a SMALL superset bounded
    /// by that look-ahead buffer, typically low tens of rows, rather than
    /// one that could include an entire large project's queue. That bound
    /// is the point of reading the inner ItemsRepeater's realization
    /// instead of the outer ListView's: see the doc comment on
    /// <see cref="_visibleEntryIdsByElement"/>.
    /// </remarks>
    private void RecomputeVisiblePreviews()
    {
        _ = ViewModel.SetVisiblePreviewsAsync(new List<string>(_visibleEntryIdsByElement.Values));
    }

    /// <summary>
    /// Walks the visual tree for the first descendant of type
    /// <typeparamref name="T"/>. WinUI does not expose a ListView's own
    /// ScrollViewer as a named property; this is the standard way to reach
    /// it once the control's template has been applied.
    /// </summary>
    private static T? FindDescendant<T>(DependencyObject root)
        where T : DependencyObject
    {
        int count = VisualTreeHelper.GetChildrenCount(root);
        for (int i = 0; i < count; i++)
        {
            DependencyObject child = VisualTreeHelper.GetChild(root, i);
            if (child is T typed)
            {
                return typed;
            }

            if (FindDescendant<T>(child) is T found)
            {
                return found;
            }
        }

        return null;
    }

    private async void OnSheetDecided(QueueEntryViewModel entry, PreviewDecision decision)
    {
        await ViewModel.OnDecidedAsync(entry, decision);
    }

    private async void OnUndo(object sender, RoutedEventArgs e)
    {
        await ViewModel.UndoAsync();
    }

    private void OnLetItSend(object sender, RoutedEventArgs e) => ViewModel.DismissUndo();

    /// <summary>
    /// Hands the update to Windows. Fire-and-forget in the same sense
    /// OnClosed is: the click handler cannot be awaited, and on the success
    /// path this process is terminated part-way through the call anyway.
    /// </summary>
    private async void OnApplyUpdateClick(object sender, RoutedEventArgs e)
    {
        await ViewModel.ApplyUpdateAsync();
    }

    /// <summary>
    /// Tears the daemon down on close.
    ///
    /// Fire-and-forget is unavoidable here -- Closed is not awaitable -- but
    /// the work it starts is bounded: DaemonHost.DisposeAsync waits on the
    /// ABI's own drain and unsubscribe timeouts and then leaks rather than
    /// blocking forever, so this cannot wedge process exit.
    /// </summary>
    private async void OnClosed(object sender, WindowEventArgs args)
    {
        _closed = true;
        (InsightsPane.Content as InsightsView)?.Dispose();
        (MissionDraftsPane.Content as MissionDraftsView)?.Dispose();
        _activationReady = false;
        _redirectedInvite.Clear();
        // Before the daemon teardown, and synchronously: an icon left in the
        // notification area after the process exits is a ghost the shell only
        // reaps when someone hovers over it, and it would claim a watcher
        // that is no longer running.
        _tray.Dispose();

        await _host.DisposeAsync();
    }
}
