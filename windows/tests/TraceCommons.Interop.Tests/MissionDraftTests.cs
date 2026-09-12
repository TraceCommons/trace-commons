using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using TraceCommons.App.ViewModels;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

public sealed class MissionDraftTests
{
    private const string Id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    private static JsonElement Json(string value) => JsonDocument.Parse(value).RootElement.Clone();
    private static string Review => $$"""{"schema_version":1,"proposal_sha256":"{{Id}}","status":"needs_curator_review","publication_authorized":false,"external_sources_verified":false,"required_reviews":["sources"]}""";
    private static string Summary => $$"""{"id":"{{Id}}","source_count":1,"status":"needs_curator_review"}""";
    private static string Proposal => """{"schema_version":1,"author_id":"author","title":"Untrusted title","source_urls":["https://example.com/source"],"claim_to_test":"Claim","task":"Task","starting_artifact":{"url":"https://example.com/artifact","sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},"evaluator_id":"evaluator","rubric_version":"rubric-1","success_criteria":["criterion"],"required_evidence":["evidence"],"allowed_models":["model"],"allowed_tools":["tool"],"budget":{"max_duration_seconds":60,"max_input_tokens":100,"max_output_tokens":50}}""";
    private static string Copy()
    {
        var map = MissionDraftDecoding.RequiredCopyKeys.ToDictionary(key => key, key => key, StringComparer.Ordinal);
        return JsonSerializer.Serialize(new { type = "copy", copy = map });
    }

    private sealed class Service : ILocalMissionDrafts
    {
        public List<JsonElement> Calls { get; } = new();
        public TaskCompletionSource<JsonElement>? PendingShow;
        public bool FailShow;
        public bool FailList;
        public Task<JsonElement> CallAsync(object operation, CancellationToken cancellationToken)
        {
            JsonElement request = JsonSerializer.SerializeToElement(operation); Calls.Add(request);
            string type = request.GetProperty("type").GetString()!;
            if (type == "show" && PendingShow != null) return PendingShow.Task;
            if (type == "show" && FailShow) throw new InvalidOperationException("private failure");
            if (type == "list" && FailList) throw new InvalidOperationException("private list failure");
            return Task.FromResult(type switch {
                "copy" => Json(Copy()),
                "list" => Json("{\"type\":\"list\",\"drafts\":[" + Summary + "]}"),
                "import" => Json("{\"type\":\"import\",\"draft\":{\"id\":\"" + Id + "\",\"review\":" + Review + ",\"inserted\":true}}"),
                "show" => Json("{\"type\":\"show\",\"draft\":{\"id\":\"" + Id + "\",\"proposal\":" + Proposal + ",\"review\":" + Review + "}}"),
                "delete" => Json("{\"type\":\"delete\",\"draft\":{\"id\":\"" + Id + "\",\"deleted\":true}}"),
                _ => throw new InvalidOperationException(type)
            });
        }
    }

    [Fact]
    public void StrictDecoderRejectsUnknownFieldsAuthorityAndMismatchedIds()
    {
        Assert.Single(MissionDraftDecoding.List(Json("{\"type\":\"list\",\"drafts\":[" + Summary + "]}")));
        Assert.Throws<JsonException>(() => MissionDraftDecoding.List(Json("{\"type\":\"list\",\"drafts\":[],\"extra\":true}")));
        Assert.Throws<InvalidOperationException>(() => MissionDraftDecoding.Import(Json("{\"type\":\"import\",\"draft\":{\"id\":\"" + Id + "\",\"review\":" + Review.Replace("false", "true", StringComparison.Ordinal) + ",\"inserted\":true}}")));
        Assert.Throws<InvalidOperationException>(() => MissionDraftDecoding.Show(Json("{\"type\":\"show\",\"draft\":{\"id\":\"" + "c".PadLeft(64, 'c') + "\",\"proposal\":" + Proposal + ",\"review\":" + Review + "}}")));
    }

    [Fact]
    public async Task FailedInspectPreservesDetailAndNavigationDiscardsLateResponse()
    {
        var service = new Service(); using var model = new MissionDraftsViewModel(service);
        await model.LoadAsync(); await model.ShowAsync(Id); Assert.Equal(Id, model.Current!.Id);
        service.FailShow = true; await model.ShowAsync(Id); Assert.Equal(Id, model.Current!.Id);
        service.FailShow = false; service.PendingShow = new(TaskCreationOptions.RunContinuationsAsynchronously);
        Task pending = model.ShowAsync(Id); model.Deactivate();
        service.PendingShow.SetResult(Json("{\"type\":\"show\",\"draft\":{\"id\":\"" + Id + "\",\"proposal\":" + Proposal + ",\"review\":" + Review + "}}"));
        await pending; Assert.Null(model.Current);
    }

    [Fact]
    public async Task DeleteInvalidatesPendingInspectAndClearsOnlyAfterSuccess()
    {
        var service = new Service(); using var model = new MissionDraftsViewModel(service);
        await model.LoadAsync();
        var completion = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        service.PendingShow = completion;
        Task pending = model.ShowAsync(Id); service.PendingShow = null;
        await model.DeleteAsync(Id);
        completion.SetResult(Json("{\"type\":\"show\",\"draft\":{\"id\":\"" + Id + "\",\"proposal\":" + Proposal + ",\"review\":" + Review + "}}"));
        await pending; Assert.Null(model.Current);
    }

    [Fact]
    public async Task CommittedDeleteClearsDetailWhenReconciliationFails()
    {
        var service = new Service(); using var model = new MissionDraftsViewModel(service);
        await model.LoadAsync(); await model.ShowAsync(Id); Assert.NotNull(model.Current);
        service.FailList = true; await model.DeleteAsync(Id);
        Assert.Null(model.Current); Assert.Contains("deleted", model.Status); Assert.Contains("error", model.Status);
    }

    [Fact]
    public void ShellIsLocalFirstAndRendersUrlsAsPlainText()
    {
        string root = Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App");
        string window = File.ReadAllText(Path.Combine(root, "MainWindow.xaml.cs.txt"));
        int start = window.IndexOf("private async void OnShowMissionDrafts", StringComparison.Ordinal);
        int end = window.IndexOf("private async void ShowInsightsPane", start, StringComparison.Ordinal);
        string handler = window[start..end];
        Assert.DoesNotContain("StartContributionsAsync", handler);
        string view = File.ReadAllText(Path.Combine(root, "Controls", "MissionDraftsView.xaml.txt"));
        Assert.DoesNotContain("Hyperlink", view); Assert.DoesNotContain("NavigateUri", view);
        Assert.Contains("StartingArtifact.Url", view); Assert.Contains("SourceUrls", view);
    }

    [Fact]
    public async Task NativeLifecycleUsesExplicitFileAndLeavesItUnchanged()
    {
        string root = Path.Combine(Path.GetTempPath(), "mission-drafts-native-" + Guid.NewGuid().ToString("N")); Directory.CreateDirectory(root);
        try
        {
            string store = Path.Combine(root, "store"); string file = Path.Combine(root, "draft.json");
            string fixture = Path.GetFullPath(Path.Combine(AppContext.BaseDirectory, "../../../../../../crates/trace-commons-protocol/tests/fixtures/mission-draft.json"));
            byte[] original = await File.ReadAllBytesAsync(fixture); await File.WriteAllBytesAsync(file, original);
            var service = new LocalMissionDrafts(store);
            Assert.Empty(MissionDraftDecoding.List(await service.CallAsync(new { type = "list" }, CancellationToken.None)));
            Assert.False(Directory.Exists(store));
            var imported = MissionDraftDecoding.Import(await service.CallAsync(new { type = "import", file }, CancellationToken.None));
            Assert.Single(MissionDraftDecoding.List(await service.CallAsync(new { type = "list" }, CancellationToken.None)));
            Assert.Equal(imported.Id, MissionDraftDecoding.Show(await service.CallAsync(new { type = "show", id = imported.Id }, CancellationToken.None)).Id);
            Assert.True(MissionDraftDecoding.Delete(await service.CallAsync(new { type = "delete", id = imported.Id }, CancellationToken.None)).Deleted);
            Assert.Equal(original, await File.ReadAllBytesAsync(file));
        }
        finally { Directory.Delete(root, true); }
    }
}
