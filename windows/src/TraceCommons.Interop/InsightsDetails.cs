using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;

namespace TraceCommons.Interop;

// Breakdown's drill-down (product owner, 2026-10-28; InsightsCreditViews.swift):
// the week on screen by project folder, Claude Code and Codex usage (traces
// and tokens, from the counter pass's rows) beside NEAR AI spend (the NEAR AI
// ledger's own charge, never a list price), and Claude, Codex and NEAR AI
// spend by model. By task and By tool were removed (owner, 2026-10-29).
// Folder names come from this computer's own folder list and stay on it. An
// unknown figure is the core's dash, never zero. Feed T only.

/// <summary>By project: one row per folder (<c>InsightsProjectRows</c>, InsightsCreditViews.swift:109-227).</summary>
public static class InsightsProjectRows
{
    /// <summary>A tool's traces counted this week and their tokens; null tokens when any trace's count is unknown.</summary>
    public sealed record Usage(int Traces, long? Tokens);

    /// <summary>
    /// One folder. <see cref="NearAi"/> is the dash when there was no call or
    /// a call had no charge; <see cref="NearAiTokens"/> is null with no NEAR AI
    /// call there; <see cref="Spend"/> (Claude and Codex at API prices) is null
    /// from a daemon that predates <c>work</c>.
    /// </summary>
    public sealed record Row(string Id, string Label, Usage? Claude, Usage? Codex, string NearAi, string? NearAiTokens, long? NearAiMicros,
        string? Spend, long SpendMicros);

    /// <summary>The columns drawn: a tool's only while some folder has a trace of it, NEAR AI's only while it was called, spend only while known.</summary>
    public sealed record Columns(bool Spend, bool Claude, bool Codex, bool NearAi);

    private const string UnknownId = "\u0000unknown";

    /// <summary>
    /// Every folder with a trace counted this week or spend this week.
    /// Largest spend first (API prices and NEAR AI together), then NEAR AI,
    /// then most tokens, then the name; the unknown folder last.
    /// </summary>
    public static IReadOnlyList<Row> Rows(InsightsCounterWeek week, Func<string, string?> copy)
    {
        var usage = new Dictionary<string, (Usage? Claude, Usage? Codex)>(StringComparer.Ordinal);
        static Usage Add(Usage? current, long? tokens)
        {
            long? known = current is null ? 0 : current.Tokens;
            return new Usage((current?.Traces ?? 0) + 1, known is { } sum && tokens is { } t ? sum + t : null);
        }

        foreach (InsightsCounterSession session in week.Sessions ?? Array.Empty<InsightsCounterSession>())
        {
            string folder = session.Folder ?? UnknownId;
            usage.TryGetValue(folder, out (Usage? Claude, Usage? Codex) entry);
            long? tokens = session.Tokens is { } t ? (long)Math.Min(t, long.MaxValue) : null;
            switch (session.Source)
            {
                case "claude_code":
                    entry.Claude = Add(entry.Claude, tokens);
                    break;
                case "codex":
                    entry.Codex = Add(entry.Codex, tokens);
                    break;
                default:
                    continue;
            }

            usage[folder] = entry;
        }

        // The first group under a key wins, as the Mac's uniquingKeysWith does.
        var spend = new Dictionary<string, InsightsCreditGroup>(StringComparer.Ordinal);
        foreach (InsightsCreditGroup group in week.NearAiCredit?.Week.ByFolder ?? Array.Empty<InsightsCreditGroup>())
        {
            spend.TryAdd(group.Key ?? UnknownId, group);
        }

        var work = new Dictionary<string, InsightsWorkTotals>(StringComparer.Ordinal);
        foreach (InsightsWorkTotals totals in week.Work?.ByFolder ?? Array.Empty<InsightsWorkTotals>())
        {
            work.TryAdd(totals.Key ?? UnknownId, totals);
        }

        string unknown = InsightsAnalyticsWords.Dash(copy);
        IEnumerable<Row> rows = usage.Keys.Union(spend.Keys).Union(work.Keys).Select(folder =>
        {
            spend.TryGetValue(folder, out InsightsCreditGroup? group);
            work.TryGetValue(folder, out InsightsWorkTotals? totals);
            usage.TryGetValue(folder, out (Usage? Claude, Usage? Codex) tools);
            string? key = folder == UnknownId ? null : folder;
            return new Row(folder, InsightsDetailsWords.Label(key, InsightsDetailsWords.Dimension.Folder, copy), tools.Claude, tools.Codex,
                group is null ? unknown : InsightsCreditWords.Spend(group.UsdMicros, group.UsdMicrosKnown, copy),
                group is null ? null : InsightsCreditWords.Tokens(group.Tokens ?? group.TokensKnown, copy),
                group?.UsdMicros ?? group?.UsdMicrosKnown,
                week.Work is null ? null : InsightsWorkWords.Spend(totals, copy),
                totals?.UsdMicrosKnown ?? 0);
        });

        static long Tokens(Row row) => (row.Claude?.Tokens ?? 0) + (row.Codex?.Tokens ?? 0);
        return rows
            .OrderBy(r => r.Id == UnknownId)
            .ThenByDescending(r => r.SpendMicros + (r.NearAiMicros ?? 0))
            .ThenByDescending(r => r.NearAiMicros ?? -1)
            .ThenByDescending(Tokens)
            .ThenBy(r => r.Label, StringComparer.Ordinal)
            .ToList();
    }

    /// <summary>Whether NEAR AI was called from this computer in the week on screen.</summary>
    public static bool HasNearAi(InsightsCounterWeek week) => week.NearAiCredit is { } credit && credit.Week.Calls != 0;

    public static Columns ColumnsOf(InsightsCounterWeek week, IReadOnlyList<Row> rows) => new(
        rows.Any(r => r.Spend is not null) && (week.Work?.ByFolder ?? Array.Empty<InsightsWorkTotals>()).Any(InsightsWorkWords.KnowsSpend),
        rows.Any(r => r.Claude is not null),
        rows.Any(r => r.Codex is not null),
        HasNearAi(week));

    /// <summary>A tool's cell: its tokens, or the dash.</summary>
    public static string Tokens(Usage? usage, Func<string, string?> copy) =>
        usage is null
            ? InsightsAnalyticsWords.Dash(copy)
            : InsightsAnalyticsWords.Figure(copy, usage.Tokens is { } t ? (ulong)Math.Max(0, t) : null);

    /// <summary>"1 trace" or "{n} traces" under a tool's tokens; null without traces.</summary>
    public static string? Traces(Usage? usage, Func<string, string?> copy) =>
        usage is null
            ? null
            : InsightsAnalyticsWords.Fill(copy(usage.Traces == 1 ? "analytics_projects_traces_one" : "analytics_projects_traces") ?? string.Empty,
                ("n", usage.Traces.ToString(CultureInfo.InvariantCulture)));
}

/// <summary>The drill-down's views and By model (<c>InsightsCreditWords</c>, <c>InsightsDetailsTable</c>).</summary>
public static class InsightsDetailsWords
{
    public enum Dimension
    {
        Folder,
        Model,
    }

    /// <summary>One By model row: the label, the tool, its tokens and its amount.</summary>
    public sealed record ModelLine(string Id, string Label, string Tool, string Tokens, string Amount);

    /// <summary>A group's name: the folder's own label or the model's; the unknown group in each list's word.</summary>
    public static string Label(string? key, Dimension dimension, Func<string, string?> copy) =>
        key ?? copy(dimension == Dimension.Folder ? "analytics_folder_unknown" : "analytics_unknown_label") ?? string.Empty;

    /// <summary>By model has something to show: spend at API prices, or NEAR AI spend by model.</summary>
    public static bool ShowsModels(InsightsCounterWeek week) =>
        week.Work is not null || (week.NearAiCredit?.Week.ByModel?.Count ?? 0) > 0;

    /// <summary>By project always; By model when it has something to show.</summary>
    public static IReadOnlyList<Dimension> Dimensions(InsightsCounterWeek week) =>
        ShowsModels(week) ? new[] { Dimension.Folder, Dimension.Model } : new[] { Dimension.Folder };

    /// <summary>The view drawn: the one chosen, or By project when this week has nothing for it.</summary>
    public static Dimension Shown(Dimension chosen, InsightsCounterWeek week) =>
        Dimensions(week).Contains(chosen) ? chosen : Dimension.Folder;

    /// <summary>Claude's and Codex's models at API prices (<c>work</c>), then NEAR AI's at the ledger's charge.</summary>
    public static IReadOnlyList<ModelLine> ModelLines(InsightsCounterWeek week, Func<string, string?> copy)
    {
        IEnumerable<ModelLine> priced = (week.Work?.ByModel ?? Array.Empty<InsightsWorkModel>()).Select(model =>
            new ModelLine(model.Tool + ":" + model.Key, model.Key, InsightsWorkWords.ToolName(model.Tool, copy),
                InsightsCreditWords.Tokens(model.Tokens, copy), InsightsWorkWords.Dollars(model.UsdMicrosKnown)));
        IEnumerable<ModelLine> charged = (week.NearAiCredit?.Week.ByModel ?? Array.Empty<InsightsCreditGroup>()).Select(group =>
            new ModelLine("near_ai:" + (group.Key ?? "\u0000unknown"), Label(group.Key, Dimension.Model, copy),
                copy("analytics_usage_near_ai") ?? string.Empty,
                InsightsCreditWords.Tokens(group.Tokens ?? group.TokensKnown, copy),
                InsightsCreditWords.Spend(group.UsdMicros, group.UsdMicrosKnown, copy)));
        return priced.Concat(charged).ToList();
    }

    /// <summary>"{x} saved" under By model while the cache saved something at API prices; null otherwise.</summary>
    public static string? CacheSavings(InsightsCounterWeek week, Func<string, string?> copy) =>
        week.Work is { } work && work.Week.CacheSavingsUsdMicrosKnown > 0
            ? InsightsAnalyticsWords.Fill(copy("analytics_cache_savings_value") ?? string.Empty,
                ("x", InsightsCreditWords.Amount(work.Week.CacheSavingsUsdMicrosKnown, copy)))
            : null;
}
