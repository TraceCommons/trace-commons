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
    private string? _routingLastChecked;
    private RoutingModes _routingModes = new();

    public ContributorSettingsViewModel(DaemonHost host)
    {
        _host = host ?? throw new ArgumentNullException(nameof(host));
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

    private void FillSettings(DaemonSettingsSnapshot? settings)
    {
        ConnectionRows.Clear();
        if (settings is null)
        {
            return;
        }

        ConnectionRows.Add(new ConnectionStatusViewModel(
            settings.ClaudeRootConfigured
                ? "Claude Code sessions folder set"
                : "Claude Code sessions read from the usual place",
            settings.ClaudeRootConfigured));
        ConnectionRows.Add(new ConnectionStatusViewModel(
            settings.CodexRootConfigured
                ? "Codex sessions folder set"
                : "Codex sessions read from the usual place",
            settings.CodexRootConfigured));
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

    private async Task WriteRoutingAsync(bool on)
    {
        _routingEvidence = null;
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

        RoutingProbeText = _routingCopy.Checking;
        IsBusy = true;
        try
        {
            string payload = TraceCommons.Interop.RoutingTools.SerializeProbeParams(
                RoutingPortValue(),
                RoutingTokenDir);
            DaemonResponse response = await _host
                .CallAsync(DaemonProtocol.Methods.ProbeRoutedTools, payload)
                .ConfigureAwait(true);

            if (response.IsError || response.Result is null)
            {
                _routingEvidence = null;
                RoutingProbeText = _routingCopy.CheckUnavailable;
            }
            else
            {
                _routingEvidence = RoutingEvidence.Parse(response.Result.Value.GetRawText());
                RoutingProbeText = TraceCommons.Interop.RoutingTools.ProbeLine(
                    _routingCopy,
                    _routingEvidence.Outcome);
            }

            RenderRoutingToolRows();
        }
        finally
        {
            IsBusy = false;
        }
    }

    /// <summary>
    /// Fills the declaration controls and the state line from the daemon's
    /// own answer.
    /// </summary>
    private void FillRouting(DaemonSettingsSnapshot? settings, DaemonStatus? status)
    {
        _routingModes = new RoutingModes
        {
            Claude = settings?.ClaudeSourceMode ?? string.Empty,
            Codex = settings?.CodexSourceMode ?? string.Empty,
            Gemini = settings?.GeminiSourceMode ?? string.Empty,
        };

        bool declared = settings?.RoutingDeclared ?? false;
        if (!declared)
        {
            // Nothing is declared, so nothing held about IronWire is still
            // about this machine's current state. Dropped rather than kept,
            // so turning the switch back on cannot paint a stale verdict
            // before a new answer lands.
            _routingEvidence = null;
            RoutingProbeText = string.Empty;
        }

        RoutingDeclared = declared;
        RoutingPort = settings?.Routing?.Port ?? TraceCommons.Interop.RoutingTools.DefaultPort;
        RoutingTokenDir = settings?.Routing?.TokenDir ?? string.Empty;
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
/// word. The four words are styled identically, deliberately: the wired word
/// is a substring of a denial that must never come back, and any test of the
/// word's text to decide how to paint it is one <c>Contains</c> away from the
/// bug that matched "unreachable" as "reachable" on this same surface.
/// </remarks>
public sealed class RoutingToolRowViewModel
{
    public RoutingToolRowViewModel(RoutingToolRow row)
    {
        ArgumentNullException.ThrowIfNull(row);
        Name = row.Name;
        Word = row.Word;
        AccessibleLabel = row.AccessibleLabel;
    }

    public string Name { get; }

    public string Word { get; }

    /// <summary>The row read as one statement, for a screen reader.</summary>
    public string AccessibleLabel { get; }
}
