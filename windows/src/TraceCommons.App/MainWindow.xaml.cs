using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.UI;
using Microsoft.UI.Xaml;
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
    /// The interruption budget. See <see cref="DigestCadence"/> for why the
    /// shell gates this a second time when the daemon already does.
    /// </summary>
    private readonly DigestCadence _digestCadence = new();

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
        ViewModel = new MainViewModel(_host);

        // The tray reflects daemon state and never drives it: it re-reads
        // status on the same events the queue does rather than being told
        // what to show by the view model, so the two cannot drift.
        _host.QueueChanged += OnTrayWorthyChange;
        _host.StatusChanged += OnTrayWorthyChange;
        _host.DigestDue += OnDigestDue;
        _tray.OpenRequested += OnTrayOpenRequested;
        _tray.QuitRequested += OnTrayQuitRequested;

        AppWindow.Closing += OnAppWindowClosing;
        Closed += OnClosed;
        Activated += OnFirstActivated;
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

    /// <summary>
    /// Intercepts the close so the consequence can be stated first.
    /// </summary>
    /// <remarks>
    /// On the window's own close button as well as on the tray's Quit. Once
    /// the app has a tray icon, "I closed the window" and "I stopped
    /// contributing" become different acts on every other platform -- and on
    /// this one they are still the same act, because the watcher is this
    /// process. A contributor must not have to guess which it was.
    /// </remarks>
    private async void OnAppWindowClosing(
        Microsoft.UI.Windowing.AppWindow sender,
        Microsoft.UI.Windowing.AppWindowClosingEventArgs args)
    {
        if (_quitConfirmed)
        {
            return;
        }

        // Cancelled first and re-closed after: AppWindow.Closing cannot be
        // awaited, so the only way to ask a question is to refuse this close
        // and start another one from the answer.
        args.Cancel = true;

        if (_quitDialogOpen)
        {
            return;
        }

        var dialog = new Microsoft.UI.Xaml.Controls.ContentDialog
        {
            XamlRoot = Content.XamlRoot,
            Title = "Quit Trace Commons?",
            Content = QuitBody,
            PrimaryButtonText = "Quit",
            CloseButtonText = "Cancel",
            DefaultButton = Microsoft.UI.Xaml.Controls.ContentDialogButton.Close,
        };

        Microsoft.UI.Xaml.Controls.ContentDialogResult result;

        _quitDialogOpen = true;
        try
        {
            result = await dialog.ShowAsync();
        }
        catch (Exception)
        {
            // WinUI allows one ContentDialog per XamlRoot, and this window now
            // has two other callers -- WithdrawDialog, from History, and
            // GoPublicDialog, from Settings. If either is open, ShowAsync
            // throws, and this handler is `async void`, so the throw would
            // take the process down.
            //
            // Catching is the whole fix, and there is no second decision to
            // make here: `args.Cancel = true` ran synchronously above, before
            // the first await, so WinUI has already honoured the cancellation
            // by the time this throw is possible. The close is refused
            // whatever happens next; the only question was whether the
            // process survived to be closed again.
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
            // QUEUE pane, while the two dialogs that can land us here are
            // reachable only from History and Settings -- so the pane
            // carrying the message would never be the pane on screen. The
            // health banner is above all three panes and would be seen, but
            // it is single-writer over the daemon's own health label, and
            // every sentence in it states what happened to the data; a
            // transient UI collision is neither, and an unrecognised label
            // there renders "Contributions are on hold.", which would be
            // false in the direction that makes people quit.
            return;
        }
        finally
        {
            _quitDialogOpen = false;
        }

        if (result == Microsoft.UI.Xaml.Controls.ContentDialogResult.Primary)
        {
            _quitConfirmed = true;
            Close();
        }
    }

    public MainViewModel ViewModel { get; }

    /// <summary>
    /// Starts the daemon on first activation rather than in the constructor:
    /// the window should be on screen before a multi-second first filesystem
    /// scan begins, so a large session history looks like loading rather than
    /// like a failure to launch.
    /// </summary>
    private async void OnFirstActivated(object sender, WindowActivatedEventArgs args)
    {
        Activated -= OnFirstActivated;
        await ViewModel.InitializeAsync();
        await RefreshTrayAsync();
        await ShowOnboardingIfNeededAsync();
    }

    /// <summary>
    /// Re-reads the one status object the tray needs and hands it over.
    /// </summary>
    /// <remarks>
    /// <c>status</c> is described in <c>ipc.rs</c> as "the tray's whole world
    /// in one object", and this takes it at its word rather than assembling
    /// the same three facts from the queue the window happens to be showing.
    /// An error frame leaves the tray as it was: a momentarily stale
    /// indicator is better than one that flips to "nothing waiting" because a
    /// call failed.
    /// </remarks>
    private async Task RefreshTrayAsync()
    {
        if (!_tray.IsPresent)
        {
            return;
        }

        DaemonResponse response = await _host
            .CallAsync(DaemonProtocol.Methods.Status)
            .ConfigureAwait(true);

        if (response.ResultAs<DaemonStatus>() is not DaemonStatus status)
        {
            return;
        }

        _tray.Update(status.QueueDepth, status.Paused, status.IsHealthy);
    }

    private async void OnTrayWorthyChange()
    {
        await RefreshTrayAsync();
    }

    /// <summary>
    /// The 4-hour digest.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Two gates stand between a <c>digest_due</c> event and an interruption,
    /// and both must pass: the daemon's, which is the shared policy every
    /// shell obeys, and <see cref="DigestCadence"/>, which is this process's
    /// own backstop. Neither can cause a notification; each can only suppress
    /// one. That is what keeps the onboarding screen's promise -- at most one
    /// notification every 4 hours, and none at all when nothing is waiting --
    /// literally true rather than approximately.
    /// </para>
    /// <para>
    /// Project labels come from the queue the window already holds, which the
    /// daemon has already reduced from paths to labels. No path, and no line
    /// of transcript, reaches a notification.
    /// </para>
    /// </remarks>
    private void OnDigestDue(int pending)
    {
        if (!_digestCadence.TryClaim(pending, DateTimeOffset.UtcNow))
        {
            return;
        }

        var labels = new List<string>();
        foreach (QueueEntryViewModel entry in ViewModel.Pending)
        {
            if (entry.ProjectLabel.Length > 0 && !labels.Contains(entry.ProjectLabel))
            {
                labels.Add(entry.ProjectLabel);
            }
        }

        labels.Sort(StringComparer.Ordinal);
        _tray.ShowDigest(DigestText.Body(pending, labels));
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
            AppWindow.Show();
            Activate();
        });
    }

    private void OnTrayQuitRequested()
    {
        _host.Dispatcher.TryEnqueue(() =>
        {
            Activate();
            Close();
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
            ViewModel.ReportAlreadyRunning(App.PendingInvite is not null);
            return;
        }

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

        var onboarding = new OnboardingWindow(_host, state);
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

    private void OnShowQueue(object sender, RoutedEventArgs e) => ViewModel.ShowQueue();

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
    private void OnShowHistory(object sender, RoutedEventArgs e)
    {
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
    private void OnHealthAction(object sender, RoutedEventArgs e)
    {
        var onboarding = new OnboardingWindow(_host, OnboardingState.Default());
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
    private void OnShowSettings(object sender, RoutedEventArgs e)
    {
        SettingsPane.Content ??= new SettingsView(_host);
        ViewModel.ShowSettings();
    }

    /// <summary>
    /// Opens the preview sheet for a row.
    /// </summary>
    /// <remarks>
    /// This is the only route to approving anything. The row itself carries no
    /// Contribute button and never will: approving from the row is approving
    /// without looking, and an approval has to cover exactly the bytes the
    /// contributor was shown.
    /// </remarks>
    private void OnLookInside(object sender, RoutedEventArgs e)
    {
        if (EntryOf(sender) is not QueueEntryViewModel entry)
        {
            return;
        }

        var sheet = new PreviewWindow(_host, entry);
        sheet.Decided += OnSheetDecided;
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
    /// Tears the daemon down on close.
    ///
    /// Fire-and-forget is unavoidable here -- Closed is not awaitable -- but
    /// the work it starts is bounded: DaemonHost.DisposeAsync waits on the
    /// ABI's own drain and unsubscribe timeouts and then leaks rather than
    /// blocking forever, so this cannot wedge process exit.
    /// </summary>
    private async void OnClosed(object sender, WindowEventArgs args)
    {
        // Before the daemon teardown, and synchronously: an icon left in the
        // notification area after the process exits is a ghost the shell only
        // reaps when someone hovers over it, and it would claim a watcher
        // that is no longer running.
        _tray.Dispose();

        await _host.DisposeAsync();
    }
}
