using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text.Json;

namespace TraceCommons.Interop;

// The token week over saved snapshots (feed S), as the local Insights service
// returns it for the Overview, Patterns and Sessions tabs. Decode only: every
// figure is computed in the core, and an absent figure is unknown, never
// zero. Wire labels are kept as the core spells them; the words for them are
// the core's ui_copy() analytics table (DRAFT, NEEDS APPROVAL, owner
// decision D17, open).

public sealed record AnalyticsCoverage(uint Known, uint Partial, uint Unknown, IReadOnlyDictionary<string, uint> Reasons)
{
    public uint Sessions => Known + Partial + Unknown;
}
/// <summary>A cache share with its parts, so the shell divides nothing.</summary>
public sealed record AnalyticsShare(ulong Numerator, ulong Denominator, ulong Permille);
/// <summary>One harness's line. Lines are never summed together.</summary>
public sealed record AnalyticsWeekSource(string Source, uint Sessions, ulong? Tokens, AnalyticsShare? CacheShare,
    ulong? LargestTokens, string Change, string BestWeek);
public sealed record AnalyticsDay(string Date, ulong Uncached, ulong CacheRead, ulong CacheWrite, ulong Output);
/// <summary>A declared model label; <c>null</c> is the unknown label.</summary>
public sealed record AnalyticsModelTokens(string? Label, ulong Tokens);
public sealed record AnalyticsToolTokens(string Source, ulong? Tokens);
public sealed record AnalyticsWeekOverview(string Feed, string WeekStart, string WeekEnd, AnalyticsCoverage Coverage,
    uint UndatedSessions, uint Sessions, IReadOnlyList<AnalyticsWeekSource> Sources, IReadOnlyList<AnalyticsDay>? ByDay,
    ulong? CodexIntervalTokens, IReadOnlyList<AnalyticsModelTokens> ByModel, IReadOnlyList<AnalyticsToolTokens> ByTool,
    string ByProject, IReadOnlyList<string> Weeks);
public sealed record AnalyticsCardSession(string SessionRef, string? Source, ulong? Tokens, AnalyticsShare? CacheShare,
    string State, IReadOnlyList<string> Reasons);
/// <summary>What makes up one Overview card's figure.</summary>
public sealed record AnalyticsCardInputs(string Card, string Feed, string WeekStart, IReadOnlyList<AnalyticsWeekSource> Sources,
    AnalyticsCoverage Coverage, IReadOnlyList<AnalyticsCardSession> Sessions);
/// <summary>One week's bar. <c>Tokens == null</c> is a gap, never a zero bar.</summary>
public sealed record AnalyticsPatternWeek(string WeekStart, ulong? Tokens);
public sealed record AnalyticsPatternCard(string Kind, ulong? Tokens, uint? Count, uint? Files, uint Sessions, string Basis,
    bool Inferred, IReadOnlyList<AnalyticsPatternWeek> Weeks, long? Change);
/// <summary>A re-read file: a letter and the extension, never a path or a name.</summary>
public sealed record AnalyticsRereadRow(string Letter, string? Ext, uint Reads, uint AfterShrink, ulong? Tokens);
public sealed record AnalyticsWeekPatterns(string Feed, string WeekStart, string WeekEnd, AnalyticsCoverage Coverage,
    uint Sessions, uint ClaudeSessions, bool ClaudeOnly, ulong LongContextThreshold, IReadOnlyList<AnalyticsPatternCard> Cards,
    IReadOnlyList<AnalyticsRereadRow> RereadFiles, IReadOnlyList<string> Weeks);
public sealed record AnalyticsPatternSession(string SessionRef, uint Count, ulong? Tokens, string State, IReadOnlyList<string> Reasons);
public sealed record AnalyticsPatternSessions(string Pattern, string Feed, string WeekStart, IReadOnlyList<AnalyticsPatternSession> Sessions);
/// <summary>One turn. Every counter is <c>null</c> when any of the turn's counters is unknown.</summary>
public sealed record AnalyticsDrillTurn(uint Ordinal, uint? Uncached, uint? CacheRead, ulong? CacheWrite, uint? Output);
/// <summary>One lettered marker. Only its kind's fields are present.</summary>
public sealed record AnalyticsDrillMarker(string Letter, uint TurnOrdinal, string Kind, string Basis, uint? PauseMinutes,
    ulong? CacheWrite, string? FileLetter, string? FileExt);
public sealed record AnalyticsSessionDrill(string Feed, string SessionRef, string? Source, string? Date, uint? Turns,
    ulong? Tokens, ulong? SpanSecs, string State, IReadOnlyList<string> Reasons, ulong LongContextThreshold,
    IReadOnlyList<AnalyticsDrillTurn>? Series, string? SeriesUnavailable, IReadOnlyList<AnalyticsDrillMarker> Markers);

public static class InsightsAnalyticsResponses
{
    public static AnalyticsWeekOverview DecodeOverview(JsonElement response)
    {
        var item = Body(response, "week_overview", "overview");
        return new AnalyticsWeekOverview(Required(item, "feed"), Required(item, "week_start"), Required(item, "week_end"),
            DecodeCoverage(item.GetProperty("coverage")), item.GetProperty("undated_sessions").GetUInt32(),
            item.GetProperty("sessions").GetUInt32(), Array(item, "sources", DecodeSource),
            Nullable(item, "by_day") is { } days ? days.EnumerateArray().Select(day => new AnalyticsDay(Required(day, "date"),
                day.GetProperty("uncached").GetUInt64(), day.GetProperty("cache_read").GetUInt64(),
                day.GetProperty("cache_write").GetUInt64(), day.GetProperty("output").GetUInt64())).ToArray() : null,
            OptionalU64(item, "codex_interval_tokens"),
            Array(item, "by_model", row => new AnalyticsModelTokens(Optional(row, "label"), row.GetProperty("tokens").GetUInt64())),
            Array(item, "by_tool", row => new AnalyticsToolTokens(Required(row, "source"), OptionalU64(row, "tokens"))),
            Required(item, "by_project"), Array(item, "weeks", week => week.GetString() ?? Invalid<string>()));
    }

    public static AnalyticsCardInputs DecodeCardInputs(JsonElement response)
    {
        var item = Body(response, "card_inputs", "inputs");
        return new AnalyticsCardInputs(Required(item, "card"), Required(item, "feed"), Required(item, "week_start"),
            Array(item, "sources", DecodeSource), DecodeCoverage(item.GetProperty("coverage")),
            Array(item, "sessions", session => new AnalyticsCardSession(Required(session, "session_ref"),
                Optional(session, "source"), OptionalU64(session, "tokens"), DecodeShare(session, "cache_share"),
                Required(session, "state"), Strings(session, "reasons"))));
    }

    public static AnalyticsWeekPatterns DecodePatterns(JsonElement response)
    {
        var item = Body(response, "patterns", "patterns");
        return new AnalyticsWeekPatterns(Required(item, "feed"), Required(item, "week_start"), Required(item, "week_end"),
            DecodeCoverage(item.GetProperty("coverage")), item.GetProperty("sessions").GetUInt32(),
            item.GetProperty("claude_sessions").GetUInt32(), item.GetProperty("claude_only").GetBoolean(),
            item.GetProperty("long_context_threshold").GetUInt64(),
            Array(item, "cards", card => new AnalyticsPatternCard(Required(card, "kind"), OptionalU64(card, "tokens"),
                OptionalU32(card, "count"), OptionalU32(card, "files"), card.GetProperty("sessions").GetUInt32(),
                Required(card, "basis"), card.GetProperty("inferred").GetBoolean(),
                Array(card, "weeks", week => new AnalyticsPatternWeek(Required(week, "week_start"), OptionalU64(week, "tokens"))),
                Nullable(card, "change")?.GetInt64())),
            Array(item, "reread_files", row => new AnalyticsRereadRow(Required(row, "letter"), Optional(row, "ext"),
                row.GetProperty("reads").GetUInt32(), row.GetProperty("after_shrink").GetUInt32(), OptionalU64(row, "tokens"))),
            Array(item, "weeks", week => week.GetString() ?? Invalid<string>()));
    }

    public static AnalyticsPatternSessions DecodePatternSessions(JsonElement response)
    {
        var item = Body(response, "pattern_sessions", "pattern_sessions");
        return new AnalyticsPatternSessions(Required(item, "pattern"), Required(item, "feed"), Required(item, "week_start"),
            Array(item, "sessions", session => new AnalyticsPatternSession(Required(session, "session_ref"),
                session.GetProperty("count").GetUInt32(), OptionalU64(session, "tokens"), Required(session, "state"),
                Strings(session, "reasons"))));
    }

    public static AnalyticsSessionDrill DecodeSessionDrill(JsonElement response)
    {
        var item = Body(response, "session_drill", "session");
        return new AnalyticsSessionDrill(Required(item, "feed"), Required(item, "session_ref"), Optional(item, "source"),
            Optional(item, "date"), OptionalU32(item, "turns"), OptionalU64(item, "tokens"), OptionalU64(item, "span_secs"),
            Required(item, "state"), Strings(item, "reasons"), item.GetProperty("long_context_threshold").GetUInt64(),
            Nullable(item, "series") is { } series ? series.EnumerateArray().Select(turn => new AnalyticsDrillTurn(
                turn.GetProperty("ordinal").GetUInt32(), OptionalU32(turn, "uncached"), OptionalU32(turn, "cache_read"),
                OptionalU64(turn, "cache_write"), OptionalU32(turn, "output"))).ToArray() : null,
            Optional(item, "series_unavailable"),
            Array(item, "markers", marker => new AnalyticsDrillMarker(Required(marker, "letter"),
                marker.GetProperty("turn_ordinal").GetUInt32(), Required(marker, "kind"), Required(marker, "basis"),
                OptionalU32(marker, "pause_minutes"), OptionalU64(marker, "cache_write"), Optional(marker, "file_letter"),
                Optional(marker, "file_ext"))));
    }

    private static AnalyticsWeekSource DecodeSource(JsonElement source) => new(Required(source, "source"),
        source.GetProperty("sessions").GetUInt32(), OptionalU64(source, "tokens"), DecodeShare(source, "cache_share"),
        Nullable(source, "largest_session") is { } largest ? largest.GetProperty("tokens").GetUInt64() : null,
        Required(source, "change"), Required(source, "best_week"));

    private static AnalyticsShare? DecodeShare(JsonElement item, string name) =>
        Nullable(item, name) is { } share ? new AnalyticsShare(share.GetProperty("numerator").GetUInt64(),
            share.GetProperty("denominator").GetUInt64(), share.GetProperty("permille").GetUInt64()) : null;

    private static AnalyticsCoverage DecodeCoverage(JsonElement coverage) => new(coverage.GetProperty("known").GetUInt32(),
        coverage.GetProperty("partial").GetUInt32(), coverage.GetProperty("unknown").GetUInt32(),
        coverage.GetProperty("reasons").EnumerateObject().ToDictionary(pair => pair.Name, pair => pair.Value.GetUInt32(), StringComparer.Ordinal));

    private static JsonElement Body(JsonElement response, string type, string field)
    {
        if (Required(response, "type") != type) Invalid<string>();
        var body = response.GetProperty(field);
        if (body.ValueKind != JsonValueKind.Object) Invalid<string>();
        return body;
    }

    private static IReadOnlyList<T> Array<T>(JsonElement item, string name, Func<JsonElement, T> decode)
    {
        var array = item.GetProperty(name);
        if (array.ValueKind != JsonValueKind.Array) Invalid<string>();
        return array.EnumerateArray().Select(decode).ToArray();
    }

    private static IReadOnlyList<string> Strings(JsonElement item, string name) =>
        Array(item, name, value => value.GetString() ?? Invalid<string>());

    private static JsonElement? Nullable(JsonElement item, string name) =>
        item.TryGetProperty(name, out var value) && value.ValueKind != JsonValueKind.Null ? value : null;

    private static string Required(JsonElement item, string name) =>
        item.GetProperty(name).GetString() ?? Invalid<string>();

    private static string? Optional(JsonElement item, string name) => Nullable(item, name)?.GetString();
    private static ulong? OptionalU64(JsonElement item, string name) => Nullable(item, name)?.GetUInt64();
    private static uint? OptionalU32(JsonElement item, string name) => Nullable(item, name)?.GetUInt32();

    private static T Invalid<T>() => throw new InvalidOperationException("insights-response-invalid");
}

/// <summary>
/// The core's analytics words, filled with the core's figures. Nothing here
/// composes a sentence: a template's <c>{name}</c> holes take numbers the core
/// computed, and every key is the core's. An unknown figure is the core's
/// dash, never zero.
/// </summary>
public static class InsightsAnalyticsWords
{
    /// <summary>The core's figure for "not shown".</summary>
    public static string Dash(Func<string, string?> copy) => copy("analytics_unavailable") ?? "";

    /// <summary>The core's word for <paramref name="key"/>, or the dash when the core has none.</summary>
    public static string Text(Func<string, string?> copy, string key) => copy(key) ?? Dash(copy);

    public static string Fill(string template, params (string Name, string Value)[] holes) =>
        holes.Aggregate(template, (filled, hole) => filled.Replace("{" + hole.Name + "}", hole.Value, StringComparison.Ordinal));

    /// <summary>A count, or the dash for unknown. A measured zero stays zero.</summary>
    public static string Figure(Func<string, string?> copy, ulong? value) =>
        value is { } known ? known.ToString("N0", CultureInfo.CurrentCulture) : Dash(copy);

    /// <summary>Whole percent from the core's per mille, rounded half up.</summary>
    public static string Share(Func<string, string?> copy, AnalyticsShare? share) =>
        share is null ? Dash(copy) : ((share.Permille + 5) / 10).ToString(CultureInfo.CurrentCulture);

    public static string CoverageLine(Func<string, string?> copy, AnalyticsCoverage coverage) =>
        Fill(Text(copy, "analytics_coverage_line"), ("k", Count(coverage.Known)), ("n", Count(coverage.Sessions)),
            ("p", Count(coverage.Partial)), ("u", Count(coverage.Unknown)));

    /// <summary>Which feed is showing. Feed S also says that weeks are not compared.</summary>
    public static IReadOnlyList<string> FeedLines(Func<string, string?> copy, string feed) => feed switch {
        "saved" => new[] { Text(copy, "analytics_feed_saved"), Text(copy, "analytics_feed_comparisons_need_counter_pass") },
        "counter_pass" => new[] { Text(copy, "analytics_feed_counter_pass") },
        "ledger" => new[] { Text(copy, "analytics_feed_ledger") },
        _ => System.Array.Empty<string>()
    };

    public static string Reason(Func<string, string?> copy, string wire) => Text(copy, "analytics_reason_" + wire);
    public static string Reasons(Func<string, string?> copy, IEnumerable<string> wires) =>
        string.Join(" · ", wires.Select(wire => Reason(copy, wire)));
    public static string State(Func<string, string?> copy, string wire) => Text(copy, "analytics_state_" + wire);
    public static string Harness(Func<string, string?> copy, string? source) => source is null ? Dash(copy) : Text(copy, source);
    public static string SourceLine(Func<string, string?> copy, string source) => Text(copy, "analytics_source_" + source);

    /// <summary>
    /// "vs last week" and "Your best week" on an Overview line. The core sends
    /// only the reason there is no figure under feed S, so both are the dash.
    /// </summary>
    public static string Change(Func<string, string?> copy, AnalyticsWeekSource source) => Dash(copy);

    public static string CacheShareLine(Func<string, string?> copy, AnalyticsWeekSource source) =>
        Fill(Text(copy, "analytics_cache_share_line"), ("source", Harness(copy, source.Source)), ("p", Share(copy, source.CacheShare)));

    public static string? LargestLine(Func<string, string?> copy, AnalyticsWeekSource source) =>
        source.LargestTokens is { } tokens
            ? Harness(copy, source.Source) + " · " + Fill(Text(copy, "analytics_largest"), ("t", Figure(copy, tokens)))
            : null;

    /// <summary>Monday to Sunday, as ISO dates.</summary>
    public static string WeekRange(string start)
    {
        if (!DateOnly.TryParseExact(start, "yyyy-MM-dd", CultureInfo.InvariantCulture, DateTimeStyles.None, out var first))
            return start;
        return start + " – " + first.AddDays(6).ToString("yyyy-MM-dd", CultureInfo.InvariantCulture);
    }

    /// <summary>The core's dated weeks, newest first, with the week on screen first when it holds no saved session.</summary>
    public static IReadOnlyList<string> WeekChoices(IReadOnlyList<string> weeks, string shown) =>
        weeks.Contains(shown, StringComparer.Ordinal) ? weeks.ToArray() : new[] { shown }.Concat(weeks).ToArray();

    public static string PatternTitle(Func<string, string?> copy, string kind) => Text(copy, "analytics_pattern_" + kind);

    /// <summary>The line under the headline: the core's count sentence, or the long-context threshold line.</summary>
    public static string CountLine(Func<string, string?> copy, AnalyticsPatternCard card, ulong threshold) => card.Kind switch {
        "repeated_reads" => Fill(Text(copy, "analytics_pattern_repeated_reads_count"),
            ("r", Figure(copy, card.Count)), ("f", Figure(copy, card.Files))),
        "retried_calls" => Fill(Text(copy, "analytics_pattern_retried_calls_count"), ("c", Figure(copy, card.Count))),
        "edit_fail_edit" => Fill(Text(copy, "analytics_pattern_edit_fail_edit_count"), ("l", Figure(copy, card.Count))),
        "long_context" => Fill(Text(copy, "analytics_pattern_long_context_line"), ("threshold", Figure(copy, threshold))),
        _ => Dash(copy)
    };

    /// <summary>How the figure was arrived at: the inferred label first (owner decision D9, open), then the basis.</summary>
    public static IReadOnlyList<string> BasisLines(Func<string, string?> copy, AnalyticsPatternCard card)
    {
        var lines = new List<string>();
        if (card.Inferred) lines.Add(Text(copy, "analytics_inferred_from_order"));
        if (card.Basis == "estimate_from_result_size") lines.Add(Text(copy, "analytics_estimate_from_result_size"));
        else if (card.Basis == "from_counters") lines.Add(Text(copy, "analytics_from_counters"));
        return lines;
    }

    /// <summary>"vs last week". Only feed T sends a figure; anything else is the dash.</summary>
    public static string PatternChange(Func<string, string?> copy, AnalyticsPatternCard card)
    {
        if (card.Change is not { } permille) return Dash(copy);
        string percent = ((Math.Abs(permille) + 5) / 10).ToString(CultureInfo.CurrentCulture);
        return Fill(Text(copy, permille < 0 ? "analytics_change_down" : "analytics_change_up"), ("p", percent));
    }

    public static string? SeeSessions(Func<string, string?> copy, AnalyticsPatternCard card) =>
        card.Sessions > 0 ? Fill(Text(copy, "analytics_see_sessions"), ("n", Count(card.Sessions))) : null;

    /// <summary>"File B · .rs", or "File B" with no extension (owner decision D8, open).</summary>
    public static string FileLabel(Func<string, string?> copy, string letter, string? ext) => ext is null
        ? Fill(Text(copy, "analytics_file_label_no_ext"), ("letter", letter))
        : Fill(Text(copy, "analytics_file_label"), ("letter", letter), ("ext", ext));

    public static string? ClaudeOnlyLine(Func<string, string?> copy, AnalyticsWeekPatterns patterns) => patterns.ClaudeOnly
        ? Fill(Text(copy, "analytics_claude_sessions_only"), ("k", Count(patterns.ClaudeSessions)), ("n", Count(patterns.Sessions)))
        : null;

    /// <summary>Hours and minutes between the first and last event, as h:mm. Not active time.</summary>
    public static string Span(Func<string, string?> copy, ulong? seconds) => seconds is { } total
        ? (total / 3600).ToString(CultureInfo.InvariantCulture) + ":" + (total % 3600 / 60).ToString("00", CultureInfo.InvariantCulture)
        : Dash(copy);

    public static string SessionHeader(Func<string, string?> copy, AnalyticsSessionDrill drill) =>
        Fill(Text(copy, "analytics_session_header"), ("date", drill.Date ?? Dash(copy)), ("harness", Harness(copy, drill.Source)),
            ("n", drill.Turns is { } turns ? Count(turns) : Dash(copy)), ("t", Figure(copy, drill.Tokens)),
            ("span", Span(copy, drill.SpanSecs)));

    /// <summary>Why there is no chart; <c>null</c> when there is one (owner decision D11, open, for Codex).</summary>
    public static string? SeriesUnavailableLine(Func<string, string?> copy, AnalyticsSessionDrill drill) =>
        drill.SeriesUnavailable switch {
            null => null,
            "not_recorded" => Text(copy, "analytics_codex_not_recorded"),
            var reason => Reason(copy, reason)
        };

    public static string TurnLabel(Func<string, string?> copy, uint ordinal) => Fill(Text(copy, "analytics_turn"), ("n", Count(ordinal)));

    /// <summary>A marker card: its title, its detail, a re-read's file label, and last the derivation label.</summary>
    public static IReadOnlyList<string> MarkerLines(Func<string, string?> copy, AnalyticsDrillMarker marker, ulong threshold)
    {
        string turn = Count(marker.TurnOrdinal);
        var lines = new List<string>();
        switch (marker.Kind)
        {
            case "cache_written_again":
                lines.Add(Fill(Text(copy, "analytics_marker_cache_rewrite"),
                    ("m", marker.PauseMinutes is { } minutes ? Count(minutes) : Dash(copy))));
                lines.Add(Fill(Text(copy, "analytics_marker_cache_rewrite_detail"), ("t", turn), ("x", Figure(copy, marker.CacheWrite))));
                break;
            case "context_shrank":
                lines.Add(Fill(Text(copy, "analytics_marker_shrank"), ("t", turn)));
                break;
            case "crossed_long_context":
                lines.Add(Fill(Text(copy, "analytics_marker_crossed"), ("threshold", Figure(copy, threshold))));
                lines.Add(Text(copy, "analytics_marker_crossed_detail"));
                break;
            case "re_read":
                string letter = marker.FileLetter ?? Dash(copy);
                lines.Add(Fill(Text(copy, "analytics_marker_reread"), ("letter", letter)));
                lines.Add(Text(copy, "analytics_marker_reread_detail"));
                lines.Add(FileLabel(copy, letter, marker.FileExt));
                break;
            default:
                return new[] { Dash(copy) };
        }
        switch (marker.Basis)
        {
            case "inferred_from_counters": lines.Add(Text(copy, "analytics_marker_inferred")); break;
            case "from_counters": lines.Add(Text(copy, "analytics_from_counters")); break;
            case "from_tool_calls": lines.Add(Text(copy, "analytics_marker_from_tool_calls")); break;
        }
        return lines;
    }

    private static string Count(uint value) => value.ToString(CultureInfo.CurrentCulture);
}
