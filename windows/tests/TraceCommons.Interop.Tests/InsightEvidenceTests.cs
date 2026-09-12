using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading;
using System.Threading.Tasks;
using TraceCommons.App.ViewModels;
using Xunit;

namespace TraceCommons.Interop.Tests;

public sealed class InsightEvidenceTests
{
    private static JsonElement Json(string text) => JsonDocument.Parse(text).RootElement.Clone();
    private static string Fixture()
    {
        var snapshot = JsonNode.Parse(InsightsTests.Insight)!;
        snapshot["model_observations"] = JsonNode.Parse("""
          {"schema_version":1,"scope":"declared_metadata_only","source_format":"codex","source_digest":"source-digest",
           "coordinates":"jsonl_physical_lines_one_based","record_count":5,"candidate_records":5,"valid_declarations":3,
           "missing_declarations":1,"invalid_declarations":1,"omitted_declarations":1,"model_labels_omitted":false,
           "mixed_declared_models":true,"declared_models":["model-a","model-b"],"declarations":[
             {"model":"model-a","record_index":1,"kind":"codex_session_metadata"},
             {"model":"model-b","record_index":2,"kind":"codex_turn_context"}]}
          """);
        snapshot["outcome_links"] = JsonNode.Parse("""
          [{"id":"git-link","source_digest":"source-digest","linked_at":"2026-09-11T10:00:00Z","provenance":"user_linked",
            "evidence":{"type":"git_commit","evidence":{"repository_path_digest":"repository-digest","object_id":"commit-object","tree_id":"tree-object",
              "parent_ids":["parent-object"],"inspected_at":"2026-09-11T09:00:00Z","provenance":"inspected_local_object"}}},
           {"id":"test-link","source_digest":"source-digest","linked_at":"2026-09-11T10:00:00Z","provenance":"user_linked",
            "evidence":{"type":"test_report","evidence":{"schema_version":1,"runner":"cargo-test","passed":4,"failed":0,"skipped":1,
              "observed_at":"2026-09-10T10:00:00Z","commit_id":null,"artifact_digest":"artifact-digest",
              "imported_at":"2026-09-11T09:00:00Z","provenance":"imported_report"}}}]
          """);
        return snapshot.ToJsonString();
    }
    private sealed class Service : ILocalInsights
    {
        public string Snapshot = Fixture();
        public readonly List<JsonElement> Calls = new();
        public bool FailMutation;
        public TaskCompletionSource<JsonElement>? PendingMutation;
        public Task<JsonElement> CallAsync(object operation, CancellationToken token)
        {
            var op = JsonSerializer.SerializeToElement(operation);
            Calls.Add(op);
            string type = op.GetProperty("type").GetString()!;
            if (type is "link_git" or "link_test_report" or "unlink_evidence")
            {
                if (FailMutation) throw new InvalidOperationException("private-path");
                if (PendingMutation != null) return PendingMutation.Task;
            }
            if (type == "explain")
            {
                var selected = JsonNode.Parse(Snapshot)!;
                selected["id"] = op.GetProperty("id").GetString();
                return Task.FromResult(Json("{\"type\":\"explain\",\"insight\":" + selected.ToJsonString() + "}"));
            }
            return Task.FromResult(type switch {
                "copy" => Json("""
                  {"type":"copy","copy":{"unknown":"Unknown","model_legacy":"Unavailable until reimport","model_notice":"Declared metadata only",
                   "model_mixed":"Multiple names declared","model_not_proven_mixed":"Missing metadata may hide other models",
                   "model_labels":"Names","model_missing":"Missing","model_invalid":"Invalid","model_omitted":"Omitted",
                   "model_coordinates_jsonl_physical_lines_one_based":"JSONL lines starting at 1","model_record_index":"Record index",
                   "model_kind_codex_session_metadata":"Session metadata","model_kind_codex_turn_context":"Turn context",
                   "link_git_provenance":"Inspected Git object","link_test_provenance":"Imported report","link_user_provenance":"Linked by you",
                   "link_git_notice":"Not merge acceptance or revert proof","link_test_notice":"Not executed or verified here",
                   "test_failed":"Reported failed","test_claimed_commit":"Unverified commit","source_digest":"Source digest",
                   "artifact_digest":"Artifact digest","link_changed_selection":"Selection changed","error":"safe-error"}}
                  """),
                "list" => Json("{\"type\":\"list\",\"insights\":[" + Snapshot + "]}"),
                "summary" => Json(InsightsSummaryTests.Response),
                "episode_list" => Json("{\"type\":\"episode_list\",\"episodes\":[]}"),
                _ => Json("{\"type\":\"" + type + "\",\"insight\":" + Snapshot + "}")
            });
        }
    }
    [Fact]
    public void OptionalProjectionDecodesLegacyAndTypedEvidenceVariants()
    {
        var legacy = InsightEvidence.Decode(Json(InsightsTests.Insight));
        Assert.Null(legacy.ModelObservations);
        Assert.Empty(legacy.OutcomeLinks);
        var current = InsightEvidence.Decode(Json(Fixture()));
        Assert.True(current.ModelObservations!.MixedDeclaredModels);
        Assert.Equal(1UL, current.ModelObservations.MissingDeclarations);
        Assert.Equal(1UL, current.ModelObservations.OmittedDeclarations);
        Assert.IsType<GitCommitObservation>(current.OutcomeLinks[0].Evidence);
        var report = Assert.IsType<TestReportObservation>(current.OutcomeLinks[1].Evidence).Evidence;
        Assert.Equal(0UL, report.Failed);
        Assert.Null(report.CommitId);
        Assert.Equal("imported_report", report.Provenance);
        var leaked = InsightEvidence.Decode(Json(Fixture().Replace(
            "codex_session_metadata", "claude_assistant_message", StringComparison.Ordinal)));
        Assert.False(leaked.ModelObservations!.IsSupported());
    }
    [Fact]
    public void CodexTurnContextSchemaAcceptsKnownAbsenceAndRejectsForgedCoverage()
    {
        const string valid = """
          {"schema_version":2,"scope":"declared_metadata_only","source_format":"codex","source_digest":"source-digest",
           "coordinates":"jsonl_physical_lines_one_based","record_count":2,"candidate_records":0,"valid_declarations":0,
           "missing_declarations":0,"invalid_declarations":0,"omitted_declarations":0,"model_labels_omitted":false,
           "mixed_declared_models":false,"declared_models":[],"declarations":[]}
          """;
        var supported = JsonSerializer.Deserialize<DeclaredModelObservations>(valid,
            new JsonSerializerOptions { PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower })!;
        Assert.True(supported.IsSupported());
        foreach (string malformed in new[] {
            valid.Replace("\"candidate_records\":0", "\"candidate_records\":1", StringComparison.Ordinal),
            valid.Replace("\"source_format\":\"codex\"", "\"source_format\":\"trajectory\"", StringComparison.Ordinal),
            valid.Replace("\"declarations\":[]", "\"declarations\":[{\"model\":\"x\",\"record_index\":1,\"kind\":\"codex_session_metadata\"}]", StringComparison.Ordinal)
        })
        {
            var rejected = JsonSerializer.Deserialize<DeclaredModelObservations>(malformed,
                new JsonSerializerOptions { PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower })!;
            Assert.False(rejected.IsSupported());
        }
    }
    [Fact]
    public void ClaudeAssistantSchemaAcceptsRepeatedDeclarationsAndRejectsWrongKinds()
    {
        const string valid = """
          {"schema_version":3,"scope":"declared_metadata_only","source_format":"claude_code","source_digest":"source-digest",
           "coordinates":"jsonl_physical_lines_one_based","record_count":3,"candidate_records":3,"valid_declarations":2,
           "missing_declarations":1,"invalid_declarations":0,"omitted_declarations":0,"model_labels_omitted":false,
           "mixed_declared_models":false,"declared_models":["claude-a"],"declarations":[
             {"model":"claude-a","record_index":1,"kind":"claude_assistant_message"},
             {"model":"claude-a","record_index":2,"kind":"claude_assistant_message"}]}
          """;
        var supported = JsonSerializer.Deserialize<DeclaredModelObservations>(valid,
            new JsonSerializerOptions { PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower })!;
        Assert.True(supported.IsSupported());
        var crossSchema = JsonSerializer.Deserialize<DeclaredModelObservations>(
            valid.Replace("\"schema_version\":3", "\"schema_version\":1", StringComparison.Ordinal),
            new JsonSerializerOptions { PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower })!;
        Assert.False(crossSchema.IsSupported());
        foreach (string malformed in new[] {
            valid.Replace("\"source_format\":\"claude_code\"", "\"source_format\":\"codex\"", StringComparison.Ordinal),
            valid.Replace("claude_assistant_message", "codex_turn_context", StringComparison.Ordinal),
            valid.Replace("\"candidate_records\":3", "\"candidate_records\":2", StringComparison.Ordinal)
        })
        {
            var rejected = JsonSerializer.Deserialize<DeclaredModelObservations>(malformed,
                new JsonSerializerOptions { PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower })!;
            Assert.False(rejected.IsSupported());
        }
    }
    [Fact]
    public async Task RenderKeepsDeclarationsAndImportedReportsSeparateFromVerifiedOutcomes()
    {
        var service = new Service();
        using var model = new InsightsViewModel(service);
        await model.LoadAsync();
        await model.ExplainAsync("snapshot-a");
        Assert.Contains("model-a, model-b", model.ModelDetails);
        Assert.Contains("Multiple names declared", model.ModelDetails);
        Assert.Contains("Missing: 1", model.ModelDetails);
        Assert.Contains("Invalid: 1", model.ModelDetails);
        Assert.Contains("Omitted: 1", model.ModelDetails);
        Assert.Contains("JSONL lines starting at 1", model.ModelDetails);
        Assert.Contains("Record index: 2", model.ModelReferences[1].Label);
        Assert.Equal("Inspected Git object", model.OutcomeEvidence[0].Label);
        Assert.Contains("Not merge acceptance or revert proof", model.OutcomeEvidence[0].Details);
        Assert.Equal("Imported report", model.OutcomeEvidence[1].Label);
        Assert.Contains("Not executed or verified here", model.OutcomeEvidence[1].Details);
        Assert.Contains("Reported failed: 0", model.OutcomeEvidence[1].Details);
        Assert.Contains("Unverified commit: Unknown", model.OutcomeEvidence[1].Details);
        var incomplete = JsonNode.Parse(service.Snapshot)!;
        var observed = incomplete["model_observations"]!;
        observed["mixed_declared_models"] = false;
        observed["record_count"] = 3;
        observed["candidate_records"] = 3;
        observed["valid_declarations"] = 1;
        observed["omitted_declarations"] = 0;
        observed["declared_models"]!.AsArray().RemoveAt(1);
        observed["declarations"]!.AsArray().RemoveAt(1);
        service.Snapshot = incomplete.ToJsonString();
        await model.ExplainAsync("snapshot-a");
        Assert.Contains("Missing metadata may hide other models", model.ModelDetails);
        Assert.DoesNotContain("single", model.ModelDetails, StringComparison.OrdinalIgnoreCase);
        service.Snapshot = InsightsTests.Insight;
        await model.ExplainAsync("snapshot-a");
        Assert.Equal("Unavailable until reimport", model.ModelDetails);
        Assert.Empty(model.OutcomeEvidence);
    }
    [Fact]
    public async Task RenderAcceptsSchemaTwoKnownAbsenceWithoutInventingAModel()
    {
        var snapshot = JsonNode.Parse(InsightsTests.Insight)!;
        snapshot["model_observations"] = JsonNode.Parse("""
          {"schema_version":2,"scope":"declared_metadata_only","source_format":"codex","source_digest":"source-digest",
           "coordinates":"jsonl_physical_lines_one_based","record_count":2,"candidate_records":0,"valid_declarations":0,
           "missing_declarations":0,"invalid_declarations":0,"omitted_declarations":0,"model_labels_omitted":false,
           "mixed_declared_models":false,"declared_models":[],"declarations":[]}
          """);
        var service = new Service { Snapshot = snapshot.ToJsonString() };
        using var model = new InsightsViewModel(service);
        await model.LoadAsync();
        await model.ExplainAsync("snapshot-a");
        Assert.Contains("model_no_labels", model.ModelDetails);
        Assert.Contains("model_candidates: 0", model.ModelDetails);
        Assert.Empty(model.ModelReferences);
    }
    [Fact]
    public async Task PickerTicketIsBoundToSelectionReentryAndClosedLifetime()
    {
        var service = new Service();
        var model = new InsightsViewModel(service);
        await model.LoadAsync();
        await model.ExplainAsync("snapshot-a");
        var original = model.CaptureEvidenceTarget()!;
        await model.ExplainAsync("snapshot-b");
        int before = service.Calls.Count;
        await model.LinkGitAsync(original, "/explicit/repo", new string('a', 40));
        Assert.Equal(before, service.Calls.Count);
        Assert.Equal("Selection changed", model.Status);
        var beforeReentry = model.CaptureEvidenceTarget()!;
        await model.LoadAsync();
        before = service.Calls.Count;
        await model.LinkTestReportAsync(beforeReentry, "/explicit/report.json");
        Assert.Equal(before, service.Calls.Count);
        await model.ExplainAsync("snapshot-a");
        before = service.Calls.Count;
        var beforeClose = model.CaptureEvidenceTarget()!;
        model.Dispose();
        await model.LinkGitAsync(beforeClose, "/explicit/repo", new string('a', 40));
        Assert.Equal(before, service.Calls.Count);
    }
    [Fact]
    public async Task LinkingUsesCapturedIdRefreshesSummaryAndFailureClearsOldSuccess()
    {
        var service = new Service();
        using var model = new InsightsViewModel(service);
        await model.LoadAsync();
        await model.ExplainAsync("snapshot-a");
        await model.LinkGitAsync(model.CaptureEvidenceTarget()!, "/explicit/repo", new string('a', 40));
        Assert.Equal(new[] { "link_git", "list", "summary", "episode_list" }, service.Calls.TakeLast(4).Select(call => call.GetProperty("type").GetString()));
        Assert.Equal("snapshot-a", service.Calls[^4].GetProperty("id").GetString());
        await model.UnlinkEvidenceAsync(model.CaptureEvidenceTarget()!, "git-link");
        Assert.Equal("unlink_evidence", service.Calls[^4].GetProperty("type").GetString());
        service.FailMutation = true;
        await model.LinkTestReportAsync(model.CaptureEvidenceTarget()!, "/explicit/report.json");
        Assert.Null(model.CurrentId);
        Assert.Empty(model.Details);
        Assert.Empty(model.ModelDetails);
        Assert.Empty(model.OutcomeEvidence);
        Assert.Null(model.Summary);
        Assert.Equal("safe-error", model.Status);
    }
    [Fact]
    public async Task ClosingDuringLinkSuppressesLatePresentation()
    {
        var service = new Service();
        var model = new InsightsViewModel(service);
        await model.LoadAsync();
        await model.ExplainAsync("snapshot-a");
        service.PendingMutation = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var operation = model.LinkTestReportAsync(model.CaptureEvidenceTarget()!, "/explicit/report.json");
        model.Dispose();
        int changes = 0;
        model.PropertyChanged += (_, _) => changes++;
        service.PendingMutation.SetResult(Json("{\"type\":\"link_test_report\",\"insight\":" + Fixture() + "}"));
        await operation;
        Assert.Empty(model.OutcomeEvidence);
        Assert.Null(model.CurrentId);
        Assert.Equal(0, changes);
    }
    [Fact]
    public void NativeControlsCaptureBeforePickersAndNavigationInvalidatesTickets()
    {
        var root = Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App");
        string code = File.ReadAllText(Path.Combine(root, "Controls", "InsightsView.xaml.cs.txt"));
        int start = code.IndexOf("private async void OnLinkGit", StringComparison.Ordinal);
        string git = code[start..code.IndexOf("private async void OnLinkTestReport", start, StringComparison.Ordinal)];
        Assert.True(git.IndexOf("CaptureEvidenceTarget()", StringComparison.Ordinal) < git.IndexOf("PickSingleFolderAsync", StringComparison.Ordinal));
        Assert.True(git.IndexOf("string commit = ViewModel.CommitId", StringComparison.Ordinal) < git.IndexOf("PickSingleFolderAsync", StringComparison.Ordinal));
        Assert.Contains("LinkGitAsync(target, folder.Path, commit)", git);
        Assert.Contains("ShowingInsights) && !ViewModel.ShowingInsights", File.ReadAllText(Path.Combine(root, "MainWindow.xaml.cs.txt")));
    }
    [Fact]
    public async Task NativeEvidenceLifecycleUsesSyntheticFilesWithoutStartingDaemon()
    {
        string root = Path.Combine(Path.GetTempPath(), "tc-evidence-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(root);
        try
        {
            string store = Path.Combine(root, "insights");
            string file = Path.Combine(root, "selected.jsonl");
            await File.WriteAllTextAsync(file, "{\"type\":\"session_meta\",\"payload\":{\"model\":\"model-a\"}}\n{\"type\":\"turn_context\",\"payload\":{\"model\":\"model-b\"}}\n{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"synthetic\"}]}}\n");
            using var model = new InsightsViewModel(new LocalInsights(store));
            await model.LoadAsync();
            Assert.False(Directory.Exists(store));
            await model.AnalyzeAsync("codex", file, true);
            Assert.NotNull(model.CurrentId);
            Assert.Contains("model-b", model.ModelDetails);
            Assert.DoesNotContain("model-a", model.ModelDetails);
            Assert.Single(model.ModelReferences);
            await model.AnalyzeAsync("codex", file, true);
            Assert.Single(model.Saved);
            string reportFile = Path.Combine(root, "report.json");
            await File.WriteAllTextAsync(reportFile, JsonSerializer.Serialize(new {
                schema_version = 1, runner = "synthetic-test", passed = 1, failed = 0, skipped = 0,
                observed_at = DateTimeOffset.UtcNow, commit_id = (string?)null
            }));
            await model.LinkTestReportAsync(model.CaptureEvidenceTarget()!, reportFile);
            var report = Assert.Single(model.OutcomeEvidence);
            Assert.Equal("Imported test report", report.Label);
            Assert.Contains("Reported failed: 0", report.Details);
            Assert.NotNull(model.Summary);
            string repository = Path.Combine(root, "repository");
            Directory.CreateDirectory(repository);
            Git(repository, "", "init", "--quiet");
            string tree = Git(repository, "", "mktree");
            string commit = Git(repository, "tree " + tree + "\nauthor Fixture <fixture@example.test> 1700000000 +0000\ncommitter Fixture <fixture@example.test> 1700000000 +0000\n\nFixture\n", "hash-object", "-t", "commit", "-w", "--stdin");
            await model.LinkGitAsync(model.CaptureEvidenceTarget()!, repository, commit);
            Assert.Equal(2, model.OutcomeEvidence.Count);
            Assert.Contains(model.OutcomeEvidence, row => row.Label == "Inspected local Git object" && row.Details.Contains(commit, StringComparison.Ordinal));
            await model.UnlinkEvidenceAsync(model.CaptureEvidenceTarget()!, report.Id);
            Assert.Equal("Inspected local Git object", Assert.Single(model.OutcomeEvidence).Label);
            Assert.True(File.Exists(reportFile));
            Assert.True(File.Exists(file));
            string absent = Path.Combine(root, "absent.jsonl");
            await File.WriteAllTextAsync(absent, "{\"type\":\"session_meta\",\"payload\":{\"model\":\"ignored\"}}\n{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"model\":\"ignored\",\"content\":[]}}\n");
            await model.AnalyzeAsync("codex", absent, true);
            Assert.Contains("No valid model names were retained.", model.ModelDetails);
            Assert.Contains("Supported metadata records: 0", model.ModelDetails);
            Assert.DoesNotContain("ignored", model.ModelDetails);
            Assert.Empty(model.ModelReferences);
            Assert.Equal(2, model.Saved.Count);
            Assert.False(File.Exists(Path.Combine(root, "contributor.json")));
        }
        finally { DeleteTestTree(root); }
    }
    private static void DeleteTestTree(string root)
    {
        if (!Directory.Exists(root)) return;
        // Git for Windows marks loose objects read-only. Normalize fixture files
        // before recursive deletion so cleanup cannot fail after a passing test.
        foreach (string file in Directory.EnumerateFiles(root, "*", SearchOption.AllDirectories))
            File.SetAttributes(file, FileAttributes.Normal);
        Directory.Delete(root, true);
    }

    [Fact]
    public void TestTreeCleanupRemovesReadOnlyGitStyleFiles()
    {
        string root = Path.Combine(Path.GetTempPath(), "tc-evidence-cleanup-" + Guid.NewGuid().ToString("N"));
        string objects = Path.Combine(root, "repository", ".git", "objects", "aa");
        Directory.CreateDirectory(objects);
        string looseObject = Path.Combine(objects, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        File.WriteAllText(looseObject, "fixture");
        File.SetAttributes(looseObject, File.GetAttributes(looseObject) | FileAttributes.ReadOnly);

        DeleteTestTree(root);

        Assert.False(Directory.Exists(root));
    }
    private static string Git(string repository, string input, params string[] arguments)
    {
        var start = new ProcessStartInfo("git") { WorkingDirectory = repository, RedirectStandardInput = true,
            RedirectStandardOutput = true, RedirectStandardError = true, UseShellExecute = false };
        foreach (var key in start.Environment.Keys.Where(key => key.StartsWith("GIT_", StringComparison.OrdinalIgnoreCase)).ToArray())
            start.Environment.Remove(key);
        start.Environment["GIT_CONFIG_NOSYSTEM"] = "1";
        start.Environment["GIT_CONFIG_GLOBAL"] = OperatingSystem.IsWindows() ? "NUL" : "/dev/null";
        foreach (string argument in arguments) start.ArgumentList.Add(argument);
        using var process = Process.Start(start)!;
        process.StandardInput.Write(input);
        process.StandardInput.Close();
        string output = process.StandardOutput.ReadToEnd();
        process.StandardError.ReadToEnd();
        process.WaitForExit();
        Assert.Equal(0, process.ExitCode);
        return output.Trim();
    }

    [Fact]
    public async Task NewPickerSupersedesOldAndCancellationInvalidatesOnlyItsOwnTicket()
    {
        var service = new Service();
        using var model = new InsightsViewModel(service);
        await model.LoadAsync();
        await model.ExplainAsync("snapshot-a");
        var older = model.CaptureEvidenceTarget()!;
        var newer = model.CaptureEvidenceTarget()!;
        int before = service.Calls.Count;
        await model.LinkGitAsync(older, "/explicit/repo", new string('a', 40));
        Assert.Equal(before, service.Calls.Count);
        model.CancelEvidenceTarget(older);
        await model.LinkTestReportAsync(newer, "/explicit/report.json");
        Assert.Equal("link_test_report", service.Calls[^4].GetProperty("type").GetString());
        var cancelled = model.CaptureEvidenceTarget()!;
        model.CancelEvidenceTarget(cancelled);
        before = service.Calls.Count;
        await model.LinkGitAsync(cancelled, "/explicit/repo", new string('a', 40));
        Assert.Equal(before, service.Calls.Count);
    }

}
