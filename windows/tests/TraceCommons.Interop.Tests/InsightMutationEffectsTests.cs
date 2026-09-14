using System;
using System.IO;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using TraceCommons.App.ViewModels;
using Xunit;

namespace TraceCommons.Interop.Tests;

public sealed class InsightMutationEffectsTests
{
    private static JsonElement Json(string value) => JsonDocument.Parse(value).RootElement.Clone();
    private sealed class Service : ILocalInsights
    {
        public bool Legacy;
        public bool FailMutation;
        public bool FailRefresh;
        public TaskCompletionSource<JsonElement>? Pending;
        public Task<JsonElement> CallAsync(object operation, CancellationToken token)
        {
            string type = JsonSerializer.SerializeToElement(operation).GetProperty("type").GetString()!;
            if (type is "analyze" or "delete")
            {
                if (FailMutation) throw new InvalidOperationException("synthetic-mutation-failure");
                if (Pending != null) return Pending.Task;
            }
            if (type == "list" && FailRefresh) throw new InvalidOperationException("synthetic-refresh-failure");
            return Task.FromResult(type switch
            {
                "copy" => Json("""{"copy":{"episode_invalidated_notice":"Episode removed","error":"safe-error"}}"""),
                "summary" => Json(InsightsSummaryTests.Response),
                "episode_list" => Json("{\"type\":\"episode_list\",\"episodes\":[]}"),
                "list" => Json("{\"insights\":[" + InsightsTests.Insight + "]}"),
                _ => Mutation(type, Legacy)
            });
        }
        public static JsonElement Mutation(string type, bool legacy = false) => Json(
            "{\"type\":\"" + type + "\",\"deleted\":true,\"insight\":" + InsightsTests.Insight +
            (legacy ? "" : ",\"mutation_effects\":{\"invalidated_episode_ids\":[\"episode-a\",\"episode-b\"]}") + "}");
    }

    [Fact]
    public void DecodingPreservesIdsAndDefaultsLegacyAbsenceToEmpty()
    {
        Assert.Empty(InsightMutationEffects.Decode(Json("{}")).InvalidatedEpisodeIds);
        Assert.Empty(InsightMutationEffects.Decode(Json("""{"mutation_effects":{"invalidated_episode_ids":[]}}""")).InvalidatedEpisodeIds);
        Assert.Equal(new[] { "episode-a", "episode-b" }, InsightMutationEffects.Decode(Service.Mutation("delete")).InvalidatedEpisodeIds);
        Assert.Throws<JsonException>(() => InsightMutationEffects.Decode(Json("""{"mutation_effects":{"invalidated_episode_ids":[null]}}""")));
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task MutationNoticeSurvivesInternalRefreshAndClearsOnNewAction(bool delete)
    {
        var service = new Service();
        using var model = new InsightsViewModel(service);
        await model.LoadAsync();
        await model.ExplainAsync("snapshot-a");
        if (delete) await model.DeleteAsync();
        else await model.AnalyzeAsync("codex", "/synthetic", true);
        Assert.Equal("Episode removed\nepisode-a\nepisode-b", model.MutationNotice);
        await model.RefreshAsync();
        Assert.Empty(model.MutationNotice);
        service.Legacy = true;
        await model.AnalyzeAsync("codex", "/synthetic", true);
        Assert.Empty(model.MutationNotice);
    }

    [Fact]
    public async Task CompletedMutationEffectsRemainVisibleWhenRefreshFails()
    {
        var service = new Service();
        using var model = new InsightsViewModel(service);
        await model.LoadAsync();
        service.FailRefresh = true;
        await model.AnalyzeAsync("codex", "/synthetic", true);
        Assert.Contains("episode-a", model.MutationNotice);
        Assert.Equal("safe-error", model.Status);
        service.FailMutation = true;
        await model.AnalyzeAsync("codex", "/synthetic", true);
        Assert.Empty(model.MutationNotice);
    }

    [Fact]
    public async Task CancellationAndCloseClearNoticesAndSuppressLateMutationResponses()
    {
        var service = new Service();
        using var model = new InsightsViewModel(service);
        await model.LoadAsync();
        await model.AnalyzeAsync("codex", "/synthetic", true);
        model.Cancel();
        Assert.Empty(model.MutationNotice);
        service.Pending = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var pending = model.AnalyzeAsync("codex", "/synthetic", true);
        model.Dispose();
        service.Pending.SetResult(Service.Mutation("analyze"));
        await pending;
        Assert.Empty(model.MutationNotice);
    }

    [Fact]
    public async Task NativeSnapshotDeletionDisclosesInvalidatedEpisodeAndRefreshesSummary()
    {
        string root = Path.Combine(Path.GetTempPath(), "insights-effects-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(root);
        try
        {
            string file = Path.Combine(root, "selected.jsonl");
            await File.WriteAllTextAsync(file, "{\"type\":\"session_meta\",\"payload\":{}}\n{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"fixture\"}]}}\n");
            var service = new LocalInsights(Path.Combine(root, "store"));
            using var model = new InsightsViewModel(service);
            await model.LoadAsync();
            await model.AnalyzeAsync("codex", file, true);
            Assert.NotNull(model.CurrentId);
            var created = await service.CallAsync(new { type = "episode_create", snapshot_ids = new[] { model.CurrentId } }, CancellationToken.None);
            string episodeId = created.GetProperty("episode").GetProperty("id").GetString()!;
            await model.DeleteAsync();
            Assert.Contains(episodeId, model.MutationNotice);
            Assert.StartsWith(model["episode_invalidated_notice"], model.MutationNotice);
            Assert.DoesNotContain("episode_invalidated_notice", model.MutationNotice);
            Assert.Null(model.CurrentId);
            Assert.Empty(model.Saved);
            Assert.NotNull(model.Summary);
            Assert.Equal(0UL, model.Summary.SavedSnapshots);
            Assert.True(File.Exists(file));
        }
        finally { Directory.Delete(root, true); }
    }
}
