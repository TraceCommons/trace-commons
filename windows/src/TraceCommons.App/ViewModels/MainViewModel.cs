using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Linq;
using System.Runtime.CompilerServices;
using System.Globalization;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using TraceCommons.App;

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
    private readonly AppUpdater? _updater;
    private readonly SemaphoreSlim _refreshGate = new(1, 1);
    private readonly SemaphoreSlim _watchingChangeGate = new(1, 1);
    private readonly DispatcherQueueTimer _undoTick;
    private string _statusText = "Starting…";
    private string _updateStatusText = string.Empty;
    private bool _isBusy;
    private bool _isPaused;
    private bool _isUpdateBannerVisible;
    private bool _isUpdateApplyEnabled;
    private bool _needsSessionRoots;

    private string _notice = string.Empty;
    private ApprovalHold? _undoHold;

    /// <summary>
    /// Which entries Undo would recall. One id for a row's submit or a
    /// preview approval; the project's approved ids -- computed by
    /// <see cref="ApprovalHold.ApprovedEntryIds"/> -- for a project-group
    /// submit, because <c>cancel</c> only ever takes one <c>entry_id</c> at a
    /// time and a batch approval has no batch recall.
    /// </summary>
    private IReadOnlyList<string> _undoEntryIds = Array.Empty<string>();

    /// <summary>
    /// The toast line behind the current undo bar, from
    /// <see cref="ApprovalHold.Toast"/>. It says exactly what happened, so
    /// there is nothing left for the headline to invent.
    /// </summary>
    private string _undoNoticeLine = string.Empty;
    private MainPane _pane = MainPane.Queue;
    private QueueLocation _queueLocation = QueueLocation.Root;
    private IReadOnlyList<ProjectQueueGroup> _groups = Array.Empty<ProjectQueueGroup>();
    private QueueGroupViewModel? _openFolder;
    private HealthCopy? _health;
    private ArmingOffer? _armingOffer;
    private HealthCopy? _budget;
    private HistoryRollup _rollup = new();

    /// <summary>
    /// Decides when an on-screen set is worth telling the daemon about --
    /// see <see cref="SetVisiblePreviewsAsync"/> -- and dedupes so a settle
    /// that reports the same set already sent produces no call at all.
    /// </summary>
    private readonly PreviewVisibilityTracker _visibilityTracker = new();

    /// <summary>
    /// The current rows, keyed by entry id, so a <c>preview_ready</c> event
    /// can find the card it belongs to without a linear scan of
    /// <see cref="Pending"/>. Rebuilt alongside <see cref="Pending"/> by
    /// <see cref="ReplacePending"/>.
    /// </summary>
    private Dictionary<string, QueueEntryViewModel> _rowsByEntryId = new(StringComparer.Ordinal);

    /// <summary>
    /// The entry ids <see cref="Pending"/> carried before the most recent
    /// <see cref="ReplacePending"/>, so it can tell which ones dropped out of
    /// the queue for good -- dismissed, submitted, expired, or superseded --
    /// and cancel their scheduled previews.
    /// </summary>
    private IReadOnlyList<string> _previousEntryIds = Array.Empty<string>();

    /// <summary>
    /// <paramref name="updater"/> is optional so the view model stays
    /// constructible without package identity. An unpackaged developer build
    /// then simply never shows the banner, rather than throwing at launch.
    /// </summary>
    public MainViewModel(DaemonHost host, AppUpdater? updater = null)
    {
        _host = host ?? throw new ArgumentNullException(nameof(host));
        _updater = updater;
        _host.QueueChanged += OnQueueChanged;
        _host.StatusChanged += OnStatusChanged;
        _host.Lagged += OnLagged;
        _host.PreviewReady += OnPreviewReady;

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
    /// The same queue, grouped by project. Rebuilt alongside <see cref="Pending"/>
    /// by <see cref="ReplacePending"/> from <see cref="QueueGrouping.ByProject"/> --
    /// the grouping rule itself (bucket key, order, whether "Submit all"
    /// shows) lives entirely in <c>TraceCommons.Interop</c> and is tested
    /// there; this collection only carries that decision to the queue view.
    /// </summary>
    public ObservableCollection<QueueGroupViewModel> Groups { get; } = new();

    /// <summary>
    /// The sessions inside the folder currently open, or empty at the root.
    /// </summary>
    /// <remarks>
    /// Its own collection rather than a nested binding into
    /// <see cref="Groups"/>, so the detail pane has exactly one thing to bind
    /// to and cannot end up rendering a folder that is no longer in
    /// <see cref="Groups"/> at all. Refilled by
    /// <see cref="SetQueueLocation"/>, which is the only thing that changes
    /// the location.
    /// </remarks>
    public ObservableCollection<QueueEntryViewModel> OpenFolderEntries { get; } = new();

    /// <summary>
    /// Whether the queue is showing the folder list rather than one folder's
    /// sessions.
    /// </summary>
    public bool IsAtQueueRoot => _queueLocation is not QueueLocation.Project;

    /// <summary>The inverse of <see cref="IsAtQueueRoot"/>, for the detail pane.</summary>
    public bool IsInQueueFolder => !IsAtQueueRoot;

    /// <summary>The open folder's label, or empty at the root.</summary>
    public string OpenFolderLabel => _openFolder?.ProjectLabel ?? string.Empty;

    /// <summary>The open folder's display path, or empty when it has none.</summary>
    public string OpenFolderPath => _openFolder?.ProjectPath ?? string.Empty;

    /// <summary>Whether the detail heading has a path to draw beneath its label.</summary>
    public bool HasOpenFolderPath => OpenFolderPath.Length > 0;

    /// <summary>The open folder's id, which "Submit all" from the detail sends.</summary>
    public string OpenFolderProjectId => _openFolder?.ProjectId ?? string.Empty;

    /// <summary>Opens one folder's sessions.</summary>
    public void OpenFolder(string projectId)
    {
        ArgumentNullException.ThrowIfNull(projectId);
        SetQueueLocation(new QueueLocation.Project(projectId));
    }

    /// <summary>Returns to the folder list.</summary>
    public void CloseFolder() => SetQueueLocation(QueueLocation.Root);

    /// <summary>
    /// Moves the queue to <paramref name="location"/>, after checking it is
    /// somewhere that still exists.
    /// </summary>
    /// <remarks>
    /// Every path that changes either the location or the queue's contents
    /// goes through here, including <see cref="ReplacePending"/>. That is
    /// what makes a folder emptying underneath the contributor -- their own
    /// "Submit all", or an upload finishing in the background -- return them
    /// to the list rather than leaving them on an empty pane with a back
    /// button and no explanation. <see cref="QueueNavigation.Resolve"/> makes
    /// that decision and is tested in TraceCommons.Interop.
    /// </remarks>
    private void SetQueueLocation(QueueLocation location)
    {
        _queueLocation = QueueNavigation.Resolve(location, _groups);
        _openFolder = _queueLocation is QueueLocation.Project project
            ? Groups.FirstOrDefault(group =>
                string.Equals(group.ProjectId, project.ProjectId, StringComparison.Ordinal))
            : null;

        OpenFolderEntries.Clear();
        if (_openFolder is not null)
        {
            foreach (QueueEntryViewModel row in _openFolder.Entries)
            {
                OpenFolderEntries.Add(row);
            }
        }

        Raise(nameof(IsAtQueueRoot));
        Raise(nameof(IsInQueueFolder));
        Raise(nameof(OpenFolderLabel));
        Raise(nameof(OpenFolderPath));
        Raise(nameof(HasOpenFolderPath));
        Raise(nameof(OpenFolderProjectId));
    }

    /// <summary>
    /// The rail's shield state for the queue.
    /// </summary>
    /// <remarks>
    /// <b>Beside the numeric count, never instead of it.</b> The request was
    /// to swap the count for an icon; at 149 waiting sessions the count is the
    /// signal a contributor actually reads, and an icon meaning "some" is a
    /// downgrade exactly at the scale that produced the feedback. The decision
    /// itself is <see cref="QueueShield.For"/>'s, in TraceCommons.Interop,
    /// where it is tested.
    /// </remarks>
    public QueueShieldState ShieldState => QueueShield.For(
        Pending.Count,
        Pending.Count(row => row.MatchedNothing),
        Pending.Count(row => row.WasTrimmed));

    /// <summary>Nothing waiting.</summary>
    public bool ShieldIsClear => ShieldState == QueueShieldState.Clear;

    /// <summary>Decisions owed, and nothing about them wants a second look.</summary>
    public bool ShieldIsWaiting => ShieldState == QueueShieldState.Waiting;

    /// <summary>Something waiting matched nothing, or was trimmed to fit.</summary>
    public bool ShieldIsAttention => ShieldState == QueueShieldState.Attention;

    /// <summary>
    /// The rail's numeric badge: how many decisions are owed.
    /// </summary>
    /// <remarks>
    /// Empty on an empty queue, so a rail with nothing waiting carries no
    /// zero. This is the figure the shield is added to, and it is the one a
    /// contributor at scale is reading.
    /// </remarks>
    public string QueueCountText =>
        Pending.Count == 0
            ? string.Empty
            : Pending.Count.ToString(CultureInfo.CurrentCulture);

    /// <summary>Whether there is a badge to draw at all.</summary>
    public bool HasQueueCount => Pending.Count > 0;

    /// <summary>
    /// Re-reads every rail signal. Called when the queue changes and when a
    /// preview lands, because a preview is what tells the shield that a
    /// session matched nothing.
    /// </summary>
    private void RaiseShield()
    {
        Raise(nameof(ShieldState));
        Raise(nameof(ShieldIsClear));
        Raise(nameof(ShieldIsWaiting));
        Raise(nameof(ShieldIsAttention));
        Raise(nameof(QueueCountText));
        Raise(nameof(HasQueueCount));
    }

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

    // --- The arming offer --------------------------------------------------
    //
    // The offer to stop being asked about one project, drawn above the cards
    // it is about. The daemon decides whether there is anything to ask
    // (ProjectPolicy::arming_suggestion) and both answers go back to it, so
    // "Not now" is remembered across relaunches and across shells rather than
    // being a dismissal this window forgets. This holds whichever offer
    // arrived and renders it; it never decides one for itself.

    /// <summary>Whether there is a project worth offering to arm.</summary>
    public bool HasArmingOffer => _armingOffer is not null;

    /// <summary>The evidence line, stated above the question.</summary>
    public string ArmingOfferEvidence =>
        _armingOffer is { } offer
            ? ArmingOfferCopy.Evidence(offer.ProjectLabel, offer.ContributedCount)
            : string.Empty;

    public string ArmingOfferQuestion =>
        _armingOffer is { } offer ? ArmingOfferCopy.Question(offer.ProjectLabel) : string.Empty;

    public string ArmingOfferConfirm => ArmingOfferCopy.Confirm;

    public string ArmingOfferDecline => ArmingOfferCopy.Decline;

    /// <summary>
    /// The offered project's opaque id, for the two calls the buttons make.
    /// Empty when there is no offer, which the window treats as "do nothing"
    /// rather than as a project to act on.
    /// </summary>
    public string ArmingOfferProjectId => _armingOffer?.ProjectId ?? string.Empty;

    /// <summary>
    /// Stores whichever offer the daemon last reported, or clears it.
    /// </summary>
    public void SetArmingOffer(ArmingOffer? offer)
    {
        if (Equals(_armingOffer, offer))
        {
            return;
        }

        _armingOffer = offer;
        Raise(nameof(HasArmingOffer));
        Raise(nameof(ArmingOfferEvidence));
        Raise(nameof(ArmingOfferQuestion));
        Raise(nameof(ArmingOfferProjectId));
    }

    // --- The daily-budget banner -------------------------------------------
    //
    // Rendered from status.daily_budget, independently of the health label.
    //
    // The daemon enforces a daily byte and upload cap, and it does set a
    // daily-cap-reached health label when one refuses an upload. But that
    // label is last in the precedence order, so any other condition takes
    // the single last_error_label slot and the cap vanishes. A contributor
    // watched fourteen approved traces sit still for an evening while this
    // window reported a full queue and said nothing about the budget that
    // was actually holding them.

    /// <summary>Whether approved traces are waiting on the daily budget.</summary>
    public bool HasBudgetBanner => _budget is not null;

    public string BudgetTitle => _budget?.Title ?? string.Empty;

    public string BudgetDetail => _budget?.Detail ?? string.Empty;

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

    /// <summary>Whether the daemon has stopped discovering and sending.</summary>
    public bool IsPaused => _isPaused;

    /// <summary>The inverse used by the Waiting header's mutually exclusive controls.</summary>
    public bool IsWatching => !_isPaused;

    /// <summary>True when there is nothing pending, for an empty-state view.</summary>
    public bool IsEmpty => Pending.Count == 0;

    /// <summary>
    /// Whether the update banner is on screen. Only ever true for a
    /// confirmed offer -- see <c>UpdateProtocol.ShouldOfferUpdate</c>.
    /// </summary>
    public bool IsUpdateBannerVisible
    {
        get => _isUpdateBannerVisible;
        private set => Set(ref _isUpdateBannerVisible, value);
    }

    /// <summary>
    /// Whether the banner's action button is live. Goes false for the
    /// duration of an apply so a second click cannot start a second
    /// handoff.
    /// </summary>
    public bool IsUpdateApplyEnabled
    {
        get => _isUpdateApplyEnabled;
        private set => Set(ref _isUpdateApplyEnabled, value);
    }

    /// <summary>
    /// The banner's message. Fixed labels only, from
    /// <c>UpdateProtocol</c> -- nothing the deployment service or the daemon
    /// said reaches this string.
    /// </summary>
    public string UpdateStatusText
    {
        get => _updateStatusText;
        private set => Set(ref _updateStatusText, value);
    }

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
    /// Lets the window put a line here for something it handled itself and
    /// this model never saw -- today, a confirmation dialog that could not be
    /// shown at all. The setter stays private: every other notice is written
    /// by the call that earned it, and a view that can overwrite that at will
    /// is how a refusal ends up displayed under a success.
    /// </summary>
    internal void ShowNotice(string line) => Notice = line;

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

    /// <summary>
    /// The submit toast: what was sent, what scrubbing did, what was flagged,
    /// what was not sent -- <see cref="ApprovalHold.Toast"/>'s line, verbatim.
    /// </summary>
    public string UndoHeadline => _undoNoticeLine;

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
                _undoEntryIds = new[] { entry.EntryId };
                _undoNoticeLine = hold.Toast.Line;
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
    /// "Submit" on a queue row: one click, no preview. Approves this entry
    /// alone and renders the daemon's counts as a toast, exactly as approving
    /// from behind a preview does -- the only difference is which surface
    /// asked, and <see cref="ApprovalHold"/> is what both paths decode the
    /// response through.
    /// </summary>
    public async Task SubmitEntryAsync(QueueEntryViewModel entry)
    {
        ArgumentNullException.ThrowIfNull(entry);

        ClearUndo();

        DaemonResponse response = await _host
            .CallAsync(DaemonProtocol.Methods.Approve, SubmitParams.ForEntry(entry.EntryId))
            .ConfigureAwait(true);

        ApplySubmitOutcome(response, new[] { entry.EntryId });

        await RefreshAsync().ConfigureAwait(true);
    }

    /// <summary>
    /// "Submit" on a project group: one <c>approve</c> call for every pending
    /// entry in <paramref name="projectId"/>.
    /// </summary>
    /// <remarks>
    /// <paramref name="projectId"/> must be the id an <c>entry_value</c>
    /// publishes as <c>project_id</c> -- never <see cref="QueueEntryViewModel.ProjectLabel"/>,
    /// which is a display string the daemon does not treat as an identifier.
    /// An unrecognised id is refused as <c>bad_params</c>, which
    /// <see cref="ApplySubmitOutcome"/> reports as a refusal rather than as a
    /// toast claiming nothing was sent -- there is no result to render one
    /// from.
    ///
    /// The candidate entry ids are read from <see cref="Pending"/> BEFORE the
    /// call, not after: <c>approve</c> can move entries out of the pending
    /// state by the time it returns, and Undo needs to know which of today's
    /// ids to recall, not tomorrow's.
    ///
    /// <paramref name="outcome"/> is the contributor's verdict, and it is
    /// <c>null</c> for the plain one-click "Submit all" -- that button never
    /// asked the question, so its call must omit the key entirely rather than
    /// send an empty one. A value supplied here applies to every entry this
    /// approval covers, which is what the "Submit all as..." menu beside the
    /// button is for. See <see cref="Verdict"/>.
    /// </remarks>
    public async Task SubmitProjectAsync(string projectId, string? outcome = null)
    {
        if (string.IsNullOrWhiteSpace(projectId))
        {
            return;
        }

        ClearUndo();

        IReadOnlyList<string> candidateEntryIds = Pending
            .Where(entry => entry.ProjectId == projectId)
            .Select(entry => entry.EntryId)
            .ToList();

        DaemonResponse response = await _host
            .CallAsync(
                DaemonProtocol.Methods.Approve,
                SubmitParams.ForProject(projectId, outcome))
            .ConfigureAwait(true);

        ApplySubmitOutcome(response, candidateEntryIds);

        await RefreshAsync().ConfigureAwait(true);
    }

    /// <summary>
    /// "Ignore project" from a project's group header: turns the project's
    /// mode to <c>ignore</c>, which the daemon answers by purging that
    /// project's own waiting sessions server-side (Task 2).
    /// </summary>
    /// <remarks>
    /// The confirmation dialog has already been shown and accepted by the
    /// time this runs -- this method only makes the call and refreshes.
    /// <see cref="RefreshAsync"/> is what actually removes the purged cards
    /// from <see cref="Groups"/>; this project's mode does not stop it being
    /// re-offered client-side, so a queue reload is not optional here the way
    /// it is after a submit.
    /// </remarks>
    public async Task IgnoreProjectAsync(string projectId, string projectLabel, int promised)
    {
        if (string.IsNullOrWhiteSpace(projectId))
        {
            return;
        }

        string payload = JsonSerializer.Serialize(
            new Dictionary<string, string>
            {
                ["project_id"] = projectId,
                ["mode"] = "ignore",
            });

        DaemonResponse response = await _host
            .CallAsync(DaemonProtocol.Methods.SetProjectMode, payload)
            .ConfigureAwait(true);

        if (response.IsError)
        {
            Notice = "That project setting couldn't be changed.";
        }
        else
        {
            // The confirmation had to name a count before this call, off a
            // queue that keeps moving; `purged` is what the daemon actually
            // did and is the authority. A daemon older than the field sends
            // none, which reads as 0 here -- so it is only reconciled when it
            // is present, since silence is not a disagreement.
            Notice = ReadPurged(response) is int purged
                ? ProjectIgnoreCopy.Reconciliation(projectLabel, promised, purged) ?? string.Empty
                : string.Empty;
        }

        await RefreshAsync().ConfigureAwait(true);
    }

    /// <summary>
    /// <c>set_project_mode</c>'s <c>purged</c>, or null when the daemon did
    /// not send one.
    /// </summary>
    private static int? ReadPurged(DaemonResponse response)
    {
        if (response.Result is not JsonElement result
            || result.ValueKind != JsonValueKind.Object
            || !result.TryGetProperty("purged", out JsonElement purged)
            || !purged.TryGetInt32(out int value))
        {
            return null;
        }
        return value;
    }

    /// <summary>
    /// The shared tail of both one-click submit paths: render the toast, and
    /// arm Undo over exactly the entries this call actually approved.
    /// </summary>
    private void ApplySubmitOutcome(DaemonResponse response, IReadOnlyList<string> candidateEntryIds)
    {
        ApprovalHold? hold = ApprovalHold.Parse(response);
        if (hold is null)
        {
            // A malformed result and a bad_params refusal look identical here
            // on purpose: neither carries counts to build a toast from, and
            // an unrecognised entry_id or project_id must read as a refusal,
            // never as "0 sessions sent" dressed up as success.
            Notice = "That couldn't be sent just now. Nothing has been sent.";
            return;
        }

        Notice = hold.Toast.Line;

        if (hold.Toast.OfferUndo && hold.IsLive(DateTimeOffset.UtcNow))
        {
            _undoHold = hold;
            _undoEntryIds = hold.ApprovedEntryIds(candidateEntryIds);
            _undoNoticeLine = hold.Toast.Line;
            RaiseUndo();
            _undoTick.Start();
        }
    }

    /// <summary>
    /// Pauses through the daemon, which persists timed deadlines across app
    /// restarts. Nothing already waiting is discarded.
    /// </summary>
    public async Task PauseAsync(PauseDuration duration)
    {
        if (!await _watchingChangeGate.WaitAsync(0).ConfigureAwait(true))
        {
            return;
        }

        try
        {
            DaemonResponse response = await _host
                .CallAsync(
                    DaemonProtocol.Methods.Pause,
                    PauseRequest.Serialize(duration, DateTimeOffset.UtcNow))
                .ConfigureAwait(true);

            Notice = response.IsError
                ? "Watching couldn't be paused just now. Nothing already waiting was changed."
                : string.Empty;

            await RefreshAsync().ConfigureAwait(true);
        }
        finally
        {
            _watchingChangeGate.Release();
        }
    }

    /// <summary>Resumes discovery and the normal upload sweep.</summary>
    public async Task ResumeAsync()
    {
        if (!await _watchingChangeGate.WaitAsync(0).ConfigureAwait(true))
        {
            return;
        }

        try
        {
            DaemonResponse response = await _host
                .CallAsync(DaemonProtocol.Methods.Resume)
                .ConfigureAwait(true);

            Notice = response.IsError
                ? "Watching couldn't be resumed just now."
                : string.Empty;

            await RefreshAsync().ConfigureAwait(true);
        }
        finally
        {
            _watchingChangeGate.Release();
        }
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
        if (_undoHold is null || _undoEntryIds.Count == 0)
        {
            return;
        }

        IReadOnlyList<string> entryIds = _undoEntryIds;
        ClearUndo();

        // cancel takes exactly one entry_id: a project-group submit approved
        // several entries in one approve call, but there is no batch cancel
        // to recall them with, so each is recalled in turn.
        //
        // The spec's Undo copy is written for exactly one recall and gives
        // no third sentence for "some of them" -- inventing one here would
        // be exactly the drift the toast contract exists to prevent, so a
        // partial recall is reported the same as a total refusal: honest
        // about what it does NOT promise (a full undo), and silent about a
        // number the spec never asked this button to say out loud. Only
        // "every one of them came back" gets the success sentence.
        bool allCancelled = true;
        foreach (string entryId in entryIds)
        {
            DaemonResponse response = await _host
                .CallAsync(DaemonProtocol.Methods.Cancel, SubmitParams.ForEntry(entryId))
                .ConfigureAwait(true);

            if (response.IsError)
            {
                allCancelled = false;
            }
        }

        Notice = allCancelled
            ? "Undone. It stays on this machine."
            : "Too late to undo: it has already gone out.";

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
        _undoEntryIds = Array.Empty<string>();
        _undoNoticeLine = string.Empty;
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
    ///
    /// One failure is not a fault at all. A daemon refused because the session
    /// sources are undeclared is waiting for an answer nobody has been asked
    /// for yet, so it sets <see cref="NeedsSessionRoots"/> and the caller
    /// shows the roots screen. Reporting that as "another instance may already
    /// be running" would be telling the contributor something false about
    /// their own machine, and leaving them with nothing to do about it.
    /// </summary>
    public async Task InitializeAsync()
    {
        try
        {
            await _host.StartAsync().ConfigureAwait(true);
            NeedsSessionRoots = false;
        }
        catch (TcException exception) when (exception.IsRootsNotDeclared)
        {
            NeedsSessionRoots = true;
            StatusText = "Trace Commons is not watching anything yet.";
            return;
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
    /// Whether the last start was refused because the contributor has not yet
    /// said which session folders to watch.
    /// </summary>
    public bool NeedsSessionRoots
    {
        get => _needsSessionRoots;
        private set
        {
            if (_needsSessionRoots != value)
            {
                _needsSessionRoots = value;
                Raise(nameof(NeedsSessionRoots));
            }
        }
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

            // Asked alongside the queue because it is drawn on the queue
            // screen, and the daemon's answer changes on exactly the events
            // that change the queue -- an upload landing is what moves a
            // project past the threshold. An error frame leaves the previous
            // offer alone rather than clearing it: a daemon that could not
            // answer has not withdrawn the question.
            DaemonResponse suggestion = await _host
                .CallAsync(DaemonProtocol.Methods.ArmingSuggestion)
                .ConfigureAwait(true);
            if (!suggestion.IsError && suggestion.Result is { } offerBody)
            {
                SetArmingOffer(ArmingOffer.Parse(offerBody));
            }

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
            if (!status.IsError && status.ResultAs<DaemonStatus>() is { } parsedStatus)
            {
                SetPaused(parsedStatus.Paused);
                // Budget first: SetHealth suppresses the bare
                // daily-cap-reached line when the budget banner is going to
                // say the same thing with real numbers, so it has to see
                // this pass's budget rather than the previous pass's.
                SetBudget(parsedStatus.DailyBudget);
                SetHealth(parsedStatus.Health?.LastErrorLabel);
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
    /// Asks the deployment service whether the feed offers something newer,
    /// and raises the banner if it does.
    ///
    /// Never surfaces a failed check. Windows checks the feed on its own
    /// schedule regardless of what this call returns, so a check that could
    /// not complete costs a contributor nothing and telling them about it
    /// buys nothing either.
    /// </summary>
    public async Task CheckForUpdateAsync()
    {
        if (_updater is null)
        {
            return;
        }

        TcUpdateAvailability availability = await _updater.CheckAsync().ConfigureAwait(true);
        if (!UpdateProtocol.ShouldOfferUpdate(availability))
        {
            IsUpdateBannerVisible = false;
            return;
        }

        UpdateStatusText = UpdateProtocol.DescribeAvailability(availability);
        IsUpdateApplyEnabled = true;
        IsUpdateBannerVisible = true;
    }

    /// <summary>
    /// Drains, tears the daemon down, and hands the update to Windows.
    ///
    /// The order is the whole point. Quiesce first, because App Installer
    /// terminates this process and a half-uploaded trace must never be the
    /// cost of an update. Then dispose the host, so the C ABI's ordered
    /// teardown runs while there is still a process to run it in. Only then
    /// hand off -- and on the success path control does not return from that
    /// call, because the process is gone.
    /// </summary>
    public async Task ApplyUpdateAsync()
    {
        if (_updater is null)
        {
            return;
        }

        IsUpdateApplyEnabled = false;
        UpdateStatusText = "Finishing any upload in progress…";

        QuiesceOutcome quiesce = await _updater.QuiesceAsync().ConfigureAwait(true);
        if (!quiesce.CanUpdate)
        {
            UpdateStatusText = UpdateProtocol.DescribeRefusal(quiesce.Outcome);
            return;
        }

        UpdateStatusText = "Installing the update…";
        await _host.DisposeAsync().ConfigureAwait(true);

        bool handedOff = await _updater.ApplyAsync().ConfigureAwait(true);
        if (!handedOff)
        {
            UpdateStatusText =
                "The update could not be installed. Windows will try again on its own schedule.";
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
        HealthCopy? next = _budget is not null && label == "daily-cap-reached"
            ? null
            : HealthCopy.ForLabel(label);
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

    /// <summary>
    /// Takes status.daily_budget and re-renders the second banner.
    /// </summary>
    /// <remarks>
    /// A second banner rather than another case in SetHealth, because the
    /// two conditions are independent: the health slot carries one label and
    /// daily-cap-reached is last in its precedence order, so a spent upload
    /// budget behind a full queue was announced by neither. Compared by
    /// value before raising, exactly as SetHealth is.
    /// </remarks>
    private void SetBudget(DailyBudget? budget)
    {
        HealthCopy? next = HealthCopy.ForBudget(budget);
        if (Equals(_budget, next))
        {
            return;
        }

        _budget = next;
        Raise(nameof(HasBudgetBanner));
        Raise(nameof(BudgetTitle));
        Raise(nameof(BudgetDetail));
    }

    private void SetPaused(bool paused)
    {
        if (_isPaused == paused)
        {
            return;
        }

        _isPaused = paused;
        Raise(nameof(IsPaused));
        Raise(nameof(IsWatching));
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

        var rowsByEntryId = new Dictionary<string, QueueEntryViewModel>(StringComparer.Ordinal);
        var currentIds = new List<string>(entries.Count);
        foreach (QueueEntry entry in entries)
        {
            var row = new QueueEntryViewModel(entry);
            Pending.Add(row);
            rowsByEntryId[entry.EntryId] = row;
            currentIds.Add(entry.EntryId);
        }

        // The grouping rule itself -- bucket key, group order, whether
        // Submit all shows -- is QueueGrouping.ByProject's, tested in
        // TraceCommons.Interop.Tests. This only reassembles which rows go
        // under each group, using QueueGrouping.KeyOf so membership is
        // computed by the exact same rule the groups were bucketed with.
        Groups.Clear();
        _groups = QueueGrouping.ByProject(entries);
        foreach (ProjectQueueGroup group in _groups)
        {
            var rows = new ObservableCollection<QueueEntryViewModel>();
            foreach (QueueEntry entry in entries)
            {
                if (QueueGrouping.KeyOf(entry) == group.ProjectId)
                {
                    rows.Add(rowsByEntryId[entry.EntryId]);
                }
            }

            Groups.Add(new QueueGroupViewModel(group, rows));
        }

        // Re-resolved on every snapshot, not only when the contributor
        // navigates. A folder can be emptied by their own "Submit all" or by
        // an upload finishing in the background, and this is what returns
        // them to the list when it is.
        SetQueueLocation(_queueLocation);

        Raise(nameof(IsEmpty));
        RaiseShield();

        // Every row above is drawn already, with no preview yet -- Preview
        // is null on a freshly built QueueEntryViewModel, which reads as
        // pending. What follows only SCHEDULES the work; nothing here blocks
        // the draw on a daemon round trip.
        //
        // An id present before this call and absent now left the queue for
        // good -- dismissed, submitted, expired, or superseded, all alike
        // from here -- and its scheduled preview is cancelled. This is the
        // queue's own membership diff, not a scroll signal: visibility
        // (SetVisiblePreviewsAsync) is a completely separate axis that only
        // ever affects build ORDER for ids still in this set.
        IReadOnlyList<string> removed = PreviewCancellation.EntriesRemoved(_previousEntryIds, currentIds);
        _previousEntryIds = currentIds;
        _rowsByEntryId = rowsByEntryId;

        foreach (string entryId in removed)
        {
            _ = _host.CallAsync(DaemonProtocol.Methods.PreviewCancel, SubmitParams.ForEntry(entryId));
        }

        // Every row just built asks for its own preview, once each, and none
        // of these calls is awaited here: preview_request is documented as
        // free to repeat -- a cache hit answers inline, a still-building one
        // is a no-op re-enqueue -- so asking again on every refresh is the
        // intended usage, not waste.
        foreach (QueueEntryViewModel row in rowsByEntryId.Values)
        {
            _ = RequestPreviewAsync(row);
        }
    }

    /// <summary>
    /// Asks the daemon's bounded scheduler for one card's preview. Answered
    /// either inline (a cache hit) or later by <see cref="OnPreviewReady"/>;
    /// either way this never blocks the card that is already on screen.
    /// </summary>
    private async Task RequestPreviewAsync(QueueEntryViewModel row)
    {
        DaemonResponse response = await _host
            .CallAsync(DaemonProtocol.Methods.PreviewRequest, SubmitParams.ForEntry(row.EntryId))
            .ConfigureAwait(true);

        if (response.IsError || response.Result is not { } result)
        {
            return;
        }

        if (PreviewCardOutcome.Parse(result) is { } outcome && outcome.EntryId == row.EntryId)
        {
            row.Preview = outcome;
            RaiseShield();
        }
    }

    /// <summary>
    /// Fills in the card a scheduled build finished for, if it is still one
    /// of today's rows. A response for a row that has since been rebuilt (a
    /// refresh replaced every <see cref="QueueEntryViewModel"/> wholesale) or
    /// dropped from the queue simply finds nothing in
    /// <see cref="_rowsByEntryId"/> and is discarded -- there is no card left
    /// for it to fill.
    /// </summary>
    private void OnPreviewReady(PreviewCardOutcome outcome)
    {
        if (_rowsByEntryId.TryGetValue(outcome.EntryId, out QueueEntryViewModel? row))
        {
            row.Preview = outcome;
            RaiseShield();
        }
    }

    /// <summary>
    /// Tells the daemon which entries are on screen right now, debounced by
    /// <see cref="_visibilityTracker"/> so a settle that reports an unchanged
    /// set produces no call at all. The caller -- <c>MainWindow</c> -- owns
    /// figuring out what "on screen" means against its own nested,
    /// virtualizing containers; this only owns not spamming the daemon with
    /// the answer.
    /// </summary>
    public Task SetVisiblePreviewsAsync(IReadOnlyList<string> visibleEntryIds)
    {
        string? paramsJson = _visibilityTracker.OnSettled(visibleEntryIds);
        return paramsJson is null
            ? Task.CompletedTask
            : _host.CallAsync(DaemonProtocol.Methods.PreviewVisible, paramsJson);
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
