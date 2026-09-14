using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;

namespace TraceCommons.Interop;

public sealed record InsightCardValue(string Type, ulong? UnsignedValue, long? SignedValue);
public sealed record InsightCardRow(string Id, string Unit, string? Label, InsightCardValue? Value, string? MissingReason);
public sealed record InsightCardCoverage(string Unit, ulong Observed, ulong Eligible);
public sealed record InsightCardDenominator(ulong Eligible, ulong Assessed, ulong Unassessed, ulong ExplicitUnknown);
public sealed record InsightQuestionCard(string Question, string MetricVersion, string State,
    IReadOnlyList<InsightCardRow> Rows, IReadOnlyList<InsightCardCoverage> Coverage,
    InsightCardDenominator? EpisodeDenominator, IReadOnlyList<string> EvidenceIds,
    IReadOnlyList<string> EpisodeIds, IReadOnlyList<string> Limitations, bool RowsOmitted);
public sealed record InsightCardResult(uint SchemaVersion, string ProviderId, string ProviderVersion,
    string RubricVersion, string InputDigest, IReadOnlyList<InsightQuestionCard> Cards, string Text);

public static class InsightCardResponses
{
    private static readonly HashSet<string> Questions = new(StringComparer.Ordinal) {
        "recorded_activity", "episode_outcomes", "observed_models", "estimated_cost"
    };
    private static readonly HashSet<string> States = new(StringComparer.Ordinal) { "observed", "partial", "unavailable" };
    private static readonly HashSet<string> Rows = new(StringComparer.Ordinal) {
        "saved_snapshots", "normalized_events", "tool_calls", "tool_failures", "timestamp_eligible", "timestamp_valid",
        "timestamp_missing", "timestamp_invalid", "earliest_recorded_at", "latest_recorded_at", "record_span",
        "eligible_episodes", "assessed_episodes", "accepted_episodes", "partial_episodes", "rejected_episodes",
        "explicit_unknown_episodes", "unassessed_episodes", "overlapping_episodes", "distinct_snapshots", "observed_model", "estimated_cost"
    };
    private static readonly HashSet<string> Units = new(StringComparer.Ordinal) {
        "saved_snapshots", "normalized_events", "tool_calls", "tool_results", "episode_groups", "distinct_saved_snapshots",
        "timestamp_records", "unix_milliseconds", "milliseconds", "model_records", "us_dollars"
    };
    private static readonly HashSet<string> Coverage = new(StringComparer.Ordinal) {
        "saved_snapshots", "normalized_events", "tool_call_events", "tool_results", "episode_groups",
        "timestamp_evidence_snapshots", "timestamp_records", "model_records"
    };
    private static readonly HashSet<string> Missing = new(StringComparer.Ordinal) {
        "no_eligible_evidence", "no_observed_value", "insufficient_timestamps", "usage_not_persisted", "pricing_unavailable"
    };
    private static readonly HashSet<string> Limitations = new(StringComparer.Ordinal) {
        "user_selected_episode_groups", "episodes_not_independent_tasks", "episode_groups_overlap", "timestamps_are_record_span",
        "record_span_is_not_active_time", "tool_failures_are_not_rejections", "model_declarations_are_observed_metadata",
        "cost_unavailable_without_persisted_usage_and_pricing"
    };

    public static InsightCardResult Decode(JsonElement response)
    {
        if (Required(response, "type") != "question_cards") Invalid();
        var result = response.GetProperty("result");
        uint schema = result.GetProperty("schema_version").GetUInt32();
        if (schema != 1) Invalid();
        var provider = result.GetProperty("provider");
        var cards = result.GetProperty("cards").EnumerateArray().Select(DecodeCard).ToArray();
        string[] expected = { "recorded_activity", "episode_outcomes", "observed_models", "estimated_cost" };
        if (cards.Length != expected.Length || !cards.Select(card => card.Question).SequenceEqual(expected, StringComparer.Ordinal))
            Invalid();
        return new InsightCardResult(schema, Required(provider, "id"), Required(provider, "version"),
            Required(provider, "rubric_version"), Required(result, "input_digest"), cards, Required(response, "text"));
    }

    private static InsightQuestionCard DecodeCard(JsonElement card)
    {
        string question = Required(card, "question");
        string state = Required(card, "state");
        if (!Questions.Contains(question) || !States.Contains(state)) Invalid();
        var rows = card.GetProperty("rows").EnumerateArray().Select(row => {
            InsightCardValue? value = null;
            if (row.GetProperty("value").ValueKind != JsonValueKind.Null)
            {
                var raw = row.GetProperty("value");
                string type = Required(raw, "type");
                value = type == "unix_milliseconds"
                    ? new InsightCardValue(type, null, raw.GetProperty("value").GetInt64())
                    : type is "count" or "milliseconds"
                        ? new InsightCardValue(type, raw.GetProperty("value").GetUInt64(), null)
                        : throw new InvalidOperationException("insights-response-invalid");
            }
            string? missing = Optional(row, "missing_reason");
            if ((value == null) == (missing == null)) Invalid();
            if (missing != null && !Missing.Contains(missing)) Invalid();
            string id = Required(row, "id");
            string unit = Required(row, "unit");
            if (!Rows.Contains(id) || !Units.Contains(unit)) Invalid();
            return new InsightCardRow(id, unit, Optional(row, "label"), value, missing);
        }).ToArray();
        var coverage = card.GetProperty("coverage").EnumerateArray().Select(item => {
            string unit = Required(item, "unit");
            if (!Coverage.Contains(unit)) Invalid();
            return new InsightCardCoverage(unit, item.GetProperty("observed").GetUInt64(), item.GetProperty("eligible").GetUInt64());
        }).ToArray();
        InsightCardDenominator? denominator = null;
        if (card.GetProperty("episode_denominator").ValueKind != JsonValueKind.Null)
        {
            var item = card.GetProperty("episode_denominator");
            denominator = new(item.GetProperty("eligible").GetUInt64(), item.GetProperty("assessed").GetUInt64(),
                item.GetProperty("unassessed").GetUInt64(), item.GetProperty("explicit_unknown").GetUInt64());
        }
        var limitations = Strings(card.GetProperty("limitations"));
        if (limitations.Any(item => !Limitations.Contains(item))) Invalid();
        return new InsightQuestionCard(question, Required(card, "metric_version"), state, rows, coverage, denominator,
            Strings(card.GetProperty("evidence_ids")), Strings(card.GetProperty("episode_ids")), limitations,
            card.GetProperty("rows_omitted").GetBoolean());
    }

    private static string Required(JsonElement value, string name) =>
        value.GetProperty(name).GetString() ?? throw new InvalidOperationException("insights-response-invalid");
    private static string? Optional(JsonElement value, string name) =>
        value.GetProperty(name).ValueKind == JsonValueKind.Null ? null : Required(value, name);
    private static string[] Strings(JsonElement value) => value.EnumerateArray().Select(item =>
        item.GetString() ?? throw new InvalidOperationException("insights-response-invalid")).ToArray();
    private static void Invalid() => throw new InvalidOperationException("insights-response-invalid");
}
