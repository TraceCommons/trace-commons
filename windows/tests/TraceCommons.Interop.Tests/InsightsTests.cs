using System;
using System.Collections.Generic;
using System.IO;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using TraceCommons.App.ViewModels;
using Xunit;

namespace TraceCommons.Interop.Tests;

public sealed class InsightsTests
{
    private static JsonElement Json(string value) => JsonDocument.Parse(value).RootElement.Clone();
    internal const string Insight = """
        {"id":"snapshot-a","source_format":"codex","analyzed_at":"2026-09-11T10:00:00Z",
         "report":{"provider":{"id":"trace-commons-local","version":"1","rubric_version":"descriptive-counts-v1"},
         "metrics":[{"id":"input_tokens","value":null,"coverage":{"observed":0,"total":2}}],
         "evidence":[{"id":"source-a","source_digest":"abcdef"}]},"manual_annotation":null}
        """;
    private sealed class Service : ILocalInsights
    {
        public List<JsonElement> Calls { get; } = new();
        public TaskCompletionSource<JsonElement>? Pending;
        public string ListedInsights = Insight;
        public string ReturnedInsight = Insight;
        public Task<JsonElement> CallAsync(object operation, CancellationToken cancellationToken)
        {
            var request = JsonSerializer.SerializeToElement(operation);
            Calls.Add(request);
            string type = request.GetProperty("type").GetString()!;
            if (Pending != null) return Pending.Task;
            return Task.FromResult(type switch {
                "copy" => Json("""{"type":"copy","copy":{"unknown":"Unknown","coverage":"Coverage","metric_input_tokens":"Input tokens","error":"safe-error","codex":"Codex rollout"}}"""),
                "summary" => Json(InsightsSummaryTests.Response),
                "episode_list" => Json("{\"type\":\"episode_list\",\"episodes\":[]}"),
                "list" => Json("{\"type\":\"list\",\"insights\":[" + ListedInsights + "]}"),
                "delete" => Json("{\"type\":\"delete\",\"deleted\":true}"),
                _ => Json("{\"type\":\"" + type + "\",\"insight\":" + ReturnedInsight + "}")
            });
        }
    }

    [Fact]
    public async Task EphemeralAnalysisPreservesUnknownAndCoverageAndRequiresExplicitSave()
    {
        var service = new Service();
        using var model = new InsightsViewModel(service);
        await model.LoadAsync();
        await model.AnalyzeAsync("codex", "/selected/file", false);
        Assert.Null(model.CurrentId);
        Assert.Contains("Input tokens: Unknown", model.Details);
        Assert.Contains("Coverage 0/2", model.Details);
        Assert.Contains("abcdef", model.Details);
        Assert.False(service.Calls.FindLast(call => call.GetProperty("type").GetString() == "analyze").GetProperty("save").GetBoolean());
        await model.AnalyzeAsync("codex", "/selected/file", true);
        Assert.Equal("snapshot-a", model.CurrentId);
        await model.AnnotateAsync("tests", "accepted");
        Assert.Equal("annotate", service.Calls[^2].GetProperty("type").GetString());
        await model.ClearAnnotationAsync();
        Assert.Equal("clear_annotation", service.Calls[^2].GetProperty("type").GetString());
        await model.DeleteAsync();
        Assert.Null(model.CurrentId);
        Assert.Equal("", model.Details);
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task RefreshClearsSavedSnapshotDeletedOrReplacedByAnotherClient(bool replacement)
    {
        var service = new Service();
        using var model = new InsightsViewModel(service);
        await model.LoadAsync();
        Assert.StartsWith("Codex rollout", model.Saved[0].Label);
        await model.ExplainAsync("snapshot-a");
        Assert.True(model.HasSavedSelection);
        Assert.NotEmpty(model.Details);
        service.ListedInsights = replacement ? Insight.Replace("snapshot-a", "snapshot-b", StringComparison.Ordinal) : "";
        await model.RefreshAsync();
        Assert.Null(model.CurrentId);
        Assert.Empty(model.Details);
        Assert.False(model.HasSavedSelection);
        Assert.Equal(replacement ? 1 : 0, model.Saved.Count);
    }

    [Fact]
    public async Task RefreshPreservesEphemeralPreviewWhenSavedListChanges()
    {
        var service = new Service();
        using var model = new InsightsViewModel(service);
        await model.LoadAsync();
        await model.AnalyzeAsync("codex", "/selected/file", false);
        string preview = model.Details;
        service.ListedInsights = "";
        await model.RefreshAsync();
        Assert.Null(model.CurrentId);
        Assert.NotEmpty(preview);
        Assert.Equal(preview, model.Details);
    }

    [Fact]
    public async Task ExistingAssessmentPopulatesSelectorsAndClearsWithSnapshot()
    {
        string annotated = Insight.Replace("\"manual_annotation\":null", "\"manual_annotation\":{\"category\":\"docs\",\"outcome\":\"accepted\",\"recorded_at\":\"2026-09-11T10:00:00Z\"}", StringComparison.Ordinal);
        var service = new Service { ReturnedInsight = annotated };
        using var model = new InsightsViewModel(service);
        await model.LoadAsync();
        await model.ExplainAsync("snapshot-a");
        Assert.Equal(2, model.CategoryIndex);
        Assert.Equal(0, model.OutcomeIndex);
        await model.SaveAssessmentAsync();
        Assert.Equal("docs", service.Calls[^2].GetProperty("category").GetString());
        Assert.Equal("accepted", service.Calls[^2].GetProperty("outcome").GetString());
        service.ListedInsights = annotated.Replace("accepted", "partial", StringComparison.Ordinal);
        await model.RefreshAsync();
        Assert.Equal(1, model.OutcomeIndex);
        service.ReturnedInsight = Insight;
        await model.ClearAnnotationAsync();
        Assert.Equal(5, model.CategoryIndex);
        Assert.Equal(3, model.OutcomeIndex);
        model.CategoryIndex = 0;
        model.OutcomeIndex = 0;
        await model.AnalyzeAsync("codex", "/selected/file", false);
        Assert.Equal(5, model.CategoryIndex);
        Assert.Equal(3, model.OutcomeIndex);
    }

    [Fact]
    public async Task ReentryWaitsForCancelledInitialLoadThenReloads()
    {
        var completion = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        var service = new Service { Pending = completion };
        using var model = new InsightsViewModel(service);
        var first = model.LoadAsync();
        model.Cancel();
        var reentry = model.LoadAsync();
        Assert.Single(service.Calls);
        service.Pending = null;
        completion.SetResult(Json("{\"type\":\"copy\",\"copy\":{}}"));
        await Task.WhenAll(first, reentry);
        Assert.Equal(5, service.Calls.Count);
        Assert.Single(model.Saved);
        Assert.False(model.Busy);
    }

    [Fact]
    public async Task CloseDiscardsEvenServiceThatIgnoresCancellation()
    {
        var service = new Service { Pending = new(TaskCreationOptions.RunContinuationsAsynchronously) };
        var model = new InsightsViewModel(service);
        var operation = model.AnalyzeAsync("codex", "/selected/file", false);
        int updates = 0;
        model.PropertyChanged += (_, _) => updates++;
        model.Dispose();
        service.Pending.SetResult(Json("{\"type\":\"analyze\",\"insight\":" + Insight + "}"));
        await operation;
        Assert.Equal("", model.Details);
        Assert.Equal(0, updates);
    }

    [Fact]
    public async Task CancelCannotReplaceResultWithLateCompletion()
    {
        var service = new Service();
        using var model = new InsightsViewModel(service);
        await model.LoadAsync();
        service.Pending = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var operation = model.AnalyzeAsync("codex", "/selected/file", true);
        model.Cancel();
        service.Pending.SetResult(Json("{\"type\":\"analyze\",\"insight\":" + Insight + "}"));
        await operation;
        Assert.Equal("", model.Details);
        Assert.Null(model.CurrentId);
        Assert.False(model.Busy);
    }

    [Fact]
    public async Task NativeLocalServiceUsesSelectedFileAndSharedStoreWithoutEnrollment()
    {
        string root = Path.Combine(Path.GetTempPath(), "insights-native-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(root);
        try
        {
            string file = Path.Combine(root, "selected.jsonl");
            string store = Path.Combine(root, "store");
            await File.WriteAllTextAsync(file, "{\"type\":\"session_meta\",\"payload\":{}}\n{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"fixture\"}]}}\n");
            var service = new LocalInsights(store);
            var copy = await service.CallAsync(new { type = "copy" }, CancellationToken.None);
            Assert.Equal("Insights", copy.GetProperty("copy").GetProperty("title").GetString());
            var empty = await service.CallAsync(new { type = "list" }, CancellationToken.None);
            Assert.Empty(empty.GetProperty("insights").EnumerateArray());
            Assert.False(Directory.Exists(store));
            var emptySummary = InsightsSummaryResponse.Decode(await service.CallAsync(new { type = "summary" }, CancellationToken.None));
            Assert.Equal(0UL, emptySummary.SavedSnapshots);
            Assert.Null(emptySummary.SnapshotAnalysisRange);
            Assert.All(emptySummary.Metrics, metric => Assert.Null(metric.ObservedValueSum));
            Assert.False(Directory.Exists(store));
            await service.CallAsync(new { type = "analyze", source = "codex", file, save = false }, CancellationToken.None);
            Assert.False(Directory.Exists(store));
            var saved = await service.CallAsync(new { type = "analyze", source = "codex", file, save = true }, CancellationToken.None);
            string id = saved.GetProperty("insight").GetProperty("id").GetString()!;
            var annotated = await service.CallAsync(new { type = "annotate", id, category = "tests", outcome = "accepted" }, CancellationToken.None);
            Assert.Equal("user_reported", annotated.GetProperty("insight").GetProperty("manual_annotation").GetProperty("provenance").GetString());
            var summary = InsightsSummaryResponse.Decode(await service.CallAsync(new { type = "summary" }, CancellationToken.None));
            Assert.Equal(1UL, summary.SavedSnapshots);
            Assert.Equal(1UL, summary.UserReported.AssessedSnapshots);
            Assert.Equal(0UL, summary.UserReported.UnassessedSnapshots);
            Assert.Equal(id, Assert.Single(summary.Snapshots).Id);
            var deleted = await service.CallAsync(new { type = "delete", id }, CancellationToken.None);
            Assert.True(deleted.GetProperty("deleted").GetBoolean());
            Assert.Equal(0UL, InsightsSummaryResponse.Decode(await service.CallAsync(new { type = "summary" }, CancellationToken.None)).SavedSnapshots);
            Assert.True(File.Exists(file));
            var error = await Assert.ThrowsAsync<InvalidOperationException>(() => service.CallAsync(new { type = "secret-invalid-request" }, CancellationToken.None));
            Assert.DoesNotContain("secret", error.Message);
        }
        finally { Directory.Delete(root, true); }
    }

    [Fact]
    public void FirstActivationDoesNotStartDaemonOrAskForRoots()
    {
        var path = Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App", "MainWindow.xaml.cs.txt");
        string source = File.ReadAllText(path);
        int start = source.IndexOf("private void OnFirstActivated(", StringComparison.Ordinal);
        int end = source.IndexOf("private bool _contributionStartupAttempted", start, StringComparison.Ordinal);
        string activation = source[start..end];
        Assert.Contains("ShowInsightsPane();", activation);
        Assert.DoesNotContain("InitializeAsync", activation);
        Assert.DoesNotContain("ShowSessionRootsAsync", activation);
        Assert.DoesNotContain("ContinueStartupAsync", activation);
        Assert.Contains("await ViewModel.InitializeAsync();", source);
        Assert.Contains("if (ViewModel.NeedsSessionRoots)", source);
        Assert.Contains("await ShowSessionRootsAsync();", source);
    }
}
