using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

// The parts of insights_week beyond the week itself (feed T only): spend at
// API list prices, goals set with /goal, NEAR AI's own charge, tokens per
// day, and the weekly figures the core compares (DaemonDataModels.swift:
// 2258-2470, InsightsWorkModels.swift:10-125). An absent figure is unknown,
// never zero.

/// <summary>One scope's totals: the week, a tool, or a folder (<c>InsightsWorkTotals</c>).</summary>
public sealed record InsightsWorkTotals(
    string? Key, int Sessions, int PricedSessions, int UnpricedSessions, int? NearAiSessions,
    long UsdMicrosKnown, bool UsdPartial, long CacheSavingsUsdMicrosKnown,
    long EditsKnown, long EditsUndoneKnown, long EditsDeniedKnown, bool EditsPartial)
{
    /// <summary>Edits neither undone nor denied, as far as known.</summary>
    public long EditsKeptKnown => Math.Max(0, EditsKnown - EditsUndoneKnown - EditsDeniedKnown);
}

/// <summary>One model's week at list prices (<c>InsightsWorkModel</c>).</summary>
public sealed record InsightsWorkModel(string Key, string Tool, int Sessions, long Tokens, long UsdMicrosKnown, bool UsdPartial);

/// <summary>One local day's spend at list prices (<c>InsightsWorkDay</c>).</summary>
public sealed record InsightsWorkDay(string Date, long UsdMicrosKnown, bool UsdPartial);

/// <summary><c>insights_week</c>'s <c>work</c> (<c>InsightsWork</c>).</summary>
public sealed record InsightsWork(
    string Basis, InsightsWorkTotals Week, InsightsWorkTotals? LastWeek, IReadOnlyList<InsightsWorkTotals> ByTool,
    IReadOnlyList<InsightsWorkModel> ByModel, IReadOnlyList<InsightsWorkTotals> ByFolder,
    IReadOnlyList<InsightsWorkDay> ByDay, IReadOnlyList<InsightsWorkDay>? LastWeekByDay);

/// <summary>One <c>/goal</c> goal: its tool, the tool's own status and what it took. Never its text.</summary>
public sealed record InsightsWeekGoal(
    string Tool, string Status, bool Met, long? Tokens, long? TimeSecs, long? Iterations, string? Folder,
    string SessionRef, string UpdatedAt);

/// <summary><c>insights_week</c>'s <c>goals</c>, newest update first.</summary>
public sealed record InsightsWeekGoals(IReadOnlyList<InsightsWeekGoal> Goals, int Met, int Total);

/// <summary>One local day's tokens per tool; null is unknown, a day without the tool a true 0.</summary>
public sealed record InsightsUsageDay(string Date, long? ClaudeCode, long? Codex);

/// <summary>One window's NEAR AI charge (<c>InsightsCreditWindow</c>).</summary>
public sealed record InsightsCreditWindow(
    string? Date, long? UsdMicros, int Sessions, int Calls, int CallsWithoutCharge, long? Tokens,
    long? UsdMicrosKnown, bool? UsdPartial, long? TokensKnown, bool? TokensPartial)
{
    public IReadOnlyList<InsightsCreditGroup>? ByFolder { get; init; }

    public IReadOnlyList<InsightsCreditGroup>? ByModel { get; init; }
}

public sealed record InsightsCreditGroup(
    string? Key, long? UsdMicros, int Sessions, int Calls, int CallsWithoutCharge, long? Tokens,
    long? UsdMicrosKnown, bool? UsdPartial, long? TokensKnown, bool? TokensPartial);

/// <summary><c>insights_week</c>'s <c>near_ai_credit</c>: the NEAR AI ledger's own charge, never a list price.</summary>
public sealed record InsightsNearAiCredit(
    InsightsCreditWindow Week, InsightsCreditWindow Today, IReadOnlyList<InsightsCreditWindow>? ByDay,
    InsightsCreditWindow Last31Days);

/// <summary>One source's change on last week; null with a reason when either week cannot be compared.</summary>
public sealed record InsightsWeekChange(string Source, long? Permille, string? Unavailable);

/// <summary>Strict readers for the parts: a part that is there but malformed refuses the reply.</summary>
internal static class InsightsWeekParts
{
    internal sealed class Malformed : Exception
    {
    }

    internal static T Fail<T>() => throw new Malformed();

    /// <summary>The part, or null when it is absent or JSON null.</summary>
    internal static T? Part<T>(JsonElement item, string name, Func<JsonElement, T> read) where T : class =>
        item.TryGetProperty(name, out JsonElement value) && value.ValueKind != JsonValueKind.Null ? read(value) : null;

    internal static JsonElement Obj(JsonElement value) => value.ValueKind == JsonValueKind.Object ? value : Fail<JsonElement>();

    internal static IReadOnlyList<T> List<T>(JsonElement value, Func<JsonElement, T> read) =>
        value.ValueKind == JsonValueKind.Array ? value.EnumerateArray().Select(read).ToList() : Fail<IReadOnlyList<T>>();

    internal static IReadOnlyList<T> ListOf<T>(JsonElement item, string name, Func<JsonElement, T> read) =>
        item.TryGetProperty(name, out JsonElement value) ? List(value, read) : Fail<IReadOnlyList<T>>();

    internal static string Str(JsonElement item, string name) =>
        item.TryGetProperty(name, out JsonElement value) && value.ValueKind == JsonValueKind.String ? value.GetString()! : Fail<string>();

    internal static string? OptStr(JsonElement item, string name)
    {
        if (!item.TryGetProperty(name, out JsonElement value) || value.ValueKind == JsonValueKind.Null) return null;
        return value.ValueKind == JsonValueKind.String ? value.GetString() : Fail<string>();
    }

    internal static long Long(JsonElement item, string name) =>
        item.TryGetProperty(name, out JsonElement value) && value.ValueKind == JsonValueKind.Number && value.TryGetInt64(out long n)
            ? n
            : Fail<long>();

    internal static long? OptLong(JsonElement item, string name)
    {
        if (!item.TryGetProperty(name, out JsonElement value) || value.ValueKind == JsonValueKind.Null) return null;
        return value.ValueKind == JsonValueKind.Number && value.TryGetInt64(out long n) ? n : Fail<long>();
    }

    internal static int Int(JsonElement item, string name) => checked((int)Long(item, name));

    internal static int? OptInt(JsonElement item, string name) => OptLong(item, name) is { } n ? checked((int)n) : null;

    internal static bool Bool(JsonElement item, string name) =>
        item.TryGetProperty(name, out JsonElement value) && value.ValueKind is JsonValueKind.True or JsonValueKind.False
            ? value.GetBoolean()
            : Fail<bool>();

    internal static bool? OptBool(JsonElement item, string name)
    {
        if (!item.TryGetProperty(name, out JsonElement value) || value.ValueKind == JsonValueKind.Null) return null;
        return value.ValueKind is JsonValueKind.True or JsonValueKind.False ? value.GetBoolean() : Fail<bool>();
    }

    internal static InsightsWorkTotals Totals(JsonElement value)
    {
        JsonElement t = Obj(value);
        return new InsightsWorkTotals(OptStr(t, "key"), Int(t, "sessions"), Int(t, "priced_sessions"), Int(t, "unpriced_sessions"),
            OptInt(t, "near_ai_sessions"), Long(t, "usd_micros_known"), Bool(t, "usd_partial"),
            Long(t, "cache_savings_usd_micros_known"), Long(t, "edits_known"), Long(t, "edits_undone_known"),
            Long(t, "edits_denied_known"), Bool(t, "edits_partial"));
    }

    internal static InsightsWorkDay Day(JsonElement value)
    {
        JsonElement d = Obj(value);
        return new InsightsWorkDay(Str(d, "date"), Long(d, "usd_micros_known"), Bool(d, "usd_partial"));
    }

    internal static InsightsWork Work(JsonElement value)
    {
        JsonElement w = Obj(value);
        return new InsightsWork(Str(w, "basis"), w.TryGetProperty("week", out JsonElement week) ? Totals(week) : Fail<InsightsWorkTotals>(),
            Part(w, "last_week", Totals), ListOf(w, "by_tool", Totals),
            ListOf(w, "by_model", m =>
            {
                JsonElement o = Obj(m);
                return new InsightsWorkModel(Str(o, "key"), Str(o, "tool"), Int(o, "sessions"), Long(o, "tokens"),
                    Long(o, "usd_micros_known"), Bool(o, "usd_partial"));
            }),
            ListOf(w, "by_folder", Totals), ListOf(w, "by_day", Day),
            Part(w, "last_week_by_day", days => List(days, Day)));
    }

    internal static InsightsWeekGoals Goals(JsonElement value)
    {
        JsonElement g = Obj(value);
        return new InsightsWeekGoals(ListOf(g, "goals", item =>
        {
            JsonElement o = Obj(item);
            return new InsightsWeekGoal(Str(o, "tool"), Str(o, "status"), Bool(o, "met"), OptLong(o, "tokens"),
                OptLong(o, "time_secs"), OptLong(o, "iterations"), OptStr(o, "folder"), Str(o, "session_ref"), Str(o, "updated_at"));
        }), Int(g, "met"), Int(g, "total"));
    }

    internal static InsightsCreditWindow Window(JsonElement value)
    {
        JsonElement w = Obj(value);
        Str(w, "from");
        Str(w, "to");
        return new InsightsCreditWindow(OptStr(w, "date"), OptLong(w, "usd_micros"), Int(w, "sessions"), Int(w, "calls"),
            Int(w, "calls_without_charge"), OptLong(w, "tokens"), OptLong(w, "usd_micros_known"), OptBool(w, "usd_partial"),
            OptLong(w, "tokens_known"), OptBool(w, "tokens_partial"))
        {
            ByFolder = Part(w, "by_folder", groups => List(groups, Group)),
            ByModel = Part(w, "by_model", groups => List(groups, Group)),
        };
    }

    /// <summary>One group of a window's charge (<c>InsightsCreditGroup</c>, DaemonDataModels.swift:2481-2513).</summary>
    internal static InsightsCreditGroup Group(JsonElement value)
    {
        JsonElement g = Obj(value);
        return new InsightsCreditGroup(OptStr(g, "key"), OptLong(g, "usd_micros"), Int(g, "sessions"), Int(g, "calls"),
            Int(g, "calls_without_charge"), OptLong(g, "tokens"), OptLong(g, "usd_micros_known"), OptBool(g, "usd_partial"),
            OptLong(g, "tokens_known"), OptBool(g, "tokens_partial"));
    }

    internal static InsightsNearAiCredit Credit(JsonElement value)
    {
        JsonElement c = Obj(value);
        Str(c, "basis");
        InsightsCreditWindow Required(string name) =>
            c.TryGetProperty(name, out JsonElement w) ? Window(w) : Fail<InsightsCreditWindow>();
        return new InsightsNearAiCredit(Required("week"), Required("today"),
            Part(c, "by_day", days => List(days, Window)), Required("last_31_days"));
    }

    internal static IReadOnlyList<InsightsUsageDay> UsageDays(JsonElement value) => List(value, item =>
    {
        JsonElement d = Obj(item);
        return new InsightsUsageDay(Str(d, "date"), OptLong(d, "claude_code"), OptLong(d, "codex"));
    });

    internal static IReadOnlyList<InsightsWeekChange> Changes(JsonElement value) => List(value, item =>
    {
        JsonElement c = Obj(item);
        return new InsightsWeekChange(Str(c, "source"), OptLong(c, "permille"), OptStr(c, "unavailable"));
    });
}

/// <summary>
/// Overview's week at a glance (product owner, 2026-10-29): spend at API
/// prices, the top model, spend by model and trends, from <c>work</c>
/// (<c>InsightsWorkWords</c>, InsightsWorkViews.swift:10-176). Every figure is
/// the sum as far as known; the dash when nothing is.
/// </summary>
public static class InsightsWorkWords
{
    public sealed record ModelRow(string Id, string Label, string Tool, string Amount, double Share, string Percent);

    public sealed record ChangeLine(string Id, string Label, string Words, bool Up);

    public sealed record DayPair(string Id, string Label, double Week, double? LastWeek);

    /// <summary>A tile: its label, value and the note under it (<c>InsightsWeekTiles</c>).</summary>
    public sealed record Tile(string Label, string Value, string Note);

    private static string Text(Func<string, string?> copy, string key) => copy(key) ?? string.Empty;

    /// <summary>Whether any of the spend is known: a trace priced in full, or a known part.</summary>
    public static bool KnowsSpend(InsightsWorkTotals? totals) =>
        totals is not null && (totals.PricedSessions + (totals.NearAiSessions ?? 0) > 0 || totals.UsdMicrosKnown > 0);

    /// <summary>Spend at list prices: the known sum to the cent; the dash when nothing is known.</summary>
    public static string Spend(InsightsWorkTotals? totals, Func<string, string?> copy) =>
        KnowsSpend(totals) ? Dollars(totals!.UsdMicrosKnown) : InsightsAnalyticsWords.Dash(copy);

    /// <summary>Two decimals from a cent up; under a cent a significant digit, never $0.00.</summary>
    public static string Dollars(long micros) => InsightsBenchmarkWords.Dollars(micros);

    /// <summary>"▲ 18% vs last week"; null without last week's figure, with no change, or when either is partial.</summary>
    public static (string Words, bool Up)? Change(long now, long? last, bool partial, Func<string, string?> copy)
    {
        if (partial || last is not { } before || before <= 0) return null;
        int percent = (int)Math.Round(Math.Abs(now - before) / (double)before * 100, MidpointRounding.AwayFromZero);
        if (percent <= 0) return null;
        bool up = now > before;
        return (InsightsAnalyticsWords.Fill(Text(copy, up ? "analytics_change_up" : "analytics_change_down"),
            ("p", percent.ToString(CultureInfo.InvariantCulture))), up);
    }

    /// <summary>Whether this week's or last week's spend is only partly known.</summary>
    public static bool SpendPartial(InsightsWork work) =>
        work.Week.UsdPartial || work.Week.PricedSessions == 0 || work.LastWeek?.UsdPartial == true;

    public static string ToolName(string tool, Func<string, string?> copy) => Text(copy, "analytics_credit_tool_" + tool);

    /// <summary>One row per model with known spend, in the daemon's order (largest first).</summary>
    public static IReadOnlyList<ModelRow> ModelRows(InsightsWork work, Func<string, string?> copy)
    {
        long total = work.ByModel.Sum(m => m.UsdMicrosKnown);
        return work.ByModel.Where(m => m.UsdMicrosKnown > 0).Select(m =>
        {
            double share = total > 0 ? m.UsdMicrosKnown / (double)total : 0;
            return new ModelRow(m.Tool + ":" + m.Key, m.Key, ToolName(m.Tool, copy), Dollars(m.UsdMicrosKnown), share,
                ((int)Math.Round(share * 100, MidpointRounding.AwayFromZero)).ToString(CultureInfo.InvariantCulture) + "%");
        }).ToList();
    }

    /// <summary>The changes that matter: spend, then each tool's tokens.</summary>
    public static IReadOnlyList<ChangeLine> Changes(InsightsWork work, IReadOnlyList<InsightsWeekChange>? tokens, Func<string, string?> copy)
    {
        var changes = new List<ChangeLine>();
        if (Change(work.Week.UsdMicrosKnown, work.LastWeek?.UsdMicrosKnown, SpendPartial(work), copy) is { } spend)
        {
            changes.Add(new ChangeLine("spend", Text(copy, "analytics_tile_spend"), spend.Words, spend.Up));
        }

        foreach (InsightsWeekChange change in tokens ?? Array.Empty<InsightsWeekChange>())
        {
            if (change.Permille is not { } permille || permille == 0) continue;
            int percent = (int)Math.Round(Math.Abs(permille) / 10.0, MidpointRounding.AwayFromZero);
            if (percent <= 0) continue;
            bool up = permille > 0;
            changes.Add(new ChangeLine("tokens:" + change.Source,
                ToolName(change.Source, copy) + " · " + Text(copy, "analytics_card_tokens"),
                InsightsAnalyticsWords.Fill(Text(copy, up ? "analytics_change_up" : "analytics_change_down"),
                    ("p", percent.ToString(CultureInfo.InvariantCulture))),
                up));
        }

        return changes;
    }

    /// <summary>"Claude and Codex at API prices", or only the tool used this week.</summary>
    public static string ApiPrices(InsightsWork work, Func<string, string?> copy)
    {
        var tools = new HashSet<string>(work.ByModel.Select(m => m.Tool), StringComparer.Ordinal);
        foreach (InsightsWorkTotals tool in ActiveTools(work))
        {
            if (tool.Key is { } key) tools.Add(key);
        }

        if (tools.SetEquals(new[] { "claude_code" })) return Text(copy, "analytics_tile_api_prices_claude");
        if (tools.SetEquals(new[] { "codex" })) return Text(copy, "analytics_tile_api_prices_codex");
        return Text(copy, "analytics_tile_api_prices");
    }

    /// <summary>The tools with a trace this week.</summary>
    public static IEnumerable<InsightsWorkTotals> ActiveTools(InsightsWork work) => work.ByTool.Where(t => t.Sessions > 0);

    /// <summary>Trends has something to draw: a day with known spend, this week or the week before, or a change.</summary>
    public static bool ShowsTrends(InsightsWork work, IReadOnlyList<InsightsWeekChange>? tokens, Func<string, string?> copy) =>
        work.ByDay.Any(d => d.UsdMicrosKnown > 0)
        || (work.LastWeekByDay ?? Array.Empty<InsightsWorkDay>()).Any(d => d.UsdMicrosKnown > 0)
        || Changes(work, tokens, copy).Count > 0;

    /// <summary>This week's days beside the week before's, in dollars.</summary>
    public static IReadOnlyList<DayPair> Days(InsightsWork work) => work.ByDay.Select((day, index) =>
    {
        string label = DateOnly.TryParseExact(day.Date, "yyyy-MM-dd", CultureInfo.InvariantCulture, DateTimeStyles.None, out DateOnly date)
            ? date.ToString("ddd", CultureInfo.GetCultureInfo("en-US"))
            : (index + 1).ToString(CultureInfo.InvariantCulture);
        InsightsWorkDay? last = work.LastWeekByDay is { } before && index < before.Count ? before[index] : null;
        return new DayPair(day.Date, label, day.UsdMicrosKnown / 1_000_000.0, last is null ? null : last.UsdMicrosKnown / 1_000_000.0);
    }).ToList();

    /// <summary>
    /// The row with Home's credit tile after the week's own, under either
    /// feed (InsightsWorkViews.swift:188-190); null leaves it out.
    /// </summary>
    public static IReadOnlyList<Tile> Tiles(InsightsWork? work, (string Value, string Detail)? credit, Func<string, string?> copy)
    {
        var tiles = new List<Tile>(Tiles(work, copy));
        if (credit is { } tile) tiles.Add(new Tile(Text(copy, "analytics_tile_credits"), tile.Value, tile.Detail));
        return tiles;
    }

    /// <summary>
    /// The tiles that lead Overview from <c>work</c>, each left out when
    /// nothing is known (<c>InsightsWeekTiles</c>, InsightsWorkViews.swift:178-203).
    /// Home's credit tile joins them in the overload above.
    /// </summary>
    public static IReadOnlyList<Tile> Tiles(InsightsWork? work, Func<string, string?> copy)
    {
        var tiles = new List<Tile>();
        if (work is null) return tiles;
        if (KnowsSpend(work.Week))
        {
            string note = Change(work.Week.UsdMicrosKnown, work.LastWeek?.UsdMicrosKnown, SpendPartial(work), copy)?.Words
                ?? ApiPrices(work, copy);
            tiles.Add(new Tile(Text(copy, "analytics_tile_spend"), Spend(work.Week, copy), note));
        }

        if (ModelRows(work, copy).FirstOrDefault() is { } top)
        {
            tiles.Add(new Tile(Text(copy, "analytics_tile_top_model"), top.Label,
                InsightsAnalyticsWords.Fill(Text(copy, "analytics_tile_share_of_spend"), ("p", top.Percent.Replace("%", "", StringComparison.Ordinal)))
                + " · " + top.Tool));
        }

        return tiles;
    }
}

/// <summary>NEAR AI's own charge in words (<c>InsightsCreditWords</c>, InsightsCreditViews.swift:3-55).</summary>
public static class InsightsCreditWords
{
    /// <summary>Dollars to the cent; under a cent a significant digit; the dash when unknown.</summary>
    public static string Amount(long? micros, Func<string, string?> copy) =>
        micros is { } m && m >= 0 ? InsightsWorkWords.Dollars(m) : InsightsAnalyticsWords.Dash(copy);

    /// <summary>"1,200,000 tokens"; the dash when unknown.</summary>
    public static string Tokens(long? tokens, Func<string, string?> copy) =>
        tokens is { } t && t >= 0
            ? InsightsAnalyticsWords.Fill(copy("analytics_usage_tokens") ?? string.Empty, ("t", InsightsAnalyticsWords.Figure(copy, (ulong)t)))
            : InsightsAnalyticsWords.Dash(copy);

    /// <summary>The exact figure, else the known part as it is; the dash only when nothing is known.</summary>
    public static string Spend(long? exact, long? known, Func<string, string?> copy) => Amount(exact ?? known, copy);

    /// <summary>"$4.12 · 1,200,000 tokens".</summary>
    public static string SpendAndTokens(InsightsCreditWindow window, Func<string, string?> copy) =>
        Spend(window.UsdMicros, window.UsdMicrosKnown, copy) + " · " + Tokens(window.Tokens ?? window.TokensKnown, copy);
}

/// <summary>The NEAR AI usage card's figures (<c>InsightsUsageWords</c>, InsightsUsageCards.swift:296-345).</summary>
public static class InsightsUsageWords
{
    /// <summary>One bar: the date, its weekday, the figure read out, and its height (null draws none).</summary>
    public sealed record Day(string Id, string Label, string Amount, double? Value);

    /// <summary>Only while NEAR AI calls from this machine were made in the last 31 days.</summary>
    public static bool ShowsNearAi(InsightsNearAiCredit? credit) => credit is not null && credit.Last31Days.Calls > 0;

    /// <summary>NEAR AI spend on each of the last 7 days; empty from a daemon that predates them.</summary>
    public static IReadOnlyList<Day> Days(InsightsNearAiCredit credit, Func<string, string?> copy) =>
        (credit.ByDay ?? Array.Empty<InsightsCreditWindow>()).Select((window, index) =>
        {
            long? micros = window.UsdMicros ?? (window.UsdPartial == true ? window.UsdMicrosKnown : null);
            return new Day(window.Date ?? index.ToString(CultureInfo.InvariantCulture), Weekday(window.Date, index),
                InsightsCreditWords.Spend(window.UsdMicros, window.UsdMicrosKnown, copy),
                micros is { } m ? m / 1_000_000.0 : null);
        }).ToList();

    /// <summary>
    /// A tool's tokens on each of the last 7 days (<c>claude_code</c> or
    /// <c>codex</c>, from <c>usage_by_day</c>; InsightsUsageCards.swift:331-344);
    /// empty from a daemon that predates them. An unknown day draws no bar.
    /// </summary>
    public static IReadOnlyList<Day> TokenBars(IReadOnlyList<InsightsUsageDay>? days, string tool, Func<string, string?> copy) =>
        (days ?? Array.Empty<InsightsUsageDay>()).Select((day, index) =>
        {
            long? tokens = tool == "codex" ? day.Codex : day.ClaudeCode;
            return new Day(day.Date, Weekday(day.Date, index),
                InsightsAnalyticsWords.Figure(copy, tokens is { } t ? (ulong)Math.Max(0, t) : null),
                tokens is { } v ? v : null);
        }).ToList();

    /// <summary>The weekday's one-letter name, or the day's position when the date does not read.</summary>
    private static string Weekday(string? date, int index) =>
        date is not null
        && DateOnly.TryParseExact(date, "yyyy-MM-dd", CultureInfo.InvariantCulture, DateTimeStyles.None, out DateOnly day)
            ? day.ToString("ddd", CultureInfo.GetCultureInfo("en-US"))[..1]
            : (index + 1).ToString(CultureInfo.InvariantCulture);
}

/// <summary>
/// Goals set with <c>/goal</c> in Claude and Codex, in the core's words
/// (<c>InsightsGoalsWords</c>, InsightsModel.swift:802-850). Status, tokens,
/// time and folder only; a goal's text never reaches the app.
/// </summary>
public static class InsightsGoalsWords
{
    /// <summary>Each status either tool sends, to its shared word; anything else is "Status unavailable".</summary>
    public static readonly IReadOnlyDictionary<string, string> StatusWords = new Dictionary<string, string>(StringComparer.Ordinal)
    {
        ["met"] = "analytics_goal_met", ["complete"] = "analytics_goal_met",
        ["open"] = "analytics_goal_in_progress", ["not_met"] = "analytics_goal_in_progress",
        ["active"] = "analytics_goal_in_progress",
        ["paused"] = "analytics_goal_paused", ["blocked"] = "analytics_goal_blocked",
        ["usage_limited"] = "analytics_goal_limit_reached", ["budget_limited"] = "analytics_goal_limit_reached",
    };

    private static string Text(Func<string, string?> copy, string key) => copy(key) ?? string.Empty;

    /// <summary>Drawn only under the counter pass, in a week with a goal (InsightsPatternsTab.swift:58).</summary>
    public static bool Shows(InsightsCounterWeek? week) =>
        week is { ShowsCounterPass: true, Goals.Total: > 0 };

    /// <summary>"Goals met {n} of {m}".</summary>
    public static string MetOf(InsightsWeekGoals goals, Func<string, string?> copy) =>
        InsightsAnalyticsWords.Fill(Text(copy, "analytics_goals_met_of"),
            ("n", goals.Met.ToString(CultureInfo.InvariantCulture)), ("m", goals.Total.ToString(CultureInfo.InvariantCulture)));

    /// <summary>"Claude" or "Codex"; the dash for a tool the core has no word for.</summary>
    public static string Tool(string tool, Func<string, string?> copy) =>
        copy("analytics_credit_tool_" + tool) is { Length: > 0 } words ? words : InsightsAnalyticsWords.Dash(copy);

    public static string Status(string status, Func<string, string?> copy) =>
        Text(copy, StatusWords.TryGetValue(status, out string? key) ? key : "analytics_goal_status_unknown");

    /// <summary>"{t} tokens · {span}"; an unknown figure is the dash, never 0.</summary>
    public static string Line(InsightsWeekGoal goal, Func<string, string?> copy) =>
        InsightsAnalyticsWords.Fill(Text(copy, "analytics_goal_line"),
            ("t", InsightsAnalyticsWords.Figure(copy, goal.Tokens is { } t ? (ulong)Math.Max(0, t) : null)),
            ("span", Span(goal.TimeSecs, copy)));

    /// <summary>Hours and minutes the tool reported, as "1h 5m"; the dash when it reported none.</summary>
    public static string Span(long? seconds, Func<string, string?> copy)
    {
        if (seconds is not { } total) return InsightsAnalyticsWords.Dash(copy);
        long s = Math.Max(0, total);
        long hours = s / 3600, minutes = s % 3600 / 60;
        if (hours == 0) return minutes.ToString(CultureInfo.InvariantCulture) + "m";
        return minutes == 0
            ? hours.ToString(CultureInfo.InvariantCulture) + "h"
            : hours.ToString(CultureInfo.InvariantCulture) + "h " + minutes.ToString(CultureInfo.InvariantCulture) + "m";
    }
}

/// <summary>One harness on the summary card.</summary>
public sealed record InsightsRecapSource(string Source, ulong Tokens, long? ChangePermille);

/// <summary>One item on the summary card, by its rule; only the fields the drawn rules read.</summary>
public sealed record InsightsRecapItem(string Kind, ulong? Permille, ulong? PreviousBestPermille);

/// <summary>The sessions past the user's context threshold.</summary>
public sealed record InsightsRecapThreshold(ulong Threshold, uint Sessions);

/// <summary>The weekly summary card the core sends on the first opens after a week closes (<c>InsightsRecap</c>).</summary>
public sealed record InsightsRecap(
    string WeekStart, string WeekEnd, uint Sessions, IReadOnlyList<InsightsRecapSource> Sources,
    IReadOnlyList<InsightsRecapItem> Items, InsightsRecapThreshold? PastThreshold)
{
    /// <summary>The card, or null when it is not one.</summary>
    public static InsightsRecap? Parse(JsonElement value)
    {
        try
        {
            JsonElement r = InsightsWeekParts.Obj(value);
            return new InsightsRecap(InsightsWeekParts.Str(r, "week_start"), InsightsWeekParts.Str(r, "week_end"),
                checked((uint)InsightsWeekParts.Long(r, "sessions")),
                InsightsWeekParts.ListOf(r, "sources", s =>
                {
                    JsonElement o = InsightsWeekParts.Obj(s);
                    return new InsightsRecapSource(InsightsWeekParts.Str(o, "source"), checked((ulong)InsightsWeekParts.Long(o, "tokens")),
                        InsightsWeekParts.OptLong(o, "change_permille"));
                }),
                InsightsWeekParts.ListOf(r, "items", i =>
                {
                    JsonElement o = InsightsWeekParts.Obj(i);
                    return new InsightsRecapItem(InsightsWeekParts.Str(o, "kind"),
                        InsightsWeekParts.OptLong(o, "permille") is { } p ? checked((ulong)p) : null,
                        InsightsWeekParts.OptLong(o, "previous_best_permille") is { } q ? checked((ulong)q) : null);
                }),
                InsightsWeekParts.Part(r, "past_threshold", t =>
                {
                    JsonElement o = InsightsWeekParts.Obj(t);
                    return new InsightsRecapThreshold(checked((ulong)InsightsWeekParts.Long(o, "threshold")),
                        checked((uint)InsightsWeekParts.Long(o, "sessions")));
                }));
        }
        catch (Exception e) when (e is InsightsWeekParts.Malformed or OverflowException)
        {
            return null;
        }
    }
}

/// <summary>
/// The summary card's words, the core's filled with the core's figures
/// (<c>InsightsComparisonsWords</c>, InsightsModel.swift:854-904). The lever
/// is not drawn: the Mac no longer draws it anywhere.
/// </summary>
public static class InsightsComparisonsWords
{
    private static string Text(Func<string, string?> copy, string key) => copy(key) ?? string.Empty;

    private static string Percent(ulong permille) => ((permille + 5) / 10).ToString(CultureInfo.InvariantCulture);

    /// <summary>"Your week · {range}".</summary>
    public static string RecapTitle(InsightsRecap recap, Func<string, string?> copy) =>
        InsightsAnalyticsWords.Fill(Text(copy, "analytics_your_week"), ("range", InsightsAnalyticsWords.WeekRange(recap.WeekStart)));

    /// <summary>"{n} traces · {p}% fewer|more tokens than last week"; null when last week cannot be compared.</summary>
    public static string? RecapChange(InsightsRecapSource source, uint sessions, Func<string, string?> copy)
    {
        if (source.ChangePermille is not { } permille) return null;
        string key = permille < 0 ? "analytics_recap_fewer" : "analytics_recap_more";
        return InsightsAnalyticsWords.Fill(Text(copy, key),
            ("n", sessions.ToString(CultureInfo.InvariantCulture)), ("p", Percent((ulong)Math.Abs(permille))));
    }

    /// <summary>One item's line by its rule; null for a kind this shell does not draw.</summary>
    public static string? RecapItem(InsightsRecapItem item, Func<string, string?> copy) => item.Kind switch
    {
        "best_cache_week" when item.Permille is { } now && item.PreviousBestPermille is { } before =>
            InsightsAnalyticsWords.Fill(Text(copy, "analytics_recap_best_cache"), ("p", Percent(now)), ("q", Percent(before))),
        _ => null,
    };

    /// <summary>"{n} traces passed your {threshold} threshold.", only when the user set one.</summary>
    public static string? RecapThreshold(InsightsRecap recap, Func<string, string?> copy) =>
        recap.PastThreshold is { } past
            ? InsightsAnalyticsWords.Fill(Text(copy, "analytics_recap_threshold"),
                ("n", past.Sessions.ToString(CultureInfo.InvariantCulture)), ("threshold", InsightsAnalyticsWords.Figure(copy, past.Threshold)))
            : null;
}

/// <summary>
/// The weekly summary card (feed T only): the core reads the daemon's weekly
/// figures, passed through unchanged; nothing is compared here
/// (<c>InsightsComparisonsModel</c>, InsightsModel.swift:715-800). Under the
/// saved feed it holds nothing and asks nothing.
/// </summary>
public sealed class InsightsComparisonsModel
{
    private readonly ILocalInsights _service;
    private readonly Func<int> _tz;
    private JsonElement? _counterWeeks;
    private string? _weekStart;
    private bool _recapCardEnabled;
    private long _token;

    public InsightsComparisonsModel(ILocalInsights service, Func<int> tz)
    {
        _service = service;
        _tz = tz;
    }

    /// <summary>Raised after the card, or its state, changes.</summary>
    public event Action? Changed;

    public bool Busy { get; private set; }

    public bool Failed { get; private set; }

    /// <summary>The card, while the core says to show it.</summary>
    public InsightsRecap? Recap { get; private set; }

    /// <summary>Completes once the latest request has settled.</summary>
    public Task Settled { get; private set; } = Task.CompletedTask;

    /// <summary>Read for the daemon's week; null weeks (feed S) clear everything.</summary>
    public Task LoadAsync(JsonElement? counterWeeks, string? weekStart, bool recapCardEnabled)
    {
        _counterWeeks = counterWeeks;
        _weekStart = weekStart;
        _recapCardEnabled = recapCardEnabled;
        if (counterWeeks is null)
        {
            Clear();
            return Task.CompletedTask;
        }

        return Reload();
    }

    public void Clear()
    {
        _token++;
        Recap = null;
        Busy = false;
        Failed = false;
        Settled = Task.CompletedTask;
        Changed?.Invoke();
    }

    private Task Reload()
    {
        if (_counterWeeks is not { } weeks) return Task.CompletedTask;
        var operation = new Dictionary<string, object?>
        {
            ["type"] = "comparisons",
            ["week_start"] = _weekStart,
            ["tz"] = _tz(),
            ["counter_weeks"] = weeks,
            ["recap_card_enabled"] = _recapCardEnabled,
        };
        Settled = Run(operation);
        return Settled;
    }

    /// <summary>"View week": the card is not shown again. The closed week to put on screen, or null.</summary>
    public string? OpenRecap()
    {
        if (_counterWeeks is null || Recap?.WeekStart is not { } week) return null;
        Settled = Write(new Dictionary<string, object?> { ["type"] = "recap_opened", ["week_start"] = week });
        return week;
    }

    private async Task Run(Dictionary<string, object?> operation)
    {
        long current = ++_token;
        Busy = true;
        Failed = false;
        Changed?.Invoke();
        InsightsRecap? recap = null;
        bool ok;
        try
        {
            JsonElement response = await _service.CallAsync(operation, CancellationToken.None).ConfigureAwait(true);
            ok = response.ValueKind == JsonValueKind.Object
                && response.TryGetProperty("type", out JsonElement type) && type.GetString() == "comparisons"
                && response.TryGetProperty("comparisons", out JsonElement found) && found.ValueKind == JsonValueKind.Object;
            if (ok && response.GetProperty("comparisons").TryGetProperty("recap", out JsonElement card) && card.ValueKind != JsonValueKind.Null)
            {
                recap = InsightsRecap.Parse(card);
                ok = recap is not null;
            }
        }
        catch (Exception)
        {
            ok = false;
        }

        if (current != _token) return;
        // A failed read never keeps an earlier week's card.
        Recap = ok ? recap : null;
        Failed = !ok;
        Busy = false;
        Changed?.Invoke();
    }

    private async Task Write(Dictionary<string, object?> operation)
    {
        long current = ++_token;
        Busy = true;
        Changed?.Invoke();
        bool ok;
        try
        {
            JsonElement response = await _service.CallAsync(operation, CancellationToken.None).ConfigureAwait(true);
            ok = response.ValueKind == JsonValueKind.Object
                && response.TryGetProperty("type", out JsonElement type) && type.GetString() == (string)operation["type"]!
                && response.TryGetProperty("state", out JsonElement state) && state.ValueKind != JsonValueKind.Null;
        }
        catch (Exception)
        {
            ok = false;
        }

        if (current != _token) return;
        Busy = false;
        if (!ok)
        {
            Failed = true;
            Changed?.Invoke();
            return;
        }

        await Reload().ConfigureAwait(true);
    }
}
