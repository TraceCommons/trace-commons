using System;
using System.IO;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// The shell's own choices for Home, the tray and the rail, kept beside the
/// app's other local state. The Mac keeps the same keys in its defaults:
/// <c>CreditIconAnimationEnabled</c> (BalanceArrival.swift:140, on unless
/// turned off), <c>edge_rail_enabled</c> (TCShellCore/EdgeRail.swift:7, off
/// until turned on), <c>home.closedCampaignFingerprint</c> and
/// <c>home.showsPromotions</c> (HomeViews.swift:126-129), and the Balance
/// tile's last-shown reading (BalanceArrivalFlourish.swift:115-116). None of
/// them is a daemon setting: the core does nothing differently for them.
/// </summary>
public sealed class ShellPreferences
{
    private readonly string _path;
    private Values _values;

    public ShellPreferences(string path)
    {
        _path = path ?? throw new ArgumentNullException(nameof(path));
        _values = Read(path);
    }

    private static readonly Lazy<ShellPreferences> SharedPreferences = new(Default);

    /// <summary>The one instance the app's windows share, so a switch in Settings is seen by the window at once.</summary>
    public static ShellPreferences Shared => SharedPreferences.Value;

    /// <summary>Choices hold for this run only and are never written: a sample launch writes nothing.</summary>
    public bool ReadOnly { get; set; }

    /// <summary>Raised after any choice changes.</summary>
    public event Action? Changed;

    /// <summary>The default file under the user's local application data.</summary>
    public static ShellPreferences Default() => new(Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
        "TraceCommons",
        "shell-preferences.json"));

    /// <summary>Settings' credit-arrival switch: on unless turned off.</summary>
    public bool CreditIconAnimation
    {
        get => CreditCelebration.IsOn(_values.CreditIconAnimation);
        set => Write(_values with { CreditIconAnimation = value });
    }

    /// <summary>Settings' edge rail switch: off until turned on.</summary>
    public bool EdgeRail
    {
        get => _values.EdgeRail ?? false;
        set => Write(_values with { EdgeRail = value });
    }

    /// <summary>The campaign closed with its X, as it was then; empty for none.</summary>
    public string ClosedCampaign
    {
        get => _values.ClosedCampaign ?? string.Empty;
        set => Write(_values with { ClosedCampaign = value });
    }

    /// <summary>Whether the user dismissed the Red Team setup instructions in Private AI.</summary>
    public bool RedTeamSetupDismissed
    {
        get => _values.RedTeamSetupDismissed;
        set => Write(_values with { RedTeamSetupDismissed = value });
    }

    /// <summary>Settings' "Show promotions": on unless turned off.</summary>
    public bool ShowsPromotions
    {
        get => _values.ShowsPromotions ?? true;
        set => Write(_values with { ShowsPromotions = value });
    }

    /// <summary>The remembered widths of the main pane beside the map and of the map itself.</summary>
    public (double Main, double Map) MonitorPaneWidths => (
        ValidPaneWidth(_values.MonitorMainWidth, MonitorPaneLayout.MainWidth, MonitorPaneLayout.MainWidth),
        ValidPaneWidth(_values.MonitorMapWidth, MonitorPaneLayout.MapMinWidth, MonitorPaneLayout.DefaultMapWidth));

    public void KeepMonitorPaneWidths(double main, double map) => Write(_values with
    {
        MonitorMainWidth = ValidPaneWidth(main, MonitorPaneLayout.MainWidth, MonitorPaneLayout.MainWidth),
        MonitorMapWidth = ValidPaneWidth(map, MonitorPaneLayout.MapMinWidth, MonitorPaneLayout.DefaultMapWidth),
    }, notify: false);

    private static double ValidPaneWidth(double? value, double minimum, double fallback) =>
        value is double width && double.IsFinite(width) && width >= minimum ? width : fallback;

    /// <summary>What the Balance tile last drew, so credit that arrived while the app was closed is marked next time.</summary>
    public BalanceArrival.Reading? BalanceShown
    {
        get => _values.BalanceShownReceived is long received ? new BalanceArrival.Reading(received, _values.BalanceShownRemaining) : null;
        set => Write(_values with { BalanceShownReceived = value?.ReceivedNanos, BalanceShownRemaining = value?.RemainingNanos });
    }

    /// <summary>The credit-arrival baseline saved for <paramref name="scope"/>, or null for no scope or another account's.</summary>
    public (long? ReceivedNanos, double? CreditFinal)? CreditBaseline(string? scope) =>
        scope is not null && _values.CreditBaselineScope == scope
            ? (_values.CreditBaselineReceived, _values.CreditBaselineFinal)
            : null;

    /// <summary>Keeps the credit-arrival baseline for <paramref name="scope"/>. Without a scope nothing is kept.</summary>
    public void SetCreditBaseline(string? scope, long? receivedNanos, double? creditFinal)
    {
        if (scope is null)
        {
            return;
        }

        Write(_values with
        {
            CreditBaselineScope = scope,
            CreditBaselineReceived = receivedNanos,
            CreditBaselineFinal = creditFinal,
        });
    }

    private void Write(Values values, bool notify = true)
    {
        bool changed = values != _values;
        _values = values;
        if (changed && notify)
        {
            Changed?.Invoke();
        }

        if (ReadOnly)
        {
            return;
        }

        try
        {
            Directory.CreateDirectory(Path.GetDirectoryName(_path)!);
            string temp = _path + ".tmp";
            File.WriteAllText(temp, JsonSerializer.Serialize(values));
            File.Move(temp, _path, overwrite: true);
        }
        catch (IOException)
        {
            // A choice that could not be saved still holds for this run.
        }
        catch (UnauthorizedAccessException)
        {
        }
    }

    private static Values Read(string path)
    {
        try
        {
            return File.Exists(path) ? JsonSerializer.Deserialize<Values>(File.ReadAllText(path)) ?? new Values() : new Values();
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or JsonException)
        {
            // Unreadable reads as never chosen: every default is the Mac's.
            return new Values();
        }
    }

    private sealed record Values
    {
        [JsonPropertyName("credit_icon_animation")]
        public bool? CreditIconAnimation { get; init; }

        [JsonPropertyName("edge_rail")]
        public bool? EdgeRail { get; init; }

        [JsonPropertyName("closed_campaign")]
        public string? ClosedCampaign { get; init; }

        [JsonPropertyName("red_team_setup_dismissed")]
        public bool RedTeamSetupDismissed { get; init; }

        [JsonPropertyName("shows_promotions")]
        public bool? ShowsPromotions { get; init; }

        [JsonPropertyName("monitor_main_width")]
        public double? MonitorMainWidth { get; init; }

        [JsonPropertyName("monitor_map_width")]
        public double? MonitorMapWidth { get; init; }

        [JsonPropertyName("balance_shown_received")]
        public long? BalanceShownReceived { get; init; }

        [JsonPropertyName("balance_shown_remaining")]
        public long? BalanceShownRemaining { get; init; }

        [JsonPropertyName("credit_baseline_scope")]
        public string? CreditBaselineScope { get; init; }

        [JsonPropertyName("credit_baseline_received")]
        public long? CreditBaselineReceived { get; init; }

        [JsonPropertyName("credit_baseline_final")]
        public double? CreditBaselineFinal { get; init; }
    }
}
