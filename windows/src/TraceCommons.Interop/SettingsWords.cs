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

/// <summary>
/// The words a section draws where its read has not answered
/// (<c>SettingsUnavailable</c>, SettingsStateRow.swift:85-109): the core's
/// request-failed line and the Settings table's Retry.
/// </summary>
public static class SettingsReadWords
{
    private static readonly Lazy<string?> RetryWord = new(() =>
        SettingsNavWords.Field(NativeMethods.TakeOwnedString(NativeMethods.tc_source_settings_copy()), "retry"));

    /// <summary>The line for a failed read; a dash when the core gave none.</summary>
    public static string Failed
    {
        get
        {
            string line = MonitorShellCopy.Top("request_failed");
            return line.Length > 0 ? line : SettingsListRow.Placeholder;
        }
    }

    /// <summary>Retry, or null when the core gave no word (then no button is drawn).</summary>
    public static string? Retry => RetryWord.Value;
}

/// <summary>
/// The Settings words the Mac reads from the core's shell table
/// (<c>SettingsLegacyWords</c>, SettingsView.swift:35-115, over
/// <c>shell_words_copy::settings_words</c>). Empty when the table or the
/// word is missing, so a control then says nothing.
/// </summary>
public static class SettingsLegacyWords
{
    private static readonly Lazy<string?> Table = new(() =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_shell_words_copy_json()));

    /// <summary>One word of the table's <c>settings</c> object.</summary>
    public static string Word(string key) => MonitorWords.WordAt(Table.Value, "settings", key);

    /// <summary><c>undecidedDropped</c> (:52-54): <c>{days}</c> filled.</summary>
    public static string UndecidedDropped(long days) => Fill(Word("undecided_dropped"), "days", days.ToString(System.Globalization.CultureInfo.InvariantCulture));

    /// <summary>The change log's heading (<c>SettingsContent.auditHeading</c>, SettingsView.swift:18).</summary>
    public static string ChangesHeading => MonitorShellCopy.Shell("changes_heading");

    /// <summary>The change log with nothing in it (<c>nothingChanged</c>, :111).</summary>
    public static string NothingChanged => Word("nothing_changed");

    /// <summary>
    /// One change's sentence (<c>SettingsContent.auditSentence</c>,
    /// SettingsView.swift:23-28): the core's sentence for the fixed action
    /// label, or its generic one for a label this build does not know, then
    /// the project's name when there is one. Empty without the table.
    /// </summary>
    public static string AuditSentence(string action, string? project) => AuditSentence(Table.Value, action, project);

    internal static string AuditSentence(string? json, string action, string? project)
    {
        string sentence = MonitorWords.WordAt(json, "settings", "audit_changed");
        if (string.IsNullOrEmpty(json))
        {
            return string.Empty;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            if (doc.RootElement.ValueKind == JsonValueKind.Object
                && doc.RootElement.TryGetProperty("settings", out JsonElement settings)
                && settings.ValueKind == JsonValueKind.Object
                && settings.TryGetProperty("audit_actions", out JsonElement actions)
                && actions.ValueKind == JsonValueKind.Object
                && actions.TryGetProperty(action, out JsonElement known)
                && known.ValueKind == JsonValueKind.String)
            {
                sentence = known.GetString() ?? sentence;
            }
        }
        catch (JsonException)
        {
            return string.Empty;
        }

        return string.IsNullOrEmpty(project) ? sentence : sentence + " " + project;
    }

    internal static string Fill(string template, string key, string value) =>
        template.Replace("{" + key + "}", value, StringComparison.Ordinal);
}
