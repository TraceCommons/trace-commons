using System;
using System.Text.Json;

namespace TraceCommons.Interop.Shell;

/// <summary>The toolbar's three pane toggles (MonitorWindowView.swift:923-940).</summary>
public enum PaneToggle
{
    Graph,
    Map,
    Inspector,
}

/// <summary>
/// Every word the Monitor window's shell draws: the tabs, the breadcrumb, the
/// toolbar toggles, the gear, a notice's X and the window titles. Each is the
/// core's, read across the C ABI, exactly where the Mac reads it; none is
/// written here, and a missing table leaves a word empty rather than typed.
/// </summary>
public static class MonitorWords
{
    private static readonly Lazy<string> Insights = new(() => InsightsTitleFrom(
        NativeMethods.TakeOwnedString(NativeMethods.tc_insights_copy_json())));

    private static readonly Lazy<string> App = new(() =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_app_name()) ?? string.Empty);

    private static readonly Lazy<string> Channel = new(() =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_channel_window_label()) ?? string.Empty);

    /// <summary>
    /// A tab's title (<c>Tab.title</c>, MonitorWindowView.swift:39-47):
    /// the shell table's three tab words, and for Insights the Insights
    /// service's own title (<c>HomeTabView.insightsHeading</c>, HomeViews.swift:63).
    /// </summary>
    public static string TabTitle(MonitorTab tab) => tab switch
    {
        MonitorTab.Home => MonitorShellCopy.Shell("tab_home"),
        MonitorTab.Inference => MonitorShellCopy.Shell("tab_inference"),
        MonitorTab.Traces => MonitorShellCopy.Shell("tab_traces"),
        _ => Insights.Value,
    };

    /// <summary>
    /// A crumb's word (MonitorWindowView.swift:749-769): a tab's title, the
    /// core's <c>history</c>, or its <c>missions</c> for the drafts page
    /// (HomeViews.swift:69-74).
    /// </summary>
    public static string CrumbTitle(Crumb crumb) => crumb.Word switch
    {
        CrumbWord.History => MonitorShellCopy.Top("history"),
        CrumbWord.Missions => MonitorShellCopy.Top("missions"),
        CrumbWord.MissionCatalogue => MonitorShellCopy.Section("home_history", "mission_catalogue"),
        _ => TabTitle(crumb.Tab),
    };

    /// <summary>
    /// A toggle's name, saying what pressing it does
    /// (<c>MonitorShellWords.graphToggle</c> and siblings, MonitorShell.swift:22-25).
    /// </summary>
    public static string Toggle(PaneToggle toggle, bool shown) => toggle switch
    {
        PaneToggle.Graph => MonitorShellCopy.Shell(shown ? "hide_graph" : "show_graph"),
        PaneToggle.Map => MonitorShellCopy.Shell(shown ? "hide_map" : "show_map"),
        _ => MonitorShellCopy.Shell(shown ? "hide_inspector" : "show_inspector"),
    };

    /// <summary>The tab strip's accessible name (<c>tabs_label</c>).</summary>
    public static string TabsLabel => MonitorShellCopy.Shell("tabs_label");

    /// <summary>The gear's name (<c>settings</c>, TraceCommonsAppMain.swift:431-435).</summary>
    public static string Settings => MonitorShellCopy.Top("settings");

    /// <summary>The Settings window's title (<c>settings_title</c>, TraceCommonsAppMain.swift:156).</summary>
    public static string SettingsTitle => MonitorShellCopy.Top("settings_title");

    /// <summary>A notice's X (<c>close</c>, MonitorNotices.swift:168).</summary>
    public static string Close => MonitorShellCopy.Top("close");

    /// <summary>A notice's Details disclosure (<c>MonitorWords.noticeDetails</c>).</summary>
    public static string NoticeDetails => MonitorShellCopy.Shell("notice_details");

    /// <summary>
    /// The undo card's X while approved traces will still send
    /// (<c>QueueLegacyWords.closeNoticeStillSends</c>, MonitorNotices.swift:661):
    /// closing it lets the approval send, and its name says so.
    /// </summary>
    public static string CloseNoticeStillSends => QueueWord("close_notice_still_sends");

    /// <summary>
    /// The first-contribution note's X (<c>ActionNoticeWords.coreDismissWord</c>,
    /// MonitorNotices.swift:324): the Traces words' <c>dismiss_action</c>.
    /// </summary>
    public static string Dismiss => TracesWord("dismiss_action");

    /// <summary>
    /// The Traces tab's text equivalent for its badge (<c>tracesDescription</c>,
    /// MonitorWindowView.swift:604-606): the core's decisions-owed sentence,
    /// "unavailable" for an unknown count and nothing at zero.
    /// </summary>
    public static string TracesDescription(long? decisionsOwed) =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_decisions_owed_text(decisionsOwed ?? -1)) ?? string.Empty;

    private static readonly Lazy<string?> ShellWordsJson = new(() =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_shell_words_copy_json()));

    private static readonly Lazy<string?> TracesWordsJson = new(() =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_monitor_traces_copy_json()));

    private static string QueueWord(string key) => WordAt(ShellWordsJson.Value, "queue", key);

    private static string TracesWord(string key) => WordAt(TracesWordsJson.Value, null, key);

    /// <summary>
    /// The string at <paramref name="key"/>, inside <paramref name="table"/>
    /// when one is named; empty when the payload or the word is missing.
    /// </summary>
    internal static string WordAt(string? json, string? table, string key)
    {
        if (string.IsNullOrEmpty(json))
        {
            return string.Empty;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            JsonElement root = doc.RootElement;
            if (table is not null)
            {
                if (root.ValueKind != JsonValueKind.Object || !root.TryGetProperty(table, out root))
                {
                    return string.Empty;
                }
            }

            return root.ValueKind == JsonValueKind.Object
                && root.TryGetProperty(key, out JsonElement word)
                && word.ValueKind == JsonValueKind.String
                ? word.GetString() ?? string.Empty
                : string.Empty;
        }
        catch (JsonException)
        {
            return string.Empty;
        }
    }

    /// <summary>The main window's title: the app's name, from <c>brand.rs</c>.</summary>
    public static string AppName => App.Value;

    /// <summary>The staging tag in the toolbar row; empty on production.</summary>
    public static string ChannelLabel => Channel.Value;

    /// <summary>The Insights title out of the shared Insights words; empty when unreadable.</summary>
    internal static string InsightsTitleFrom(string? json)
    {
        if (string.IsNullOrEmpty(json))
        {
            return string.Empty;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            return doc.RootElement.ValueKind == JsonValueKind.Object
                && doc.RootElement.TryGetProperty("title", out JsonElement title)
                && title.ValueKind == JsonValueKind.String
                ? title.GetString() ?? string.Empty
                : string.Empty;
        }
        catch (JsonException)
        {
            return string.Empty;
        }
    }
}
