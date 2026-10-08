using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
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

    [Fact]
    public void NonRootsStartFailureNavigatesToQueueSoStatusTextIsVisible()
    {
        var path = Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App", "MainWindow.xaml.cs.txt");
        string source = File.ReadAllText(path);
        int start = source.IndexOf("private async Task<bool> StartContributionsAsync()", StringComparison.Ordinal);
        int end = source.IndexOf("private async Task ContinueStartupAsync()", start, StringComparison.Ordinal);
        string method = source[start..end];
        int rootsCheck = method.IndexOf("if (ViewModel.NeedsSessionRoots)", StringComparison.Ordinal);
        int rootsBranchEnd = method.IndexOf("return false;", rootsCheck, StringComparison.Ordinal);
        string afterRootsBranch = method[rootsBranchEnd..];
        // The session-roots-undeclared branch must remain unchanged: it
        // shows the roots screen, not Queue.
        Assert.DoesNotContain("ViewModel.ShowQueue()", method[..rootsBranchEnd]);
        // Any other start failure (for example "another instance may already
        // be running") must navigate to Queue so ViewModel.StatusText, which
        // is only rendered in the Queue pane's header chip, reaches the
        // contributor regardless of which pane the click came from.
        Assert.Contains("ViewModel.ShowQueue();", afterRootsBranch);
    }
}

/// <summary>
/// The Overview, Patterns and Sessions tabs. The words are the core's, read
/// through the real <c>copy</c> operation; the week responses are the shared
/// fixtures the local service produced, so no Claude session is saved here
/// and no digest key is ever asked for.
/// </summary>
public sealed class InsightsAnalyticsTests
{
    private static JsonElement Fixture(string name) => JsonDocument.Parse(
        File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "insights-analytics", name))).RootElement.Clone();

    private static readonly Lazy<Dictionary<string, string>> CoreCopy = new(() =>
    {
        var copy = new LocalInsights(Path.Combine(Path.GetTempPath(), "insights-copy-" + Guid.NewGuid().ToString("N")))
            .CallAsync(new { type = "copy" }, CancellationToken.None).GetAwaiter().GetResult();
        var words = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (var pair in copy.GetProperty("copy").EnumerateObject()) words[pair.Name] = pair.Value.GetString()!;
        return words;
    });
    private static string W(string key) => CoreCopy.Value[key];
    private static string Fill(string key, params (string, string)[] holes) => InsightsAnalyticsWords.Fill(W(key), holes);
    private static string N(ulong value) => value.ToString("N0", CultureInfo.CurrentCulture);

    private const string SavedList = """
        {"type":"list","insights":[
         {"id":"7eaa545c58cb9081a266131b5d3b46b8e5c7c0b6f08f30b0f29dd622cbd66199","source_format":"codex","analyzed_at":"2026-09-15T10:00:00Z",
          "report":{"provider":{"id":"trace-commons-local","version":"1","rubric_version":"descriptive-counts-v1"},"metrics":[],"evidence":[]},"manual_annotation":null},
         {"id":"f2f7dee53b071ed4bc01ed45e065eccd8f494c23ca6fe06852b2036071258d01","source_format":"claude_code","analyzed_at":"2026-09-16T10:00:00Z",
          "report":{"provider":{"id":"trace-commons-local","version":"1","rubric_version":"descriptive-counts-v1"},"metrics":[],"evidence":[]},"manual_annotation":null}]}
        """;

    private sealed class Service : ILocalInsights
    {
        public List<JsonElement> Calls { get; } = new();
        public string Overview = "week_overview.json";
        public string Patterns = "patterns.json";
        public HashSet<string> Failing { get; } = new(StringComparer.Ordinal);
        public Task<JsonElement> CallAsync(object operation, CancellationToken cancellationToken)
        {
            var request = JsonSerializer.SerializeToElement(operation);
            Calls.Add(request);
            string type = request.GetProperty("type").GetString()!;
            if (Failing.Contains(type)) throw new InvalidOperationException("insights-operation-failed");
            return Task.FromResult(type switch {
                "copy" => JsonSerializer.SerializeToElement(new { type = "copy", copy = CoreCopy.Value }),
                "list" => JsonDocument.Parse(SavedList).RootElement.Clone(),
                "summary" => JsonDocument.Parse(InsightsSummaryTests.Response).RootElement.Clone(),
                "episode_list" => JsonDocument.Parse("{\"type\":\"episode_list\",\"episodes\":[]}").RootElement.Clone(),
                "week_overview" => Fixture(Overview),
                "card_inputs" => Fixture(request.GetProperty("card").GetString() == "cache_share"
                    ? "card_inputs_cache_share.json" : "card_inputs_tokens.json"),
                "patterns" => Fixture(Patterns),
                "pattern_sessions" => Fixture("pattern_sessions.json"),
                "session_drill" => Fixture(request.GetProperty("snapshot_id").GetString()!.StartsWith("7eaa", StringComparison.Ordinal)
                    ? "session_drill_codex.json" : "session_drill_claude.json"),
                _ => throw new InvalidOperationException("insights-operation-failed")
            });
        }
        public JsonElement Last(string type) => Calls.FindLast(call => call.GetProperty("type").GetString() == type);
        public int Count(string type) => Calls.FindAll(call => call.GetProperty("type").GetString() == type).Count;
    }

    private static async Task<(InsightsViewModel, Service)> Loaded()
    {
        var service = new Service();
        var model = new InsightsViewModel(service);
        await model.LoadAsync();
        return (model, service);
    }

    [Fact]
    public async Task NothingIsReadForATabUntilOneIsShown()
    {
        var (model, service) = await Loaded();
        using var _ = model;
        Assert.Equal(0, service.Count("week_overview"));
        Assert.Equal(0, service.Count("patterns"));
        Assert.Equal(0, service.Count("session_drill"));
        Assert.Equal(3, InsightsViewModel.AnalyzeTab);
    }

    [Fact]
    public async Task OverviewWordsTheSavedWeekFromTheCoreAndNeverSumsHarnesses()
    {
        var (model, service) = await Loaded();
        using var _ = model;
        await model.SelectTabAsync(InsightsViewModel.OverviewTab);
        var request = service.Last("week_overview");
        Assert.Equal((int)TimeZoneInfo.Local.GetUtcOffset(DateTime.UtcNow).TotalSeconds, request.GetProperty("tz").GetInt32());
        Assert.Equal(JsonValueKind.Null, request.GetProperty("week_start").ValueKind);
        var overview = model.Overview;
        Assert.False(overview.Failed);
        Assert.Equal(Fill("analytics_coverage_line", ("k", "1"), ("n", "2"), ("p", "1"), ("u", "0")), overview.CoverageLine);
        Assert.Equal(new[] { W("analytics_feed_saved"), W("analytics_feed_comparisons_need_counter_pass") }, overview.Notes);
        // One line per harness, never a total.
        Assert.Equal(2, overview.TokenLines.Count);
        Assert.Equal(N(37235), overview.TokenLines[0].Figure);
        Assert.Equal(W("analytics_source_claude_code"), overview.TokenLines[0].Line);
        Assert.Equal(W("analytics_unavailable"), overview.TokenLines[0].Change);
        Assert.Equal(W("analytics_source_codex"), overview.TokenLines[1].Line);
        Assert.Equal(Fill("analytics_cache_share_line", ("source", W("claude_code")), ("p", "66")), overview.CacheLines[0].Line);
        // No cache share for Codex: the dash and an empty bar, never 0%.
        Assert.Equal(Fill("analytics_cache_share_line", ("source", W("codex")), ("p", W("analytics_unavailable"))), overview.CacheLines[1].Line);
        Assert.Equal(0, overview.CacheLines[1].Length);
        Assert.Equal("2", overview.SessionsFigure);
        Assert.Equal(W("claude_code") + " · " + Fill("analytics_largest", ("t", N(37235))), overview.LargestLines[0]);
        Assert.Equal(7, overview.Days.Count);
        Assert.Equal(4, overview.Days[0].Bars.Count);
        Assert.Equal(W("analytics_series_cache_read"), overview.Days[0].Bars[1].Label);
        Assert.Equal(N(24500), overview.Days[0].Bars[1].Figure);
        Assert.Equal(W("analytics_codex_interval"), overview.CodexIntervalLabel);
        Assert.Equal("0", overview.CodexIntervalFigure);
        Assert.Equal("claude-fixture-model", Assert.Single(overview.Models).Label);
        Assert.Equal(W("analytics_unavailable"), overview.ProjectFigure);
        Assert.Equal(W("analytics_by_project_unavailable"), overview.ProjectNote);
        Assert.Equal(new[] { W("claude_code"), W("codex") }, overview.Tools.Select(tool => tool.Label));
        Assert.Equal("2026-09-14 – 2026-09-20", Assert.Single(overview.Weeks).Label);
        Assert.Equal(0, overview.SelectedWeekIndex);
    }

    [Fact]
    public async Task AnOverviewCardOpensWhatMakesUpItsNumberAndASecondPressClosesIt()
    {
        var (model, service) = await Loaded();
        using var _ = model;
        await model.SelectTabAsync(InsightsViewModel.OverviewTab);
        await model.Overview.ToggleInputsAsync("tokens");
        var request = service.Last("card_inputs");
        Assert.Equal("tokens", request.GetProperty("card").GetString());
        Assert.Equal("2026-09-14", request.GetProperty("week_start").GetString());
        Assert.Equal("tokens", model.Overview.OpenCard);
        Assert.Equal(W("analytics_drill_title"), model.Overview.InputsTitle);
        Assert.Equal(new AnalyticsTableRow(W("analytics_drill_session"), W("analytics_drill_tokens"),
            W("analytics_drill_coverage"), W("analytics_drill_reason")), model.Overview.InputsHeader);
        Assert.Equal(2, model.Overview.Inputs.Count);
        Assert.Equal(N(37235), model.Overview.Inputs[0].Tokens);
        Assert.Equal(W("analytics_state_partial"), model.Overview.Inputs[0].Coverage);
        Assert.Equal(W("analytics_reason_some_turns_unknown"), model.Overview.Inputs[0].Reason);
        Assert.StartsWith(W("claude_code"), model.Overview.Inputs[0].Session);
        await model.Overview.ToggleInputsAsync("cache_share");
        Assert.Equal(N(24500) + " / " + N(37115), model.Overview.Inputs[0].Tokens);
        Assert.Equal(W("analytics_unavailable"), model.Overview.Inputs[1].Tokens);
        int calls = service.Count("card_inputs");
        await model.Overview.ToggleInputsAsync("cache_share");
        Assert.Null(model.Overview.OpenCard);
        Assert.Null(model.Overview.InputsHeader);
        Assert.Empty(model.Overview.Inputs);
        Assert.Equal(calls, service.Count("card_inputs"));
    }

    [Fact]
    public async Task AFailedWeekReadClearsTheFiguresAndShowsTheDash()
    {
        var (model, service) = await Loaded();
        using var _ = model;
        await model.SelectTabAsync(InsightsViewModel.OverviewTab);
        Assert.NotEmpty(model.Overview.TokenLines);
        service.Failing.Add("week_overview");
        await model.Overview.SelectWeekAsync(0);
        Assert.True(model.Overview.Failed);
        Assert.Empty(model.Overview.TokenLines);
        Assert.Equal("", model.Overview.CoverageLine);
        Assert.Equal(W("analytics_unavailable"), model.Overview.Status);
    }

    [Fact]
    public async Task AnEmptyWeekIsUnknownNotZero()
    {
        var (model, service) = await Loaded();
        using var _ = model;
        service.Overview = "week_overview_empty.json";
        service.Patterns = "patterns_empty.json";
        await model.SelectTabAsync(InsightsViewModel.OverviewTab);
        Assert.Empty(model.Overview.TokenLines);
        Assert.Equal(W("analytics_unavailable"), model.Overview.TokensEmpty);
        Assert.Equal(new[] { "2026-10-05", "2026-09-14" }, model.Overview.Weeks.Select(week => week.Value));
        await model.SelectTabAsync(InsightsViewModel.PatternsTab);
        Assert.All(model.Patterns.Cards, card => Assert.Equal(W("analytics_unavailable"), card.Headline));
        Assert.All(model.Patterns.Cards, card => Assert.True(card.Weeks[5].Gap));
    }

    [Fact]
    public async Task PatternsLeadWithTokensLabelInferenceAndLeaveGapsForAbsentWeeks()
    {
        var (model, service) = await Loaded();
        using var _ = model;
        await model.SelectTabAsync(InsightsViewModel.PatternsTab);
        var patterns = model.Patterns;
        Assert.Equal(JsonValueKind.Number, service.Last("patterns").GetProperty("tz").ValueKind);
        Assert.Equal(new[] { W("analytics_pattern_repeated_reads"), W("analytics_pattern_retried_calls"),
            W("analytics_pattern_edit_fail_edit"), W("analytics_pattern_long_context") }, patterns.Cards.Select(card => card.Title));
        Assert.Equal("0", patterns.Cards[0].Headline);
        Assert.Equal(Fill("analytics_pattern_repeated_reads_count", ("r", "0"), ("f", "0")), patterns.Cards[0].CountLine);
        Assert.Equal(W("analytics_unavailable"), patterns.Cards[3].Headline);
        Assert.Equal(Fill("analytics_pattern_long_context_line", ("threshold", N(200000))), patterns.Cards[3].CountLine);
        Assert.Equal(W("analytics_inferred_from_order") + "\n" + W("analytics_estimate_from_result_size"), patterns.Cards[2].Basis);
        Assert.Equal(W("analytics_unavailable"), patterns.Cards[0].Change);
        // Six weekly marks, oldest first; an absent week is a gap.
        Assert.Equal(new[] { true, true, true, true, true, false }, patterns.Cards[0].Weeks.Select(week => week.Gap));
        Assert.All(patterns.Cards[3].Weeks, week => Assert.True(week.Gap));
        Assert.False(patterns.Cards[0].HasSessions);
        Assert.Equal(Fill("analytics_claude_sessions_only", ("k", "1"), ("n", "2")), patterns.ClaudeOnlyLine);
        Assert.Equal(new[] { Fill("analytics_file_label", ("letter", "A"), ("ext", ".rs")), Fill("analytics_file_label_no_ext", ("letter", "B")) },
            patterns.Reread.Select(row => row.File));
        Assert.Equal(W("analytics_unavailable"), patterns.Reread[1].Tokens);
        Assert.Equal(W("analytics_reread_after_shrink"), patterns.RereadHeader!.AfterShrink);
        Assert.Equal("", patterns.RereadEmpty);
        Assert.Equal(new[] { W("analytics_patterns_intro"), W("analytics_patterns_overlap"),
            Fill("analytics_coverage_line", ("k", "1"), ("n", "2"), ("p", "1"), ("u", "0")),
            W("analytics_feed_saved"), W("analytics_feed_comparisons_need_counter_pass") }, patterns.Notes);
        await patterns.ToggleSessionsAsync("repeated_reads");
        Assert.Equal("repeated_reads", service.Last("pattern_sessions").GetProperty("pattern").GetString());
        Assert.Equal(W("analytics_pattern_repeated_reads"), patterns.SessionsTitle);
        var row = Assert.Single(patterns.Sessions);
        Assert.Equal(W("analytics_unavailable"), row.Tokens);
        Assert.Equal(W("analytics_state_partial"), row.Coverage);
    }

    [Fact]
    public async Task SessionsFollowTheSavedListNewestFirstAndDrawNoUnknownTurn()
    {
        var (model, service) = await Loaded();
        using var _ = model;
        await model.SelectTabAsync(InsightsViewModel.SessionsTab);
        var sessions = model.Sessions;
        Assert.Equal(new[] { "f2f7dee53b071ed4bc01ed45e065eccd8f494c23ca6fe06852b2036071258d01",
            "7eaa545c58cb9081a266131b5d3b46b8e5c7c0b6f08f30b0f29dd622cbd66199" }, sessions.Choices.Select(choice => choice.Value));
        Assert.Equal(0, sessions.SelectedIndex);
        Assert.StartsWith("f2f7", service.Last("session_drill").GetProperty("snapshot_id").GetString());
        Assert.Equal(Fill("analytics_session_header", ("date", "2026-09-14"), ("harness", W("claude_code")), ("n", "6"),
            ("t", N(49869)), ("span", "0:08")), sessions.Header);
        Assert.Equal(new[] { W("analytics_feed_saved"), W("analytics_reason_some_turns_unknown") }, sessions.Notes);
        Assert.Equal("", sessions.UnavailableLine);
        Assert.Equal(6, sessions.Turns.Count);
        Assert.Equal(Fill("analytics_turn", ("n", "0")), sessions.Turns[0].Label);
        Assert.Equal(new[] { N(12000), N(5), N(500) }, sessions.Turns[1].Bars.Select(bar => bar.Figure));
        // A turn with an unknown counter draws nothing, never a zero bar.
        Assert.Empty(sessions.Turns[3].Bars);
        Assert.Equal(W("analytics_unavailable"), sessions.Turns[3].Gap);
        Assert.Equal("A", sessions.Turns[2].Letters);
        Assert.Equal("D E", sessions.Turns[5].Letters);
        Assert.Equal(5, sessions.Markers.Count);
        Assert.Equal(Fill("analytics_marker_cache_rewrite", ("m", "6")), sessions.Markers[0].Title);
        Assert.Equal(Fill("analytics_marker_cache_rewrite_detail", ("t", "2"), ("x", N(12600))) + "\n" + W("analytics_marker_inferred"),
            sessions.Markers[0].Details);
        Assert.Equal(Fill("analytics_marker_reread", ("letter", "B")), sessions.Markers[3].Title);
        Assert.Contains(Fill("analytics_file_label", ("letter", "B"), ("ext", ".rs")), sessions.Markers[3].Details);
        Assert.EndsWith(W("analytics_marker_from_tool_calls"), sessions.Markers[3].Details);

        await sessions.SelectAsync(1);
        Assert.StartsWith("7eaa", service.Last("session_drill").GetProperty("snapshot_id").GetString());
        Assert.Equal(W("analytics_codex_not_recorded"), sessions.UnavailableLine);
        Assert.Empty(sessions.Turns);
        Assert.Empty(sessions.Markers);
    }

    [Fact]
    public async Task ASavedListChangeRereadsTheTabOnScreenOnly()
    {
        var (model, service) = await Loaded();
        using var _ = model;
        await model.SelectTabAsync(InsightsViewModel.OverviewTab);
        int overview = service.Count("week_overview");
        await model.RefreshAsync();
        // The same saved list: nothing to re-read.
        Assert.Equal(overview, service.Count("week_overview"));
        await model.SelectTabAsync(InsightsViewModel.AnalyzeTab);
        Assert.Equal(overview, service.Count("week_overview"));
        Assert.Equal(0, service.Count("patterns"));
    }

    [Fact]
    public async Task AnEmptyStoreReadsAsUnknownAndIsNotCreated()
    {
        string root = Path.Combine(Path.GetTempPath(), "insights-analytics-" + Guid.NewGuid().ToString("N"));
        string store = Path.Combine(root, "store");
        using var model = new InsightsViewModel(new LocalInsights(store));
        await model.LoadAsync();
        await model.SelectTabAsync(InsightsViewModel.OverviewTab);
        Assert.False(model.Overview.Failed);
        Assert.Empty(model.Overview.TokenLines);
        Assert.Equal(W("analytics_unavailable"), model.Overview.TokensEmpty);
        Assert.Equal(Fill("analytics_coverage_line", ("k", "0"), ("n", "0"), ("p", "0"), ("u", "0")), model.Overview.CoverageLine);
        await model.SelectTabAsync(InsightsViewModel.PatternsTab);
        Assert.False(model.Patterns.Failed);
        Assert.All(model.Patterns.Cards, card => Assert.Equal(W("analytics_unavailable"), card.Headline));
        await model.SelectTabAsync(InsightsViewModel.SessionsTab);
        Assert.Empty(model.Sessions.Choices);
        Assert.Equal(W("analytics_unavailable"), model.Sessions.Status);
        Assert.False(Directory.Exists(store));
    }

    [Fact]
    public async Task TheAnalyticsReadsRefuseWithTheirOwnSafeCodes()
    {
        string store = Path.Combine(Path.GetTempPath(), "insights-analytics-" + Guid.NewGuid().ToString("N"), "store");
        var service = new LocalInsights(store);
        var missing = await Assert.ThrowsAsync<InsightsServiceException>(() =>
            service.CallAsync(new { type = "session_drill", snapshot_id = "absent", tz = 0 }, CancellationToken.None));
        Assert.Equal("insights_not_found", missing.Code);
        var tz = await Assert.ThrowsAsync<InsightsServiceException>(() =>
            service.CallAsync(new { type = "week_overview", tz = 19 * 3600 }, CancellationToken.None));
        Assert.Equal("insights_tz_invalid", tz.Code);
        var weeks = await Assert.ThrowsAsync<InsightsServiceException>(() =>
            service.CallAsync(new { type = "patterns", tz = 0, weeks = 7 }, CancellationToken.None));
        Assert.Equal("insights_weeks_invalid", weeks.Code);
        Assert.False(Directory.Exists(store));
    }

    [Fact]
    public void TheWindowShowsTheFourTabsAndSpendDisabledWithCoreWordsOnly()
    {
        string xaml = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App", "Controls", "InsightsView.xaml.txt"));
        string code = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App", "Controls", "InsightsView.xaml.cs.txt"));
        int overview = xaml.IndexOf("Header=\"{Binding [analytics_tab_overview]}\"", StringComparison.Ordinal);
        int patterns = xaml.IndexOf("Header=\"{Binding [analytics_tab_patterns]}\"", StringComparison.Ordinal);
        int sessions = xaml.IndexOf("Header=\"{Binding [analytics_tab_sessions]}\"", StringComparison.Ordinal);
        int analyze = xaml.IndexOf("Header=\"{Binding [analytics_tab_analyze]}\"", StringComparison.Ordinal);
        Assert.True(overview >= 0 && overview < patterns && patterns < sessions && sessions < analyze);
        Assert.Contains("{Binding [analytics_tab_spend]}", xaml);
        Assert.Contains("{Binding [analytics_later]}", xaml);
        Assert.DoesNotContain("ProgressRing", xaml);
        Assert.Contains("ViewModel.SelectTabAsync(", code);
    }
}
