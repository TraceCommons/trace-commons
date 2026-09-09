using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Runtime.CompilerServices;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Threading.Tasks;
using TraceCommons.Interop;

namespace TraceCommons.App.ViewModels;

/// <summary>
/// The six onboarding screens from the shared design spec, as state.
/// </summary>
/// <remarks>
/// <para>
/// Until this existed the Windows app could not enrol anyone: there was no
/// invite handling in it at all, so an app-only contributor had to install
/// the CLI and run <c>login</c> there. None of it needed new protocol --
/// every method called here was already in the daemon's pinned
/// <c>METHODS</c> array and reachable through <c>tc_call</c>. What was
/// missing was the screens.
/// </para>
/// <para>
/// Three behaviours here are contract rather than styling, and each is
/// commented where it is enforced: one failure sentence for the whole invite
/// path, scope rows that come from <c>consent_options</c> rather than a
/// table in this file, and a completion flag that is not
/// <c>status.logged_in</c>.
/// </para>
/// </remarks>
public sealed class OnboardingViewModel : INotifyPropertyChanged
{
    private static readonly OnboardingCopy? SharedOnboardingCopy = OnboardingCopy.Load();

    /// The shared payload, for the login-enrolment offer's own sentences.
    /// Loaded once for the same reason the onboarding copy is: these are
    /// facts about the build, not about a running daemon.
    private static readonly PrivateInferenceCopy? SharedPrivateInferenceCopy =
        PrivateInferenceSurface.Copy();
    public string WelcomeBody => SharedOnboardingCopy?.WelcomeBody ?? "";
    public string DoneBody => SharedOnboardingCopy?.DoneBody ?? "";
    /// <summary>
    /// The single sentence shown for every invite failure.
    /// </summary>
    /// <remarks>
    /// <c>enroll</c> answers <c>enroll-failed</c> and never echoes the
    /// underlying HTTP condition, so an invite this app rejected before
    /// sending and one the daemon refused are reported identically. Anything
    /// more specific would either invent detail the daemon withheld or leak
    /// the detail it deliberately withheld.
    /// </remarks>
    public const string InviteFailed =
        "This invite link is no longer valid. Ask whoever sent it for a new one.";

    private readonly DaemonHost _host;
    private readonly OnboardingState _state;

    private OnboardingStep _step = OnboardingStep.Welcome;
    private string _invite = string.Empty;
    private string _instanceLine = string.Empty;
    private bool _inviteFailed;
    private string _projectNotice = string.Empty;
    private bool _isBusy;
    private bool _scanOffered;
    private bool _useNearAiScan;
    private string? _tenantId;

    public OnboardingViewModel(DaemonHost host, OnboardingState state)
    {
        _host = host ?? throw new ArgumentNullException(nameof(host));
        _state = state ?? throw new ArgumentNullException(nameof(state));
        NearAccount = new NearAccountConnection((method, payload) => _host.CallAsync(method, payload),
            async uri => await Windows.System.Launcher.LaunchUriAsync(uri), () => !IsBusy);
        NearAccount.PropertyChanged += (_, _) => { Raise(nameof(CanConnect)); Raise(nameof(CanUseInvite)); };
        NearAccount.Completed += OnNearAccountCompleted;

    }

    public event PropertyChangedEventHandler? PropertyChanged;

    /// <summary>Raised when the flow reaches its end and the window may close.</summary>
    public event Action? Finished;

    public ObservableCollection<ConsentScopeViewModel> AlwaysIncluded { get; } = new();

    public ObservableCollection<ConsentScopeViewModel> Optional { get; } = new();

    /// <summary>
    /// Scopes granting no data use at all, which is what
    /// <c>grants_data_use: false</c> means. Kept in their own group because
    /// presenting one beside four real data-use scopes with equal weight
    /// would mislead in both directions.
    /// </summary>
    public ObservableCollection<ConsentScopeViewModel> Credit { get; } = new();

    public OnboardingStep Step
    {
        get => _step;
        private set
        {
            if (_step == value)
            {
                return;
            }

            _step = value;
            Raise(nameof(Step));
            Raise(nameof(IsWelcome));
            Raise(nameof(IsConnect));
            Raise(nameof(IsConsent));
            Raise(nameof(IsScan));
            Raise(nameof(IsWatch));
            Raise(nameof(IsDone));
        }
    }

    // --- Joining with a NEAR AI login -----------------------------------
    //
    // The way in that needs no wallet, drawn beside the wallet ceremony and
    // replacing none of it. A contributor cannot produce an admissible
    // receipt without a NEAR AI account in the first place, so requiring a
    // wallet as well is a second onboarding for an identity they already
    // hold.
    //
    // Nothing here authors a sentence and nothing branches on a control name.
    // The ten refusals reach NearAiEnrollSurface, which asks the shared table.

    private string _nearAiCommons = string.Empty;
    private bool _nearAiPending;
    private bool _nearAiJoined;
    private string? _nearAiRefusal;

    /// <summary>
    /// Whether the daemon reports a usable NEAR AI sign-in.
    /// </summary>
    /// <remarks>
    /// False until the first status read answers, which is the reading that
    /// claims less: a card that has not been told yet must not offer a
    /// control whose only outcome is <c>near_ai_enroll_no_session</c>.
    /// </remarks>
    private bool _nearAiSignedIn;

    public string NearAiCommons
    {
        get => _nearAiCommons;
        set
        {
            if (_nearAiCommons == value) return;
            _nearAiCommons = value;
            Raise(nameof(NearAiCommons));
            Raise(nameof(CanJoinWithNearAi));
        }
    }

    public string NearAiTitle => SharedPrivateInferenceCopy?.NearAiEnrollTitle ?? string.Empty;
    public string NearAiWhat => SharedPrivateInferenceCopy?.NearAiEnrollWhat ?? string.Empty;
    public string NearAiAction => SharedPrivateInferenceCopy?.NearAiEnrollAction ?? string.Empty;
    public string NearAiWorking => _nearAiPending ? SharedPrivateInferenceCopy?.NearAiEnrollWorking ?? string.Empty : string.Empty;
    public string NearAiDone => _nearAiJoined ? SharedPrivateInferenceCopy?.NearAiEnrollDone ?? string.Empty : string.Empty;

    /// <summary>
    /// The step to take when there is no sign-in yet. Not a refusal: nothing
    /// went wrong and nothing was attempted.
    /// </summary>
    public string NearAiNeedsLogin =>
        _nearAiSignedIn || _nearAiJoined ? string.Empty : SharedPrivateInferenceCopy?.NearAiEnrollNeedsLogin ?? string.Empty;

    /// <summary>The daemon's refusal, in the shared table's words.</summary>
    public string NearAiRefusalMessage =>
        _nearAiRefusal is null ? string.Empty : NearAiEnrollSurface.Line(_nearAiRefusal) ?? string.Empty;

    /// <summary>
    /// The control is OFFERED only where it can succeed, and hidden rather
    /// than disabled otherwise.
    /// </summary>
    public bool CanOfferNearAiJoin => _nearAiSignedIn && !_nearAiJoined;

    public bool CanJoinWithNearAi =>
        CanOfferNearAiJoin && !_nearAiPending && !string.IsNullOrWhiteSpace(_nearAiCommons);

    /// <summary>
    /// Reads whether a sign-in exists before offering a control that would
    /// otherwise only refuse.
    /// </summary>
    public async Task RefreshNearAiSignInAsync()
    {
        DaemonResponse response = await _host.CallAsync("near_ai_credential_status").ConfigureAwait(true);
        bool present = !response.IsError
            && response.Result is { ValueKind: System.Text.Json.JsonValueKind.Object } value
            && value.TryGetProperty("state", out var state)
            && state.ValueKind == System.Text.Json.JsonValueKind.String
            && state.GetString() == NearAiEnrollSurface.CredentialStatePresent;
        _nearAiSignedIn = present;
        RaiseNearAi();
    }

    /// <summary>
    /// Joins with the retained NEAR AI login.
    /// </summary>
    /// <remarks>
    /// The refusal is stored as the daemon's control name, never as a
    /// sentence: this view model does not know which of the ten it is and
    /// must not guess.
    /// </remarks>
    public async Task JoinWithNearAiAsync()
    {
        if (!CanJoinWithNearAi) return;
        _nearAiPending = true;
        _nearAiRefusal = null;
        RaiseNearAi();
        try
        {
            DaemonResponse response = await _host
                .CallAsync("near_ai_account_enroll", NearAiEnrollRequest(_nearAiCommons.Trim()))
                .ConfigureAwait(true);
            if (response.IsError)
            {
                // `message` is the daemon's control name -- the enrolment
                // handler answers a label and nothing else, because the
                // errors underneath can quote a remote body or a URL.
                string label = response.Error?.Message ?? string.Empty;
                _nearAiRefusal = string.IsNullOrEmpty(label)
                    ? NearAiEnrollSurface.Unavailable
                    : label;
            }
            else
            {
                // Read from the answer rather than assumed from the absence
                // of an error: a response that never said it enrolled is not
                // one to celebrate.
                _nearAiJoined = response.Result is { ValueKind: System.Text.Json.JsonValueKind.Object } ok
                    && ok.TryGetProperty("enrolled", out var enrolled)
                    && enrolled.ValueKind == System.Text.Json.JsonValueKind.True;
                if (_nearAiJoined) OnNearAccountCompleted();
            }
        }
        catch
        {
            _nearAiRefusal = NearAiEnrollSurface.Unavailable;
        }
        finally
        {
            _nearAiPending = false;
            RaiseNearAi();
        }
    }

    private static string NearAiEnrollRequest(string commons) =>
        System.Text.Json.JsonSerializer.Serialize(new { ingest_url = commons });

    private void RaiseNearAi()
    {
        Raise(nameof(CanOfferNearAiJoin));
        Raise(nameof(CanJoinWithNearAi));
        Raise(nameof(NearAiNeedsLogin));
        Raise(nameof(NearAiRefusalMessage));
        Raise(nameof(NearAiWorking));
        Raise(nameof(NearAiDone));
    }

    public bool IsWelcome => Step == OnboardingStep.Welcome;
    public bool IsConnect => Step == OnboardingStep.Connect;
    public bool IsConsent => Step == OnboardingStep.Consent;
    public bool IsScan => Step == OnboardingStep.Scan;
    public bool IsWatch => Step == OnboardingStep.Watch;
    public bool IsDone => Step == OnboardingStep.Done;

    /// <summary>
    /// The invite text. A credential: it is read once, on Connect, and
    /// cleared as soon as <c>enroll</c> accepts it.
    /// </summary>
    public string Invite
    {
        get => _invite;
        set
        {
            if (!Set(ref _invite, value))
            {
                return;
            }

            // A fresh keystroke is not a failure. The failure sentence
            // belongs to a submitted invite, not a half-pasted one.
            InviteFailedVisible = false;
            InstanceLine = string.Empty;
            Raise(nameof(CanConnect));
        }
    }

    /// <summary>
    /// "This invite is for host." -- the spec asks that the instance be
    /// resolved and shown before committing.
    /// </summary>
    public string InstanceLine
    {
        get => _instanceLine;
        private set => Set(ref _instanceLine, value);
    }

    public bool HasInstanceLine => !string.IsNullOrEmpty(_instanceLine);

    public bool InviteFailedVisible
    {
        get => _inviteFailed;
        private set => Set(ref _inviteFailed, value);
    }

    /// <summary>
    /// What screen 5 says when a project-mode write did not land, empty when
    /// there is nothing to report.
    ///
    /// Settings sets a notice on the same refusal. Without one here a refused
    /// write is indistinguishable from a click that did nothing: the button
    /// re-enables, the row reads the same, and the contributor is left believing
    /// a consent field changed when it did not.
    /// </summary>
    public string ProjectNotice
    {
        get => _projectNotice;
        private set
        {
            if (Set(ref _projectNotice, value))
            {
                Raise(nameof(HasProjectNotice));
            }
        }
    }

    public bool HasProjectNotice => _projectNotice.Length > 0;

    public NearAccountConnection NearAccount { get; }
    public bool CanUseWallet => !IsBusy;
    public bool CanUseInvite => !IsBusy && !NearAccount.Busy;
    public bool CanConnect => !string.IsNullOrWhiteSpace(_invite) && CanUseInvite;
    private async void OnNearAccountCompleted()
    {
        Invite = string.Empty;
        await LoadConsentOptionsAsync().ConfigureAwait(true);
        Step = OnboardingStep.Consent;
    }


    public bool IsBusy
    {
        get => _isBusy;
        private set
        {
            if (!Set(ref _isBusy, value))
            {
                return;
            }

            Raise(nameof(CanConnect));
            Raise(nameof(CanUseInvite));
            Raise(nameof(CanUseWallet));
        }
    }

    /// <summary>
    /// Whether the operator offers the second scanner, from
    /// <c>get_settings</c>. Screen 4 is skipped when they do not, rather than
    /// offering a choice between one option and an option that does not exist.
    /// </summary>
    public bool ScanOffered
    {
        get => _scanOffered;
        private set => Set(ref _scanOffered, value);
    }

    public bool UseNearAiScan
    {
        get => _useNearAiScan;
        set => Set(ref _useNearAiScan, value);
    }

    public ObservableCollection<ProjectViewModel> Projects { get; } = new();

    /// <summary>
    /// True when the daemon reported no projects at all, so screen 5 can say
    /// so rather than rendering a heading above nothing.
    /// </summary>
    public bool HasNoProjects => Projects.Count == 0;

    // Screen 5's words come from WatchCopy rather than being repeated as XAML
    // literals, so the strings the tests check are the strings that render.
    // The same idiom the roots window uses.
    public string WatchSubtitle => WatchCopy.Subtitle;

    public string WatchSection => WatchCopy.Section.ToUpperInvariant();

    public string WatchEmpty => WatchCopy.Empty;

    /// <summary>
    /// The link under screen 1's scrubbing paragraph. Bound rather than
    /// repeated so the label and the dialog it opens cannot drift apart.
    /// </summary>
    public string WhatGetsRemovedLabel => ScrubDetectorCopy.LinkLabel;

    public void GetStarted() => Step = OnboardingStep.Connect;

    /// <summary>
    /// Fills the invite from a <c>tracecommons://</c> deep link and opens on
    /// Connect.
    /// </summary>
    /// <remarks>
    /// It fills the field and stops. A URL handler is not a person agreeing
    /// to join a particular commons, and that agreement is the decision the
    /// Connect screen exists to ask for, so the button is still left to
    /// press.
    /// </remarks>
    public void OfferInvite(string invite)
    {
        Invite = invite;
        Step = OnboardingStep.Connect;
    }

    /// <summary>
    /// Sends the invite to <c>enroll</c>.
    /// </summary>
    public async Task ConnectAsync()
    {
        if (!CanConnect)
        {
            return;
        }

        IsBusy = true;
        InviteFailedVisible = false;

        try
        {
            // No `scopes` here on purpose. Absent means floor-scope-only, and
            // the scopes screen is next; sending a guess now would grant
            // something the contributor has not been asked about yet.
            string payload = JsonSerializer.Serialize(new EnrollRequest { Invite = _invite });
            DaemonResponse response = await _host
                .CallAsync(DaemonProtocol.Methods.Enroll, payload)
                .ConfigureAwait(true);

            if (response.IsError)
            {
                // Deliberately not reading response.Error.Code into the UI.
                InviteFailedVisible = true;
                return;
            }

            // The field held a credential and its work is done.
            Invite = string.Empty;

            await LoadConsentOptionsAsync().ConfigureAwait(true);
            Step = OnboardingStep.Consent;
        }
        finally
        {
            IsBusy = false;
        }
    }

    /// <summary>
    /// Reads the scope list and whether screen 4 has anything to offer.
    /// </summary>
    public async Task LoadConsentOptionsAsync()
    {
        DaemonResponse options = await _host
            .CallAsync(DaemonProtocol.Methods.ConsentOptions)
            .ConfigureAwait(true);

        AlwaysIncluded.Clear();
        Optional.Clear();
        Credit.Clear();

        // The list and the descriptions are the daemon's, never a table in
        // this file, so an operator who changes them changes what this screen
        // says without a new client.
        ConsentOptionsPayload? parsed = options.ResultAs<ConsentOptionsPayload>();
        foreach (ConsentOption scope in parsed?.Scopes ?? new List<ConsentOption>())
        {
            var row = new ConsentScopeViewModel(scope);
            if (scope.AlwaysOn)
            {
                AlwaysIncluded.Add(row);
            }
            else if (scope.GrantsDataUse)
            {
                Optional.Add(row);
            }
            else
            {
                Credit.Add(row);
            }
        }

        DaemonResponse settings = await _host
            .CallAsync(DaemonProtocol.Methods.GetSettings)
            .ConfigureAwait(true);

        ScanOffered = settings.Result is JsonElement element
                      && element.TryGetProperty("near_ai_configured", out JsonElement configured)
                      && configured.ValueKind == JsonValueKind.True;
    }

    /// <summary>
    /// Sends the chosen scopes and moves on.
    /// </summary>
    public async Task ConfirmConsentAsync()
    {
        var chosen = new List<string>();
        foreach (ConsentScopeViewModel row in Optional)
        {
            if (row.IsSelected)
            {
                chosen.Add(row.Name);
            }
        }

        foreach (ConsentScopeViewModel row in Credit)
        {
            if (row.IsSelected)
            {
                chosen.Add(row.Name);
            }
        }

        // The floor scope is not sent: it is not optional, and the daemon
        // validates what arrives against the same list the options came from.
        string payload = JsonSerializer.Serialize(new ScopesRequest { Scopes = chosen });
        await _host
            .CallAsync(DaemonProtocol.Methods.SetConsentScopes, payload)
            .ConfigureAwait(true);

        if (ScanOffered)
        {
            Step = OnboardingStep.Scan;
            return;
        }

        await LoadProjectsAsync().ConfigureAwait(true);
        Step = OnboardingStep.Watch;
    }

    /// <summary>
    /// Records the second-scanner choice and moves on.
    /// </summary>
    public async Task ConfirmScanAsync()
    {
        if (UseNearAiScan)
        {
            // Without this call the daemon refuses the filter forever and the
            // contributor experiences unexplained paralysis. It is the only
            // way an app-only contributor clears the notice, because they
            // never see the CLI's stdout version of it.
            await _host
                .CallAsync(DaemonProtocol.Methods.AcknowledgeNearAiNotice)
                .ConfigureAwait(true);
        }

        await LoadProjectsAsync().ConfigureAwait(true);
        Step = OnboardingStep.Watch;
    }

    public async Task LoadProjectsAsync()
    {
        // A stale refusal must not outlive the state it described.
        ProjectNotice = string.Empty;

        DaemonResponse response = await _host
            .CallAsync(DaemonProtocol.Methods.ListProjects)
            .ConfigureAwait(true);

        Projects.Clear();
        ProjectSettingsPayload? parsed = response.ResultAs<ProjectSettingsPayload>();
        foreach (ProjectSetting project in parsed?.Projects ?? new List<ProjectSetting>())
        {
            if (!string.IsNullOrEmpty(project.ProjectId))
            {
                Projects.Add(new ProjectViewModel(project));
            }
        }

        // An empty list is a real state, not a transient one: it is what every
        // machine showed before the local_path deserialisation bug was fixed,
        // and it rendered as a title above nothing at all.
        Raise(nameof(HasNoProjects));
    }

    /// <summary>
    /// Excludes a project or restores manual review after an accidental Ignore.
    /// </summary>
    /// <remarks>
    /// Only manual review and Ignore are offered here. Excluding
    /// the client repo is a live thought at this moment and never returns,
    /// whereas arming automation before a single preview has been seen is
    /// asking for trust that has not been earned yet.
    /// </remarks>
    public async Task IgnoreProjectAsync(ProjectViewModel project)
    {
        ArgumentNullException.ThrowIfNull(project);

        if (!project.CanToggle || ProjectManualMode.Next(project.Mode) is not string next)
        {
            return;
        }

        string projectId = project.ProjectId;
        project.IsPending = true;
        try
        {
            string payload = JsonSerializer.Serialize(
                new ProjectModeRequest { ProjectId = projectId, Mode = next });
            DaemonResponse response = await _host
                .CallAsync(DaemonProtocol.Methods.SetProjectMode, payload)
                .ConfigureAwait(true);

            // The rows are rebuilt from a fresh list_projects rather than from
            // `next`: what this screen shows about a consent field has to be
            // what the daemon stores, and a write that was refused or that
            // stored something else is invisible to a shell that believes its
            // own request. The re-read runs on the failure path too -- that is
            // the path where the two can disagree.
            await LoadProjectsAsync().ConfigureAwait(true);
            string? persisted = FindProject(projectId)?.Mode;
            ProjectNotice = ProjectManualMode.NoticeFor(response.IsError, next, persisted);
        }
        finally
        {
            project.IsPending = false;
        }
    }

    private ProjectViewModel? FindProject(string projectId)
    {
        foreach (ProjectViewModel candidate in Projects)
        {
            if (string.Equals(candidate.ProjectId, projectId, StringComparison.Ordinal))
            {
                return candidate;
            }
        }

        return null;
    }

    /// <summary>
    /// Records that the flow finished, for this tenant.
    /// </summary>
    public async Task FinishWatchingAsync()
    {
        DaemonResponse status = await _host
            .CallAsync(DaemonProtocol.Methods.Status)
            .ConfigureAwait(true);

        if (status.Result is JsonElement element
            && element.TryGetProperty("tenant_id", out JsonElement tenant)
            && tenant.ValueKind == JsonValueKind.String)
        {
            _tenantId = tenant.GetString();
        }

        _state.MarkComplete(_tenantId);
        Step = OnboardingStep.Done;
    }

    public void Finish() => Finished?.Invoke();

    /// <summary>
    /// Resolves the instance an invite names, for display before committing.
    /// </summary>
    /// <remarks>
    /// Host only. The obvious thing would be to show more of the invite, but
    /// the rest of it is the credential.
    /// </remarks>
    public void ResolveInstance(string? issuerHost)
    {
        InstanceLine = string.IsNullOrEmpty(issuerHost)
            ? string.Empty
            : $"This invite is for {issuerHost}.";
        Raise(nameof(HasInstanceLine));
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

    private sealed class EnrollRequest
    {
        [JsonPropertyName("invite")]
        public string Invite { get; set; } = string.Empty;
    }

    private sealed class ScopesRequest
    {
        [JsonPropertyName("scopes")]
        public List<string> Scopes { get; set; } = new();
    }

    private sealed class ProjectModeRequest
    {
        [JsonPropertyName("project_id")]
        public string ProjectId { get; set; } = string.Empty;

        [JsonPropertyName("mode")]
        public string Mode { get; set; } = string.Empty;
    }
}

public enum OnboardingStep
{
    Welcome,
    Connect,
    Consent,
    Scan,
    Watch,
    Done,
}

/// <summary>A scope row: the daemon's description, and a local short title.</summary>
public sealed class ConsentScopeViewModel : INotifyPropertyChanged
{
    private bool _isSelected;

    public ConsentScopeViewModel(ConsentOption scope)
    {
        ArgumentNullException.ThrowIfNull(scope);

        Name = scope.Name;
        Description = scope.Description;
        AlwaysOn = scope.AlwaysOn;
        Title = ScopeTitle(scope.Name);
        _isSelected = scope.AlwaysOn;
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    public string Name { get; }

    /// <summary>The daemon's own words, verbatim.</summary>
    public string Description { get; }

    public string Title { get; }

    public bool AlwaysOn { get; }

    /// <summary>An always-on scope is shown checked and cannot be unchecked.</summary>
    public bool IsEnabled => !AlwaysOn;

    public bool IsSelected
    {
        get => _isSelected;
        set
        {
            if (_isSelected == value || AlwaysOn)
            {
                return;
            }

            _isSelected = value;
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(IsSelected)));
        }
    }

    /// <summary>
    /// The short bold label for a scope.
    /// </summary>
    /// <remarks>
    /// <c>consent_options</c> carries the wire name and the description but
    /// no human title, so every shell maps them and all of them must agree.
    /// The fallback matters as much as the table: an operator who adds a
    /// scope this build has never heard of still gets a readable row, with
    /// the daemon's description beside it.
    /// </remarks>
    public static string ScopeTitle(string wireName) => wireName switch
    {
        "debugging_evaluation" => "Finding bugs and measuring agents",
        "benchmark_only" or "benchmark_creation" => "Turn my traces into test cases",
        "ranking_training" or "reward_model_training" => "Train models that judge agent output",
        "model_training" => "Train coding models directly",
        "public_attribution" => "List my handle publicly as a contributor",
        _ => wireName.Replace('_', ' '),
    };
}

public sealed class ProjectViewModel : INotifyPropertyChanged
{
    private string _mode;
    private bool _isPending;

    public ProjectViewModel(ProjectSetting project)
    {
        ArgumentNullException.ThrowIfNull(project);
        ProjectId = project.ProjectId;

        // Both the name and the line beneath it come from WatchCopy, which is
        // in the interop assembly precisely so they are exercised by tests on a
        // machine that cannot build WinUI. Which row this IS comes from the
        // daemon's own flag, never from the label and never from re-deriving
        // the opaque id.
        IsUnresolvable = project.IsUnresolvedBucket;
        ProjectLabel = WatchCopy.LabelFor(IsUnresolvable, project.ProjectLabel);
        _mode = project.Mode;
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    public string ProjectId { get; }

    public string ProjectLabel { get; }

    /// <summary>
    /// True for the bucket holding sessions whose project the daemon cannot
    /// name. It can be silenced but never armed, and the daemon enforces that
    /// on its own -- this flag only decides what the row says.
    /// </summary>
    public bool IsUnresolvable { get; }

    /// <summary>
    /// The line beneath the name: the mode for an ordinary row, and for the
    /// unresolvable bucket the note explaining why it can never be armed. The
    /// note REPLACES the mode rather than joining it, because "you'll always be
    /// asked" already says what "Ask me first" says.
    /// </summary>
    public string SubLine => WatchCopy.SubLineFor(IsUnresolvable, _mode);

    public string Mode => _mode;

    /// <summary>
    /// The button's words, empty when there is no transition out of this mode.
    /// <see cref="HasAction"/> hides the control in that case rather than
    /// leaving a disabled button with nothing written on it.
    /// </summary>
    public string ActionText => WatchCopy.ActionFor(_mode) ?? string.Empty;

    public bool HasAction => WatchCopy.ActionFor(_mode) is not null;

    public bool CanToggle => !_isPending && ProjectManualMode.Next(_mode) is not null;

    public bool IsPending
    {
        get => _isPending;
        set
        {
            _isPending = value;
            RaiseMode();
        }
    }

    // There is deliberately no SetMode here. A row's mode arrives from
    // list_projects and nowhere else: the one caller this class had set it
    // from the value the shell had just sent, which is the optimism the
    // re-read in IgnoreProjectAsync replaced.

    private void RaiseMode()
    {
        string[] properties =
        {
            nameof(Mode),
            nameof(ActionText),
            nameof(HasAction),
            nameof(CanToggle),
            nameof(SubLine),
        };

        foreach (string property in properties)
        {
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(property));
        }
    }
}
