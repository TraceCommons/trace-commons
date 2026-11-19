using System;
using System.Text.Json;
using TraceCommons.Interop.Shell;

namespace TraceCommons.Interop;

/// <summary>
/// The General section's words (<c>tc_settings_general_copy_json</c>): the
/// words the Mac spells in Swift (<c>SettingsWords</c>,
/// SettingsSections.swift:191-196) and the Windows app reads from the core.
/// Null for an unparseable payload or an empty field, never a partly filled
/// value.
/// </summary>
public sealed record SettingsGeneralCopy(string General, string Appearance, string Light, string Dark, string System)
{
    public static SettingsGeneralCopy? Decode(string? json)
    {
        if (string.IsNullOrEmpty(json))
        {
            return null;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            JsonElement root = doc.RootElement;
            if (root.ValueKind != JsonValueKind.Object)
            {
                return null;
            }

            string? Read(string key) =>
                root.TryGetProperty(key, out JsonElement value) && value.ValueKind == JsonValueKind.String
                    && value.GetString() is { } word && word.Trim().Length > 0
                    ? word
                    : null;

            return Read("general") is { } general && Read("appearance") is { } appearance
                && Read("light") is { } light && Read("dark") is { } dark && Read("system") is { } system
                ? new SettingsGeneralCopy(general, appearance, light, dark, system)
                : null;
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>The core's words, read once.</summary>
    public static SettingsGeneralCopy? Live => LiveCopy.Value;

    private static readonly Lazy<SettingsGeneralCopy?> LiveCopy = new(() =>
        Decode(NativeMethods.TakeOwnedString(NativeMethods.tc_settings_general_copy_json())));
}

/// <summary>
/// The live words the Settings window's list names sections by, each read
/// from the core where the Mac reads it (<c>SettingsSection.navName</c>,
/// SettingsSections.swift:94-118). Read once, as the Mac's static words are.
/// </summary>
public static class SettingsNavWordsLive
{
    private static readonly Lazy<SettingsNavWords> Words = new(Load);

    public static SettingsNavWords Value => Words.Value;

    private static SettingsNavWords Load()
    {
        string? monitor = NativeMethods.TakeOwnedString(NativeMethods.tc_monitor_screens_copy_json());
        string? insights = MonitorWords.TabTitle(MonitorTab.Insights);
        string? updates = MonitorShellCopy.Shell("updates_eyebrow");
        string? referrals = SettingsNavWords.Field(
            NativeMethods.TakeOwnedString(NativeMethods.tc_referral_copy_json()), "title");
        string? saved = SettingsNavWords.Field(
            NativeMethods.TakeOwnedString(NativeMethods.tc_managed_copy_json()), "accounts_title");
        return new SettingsNavWords(
            SettingsNavWords.ParseNav(monitor),
            NullIfEmpty(insights),
            NullIfEmpty(updates),
            NullIfEmpty(referrals),
            NullIfEmpty(saved),
            SettingsGeneralCopy.Live?.General);
    }

    private static string? NullIfEmpty(string? word) => string.IsNullOrEmpty(word) ? null : word;
}
