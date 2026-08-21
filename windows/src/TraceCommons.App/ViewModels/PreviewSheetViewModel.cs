using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Globalization;
using System.Runtime.CompilerServices;
using System.Threading.Tasks;
using TraceCommons.Interop;

namespace TraceCommons.App.ViewModels;

/// <summary>Which tab of the preview sheet is showing.</summary>
public enum PreviewTab
{
    /// <summary>
    /// First and focused, always. "Does this mention my client's name?" is a
    /// question a contributor can answer in five seconds; judging redaction
    /// quality by eye is not, and this sheet never asks them to.
    /// </summary>
    Search,
    WhatsInIt,
    Transcript,
    Permissions,
}

/// <summary>
/// "Look inside": the one surface in this app that deliberately shows trace
/// content, because consent to send something you cannot see is not consent.
///
/// <para>
/// Four tabs in the shared spec's order, and <b>Contribute exists here and
/// nowhere else</b>. The queue row has no approve button on purpose:
/// approving from the row is approving without looking, which is the misclick
/// the preview-then-approve rule exists to prevent.
/// </para>
/// <para>
/// The invariant this whole class serves is that <b>an approval covers exactly
/// the bytes a preview pinned</b>. It is enforced by <see cref="Gate"/>, which
/// lives in the interop assembly so it can be tested on a machine that cannot
/// build WinUI, and it is the only thing that arms
/// <see cref="CanContribute"/>. It used to enforce two more conditions -- a
/// transcript shown and an acknowledgement ticked -- and <see cref="ReadGate"/>
/// records why they went and what took their place. The Linux shell applies the
/// same rule in <c>sync_contribute</c>; the macOS sheet applies it through
/// <c>TCShellCore.ReadGate</c>.
/// </para>
/// <para>
/// <b>One sheet, one session, one decision.</b> Both decisions close the
/// sheet. It does not load the next waiting session into itself, which the
/// shared spec's "Approving" section describes and the macOS sheet
/// deliberately stopped doing: that put Contribute under the same pixels for a
/// second session, so one more click sent a transcript nobody had looked at,
/// and it stranded the recovery bar behind a sheet where it could not be seen.
/// A sheet that advanced would also have to re-pin under the contributor's
/// cursor, which is a worse thing to get wrong than an extra click is to
/// require.
/// </para>
/// </summary>
public sealed class PreviewSheetViewModel : INotifyPropertyChanged, IDisposable
{
    /// <summary>
    /// Shown where the sheet body would be while the redaction pass runs.
    /// </summary>
    public const string LoadingTitle = "Scrubbing it locally…";

    public const string LoadingDetail = "Reading the session and running the redaction pass.";

    /// <summary>
    /// A preview that could not be opened or could not be understood. The
    /// second sentence is the promise that makes the failure survivable.
    /// </summary>
    public const string FailureTitle = "This one can't be shown.";

    public const string FailureDetail =
        "Nothing has been sent, and nothing will be until it can be shown to you.";

    /// <summary>
    /// The scrubbing caveat, word for word as the queue window prints it and
    /// as the macOS and Linux shells print it.
    ///
    /// It has to be identical everywhere it appears, so that a person who read
    /// it under the queue recognises it above Contribute rather than reading a
    /// second, weaker message. Do not reword it here.
    /// </summary>
    public const string ScrubbingCaveat =
        "Scrubbing is pattern-based. It misses things it hasn't seen before.";

    /// <summary>
    /// Recent searches, kept for the life of the process and never written to
    /// disk.
    /// </summary>
    /// <remarks>
    /// The shared spec asks for these to persist so the second trace is one
    /// keystroke, and the macOS shell persists them. This one deliberately
    /// does not: a recent search is the contributor's own list of the things
    /// they are worried about leaking -- a client's name, an internal code
    /// name, an address -- and writing that list to disk creates a small file
    /// of exactly the material the rest of the app works to keep on the
    /// machine's own terms. In-session recall covers the case the spec argues
    /// for, which is checking several traces for the same term in one sitting.
    /// </remarks>
    private static readonly List<string> ProcessRecentSearches = new();

    private readonly DaemonHost _host;

    private TcPreview? _preview;
    private PreviewSummary? _summary;
    private string _transcript = string.Empty;
    private string _needle = string.Empty;
    private IReadOnlyList<int> _matches = Array.Empty<int>();
    private bool _searched;
    private bool _searchFailed;
    private bool _loading = true;
    private bool _failed;
    private bool _deciding;
    private PreviewTab _tab = PreviewTab.Search;

    public PreviewSheetViewModel(DaemonHost host, QueueEntryViewModel entry)
    {
        _host = host ?? throw new ArgumentNullException(nameof(host));
        Entry = entry ?? throw new ArgumentNullException(nameof(entry));

        // Every gate transition re-raises the properties the footer binds to,
        // so there is no path that changes a condition without the button
        // noticing.
        Gate.Changed += OnGateChanged;
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    /// <summary>
    /// Raised once the contributor has decided, with the approval's hold when
    /// there is one to undo against.
    /// </summary>
    /// <remarks>
    /// The undo belongs to the queue window, not to this sheet: recovery has
    /// to live on the screen a contributor lands on after deciding, not behind
    /// a sheet that has already closed.
    /// </remarks>
    public event Action<PreviewDecision>? Decided;

    public QueueEntryViewModel Entry { get; }

    /// <summary>The consent invariant. See <see cref="ReadGate"/>.</summary>
    public ReadGate Gate { get; } = new();

    /// <summary>Matched excerpts for the current search, newest search only.</summary>
    public ObservableCollection<string> Excerpts { get; } = new();

    /// <summary>The contributor's earlier search terms, for one-click recall.</summary>
    public ObservableCollection<string> RecentSearches { get; } = new();

    public bool IsLoading
    {
        get => _loading;
        private set
        {
            if (Set(ref _loading, value))
            {
                Raise(nameof(IsShowingContent));
            }
        }
    }

    public bool HasFailed
    {
        get => _failed;
        private set
        {
            if (Set(ref _failed, value))
            {
                Raise(nameof(IsShowingContent));
            }
        }
    }

    public bool IsShowingContent => !IsLoading && !HasFailed;

    /// <summary>
    /// The redacted transcript: the exact bytes an approval covers.
    ///
    /// Trace content. It is bound to a text control and to nothing else --
    /// never a log line, never an error string, never a notification.
    /// </summary>
    public string Transcript
    {
        get => _transcript;
        private set => Set(ref _transcript, value);
    }

    public PreviewTab Tab
    {
        get => _tab;
        private set
        {
            if (!Set(ref _tab, value))
            {
                return;
            }

            Raise(nameof(IsSearchTab));
            Raise(nameof(IsWhatsInItTab));
            Raise(nameof(IsTranscriptTab));
            Raise(nameof(IsPermissionsTab));
        }
    }

    public bool IsSearchTab => Tab == PreviewTab.Search;
    public bool IsWhatsInItTab => Tab == PreviewTab.WhatsInIt;
    public bool IsTranscriptTab => Tab == PreviewTab.Transcript;
    public bool IsPermissionsTab => Tab == PreviewTab.Permissions;

    /// <summary>What would leave this machine, in bytes, from the preview.</summary>
    /// <remarks>
    /// Deliberately not the queue row's figure. A queue entry's
    /// <c>size_bytes</c> is the session file on disk; "would send" is the
    /// redacted envelope, which is usually larger because it also carries
    /// schema, consent and privacy metadata. Only a preview knows it, which is
    /// why only this screen prints it.
    /// </remarks>
    public string WouldSendText =>
        _summary is null ? "—" : QueueEntryViewModel.FormatBytes(_summary.WouldSendBytes);

    public string RawSessionText =>
        _summary is null ? "—" : QueueEntryViewModel.FormatBytes(_summary.RawSessionBytes);

    /// <summary>"12 secrets · 4 tokens", or "nothing matched".</summary>
    public string ScrubbingFoundText => _summary?.ScrubbingFound ?? "—";

    /// <summary>
    /// True when scrubbing removed nothing, which is the one manifest state
    /// drawn to be found rather than reassured over: a session that obviously
    /// touched credentials and matched no pattern is worth a second look.
    /// </summary>
    public bool NothingMatched => _summary is not null && _summary.Redactions.Count == 0;

    public string TurnsText =>
        _summary is null
            ? "—"
            : _summary.EventCount.ToString("N0", CultureInfo.CurrentCulture);

    public string ResidualRiskText =>
        string.IsNullOrWhiteSpace(_summary?.ResidualRisk)
            ? "—"
            : _summary!.ResidualRisk.Replace('_', ' ');

    /// <summary>Category labels only. The matched text is never reported.</summary>
    public string PiiLabelsText =>
        _summary is null || _summary.PiiLabelsPresent.Count == 0
            ? string.Empty
            : string.Join(", ", _summary.PiiLabelsPresent);

    public bool HasPiiLabels => PiiLabelsText.Length > 0;

    /// <summary>
    /// One row per category scrubbing removed, for "What's in it".
    /// </summary>
    public ObservableCollection<string> RedactionRows { get; } = new();

    /// <summary>
    /// The scopes this upload asks for, restated at the moment of consent
    /// rather than only at onboarding.
    /// </summary>
    public ObservableCollection<PermissionRow> Permissions { get; } = new();

    /// <summary>Badge on the "What's in it" tab: how much scrubbing removed.</summary>
    public string RedactionBadge =>
        _summary is null || _summary.TotalRedactions == 0
            ? string.Empty
            : _summary.TotalRedactions.ToString(CultureInfo.CurrentCulture);

    public bool HasRedactionBadge => RedactionBadge.Length > 0;

    public string PermissionsBadge =>
        _summary is null
            ? string.Empty
            : _summary.ConsentScopes.Count.ToString(CultureInfo.CurrentCulture);

    public string Needle
    {
        get => _needle;
        set => Set(ref _needle, value ?? string.Empty);
    }

    /// <summary>The answer to the only question the Search tab exists for.</summary>
    public string SearchResultText =>
        !_searched || Needle.Length == 0
            ? "Type to search. Nothing is sent while you look."
            : _searchFailed ? "The search couldn't run on this trace."
            : _matches.Count == 0 ? "0 matches"
            : _matches.Count == 1 ? "1 match"
            : $"{_matches.Count} matches";

    /// <summary>
    /// True for a search that found nothing, so the caveat beneath it can be
    /// shown. A search finding nothing is not evidence that nothing is there.
    /// </summary>
    public bool ShowNothingMatchedNote =>
        _searched && Needle.Length > 0 && !_searchFailed && _matches.Count == 0;

    public string NothingMatchedNote =>
        "A search only finds what is written the way you typed it. If it matters, try the "
        + "other spellings you would worry about — a hostname, an internal code name, an address.";

    /// <summary>Whether there is a pinned preview to contribute, and no
    /// decision already in flight.</summary>
    public bool CanContribute => Gate.CanContribute && !_deciding;

    public bool CanDecide => !_deciding;

    public string ContributeHelp => Gate.Help;

    /// <summary>
    /// What the sheet says about redaction above Contribute. Always shown,
    /// because it is a statement about the mechanism and not a report on
    /// the state of anything.
    /// </summary>
    public string GateStatement => ReadGate.Statement;

    public void SelectTab(PreviewTab tab) => Tab = tab;

    /// <summary>
    /// Opens the preview and fills the sheet.
    ///
    /// Every failure path ends with the gate unpinned, so a sheet that could
    /// not show the bytes cannot approve them.
    /// </summary>
    public async Task LoadAsync()
    {
        IsLoading = true;
        HasFailed = false;

        TcPreview preview;
        try
        {
            preview = await _host.OpenPreviewAsync(Entry.EntryId).ConfigureAwait(true);
        }
        catch (TcException)
        {
            // The ABI label is not interpolated into the message shown. It is
            // a fixed label and safe, but "this one can't be shown" plus the
            // promise underneath is what a contributor needs, and the label
            // would only invite them to debug the daemon.
            Fail();
            return;
        }

        _preview = preview;
        Transcript = preview.Body;

        PreviewSummary? summary = PreviewSummary.Parse(preview.SummaryJson);
        if (summary is null)
        {
            Fail();
            return;
        }

        _summary = summary;
        FillManifest(summary);

        // An unenrolled preview is an illustration: it was built from a
        // placeholder identity, nothing was pinned, and no approval can bind
        // to it. The gate holds Contribute shut and says so.
        Gate.SetPinnedPreview(summary.Enrolled);

        foreach (string term in ProcessRecentSearches)
        {
            RecentSearches.Add(term);
        }

        IsLoading = false;
    }

    /// <summary>
    /// Runs the search over the redacted body.
    /// </summary>
    /// <remarks>
    /// On the UI thread deliberately, as the macOS sheet does: the scan is a
    /// local in-memory pass, and keeping every touch of the <c>tc_preview*</c>
    /// pointer on one thread is what the ABI header asks for. Its wrong-pointer
    /// check narrows accidental misuse to an error; it does not make concurrent
    /// use safe.
    /// </remarks>
    public void RunSearch()
    {
        _searched = true;
        _searchFailed = false;
        Excerpts.Clear();

        if (Needle.Length == 0 || _preview is null)
        {
            _matches = Array.Empty<int>();
            RaiseSearchResults();
            return;
        }

        try
        {
            _matches = _preview.Search(Needle);
        }
        catch (TcException)
        {
            _matches = Array.Empty<int>();
            _searchFailed = true;
            RaiseSearchResults();
            return;
        }
        catch (ObjectDisposedException)
        {
            _matches = Array.Empty<int>();
            _searchFailed = true;
            RaiseSearchResults();
            return;
        }

        foreach (string excerpt in SearchContexts.Build(Transcript, Needle, _matches))
        {
            Excerpts.Add(excerpt);
        }

        if (_matches.Count > 0)
        {
            Remember(Needle);
        }

        RaiseSearchResults();
    }

    /// <summary>
    /// "Not this one": skips this session only, and says as much in its
    /// tooltip. The project keeps being offered, which is what makes dismiss
    /// and ignore different decisions rather than the same button.
    /// </summary>
    public async Task DismissAsync()
    {
        if (_deciding)
        {
            return;
        }

        SetDeciding(true);

        DaemonResponse response = await _host
            .CallAsync(DaemonProtocol.Methods.Dismiss, EntryParams())
            .ConfigureAwait(true);

        SetDeciding(false);
        Decided?.Invoke(
            response.IsError
                ? PreviewDecision.Failed("That couldn't be skipped just now. Nothing has been sent.")
                : PreviewDecision.Dismissed());
    }

    /// <summary>
    /// The one irreversible click in the product.
    ///
    /// It is behind the preview by design -- it cannot arm until one has
    /// loaded and pinned -- and it carries no keyboard accelerator: an
    /// approval one Return away from a hand resting on the keyboard is the
    /// misclick this sheet was built to make impossible.
    /// </summary>
    public async Task ContributeAsync()
    {
        // Re-checked here rather than trusted from the button's enabled state.
        // The gate is the invariant; a disabled control is only how it is
        // usually expressed.
        if (!CanContribute)
        {
            return;
        }

        SetDeciding(true);

        DaemonResponse response = await _host
            .CallAsync(DaemonProtocol.Methods.Approve, EntryParams())
            .ConfigureAwait(true);

        SetDeciding(false);

        if (response.IsError)
        {
            Decided?.Invoke(
                PreviewDecision.Failed(
                    "That couldn't be approved just now. Nothing has been sent."));
            return;
        }

        Decided?.Invoke(PreviewDecision.Approved(ApprovalHold.Parse(response)));
    }

    /// <summary>
    /// Frees the preview.
    ///
    /// The native body dies with it, which is the point: the one content
    /// exemption in the ABI is bounded to an open sheet and does not outlive
    /// the window that asked for it.
    /// </summary>
    public void Dispose()
    {
        Gate.Changed -= OnGateChanged;
        _preview?.Dispose();
        _preview = null;
    }

    private string EntryParams() =>
        System.Text.Json.JsonSerializer.Serialize(
            new Dictionary<string, string> { ["entry_id"] = Entry.EntryId });

    private void FillManifest(PreviewSummary summary)
    {
        RedactionRows.Clear();
        foreach (KeyValuePair<string, int> pair in summary.Redactions)
        {
            RedactionRows.Add(
                string.Format(
                    CultureInfo.CurrentCulture,
                    "{0} × {1}",
                    pair.Value,
                    pair.Key.Replace('_', ' ')));
        }

        Permissions.Clear();
        foreach (string scope in summary.ConsentScopes)
        {
            Permissions.Add(new PermissionRow(ConsentScopeViewModel.ScopeTitle(scope)));
        }

        Raise(nameof(WouldSendText));
        Raise(nameof(RawSessionText));
        Raise(nameof(ScrubbingFoundText));
        Raise(nameof(NothingMatched));
        Raise(nameof(TurnsText));
        Raise(nameof(ResidualRiskText));
        Raise(nameof(PiiLabelsText));
        Raise(nameof(HasPiiLabels));
        Raise(nameof(RedactionBadge));
        Raise(nameof(HasRedactionBadge));
        Raise(nameof(PermissionsBadge));
    }

    private void Fail()
    {
        // Unpin first. A sheet that cannot show the bytes must not be able to
        // approve them, and this is the line that guarantees it regardless of
        // which failure got here.
        Gate.SetPinnedPreview(false);
        _summary = null;
        Transcript = string.Empty;
        IsLoading = false;
        HasFailed = true;
    }

    private void Remember(string term)
    {
        ProcessRecentSearches.Remove(term);
        ProcessRecentSearches.Insert(0, term);
        while (ProcessRecentSearches.Count > 6)
        {
            ProcessRecentSearches.RemoveAt(ProcessRecentSearches.Count - 1);
        }

        RecentSearches.Clear();
        foreach (string recent in ProcessRecentSearches)
        {
            RecentSearches.Add(recent);
        }
    }

    private void SetDeciding(bool deciding)
    {
        _deciding = deciding;
        Raise(nameof(CanContribute));
        Raise(nameof(CanDecide));
    }

    private void OnGateChanged()
    {
        Raise(nameof(CanContribute));
        Raise(nameof(ContributeHelp));
    }

    private void RaiseSearchResults()
    {
        Raise(nameof(SearchResultText));
        Raise(nameof(ShowNothingMatchedNote));
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

/// <summary>One scope row in the Permissions tab.</summary>
public sealed class PermissionRow
{
    public PermissionRow(string title)
    {
        Title = title;
    }

    public string Title { get; }
}

/// <summary>What the contributor decided, handed back to the queue window.</summary>
public sealed class PreviewDecision
{
    private PreviewDecision(PreviewOutcome outcome, ApprovalHold? hold, string? message)
    {
        Outcome = outcome;
        Hold = hold;
        Message = message;
    }

    public PreviewOutcome Outcome { get; }

    /// <summary>
    /// The daemon's hold on an approval, or null when it granted none. Null
    /// means no undo may be offered, which the queue window says plainly
    /// rather than drawing a button that would fail.
    /// </summary>
    public ApprovalHold? Hold { get; }

    /// <summary>A fixed sentence for a decision the daemon refused.</summary>
    public string? Message { get; }

    public static PreviewDecision Approved(ApprovalHold? hold) =>
        new(PreviewOutcome.Approved, hold, null);

    public static PreviewDecision Dismissed() => new(PreviewOutcome.Dismissed, null, null);

    public static PreviewDecision Failed(string message) =>
        new(PreviewOutcome.Failed, null, message);
}

public enum PreviewOutcome
{
    Approved,
    Dismissed,
    Failed,
}
