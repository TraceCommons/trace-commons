using System;
using System.Collections.Generic;
using System.Linq;
using System.IO;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using TraceCommons.App.ViewModels;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

public sealed class InsightCardTests
{
    private const string Snapshot = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    private const string Episode = "11111111-1111-4111-8111-111111111111";
    private static JsonElement Json(string value) => JsonDocument.Parse(value).RootElement.Clone();
    private static string Card(string question, string state = "observed", string rows = "[]", string coverage = "[]",
        string evidence = "[]", string episodes = "[]", string limitations = "[]", string denominator = "null") => $$"""
        {"question":"{{question}}","metric_version":"{{question}}-v1","state":"{{state}}","rows":{{rows}},
         "coverage":{{coverage}},"episode_denominator":{{denominator}},"evidence_ids":{{evidence}},
         "episode_ids":{{episodes}},"limitations":{{limitations}},"rows_omitted":false}
        """;
    private static string Response => "{\"type\":\"question_cards\",\"result\":{\"schema_version\":1," +
        "\"provider\":{\"id\":\"trace-commons-first-party\",\"version\":\"1\",\"rubric_version\":\"deterministic-question-cards-v1\"}," +
        "\"input_digest\":\"" + new string('a', 64) + "\",\"cards\":[" +
        Card("recorded_activity", rows: "[{\"id\":\"saved_snapshots\",\"unit\":\"saved_snapshots\",\"label\":null,\"value\":{\"type\":\"count\",\"value\":1},\"missing_reason\":null}]",
            coverage: "[{\"unit\":\"saved_snapshots\",\"observed\":1,\"eligible\":1}]", evidence: "[\"" + Snapshot + "\"]",
            limitations: "[\"timestamps_are_record_span\"]") + "," +
        Card("episode_outcomes", evidence: "[\"" + Snapshot + "\"]", episodes: "[\"" + Episode + "\"]") + "," +
        Card("observed_models", state: "unavailable") + "," +
        Card("estimated_cost", state: "unavailable", rows: "[{\"id\":\"estimated_cost\",\"unit\":\"us_dollars\",\"label\":null,\"value\":null,\"missing_reason\":\"usage_not_persisted\"}]") +
        "]},\"text\":\"shared text\"}";

    private sealed class Service : ILocalInsights
    {
        public List<JsonElement> Calls { get; } = new();
        public TaskCompletionSource<JsonElement>? PendingCards;
        public Task<JsonElement> CallAsync(object operation, CancellationToken cancellationToken)
        {
            var request = JsonSerializer.SerializeToElement(operation);
            Calls.Add(request);
            string type = request.GetProperty("type").GetString()!;
            if (type == "question_cards" && PendingCards != null) return PendingCards.Task;
            return Task.FromResult(type switch {
                "copy" => Json("""{"type":"copy","copy":{"working":"Working","error":"Safe error","card_question_recorded_activity":"Recorded activity","card_question_episode_outcomes":"Your episode outcomes","card_question_observed_models":"Observed model labels","card_question_estimated_cost":"Estimated cost","card_state_observed":"Observed","card_state_unavailable":"Unavailable","card_row_saved_snapshots":"Saved snapshots","card_row_estimated_cost":"Estimated cost","card_coverage_saved_snapshots":"Saved snapshots","card_limitation_timestamps_are_record_span":"Record span caution","card_missing_usage_not_persisted":"Usage unavailable","card_evidence":"Saved evidence","card_episodes":"Episode evidence"}}"""),
                "list" => Json("{\"type\":\"list\",\"insights\":[]}"),
                "summary" => Json(InsightsSummaryTests.Response),
                "episode_list" => Json("{\"type\":\"episode_list\",\"episodes\":[]}"),
                "question_cards" => Json(Response),
                _ => throw new InvalidOperationException(type)
            });
        }
    }

    [Fact]
    public void TypedDecoderRejectsWrongQuestionOrderAndValueMissingness()
    {
        var result = InsightCardResponses.Decode(Json(Response));
        Assert.Equal(4, result.Cards.Count);
        Assert.Equal(1UL, result.Cards[0].Rows[0].Value!.UnsignedValue);
        Assert.Equal("usage_not_persisted", result.Cards[3].Rows[0].MissingReason);
        Assert.Throws<InvalidOperationException>(() => InsightCardResponses.Decode(Json(Response.Replace(
            "recorded_activity", "estimated_cost", StringComparison.Ordinal))));
        Assert.Throws<InvalidOperationException>(() => InsightCardResponses.Decode(Json(Response.Replace(
            "\"missing_reason\":null", "\"missing_reason\":\"no_observed_value\"", StringComparison.Ordinal))));
    }

    [Fact]
    public async Task ViewModelSendsExplicitCanonicalSelectionAndRendersOnlySharedRows()
    {
        var service = new Service();
        using var model = new InsightsViewModel(service);
        await model.LoadAsync();
        await model.LoadQuestionCardsAsync(new[] { Snapshot, Snapshot }, new[] { Episode, Episode });
        var call = service.Calls.Single(item => item.GetProperty("type").GetString() == "question_cards");
        Assert.Equal(Snapshot, Assert.Single(call.GetProperty("snapshot_ids").EnumerateArray()).GetString());
        Assert.Equal(Episode, Assert.Single(call.GetProperty("episode_ids").EnumerateArray()).GetString());
        Assert.Equal(4, call.GetProperty("questions").GetArrayLength());
        Assert.Equal("Recorded activity", model.QuestionCards[0].Question);
        Assert.Equal("Saved snapshots", model.QuestionCards[0].Rows[0].Label);
        Assert.Equal("1", model.QuestionCards[0].Rows[0].Value);
        Assert.Equal("Saved snapshots: 1 / 1", model.QuestionCards[0].Coverage);
        Assert.Equal("Record span caution", model.QuestionCards[0].Limitations);
        Assert.Equal("Usage unavailable", model.QuestionCards[3].Rows[0].Value);
        await model.OpenCardEvidenceAsync(Snapshot);
        Assert.Equal("explain", service.Calls[^1].GetProperty("type").GetString());
        await model.LoadQuestionCardsAsync(new[] { Snapshot }, new[] { Episode });
        await model.OpenCardEpisodeAsync(Episode);
        Assert.Equal("episode_explain", service.Calls[^1].GetProperty("type").GetString());
    }

    [Fact]
    public async Task EmptySelectionIsSentAsNoEvidence()
    {
        var service = new Service();
        using var model = new InsightsViewModel(service);
        await model.LoadAsync();
        await model.LoadQuestionCardsAsync(Array.Empty<string>(), Array.Empty<string>());
        var call = service.Calls.Single(item => item.GetProperty("type").GetString() == "question_cards");
        Assert.Empty(call.GetProperty("snapshot_ids").EnumerateArray());
        Assert.Empty(call.GetProperty("episode_ids").EnumerateArray());
    }

    [Fact]
    public async Task CancellationPreventsLateCardPresentationAndNavigationRequiresReturnedEvidence()
    {
        var service = new Service();
        using var model = new InsightsViewModel(service);
        await model.LoadAsync();
        service.PendingCards = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var load = model.LoadQuestionCardsAsync(new[] { Snapshot }, new[] { Episode });
        model.Cancel();
        service.PendingCards.SetResult(Json(Response));
        await load;
        Assert.Empty(model.QuestionCards);
        int calls = service.Calls.Count;
        await model.OpenCardEvidenceAsync(Snapshot);
        await model.OpenCardEpisodeAsync(Episode);
        Assert.Equal(calls, service.Calls.Count);
    }

    [Fact]
    public async Task NativeSavedSnapshotProjectsFourCardsThroughTheWindowsBridge()
    {
        string root = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "insights-cards-windows-" + Guid.NewGuid().ToString("N"));
        System.IO.Directory.CreateDirectory(root);
        try
        {
            string file = System.IO.Path.Combine(root, "source.jsonl");
            await System.IO.File.WriteAllTextAsync(file,
                "{\"role\":\"meta\",\"source\":\"claude-code\",\"model\":\"fixture-model\"}\n" +
                "{\"role\":\"user\",\"timestamp\":\"2026-09-11T12:00:00.123Z\",\"content\":\"private\"}\n" +
                "{\"role\":\"assistant\",\"timestamp\":\"2026-09-11T12:00:01.124Z\",\"content\":\"done\"}\n");
            var service = new LocalInsights(System.IO.Path.Combine(root, "store"));
            using var model = new InsightsViewModel(service);
            await model.LoadAsync();
            await model.AnalyzeAsync("trajectory", file, true);
            await model.LoadQuestionCardsAsync(new[] { model.CurrentId! }, Array.Empty<string>());
            Assert.Equal(4, model.QuestionCards.Count);
            Assert.Contains(model.QuestionCards[0].Rows, row => row.Label == model["card_row_record_span"] && row.Value.Contains("1,001", StringComparison.Ordinal));
            Assert.Contains(model.QuestionCards[2].Rows, row => row.Label.Contains("fixture-model", StringComparison.Ordinal));
            Assert.Contains(model["card_missing_pricing_unavailable"], model.QuestionCards[3].Rows[0].Value, StringComparison.Ordinal);
            var missing = await Assert.ThrowsAsync<InsightsServiceException>(() => service.CallAsync(new {
                type = "question_cards", questions = new[] { "recorded_activity" },
                snapshot_ids = new[] { new string('b', 64) }, episode_ids = Array.Empty<string>()
            }, CancellationToken.None));
            Assert.Equal("insights_card_snapshot_not_found", missing.Code);
        }
        finally { System.IO.Directory.Delete(root, true); }
    }

    [Fact]
    public void WindowsSurfaceUsesSeparateExplicitSelectionsAndReturnedEvidenceLinks()
    {
        string xaml = File.ReadAllText(Path.Combine(AppContext.BaseDirectory,
            "shell-source", "TraceCommons.App", "Controls", "InsightsView.xaml.txt"));
        string code = File.ReadAllText(Path.Combine(AppContext.BaseDirectory,
            "shell-source", "TraceCommons.App", "Controls", "InsightsView.xaml.cs.txt"));
        Assert.Contains("x:Name=\"CardSavedList\"", xaml);
        Assert.Contains("x:Name=\"CardEpisodeList\"", xaml);
        Assert.Contains("SelectionMode=\"Multiple\"", xaml);
        Assert.Contains("Header=\"{Binding [card_title]}\"", xaml);
        Assert.Contains("Text=\"{Binding [card_selection_notice]}\"", xaml);
        Assert.Contains("Text=\"{Binding [card_choose_evidence]}\"", xaml);
        Assert.Contains("Content=\"{Binding [card_update]}\"", xaml);
        Assert.Contains("ItemsSource=\"{Binding QuestionCards}\"", xaml);
        Assert.Contains("Click=\"OnCardEvidence\"", xaml);
        Assert.Contains("Click=\"OnCardEpisode\"", xaml);
        Assert.Contains("CardSavedList.SelectedItems.Cast<SavedInsight>()", code);
        Assert.Contains("CardEpisodeList.SelectedItems.Cast<EpisodeRow>()", code);
    }
}
