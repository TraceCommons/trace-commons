using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// W21: an opened History record's detail and its public-run editor, pinned
/// to the Mac's tests (macos/Tests/TraceCommonsAppTests/SessionPublicationTests.swift,
/// QuarantineExplanationTests.swift, ComputeSkillsParityTests.swift,
/// UnknownContributionStatusTests.swift) and the GTK port's
/// (crates/trace-commons-contributor-gtk/src/ui/history_detail_tests.rs).
/// </summary>
public class PublicRunTests
{
    /// <summary>The Mac's synthetic <c>history_detail</c> answer (SessionPublicationTests.swift:85).</summary>
    private const string MacDetail =
        "{\"owner_scope_sha256\":\"sha256:owner-a\",\"task\":\"Repair the generated client without editing its checked-in output.\",\"task_success\":\"partial\",\"user_feedback\":\"correction\",\"human_correction\":\"Use the bounded retry path.\",\"evidence\":[{\"event_id\":\"11111111-1111-4111-8111-111111111111\",\"kind\":\"tool_result\",\"excerpt\":\"The bounded retry completed on the second attempt.\"}],\"contribution_status\":\"accepted\",\"permitted_uses\":[\"debugging\",\"evaluation\"],\"contributed_version\":\"trace-contribution/1\",\"consent_policy_version\":\"consent/1\",\"redaction_pipeline_version\":\"redaction/3\",\"publication\":null,\"publication_version\":0,\"retained_source_slug\":\"source-workflow\"}";

    /// <summary>The Mac's synthetic <c>publish_public_run</c> answer (SessionPublicationTests.swift:91).</summary>
    private const string MacPage =
        "{\"slug\":\"bounded-retry\",\"title\":\"Bounded retry recovery\",\"outcome_summary\":\"The workflow recovered after one limited retry.\",\"workflow\":\"Retry once with the corrected bounded input.\",\"reuse_permission\":\"cc_by_4_0\",\"evidence\":[{\"excerpt\":\"The bounded retry completed on the second attempt.\"}],\"task_success\":\"partial\",\"contributed_version\":\"trace-contribution/1\",\"version\":1,\"published_at\":\"2026-09-10T12:00:00Z\",\"variations\":[]}";

    /// <summary>
    /// The three methods the inspector sends are the daemon's own
    /// (crates/trace-commons-contributor/src/daemon/ipc.rs:457, 522, 550),
    /// as <c>RoutingSurfaceTests.TheProbeMethodIsTheDaemonsOwn</c> pins its
    /// probe: the sample refusal list names its own literals, so a typo here
    /// would otherwise pass every test and fail only against a live daemon.
    /// </summary>
    [Fact]
    public void TheDetailAndPublicationMethodsAreTheDaemonsOwn()
    {
        Assert.Equal("history_detail", DaemonProtocol.Methods.HistoryDetail);
        Assert.Equal("publish_public_run", DaemonProtocol.Methods.PublishPublicRun);
        Assert.Equal("unpublish_public_run", DaemonProtocol.Methods.UnpublishPublicRun);
    }

    private static SessionDetail Detail(string json)
    {
        using JsonDocument doc = JsonDocument.Parse(json);
        return SessionDetail.Parse(doc.RootElement)!;
    }

    private static PublicRunCopy Copy => PublicRunCopy.Current!;

    [Fact]
    public void TheCoresTableDecodesWithEveryFieldTheMacRequires()
    {
        PublicRunCopy? copy = PublicRunCopy.Current;
        Assert.NotNull(copy);
        Assert.Equal(new[] { "cc_by_4_0", "cc0_1_0" }, copy!.ReusePermissions.Select(c => c.Permission));
        Assert.Null(PublicRunCopy.Parse("{\"all_contributions\":\"x\"}"));
        Assert.Null(PublicRunCopy.Parse("not json"));
    }

    /// <summary>SessionPublicationTests.swift:117-148, the decode half.</summary>
    [Fact]
    public void TheDetailDecodesAsTheMacDecodesIt()
    {
        SessionDetail detail = Detail(MacDetail);
        Assert.Equal("Repair the generated client without editing its checked-in output.", detail.Task);
        Assert.Equal("partial", detail.TaskSuccess);
        Assert.Equal("sha256:owner-a", detail.OwnerScopeSha256);
        Assert.Equal("Partly completed", Copy.TaskOutcomeLabel(detail.TaskSuccess));
        Assert.Equal("Feedback: correction supplied", Copy.FeedbackLabel(detail.UserFeedback));
        Assert.Equal(new[] { "11111111-1111-4111-8111-111111111111" }, detail.Evidence.Select(e => e.EventId));
        Assert.Equal(new[] { "Tool result" }, detail.Evidence.Select(e => Copy.EvidenceKindLabel(e.Kind)));
        Assert.Equal(new[] { "debugging", "evaluation" }, detail.PermittedUses);
        Assert.Equal("consent/1", detail.ConsentPolicyVersion);
        Assert.Null(detail.Publication);
        Assert.Equal(0, detail.PublicationVersion);
        Assert.Equal("source-workflow", detail.RetainedSourceSlug);
        Assert.True(detail.Accepted);
    }

    /// <summary>history_detail_tests.rs:20-36: the versions and evidence are required.</summary>
    [Fact]
    public void ADetailMissingARequiredFieldIsNotADetail()
    {
        foreach (string key in new[] { "consent_policy_version", "evidence", "publication_version" })
        {
            var node = System.Text.Json.Nodes.JsonNode.Parse(MacDetail)!.AsObject();
            node.Remove(key);
            using JsonDocument doc = JsonDocument.Parse(node.ToJsonString());
            Assert.Null(SessionDetail.Parse(doc.RootElement));
        }
    }

    /// <summary>SessionPublicationTests.swift:173-182: an older server's record still decodes.</summary>
    [Fact]
    public void AnOlderServersRecordDecodesWithoutTheNewFields()
    {
        SessionDetail detail = Detail(
            "{\"task_success\":\"success\",\"task_outcome\":\"Completed\",\"user_feedback\":\"none\",\"evidence\":[],\"contributed_version\":\"trace-contribution/1\",\"consent_policy_version\":\"consent/1\",\"redaction_pipeline_version\":\"redaction/3\",\"publication\":null,\"publication_version\":0}");
        Assert.Null(detail.Task);
        Assert.Null(detail.PermittedUses);
        Assert.Null(detail.ContributionStatus);
    }

    /// <summary>
    /// UnknownContributionStatusTests.swift:27-79 and history_detail_tests.rs:41-60:
    /// the core's words, its unavailable word for an unknown status, and no
    /// tag at all without the core's table.
    /// </summary>
    [Fact]
    public void TheStatusWordIsTheCoresAndNeverTheRawToken()
    {
        Assert.Equal("In the commons", HistoryInspection.StatusWord(Copy, "accepted"));
        Assert.Equal(Copy["contribution_status_unavailable"], HistoryInspection.StatusWord(Copy, "a-status-from-the-future"));
        Assert.Equal(Copy["contribution_status_unavailable"], HistoryInspection.StatusWord(Copy, null));
        Assert.Null(HistoryInspection.StatusWord(null, "accepted"));
        Assert.Equal(Copy["unrecognized_value"], Copy.PermittedUseLabel("not-a-use"));
    }

    /// <summary>SessionPublicationTests.swift:280-314: closed statuses, and unknown fails closed.</summary>
    [Fact]
    public void TerminalStatusesAreTheMacs()
    {
        foreach (string status in new[] { "withdrawn", "revoked", "purged", "expired", "a-status-from-the-future", "unknown", "", "Accepted" })
        {
            Assert.True(ContributionGate.IsTerminal(status), status);
        }

        foreach (string status in new[] { "submitted", "received", "quarantined", "awaiting_pii_backstop", "accepted", "rejected" })
        {
            Assert.False(ContributionGate.IsTerminal(status), status);
        }

        Assert.False(ContributionGate.IsTerminal(null));
    }

    /// <summary>
    /// The editor is offered only on an accepted record with a task outcome
    /// that is not withdrawn (SkillLearningView.swift:676-680), the rule
    /// ComputeSkillsParityTests.swift:180-200 pins for the same gate.
    /// </summary>
    [Fact]
    public void TheEditorIsOfferedOnlyOnAnActiveContribution()
    {
        SessionDetail detail = Detail(MacDetail);
        Assert.True(ContributionGate.IsActive(detail, "accepted", withdrawalCompleted: false));
        Assert.False(ContributionGate.IsActive(detail, "accepted", withdrawalCompleted: true));
        Assert.False(ContributionGate.IsActive(detail with { ContributionStatus = "quarantined" }, "quarantined", false));
        Assert.False(ContributionGate.IsActive(detail with { TaskSuccess = null }, "accepted", false));
        Assert.True(ContributionGate.IsWithdrawn(detail with { ContributionStatus = "withdrawn" }, "accepted", false));
        Assert.True(ContributionGate.WithdrawalCompleted("purged", withdrawnHere: false));
    }

    /// <summary>ComputeSkillsParityTests.swift:154-163: the status is said once.</summary>
    [Fact]
    public void TheInspectorSaysTheStatusOnce()
    {
        Assert.True(HistoryInspection.TagsStatus(recordResolves: false, detail: null));
        Assert.True(HistoryInspection.TagsStatus(recordResolves: true, detail: null));
        Assert.False(HistoryInspection.TagsStatus(recordResolves: true, detail: Detail(MacDetail)));
    }

    /// <summary>ComputeSkillsParityTests.swift:140-148: the row is found by its submission id.</summary>
    [Fact]
    public void TheInspectorResolvesTheRowBySubmissionId()
    {
        var rows = new[] { "s-1" };
        Assert.Equal("s-1", HistoryInspection.Opened("s-1", true, rows, r => r));
        Assert.Null(HistoryInspection.Opened("s-2", true, rows, r => r));
        Assert.Null(HistoryInspection.Opened("", true, rows, r => r));
        Assert.Null(HistoryInspection.Opened("s-1", false, rows, r => r));
    }

    /// <summary>QuarantineExplanationTests.swift:33-83.</summary>
    [Fact]
    public void HeldExplanationsAreDistinctFirstSeenAndDigestFree()
    {
        const string held = "Quarantined for privacy review; credit is pending review.";
        const string backstop = "Held pending an automated privacy backstop verdict; not yet in the corpus.";
        const string digest = "Attributed to tenant tenant_sha256:8719ab8d740b9882d27c80f473bfe5b1";
        Assert.Equal(new[] { held }, HistoryInspection.ExplanationLines(Enumerable.Repeat(new[] { held }, 210)));
        Assert.Equal(new[] { held }, HistoryInspection.ExplanationLines(new[] { new[] { held, digest } }));
        Assert.Equal(new[] { held, backstop }, HistoryInspection.ExplanationLines(new[] { new[] { held }, new[] { backstop }, new[] { held } }));
        Assert.Empty(HistoryInspection.ExplanationLines(new[] { new[] { digest } }));
    }

    /// <summary>
    /// SessionPublicationTests.swift:316-360: the editor is validated by the
    /// core, which trims, extracts the source slug, and refuses a missing
    /// permission. Nothing here validates in C#.
    /// </summary>
    [Fact]
    public void TheEditorIsValidatedByTheCore()
    {
        var evidence = new List<(string, string)> { ("11111111-1111-4111-8111-111111111111", "The bounded retry completed on the second attempt.") };
        var input = new PublicRunEditorInput(
            "  Bounded retry recovery  ", "  The workflow recovered.  ", null,
            "  Retry once with the corrected bounded input.  ", "cc_by_4_0", evidence, "  source-workflow  ");
        (PublicRunDraft? draft, string? error) = PublicRunEditor.Validate(input)!.Value;
        Assert.Null(error);
        Assert.Equal("Bounded retry recovery", draft!.Title);
        Assert.Equal("The workflow recovered.", draft.OutcomeSummary);
        Assert.Equal("Retry once with the corrected bounded input.", draft.Workflow);
        Assert.Equal("source-workflow", draft.SourceSlug);

        (PublicRunDraft? none, string? problem) = PublicRunEditor.Validate(input with { ReusePermission = null })!.Value;
        Assert.Null(none);
        Assert.Equal(Copy["validation_permission_required"], problem);
    }

    /// <summary>
    /// SessionPublicationTests.swift:150-170: the publish sends the reviewed
    /// draft as the core returned it, the detail's task outcome, contributed
    /// version and publication version, and nothing else; an absent source
    /// stays absent.
    /// </summary>
    [Fact]
    public void ThePublishSendsOnlyTheReviewedDraft()
    {
        SessionDetail detail = Detail(MacDetail);
        var evidence = new List<(string, string)> { ("11111111-1111-4111-8111-111111111111", "The bounded retry completed on the second attempt.") };
        var input = new PublicRunEditorInput(
            "Bounded retry recovery", "The workflow recovered after one limited retry.", "Use the bounded retry path.",
            "Retry once with the corrected bounded input.", "cc_by_4_0", evidence, string.Empty);
        PublicRunDraft draft = PublicRunEditor.Validate(input)!.Value.Draft!;

        string json = PublicRunEditor.PublishParams("s-1", draft, detail)!;
        using JsonDocument doc = JsonDocument.Parse(json);
        JsonElement root = doc.RootElement;
        Assert.Equal(
            new[] { "contributed_version", "draft", "expected_publication_version", "submission_id", "task_success" },
            root.EnumerateObject().Select(p => p.Name).OrderBy(n => n, System.StringComparer.Ordinal));
        Assert.Equal("partial", root.GetProperty("task_success").GetString());
        Assert.Equal("trace-contribution/1", root.GetProperty("contributed_version").GetString());
        Assert.Equal(0, root.GetProperty("expected_publication_version").GetInt32());
        Assert.Equal(
            new[] { "correction_excerpt", "evidence", "outcome_summary", "reuse_permission", "title", "workflow" },
            root.GetProperty("draft").EnumerateObject().Select(p => p.Name).OrderBy(n => n, System.StringComparer.Ordinal));
        Assert.False(root.GetProperty("draft").TryGetProperty("source_slug", out _));
        Assert.False(root.TryGetProperty("approval_sha256", out _));

        // No task outcome, no publish (AppModel.swift:4263-4265).
        Assert.Null(PublicRunEditor.PublishParams("s-1", draft, detail with { TaskSuccess = null }));
    }

    /// <summary>
    /// At most four evidence items (SessionDetailView.swift:318-321, 488-499),
    /// and a published page restores its evidence by excerpt (:520-528).
    /// </summary>
    [Fact]
    public void EvidenceStopsAtFourAndRestoresByExcerpt()
    {
        IReadOnlySet<string> selected = new HashSet<string>();
        foreach (string id in new[] { "a", "b", "c", "d", "e" })
        {
            selected = PublicRunEditor.Toggle(selected, id, on: true);
        }

        Assert.Equal(new[] { "a", "b", "c", "d" }, selected.OrderBy(s => s));
        Assert.False(PublicRunEditor.CanSelect(selected, "e"));
        Assert.True(PublicRunEditor.CanSelect(selected, "a"));
        Assert.Equal(3, PublicRunEditor.Toggle(selected, "a", on: false).Count);

        var record = new[]
        {
            new SessionEvidence("1", "tool_result", "same"),
            new SessionEvidence("2", "tool_result", "same"),
            new SessionEvidence("3", "tool_result", "other"),
        };
        Assert.Equal(
            new[] { "1", "2", "3" },
            PublicRunEditor.Restore(record, new[] { new PublicRunPageEvidence("same"), new PublicRunPageEvidence("same"), new PublicRunPageEvidence("other"), new PublicRunPageEvidence("gone") }).OrderBy(s => s));
        Assert.Equal("12/100", PublicRunEditor.Counter("Bounded retr", PublicRunEditor.TitleLimit));
    }

    /// <summary>
    /// SessionPublicationTests.swift:363-436: a publish answer is a page; an
    /// unpublish answer returns the version the next publish expects, so the
    /// record goes back to private at that version.
    /// </summary>
    [Fact]
    public void PublishAndUnpublishAnswersDecode()
    {
        using JsonDocument page = JsonDocument.Parse(MacPage);
        PublicRunPage published = PublicRunJson.Page(page.RootElement)!;
        Assert.Equal("bounded-retry", published.Slug);
        Assert.Equal("cc_by_4_0", published.ReusePermission);
        Assert.Equal(1, published.Version);

        using JsonDocument unpublished = JsonDocument.Parse("{\"unpublished\":true,\"expected_publication_version\":2,\"credential_warning\":\"commons_credential_storage_unavailable\"}");
        (int version, string? warning) = PublicRunJson.Unpublished(unpublished.RootElement)!.Value;
        Assert.Equal(2, version);
        Assert.Equal("commons_credential_storage_unavailable", warning);
    }

    /// <summary>
    /// The lines a failed read or publish shows are the core's
    /// (history_detail_tests.rs:56-60; SessionPublicationTests.swift:390-470),
    /// and never echo the label.
    /// </summary>
    [Fact]
    public void FailureLinesAreTheCores()
    {
        Assert.Equal(Copy["session_not_found"], PublicRunErrors.SessionDetailLine("session-detail-not-found"));
        Assert.Equal(Copy["publication_conflict"], PublicRunErrors.PublicationLine("public-run-conflict"));
        Assert.DoesNotContain("sample-unrecorded", PublicRunErrors.SessionDetailLine("sample-unrecorded"));
        Assert.NotEmpty(PublicRunErrors.SessionDetailLine("sample-unrecorded"));
    }

    /// <summary>
    /// HomeViews.swift:2270-2276: held is never drawn as rejected, accepted
    /// reads as done.
    /// </summary>
    [Fact]
    public void TheTagToneIsTheMacs()
    {
        Assert.Equal(StatusTone.On, HistoryInspection.Tone("accepted"));
        Assert.Equal(StatusTone.Ask, HistoryInspection.Tone("quarantined"));
        Assert.Equal(StatusTone.Ask, HistoryInspection.Tone("submitted"));
        Assert.Equal(StatusTone.Neutral, HistoryInspection.Tone("rejected"));
    }

    private static PublicRunPage Page(string json)
    {
        using JsonDocument doc = JsonDocument.Parse(json);
        return PublicRunJson.Page(doc.RootElement)!;
    }

    /// <summary>
    /// W21 spec: a refused publish leaves the record private and unchanged
    /// (AppModel.swift:4292-4295; SessionPublicationTests.swift:390-436).
    /// </summary>
    [Fact]
    public void ARefusedPublishLeavesTheRecordPrivate()
    {
        var state = new HistoryDetailState();
        state.ReadSucceeded(Detail(MacDetail));
        state.PublicationStarted();
        state.PublicationFailed("public-run-conflict");
        Assert.Null(state.Detail!.Publication);
        Assert.Equal(0, state.Detail.PublicationVersion);
        Assert.Equal(Copy["publication_conflict"], state.PublicationError);
    }

    /// <summary>
    /// W21 spec: unpublish returns the run to its earlier, private state at
    /// the version the daemon names (AppModel.swift:4312-4317).
    /// </summary>
    [Fact]
    public void UnpublishReturnsTheRunToPrivateAtTheDaemonsVersion()
    {
        var state = new HistoryDetailState();
        state.ReadSucceeded(Detail(MacDetail));
        state.Published(Page(MacPage));
        Assert.Equal("bounded-retry", state.Detail!.Publication!.Slug);
        Assert.Equal(1, state.Detail.PublicationVersion);
        Assert.Null(state.PublicationError);
        state.Unpublished(2, null);
        Assert.Null(state.Detail!.Publication);
        Assert.Equal(2, state.Detail.PublicationVersion);
        Assert.Null(state.PublicationError);
    }

    /// <summary>
    /// SessionPublicationTests.swift:438-470: a committed change stays on
    /// screen when the credential could not be stored, and the core says so.
    /// </summary>
    [Fact]
    public void ACredentialWarningKeepsTheCommittedChangeAndSaysSo()
    {
        var state = new HistoryDetailState();
        state.ReadSucceeded(Detail(MacDetail));
        string warned = MacPage.Replace("\"variations\":[]", "\"variations\":[],\"credential_warning\":\"commons_credential_storage_unavailable\"");
        state.Published(Page(warned));
        Assert.NotNull(state.Detail!.Publication);
        Assert.Equal(PublicRunErrors.PublicationLine("commons_credential_storage_unavailable"), state.PublicationError);
        Assert.Equal(Copy["credential_storage_warning"], state.PublicationError);
    }

    /// <summary>
    /// SessionPublicationTests.swift:483-575 and AppModel.swift:4228-4238: a
    /// failed reload keeps the detail it holds, unless its label ends the
    /// account, which clears it; a failed read says the core's line.
    /// </summary>
    [Fact]
    public void AFailedReadKeepsTheDetailUnlessItEndsTheAccount()
    {
        var state = new HistoryDetailState();
        state.ReadSucceeded(Detail(MacDetail));
        state.ReadFailed("synthetic-test-failure");
        Assert.NotNull(state.Detail);
        Assert.Equal(PublicRunErrors.SessionDetailLine("synthetic-test-failure"), state.ReadError);
        foreach (string label in HistoryDetailState.AccountEndingLabels)
        {
            state.ReadSucceeded(Detail(MacDetail));
            state.ReadFailed(label);
            Assert.Null(state.Detail);
            Assert.Equal(PublicRunErrors.SessionDetailLine(label), state.ReadError);
        }

        Assert.Equal(new[] { "account-session-required", "session-detail-not-found", "session-owner-changed" }, HistoryDetailState.AccountEndingLabels);
    }

    /// <summary>
    /// AppModel.swift:4243-4249: a read under another account replaces what
    /// the earlier account's read left, its publication error included.
    /// </summary>
    [Fact]
    public void AReadUnderAnotherAccountReplacesWhatWasHeld()
    {
        var state = new HistoryDetailState();
        state.ReadSucceeded(Detail(MacDetail));
        state.PublicationFailed("public-run-conflict");
        Assert.NotNull(state.PublicationError);
        state.ReadSucceeded(Detail(MacDetail.Replace("sha256:owner-a", "sha256:owner-b")));
        Assert.Null(state.PublicationError);
        state.PublicationFailed("public-run-conflict");
        state.ReadSucceeded(Detail(MacDetail.Replace("sha256:owner-a", "sha256:owner-b")));
        Assert.NotNull(state.PublicationError);
    }
}
