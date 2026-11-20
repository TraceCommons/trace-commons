using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Breakdown's drill-down, By project and By model, which every usage
/// card's "By project" opens (product owner, 2026-10-28). Ported from
/// macos/Tests/TraceCommonsAppTests/InsightsCreditTests.swift:28-40 and :111-152.
/// </summary>
public sealed class InsightsDetailsTests
{
    private static readonly Dictionary<string, string> Words = new()
    {
        ["analytics_unavailable"] = "—", ["analytics_unknown_label"] = "Unknown",
        ["analytics_folder_unknown"] = "Unknown folder", ["analytics_credit_tool_codex"] = "Codex",
        ["analytics_credit_tool_claude_code"] = "Claude", ["analytics_usage_near_ai"] = "NEAR AI",
        ["analytics_projects_traces"] = "{n} traces", ["analytics_projects_traces_one"] = "{n} trace",
        ["analytics_usage_tokens"] = "{t} tokens", ["analytics_cache_savings_value"] = "{x} saved",
    };

    private static string? Copy(string key) => Words.TryGetValue(key, out string? word) ? word : null;

    private const string Dash = "—";

    private static InsightsCounterWeek Week(string sessions, string? credit = null, string? work = null) =>
        InsightsCounterWeek.Parse(JsonDocument.Parse("""
            {"enabled": true, "feed": "counter_pass", "readable": true, "iso_week": "2026-W43", "week_start": "2026-10-19",
             "overview": {"feed": "counter_pass"},
             "rollup": {"week_start": "2026-10-19", "sessions": [
            """ + sessions + "]}"
            + (credit is null ? "" : ", \"near_ai_credit\": " + credit)
            + (work is null ? "" : ", \"work\": " + work) + "}").RootElement)!;

    private static string Session(string source, string? folder, long? tokens) =>
        "{\"source\": \"" + source + "\", \"folder\": " + (folder is null ? "null" : "\"" + folder + "\"")
        + ", \"tokens\": " + (tokens is { } t ? t.ToString(System.Globalization.CultureInfo.InvariantCulture) : "null")
        + ", \"state\": \"known\", \"reasons\": []}";

    private static string CreditWindow(int calls, string extra = "") =>
        "{\"from\": \"2026-10-19\", \"to\": \"2026-10-26\", \"usd_micros\": 13400, \"sessions\": 2, \"calls\": " + calls
        + ", \"calls_without_charge\": 0" + extra + "}";

    private static string Credit(string weekExtra = "", int calls = 2) =>
        "{\"basis\": \"near_ai_ledger\", \"week\": " + CreditWindow(calls, weekExtra) + ", \"today\": " + CreditWindow(0)
        + ", \"last_31_days\": " + CreditWindow(calls) + "}";

    private const string ByFolder = """
        , "by_folder": [
          {"key": "site", "usd_micros": 900, "sessions": 1, "calls": 1, "calls_without_charge": 0, "tokens": 4200},
          {"key": null, "usd_micros": 12500, "sessions": 1, "calls": 1, "calls_without_charge": 0}
        ]
        """;

    /// <summary>By project: one row per folder, Claude Code and Codex traces and tokens beside NEAR AI spend, the unknown folder last.</summary>
    [Fact]
    public void ByProjectJoinsTokensAndSpendPerFolder()
    {
        InsightsCounterWeek week = Week(
            Session("claude_code", "app", 37_235) + "," + Session("codex", null, 110) + "," + Session("claude_code", "app", null),
            Credit(ByFolder));
        IReadOnlyList<InsightsProjectRows.Row> table = InsightsProjectRows.Rows(week, Copy);
        Assert.Equal(new[] { "site", "app", "Unknown folder" }, table.Select(r => r.Label));
        InsightsProjectRows.Row app = table[1];
        Assert.Equal(new InsightsProjectRows.Usage(2, null), app.Claude);
        Assert.Null(app.Codex);
        Assert.Equal(Dash, InsightsProjectRows.Tokens(app.Claude, Copy));
        Assert.Equal("2 traces", InsightsProjectRows.Traces(app.Claude, Copy));
        Assert.Equal(Dash, InsightsProjectRows.Tokens(app.Codex, Copy));
        Assert.Null(InsightsProjectRows.Traces(app.Codex, Copy));
        Assert.Equal(Dash, app.NearAi);
        InsightsProjectRows.Row unknown = table[2];
        Assert.Equal(new InsightsProjectRows.Usage(1, 110), unknown.Codex);
        Assert.Equal("1 trace", InsightsProjectRows.Traces(unknown.Codex, Copy));
        Assert.Equal(InsightsAnalyticsWords.Figure(Copy, 110), InsightsProjectRows.Tokens(unknown.Codex, Copy));
        Assert.Equal("$0.01", unknown.NearAi);
        Assert.Equal("$0.0009", table[0].NearAi);
        Assert.Equal(InsightsAnalyticsWords.Figure(Copy, 4_200) + " tokens", table[0].NearAiTokens);
        Assert.Equal(Dash, unknown.NearAiTokens);
        Assert.Null(app.NearAiTokens);
        Assert.Null(app.Spend);
        InsightsProjectRows.Columns columns = InsightsProjectRows.ColumnsOf(week, table);
        Assert.Equal(new InsightsProjectRows.Columns(Spend: false, Claude: true, Codex: true, NearAi: true), columns);
    }

    /// <summary>Spend at API prices joins by folder and leads the order; ties go to NEAR AI, then tokens, then the name.</summary>
    [Fact]
    public void SpendLeadsTheOrderAndTheUnknownFolderIsLast()
    {
        const string work = """
            {"basis": "api_list",
             "week": {"key": null, "sessions": 3, "priced_sessions": 3, "unpriced_sessions": 0, "usd_micros_known": 3000000, "usd_partial": false, "cache_savings_usd_micros_known": 0, "edits_known": 0, "edits_undone_known": 0, "edits_denied_known": 0, "edits_partial": false},
             "by_tool": [], "by_model": [], "by_day": [],
             "by_folder": [
               {"key": "beta", "sessions": 1, "priced_sessions": 1, "unpriced_sessions": 0, "usd_micros_known": 2000000, "usd_partial": false, "cache_savings_usd_micros_known": 0, "edits_known": 0, "edits_undone_known": 0, "edits_denied_known": 0, "edits_partial": false},
               {"key": null, "sessions": 1, "priced_sessions": 1, "unpriced_sessions": 0, "usd_micros_known": 9000000, "usd_partial": false, "cache_savings_usd_micros_known": 0, "edits_known": 0, "edits_undone_known": 0, "edits_denied_known": 0, "edits_partial": false}
             ]}
            """;
        InsightsCounterWeek week = Week(
            Session("claude_code", "alpha", 500) + "," + Session("claude_code", "zeta", 500) + "," + Session("codex", "gamma", 900)
            + "," + Session("claude_code", "beta", 10) + "," + Session("claude_code", null, 10),
            work: work);
        IReadOnlyList<InsightsProjectRows.Row> rows = InsightsProjectRows.Rows(week, Copy);
        Assert.Equal(new[] { "beta", "gamma", "alpha", "zeta", "Unknown folder" }, rows.Select(r => r.Label));
        Assert.Equal("$2.00", rows[0].Spend);
        Assert.Equal(Dash, rows[1].Spend);
        Assert.Equal(new InsightsProjectRows.Columns(Spend: true, Claude: true, Codex: true, NearAi: false), InsightsProjectRows.ColumnsOf(week, rows));
    }

    /// <summary>A tool's column shows only while some folder has a trace of it; NEAR AI's only while it was called this week.</summary>
    [Fact]
    public void AToolWithNoTraceHasNoColumn()
    {
        InsightsCounterWeek claudeOnly = Week(Session("claude_code", "app", 10), Credit(calls: 0));
        IReadOnlyList<InsightsProjectRows.Row> rows = InsightsProjectRows.Rows(claudeOnly, Copy);
        Assert.Equal(new InsightsProjectRows.Columns(Spend: false, Claude: true, Codex: false, NearAi: false), InsightsProjectRows.ColumnsOf(claudeOnly, rows));
        Assert.False(InsightsProjectRows.HasNearAi(claudeOnly));
        Assert.Empty(InsightsProjectRows.Rows(Week(""), Copy));
    }

    /// <summary>By task and By tool are gone; By model shows only with something to show.</summary>
    [Fact]
    public void TheDrillDownIsByProjectAndByModelOnly()
    {
        Assert.Equal("Unknown folder", InsightsDetailsWords.Label(null, InsightsDetailsWords.Dimension.Folder, Copy));
        Assert.Equal("app", InsightsDetailsWords.Label("app", InsightsDetailsWords.Dimension.Folder, Copy));
        Assert.Equal("Unknown", InsightsDetailsWords.Label(null, InsightsDetailsWords.Dimension.Model, Copy));
        InsightsCounterWeek bare = Week(Session("claude_code", "app", 10));
        Assert.Equal(new[] { InsightsDetailsWords.Dimension.Folder }, InsightsDetailsWords.Dimensions(bare));
        InsightsCounterWeek withModels = Week(Session("claude_code", "app", 10), Credit("""
            , "by_model": [{"key": "glm-4.6", "usd_micros": 5000, "usd_micros_known": 5000, "sessions": 1, "calls": 2, "calls_without_charge": 0, "tokens": 1000}]
            """));
        Assert.Equal(new[] { InsightsDetailsWords.Dimension.Folder, InsightsDetailsWords.Dimension.Model }, InsightsDetailsWords.Dimensions(withModels));
        InsightsDetailsWords.ModelLine line = Assert.Single(InsightsDetailsWords.ModelLines(withModels, Copy));
        Assert.Equal(new InsightsDetailsWords.ModelLine("near_ai:glm-4.6", "glm-4.6", "NEAR AI", InsightsAnalyticsWords.Figure(Copy, 1_000) + " tokens", "$0.005"), line);
        Assert.Equal(InsightsDetailsWords.Dimension.Folder, InsightsDetailsWords.Shown(InsightsDetailsWords.Dimension.Model, bare));
        Assert.Equal(InsightsDetailsWords.Dimension.Model, InsightsDetailsWords.Shown(InsightsDetailsWords.Dimension.Model, withModels));
    }

    /// <summary>Claude's and Codex's models at API prices come first, then NEAR AI's at its charge; cache savings only when some.</summary>
    [Fact]
    public void ByModelListsPricedModelsThenNearAiAndCacheSavings()
    {
        const string work = """
            {"basis": "api_list",
             "week": {"key": null, "sessions": 1, "priced_sessions": 1, "unpriced_sessions": 0, "usd_micros_known": 4120000, "usd_partial": false, "cache_savings_usd_micros_known": 250000, "edits_known": 0, "edits_undone_known": 0, "edits_denied_known": 0, "edits_partial": false},
             "by_tool": [], "by_folder": [], "by_day": [],
             "by_model": [{"key": "claude-sonnet-4-5", "tool": "claude_code", "sessions": 1, "tokens": 37235, "usd_micros_known": 4120000, "usd_partial": false}]}
            """;
        InsightsCounterWeek week = Week(Session("claude_code", "app", 37_235), Credit("""
            , "by_model": [{"key": null, "usd_micros": null, "sessions": 1, "calls": 1, "calls_without_charge": 1}]
            """), work);
        IReadOnlyList<InsightsDetailsWords.ModelLine> lines = InsightsDetailsWords.ModelLines(week, Copy);
        Assert.Equal(new[] { "claude_code:claude-sonnet-4-5", "near_ai:\u0000unknown" }, lines.Select(l => l.Id));
        Assert.Equal(new InsightsDetailsWords.ModelLine("claude_code:claude-sonnet-4-5", "claude-sonnet-4-5", "Claude",
            InsightsAnalyticsWords.Figure(Copy, 37_235) + " tokens", "$4.12"), lines[0]);
        Assert.Equal(("Unknown", "NEAR AI", Dash, Dash), (lines[1].Label, lines[1].Tool, lines[1].Tokens, lines[1].Amount));
        Assert.Equal("$0.25 saved", InsightsDetailsWords.CacheSavings(week, Copy));
        Assert.Null(InsightsDetailsWords.CacheSavings(Week(""), Copy));
    }

    /// <summary>A credit group that is there but malformed refuses the whole week, as the Mac's decode does.</summary>
    [Fact]
    public void AMalformedGroupIsNoWeek()
    {
        string sessions = Session("claude_code", "app", 10);
        Assert.NotNull(Week(sessions, Credit(ByFolder)).NearAiCredit!.Week.ByFolder);
        foreach (string broken in new[]
        {
            """, "by_folder": {}""",
            """, "by_folder": [{"key": "a", "usd_micros": 1, "calls": 1, "calls_without_charge": 0}]""",
            """, "by_folder": [{"key": 4, "usd_micros": 1, "sessions": 1, "calls": 1, "calls_without_charge": 0}]""",
            """, "by_model": [{"key": "a", "usd_micros": "1", "sessions": 1, "calls": 1, "calls_without_charge": 0}]""",
            """, "by_model": [{"key": "a", "usd_micros": 1, "sessions": 1, "calls": 1, "calls_without_charge": 0, "tokens_partial": 1}]""",
        })
        {
            Assert.Null(InsightsCounterWeek.Parse(JsonDocument.Parse("""
                {"enabled": true, "feed": "counter_pass", "readable": true, "iso_week": "2026-W43", "week_start": "2026-10-19",
                 "overview": {"feed": "counter_pass"}, "rollup": {"week_start": "2026-10-19", "sessions": []},
                 "near_ai_credit":
                """ + Credit(broken) + "}").RootElement));
        }

        InsightsCreditGroup group = Week(sessions, Credit("""
            , "by_folder": [{"key": "a", "usd_micros": null, "usd_micros_known": 7, "usd_partial": true, "sessions": 1, "calls": 2, "calls_without_charge": 1, "tokens": null, "tokens_known": 3, "tokens_partial": true}]
            """)).NearAiCredit!.Week.ByFolder!.Single();
        Assert.Equal(new InsightsCreditGroup("a", null, 1, 2, 1, null, 7, true, 3, true), group);
    }
}
