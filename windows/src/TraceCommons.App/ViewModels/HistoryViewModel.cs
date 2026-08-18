using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Globalization;
using System.Linq;
using System.Runtime.CompilerServices;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using TraceCommons.Interop;

namespace TraceCommons.App.ViewModels;

/// <summary>
/// The history screen's state: what this account has sent, what became of it,
/// what it earned, and the one act on this screen that is the contributor's
/// own -- withdrawal.
///
/// Driven by four read methods (<c>list_history</c>, <c>history_rollup</c>,
/// <c>refresh_history</c>, <c>queue_outcome_counts</c>) and one write
/// (<c>withdraw</c>). Every one of them was already in the daemon's pinned
/// METHODS array; nothing here adds to it.
///
/// UI-thread-affine like <see cref="MainViewModel"/>, for the same reason:
/// <see cref="DaemonHost"/> hops before it raises anything.
/// </summary>
public sealed class HistoryViewModel : INotifyPropertyChanged
{
    private readonly DaemonHost _host;
    private readonly SemaphoreSlim _refreshGate = new(1, 1);

    /// <summary>
    /// What a withdrawal attempt did, by submission id.
    /// </summary>
    /// <remarks>
    /// Kept here rather than on the row because the rows are rebuilt from the
    /// daemon's cache on every refresh, and the tier the server applied must
    /// survive that: <c>list_history</c> reports a record's status, never the
    /// tier a withdrawal resolved to. Without this map, re-reading history
    /// after a successful withdrawal would replace "here is exactly what that
    /// achieved" with a bare chip -- which is rule 1 broken by a refresh.
    /// </remarks>
    private readonly Dictionary<string, WithdrawalAttempt> _withdrawals =
        new(StringComparer.Ordinal);

    private HistoryRollup _rollup = new();
    private bool _isBusy;
    private bool _loaded;
    private string _notice = string.Empty;

    public HistoryViewModel(DaemonHost host)
    {
        _host = host ?? throw new ArgumentNullException(nameof(host));
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    /// <summary>The page of records, newest first, as the daemon orders them.</summary>
    public ObservableCollection<HistoryRecordViewModel> Records { get; } = new();

    /// <summary>
    /// Entries that reached the queue and did not go out, by the daemon's own
    /// reason label.
    /// </summary>
    public ObservableCollection<OutcomeCountViewModel> Outcomes { get; } = new();

    public bool IsBusy
    {
        get => _isBusy;
        private set
        {
            if (Set(ref _isBusy, value))
            {
                Raise(nameof(IsNotBusy));
            }
        }
    }

    public bool IsNotBusy => !_isBusy;

    /// <summary>
    /// True only once a load has completed and found nothing. Before that the
    /// screen is loading, not empty, and an empty state shown during the first
    /// read would read as "you have contributed nothing" to someone who has.
    /// </summary>
    public bool IsEmpty => _loaded && Records.Count == 0;

    /// <summary>
    /// A one-line result of the last action. Always a fixed sentence: the
    /// daemon's labels are content-free by contract and nothing here is the
    /// first place that stops being true.
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

    // --- The three stat cards ---------------------------------------------
    // Three figures, never one column of mixed semantics, and held is
    // reported separately from everything else because a contributor who sees
    // it grouped with failures reads it as rejection.

    // Formatted here rather than bound as ints. x:Bind is strongly typed and
    // performs no implicit ToString for TextBlock.Text, so an int bound
    // straight to a figure is a compile error on Windows and nowhere else --
    // exactly the class of break this project cannot see from macOS.
    public string InTheCommonsCountText =>
        _rollup.AllTime.Accepted.ToString(CultureInfo.CurrentCulture);

    public string HeldCountText => _rollup.Quarantined.ToString(CultureInfo.CurrentCulture);

    public string WaitingCountText =>
        _rollup.WaitingToBeScored.ToString(CultureInfo.CurrentCulture);

    public string InTheCommonsLabel => HistoryCopy.InTheCommons;

    public string HeldLabel => HistoryCopy.QuarantineHeading;

    public string WaitingLabel => HistoryCopy.WaitingToBeScored;

    /// <summary>
    /// The held group appears only when something is held. There is no empty
    /// state for a section that is a consequence of a state nobody is in.
    /// </summary>
    public bool HasHeld => _rollup.Quarantined > 0;

    public string HeldGroupHeading => string.Format(
        CultureInfo.CurrentCulture,
        "{0} — {1} {2}",
        HistoryCopy.QuarantineHeading,
        _rollup.Quarantined,
        _rollup.Quarantined == 1 ? "trace" : "traces");

    public string QuarantineBody => HistoryCopy.QuarantineBody;

    /// <summary>
    /// Why the held group has no "withdraw all of these" button even though
    /// the shared design draws one. See <see cref="WithdrawCopy.NoBulk"/>: the
    /// bulk call reports only counts, so rule 1 cannot be honoured for it.
    /// </summary>
    public string NoBulkNote => WithdrawCopy.NoBulk;

    // --- Credit -----------------------------------------------------------
    // Credit is a record, not a currency, so it is set as a ledger figure:
    // unadorned, no symbol, nothing that could read as a score. The prose
    // beside it is what stops the number being mistaken for one.

    public string CreditRecordedText =>
        string.Format(CultureInfo.CurrentCulture, "{0:0.0}", _rollup.CreditFinal);

    public string CreditPendingText =>
        string.Format(CultureInfo.CurrentCulture, "{0:0.0}", _rollup.CreditPending);

    public string CreditBody => HistoryCopy.CreditBody;

    /// <summary>
    /// Whether the figures may be shown at all.
    /// </summary>
    /// <remarks>
    /// <c>last_refreshed_at: null</c> means history has never been refreshed
    /// from the server, and a confident "0.0" drawn from a cache that has
    /// never been filled is a lie about a number people care about. So the
    /// figures give way to <see cref="HistoryCopy.NotSyncedYet"/> instead.
    /// </remarks>
    public bool HasFigures => _rollup.LastRefreshedAt is not null;

    public bool HasNoFigures => !HasFigures;

    public string NotSyncedYet => HistoryCopy.NotSyncedYet;

    /// <summary>When the figures were last known to be true.</summary>
    public string RefreshedText =>
        DateTimeOffset.TryParse(
            _rollup.LastRefreshedAt,
            CultureInfo.InvariantCulture,
            DateTimeStyles.RoundtripKind,
            out DateTimeOffset parsed)
            ? string.Format(
                CultureInfo.CurrentCulture,
                "Refreshed {0}",
                parsed.ToLocalTime().ToString("g", CultureInfo.CurrentCulture))
            : string.Empty;

    // --- Community --------------------------------------------------------

    /// <summary>
    /// The public roster standing, drawn only for a roster member.
    /// </summary>
    /// <remarks>
    /// Absent means no standing, and absent is not null: the contract omits
    /// the object entirely -- no published handle, no served snapshot, not on
    /// the roster, or a count that cannot be represented -- and every one of
    /// those is rendered identically by drawing no section at all. There is no
    /// empty state here.
    /// </remarks>
    public bool HasCommunity => _rollup.Community is not null;

    public string CommunityRankText =>
        _rollup.Community?.Rank is { } rank
            ? string.Format(CultureInfo.CurrentCulture, "#{0}", rank)
            : "—";

    public string CommunityCreditText =>
        _rollup.Community is { } standing
            ? string.Format(CultureInfo.CurrentCulture, "{0:0.0}", standing.NoveltyCredit)
            : "—";

    public string CommunityAcceptedText =>
        _rollup.Community is { } standing
            ? string.Format(
                CultureInfo.CurrentCulture,
                "{0} in {1}",
                standing.AcceptedInWindow,
                string.IsNullOrWhiteSpace(standing.WindowLabel) ? "the window" : standing.WindowLabel)
            : "—";

    /// <summary>
    /// A decimal in 0..=1 on the wire, not a percentage, and null is a dash
    /// rather than "0%" -- which would be a claim rather than an absence.
    /// </summary>
    public string CommunityAcceptRateText =>
        _rollup.Community?.AcceptRate is { } rate
            ? string.Format(CultureInfo.CurrentCulture, "{0:0}%", rate * 100)
            : "—";

    /// <summary>
    /// Analytics that are withheld are stated in words, never as an empty
    /// chart.
    /// </summary>
    public bool ShowsAnalyticsWithheld => _rollup.Community?.AnalyticsWithheld == true;

    public string AnalyticsWithheldText =>
        "Corpus analytics are withheld. The server publishes the roster on consent, but will "
        + "not publish aggregates without an approved noise mechanism -- so nothing is charted "
        + "here either.";

    // --- Outcomes ---------------------------------------------------------

    public bool HasOutcomes => Outcomes.Count > 0;

    public string OutcomesHeading => HistoryCopy.OutcomesHeading;

    public string OutcomesFootnote => HistoryCopy.OutcomesFootnote;

    // --- Reads ------------------------------------------------------------

    /// <summary>
    /// Reads the three history surfaces.
    ///
    /// Serialized by a gate for the same reason <see cref="MainViewModel"/>'s
    /// refresh is: two refreshes racing to rewrite one ObservableCollection
    /// produce flicker at best, and a refresh already in flight makes a second
    /// request redundant.
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

            DaemonResponse rollup = await _host
                .CallAsync(DaemonProtocol.Methods.HistoryRollup)
                .ConfigureAwait(true);
            DaemonResponse history = await _host
                .CallAsync(
                    DaemonProtocol.Methods.ListHistory,
                    JsonSerializer.Serialize(new Dictionary<string, int> { ["limit"] = 200 }))
                .ConfigureAwait(true);
            DaemonResponse outcomes = await _host
                .CallAsync(DaemonProtocol.Methods.QueueOutcomeCounts)
                .ConfigureAwait(true);

            // A rollup that cannot be read leaves the previous figures rather
            // than zeroing them: zeros drawn from a failed read are the same
            // false confidence a null `last_refreshed_at` exists to prevent.
            if (rollup.ResultAs<HistoryRollup>() is { } parsed)
            {
                _rollup = parsed;
            }

            if (history.IsError)
            {
                Notice = HistoryCopy.HistoryUnavailable;
            }
            else
            {
                ReplaceRecords(history.ResultAs<HistoryList>()?.History ?? new List<HistoryRecord>());
                _loaded = true;
            }

            ReplaceOutcomes(outcomes.ResultAs<QueueOutcomeCounts>()?.Reasons);
            RaiseEverything();
        }
        finally
        {
            IsBusy = false;
            _refreshGate.Release();
        }
    }

    /// <summary>
    /// The <c>refresh_history</c> control.
    /// </summary>
    /// <remarks>
    /// What this achieves, exactly: the daemon's background poller owns the
    /// network call, and <c>refresh_history</c> answers <c>requested: true</c>
    /// without making one. So the notice says the ask landed and nothing more
    /// -- see <see cref="HistoryCopy.CheckForUpdatesAsked"/>. History is
    /// re-read straight afterwards anyway, which is free and picks up anything
    /// the poller has already brought in since this screen was last drawn.
    /// </remarks>
    public async Task CheckForUpdatesAsync()
    {
        DaemonResponse response = await _host
            .CallAsync(DaemonProtocol.Methods.RefreshHistory)
            .ConfigureAwait(true);

        Notice = response.IsError
            ? HistoryCopy.HistoryUnavailable
            : HistoryCopy.CheckForUpdatesAsked;

        await RefreshAsync().ConfigureAwait(true);
    }

    // --- Withdrawal -------------------------------------------------------

    /// <summary>
    /// Withdraws one submission and reports the tier the server applied.
    /// </summary>
    /// <remarks>
    /// The confirmation is the caller's to have shown already -- it is a
    /// dialog, and dialogs belong to the view. What this method owes the
    /// contract is the part after the button:
    ///
    /// <list type="bullet">
    /// <item>the outcome is filed under the submission id, so it survives the
    /// re-read below;</item>
    /// <item>on success history is <b>re-read</b> rather than the row being
    /// optimistically flipped -- a row that claimed "withdrawn" on the
    /// strength of this process's optimism would be this screen asserting
    /// something only the daemon's cache can know;</item>
    /// <item>the record is never removed and never re-labelled as a failure.
    /// It comes back from the re-read reading as withdrawn, which is what it
    /// is.</item>
    /// </list>
    /// </remarks>
    public async Task WithdrawAsync(HistoryRecordViewModel record)
    {
        ArgumentNullException.ThrowIfNull(record);

        if (record.SubmissionId.Length == 0)
        {
            return;
        }

        string id = record.SubmissionId;
        _withdrawals[id] = WithdrawalAttempt.InFlight();
        RebuildRows();

        DaemonResponse response = await _host
            .CallAsync(
                DaemonProtocol.Methods.Withdraw,
                JsonSerializer.Serialize(
                    new Dictionary<string, string> { ["submission_id"] = id }))
            .ConfigureAwait(true);

        if (response.IsError)
        {
            // The label rides on the error's MESSAGE, not its code: the code
            // is the generic `unavailable` that a dozen other failures share,
            // and `account-session-required` -- the one contributors will
            // actually hit -- is only distinguishable in the message.
            _withdrawals[id] = WithdrawalAttempt.Failed(response.Error!.Message);
            RebuildRows();
            return;
        }

        _withdrawals[id] = WithdrawalAttempt.Done(
            response.ResultAs<WithdrawResult>()?.DistributionReach);

        await RefreshAsync().ConfigureAwait(true);
    }

    // --- Plumbing ---------------------------------------------------------

    private List<HistoryRecord> _lastPage = new();

    /// <summary>
    /// Clear-and-refill rather than a diff, matching the queue: the daemon is
    /// the sole authority on this list, and a diff would introduce an
    /// opportunity for the local view to disagree with it.
    /// </summary>
    private void ReplaceRecords(List<HistoryRecord> records)
    {
        _lastPage = records;
        RebuildRows();
    }

    /// <summary>
    /// Re-projects the last page through the current withdrawal outcomes,
    /// without another round trip. Used when only the local attempt state
    /// changed -- going in-flight, or a failure that keeps the button.
    /// </summary>
    private void RebuildRows()
    {
        Records.Clear();
        foreach (HistoryRecord record in _lastPage)
        {
            _withdrawals.TryGetValue(record.SubmissionId, out WithdrawalAttempt? attempt);
            Records.Add(new HistoryRecordViewModel(record, attempt));
        }

        Raise(nameof(IsEmpty));
    }

    private void ReplaceOutcomes(Dictionary<string, int>? reasons)
    {
        Outcomes.Clear();
        if (reasons is null)
        {
            return;
        }

        // Largest first, ties alphabetical, so the same counts always render
        // in the same order.
        foreach (KeyValuePair<string, int> pair in reasons
                     .OrderByDescending(pair => pair.Value)
                     .ThenBy(pair => pair.Key, StringComparer.Ordinal))
        {
            Outcomes.Add(new OutcomeCountViewModel(pair.Key, pair.Value));
        }
    }

    /// <summary>
    /// Raises everything the rollup feeds. Explicit rather than clever: the
    /// rollup is replaced wholesale by one read, so the properties derived
    /// from it all change at the same instant.
    /// </summary>
    private void RaiseEverything()
    {
        foreach (string name in new[]
                 {
                     nameof(InTheCommonsCountText),
                     nameof(HeldCountText),
                     nameof(WaitingCountText),
                     nameof(HasHeld),
                     nameof(HeldGroupHeading),
                     nameof(CreditRecordedText),
                     nameof(CreditPendingText),
                     nameof(HasFigures),
                     nameof(HasNoFigures),
                     nameof(RefreshedText),
                     nameof(HasCommunity),
                     nameof(CommunityRankText),
                     nameof(CommunityCreditText),
                     nameof(CommunityAcceptedText),
                     nameof(CommunityAcceptRateText),
                     nameof(ShowsAnalyticsWithheld),
                     nameof(HasOutcomes),
                     nameof(IsEmpty),
                 })
        {
            Raise(name);
        }
    }

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

/// <summary>
/// One <c>queue_outcome_counts</c> row: the daemon's own reason label and how
/// many entries carry it.
/// </summary>
/// <remarks>
/// The label is shown tidied but not remapped. It is already a fixed,
/// contributor-readable label by contract, and a second vocabulary here would
/// drift from the daemon's -- the same reasoning the queue row's
/// <c>ReasonLabel</c> follows.
/// </remarks>
public sealed class OutcomeCountViewModel
{
    public OutcomeCountViewModel(string label, int count)
    {
        Label = string.IsNullOrWhiteSpace(label)
            ? "—"
            : label.Replace('-', ' ').Replace('_', ' ');
        CountText = count.ToString(CultureInfo.CurrentCulture);
    }

    public string Label { get; }

    public string CountText { get; }
}
