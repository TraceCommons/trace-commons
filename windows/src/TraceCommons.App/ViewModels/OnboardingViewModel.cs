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
    private bool _isBusy;
    private bool _scanOffered;
    private bool _useNearAiScan;
    private string? _tenantId;

    public OnboardingViewModel(DaemonHost host, OnboardingState state)
    {
        _host = host ?? throw new ArgumentNullException(nameof(host));
        _state = state ?? throw new ArgumentNullException(nameof(state));
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

    public bool CanConnect => !string.IsNullOrWhiteSpace(_invite) && !_isBusy;

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
    /// Excludes a project from watching.
    /// </summary>
    /// <remarks>
    /// Ignore is offered here and auto-upload deliberately is not. Excluding
    /// the client repo is a live thought at this moment and never returns,
    /// whereas arming automation before a single preview has been seen is
    /// asking for trust that has not been earned yet.
    /// </remarks>
    public async Task IgnoreProjectAsync(ProjectViewModel project)
    {
        ArgumentNullException.ThrowIfNull(project);

        string payload = JsonSerializer.Serialize(
            new ProjectModeRequest { ProjectId = project.ProjectId, Mode = "ignore" });

        DaemonResponse response = await _host
            .CallAsync(DaemonProtocol.Methods.SetProjectMode, payload)
            .ConfigureAwait(true);
        if (!response.IsError)
        {
            project.SetMode("ignore");
        }
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
    private bool _isIgnored;

    public ProjectViewModel(ProjectSetting project)
    {
        ArgumentNullException.ThrowIfNull(project);
        ProjectId = project.ProjectId;

        // Both the name and the line beneath it come from WatchCopy, which is
        // in the interop assembly precisely so they are exercised by tests on a
        // machine that cannot build WinUI. Nothing about which row this is gets
        // decided here.
        ProjectLabel = WatchCopy.LabelFor(project.ProjectId, project.ProjectLabel);
        IsUnresolvable = WatchCopy.IsUnresolvable(project.ProjectId);
        _isIgnored = project.Mode == "ignore";
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
    public string SubLine => WatchCopy.SubLineFor(ProjectId, _isIgnored ? "ignore" : "ask");

    public bool IsIgnored
    {
        get => _isIgnored;
        set
        {
            if (_isIgnored == value)
            {
                return;
            }

            _isIgnored = value;
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(IsIgnored)));
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(IsNotIgnored)));
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(SubLine)));
        }
    }

    public bool IsNotIgnored => !_isIgnored;

    public void SetMode(string mode) => IsIgnored = mode == "ignore";
}
