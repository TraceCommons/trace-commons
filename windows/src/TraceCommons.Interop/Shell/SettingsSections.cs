using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// The Settings window's sections, in the Mac's grouped order
/// (<c>SettingsSection</c>, SettingsSections.swift:12-50; spec W16-D1 (a)):
/// Account, then what is watched, then what is shared, then Private AI, then
/// the app's own preferences, then the advanced sections. The declaration
/// order is the body's order.
/// </summary>
public enum SettingsSection
{
    Account,
    /// <summary>Listed only while something in it applies (SettingsModal.swift:34-41).</summary>
    Payouts,
    Referrals,
    Watching,
    /// <summary>Drawn under Watching's name (SettingsSections.swift:22-24).</summary>
    WatchedFolders,
    Tools,
    Insights,
    Projects,
    Consent,
    PublicProfile,
    Changes,
    PrivateAI,
    SavedAccounts,
    General,
    Startup,
    /// <summary>Drawn under Startup's name (SettingsSections.swift:81-85).</summary>
    Notifications,
    Updates,
    Witness,
    Compute,
}

/// <summary>A section's group; the list draws a hairline where it changes (SettingsSections.swift:62-76).</summary>
public enum SettingsGroup
{
    Account,
    Capture,
    Sharing,
    PrivateAI,
    App,
    Advanced,
}

/// <summary>
/// What the list draws for a section: its title, or, while the words that
/// name it have not loaded, a disabled placeholder (<c>ListRow</c>,
/// SettingsSections.swift:120-136). A row never vanishes.
/// </summary>
public readonly record struct SettingsListRow(string Text, bool Enabled)
{
    /// <summary>The Mac's placeholder for a row whose words have not loaded.</summary>
    public const string Placeholder = "—";

    public static SettingsListRow Row(string? title) =>
        string.IsNullOrEmpty(title) ? new SettingsListRow(Placeholder, false) : new SettingsListRow(title, true);
}

/// <summary>
/// The words the list names sections by, each from the core
/// (<c>SettingsSection.navName</c>, SettingsSections.swift:94-118): the
/// monitor table's <c>settings_nav</c>, the Insights title, the shell's
/// Updates eyebrow, the referral words' title, the managed table's saved
/// accounts title and the General word. A word the core did not give is
/// null, which the list draws as its placeholder.
/// </summary>
public sealed record SettingsNavWords(
    IReadOnlyDictionary<string, string> Nav,
    string? Insights,
    string? Updates,
    string? Referrals,
    string? SavedAccounts,
    string? General)
{
    /// <summary>A <c>settings_nav</c> word, or null when absent.</summary>
    public string? NavWord(string key) => Nav.TryGetValue(key, out string? word) ? word : null;

    /// <summary>
    /// Reads the <c>settings_nav</c> object out of the monitor table's JSON
    /// (<c>tc_monitor_screens_copy_json</c>). Anything else reads as no words.
    /// </summary>
    public static IReadOnlyDictionary<string, string> ParseNav(string? monitorScreensJson)
    {
        var nav = new Dictionary<string, string>(StringComparer.Ordinal);
        if (string.IsNullOrEmpty(monitorScreensJson))
        {
            return nav;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(monitorScreensJson);
            if (doc.RootElement.ValueKind == JsonValueKind.Object
                && doc.RootElement.TryGetProperty("settings_nav", out JsonElement table)
                && table.ValueKind == JsonValueKind.Object)
            {
                foreach (JsonProperty field in table.EnumerateObject())
                {
                    if (field.Value.ValueKind == JsonValueKind.String && field.Value.GetString() is { } word)
                    {
                        nav[field.Name] = word;
                    }
                }
            }
        }
        catch (JsonException)
        {
            nav.Clear();
        }

        return nav;
    }

    /// <summary>One string field of a JSON object, or null.</summary>
    public static string? Field(string? json, string key)
    {
        if (string.IsNullOrEmpty(json))
        {
            return null;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            return doc.RootElement.ValueKind == JsonValueKind.Object
                && doc.RootElement.TryGetProperty(key, out JsonElement value)
                && value.ValueKind == JsonValueKind.String
                ? value.GetString()
                : null;
        }
        catch (JsonException)
        {
            return null;
        }
    }
}

/// <summary>The Mac's rules for the Settings window's sections (SettingsSections.swift:52-136).</summary>
public static class SettingsSections
{
    /// <summary>Every section, in the body's order (<c>SettingsModal.sections</c>).</summary>
    public static readonly IReadOnlyList<SettingsSection> All =
        Enum.GetValues<SettingsSection>().ToArray();

    /// <summary>The sections the list can name, in order (<c>SettingsSection.listed</c>).</summary>
    public static readonly IReadOnlyList<SettingsSection> Listed = All.Where(IsListed).ToArray();

    /// <summary>
    /// Whether the window draws this section while onboarding is required
    /// (<c>availableBeforeOnboarding</c>, SettingsSections.swift:52-60, R-43).
    /// </summary>
    public static bool AvailableBeforeOnboarding(SettingsSection section) => section switch
    {
        SettingsSection.General or SettingsSection.Account or SettingsSection.Startup
            or SettingsSection.Notifications or SettingsSection.Updates or SettingsSection.PrivateAI
            or SettingsSection.Compute => true,
        _ => false,
    };

    /// <summary>The section's group (SettingsSections.swift:68-77).</summary>
    public static SettingsGroup Group(SettingsSection section) => section switch
    {
        SettingsSection.Account or SettingsSection.Payouts or SettingsSection.Referrals => SettingsGroup.Account,
        SettingsSection.Watching or SettingsSection.WatchedFolders or SettingsSection.Tools
            or SettingsSection.Insights => SettingsGroup.Capture,
        SettingsSection.Projects or SettingsSection.Consent or SettingsSection.PublicProfile
            or SettingsSection.Changes => SettingsGroup.Sharing,
        SettingsSection.PrivateAI or SettingsSection.SavedAccounts => SettingsGroup.PrivateAI,
        SettingsSection.General or SettingsSection.Startup or SettingsSection.Notifications
            or SettingsSection.Updates => SettingsGroup.App,
        _ => SettingsGroup.Advanced,
    };

    /// <summary>Whether the list names this section (SettingsSections.swift:79-85).</summary>
    public static bool IsListed(SettingsSection section) =>
        section != SettingsSection.Notifications && section != SettingsSection.WatchedFolders;

    /// <summary>
    /// Whether the list draws a hairline above this row: the first row of
    /// every group but the first (SettingsSections.swift:87-92, D17).
    /// </summary>
    public static bool StartsGroup(SettingsSection section, IReadOnlyList<SettingsSection> rows)
    {
        int index = -1;
        for (int i = 0; i < rows.Count; i++)
        {
            if (rows[i] == section)
            {
                index = i;
                break;
            }
        }

        return index > 0 && Group(rows[index - 1]) != Group(section);
    }

    /// <summary>
    /// The rows the list draws now: every listed section but Payouts while
    /// nothing in it applies (<c>SettingsModal.rows</c>, SettingsModal.swift:38-41).
    /// </summary>
    public static IReadOnlyList<SettingsSection> Rows(bool payoutsShown) =>
        Listed.Where(s => s != SettingsSection.Payouts || payoutsShown).ToArray();

    /// <summary>
    /// The name the list and the body's rule use for a section, or null for a
    /// section drawn under the one before it and while the words have not
    /// loaded (<c>navName</c>, SettingsSections.swift:94-118). General never
    /// waits on the rest of the table.
    /// </summary>
    public static string? NavName(SettingsSection section, SettingsNavWords? words)
    {
        if (section == SettingsSection.General)
        {
            return words?.General;
        }

        if (words is null)
        {
            return null;
        }

        return section switch
        {
            SettingsSection.Account => words.NavWord("account"),
            SettingsSection.Payouts => words.NavWord("payouts"),
            SettingsSection.Referrals => words.Referrals,
            SettingsSection.Watching => words.NavWord("watching"),
            SettingsSection.WatchedFolders or SettingsSection.Notifications => null,
            SettingsSection.Tools => words.NavWord("tools"),
            SettingsSection.Insights => words.Insights,
            SettingsSection.Projects => words.NavWord("projects"),
            SettingsSection.Consent => words.NavWord("uses"),
            SettingsSection.PublicProfile => words.NavWord("profile"),
            SettingsSection.Changes => words.NavWord("log"),
            SettingsSection.PrivateAI => words.NavWord("private_ai"),
            SettingsSection.SavedAccounts => words.SavedAccounts,
            SettingsSection.Startup => words.NavWord("startup"),
            SettingsSection.Updates => words.Updates,
            SettingsSection.Witness => words.NavWord("witness"),
            SettingsSection.Compute => words.NavWord("compute"),
            _ => null,
        };
    }

    /// <summary>The row the list draws for a section (<c>listRow</c>, SettingsSections.swift:120-123).</summary>
    public static SettingsListRow ListRow(SettingsSection section, SettingsNavWords? words) =>
        SettingsListRow.Row(NavName(section, words));
}
