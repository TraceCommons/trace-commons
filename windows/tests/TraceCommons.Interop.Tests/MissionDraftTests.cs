using System;
using System.Collections.Generic;
using System.Collections.Concurrent;
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
    private const string SecondId = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
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

    private sealed class MutationRaceService : ILocalMissionDrafts
    {
        private int _draftCount = 1;
        private bool _blockNextList;
        public List<string> Calls { get; } = new();
        public TaskCompletionSource<bool> ReconcileEntered { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);
        public TaskCompletionSource<bool> ReleaseReconcile { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);

        public async Task<JsonElement> CallAsync(object operation, CancellationToken cancellationToken)
        {
            JsonElement request = JsonSerializer.SerializeToElement(operation);
            string type = request.GetProperty("type").GetString()!;
            Calls.Add(type);
            if (type == "import")
            {
                _draftCount = 2;
                _blockNextList = true;
                return Json("{\"type\":\"import\",\"draft\":{\"id\":\"" + SecondId + "\",\"review\":" + Review.Replace(Id, SecondId, StringComparison.Ordinal) + ",\"inserted\":true}}");
            }
            if (type == "delete")
            {
                _draftCount = 0;
                _blockNextList = true;
                return Json("{\"type\":\"delete\",\"draft\":{\"id\":\"" + Id + "\",\"deleted\":true}}");
            }
            if (type == "list")
            {
                if (_blockNextList)
                {
                    _blockNextList = false;
                    ReconcileEntered.SetResult(true);
                    await ReleaseReconcile.Task; // Deliberately ignores cancellation like a native call already in progress.
                }
                string drafts = _draftCount switch
                {
                    0 => string.Empty,
                    1 => Summary,
                    _ => Summary + "," + Summary.Replace(Id, SecondId, StringComparison.Ordinal)
                };
                return Json("{\"type\":\"list\",\"drafts\":[" + drafts + "]}");
            }
            return type switch
            {
                "copy" => Json(Copy()),
                "show" => Json("{\"type\":\"show\",\"draft\":{\"id\":\"" + Id + "\",\"proposal\":" + Proposal + ",\"review\":" + Review + "}}"),
                _ => throw new InvalidOperationException(type)
            };
        }
    }

    private sealed class ManualSynchronizationContext : SynchronizationContext
    {
        private readonly ConcurrentQueue<(SendOrPostCallback Callback, object? State)> _work = new();
        public override void Post(SendOrPostCallback callback, object? state) => _work.Enqueue((callback, state));
        public bool RunOne()
        {
            if (!_work.TryDequeue(out var work)) return false;
            SynchronizationContext? prior = Current;
            SetSynchronizationContext(this);
            try { work.Callback(work.State); }
            finally { SetSynchronizationContext(prior); }
            return true;
        }
        public void RunAll() { while (RunOne()) { } }
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
    public async Task NamedCopyPropertiesPopulateAndNotifyCompiledBindings()
    {
        var service = new Service();
        using var model = new MissionDraftsViewModel(service);
        bool allPropertiesChanged = false;
        model.PropertyChanged += (_, change) => allPropertiesChanged |= string.IsNullOrEmpty(change.PropertyName);

        await model.LoadAsync();

        Assert.True(allPropertiesChanged);
        Assert.Equal("title", model.Title);
        Assert.Equal("choose_file", model.ChooseFile);
        Assert.Equal("display_notice", model.DisplayNotice);
        Assert.Equal("output_tokens", model.OutputTokens);
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
    public async Task ConflictingDeleteWaitsForPendingInspect()
    {
        var service = new Service(); using var model = new MissionDraftsViewModel(service);
        await model.LoadAsync();
        var completion = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        service.PendingShow = completion;
        Task pending = model.ShowAsync(Id); service.PendingShow = null;
        Task delete = model.DeleteAsync(Id);
        Assert.False(delete.IsCompleted);
        completion.SetResult(Json("{\"type\":\"show\",\"draft\":{\"id\":\"" + Id + "\",\"proposal\":" + Proposal + ",\"review\":" + Review + "}}"));
        await pending; await delete; Assert.Null(model.Current);
    }

    [Fact]
    public async Task ImportReconciliationCannotBeSupersededByInspect()
    {
        var service = new MutationRaceService(); using var model = new MissionDraftsViewModel(service);
        await model.LoadAsync();
        Task import = model.ImportAsync("/explicit/draft.json");
        await service.ReconcileEntered.Task;
        Task show = model.ShowAsync(Id);
        Assert.True(model.IsBusy); Assert.False(model.CanAct); Assert.False(show.IsCompleted);
        Assert.Equal(0, service.Calls.Count(call => call == "show"));
        service.ReleaseReconcile.SetResult(true);
        await Task.WhenAll(import, show);
        Assert.Equal(2, model.Drafts.Count); Assert.Equal(Id, model.Current?.Id); Assert.False(model.IsBusy);
    }

    [Fact]
    public async Task DeleteReconciliationCannotBeSupersededByInspect()
    {
        var service = new MutationRaceService(); using var model = new MissionDraftsViewModel(service);
        await model.LoadAsync(); await model.ShowAsync(Id); Assert.NotNull(model.Current);
        Task delete = model.DeleteAsync(Id);
        await service.ReconcileEntered.Task;
        Task show = model.ShowAsync(Id);
        Assert.True(model.IsBusy); Assert.False(model.CanAct); Assert.False(show.IsCompleted);
        Assert.Equal(1, service.Calls.Count(call => call == "show"));
        service.ReleaseReconcile.SetResult(true);
        await Task.WhenAll(delete, show);
        Assert.Empty(model.Drafts); Assert.Null(model.Current); Assert.False(model.IsBusy);
    }

    [Fact]
    public async Task NavigationRejectsActionWhosePermitWasGrantedBeforeItsContinuation()
    {
        var service = new Service(); using var model = new MissionDraftsViewModel(service);
        await model.LoadAsync();
        var showCompletion = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        service.PendingShow = showCompletion;
        var context = new ManualSynchronizationContext();
        SynchronizationContext? prior = SynchronizationContext.Current;
        SynchronizationContext.SetSynchronizationContext(context);
        Task show;
        Task import;
        try
        {
            show = model.ShowAsync(Id);
            import = model.ImportAsync("/stale/draft.json");
        }
        finally { SynchronizationContext.SetSynchronizationContext(prior); }

        showCompletion.SetResult(Json("{\"type\":\"show\",\"draft\":{\"id\":\"" + Id + "\",\"proposal\":" + Proposal + ",\"review\":" + Review + "}}"));
        Assert.True(context.RunOne()); // The show completes and grants the queued import its permit.
        model.Deactivate();
        context.RunAll();              // The import continuation observes its cancelled lifetime.
        await Task.WhenAll(show, import);

        Assert.DoesNotContain(service.Calls, call => call.GetProperty("type").GetString() == "import");
        await model.RefreshAsync();    // A rejected stale action returned the permit.
        Assert.False(model.IsBusy);
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
        Assert.DoesNotContain("ViewModel[", view);
        Assert.Contains("ViewModel.Title", view); Assert.Contains("ViewModel.OutputTokens", view);
        Assert.Contains("StartingArtifact.Url", view); Assert.Contains("SourceUrls", view);
        Assert.Equal(4, view.Split("IsEnabled=\"{x:Bind ViewModel.CanAct, Mode=OneWay}\"", StringSplitOptions.None).Length - 1);
        Assert.Contains("IsEnabled=\"{x:Bind CanImport, Mode=OneWay}\"", view);
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
