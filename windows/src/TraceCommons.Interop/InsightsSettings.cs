using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

/// <summary>
/// A daemon-backed Insights setting (<c>AppModel.setInsightsSwitch</c>).
/// The switch reads the value the daemon
/// confirmed and is drawn only once the daemon reported one. A write is one
/// <c>set_settings</c>; it stands only when the reply carries the asked
/// value, otherwise the reply's value (when it has one) is kept and the
/// core's request-failed line shows; a failed call reads the settings again
/// and shows the same line. One write at a time.
/// </summary>
public abstract class InsightsSettingSwitch
{
    private readonly Func<string, string, Task<DaemonResponse>> _call;
    private readonly string _key;
    private readonly Func<DaemonSettingsSnapshot, bool?> _value;

    protected InsightsSettingSwitch(Func<string, string, Task<DaemonResponse>> call,
        string key, Func<DaemonSettingsSnapshot, bool?> value)
    {
        _call = call;
        _key = key;
        _value = value;
    }

    /// <summary>The settings have answered (<c>daemonSettings != nil</c>).</summary>
    public bool SettingsAnswered { get; private set; }

    /// <summary>The latest settings read failed.</summary>
    public bool ReadFailed { get; private set; }

    /// <summary>The confirmed value; null until the daemon reports one.</summary>
    public bool? On { get; private set; }

    /// <summary>A write is out (<c>insightsSwitchBusy</c>).</summary>
    public bool Busy { get; private set; }

    /// <summary>The refusal line under the switch, or null (<c>insightsSwitchRefusal</c>).</summary>
    public string? Refusal { get; private set; }

    /// <summary>
    /// The request-failed line the Mac shows (<c>MonitorWords.table?.requestFailed</c>).
    /// Settable so tests need not load the core.
    /// </summary>
    public Func<string> RefusalLine { get; init; } = () => MonitorShellCopy.Top("request_failed");

    /// <summary>Reads the settings (<c>refreshSettings</c>, run each time the card appears).</summary>
    public async Task ReadAsync()
    {
        DaemonSettingsSnapshot? settings = await SettingsAsync(DaemonProtocol.Methods.GetSettings, "{}").ConfigureAwait(true);
        ReadFailed = settings is null;
        if (settings is not null)
        {
            SettingsAnswered = true;
            On = _value(settings);
        }
    }

    /// <summary>Writes the switch (AppModel.swift:3159-3184).</summary>
    public async Task SetAsync(bool on)
    {
        if (Busy)
        {
            return;
        }

        Busy = true;
        try
        {
            string parameters = JsonSerializer.Serialize(new Dictionary<string, bool> { [_key] = on });
            DaemonSettingsSnapshot? reply = await SettingsAsync(DaemonProtocol.Methods.SetSettings, parameters).ConfigureAwait(true);
            if (reply is not null && _value(reply) == on)
            {
                On = on;
                Refusal = null;
            }
            else if (reply is not null)
            {
                if (_value(reply) is { } kept)
                {
                    On = kept;
                }

                Refusal = RefusalLine();
            }
            else
            {
                if (await SettingsAsync(DaemonProtocol.Methods.GetSettings, "{}").ConfigureAwait(true) is { } confirmed)
                {
                    On = _value(confirmed);
                }

                Refusal = RefusalLine();
            }
        }
        finally
        {
            Busy = false;
        }
    }

    private async Task<DaemonSettingsSnapshot?> SettingsAsync(string method, string parameters)
    {
        try
        {
            DaemonResponse response = await _call(method, parameters).ConfigureAwait(true);
            return response.IsError ? null : response.ResultAs<DaemonSettingsSnapshot>();
        }
        catch (Exception)
        {
            return null;
        }
    }
}

public sealed class InsightsRecapSwitch(Func<string, string, Task<DaemonResponse>> call)
    : InsightsSettingSwitch(call, InsightsRecapSwitch.Key, settings => settings.InsightsRecapCardEnabled)
{
    public const string Key = "insights_recap_card_enabled";
}

public sealed class InsightsLedgerFeedSwitch(Func<string, string, Task<DaemonResponse>> call)
    : InsightsSettingSwitch(call, InsightsLedgerFeedSwitch.Key, settings => settings.InsightsLedgerFeed)
{
    public const string Key = "insights_ledger_feed";
}

/// <summary>The Insights words this card reads (<c>tc_insights_copy_json</c>, <c>TCInsights.copy</c>).</summary>
public static class InsightsSettingsWords
{
    private static readonly Lazy<IReadOnlyDictionary<string, string>> Words = new(() =>
        Parse(NativeMethods.TakeOwnedString(NativeMethods.tc_insights_copy_json())));

    /// <summary>A word, or empty when the core gave none (<c>InsightsOverviewWords.text</c>).</summary>
    public static string Text(string key) => Words.Value.TryGetValue(key, out string? word) ? word : string.Empty;

    internal static IReadOnlyDictionary<string, string> Parse(string? json)
    {
        var words = new Dictionary<string, string>();
        if (string.IsNullOrEmpty(json))
        {
            return words;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            if (doc.RootElement.ValueKind != JsonValueKind.Object)
            {
                return words;
            }

            foreach (JsonProperty property in doc.RootElement.EnumerateObject())
            {
                if (property.Value.ValueKind == JsonValueKind.String)
                {
                    words[property.Name] = property.Value.GetString() ?? string.Empty;
                }
            }
        }
        catch (JsonException)
        {
        }

        return words;
    }
}
