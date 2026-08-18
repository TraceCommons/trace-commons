using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Runtime.CompilerServices;
using System.Globalization;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.UI.Dispatching;
using TraceCommons.Interop;

namespace TraceCommons.App.ViewModels;

/// <summary>
/// The main window's state: the pending queue, a status line, and the refresh
/// path that keeps them current.
///
/// Every member here is UI-thread-affine. <see cref="DaemonHost"/> guarantees
/// that by hopping before it raises anything, so nothing in this class needs
/// its own synchronization.
/// </summary>
public sealed class MainViewModel : INotifyPropertyChanged
{
    /// <summary>
    /// The undo bar's body, from the Linux shell word for word.
    ///
    /// It promises exactly two things and no more: the send happens on the
    /// watcher's next sweep, and undo works until that sweep starts. Neither
    /// sentence claims this window can see the send land, because it cannot.
    /// </summary>
    public const string UndoBody =
        "The watcher sends approved sessions on its next sweep. Undo works until the sweep "
        + "starts, and says so plainly if it is already too late.";

    /// <summary>
    /// What is said when the daemon granted no hold. There is nothing to
    /// undo, so nothing offers to.
    /// </summary>
    public const string ApprovedNoUndo = "Approved. It goes out on the next pass.";

    private readonly DaemonHost _host;
    private readonly SemaphoreSlim _refreshGate = new(1, 1);
    private readonly DispatcherQueueTimer _undoTick;
    private string _statusText = "Starting…";
    private bool _isBusy;
    private string _notice = string.Empty;
    private ApprovalHold? _undoHold;
    private string _undoEntryId = string.Empty;
    private string _undoProjectLabel = string.Empty;
    private MainPane _pane = MainPane.Queue;
    private HealthCopy? _health;
    private HistoryRollup _rollup = new();

    public MainViewModel(DaemonHost host)
    {
        _host = host ?? throw new ArgumentNullException(nameof(host));
        _host.QueueChanged += OnQueueChanged;
        _host.StatusChanged += OnStatusChanged;
        _host.Lagged += OnLagged;

        // One tick per second, only while an undo is live. It moves the
        // remaining count and retires the bar when the daemon's hold runs
        // out; it does not rebuild the bar, so a pointer already resting on
        // Undo does not have the button pulled out from under it.
        _undoTick = _host.Dispatcher.CreateTimer();
        _undoTick.Interval = TimeSpan.FromSeconds(1);
        _undoTick.Tick += (_, _) => OnUndoTick();
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    /// <summary>The pending queue, newest state as the daemon reports it.</summary>
    public ObservableCollection<QueueEntryViewModel> Pending { get; } = new();

    /// <summary>
    /// Which of the rail's destinations is showing.
    /// </summary>
    /// <remarks>
    /// One field and a derived boolean per destination, rather than one
    /// boolean per destination kept in step by hand. Every pane binds its
    /// Visibility to one of these directly -- which is why they stay booleans
    /// and the enum stays private -- and with three of them the invariant that
    /// exactly one is true is worth having the compiler hold rather than the
    /// setters. The queue is what opens, because the queue is what has
    /// something waiting on the contributor.
    /// </remarks>
    private enum MainPane
    {
        Queue,
        History,
        Settings,
    }

    public bool ShowingQueue => _pane == MainPane.Queue;

    public bool ShowingHistory => _pane == MainPane.History;

    public bool ShowingSettings => _pane == MainPane.Settings;

    public void ShowQueue() => SetPane(MainPane.Queue);

    public void ShowHistory() => SetPane(MainPane.History);

    public void ShowSettings() => SetPane(MainPane.Settings);

    private void SetPane(MainPane pane)
    {
        if (_pane == pane)
        {
            return;
        }

        _pane = pane;

        // All three are raised on every change rather than only the two that
        // moved: the rail's selection bars and the panes both bind to these,
        // and a destination left un-raised is a rail row that stays lit for a
        // pane that is no longer on screen.
        Raise(nameof(ShowingQueue));
        Raise(nameof(ShowingHistory));
        Raise(nameof(ShowingSettings));
    }

    // --- The health banner -------------------------------------------------
    //
    // Rendered from status.health.last_error_label and nothing else.
    //
    // The daemon owns the precedence order between conditions
    // (daemon::health::precedence: not-logged-in outranks the near-AI notice,
    // which outranks the self-test failure, and so on), and it sends exactly
    // one already-resolved label. A client that reconstructed that order would
    // eventually disagree with the daemon, and therefore with the tray, about
    // what is wrong -- so this stores whichever label arrived and hands it
    // straight to HealthCopy without ranking, merging or synthesising one. The
    // Linux shell's render_health carries the same note for the same reason.

    /// <summary>Whether anything is holding contributions up.</summary>
    public bool HasHealthBanner => _health is not null;

    public string HealthTitle => _health?.Title ?? string.Empty;

    public string HealthDetail => _health?.Detail ?? string.Empty;

    /// <summary>
    /// Whether this condition has an action worth offering.
    /// </summary>
    /// <remarks>
    /// Only two labels get one. The rest clear on their own, and a button that
    /// cannot change the condition it sits beside teaches a contributor that
    /// the buttons in this app do nothing -- a lesson they would then apply to
    /// Undo, which is the one control here that must be believed.
    /// </remarks>
    public bool HasHealthAction => _health?.ActionLabel is not null;

    public string HealthActionLabel => _health?.ActionLabel ?? string.Empty;

    // --- The week band -----------------------------------------------------
    //
    // Backed by history_rollup: counters the daemon already holds, and the
    // same read History makes. The queue asks for it in its own refresh rather
    // than taking it from the History screen, so the band is filled whether or
    // not History has ever been opened -- History's view is built lazily on
    // first nav and would otherwise leave this blank until someone clicked it.
    // App::refresh in the Linux shell makes the same call for the same reason.

    public string ThisWeekLabel => WeekBandCopy.ThisWeek;

    public string ContributedLabel => WeekBandCopy.Contributed;

    public string HeldLabel => WeekBandCopy.Held;

    public string InTheCommonsLabel => WeekBandCopy.InTheCommons;

    // Formatted to strings here rather than bound as ints, for the reason
    // HistoryViewModel records: x:Bind is strongly typed and performs no
    // implicit ToString for TextBlock.Text, so an int bound straight to a
    // figure is a compile error on Windows and nowhere else.
    public string ContributedCountText =>
        _rollup.Week.Submitted.ToString(CultureInfo.CurrentCulture);

    public string HeldCountText =>
        _rollup.Week.Quarantined.ToString(CultureInfo.CurrentCulture);

    /// <summary>
    /// In the commons: all time, not this week.
    /// </summary>
    /// <remarks>
    /// This one figure is deliberately not a weekly slice. "In the commons" is
    /// a standing total, and slicing it by week would read as the commons
    /// shrinking every Monday -- untrue, and discouraging in exactly the place
    /// a contributor looks for evidence that their work went somewhere. The
    /// Linux shell takes all_time here and says the same thing.
    /// </remarks>
    public string InTheCommonsCountText =>
        _rollup.AllTime.Accepted.ToString(CultureInfo.CurrentCulture);

    /// <summary>
    /// A short, human-readable status line. Fixed labels only -- everything
    /// the daemon hands us is already a label rather than a path or a token,
    /// and nothing here should be the first place that stops being true.
    /// </summary>
    public string StatusText
    {
        get => _statusText;
        private set => Set(ref _statusText, value);
    }

    public bool IsBusy
    {
        get => _isBusy;
        private set
        {
            if (_isBusy == value)
            {
                return;
            }

            _isBusy = value;
            Raise(nameof(IsBusy));

            // Raised explicitly rather than bound through a value converter in
            // XAML. One inverted bool does not justify a converter class, and a
            // converter would also have to be registered in App.xaml resources
            // to be reachable from a DataTemplate.
            Raise(nameof(IsNotBusy));
        }
    }

    /// <summary>The inverse of <see cref="IsBusy"/>, for enabling controls.</summary>
    public bool IsNotBusy => !_isBusy;

    /// <summary>True when there is nothing pending, for an empty-state view.</summary>
    public bool IsEmpty => Pending.Count == 0;

    /// <summary>
    /// A one-line result of the last decision, for the cases with no undo to
    /// offer: an approval the daemon held for no time at all, or one it
    /// refused. Always a fixed sentence.
    /// </summary>
    public string Notice
    {
        get => _notice;
        private set
        {
            if (Set(ref _notice, value))
            {
                Raise(nameof(HasNotice));
            }
        }
    }

    public bool HasNotice => _notice.Length > 0;

    /// <summary>
    /// Whether an approval can still be recalled.
    ///
    /// The five-second undo the shared spec asks for is trivially cheap and it
    /// converts a misclick from permanent into a non-event. It is counted
    /// against the DAEMON'S hold deadline rather than a timer invented here:
    /// a bar outliving the hold would offer a recall that cannot work, and one
    /// retiring early would take away a recall that still would.
    /// </summary>
    public bool HasUndo => _undoHold is not null;

    /// <summary>"Approved trace-commons-server. Still on this machine."</summary>
    public string UndoHeadline =>
        _undoProjectLabel.Length == 0
            ? string.Empty
            : $"Approved {_undoProjectLabel}. Still on this machine.";

    /// <summary>"Undo (4)" -- the spec's countdown, on the daemon's clock.</summary>
    public string UndoButtonText =>
        _undoHold is null
            ? "Undo"
            : string.Format(
                CultureInfo.CurrentCulture,
                "Undo ({0})",
                _undoHold.RemainingSeconds(DateTimeOffset.UtcNow));

    /// <summary>
    /// The other half of the pair. Not "Dismiss": what this button does is let
    /// the send happen, and it should say so.
    /// </summary>
    public const string LetItSend = "Let it send";

    /// <summary>
    /// Records what the preview sheet decided.
    /// </summary>
    /// <remarks>
    /// The sheet performs the decision -- it is the only surface that may,
    /// because it is the only one behind the read gate -- and hands the result
    /// here so recovery lands on the screen the contributor is looking at
    /// rather than behind a sheet that has already closed.
    /// </remarks>
    public async Task OnDecidedAsync(QueueEntryViewModel entry, PreviewDecision decision)
    {
        ArgumentNullException.ThrowIfNull(entry);
        ArgumentNullException.ThrowIfNull(decision);

        ClearUndo();

        switch (decision.Outcome)
        {
            case PreviewOutcome.Approved when decision.Hold is { } hold
                                              && hold.IsLive(DateTimeOffset.UtcNow):
                _undoHold = hold;
                _undoEntryId = entry.EntryId;
                _undoProjectLabel = entry.ProjectLabel;
                Notice = string.Empty;
                RaiseUndo();
                _undoTick.Start();
                break;

            case PreviewOutcome.Approved:
                // No hold, or one that had already expired by the time the
                // response arrived. Saying so is the honest option; a button
                // that would be refused is not.
                Notice = ApprovedNoUndo;
                break;

            case PreviewOutcome.Failed:
                Notice = decision.Message ?? string.Empty;
                break;

            case PreviewOutcome.Dismissed:
                Notice = string.Empty;
                break;
        }

        await RefreshAsync().ConfigureAwait(true);
    }

    /// <summary>
    /// "Not this one" from a queue row: refuses this session and leaves the
    /// project being offered.
    /// </summary>
    /// <remarks>
    /// Reachable without a preview on purpose. Declining is safe in the
    /// direction that matters -- nothing leaves the machine -- and making a
    /// contributor read a transcript before they may refuse it would push them
    /// towards approving just to clear the row.
    /// </remarks>
    public async Task DismissAsync(QueueEntryViewModel entry)
    {
        ArgumentNullException.ThrowIfNull(entry);

        DaemonResponse response = await _host
            .CallAsync(
                DaemonProtocol.Methods.Dismiss,
                JsonSerializer.Serialize(
                    new Dictionary<string, string> { ["entry_id"] = entry.EntryId }))
            .ConfigureAwait(true);

        Notice = response.IsError
            ? "That couldn't be skipped just now. Nothing has been sent."
            : string.Empty;

        await RefreshAsync().ConfigureAwait(true);
    }

    /// <summary>
    /// Recalls an approval, backed by the daemon's <c>cancel</c>.
    /// </summary>
    /// <remarks>
    /// A refusal here is reported rather than swallowed: <c>cancel</c> refuses
    /// anything an upload pass has already claimed, and someone who pressed
    /// Undo is owed the truth about whether it worked.
    /// </remarks>
    public async Task UndoAsync()
    {
        if (_undoHold is null || _undoEntryId.Length == 0)
        {
            return;
        }

        string entryId = _undoEntryId;
        ClearUndo();

        DaemonResponse response = await _host
            .CallAsync(
                DaemonProtocol.Methods.Cancel,
                JsonSerializer.Serialize(
                    new Dictionary<string, string> { ["entry_id"] = entryId }))
            .ConfigureAwait(true);

        Notice = response.IsError
            ? "Too late to undo: it has already gone out."
            : "Undone. It stays on this machine.";

        await RefreshAsync().ConfigureAwait(true);
    }

    /// <summary>
    /// Retires the undo bar without cancelling. The hold simply runs out on
    /// its own, which is what the contributor asked for by pressing this.
    /// </summary>
    public void DismissUndo()
    {
        ClearUndo();
        Notice = ApprovedNoUndo;
    }

    private void OnUndoTick()
    {
        if (_undoHold is null)
        {
            _undoTick.Stop();
            return;
        }

        if (!_undoHold.IsLive(DateTimeOffset.UtcNow))
        {
            ClearUndo();
            return;
        }

        Raise(nameof(UndoButtonText));
    }

    private void ClearUndo()
    {
        _undoTick.Stop();
        _undoHold = null;
        _undoEntryId = string.Empty;
        _undoProjectLabel = string.Empty;
        RaiseUndo();
    }

    private void RaiseUndo()
    {
        Raise(nameof(HasUndo));
        Raise(nameof(UndoHeadline));
        Raise(nameof(UndoButtonText));
    }

    /// <summary>
    /// Starts the daemon and loads the first queue snapshot.
    ///
    /// A start failure is shown rather than thrown: the overwhelmingly likely
    /// cause is another instance already holding the state directory's lock,
    /// which is a thing to tell the contributor plainly, not a crash.
    /// </summary>
    public async Task InitializeAsync()
    {
        try
        {
            await _host.StartAsync().ConfigureAwait(true);
        }
        catch (TcException)
        {
            // Deliberately not interpolating the exception message. It is a
            // fixed ABI label, but the UI string is more useful for saying
            // what to do about it.
            StatusText = "Could not start. Another Trace Commons instance may already be running.";
            return;
        }

        await RefreshAsync().ConfigureAwait(true);
    }

    /// <summary>
    /// Refetches the queue and the status line.
    ///
    /// Serialized by a gate rather than allowed to overlap: events can arrive
    /// in bursts, and two refreshes racing to rewrite one ObservableCollection
    /// produces flicker at best. A refresh already in flight makes a second
    /// request redundant, since the later one would read the same daemon state
    /// anyway.
    /// </summary>
    public async Task RefreshAsync()
    {
        if (!await _refreshGate.WaitAsync(0).ConfigureAwait(true))
        {
            return;
        }

        try
        {
            IsBusy = true;

            IReadOnlyList<QueueEntry> pending = await _host.ListPendingAsync().ConfigureAwait(true);
            ReplacePending(pending);

            DaemonResponse status = await _host
                .CallAsync(DaemonProtocol.Methods.Status)
                .ConfigureAwait(true);

            StatusText = status.IsError
                ? $"Daemon unavailable ({status.Error!.Code})"
                : DescribeQueue(Pending.Count);

            // The banner comes out of the status read this method already
            // makes. An error frame leaves the previous banner rather than
            // clearing it: a daemon that could not answer has not told us the
            // condition is over, and clearing on silence would retract a
            // "nothing is being sent" the contributor is entitled to keep
            // seeing until something says otherwise.
            if (!status.IsError)
            {
                SetHealth(status.ResultAs<DaemonStatus>()?.Health?.LastErrorLabel);
            }

            DaemonResponse rollup = await _host
                .CallAsync(DaemonProtocol.Methods.HistoryRollup)
                .ConfigureAwait(true);

            // A rollup that cannot be read keeps the previous figures rather
            // than zeroing them, matching HistoryViewModel: zeros drawn from a
            // failed read are a confident claim about someone's contributions
            // that nothing actually made.
            if (rollup.ResultAs<HistoryRollup>() is { } parsed)
            {
                _rollup = parsed;
                Raise(nameof(ContributedCountText));
                Raise(nameof(HeldCountText));
                Raise(nameof(InTheCommonsCountText));
            }
        }
        finally
        {
            IsBusy = false;
            _refreshGate.Release();
        }
    }

    /// <summary>
    /// Says that another copy of the app owns the daemon, and what to do
    /// about it.
    /// </summary>
    /// <remarks>
    /// Split into two sentences by whether an invite was on the command
    /// line, because the two situations need different next actions. Someone
    /// who double-clicked the app just needs to find the window they already
    /// have. Someone who clicked an invite link in mail is holding something
    /// they were trying to use, and needs to be told where to use it --
    /// otherwise the link looks broken and the invite looks dead, which is
    /// the impression this whole path exists to avoid giving.
    /// </remarks>
    public void ReportAlreadyRunning(bool withInvite)
    {
        StatusText = withInvite
            ? "Trace Commons is already running. Open that window and paste your invite there."
            : "Trace Commons is already running. Use the window that is already open.";
    }

    /// <summary>
    /// Takes the daemon's single health label and re-renders the banner.
    /// </summary>
    /// <remarks>
    /// Compared by value before raising, so a status event that repeats an
    /// unchanged condition does not rebuild the banner underneath a pointer
    /// already resting on its action button -- the same care the undo bar's
    /// tick takes, and for the same reason.
    /// </remarks>
    private void SetHealth(string? label)
    {
        HealthCopy? next = HealthCopy.ForLabel(label);
        if (Equals(_health, next))
        {
            return;
        }

        _health = next;
        Raise(nameof(HasHealthBanner));
        Raise(nameof(HealthTitle));
        Raise(nameof(HealthDetail));
        Raise(nameof(HasHealthAction));
        Raise(nameof(HealthActionLabel));
    }

    private static string DescribeQueue(int count) => count switch
    {
        0 => "No sessions waiting for review.",
        1 => "1 session waiting for review.",
        _ => $"{count} sessions waiting for review.",
    };

    /// <summary>
    /// Rewrites the collection in place.
    ///
    /// Clear-and-refill rather than a diff: the queue is small, the daemon is
    /// the sole authority on its contents, and a diff would introduce an
    /// opportunity for the local view to disagree with the daemon -- which is
    /// the exact class of bug a full refetch on every event exists to avoid.
    /// </summary>
    private void ReplacePending(IReadOnlyList<QueueEntry> entries)
    {
        Pending.Clear();
        foreach (QueueEntry entry in entries)
        {
            Pending.Add(new QueueEntryViewModel(entry));
        }

        Raise(nameof(IsEmpty));
    }

    private async void OnQueueChanged()
    {
        // async void because this is an event handler, which is the one place
        // it is correct. Exceptions are contained by RefreshAsync's own
        // handling of error frames; nothing it calls throws on a daemon error.
        await RefreshAsync().ConfigureAwait(true);
    }

    private async void OnStatusChanged()
    {
        await RefreshAsync().ConfigureAwait(true);
    }

    private void OnLagged(int skipped)
    {
        // Surfaced rather than swallowed. A lag means the app missed events;
        // the refetch that follows corrects the data, but the contributor
        // deserves to know the view briefly was not live.
        StatusText = skipped > 0
            ? $"Reconnecting… ({skipped} updates missed)"
            : "Reconnecting…";
    }

    /// <summary>
    /// Assigns and notifies, reporting whether anything changed so a setter
    /// can raise the properties derived from it without re-notifying on a
    /// no-op write.
    /// </summary>
    private bool Set<T>(ref T field, T value, [CallerMemberName] string? name = null)
    {
        if (EqualityComparer<T>.Default.Equals(field, value))
        {
            return false;
        }

        field = value;
        Raise(name);
        return true;
    }

    private void Raise(string? name) =>
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));
}
