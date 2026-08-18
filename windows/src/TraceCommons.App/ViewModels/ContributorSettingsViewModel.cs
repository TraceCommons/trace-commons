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
            FillSettings(settingsResponse.ResultAs<DaemonSettingsSnapshot>());

            DaemonResponse optionsResponse = await _host
                .CallAsync(DaemonProtocol.Methods.ConsentOptions)
                .ConfigureAwait(true);
            FillConsent(
                optionsResponse.ResultAs<ConsentOptionsPayload>(),
                status?.ConsentScopes ?? new List<string>());

            await LoadProjectsAsync().ConfigureAwait(true);
            await LoadAuditAsync().ConfigureAwait(true);

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
            SetDisplayedBehavior(setting, displayedValue);
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
        ProjectLabel = string.IsNullOrWhiteSpace(project.ProjectLabel)
            ? "Unknown project"
            : project.ProjectLabel;
        _mode = project.Mode;
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    public string ProjectId { get; }

    public string ProjectLabel { get; }

    public string Mode => _mode;

    public string StateText => _mode switch
    {
        "ignore" => "Never offered",
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
