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

public sealed class InsightEpisodeTests
{
    private const string EpisodeId = "11111111-1111-4111-8111-111111111111";
    private const string SnapshotId = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    private static JsonElement Json(string value) => JsonDocument.Parse(value).RootElement.Clone();
    private static string Episode(ulong revision = 1, ulong membershipRevision = 1, string assessment = "null") => $$"""
        {"schema_version":1,"id":"{{EpisodeId}}","revision":{{revision}},"membership_revision":{{membershipRevision}},
         "created_at":"2026-09-11T10:00:00Z","updated_at":"2026-09-11T10:00:00Z","provenance":"user_selected_whole_snapshots",
         "members":[{"snapshot_id":"{{SnapshotId}}","source_digest":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}],
         "manual_assessment":{{assessment}}}
        """;
    private static string Insight => InsightsTests.Insight.Replace("snapshot-a", SnapshotId, StringComparison.Ordinal)
        .Replace("abcdef", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", StringComparison.Ordinal);
    private static string Detail(ulong revision = 1, ulong membershipRevision = 1, string assessment = "null") =>
        "{\"type\":\"episode_explain\",\"detail\":{\"episode\":" + Episode(revision, membershipRevision, assessment) +
        ",\"members\":[" + Insight + "],\"overlap\":[{\"snapshot_id\":\"" + SnapshotId +
        "\",\"episode_ids\":[\"22222222-2222-4222-8222-222222222222\"]}],\"resolved_at\":\"2026-09-11T10:01:00Z\"}}";

    private sealed class Service : ILocalInsights
    {
        public List<JsonElement> Calls { get; } = new();
        public ulong Revision = 1;
        public ulong MembershipRevision = 1;
        public bool Exists = true;
        public bool ConflictReplace;
        public bool FailDetail;
        public bool MismatchedDetail;
        public bool FailSnapshotRefreshAfterDelete;
        private bool _snapshotDeleted;
        public TaskCompletionSource<JsonElement>? PendingExplain;
        public Task<JsonElement> CallAsync(object operation, CancellationToken cancellationToken)
        {
            var request = JsonSerializer.SerializeToElement(operation);
            Calls.Add(request);
            string type = request.GetProperty("type").GetString()!;
            if (type == "episode_replace_members" && ConflictReplace)
            {
                ConflictReplace = false;
                Revision++;
                throw new InsightsServiceException("insights_episode_revision_conflict");
            }
            if (type == "episode_explain" && PendingExplain != null) return PendingExplain.Task;
            if (type == "episode_explain" && FailDetail) throw new InvalidOperationException("private-detail-failure");
            if (type == "episode_create") Exists = true;
            if (type == "episode_delete") Exists = false;
            if (type == "delete") _snapshotDeleted = true;
            if (type == "list" && _snapshotDeleted && FailSnapshotRefreshAfterDelete)
                throw new InvalidOperationException("private-list-failure");
            if (type == "episode_annotate") Revision++;
            if (type == "episode_replace_members") { Revision++; MembershipRevision++; }
            return Task.FromResult(type switch {
                "copy" => Json("""{"type":"copy","copy":{"error":"Safe error","episode_empty":"No groups","episode_create_success":"Created","episode_members_saved":"Members saved","episode_membership_changed":"Assessment cleared","episode_assessment_saved":"Assessment saved","episode_deleted":"Deleted","episode_invalidated_notice":"Groups removed","episode_revision_conflict":"Review conflict","episode_missing":"Missing","episode_no_overlap":"No overlap","episode_overlaps":"Overlap","episode_member_evidence":"Evidence","episode_unassessed":"Unassessed","episode_id":"ID","episode_revision":"Revision","episode_membership_revision":"Membership revision","episode_created_at":"Created at","episode_updated_at":"Updated at","episode_resolved":"Resolved","episode_assessment":"Assessment","episode_list_unavailable":"List unavailable","episode_detail_unavailable":"Detail unavailable","episode_selection_empty":"Select members","episode_invalid":"Invalid","episode_missing_members":"Missing members","episode_member_limit":"Member limit","episode_limit":"Episode limit","episode_response_too_large":"Too large","codex":"Codex","link_empty":"No evidence","category_tests":"Tests","outcome_accepted":"Accepted"}}"""),
                "list" => Json("{\"type\":\"list\",\"insights\":[" + Insight + "]}"),
                "summary" => Json(InsightsSummaryTests.Response),
                "episode_list" => Json("{\"type\":\"episode_list\",\"episodes\":" + (Exists ? "[{\"episode\":" + Episode(Revision, MembershipRevision) + ",\"overlapping_episode_ids\":[]}]" : "[]") + "}"),
                "episode_explain" => Json(MismatchedDetail ? Detail(Revision, MembershipRevision).Replace(EpisodeId, "33333333-3333-4333-8333-333333333333", StringComparison.Ordinal) : Detail(Revision, MembershipRevision)),
                "episode_create" => Json("{\"type\":\"episode_create\",\"episode\":" + Episode(Revision, MembershipRevision) + "}"),
                "episode_replace_members" => Json("{\"type\":\"episode_replace_members\",\"episode\":" + Episode(Revision, MembershipRevision) + "}"),
                "episode_annotate" => Json("{\"type\":\"episode_annotate\",\"episode\":" + Episode(Revision, MembershipRevision) + "}"),
                "episode_clear_assessment" => Json("{\"type\":\"episode_clear_assessment\",\"episode\":" + Episode(Revision, MembershipRevision) + "}"),
                "episode_delete" => Json("{\"type\":\"episode_delete\",\"episode\":" + Episode(Revision, MembershipRevision) + "}"),
                "explain" => Json("{\"type\":\"explain\",\"insight\":" + Insight + "}"),
                "delete" => Json("{\"type\":\"delete\",\"deleted\":true,\"mutation_effects\":{\"invalidated_episode_ids\":[\"" + EpisodeId + "\"]}}"),
                _ => throw new InvalidOperationException(type)
            });
        }
    }

    [Fact]
    public void TypedResponsesPreserveMembershipOverlapAndEvidenceAndRejectWrongSemantics()
    {
        var detail = InsightEpisodeResponses.DecodeDetail(Json(Detail()));
        Assert.Equal(EpisodeId, detail.Episode.Id);
        Assert.Equal(SnapshotId, Assert.Single(detail.Episode.Members).SnapshotId);
        Assert.Equal("22222222-2222-4222-8222-222222222222", Assert.Single(detail.Overlap).EpisodeIds[0]);
        Assert.Equal("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", Assert.Single(detail.Members).Evidence[0].SourceDigest);
        Assert.Throws<InvalidOperationException>(() => InsightEpisodeResponses.DecodeDetail(Json(Detail().Replace(
            "user_selected_whole_snapshots", "inferred_task", StringComparison.Ordinal))));
        Assert.Throws<InvalidOperationException>(() => InsightEpisodeResponses.DecodeList(Json("{\"type\":\"episode_list\",\"episodes\":[{\"episode\":" +
            Episode(0) + ",\"overlapping_episode_ids\":[]}]}")));
    }

    [Fact]
    public async Task CreatesOpensAndMutatesWithFrozenRevisionAndIndependentAssessment()
    {
        var service = new Service { Exists = false };
        using var model = new InsightsViewModel(service);
        await model.LoadAsync();
        await model.CreateEpisodeAsync(new[] { SnapshotId });
        Assert.Equal(EpisodeId, model.CurrentEpisodeId);
        Assert.Equal("Created", model.EpisodeStatus);
        var target = model.CaptureEpisodeTarget();
        Assert.NotNull(target);
        await model.SaveEpisodeAssessmentAsync(target!);
        var annotate = service.Calls.Single(call => call.GetProperty("type").GetString() == "episode_annotate");
        Assert.Equal(1UL, annotate.GetProperty("expected_revision").GetUInt64());
        Assert.Equal("unknown", annotate.GetProperty("category").GetString());
        Assert.Equal("unknown", annotate.GetProperty("outcome").GetString());
        target = model.CaptureEpisodeTarget();
        await model.ReplaceEpisodeMembersAsync(target!, new[] { SnapshotId });
        Assert.Equal("Members saved\nAssessment cleared", model.EpisodeStatus);
        Assert.Contains("Overlap", Assert.Single(model.EpisodeMembers).Details);
        Assert.Contains("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", model.EpisodeMembers[0].Details);
    }

    [Fact]
    public async Task ConflictRefreshesForReviewOnceAndInvalidatesFrozenDraft()
    {
        var service = new Service();
        using var model = new InsightsViewModel(service);
        await model.LoadAsync();
        await model.OpenEpisodeAsync(EpisodeId);
        var stale = model.CaptureEpisodeTarget()!;
        service.ConflictReplace = true;
        await model.ReplaceEpisodeMembersAsync(stale, new[] { SnapshotId });
        Assert.Equal("Review conflict", model.EpisodeStatus);
        Assert.Equal(2UL, model.CurrentEpisodeRevision);
        Assert.Single(service.Calls, call => call.GetProperty("type").GetString() == "episode_replace_members");
        int before = service.Calls.Count;
        await model.DeleteEpisodeAsync(stale);
        Assert.Equal(before, service.Calls.Count);
        Assert.Equal("Review conflict", model.EpisodeStatus);
        Assert.Null(model.CaptureEpisodeTarget());
    }

    [Fact]
    public async Task ControlDraftBindingRebindsAfterReconciliationAndRequiresReopenAfterConflict()
    {
        var service = new Service { Exists = false };
        using var model = new InsightsViewModel(service);
        var binding = new EpisodeMemberDraftBinding();
        await model.LoadAsync();
        await model.CreateEpisodeAsync(new[] { SnapshotId });
        Assert.True(binding.Reconcile(model, EpisodeId));
        var createDraft = binding.Consume()!;
        await model.ReplaceEpisodeMembersAsync(createDraft, new[] { SnapshotId });
        Assert.True(binding.Reconcile(model, EpisodeId));
        var editDraft = binding.Consume()!;
        Assert.True(editDraft.Revision > createDraft.Revision);
        await model.SaveEpisodeAssessmentAsync(editDraft);
        Assert.True(binding.Reconcile(model, EpisodeId));
        var assessmentDraft = binding.Consume()!;
        Assert.True(assessmentDraft.Revision > editDraft.Revision);
        service.ConflictReplace = true;
        await model.ReplaceEpisodeMembersAsync(assessmentDraft, new[] { SnapshotId });
        Assert.False(binding.Reconcile(model, EpisodeId));
        Assert.False(binding.HasDraft);
        await model.OpenEpisodeAsync(EpisodeId);
        Assert.True(binding.Reconcile(model, EpisodeId));
        Assert.True(binding.HasDraft);
    }

    [Fact]
    public async Task CommittedMutationWithFailedReconciliationClearsEditableDetailAndKeepsBothNotices()
    {
        var service = new Service();
        using var model = new InsightsViewModel(service);
        await model.LoadAsync();
        await model.OpenEpisodeAsync(EpisodeId);
        var target = model.CaptureEpisodeTarget()!;
        service.FailDetail = true;
        await model.SaveEpisodeAssessmentAsync(target);
        Assert.Null(model.CurrentEpisodeId);
        Assert.Null(model.CaptureEpisodeTarget());
        Assert.Contains("Assessment saved", model.EpisodeStatus);
        Assert.Contains("Detail unavailable", model.EpisodeStatus);
        Assert.DoesNotContain("private-detail-failure", model.EpisodeStatus);
    }

    [Fact]
    public async Task CommittedMembershipChangeKeepsAssessmentClearedNoticeWhenRefreshFails()
    {
        var service = new Service();
        using var model = new InsightsViewModel(service);
        await model.LoadAsync();
        await model.OpenEpisodeAsync(EpisodeId);
        var target = model.CaptureEpisodeTarget()!;
        service.FailDetail = true;
        await model.ReplaceEpisodeMembersAsync(target, new[] { SnapshotId });
        Assert.Null(model.CaptureEpisodeTarget());
        Assert.Equal("Members saved\nAssessment cleared\nDetail unavailable", model.EpisodeStatus);
    }

    [Fact]
    public async Task MismatchedDetailIdIsNeverPresented()
    {
        var service = new Service { MismatchedDetail = true };
        using var model = new InsightsViewModel(service);
        await model.LoadAsync();
        await model.OpenEpisodeAsync(EpisodeId);
        Assert.Null(model.CurrentEpisodeId);
        Assert.Empty(model.EpisodeMembers);
        Assert.Empty(model.Status);
        Assert.Equal("Detail unavailable", model.EpisodeStatus);
    }

    [Fact]
    public async Task SnapshotCleanupClearsInvalidatedEpisodeBeforeFailedRefreshAndKeepsNotice()
    {
        var service = new Service { FailSnapshotRefreshAfterDelete = true };
        using var model = new InsightsViewModel(service);
        await model.LoadAsync();
        await model.ExplainAsync(SnapshotId);
        await model.OpenEpisodeAsync(EpisodeId);
        Assert.NotNull(model.CurrentEpisodeId);
        await model.DeleteAsync();
        Assert.Null(model.CurrentEpisodeId);
        Assert.Null(model.CaptureEpisodeTarget());
        Assert.Contains(EpisodeId, model.MutationNotice);
        Assert.Equal("Safe error", model.Status);
    }

    [Fact]
    public async Task CancellationInvalidatesPendingEpisodePresentation()
    {
        var service = new Service { PendingExplain = new(TaskCreationOptions.RunContinuationsAsynchronously) };
        using var model = new InsightsViewModel(service);
        var open = model.OpenEpisodeAsync(EpisodeId);
        model.Cancel();
        service.PendingExplain.SetResult(Json(Detail()));
        await open;
        Assert.Null(model.CurrentEpisodeId);
        Assert.Empty(model.EpisodeMembers);
    }

    [Fact]
    public async Task NativeEpisodeLifecyclePreservesSnapshotsAndRequiresCurrentRevision()
    {
        string root = Path.Combine(Path.GetTempPath(), "insights-episodes-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(root);
        try
        {
            string first = Path.Combine(root, "first.jsonl");
            string second = Path.Combine(root, "second.jsonl");
            await File.WriteAllTextAsync(first, "{\"type\":\"session_meta\",\"payload\":{}}\n{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"first fixture\"}]}}\n");
            await File.WriteAllTextAsync(second, "{\"type\":\"session_meta\",\"payload\":{}}\n{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"second fixture\"}]}}\n");
            var service = new LocalInsights(Path.Combine(root, "store"));
            using var model = new InsightsViewModel(service);
            await model.LoadAsync();
            await model.AnalyzeAsync("codex", first, true);
            string firstId = model.CurrentId!;
            await model.AnalyzeAsync("codex", second, true);
            string secondId = model.CurrentId!;
            await model.CreateEpisodeAsync(new[] { firstId });
            string episodeId = model.CurrentEpisodeId!;
            var original = model.CaptureEpisodeTarget()!;
            model.EpisodeCategoryIndex = 1;
            model.EpisodeOutcomeIndex = 0;
            await model.SaveEpisodeAssessmentAsync(original);
            var assessed = model.CaptureEpisodeTarget()!;
            await model.ReplaceEpisodeMembersAsync(assessed, new[] { firstId });
            Assert.Equal(model["episode_members_saved"], model.EpisodeStatus);
            Assert.Contains(model["category_tests"], model.EpisodeDetails);
            var beforeChange = model.CaptureEpisodeTarget()!;
            await model.ReplaceEpisodeMembersAsync(beforeChange, new[] { firstId, secondId });
            Assert.Contains(model["episode_membership_changed"], model.EpisodeStatus);
            Assert.Contains(model["episode_unassessed"], model.EpisodeDetails);
            await Assert.ThrowsAsync<InsightsServiceException>(() => service.CallAsync(new {
                type = "episode_delete", id = episodeId, expected_revision = original.Revision
            }, CancellationToken.None));
            var current = model.CaptureEpisodeTarget()!;
            await model.DeleteEpisodeAsync(current);
            Assert.Empty(model.Episodes);
            Assert.Equal(2, model.Saved.Count);
            Assert.True(File.Exists(first));
            Assert.True(File.Exists(second));
        }
        finally { Directory.Delete(root, true); }
    }
}
