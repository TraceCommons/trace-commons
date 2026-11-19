using System;
using System.Text.Json;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Monitor window's words come through the real C ABI, from the tables the
/// Mac reads them from, and none is written in this shell.
/// </summary>
/// <remarks>
/// These run against the real cdylib: they are the wiring between
/// <see cref="MonitorWords"/> and the core, not a restatement of it. Each
/// expected word is read here from the export's own JSON, never from the
/// class under test.
/// </remarks>
public class MonitorWordsTests
{
    private static JsonElement ScreensTable()
    {
        string? json = NativeMethods.TakeOwnedString(NativeMethods.tc_monitor_screens_copy_json());
        Assert.False(string.IsNullOrEmpty(json));
        return JsonDocument.Parse(json!).RootElement.Clone();
    }

    [Theory]
    // MonitorWindowView.swift:39-47: the shell table's tab words.
    [InlineData(MonitorTab.Home, "tab_home")]
    [InlineData(MonitorTab.Inference, "tab_inference")]
    [InlineData(MonitorTab.Traces, "tab_traces")]
    public void TabTitlesAreTheShellTablesWords(MonitorTab tab, string key)
    {
        string want = ScreensTable().GetProperty("shell").GetProperty(key).GetString()!;
        Assert.False(string.IsNullOrWhiteSpace(want));
        Assert.Equal(want, MonitorWords.TabTitle(tab));
    }

    [Fact]
    public void TheInsightsTabIsTheInsightsServicesOwnTitle()
    {
        // HomeViews.swift:63: insightsHeading is the Insights copy's "title".
        string? json = NativeMethods.TakeOwnedString(NativeMethods.tc_insights_copy_json());
        string want = JsonDocument.Parse(json!).RootElement.GetProperty("title").GetString()!;
        Assert.False(string.IsNullOrWhiteSpace(want));
        Assert.Equal(want, MonitorWords.TabTitle(MonitorTab.Insights));
    }

    [Theory]
    [InlineData(PaneToggle.Graph, "show_graph", "hide_graph")]
    [InlineData(PaneToggle.Map, "show_map", "hide_map")]
    [InlineData(PaneToggle.Inspector, "show_inspector", "hide_inspector")]
    public void EachToggleNamesWhatPressingItDoes(PaneToggle toggle, string show, string hide)
    {
        // MonitorShell.swift:22-25.
        JsonElement shell = ScreensTable().GetProperty("shell");
        Assert.Equal(shell.GetProperty(show).GetString(), MonitorWords.Toggle(toggle, shown: false));
        Assert.Equal(shell.GetProperty(hide).GetString(), MonitorWords.Toggle(toggle, shown: true));
        Assert.NotEqual(MonitorWords.Toggle(toggle, false), MonitorWords.Toggle(toggle, true));
    }

    [Fact]
    public void TheCrumbsGearXAndSettingsTitleAreTheTablesTopLevelWords()
    {
        JsonElement table = ScreensTable();
        Assert.Equal(table.GetProperty("history").GetString(), MonitorWords.CrumbTitle(new Crumb(CrumbWord.History, MonitorTab.Traces, false)));
        Assert.Equal(table.GetProperty("missions").GetString(), MonitorWords.CrumbTitle(new Crumb(CrumbWord.Missions, MonitorTab.Home, false)));
        Assert.Equal(table.GetProperty("home_history").GetProperty("mission_catalogue").GetString(), MonitorWords.CrumbTitle(new Crumb(CrumbWord.MissionCatalogue, MonitorTab.Home, false)));
        Assert.NotEmpty(MonitorWords.CrumbTitle(new Crumb(CrumbWord.MissionCatalogue, MonitorTab.Home, false)));
        Assert.Equal(table.GetProperty("shell").GetProperty("tab_traces").GetString(), MonitorWords.CrumbTitle(new Crumb(CrumbWord.Tab, MonitorTab.Traces, true)));
        Assert.Equal(table.GetProperty("settings").GetString(), MonitorWords.Settings);
        Assert.Equal(table.GetProperty("settings_title").GetString(), MonitorWords.SettingsTitle);
        Assert.Equal(table.GetProperty("close").GetString(), MonitorWords.Close);
        Assert.Equal(table.GetProperty("shell").GetProperty("tabs_label").GetString(), MonitorWords.TabsLabel);
        foreach (string word in new[] { MonitorWords.Settings, MonitorWords.SettingsTitle, MonitorWords.Close, MonitorWords.TabsLabel })
        {
            Assert.False(string.IsNullOrWhiteSpace(word));
        }
    }

    [Fact]
    public void TheWindowTitleIsTheBrandNameNeverAFormerOrTheCommonsName()
    {
        // brand.rs APP_NAME through tc_app_name; "Near.ai" is a former name
        // (brand.rs FORMER_APP_NAMES) and the commons is never named.
        string name = MonitorWords.AppName;
        Assert.False(string.IsNullOrWhiteSpace(name));
        Assert.NotEqual("Near.ai", name);
        Assert.DoesNotContain("Trace Commons", name, StringComparison.OrdinalIgnoreCase);
        Assert.Equal(NativeMethods.TakeOwnedString(NativeMethods.tc_app_name()), name);
    }

    [Fact]
    public void TheStagingTagIsEmptyOnAProductionBuild()
    {
        // tc_channel_window_label: the core's word on staging, empty on production.
        Assert.Equal(NativeMethods.TakeOwnedString(NativeMethods.tc_channel_window_label()) ?? string.Empty, MonitorWords.ChannelLabel);
    }

    [Fact]
    public void AnUnreadableTableHasNoWords()
    {
        Assert.Empty(MonitorShellCopy.Parse("[1,2]").Top);
        Assert.Empty(MonitorShellCopy.Parse("{not json").Shell);
        Assert.Equal(string.Empty, MonitorWords.InsightsTitleFrom("{\"title\":3}"));
        MonitorShellCopy.Table table = MonitorShellCopy.Parse("{\"history\":\"H\",\"shell\":{\"tab_home\":\"Home\"},\"n\":1}");
        Assert.Equal("H", table.Top["history"]);
        Assert.Equal("Home", table.Shell["tab_home"]);
        Assert.False(table.Top.ContainsKey("n"));
    }

    [Fact]
    public void TheUndoXSaysApprovedTracesStillSend()
    {
        // MonitorNotices.swift:661: QueueLegacyWords.closeNoticeStillSends, the
        // shell words' queue table (shell_words_copy.rs).
        string? json = NativeMethods.TakeOwnedString(NativeMethods.tc_shell_words_copy_json());
        string want = JsonDocument.Parse(json!).RootElement.GetProperty("queue").GetProperty("close_notice_still_sends").GetString()!;
        Assert.False(string.IsNullOrWhiteSpace(want));
        Assert.Equal(want, MonitorWords.CloseNoticeStillSends);
        Assert.NotEqual(MonitorWords.Close, MonitorWords.CloseNoticeStillSends);
    }

    [Fact]
    public void TheFirstContributionXIsTheCoresDismissWord()
    {
        // MonitorNotices.swift:324: ActionNoticeWords.coreDismissWord, the
        // Traces words' dismiss_action (SettingsView.swift:126).
        string? json = NativeMethods.TakeOwnedString(NativeMethods.tc_monitor_traces_copy_json());
        string want = JsonDocument.Parse(json!).RootElement.GetProperty("dismiss_action").GetString()!;
        Assert.False(string.IsNullOrWhiteSpace(want));
        Assert.Equal(want, MonitorWords.Dismiss);
    }

    [Fact]
    public void TheTracesTabsTextEquivalentIsTheCoresDecisionsOwedSentence()
    {
        // MonitorWindowView.swift:604-606: decisionsOwedText; nothing at zero,
        // "unavailable" for an unknown count, never zero.
        Assert.Equal(NativeMethods.TakeOwnedString(NativeMethods.tc_decisions_owed_text(3)), MonitorWords.TracesDescription(3));
        Assert.Equal(NativeMethods.TakeOwnedString(NativeMethods.tc_decisions_owed_text(-1)), MonitorWords.TracesDescription(null));
        Assert.Equal(string.Empty, MonitorWords.TracesDescription(0));
        Assert.False(string.IsNullOrWhiteSpace(MonitorWords.TracesDescription(null)));
        Assert.NotEqual(MonitorWords.TracesDescription(null), MonitorWords.TracesDescription(3));
    }

    [Fact]
    public void AWordMissingFromItsTableIsEmpty()
    {
        Assert.Equal(string.Empty, MonitorWords.WordAt(null, "queue", "close_notice"));
        Assert.Equal(string.Empty, MonitorWords.WordAt("{\"queue\":3}", "queue", "close_notice"));
        Assert.Equal(string.Empty, MonitorWords.WordAt("{not json", null, "dismiss_action"));
        Assert.Equal("X", MonitorWords.WordAt("{\"queue\":{\"close_notice\":\"X\"}}", "queue", "close_notice"));
    }

    [Fact]
    public void AStoredDraftIsNamedByItsIdNotItsTypeName()
    {
        // A ListView item without a name is named by its ToString (W2 box, fix 3's sibling).
        var draft = new MissionDraftSummary { Id = "draft-1", SourceCount = 2, Status = "s" };
        Assert.Equal("draft-1", draft.ToString());
    }
}
