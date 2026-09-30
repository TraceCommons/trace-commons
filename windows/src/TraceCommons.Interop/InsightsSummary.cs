using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>Shared derived-store summary. No shell-side aggregation or model inference.</summary>
public sealed class InsightsSummaryResponse
{
    private static readonly HashSet<string> RequiredLimitations = new(StringComparer.Ordinal) {
        "selected_saved_sessions_are_not_verified_tasks", "assessments_are_user_reported",
        "observed_sums_require_both_coverages", "analysis_dates_are_not_activity_time",
        "source_formats_are_not_model_identity", "no_model_rankings_time_savings_or_cost"
    };
    private static readonly HashSet<string> CoverageUnits = new(StringComparer.Ordinal) {
        "session_snapshots", "normalized_events", "tool_results"
    };
    public required string Type { get; init; }
    public required SavedInsightsSummary Summary { get; init; }

    public static SavedInsightsSummary Decode(JsonElement response)
    {
        var value = response.Deserialize<InsightsSummaryResponse>(new JsonSerializerOptions {
            PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower
        });
        if (value?.Type != "summary" || value.Summary == null || value.Summary.SchemaVersion != 1 ||
            value.Summary.Scope != "all_saved_selected_session_snapshots")
            throw new InvalidOperationException("insights-response-invalid");
        if (value.Summary.Limitations == null || !RequiredLimitations.SetEquals(value.Summary.Limitations) ||
            value.Summary.Metrics == null || value.Summary.Metrics.Any(metric => metric == null || !CoverageUnits.Contains(metric.CoverageUnit)))
            throw new InvalidOperationException("insights-response-invalid");
        return value.Summary;
    }
}
public sealed class SavedInsightsSummary
{
    public required uint SchemaVersion { get; init; }
    public required string Scope { get; init; }
    public required string[] Limitations { get; init; }
    public required SummaryProvider Provider { get; init; }
    public required ulong SavedSnapshots { get; init; }
    public required SnapshotAnalysisRange? SnapshotAnalysisRange { get; init; }
    public required UserReportedSummary UserReported { get; init; }
    public required SummaryMetric[] Metrics { get; init; }
    public required SummarySnapshot[] Snapshots { get; init; }
}
public sealed class SummaryProvider
{
    public required string Id { get; init; }
    public required string Version { get; init; }
    public required string RubricVersion { get; init; }
    public required string ExecutionMode { get; init; }
    public required uint SchemaVersion { get; init; }
}
public sealed class SnapshotAnalysisRange
{
    public required DateTimeOffset Oldest { get; init; }
    public required DateTimeOffset Newest { get; init; }
}
public sealed class UserReportedSummary
{
    public required ulong AssessedSnapshots { get; init; }
    public required ulong UnassessedSnapshots { get; init; }
    public required SummaryCategory[] Categories { get; init; }
    public required SummaryOutcome[] Outcomes { get; init; }
}
public sealed class SummaryCategory
{
    public required string Category { get; init; }
    public required ulong Snapshots { get; init; }
    public required string[] EvidenceSnapshotIds { get; init; }
}
public sealed class SummaryOutcome
{
    public required string Outcome { get; init; }
    public required ulong Snapshots { get; init; }
    public required string[] EvidenceSnapshotIds { get; init; }
}
public sealed class SummaryMetric
{
    public required string Id { get; init; }
    public required ulong? ObservedValueSum { get; init; }
    public required ulong AvailableSnapshots { get; init; }
    public required ulong MissingSnapshots { get; init; }
    public required SummaryCoverage RecordCoverage { get; init; }
    public required string CoverageUnit { get; init; }
    public required string[] EvidenceSnapshotIds { get; init; }
}
public sealed class SummaryCoverage
{
    public required ulong Observed { get; init; }
    public required ulong Total { get; init; }
}
public sealed class SummarySnapshot
{
    public required string Id { get; init; }
    public required string SourceFormat { get; init; }
    public required DateTimeOffset AnalyzedAt { get; init; }
    public required SummaryEvidence[] Evidence { get; init; }
}
public sealed class SummaryEvidence
{
    public required string Id { get; init; }
    public required string SourceDigest { get; init; }
}
