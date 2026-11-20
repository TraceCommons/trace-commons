using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>
/// One counted session in <c>insights_week</c>'s <c>rollup.sessions</c>
/// (<c>DaemonData.InsightsWeekSession</c>, DaemonDataModels.swift:2943-2971):
/// labels and counts only. An absent figure is unknown, never zero.
/// </summary>
public sealed record InsightsCounterSession(
    string? Source, string? Folder, string? SessionRef, ulong? Tokens, string State,
    IReadOnlyList<string> Reasons, string? StartedAt, string? LastEventAt, IReadOnlyList<ulong>? ByDay);

/// <summary>
/// <c>insights_week</c>: one local ISO week from the daemon's counter pass
/// (Insights feed T, <c>DaemonData.InsightsWeek</c>,
/// DaemonDataModels.swift:2253-2325). It is drawn only when
/// <see cref="ShowsCounterPass"/>; anything else is the saved-imports feed.
/// The core's Overview and Patterns are kept as the JSON they came as, so the
/// window decodes them with the same readers the saved week uses.
/// </summary>
public sealed class InsightsCounterWeek
{
    public const string Method = "insights_week";

    private InsightsCounterWeek(bool enabled, string feed, bool? readable, string? weekStart, string? isoWeek,
        JsonElement? overview, JsonElement? patterns, IReadOnlyList<InsightsCounterSession>? sessions)
    {
        Enabled = enabled;
        Feed = feed;
        Readable = readable;
        WeekStart = weekStart;
        IsoWeek = isoWeek;
        Overview = overview;
        Patterns = patterns;
        Sessions = sessions;
    }

    public bool Enabled { get; }

    public string Feed { get; }

    /// <summary>Null while disabled; false when the store or key could not be read, never drawn as zero.</summary>
    public bool? Readable { get; }

    public string? WeekStart { get; }

    public string? IsoWeek { get; }

    /// <summary>The core's Overview for the week, as sent.</summary>
    public JsonElement? Overview { get; }

    /// <summary>The core's Patterns for the week, as sent.</summary>
    public JsonElement? Patterns { get; }

    /// <summary>The rollup's counted sessions, in the daemon's order; null when no rollup came.</summary>
    public IReadOnlyList<InsightsCounterSession>? Sessions { get; }

    /// <summary>Enabled, readable, from the counter pass, with a rollup and the core's Overview (:2308-2310).</summary>
    /// <summary>Spend at API list prices; null from a daemon that predates it.</summary>
    public InsightsWork? Work { get; private init; }

    /// <summary>The goals set with <c>/goal</c> this week; null from a daemon that predates them.</summary>
    public InsightsWeekGoals? Goals { get; private init; }

    /// <summary>NEAR AI's own charge; null while routing is unavailable, never zero.</summary>
    public InsightsNearAiCredit? NearAiCredit { get; private init; }

    /// <summary>Claude Code and Codex tokens on each of the last 7 local days.</summary>
    public IReadOnlyList<InsightsUsageDay>? UsageByDay { get; private init; }

    /// <summary>The kept weeks' figures, passed to the core's <c>comparisons</c> unchanged.</summary>
    public JsonElement? History { get; private init; }

    /// <summary>The daemon's <c>insights_recap_card_enabled</c>, passed to <c>comparisons</c>.</summary>
    public bool? RecapCardEnabled { get; private init; }

    /// <summary>Each source's change on last week.</summary>
    public IReadOnlyList<InsightsWeekChange>? ChangeVsLastWeek { get; private init; }

    public bool ShowsCounterPass =>
        Enabled && Readable == true && Feed == "counter_pass" && Sessions is not null && Overview is not null;

    /// <summary>
    /// The request's parameters (<c>LiveDaemonClient.insightsWeek</c>,
    /// LiveDaemonClient.swift:298-301): the shell's offset always, the ISO week
    /// only when one is asked for.
    /// </summary>
    public static string Parameters(string? isoWeek, int tz) => isoWeek is null
        ? JsonSerializer.Serialize(new { tz })
        : JsonSerializer.Serialize(new { tz, iso_week = isoWeek });

    /// <summary>A reply, or null when it is not an object with the fields every reply carries.</summary>
    public static InsightsCounterWeek? Parse(JsonElement result)
    {
        if (result.ValueKind != JsonValueKind.Object
            || !result.TryGetProperty("enabled", out JsonElement enabled) || enabled.ValueKind is not (JsonValueKind.True or JsonValueKind.False)
            || Str(result, "feed") is not { } feed)
        {
            return null;
        }

        bool? readable = result.TryGetProperty("readable", out JsonElement r) && r.ValueKind is JsonValueKind.True or JsonValueKind.False
            ? r.GetBoolean()
            : null;
        JsonElement? overview = Obj(result, "overview");
        JsonElement? patterns = Obj(result, "patterns");
        IReadOnlyList<InsightsCounterSession>? sessions = null;
        if (Obj(result, "rollup") is { } rollup)
        {
            if (!rollup.TryGetProperty("sessions", out JsonElement rows) || rows.ValueKind != JsonValueKind.Array)
            {
                return null;
            }

            var list = new List<InsightsCounterSession>();
            foreach (JsonElement row in rows.EnumerateArray())
            {
                if (Session(row) is not { } session)
                {
                    return null;
                }

                list.Add(session);
            }

            sessions = list;
        }

        // The Mac decodes the whole reply or none of it: a part that is there
        // but malformed refuses the reply (DaemonDataModels.swift:2258-2328).
        try
        {
            JsonElement? history = null;
            if (result.TryGetProperty("history", out JsonElement kept) && kept.ValueKind != JsonValueKind.Null)
            {
                history = kept.ValueKind == JsonValueKind.Array ? kept.Clone() : InsightsWeekParts.Fail<JsonElement>();
            }

            return new InsightsCounterWeek(enabled.GetBoolean(), feed, readable, Str(result, "week_start"), Str(result, "iso_week"),
                overview, patterns, sessions)
            {
                Work = InsightsWeekParts.Part(result, "work", InsightsWeekParts.Work),
                Goals = InsightsWeekParts.Part(result, "goals", InsightsWeekParts.Goals),
                NearAiCredit = InsightsWeekParts.Part(result, "near_ai_credit", InsightsWeekParts.Credit),
                UsageByDay = InsightsWeekParts.Part(result, "usage_by_day", InsightsWeekParts.UsageDays),
                History = history,
                RecapCardEnabled = InsightsWeekParts.OptBool(result, "recap_card_enabled"),
                ChangeVsLastWeek = InsightsWeekParts.Part(result, "change_vs_last_week", InsightsWeekParts.Changes),
            };
        }
        catch (Exception e) when (e is InsightsWeekParts.Malformed or OverflowException)
        {
            return null;
        }
    }

    private static InsightsCounterSession? Session(JsonElement row)
    {
        if (row.ValueKind != JsonValueKind.Object || Str(row, "state") is not { } state)
        {
            return null;
        }

        var reasons = new List<string>();
        if (row.TryGetProperty("reasons", out JsonElement list) && list.ValueKind == JsonValueKind.Array)
        {
            reasons.AddRange(list.EnumerateArray().Where(e => e.ValueKind == JsonValueKind.String).Select(e => e.GetString()!));
        }

        ulong? tokens = row.TryGetProperty("tokens", out JsonElement t) && t.ValueKind == JsonValueKind.Number && t.TryGetUInt64(out ulong value)
            ? value
            : null;
        IReadOnlyList<ulong>? byDay = null;
        if (row.TryGetProperty("by_day", out JsonElement days) && days.ValueKind == JsonValueKind.Array)
        {
            // The core's unsigned figures: a negative day reads as 0 (InsightsDayBreakdown.split).
            byDay = days.EnumerateArray()
                .Select(d => d.ValueKind == JsonValueKind.Number && d.TryGetInt64(out long n) ? (ulong)Math.Max(0, n) : 0UL)
                .ToList();
        }

        return new InsightsCounterSession(Str(row, "source"), Str(row, "folder"), Str(row, "session_ref"), tokens, state,
            reasons, Str(row, "started_at"), Str(row, "last_event_at"), byDay);
    }

    /// <summary>
    /// <c>yyyy-MM-dd</c> (a Monday) as the ISO week <c>insights_week</c> takes
    /// (<c>InsightsOverviewWords.isoWeek</c>, InsightsModel.swift:318-327);
    /// null for anything that is not a date.
    /// </summary>
    public static string? IsoWeekOf(string weekStart)
    {
        if (!DateTime.TryParseExact(weekStart, "yyyy-MM-dd", CultureInfo.InvariantCulture, DateTimeStyles.None, out DateTime day))
        {
            return null;
        }

        return string.Format(CultureInfo.InvariantCulture, "{0:0000}-W{1:00}", ISOWeek.GetYear(day), ISOWeek.GetWeekOfYear(day));
    }

    /// <summary>
    /// A counted session's title: its folder, or the core's unknown-folder
    /// word, then its reference (<c>InsightsSessionsWords.counterTitle</c>,
    /// InsightsModel.swift:619-624).
    /// </summary>
    public static string Title(InsightsCounterSession row, Func<string, string?> copy)
    {
        string folder = string.IsNullOrEmpty(row.Folder) ? copy("analytics_folder_unknown") ?? string.Empty : row.Folder;
        return string.IsNullOrEmpty(row.SessionRef) ? folder : folder + " · " + row.SessionRef;
    }

    private static string? Str(JsonElement item, string name) =>
        item.TryGetProperty(name, out JsonElement value) && value.ValueKind == JsonValueKind.String ? value.GetString() : null;

    private static JsonElement? Obj(JsonElement item, string name) =>
        item.TryGetProperty(name, out JsonElement value) && value.ValueKind == JsonValueKind.Object ? value.Clone() : null;
}
