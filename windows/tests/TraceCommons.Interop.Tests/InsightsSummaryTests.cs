using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using TraceCommons.App.ViewModels;
using Xunit;

namespace TraceCommons.Interop.Tests;

public sealed class InsightsSummaryTests
{
    internal const string Response = """
    {"type":"summary","summary":{
      "schema_version":1,"scope":"all_saved_selected_session_snapshots",
      "limitations":["assessments_are_user_reported","observed_sums_require_both_coverages","analysis_dates_are_not_activity_time"],
      "provider":{"id":"trace-commons-local","version":"1","rubric_version":"descriptive-counts-v1","execution_mode":"local","schema_version":1},
      "saved_snapshots":2,
      "snapshot_analysis_range":{"oldest":"2026-09-10T10:00:00Z","newest":"2026-09-11T10:00:00Z"},
      "user_reported":{"assessed_snapshots":1,"unassessed_snapshots":1,
        "categories":[{"category":"unknown","snapshots":1,"evidence_snapshot_ids":["snapshot-a"]}],
        "outcomes":[{"outcome":"unknown","snapshots":1,"evidence_snapshot_ids":["snapshot-a"]}]},
      "metrics":[
        {"id":"input_tokens","observed_value_sum":null,"available_snapshots":0,"missing_snapshots":2,
         "record_coverage":{"observed":0,"total":2},"coverage_unit":"session_snapshots","evidence_snapshot_ids":[]},
        {"id":"tool_calls","observed_value_sum":0,"available_snapshots":1,"missing_snapshots":1,
         "record_coverage":{"observed":1,"total":5},"coverage_unit":"normalized_events","evidence_snapshot_ids":["snapshot-a"]}],
      "snapshots":[
        {"id":"snapshot-a","source_format":"codex","analyzed_at":"2026-09-10T10:00:00Z","evidence":[{"id":"source-a","source_digest":"digest-a"}]},
        {"id":"snapshot-b","source_format":"trajectory","analyzed_at":"2026-09-11T10:00:00Z","evidence":[{"id":"source-b","source_digest":"digest-b"}]}]
    }}
    """;
    private static JsonElement Json(string value) => JsonDocument.Parse(value).RootElement.Clone();
    private sealed class Service : ILocalInsights
    {
        public readonly List<string> Operations = new();
        public string Summary = Response;
        public bool FailSummary;
        public TaskCompletionSource<JsonElement>? PendingSummary;
        public Task<JsonElement> CallAsync(object operation, CancellationToken cancellationToken)
        {
            var op = JsonSerializer.SerializeToElement(operation);
            string type = op.GetProperty("type").GetString()!;
            Operations.Add(type);
            if (type == "summary")
            {
                if (FailSummary) throw new InvalidOperationException("private-path-must-not-render");
                return PendingSummary?.Task ?? Task.FromResult(Json(Summary));
            }
            if (type == "copy") return Task.FromResult(Json("""
                {"type":"copy","copy":{"unknown":"Unknown","summary_unassessed":"Unassessed","summary_assessed":"Assessed",
                "summary_observed_sum":"Observed total","summary_available":"Available snapshots","summary_missing":"Missing snapshots",
                "summary_record_coverage":"Record coverage","summary_unit_normalized_events":"Normalized events","summary_unit_session_snapshots":"Session snapshots",
                "summary_analysis_range":"Analysis dates","category":"Category","outcome":"Outcome","category_unknown":"Unknown category",
                "outcome_unknown":"Unknown outcome","codex":"Codex rollout","error":"safe-error","summary_unavailable":"Unavailable"}}
                """));
            if (type == "list") return Task.FromResult(Json("{\"type\":\"list\",\"insights\":[" + InsightsTests.Insight + "]}"));
            if (type == "delete") return Task.FromResult(Json("{\"type\":\"delete\",\"deleted\":true}"));
            return Task.FromResult(Json("{\"type\":\"" + type + "\",\"insight\":" + InsightsTests.Insight + "}"));
        }
    }
    [Fact]
    public void TypedDecodePreservesUnknownKnownZeroBothCoveragesAndUnits()
    {
        var summary = InsightsSummaryResponse.Decode(Json(Response));
        Assert.Null(summary.Metrics[0].ObservedValueSum);
        Assert.Equal(0UL, summary.Metrics[1].ObservedValueSum);
        Assert.Equal(1UL, summary.Metrics[1].AvailableSnapshots);
        Assert.Equal(1UL, summary.Metrics[1].MissingSnapshots);
        Assert.Equal(1UL, summary.Metrics[1].RecordCoverage.Observed);
        Assert.Equal(5UL, summary.Metrics[1].RecordCoverage.Total);
        Assert.Equal("normalized_events", summary.Metrics[1].CoverageUnit);
        Assert.Equal(1UL, summary.UserReported.UnassessedSnapshots);
        Assert.Equal("unknown", summary.UserReported.Outcomes[0].Outcome);
        Assert.Equal("digest-a", summary.Snapshots[0].Evidence[0].SourceDigest);
        Assert.Throws<InvalidOperationException>(() => InsightsSummaryResponse.Decode(Json(Response.Replace("\"schema_version\":1", "\"schema_version\":2", StringComparison.Ordinal))));
        Assert.Throws<JsonException>(() => InsightsSummaryResponse.Decode(Json("{\"type\":\"summary\",\"summary\":{}}")));
    }
    [Fact]
    public async Task SummaryRendersEvidenceAndRefreshesAfterEverySavedMutationWithoutSourceReads()
    {
        var service = new Service();
        using var model = new InsightsViewModel(service);
        await model.LoadAsync();
        Assert.Equal(new[] { "copy", "list", "summary" }, service.Operations);
        Assert.Contains("Unassessed: 1", model.SummaryDetails);
        Assert.Contains("Analysis dates:", model.SummaryDetails);
        Assert.Contains(model.SummaryRows, row => row.Label == "Outcome · Unknown outcome" && row.Details == "1");
        Assert.Contains("Observed total: Unknown", model.SummaryRows[0].Details);
        Assert.Contains("Observed total: 0", model.SummaryRows[1].Details);
        Assert.Contains("Available snapshots: 1", model.SummaryRows[1].Details);
        Assert.Contains("Missing snapshots: 1", model.SummaryRows[1].Details);
        Assert.Contains("Record coverage: 1/5 · Normalized events", model.SummaryRows[1].Details);
        Assert.Equal("snapshot-a", model.SummaryRows[1].Evidence[0].Id);
        Assert.Contains("digest-a", model.SummaryRows[1].Evidence[0].Label);
        await model.ExplainSummaryEvidenceAsync("snapshot-a");
        Assert.Equal("snapshot-a", model.CurrentId);
        Assert.Equal("explain", service.Operations[^1]);
        await model.AnnotateAsync("docs", "accepted");
        Assert.Equal(new[] { "annotate", "summary" }, service.Operations.TakeLast(2));
        await model.ClearAnnotationAsync();
        Assert.Equal(new[] { "clear_annotation", "summary" }, service.Operations.TakeLast(2));
        await model.DeleteAsync();
        Assert.Equal(new[] { "delete", "list", "summary" }, service.Operations.TakeLast(3));
        await model.AnalyzeAsync("codex", "/explicit/file", true);
        Assert.Equal(new[] { "analyze", "list", "summary" }, service.Operations.TakeLast(3));
        await model.RefreshAsync();
        Assert.Equal(new[] { "list", "summary" }, service.Operations.TakeLast(2));
    }
    [Fact]
    public async Task FailedOrCancelledRefreshClearsStaleSummaryAndReentryRecovers()
    {
        var service = new Service();
        using var model = new InsightsViewModel(service);
        await model.LoadAsync();
        Assert.NotNull(model.Summary);
        service.FailSummary = true;
        await model.RefreshAsync();
        Assert.Null(model.Summary);
        Assert.Empty(model.SummaryDetails);
        Assert.Empty(model.SummaryRows);
        Assert.Equal("safe-error", model.Status);
        service.FailSummary = false;
        service.PendingSummary = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var refresh = model.RefreshAsync();
        model.Cancel();
        service.PendingSummary.SetResult(Json(Response));
        await refresh;
        Assert.Null(model.Summary);
        service.PendingSummary = null;
        await model.LoadAsync();
        Assert.NotNull(model.Summary);
    }
    [Fact]
    public async Task ClosingDuringSummaryReadDiscardsLateResponse()
    {
        var service = new Service();
        var model = new InsightsViewModel(service);
        await model.LoadAsync();
        service.PendingSummary = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var refresh = model.RefreshAsync();
        model.Dispose();
        int changes = 0;
        model.PropertyChanged += (_, _) => changes++;
        service.PendingSummary.SetResult(Json(Response));
        await refresh;
        Assert.Null(model.Summary);
        Assert.Empty(model.SummaryRows);
        Assert.Equal(0, changes);
    }

}
