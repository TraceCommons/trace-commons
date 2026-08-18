using System;
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

        Closed += OnClosed;
        Activated += OnFirstActivated;
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
        await ShowOnboardingIfNeededAsync();
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
        await _host.DisposeAsync();
    }
}
