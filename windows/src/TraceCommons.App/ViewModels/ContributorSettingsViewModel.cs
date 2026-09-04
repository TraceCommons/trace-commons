using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Globalization;
using System.Runtime.CompilerServices;
using System.Text.Json;
using System.Threading.Tasks;
using TraceCommons.Interop;

namespace TraceCommons.App.ViewModels;

/// <summary>
/// The device settings that are not public-profile state: connection facts,
/// startup, consent scopes, and per-project ask/ignore choices.
/// </summary>
public sealed class ContributorSettingsViewModel : INotifyPropertyChanged
{
    private readonly DaemonHost _host;
    private readonly HashSet<string> _preservedNonDataScopes = new(StringComparer.Ordinal);
    private bool _isBusy;
    private bool _isLoaded;
    private bool _startAtLogin;
    private bool _startupSupported = true;
    private bool _connected;
    private double _quiescenceMinutes;
    private double _approvalHoldSeconds;
    private double _digestHours;
    private long _queueTtlDays;
    private bool _localNotifications;
    private string _notice = string.Empty;

    /// <summary>
    /// The routing surface's words, read once from the Rust across the C ABI.
    ///
    /// Null when the call failed or the payload would not parse, and the whole
    /// surface is hidden in that case rather than rendered with blanks beside
    /// the tool names. Nothing on this surface is written here: see
    /// <see cref="RoutingTools"/>.
    /// </summary>
    private readonly RoutingCopy? _routingCopy = RoutingSurface.Copy();

    private RoutingEvidence? _routingEvidence;
    private bool _routingDeclared;
    private double _routingPort = TraceCommons.Interop.RoutingTools.DefaultPort;
    private string _routingTokenDir = string.Empty;
    private string _routingProbeText = string.Empty;
    private string _routingStateText = string.Empty;
    private RoutingTone _routingStateTone = RoutingTone.Neutral;
    private string? _routingLastChecked;
    private RoutingModes _routingModes = new();

    /// <summary>
    /// What a running IronWire published about itself, as far as this app has
    /// asked.
    /// </summary>
    /// <remarks>
    /// Starts at nothing found rather than null, because that is the state of
    /// a machine nobody has asked about yet AND of a machine without
    /// IronWire, and this card says the same thing about both: here are the
    /// fields, say which port.
    /// </remarks>
    private RoutingDiscovery _routingDiscovery = RoutingDiscovery.Nothing;

    /// <summary>
    /// Whether the contributor has opened or closed the port-and-folder
    /// disclosure. Null is "they have not said", and then it follows what
    /// discovery found.
    /// </summary>
    private bool? _routingOverrideOpen;

    /// <summary>
    /// Whether a daemon event may re-ask IronWire, and whether an answer
    /// still describes the declaration this machine holds now. The rules and
    /// their reasons live in <see cref="RoutingRefreshGate"/>, where they are
    /// tested off Windows.
    /// </summary>
    private readonly RoutingRefreshGate _routingGate = new();

    public ContributorSettingsViewModel(DaemonHost host)
    {
        _host = host ?? throw new ArgumentNullException(nameof(host));

        // The routing card is the daemon's state, not this window's: the
        // reader moves from awaiting-rows to rows-seen on its own, the proxy
        // comes up, the daemon restarts and loses its per-process stamp.
        // Without this the card showed whatever was true when it was opened
        // until the contributor touched something. This is the same event
        // MainViewModel refreshes on, and DaemonHost raises it for the ABI's
        // lag and resync frames too, so a missed delta repaints this surface
        // as well. No timer is added: the event path already exists.
        //
        // Never unsubscribed, matching MainViewModel. SettingsView is built
        // once with `??=` and lives as long as the window.
        _host.StatusChanged += OnDaemonStatusChanged;
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    public ObservableCollection<ConnectionStatusViewModel> ConnectionRows { get; } = new();

    public ObservableCollection<ConsentScopeViewModel> AlwaysIncluded { get; } = new();

    public ObservableCollection<ConsentScopeViewModel> OptionalScopes { get; } = new();

    public ObservableCollection<ProjectSettingViewModel> Projects { get; } = new();

    public ObservableCollection<AuditSettingViewModel> AuditEntries { get; } = new();

    /// <summary>One row per tool, each carrying exactly one of the four shared words.</summary>
    public ObservableCollection<RoutingToolRowViewModel> RoutingToolRows { get; } = new();

    // --- The routing surface's fixed words ------------------------------
    //
    // Every one of these is the payload's, never this shell's. A string
    // literal here would be a fourth place the wording can drift to, and one
    // of them is a privacy claim.

    /// <summary>Whether the shared words arrived at all.</summary>
    public bool RoutingAvailable => _routingCopy is not null;

    public string RoutingToolsHeading => _routingCopy?.ToolsHeading ?? string.Empty;

    public string RoutingIntro => _routingCopy?.Intro ?? string.Empty;

    public string RoutingToggleText => _routingCopy?.Toggle ?? string.Empty;

    /// <summary>
    /// Said out loud because the obvious worry is that it is not true.
    /// Nothing on this surface waits on the app being started again.
    /// </summary>
    public string RoutingAppliesAtOnceText => _routingCopy?.AppliesAtOnce ?? string.Empty;

    public string RoutingPortTitle => _routingCopy?.PortTitle ?? string.Empty;

    public string RoutingPortNote => _routingCopy?.PortNote ?? string.Empty;

    public string RoutingFolderTitle => _routingCopy?.FolderTitle ?? string.Empty;

    public string RoutingFolderNote => _routingCopy?.FolderNote ?? string.Empty;

    public string RoutingApplyText => _routingCopy?.Apply ?? string.Empty;

    public string RoutingConnectText => _routingCopy?.Connect ?? string.Empty;

    public string RoutingLookAgainText => _routingCopy?.LookAgain ?? string.Empty;

    public string RoutingOverrideTitle => _routingCopy?.OverrideTitle ?? string.Empty;

    /// <summary>
    /// What the machine already knows, in the shared sentence -- for both
    /// states, because a machine without IronWire is the ordinary machine and
    /// gets a sentence rather than an error.
    /// </summary>
    public string RoutingDiscoveryText => _routingCopy is null
        ? string.Empty
        : TraceCommons.Interop.RoutingTools.DiscoveryLine(_routingCopy, _routingDiscovery);

    /// <summary>
    /// Whether to offer the one-press connect.
    /// </summary>
    /// <remarks>
    /// Only where there is something to connect to and nothing is declared.
    /// Where something is declared the switch is already on and this would be
    /// a second Apply.
    /// </remarks>
    public bool RoutingConnectOffered => _routingDiscovery.Found && !_routingDeclared && !_isBusy;

    /// <summary>
    /// Whether the port and folder are shown open.
    /// </summary>
    /// <remarks>
    /// Follows what discovery found until the contributor says otherwise.
    /// Where nothing was discovered they are the only way to answer, so they
    /// are open: this inverts the default, it does not remove the manual
    /// path.
    /// </remarks>
    public bool RoutingOverrideOpen
    {
        get => _routingOverrideOpen
            ?? !TraceCommons.Interop.RoutingTools.OverrideIsCollapsed(_routingDiscovery);
        set
        {
            if (_routingOverrideOpen == value)
            {
                return;
            }

            _routingOverrideOpen = value;
            Raise(nameof(RoutingOverrideOpen));
        }
    }

    /// <summary>
    /// Whether IronWire is declared on this machine.
    /// </summary>
    /// <remarks>
    /// Deliberately not an input to any tool's word. Declaring IronWire here
    /// has no causal relation to whether a tool is configured to send through
    /// it, and reading this switch is what let a contributor see the wired
    /// word on the same card as "Nothing answered on port 8463".
    /// </remarks>
    public bool RoutingDeclared
    {
        get => _routingDeclared;
        private set
        {
            if (Set(ref _routingDeclared, value))
            {
                Raise(nameof(RoutingControlsEnabled));
                Raise(nameof(RoutingConnectOffered));
            }
        }
    }

    /// <summary>The port and folder boxes are the override, live only while the switch is on.</summary>
    public bool RoutingControlsEnabled => _routingDeclared && !_isBusy;

    /// <summary>
    /// The port, shown filled in with IronWire's conventional number so
    /// nobody has to know it.
    /// </summary>
    /// <remarks>
    /// <b>Shown is not declared.</b> Nothing is written until the contributor
    /// turns the switch on: a displayed default that wrote itself would have
    /// this window announce a local service nobody mentioned.
    /// </remarks>
    public double RoutingPort
    {
        get => _routingPort;
        set => Set(ref _routingPort, value);
    }

    public string RoutingTokenDir
    {
        get => _routingTokenDir;
        set => Set(ref _routingTokenDir, value ?? string.Empty);
    }

    /// <summary>What the last check answered, or empty while nothing has been asked.</summary>
    public string RoutingProbeText
    {
        get => _routingProbeText;
        private set
        {
            if (Set(ref _routingProbeText, value))
            {
                Raise(nameof(HasRoutingProbeText));
            }
        }
    }

    public bool HasRoutingProbeText => _routingProbeText.Length > 0;

    /// <summary>The daemon's three-state view of what it is seeing.</summary>
    public string RoutingStateText
    {
        get => _routingStateText;
        private set => Set(ref _routingStateText, value);
    }

    /// <summary>
    /// How firmly that sentence reads, from the daemon's state.
    /// </summary>
    /// <remarks>
    /// <see cref="RoutingTools.StateTone"/> already gated the "last checked"
    /// stamp inside <see cref="RoutingTools.StatusLine"/>, but this shell
    /// threw the tone away and painted the sentence flat. GTK has painted
    /// this row from the same three states since it was written; this is
    /// that parity.
    ///
    /// <c>awaiting_rows</c> is held and not broken -- a reader built a moment
    /// ago starts cold by construction, and that is the state a contributor
    /// sees immediately after touching anything on this card.
    ///
    /// <c>token_unreadable</c> is the one state that reads as attention. It
    /// is a fact about this machine and not an alarm about anything remote,
    /// but it is not neutral either: neutral is the off sentence's reading,
    /// and this state's switch is on.
    ///
    /// From the state, never from <see cref="RoutingStateText"/>. The word
    /// half of this surface used to recover its tone by comparing a rendered
    /// privacy claim, and that is the shape being removed everywhere.
    /// </remarks>
    public RoutingTone RoutingStateTone
    {
        get => _routingStateTone;
        private set
        {
            if (_routingStateTone == value)
            {
                return;
            }

            _routingStateTone = value;
            Raise(nameof(RoutingStateTone));
            Raise(nameof(RoutingStateIsClear));
            Raise(nameof(RoutingStateIsHeld));
            Raise(nameof(RoutingStateIsAttention));
            Raise(nameof(RoutingStateIsNeutral));
        }
    }

    /// <summary>
    /// The XAML projection of <see cref="RoutingStateTone"/>, and only that.
    ///
    /// Four visibilities rather than one bound brush because the tone
    /// colours live in a theme dictionary and only <c>ThemeResource</c>
    /// resolves those correctly in both themes. All three read the enum;
    /// none reads the sentence.
    /// </summary>
    public bool RoutingStateIsClear => RoutingStateTone == RoutingTone.Clear;

    /// <summary>The held reading. Normal, never a fault.</summary>
    public bool RoutingStateIsHeld => RoutingStateTone == RoutingTone.Held;

    /// <summary>
    /// The one reading that asks for something: declared, and no reader
    /// could be built. Painted apart from the other three because neutral
    /// reads as off and held reads as normal, and this state is neither.
    /// </summary>
    public bool RoutingStateIsAttention => RoutingStateTone == RoutingTone.Attention;

    /// <summary>Says nothing either way, and is the default.</summary>
    public bool RoutingStateIsNeutral =>
        RoutingStateTone != RoutingTone.Clear
        && RoutingStateTone != RoutingTone.Held
        && RoutingStateTone != RoutingTone.Attention;

    /// <summary>
    /// When the daemon last got an answer.
    /// </summary>
    /// <remarks>
    /// Per-process: the stamp lives in the running daemon and starts empty
    /// again every time that process comes back up, so it is a "last checked"
    /// and never an install date or a "connected since". Withheld entirely on
    /// the state that has had no answer at all.
    /// </remarks>
    public string RoutingLastChecked => _routingLastChecked ?? string.Empty;

    public bool HasRoutingLastChecked => !string.IsNullOrEmpty(_routingLastChecked);

    private void SetRoutingLastChecked(string? value)
    {
        if (string.Equals(_routingLastChecked, value, StringComparison.Ordinal))
        {
            return;
        }

        _routingLastChecked = value;
        Raise(nameof(RoutingLastChecked));
        Raise(nameof(HasRoutingLastChecked));
    }

    public bool IsBusy
    {
        get => _isBusy;
        private set
        {
            if (Set(ref _isBusy, value))
            {
                Raise(nameof(IsNotBusy));
                Raise(nameof(RoutingControlsEnabled));
                Raise(nameof(RoutingConnectOffered));
            }
        }
    }

    public bool IsNotBusy => !_isBusy;

    public bool IsLoaded
    {
        get => _isLoaded;
        private set => Set(ref _isLoaded, value);
    }

    public bool StartupSupported
    {
        get => _startupSupported;
        private set => Set(ref _startupSupported, value);
    }

    public bool StartAtLogin
    {
        get => _startAtLogin;
        private set => Set(ref _startAtLogin, value);
    }

    public bool Connected
    {
        get => _connected;
        private set
        {
            if (Set(ref _connected, value))
            {
                Raise(nameof(ConnectionText));
                Raise(nameof(ConnectionDetail));
                Raise(nameof(HasConnectionDetail));
            }
        }
    }

    public string ConnectionText => Connected ? "Connected" : "Not connected";

    public string ConnectionDetail => Connected
        ? string.Empty
        : "Sessions are being queued, but nothing can be sent.";

    public bool HasConnectionDetail => !Connected;

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

    public bool HasProjects => Projects.Count > 0;

    public bool HasNoProjects => Projects.Count == 0;

    public bool HasAuditEntries => AuditEntries.Count > 0;

    public bool HasNoAuditEntries => AuditEntries.Count == 0;

    public double QuiescenceMinutes
    {
        get => _quiescenceMinutes;
        private set => Set(ref _quiescenceMinutes, value);
    }

    public double ApprovalHoldSeconds
    {
        get => _approvalHoldSeconds;
        private set
        {
            if (Set(ref _approvalHoldSeconds, value))
            {
                Raise(nameof(HasNoUndoWindow));
            }
        }
    }

    public bool HasNoUndoWindow => ApprovalHoldSeconds == 0;

    public double DigestHours
    {
        get => _digestHours;
        private set => Set(ref _digestHours, value);
    }

    public string QueueExpiryText =>
        $"Undecided sessions are dropped after {_queueTtlDays} days. Dropped means never sent.";

    public string NotificationOwnerText => _localNotifications
        ? "Notifications are rendered by the background daemon."
        : "Notifications are rendered by this app.";

    public async Task LoadAsync()
    {
        IsBusy = true;
        try
        {
            DaemonResponse statusResponse = await _host
                .CallAsync(DaemonProtocol.Methods.Status)
                .ConfigureAwait(true);
            DaemonStatus? status = statusResponse.ResultAs<DaemonStatus>();
            Connected = status?.LoggedIn ?? false;

            DaemonResponse settingsResponse = await _host
                .CallAsync(DaemonProtocol.Methods.GetSettings)
                .ConfigureAwait(true);
            DaemonSettingsSnapshot? snapshot = settingsResponse.ResultAs<DaemonSettingsSnapshot>();
            FillSettings(snapshot);
            // Asked before the port box is filled, so a discovered port can
            // reach it. It reads a file, opens no connection and declares
            // nothing.
            await DiscoverRoutingAsync().ConfigureAwait(true);
            FillRouting(snapshot, status);

            DaemonResponse optionsResponse = await _host
                .CallAsync(DaemonProtocol.Methods.ConsentOptions)
                .ConfigureAwait(true);
            FillConsent(
                optionsResponse.ResultAs<ConsentOptionsPayload>(),
                status?.ConsentScopes ?? new List<string>());

            await LoadProjectsAsync().ConfigureAwait(true);
            await LoadAuditAsync().ConfigureAwait(true);
            if (RoutingDeclared)
            {
                await CheckRoutingAsync().ConfigureAwait(true);
            }

            StartupRegistrationState startup = await StartupRegistration
                .GetStateAsync()
                .ConfigureAwait(true);
            StartupSupported = startup.IsSupported;
            StartAtLogin = startup.IsEnabled;
            Notice = statusResponse.IsError
                ? "Settings couldn't be read just now."
                : startup.Notice;
            IsLoaded = true;
        }
        finally
        {
            IsBusy = false;
        }
    }

    public async Task SetStartAtLoginAsync(bool enabled)
    {
        if (!IsLoaded || IsBusy || !StartupSupported || enabled == StartAtLogin)
        {
            return;
        }

        IsBusy = true;
        try
        {
            // Keep the source in step with the contributor's requested
            // position so a refused result can produce a real property
            // change back to the authoritative state.
            StartAtLogin = enabled;
            StartupRegistrationState startup = await StartupRegistration
                .SetEnabledAsync(enabled)
                .ConfigureAwait(true);
            StartupSupported = startup.IsSupported;
            StartAtLogin = startup.IsEnabled;
            Notice = startup.Notice;
        }
        finally
        {
            IsBusy = false;
        }
    }

    /// <summary>
    /// Commits every optional data-use scope as one set. Non-data scopes such
    /// as public attribution are preserved because their separate consent
    /// surface owns them.
    /// </summary>
    public async Task SaveConsentAsync()
    {
        if (!IsLoaded || IsBusy)
        {
            return;
        }

        var scopes = new List<string>(_preservedNonDataScopes);
        foreach (ConsentScopeViewModel row in OptionalScopes)
        {
            if (row.IsSelected)
            {
                scopes.Add(row.Name);
            }
        }

        IsBusy = true;
        try
        {
            string payload = JsonSerializer.Serialize(new { scopes });
            DaemonResponse response = await _host
                .CallAsync(DaemonProtocol.Methods.SetConsentScopes, payload)
                .ConfigureAwait(true);

            Notice = response.IsError
                ? "Permissions couldn't be changed. The previous choices still apply."
                : "Permissions updated for traces sent from now on.";

            if (response.IsError)
            {
                await ReloadConsentAsync().ConfigureAwait(true);
            }
            else
            {
                await LoadAuditAsync().ConfigureAwait(true);
            }
        }
        finally
        {
            IsBusy = false;
        }
    }

    public async Task ToggleProjectAsync(ProjectSettingViewModel project)
    {
        ArgumentNullException.ThrowIfNull(project);
        if (!IsLoaded || IsBusy || !project.CanToggle)
        {
            return;
        }

        string next = project.Mode == "ignore" ? "ask" : "ignore";
        string payload = JsonSerializer.Serialize(
            new Dictionary<string, string>
            {
                ["project_id"] = project.ProjectId,
                ["mode"] = next,
            });

        IsBusy = true;
        try
        {
            DaemonResponse response = await _host
                .CallAsync(DaemonProtocol.Methods.SetProjectMode, payload)
                .ConfigureAwait(true);

            Notice = response.IsError
                ? "That project setting couldn't be changed."
                : string.Empty;
            if (!response.IsError)
            {
                project.SetMode(next);
            }
        }
        finally
        {
            IsBusy = false;
        }
    }

    public async Task SaveBehaviorAsync(BehaviorSetting setting, double displayedValue)
    {
        if (!IsLoaded || IsBusy)
        {
            return;
        }

        string payload;
        try
        {
            payload = BehaviorSettingsRequest.Serialize(setting, displayedValue);
        }
        catch (ArgumentOutOfRangeException)
        {
            Notice = "That value is outside the supported range.";
            return;
        }

        IsBusy = true;
        try
        {
            SetDisplayedBehavior(setting, displayedValue);
            DaemonResponse response = await _host
                .CallAsync(DaemonProtocol.Methods.SetSettings, payload)
                .ConfigureAwait(true);
            DaemonSettingsSnapshot? settings = response.ResultAs<DaemonSettingsSnapshot>();
            if (response.IsError || settings is null)
            {
                Notice = "That couldn't be changed just now. Nothing was changed.";
                DaemonResponse current = await _host
                    .CallAsync(DaemonProtocol.Methods.GetSettings)
                    .ConfigureAwait(true);
                FillSettings(current.ResultAs<DaemonSettingsSnapshot>());
            }
            else
            {
                FillSettings(settings);
                Notice = string.Empty;
            }
        }
        finally
        {
            IsBusy = false;
        }
    }

    private void SetDisplayedBehavior(BehaviorSetting setting, double value)
    {
        switch (setting)
        {
            case BehaviorSetting.QuiescenceMinutes:
                QuiescenceMinutes = value;
                break;
            case BehaviorSetting.ApprovalHoldSeconds:
                ApprovalHoldSeconds = value;
                break;
            case BehaviorSetting.DigestHours:
                DigestHours = value;
                break;
            default:
                throw new ArgumentOutOfRangeException(nameof(setting));
        }
    }

    /// <summary>
    /// One session-source row. The satisfied tone tracks the same mode word
    /// the sentence does, so the tick and the words cannot disagree.
    /// </summary>
    private void AddSourceRow(string tool, string sourceMode)
    {
        string? line = SourceChecks.CheckLine(tool, sourceMode);
        if (line is null)
        {
            return;
        }

        ConnectionRows.Add(new ConnectionStatusViewModel(line, sourceMode == "watch"));
    }

    private void FillSettings(DaemonSettingsSnapshot? settings)
    {
        ConnectionRows.Clear();
        if (settings is null)
        {
            return;
        }

        // The row's words come from the Rust, chosen by the MODE. This shell
        // branched on ClaudeRootConfigured, which is (mode == "watch") and
        // therefore false for "off" as well as for "unset" -- so a
        // contributor who declared Claude Code off was told its sessions
        // were being read from the usual place. Nothing is read from an off
        // source. A null line means the call failed; the row is left out
        // rather than filled with a sentence written here.
        AddSourceRow(SourceChecks.Claude, settings.ClaudeSourceMode);
        AddSourceRow(SourceChecks.Codex, settings.CodexSourceMode);
        ConnectionRows.Add(new ConnectionStatusViewModel(
            settings.NearAiConfigured
                ? "Extra privacy scan configured"
                : "No extra privacy scan",
            settings.NearAiConfigured));

        QuiescenceMinutes = settings.QuiescenceSeconds / 60.0;
        ApprovalHoldSeconds = settings.ApprovalHoldSeconds;
        DigestHours = settings.DigestIntervalSeconds / 3600.0;
        _queueTtlDays = settings.QueueTtlDays;
        _localNotifications = settings.LocalNotifications;
        Raise(nameof(QueueExpiryText));
        Raise(nameof(NotificationOwnerText));
    }

    private void FillConsent(ConsentOptionsPayload? options, IReadOnlyCollection<string> granted)
    {
        AlwaysIncluded.Clear();
        OptionalScopes.Clear();
        _preservedNonDataScopes.Clear();

        var grantedSet = new HashSet<string>(granted, StringComparer.Ordinal);
        foreach (ConsentOption option in options?.Scopes ?? new List<ConsentOption>())
        {
            var row = new ConsentScopeViewModel(option);
            if (!option.AlwaysOn)
            {
                row.IsSelected = grantedSet.Contains(option.Name);
            }

            if (option.AlwaysOn)
            {
                AlwaysIncluded.Add(row);
            }
            else if (option.GrantsDataUse)
            {
                OptionalScopes.Add(row);
            }
            else if (grantedSet.Contains(option.Name))
            {
                _preservedNonDataScopes.Add(option.Name);
            }
        }
    }

    private async Task ReloadConsentAsync()
    {
        DaemonResponse status = await _host
            .CallAsync(DaemonProtocol.Methods.Status)
            .ConfigureAwait(true);
        DaemonResponse options = await _host
            .CallAsync(DaemonProtocol.Methods.ConsentOptions)
            .ConfigureAwait(true);
        FillConsent(
            options.ResultAs<ConsentOptionsPayload>(),
            status.ResultAs<DaemonStatus>()?.ConsentScopes ?? new List<string>());
    }

    private async Task LoadProjectsAsync()
    {
        DaemonResponse response = await _host
            .CallAsync(DaemonProtocol.Methods.ListProjects)
            .ConfigureAwait(true);

        Projects.Clear();
        foreach (ProjectSetting project in response.ResultAs<ProjectSettingsPayload>()?.Projects
                 ?? new List<ProjectSetting>())
        {
            if (!string.IsNullOrWhiteSpace(project.ProjectId))
            {
                Projects.Add(new ProjectSettingViewModel(project));
            }
        }

        Raise(nameof(HasProjects));
        Raise(nameof(HasNoProjects));
    }

    private async Task LoadAuditAsync()
    {
        DaemonResponse response = await _host
            .CallAsync(DaemonProtocol.Methods.ListAudit, "{\"limit\":20}")
            .ConfigureAwait(true);

        AuditEntries.Clear();
        foreach (AuditSettingEntry entry in response.ResultAs<AuditSettingsPayload>()?.Entries
                 ?? new List<AuditSettingEntry>())
        {
            AuditEntries.Add(new AuditSettingViewModel(entry));
        }

        Raise(nameof(HasAuditEntries));
        Raise(nameof(HasNoAuditEntries));
    }

    // --- The routing surface --------------------------------------------

    /// <summary>
    /// Turns the declaration on or off. One <c>set_settings</c> key, written
    /// the moment the switch moves.
    /// </summary>
    /// <remarks>
    /// What IronWire said about the old declaration is dropped BEFORE the
    /// write, not after a replacement arrives: the words must stop asserting
    /// immediately, not once something new lands.
    /// </remarks>
    public async Task SetRoutingEnabledAsync(bool on)
    {
        if (!IsLoaded || IsBusy || on == RoutingDeclared)
        {
            return;
        }

        await WriteRoutingAsync(on).ConfigureAwait(true);
    }

    /// <summary>
    /// Rewrites the declaration from the port and folder boxes, then asks
    /// again. The probe runs only from here and from the switch: a human
    /// pressing something. Nothing on the submission path calls it.
    /// </summary>
    public async Task ApplyRoutingAsync()
    {
        if (!IsLoaded || IsBusy || !RoutingDeclared)
        {
            return;
        }

        await WriteRoutingAsync(true).ConfigureAwait(true);
    }

    /// <summary>
    /// Asks what the machine already knows, and shows it.
    /// </summary>
    /// <remarks>
    /// <para>
    /// <b>Writes nothing and reads nothing of the contributor's.</b> It reads
    /// one file IronWire left, learns a port from it, and puts that port in a
    /// box. Declaring is still the switch and the two buttons; a discovery
    /// that declared on its own would be this window announcing a local
    /// service nobody mentioned, which is what the declaration exists to
    /// stop.
    /// </para>
    /// <para>
    /// A call that did not run degrades to nothing found, which is also what
    /// a machine without IronWire answers. Both mean there is nothing to
    /// offer, and neither is a fault.
    /// </para>
    /// </remarks>
    public async Task DiscoverRoutingAsync()
    {
        DaemonResponse response = await _host
            .CallAsync(DaemonProtocol.Methods.DiscoverRouting)
            .ConfigureAwait(true);
        _routingDiscovery = response.IsError || response.Result is null
            ? RoutingDiscovery.Nothing
            : RoutingDiscovery.Parse(response.Result.Value.GetRawText());
        Raise(nameof(RoutingDiscoveryText));
        Raise(nameof(RoutingConnectOffered));
        Raise(nameof(RoutingOverrideOpen));
    }

    /// <summary>
    /// Asks again, then repaints the port box if nothing is declared.
    /// </summary>
    /// <remarks>
    /// For the contributor who started IronWire after opening this window.
    /// Offered rather than polled: this card does not go looking at a file on
    /// a timer.
    /// </remarks>
    public async Task LookAgainAsync()
    {
        if (!IsLoaded || IsBusy)
        {
            return;
        }

        await DiscoverRoutingAsync().ConfigureAwait(true);
        if (!RoutingDeclared)
        {
            RoutingPort = TraceCommons.Interop.RoutingTools.ShownPort(
                null,
                _routingDiscovery.Port);
        }
    }

    /// <summary>
    /// The shortcut past the two boxes: turn it on and check, in one press.
    /// </summary>
    /// <remarks>
    /// It writes the port that is ON SCREEN -- the discovered one, or
    /// whatever was typed over it -- so a press cannot declare a number
    /// different from the one displayed.
    /// </remarks>
    public async Task ConnectRoutingAsync()
    {
        if (!IsLoaded || IsBusy || RoutingDeclared)
        {
            return;
        }

        await WriteRoutingAsync(true).ConfigureAwait(true);
    }

    private async Task WriteRoutingAsync(bool on)
    {
        _routingEvidence = null;
        _routingGate.Forget();
        RenderRoutingToolRows();
        RoutingDeclared = on;
        RoutingProbeText = on && _routingCopy is not null ? _routingCopy.Checking : string.Empty;

        IsBusy = true;
        try
        {
            string payload = TraceCommons.Interop.RoutingTools.SerializeDeclaration(
                on,
                RoutingPortValue(),
                RoutingTokenDir);
            DaemonResponse response = await _host
                .CallAsync(DaemonProtocol.Methods.SetSettings, payload)
                .ConfigureAwait(true);
            if (response.IsError)
            {
                // The error label is a fixed one by contract and is not a
                // sentence anybody can act on. What matters is that nothing
                // changed.
                RoutingProbeText = string.Empty;
                Notice = "That couldn't be changed just now. Nothing was changed.";
            }

            DaemonResponse current = await _host
                .CallAsync(DaemonProtocol.Methods.GetSettings)
                .ConfigureAwait(true);
            DaemonResponse status = await _host
                .CallAsync(DaemonProtocol.Methods.Status)
                .ConfigureAwait(true);
            FillRouting(
                current.ResultAs<DaemonSettingsSnapshot>(),
                status.ResultAs<DaemonStatus>());
        }
        finally
        {
            IsBusy = false;
        }

        if (RoutingDeclared)
        {
            await CheckRoutingAsync().ConfigureAwait(true);
        }
    }

    /// <summary>
    /// Asks IronWire which tools on this machine are pointed at it, and
    /// repaints the words from the answer.
    /// </summary>
    /// <remarks>
    /// A call that did not run is not a fact about any tool: the evidence is
    /// left empty, so every word stays at the no-verdict one.
    /// </remarks>
    private async Task CheckRoutingAsync()
    {
        if (_routingCopy is null)
        {
            return;
        }

        long ticket = _routingGate.BeginProbe();
        RoutingProbeText = _routingCopy.Checking;
        IsBusy = true;
        try
        {
            RoutingEvidence? evidence = await AskRoutedToolsAsync(
                    RoutingPortValue(),
                    RoutingTokenDir)
                .ConfigureAwait(true);

            if (evidence is null)
            {
                if (_routingGate.CompleteWithoutAnswer(ticket))
                {
                    _routingEvidence = null;
                    RoutingProbeText = _routingCopy.CheckUnavailable;
                }
            }
            else if (_routingGate.CompleteWithAnswer(ticket, DateTimeOffset.UtcNow))
            {
                _routingEvidence = evidence;
                RoutingProbeText = TraceCommons.Interop.RoutingTools.ProbeLine(
                    _routingCopy,
                    evidence.Outcome);
            }

            RenderRoutingToolRows();
        }
        finally
        {
            IsBusy = false;
        }
    }

    /// <summary>
    /// A daemon event says something may have changed, so this card is
    /// repainted from the daemon rather than left showing what was true when
    /// it was opened.
    /// </summary>
    /// <remarks>
    /// async void because this is an event handler, which is the one place it
    /// is correct. Nothing it calls throws on a daemon error: an error frame
    /// is a parsed response, and a card that could not be refreshed keeps
    /// what it had rather than blanking.
    /// </remarks>
    private async void OnDaemonStatusChanged()
    {
        await RefreshRoutingAsync().ConfigureAwait(true);
    }

    /// <summary>
    /// Re-reads the daemon's routing state and repaints the card.
    /// </summary>
    /// <remarks>
    /// Only the routing card. Everything else on this screen is a knob whose
    /// value this window is the author of, and refetching those under a
    /// contributor's hands would be a different change.
    ///
    /// Skipped while a write is in flight: that write ends by filling this
    /// card from the daemon's own answer, and a repaint racing it would paint
    /// the state from before it.
    /// </remarks>
    private async Task RefreshRoutingAsync()
    {
        if (!IsLoaded || IsBusy)
        {
            return;
        }

        DaemonResponse settingsResponse = await _host
            .CallAsync(DaemonProtocol.Methods.GetSettings)
            .ConfigureAwait(true);
        DaemonResponse statusResponse = await _host
            .CallAsync(DaemonProtocol.Methods.Status)
            .ConfigureAwait(true);

        DaemonSettingsSnapshot? snapshot = settingsResponse.ResultAs<DaemonSettingsSnapshot>();
        FillRouting(
            snapshot,
            statusResponse.ResultAs<DaemonStatus>(),
            fillDeclarationFields: false);

        await ReAskRoutedToolsAsync(snapshot).ConfigureAwait(true);
    }

    /// <summary>
    /// Asks IronWire again on a daemon event, if the gate allows it.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Three differences from the human path, each deliberate. It does not
    /// raise <see cref="IsBusy"/>: an event nobody asked for must not disable
    /// the controls under a contributor's hands. It does not touch
    /// <see cref="RoutingProbeText"/>: that line answers a press, and a
    /// background call rewriting it would answer a question nobody asked.
    /// And a call that did not run drops nothing -- the previous answer stays
    /// on screen rather than every word blanking because one background call
    /// failed.
    /// </para>
    /// <para>
    /// The declared port and folder are used, never the boxes': an event can
    /// land while somebody is typing into the port field, and the question
    /// has to be about the declaration the daemon is actually holding.
    /// </para>
    /// </remarks>
    private async Task ReAskRoutedToolsAsync(DaemonSettingsSnapshot? settings)
    {
        if (_routingCopy is null
            || !_routingGate.TryBeginProbe(RoutingDeclared, DateTimeOffset.UtcNow, out long ticket))
        {
            return;
        }

        RoutingEvidence? evidence = await AskRoutedToolsAsync(
                settings?.Routing?.Port ?? TraceCommons.Interop.RoutingTools.DefaultPort,
                settings?.Routing?.TokenDir ?? string.Empty)
            .ConfigureAwait(true);

        if (evidence is null)
        {
            _routingGate.CompleteWithoutAnswer(ticket);
            return;
        }

        if (_routingGate.CompleteWithAnswer(ticket, DateTimeOffset.UtcNow))
        {
            _routingEvidence = evidence;
            RenderRoutingToolRows();
        }
    }

    /// <summary>
    /// One <c>probe_routed_tools</c> call, or null when it did not run.
    /// </summary>
    /// <remarks>
    /// Null and not an empty answer: a call that did not run is not a fact
    /// about any tool, and the two callers say different things about that.
    /// </remarks>
    private async Task<RoutingEvidence?> AskRoutedToolsAsync(ushort port, string tokenDir)
    {
        string payload = TraceCommons.Interop.RoutingTools.SerializeProbeParams(port, tokenDir);
        DaemonResponse response = await _host
            .CallAsync(DaemonProtocol.Methods.ProbeRoutedTools, payload)
            .ConfigureAwait(true);

        return response.IsError || response.Result is null
            ? null
            : RoutingEvidence.Parse(response.Result.Value.GetRawText());
    }

    /// <summary>
    /// Fills the declaration controls and the state line from the daemon's
    /// own answer.
    /// </summary>
    /// <param name="fillDeclarationFields">
    /// Whether the port and folder boxes are refilled from the daemon's
    /// declaration. False on a repaint driven by a daemon event: those two
    /// are written when Apply is pressed rather than as they are typed, so an
    /// event landing mid-edit would otherwise replace a half-typed port with
    /// the declared one.
    /// </param>
    private void FillRouting(
        DaemonSettingsSnapshot? settings,
        DaemonStatus? status,
        bool fillDeclarationFields = true)
    {
        _routingModes = new RoutingModes
        {
            Claude = settings?.ClaudeSourceMode ?? string.Empty,
            Codex = settings?.CodexSourceMode ?? string.Empty,
            Gemini = settings?.GeminiSourceMode ?? string.Empty,
            Cline = settings?.ClineSourceMode ?? string.Empty,
        };

        bool declared = settings?.RoutingDeclared ?? false;
        if (!declared)
        {
            // Nothing is declared, so nothing held about IronWire is still
            // about this machine's current state. Dropped rather than kept,
            // so turning the switch back on cannot paint a stale verdict
            // before a new answer lands.
            _routingEvidence = null;
            _routingGate.Forget();
            RoutingProbeText = string.Empty;
        }

        RoutingDeclared = declared;
        if (fillDeclarationFields)
        {
            // The precedence is the rule the whole feature turns on: a
            // declared port always wins, a discovered one fills in only where
            // there is none, and the conventional number is what is left. See
            // RoutingTools.ShownPort.
            RoutingPort = TraceCommons.Interop.RoutingTools.ShownPort(
                settings?.Routing?.Port,
                _routingDiscovery.Port);
            RoutingTokenDir = settings?.Routing?.TokenDir ?? string.Empty;
        }

        RenderRoutingToolRows();

        if (_routingCopy is null)
        {
            return;
        }

        RoutingStatusLine line = TraceCommons.Interop.RoutingTools.StatusLine(
            _routingCopy,
            status?.RoutingState ?? string.Empty,
            status?.Routing?.LastRefreshAt);
        RoutingStateText = line.Text;
        RoutingStateTone = line.Tone;
        SetRoutingLastChecked(line.LastChecked);
    }

    /// <summary>
    /// The single painter for the tool rows. Both things that can change a
    /// word go through it, so neither can arrive and blank what the other
    /// established.
    /// </summary>
    private void RenderRoutingToolRows()
    {
        RoutingToolRows.Clear();
        if (_routingCopy is null)
        {
            return;
        }

        foreach (RoutingToolRow row in TraceCommons.Interop.RoutingTools.Rows(
                     _routingCopy,
                     _routingModes,
                     _routingEvidence))
        {
            RoutingToolRows.Add(new RoutingToolRowViewModel(row));
        }
    }

    private ushort RoutingPortValue()
    {
        double value = Math.Round(RoutingPort, MidpointRounding.AwayFromZero);
        if (value < 1)
        {
            return TraceCommons.Interop.RoutingTools.DefaultPort;
        }

        return value > ushort.MaxValue
            ? TraceCommons.Interop.RoutingTools.DefaultPort
            : (ushort)value;
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

public sealed class AuditSettingViewModel
{
    public AuditSettingViewModel(AuditSettingEntry entry)
    {
        ArgumentNullException.ThrowIfNull(entry);
        AtText = entry.At.ToLocalTime().ToString("MMM d, HH:mm", CultureInfo.CurrentCulture);
        string sentence = entry.Action switch
        {
            "armed-auto-upload" => "Automatic contributing turned on for",
            "disarmed-auto-upload" => "Automatic contributing turned off for",
            "queue-bulk-approved" => "The whole queue was approved",
            "consent-scopes-changed" => "Permissions changed",
            "near-ai-notice-acknowledged" => "The extra privacy scan was confirmed",
            _ => "Changed",
        };
        WhatText = string.IsNullOrWhiteSpace(entry.ProjectLabel)
            ? sentence
            : sentence + " " + entry.ProjectLabel;
    }

    public string AtText { get; }

    public string WhatText { get; }
}

public sealed class ConnectionStatusViewModel
{
    public ConnectionStatusViewModel(string text, bool configured)
    {
        Text = text;
        State = configured ? "Set" : "Default";
    }

    public string Text { get; }

    public string State { get; }
}

public sealed class ProjectSettingViewModel : INotifyPropertyChanged
{
    private string _mode;

    public ProjectSettingViewModel(ProjectSetting project)
    {
        ArgumentNullException.ThrowIfNull(project);
        ProjectId = project.ProjectId;

        // The daemon marks this row; this shell never infers it from the label,
        // which is display text and carries the slug "unknown-project". Note
        // the blank-label fallback below does NOT cover the bucket: that slug is
        // not blank, which is why this row rendered as raw slug for so long.
        IsUnresolvedBucket = project.IsUnresolvedBucket;
        ProjectLabel = IsUnresolvedBucket
            ? UnresolvedBucketCopy.Label
            : string.IsNullOrWhiteSpace(project.ProjectLabel)
                ? "Unknown project"
                : project.ProjectLabel;
        _mode = project.Mode;
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    public string ProjectId { get; }

    public string ProjectLabel { get; }

    /// <summary>
    /// The row holding sessions whose project the daemon cannot name. It can be
    /// silenced but never armed, and the daemon enforces that itself.
    /// </summary>
    public bool IsUnresolvedBucket { get; }

    /// <summary>
    /// The explanation shown beneath the name, or empty for an ordinary row.
    ///
    /// It sits under the name rather than in the state column: the note is a
    /// sentence and that column holds two or three words, and Settings keeps
    /// its state column populated for every row because a blank cell in a list
    /// reads as a fault rather than as an absence.
    /// </summary>
    public string Note => IsUnresolvedBucket ? UnresolvedBucketCopy.Note : string.Empty;

    public bool HasNote => IsUnresolvedBucket;

    public string Mode => _mode;

    public string StateText => _mode switch
    {
        "ignore" => "Never offered",

        // Unreachable for the unresolvable bucket, and deliberately guarded
        // rather than trusted: the daemon refuses auto_upload for it in two
        // places, so if this row ever reported that mode the honest reading is
        // that something is wrong, not that it was armed. Saying "Contributed
        // without asking" there would be the one claim this row must never
        // make.
        "auto_upload" when !UnresolvedBucketCopy.MayOfferAutoUpload(IsUnresolvedBucket)
            => "Asks you first",
        "auto_upload" => "Contributed without asking",
        _ => "Asks you first",
    };

    public string ActionText => _mode == "ignore" ? "Ask again" : "Ignore";

    public bool CanToggle => _mode is "ask" or "ignore";

    public void SetMode(string mode)
    {
        if (_mode == mode)
        {
            return;
        }

        _mode = mode;
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(Mode)));
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(StateText)));
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(ActionText)));
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(CanToggle)));
    }
}

/// <summary>
/// One tool's name and its one word.
/// </summary>
/// <remarks>
/// Both strings come from the shared source across the C ABI; nothing here
/// composes wording, and no property here derives a second verdict from the
/// word.
///
/// The wired row is toned, matching the GTK shell. The tone arrives on
/// <see cref="RoutingToolRow"/>, decided by the same shared branch table that
/// chose the word, and NOTHING here reads <see cref="Word"/> to reach it: the
/// wired word is a substring of a denial that must never come back, and a
/// test of the word's text to decide how to paint it is one <c>Contains</c>
/// away from the bug that matched "unreachable" as "reachable" on this same
/// surface.
/// </remarks>
public sealed class RoutingToolRowViewModel
{
    public RoutingToolRowViewModel(RoutingToolRow row)
    {
        ArgumentNullException.ThrowIfNull(row);
        Name = row.Name;
        Word = row.Word;
        Tone = row.Tone;
        AccessibleLabel = row.AccessibleLabel;
    }

    public string Name { get; }

    public string Word { get; }

    /// <summary>
    /// How the word is painted, straight from the shared table.
    /// </summary>
    public RoutingTone Tone { get; }

    /// <summary>
    /// The XAML projection of <see cref="Tone"/>, and only that.
    ///
    /// Two visibilities rather than one bound brush because the tone colours
    /// live in a theme dictionary and only <c>ThemeResource</c> resolves them
    /// correctly in both themes. Both read the enum; neither reads the word.
    /// </summary>
    public bool ShowsClearWord => Tone == RoutingTone.Clear;

    /// <summary>The other half of <see cref="ShowsClearWord"/>.</summary>
    public bool ShowsNeutralWord => Tone != RoutingTone.Clear;

    /// <summary>The row read as one statement, for a screen reader.</summary>
    public string AccessibleLabel { get; }
}
