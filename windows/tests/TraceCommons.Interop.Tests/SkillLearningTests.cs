using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading.Tasks;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The tested-skill workflow against the Mac's own expectations
/// (macos/Tests/TraceCommonsAppTests/SkillLearningTests.swift,
/// SkillLearningClientTests.swift, the state assertions of
/// SkillLearningRenderingTests.swift, and the gate cases of
/// ComputeSkillsParityTests.swift:164-216), driven through the same daemon
/// double (SkillLearningTestSupport.swift) answering with JSON frames.
/// </summary>
/// <remarks>
/// The Mac compares a failure with the real core's
/// <c>TCSkillLearning.errorLine(label:)</c>; here the model is given
/// <see cref="Line"/>, which names the label it was asked for, so each
/// assertion still pins which label settled the session without needing the
/// cdylib. The Mac's <c>waitUntil</c> polling is replaced by awaiting the
/// action, whose task completes once the session has settled.
/// </remarks>
public sealed class SkillLearningTests
{
    private const string RecordId = "22222222-2222-4222-8222-222222222222";
    private const string SecondId = "88888888-8888-4888-8888-888888888888";
    private const string Mismatch = "99999999-9999-4999-8999-999999999999";
    private const string LineageInvalid = "skill-source-lineage-invalid";

    private static readonly SkillLearningCopy Copy = SkillLearningCopy.Parse(
        new JsonObject(SkillLearningCopy.RequiredWords.Select(key =>
            KeyValuePair.Create(key, (JsonNode?)JsonValue.Create("word:" + key)))).ToJsonString())!;

    private readonly SkillLearningFakeDaemon _daemon = new();

    private static string? Line(string label) => "line:" + label;

    private static SkillLearningModel Model(SkillLearningFakeDaemon daemon) => new(daemon.CallAsync, Copy, Line);

    // SkillLearningTests.swift:23-45.
    [Fact]
    public void StoreBoundsSettledSessionsAndKeepsTypedRecoveryLabels()
    {
        var store = new SkillLearningStore();
        for (int index = 0; index < 30; index++)
        {
            string id = "session-" + index;
            Assert.True(store.BeginInstallStatusLoading(id));
            Assert.False(store.BeginInstallStatusLoading(id));
            Assert.True(store.FinishInstallStatusLoading(id, null));
            Assert.False(store.BeginInstallStatusLoading(id));
        }

        Assert.Equal(24, store.RetainedSessionCount);
        Assert.True(SkillLearningDaemonFailure.FromLabel("skill-review-unknown").InvalidatesWorkflow);
        Assert.Equal(
            SkillLearningDaemonFailureKind.InstallUnknown,
            SkillLearningDaemonFailure.FromLabel("skill-install-unknown").Kind);
        Assert.False(SkillLearningDaemonFailure.FromLabel("skill-install-unknown").InvalidatesWorkflow);
    }

    // SkillLearningTests.swift:47-146.
    [Fact]
    public async Task StoreAllowsOnlyLegalTransitionsAndRestoresThePriorPhaseAfterRollback()
    {
        var client = new SkillLearningClient(_daemon.CallAsync);
        SkillCandidate candidate = await client.SkillCandidateAsync(RecordId);
        SkillReview review = await client.ReviewSkillAsync(candidate.CandidateId, candidate.Draft);
        SkillEvaluationReport report = await client.EvaluateSkillAsync(review.ReviewId, review.SkillSha256);
        SkillInstallPlan plan = await client.SkillInstallPlanAsync(report.EvaluationId);
        SkillLearningInstall installed = await client.InstallSkillAsync(plan.PlanId, plan.SkillSha256, plan.MarkerFileSha256);
        SkillCandidate wrongCandidate = await client.SkillCandidateAsync(SecondId);
        _daemon.SetResponseField("skill_review", "candidate_id", Mismatch);
        SkillReview wrongReview = await client.ReviewSkillAsync(candidate.CandidateId, candidate.Draft);
        _daemon.SetResponseField("skill_review", "candidate_id", null);
        _daemon.SetResponseField("skill_evaluate", "review_id", Mismatch);
        SkillEvaluationReport wrongReport = await client.EvaluateSkillAsync(review.ReviewId, review.SkillSha256);
        _daemon.SetResponseField("skill_evaluate", "review_id", null);
        _daemon.SetResponseField("skill_install_plan", "evaluation_id", Mismatch);
        SkillInstallPlan wrongPlan = await client.SkillInstallPlanAsync(report.EvaluationId);
        _daemon.SetResponseField("skill_install_plan", "evaluation_id", null);
        _daemon.SetResponseField("skill_install_commit", "evaluation_id", Mismatch);
        SkillLearningInstall wrongInstalled = await client.InstallSkillAsync(plan.PlanId, plan.SkillSha256, plan.MarkerFileSha256);
        _daemon.SetResponseField("skill_install_commit", "evaluation_id", null);

        var store = new SkillLearningStore();
        const string id = RecordId;
        Assert.Null(store.BeginReview(id));
        Assert.Null(store.BeginInstallPlanning(id));
        Assert.False(store.BeginLearning(id));
        Assert.True(store.BeginInstallStatusLoading(id));
        Assert.True(store.FinishInstallStatusLoading(id, null));
        Assert.True(store.BeginLearning(id));
        Assert.False(store.BeginLearning(id));
        Assert.False(store.FinishLearning(id, wrongCandidate));
        Assert.True(store.State(id).IsWorking);
        Assert.True(store.FinishLearning(id, candidate));

        Assert.False(store.BeginLearning(id));
        Assert.Equal(candidate, store.BeginReview(id)?.Candidate);
        Assert.False(store.FinishReview(id, wrongReview));
        Assert.True(store.State(id).IsWorking);
        Assert.True(store.FinishReview(id, review));
        Assert.Equal(review, store.BeginEvaluation(id));
        Assert.False(store.FinishEvaluation(id, wrongReport));
        Assert.True(store.State(id).IsWorking);
        Assert.True(store.FinishEvaluation(id, report));
        Assert.Equal(report, store.BeginInstallPlanning(id));
        Assert.False(store.FinishInstallPlanning(id, wrongPlan));
        Assert.True(store.State(id).IsWorking);
        Assert.True(store.FinishInstallPlanning(id, plan));
        Assert.Equal(plan, store.BeginInstall(id));
        Assert.False(store.FinishInstall(id, wrongInstalled));
        Assert.True(store.State(id).IsWorking);
        Assert.True(store.FinishInstall(id, installed));
        Assert.Equal(installed, store.BeginRollback(id));
        store.FinishRollback(id, removed: true, incompleteMessage: "incomplete");

        SkillLearningPhase restored = store.State(id).Phase;
        Assert.Equal(SkillLearningPhaseKind.Evaluated, restored.Kind);
        Assert.Equal(candidate, restored.Candidate);
        Assert.Equal(review, restored.Review);
        Assert.Equal(report, restored.Report);
        Assert.Null(store.State(id).Operation);
        Assert.Null(store.State(id).Failure);
    }

    // SkillLearningTests.swift:148-173.
    [Fact]
    public async Task ModelCompletesApprovalEvaluationInstallAndRollback()
    {
        SkillLearningModel model = Model(_daemon);

        await model.LearnSkillAsync(RecordId);
        SkillCandidate candidate = Some(CandidateOf(model, RecordId));

        await model.ReviewSkillAsync(RecordId, candidate.Draft);
        Assert.NotNull(ReviewOf(model, RecordId));
        await model.TestSkillAsync(RecordId);
        Assert.True(Some(EvaluationOf(model, RecordId)).InstallAllowed);

        await model.ReviewSkillInstallAsync(RecordId);
        Assert.NotNull(InstallPlanOf(model, RecordId));
        await model.InstallSkillAsync(RecordId);
        Assert.NotNull(InstalledSkillOf(model, RecordId));
        await model.RollbackSkillAsync(RecordId);
        Assert.NotNull(EvaluationOf(model, RecordId));
        Assert.False(model.State(RecordId).IsWorking);
        Assert.Null(model.State(RecordId).Failure);
    }

    // SkillLearningTests.swift:175-254.
    [Fact]
    public async Task MismatchedSuccessResponsesClearBusyStateAndKeepTheRecoverablePhase()
    {
        SkillLearningModel model = Model(_daemon);
        string? expectedFailure = Line(LineageInvalid);

        await model.LearnSkillAsync(RecordId);
        SkillDraft draft = Some(CandidateOf(model, RecordId)).Draft;

        _daemon.SetResponseField("skill_review", "candidate_id", Mismatch);
        await model.ReviewSkillAsync(RecordId, draft);
        Assert.False(model.State(RecordId).IsWorking);
        Assert.Equal(SkillLearningPhaseKind.Candidate, model.State(RecordId).Phase.Kind);
        Assert.Equal(expectedFailure, model.State(RecordId).Failure);

        _daemon.SetResponseField("skill_review", "candidate_id", null);
        await model.ReviewSkillAsync(RecordId, draft);
        Assert.NotNull(ReviewOf(model, RecordId));
        Assert.False(model.State(RecordId).IsWorking);

        _daemon.SetResponseField("skill_evaluate", "review_id", Mismatch);
        await model.TestSkillAsync(RecordId);
        Assert.False(model.State(RecordId).IsWorking);
        Assert.Equal(SkillLearningPhaseKind.Reviewed, model.State(RecordId).Phase.Kind);
        Assert.Equal(expectedFailure, model.State(RecordId).Failure);

        _daemon.SetResponseField("skill_evaluate", "review_id", null);
        await model.TestSkillAsync(RecordId);
        Assert.NotNull(EvaluationOf(model, RecordId));
        Assert.False(model.State(RecordId).IsWorking);

        _daemon.SetResponseField("skill_install_plan", "evaluation_id", Mismatch);
        await model.ReviewSkillInstallAsync(RecordId);
        Assert.False(model.State(RecordId).IsWorking);
        Assert.Equal(SkillLearningPhaseKind.Evaluated, model.State(RecordId).Phase.Kind);
        Assert.Equal(expectedFailure, model.State(RecordId).Failure);

        _daemon.SetResponseField("skill_install_plan", "evaluation_id", null);
        await model.ReviewSkillInstallAsync(RecordId);
        Assert.NotNull(InstallPlanOf(model, RecordId));
        Assert.False(model.State(RecordId).IsWorking);

        _daemon.SetResponseField("skill_install_commit", "evaluation_id", Mismatch);
        await model.InstallSkillAsync(RecordId);
        Assert.NotNull(InstalledSkillOf(model, RecordId));
        Assert.False(model.State(RecordId).IsWorking);
        Assert.Equal(SkillLearningPhaseKind.Evaluated, model.State(RecordId).Phase.Kind);
        Assert.Null(model.State(RecordId).Failure);
        Assert.Equal(
            new[] { "skill_install_commit", "skill_install_status" },
            _daemon.Calls.TakeLast(2).Select(call => call.Method));
    }

    // SkillLearningTests.swift:256-295.
    [Fact]
    public async Task ApprovedCustomDraftSurvivesEdit()
    {
        SkillLearningModel model = Model(_daemon);
        var customDraft = new SkillDraft(
            "repair-generated-artifacts",
            "Use when an artifact is generated from a source definition.",
            "# Procedure\n\nEdit the source definition, regenerate, and run drift checks.");

        await model.LearnSkillAsync(RecordId);
        Assert.NotNull(CandidateOf(model, RecordId));
        await model.ReviewSkillAsync(RecordId, customDraft);
        Assert.Equal(customDraft, ReviewOf(model, RecordId)?.Draft);

        model.EditSkill(RecordId);
        SkillLearningPhase phase = model.State(RecordId).Phase;
        Assert.Equal(SkillLearningPhaseKind.Candidate, phase.Kind);
        Assert.Equal(customDraft, phase.Candidate?.Draft);

        await model.ReviewSkillAsync(RecordId, customDraft);
        Assert.NotNull(ReviewOf(model, RecordId));
        Assert.False(model.State(RecordId).IsWorking);
        SkillLearningFakeDaemon.Call replacement = _daemon.Calls.Last(call => call.Method == "skill_review");
        Assert.Equal("44444444-4444-4444-8444-444444444444", Text(Params(replacement), "replaces_review_id"));
    }

    // SkillLearningTests.swift:297-340.
    [Fact]
    public async Task FreshClientCarriesRecoveredReviewIdentityIntoPaidEvaluationReuse()
    {
        const string reviewId = "44444444-4444-4444-8444-444444444444";
        var customDraft = new SkillDraft(
            "repair-generated-artifacts",
            "Use when an artifact is generated from a source definition.",
            "# Procedure\n\nEdit the source definition, regenerate, and run drift checks.");
        _daemon.SeedRecoveredReview(customDraft, reviewId);
        SkillLearningModel model = Model(_daemon);

        await model.LearnSkillAsync(RecordId);
        Assert.False(model.State(RecordId).IsWorking);
        SkillCandidate recovered = Some(CandidateOf(model, RecordId));
        Assert.Equal(customDraft, recovered.Draft);
        Assert.Equal(reviewId, recovered.ReplacesReviewId);
        Assert.Equal(reviewId, model.State(RecordId).ReplacesReviewId);

        await model.ReviewSkillAsync(RecordId, recovered.Draft);
        Assert.NotNull(ReviewOf(model, RecordId));
        SkillLearningFakeDaemon.Call replacement = _daemon.Calls.Last(call => call.Method == "skill_review");
        Assert.Equal(reviewId, Text(Params(replacement), "replaces_review_id"));

        await model.TestSkillAsync(RecordId);
        Assert.Equal(reviewId, EvaluationOf(model, RecordId)?.ReviewId);
        Assert.Single(_daemon.Calls, call => call.Method == "skill_evaluate");
    }

    // SkillLearningTests.swift:342-402.
    [Fact]
    public async Task ExpiredOpaqueIdsResetToIdleAndPreserveDraftAcrossTheNextLearn()
    {
        var cases = new[]
        {
            (Method: "skill_review", Label: "skill-candidate-unknown"),
            (Method: "skill_evaluate", Label: "skill-review-unknown"),
            (Method: "skill_install_plan", Label: "skill-evaluation-unknown"),
        };

        for (int index = 0; index < cases.Length; index++)
        {
            var testCase = cases[index];
            var localDaemon = new SkillLearningFakeDaemon();
            SkillLearningModel model = Model(localDaemon);
            var customDraft = new SkillDraft(
                "repair-generated-artifacts-" + index,
                "Use when generated output has an authoritative source.",
                "# Procedure\n\nChange the source, regenerate, and verify drift.");

            await model.LearnSkillAsync(RecordId);
            Assert.NotNull(CandidateOf(model, RecordId));

            if (index == 0)
            {
                localDaemon.SetFailure(testCase.Method, testCase.Label);
                await model.ReviewSkillAsync(RecordId, customDraft);
            }
            else
            {
                await model.ReviewSkillAsync(RecordId, customDraft);
                Assert.NotNull(ReviewOf(model, RecordId));
                if (index == 1)
                {
                    localDaemon.SetFailure(testCase.Method, testCase.Label);
                    await model.TestSkillAsync(RecordId);
                }
                else
                {
                    await model.TestSkillAsync(RecordId);
                    Assert.NotNull(EvaluationOf(model, RecordId));
                    localDaemon.SetFailure(testCase.Method, testCase.Label);
                    await model.ReviewSkillInstallAsync(RecordId);
                }
            }

            Assert.Equal(SkillLearningPhaseKind.Idle, model.State(RecordId).Phase.Kind);
            Assert.False(model.State(RecordId).IsWorking);
            Assert.Equal(Line(testCase.Label), model.State(RecordId).Failure);

            localDaemon.SetFailure(testCase.Method, null);
            await model.LearnSkillAsync(RecordId);
            Assert.False(model.State(RecordId).IsWorking);
            Assert.Equal(customDraft, CandidateOf(model, RecordId)?.Draft);
        }
    }

    // SkillLearningTests.swift:404-443.
    [Fact]
    public async Task ExpiredDraftSurvivesAFailedRelearnBeforeRecovery()
    {
        SkillLearningModel model = Model(_daemon);
        var customDraft = new SkillDraft(
            "repair-generated-artifacts",
            "Use when generated output has an authoritative source.",
            "# Procedure\n\nChange the source, regenerate, and verify drift.");

        await model.LearnSkillAsync(RecordId);
        Assert.NotNull(CandidateOf(model, RecordId));
        await model.ReviewSkillAsync(RecordId, customDraft);
        Assert.NotNull(ReviewOf(model, RecordId));
        _daemon.SetFailure("skill_evaluate", "skill-review-unknown");
        await model.TestSkillAsync(RecordId);
        Assert.Equal(SkillLearningPhaseKind.Idle, model.State(RecordId).Phase.Kind);
        Assert.False(model.State(RecordId).IsWorking);

        _daemon.SetFailure("skill_evaluate", null);
        _daemon.SetFailure("skill_candidate", "skill-candidate-unavailable");
        await model.LearnSkillAsync(RecordId);
        Assert.Equal(Line("skill-candidate-unavailable"), model.State(RecordId).Failure);
        Assert.False(model.State(RecordId).IsWorking);

        _daemon.SetFailure("skill_candidate", null);
        await model.LearnSkillAsync(RecordId);
        Assert.False(model.State(RecordId).IsWorking);
        Assert.Equal(customDraft, CandidateOf(model, RecordId)?.Draft);
    }

    // SkillLearningTests.swift:445-475.
    [Theory]
    [InlineData(true, false)]
    [InlineData(false, true)]
    public async Task InitialRecoveryRejectsContradictoryInstallStatusShapes(bool installed, bool includesSkill)
    {
        var localDaemon = new SkillLearningFakeDaemon();
        localDaemon.SetInstallStatusShape(installed, includesSkill);
        SkillLearningModel model = Model(localDaemon);

        await model.LearnSkillAsync(RecordId);
        Assert.Equal(Line(LineageInvalid), model.State(RecordId).Failure);
        Assert.False(model.State(RecordId).IsWorking);
        Assert.Equal(SkillLearningPhaseKind.Idle, model.State(RecordId).Phase.Kind);
        Assert.Equal(new[] { "skill_install_status" }, localDaemon.Calls.Select(call => call.Method));
    }

    // SkillLearningTests.swift:477-497.
    [Fact]
    public async Task LocalInstallRecoveryRejectsAnotherSourceLineage()
    {
        _daemon.SeedInstalledSkill("repair-generated-artifacts", Mismatch);
        _daemon.SetInstallStatusShape(true, true);
        SkillLearningModel model = Model(_daemon);

        await model.EnsureLocalInstalledSkillStatusAsync(RecordId);
        Assert.Equal(Line(LineageInvalid), model.State(RecordId).Failure);
        Assert.False(model.State(RecordId).IsWorking);
        Assert.Equal(SkillLearningPhaseKind.Idle, model.State(RecordId).Phase.Kind);
        Assert.Null(model.State(RecordId).SkillLearningInstall);
    }

    // SkillLearningTests.swift:499-534.
    [Fact]
    public async Task RollbackUnknownReconcilesADeletedInstallationThroughStatus()
    {
        SkillLearningModel model = Model(_daemon);
        await ThroughInstall(model);
        Assert.NotNull(InstalledSkillOf(model, RecordId));

        int statusCallsBeforeRollback = _daemon.Calls.Count(call => call.Method == "skill_install_status");
        _daemon.SetInstalled(false);
        _daemon.SetFailure("skill_install_rollback", "skill-install-unknown");
        await model.RollbackSkillAsync(RecordId);
        Assert.NotNull(EvaluationOf(model, RecordId));
        Assert.False(model.State(RecordId).IsWorking);

        Assert.Null(model.State(RecordId).Failure);
        Assert.Null(InstalledSkillOf(model, RecordId));
        Assert.Equal(statusCallsBeforeRollback + 1, _daemon.Calls.Count(call => call.Method == "skill_install_status"));
    }

    // SkillLearningTests.swift:536-570.
    [Fact]
    public async Task OccupiedInstallCanBeRetriedWithoutRepeatingEvaluation()
    {
        _daemon.SetInstallOccupied(true);
        SkillLearningModel model = Model(_daemon);

        await ThroughEvaluation(model);
        await model.ReviewSkillInstallAsync(RecordId);
        Assert.True(InstallPlanOf(model, RecordId)?.Occupied);

        _daemon.SetInstallOccupied(false);
        await model.ReviewSkillInstallAsync(RecordId);
        Assert.True(InstallPlanOf(model, RecordId)?.CanInstall);

        Assert.Equal(2, _daemon.Calls.Count(call => call.Method == "skill_install_plan"));
        Assert.Single(_daemon.Calls, call => call.Method == "skill_evaluate");
    }

    // SkillLearningTests.swift:572-595.
    [Fact]
    public async Task FailedGateCannotCreateAnInstallationPlan()
    {
        _daemon.SetGatePasses(false);
        SkillLearningModel model = Model(_daemon);
        await ThroughEvaluation(model);

        int before = _daemon.Calls.Count;
        await model.ReviewSkillInstallAsync(RecordId);
        Assert.Equal(before, _daemon.Calls.Count);
        Assert.Null(InstallPlanOf(model, RecordId));
        Assert.Equal(SkillLearningPhaseKind.Evaluated, model.State(RecordId).Phase.Kind);
    }

    // SkillLearningTests.swift:597-626.
    [Fact]
    public async Task EvaluationFailureClearsBusyStateAndUsesSharedCopy()
    {
        SkillLearningModel model = Model(_daemon);
        await model.LearnSkillAsync(RecordId);
        await model.ReviewSkillAsync(RecordId, Some(CandidateOf(model, RecordId)).Draft);
        Assert.NotNull(ReviewOf(model, RecordId));
        _daemon.SetFailure("skill_evaluate", "skill-evaluation-request-unavailable");

        await model.TestSkillAsync(RecordId);
        Assert.False(model.State(RecordId).IsWorking);
        Assert.Null(EvaluationOf(model, RecordId));
        Assert.Equal(Line("skill-evaluation-request-unavailable"), model.State(RecordId).Failure);

        _daemon.SetFailure("skill_evaluate", null);
        await model.TestSkillAsync(RecordId);
        Assert.NotNull(EvaluationOf(model, RecordId));
        model.EditSkill(RecordId);
        Assert.Equal(SkillLearningPhaseKind.Candidate, model.State(RecordId).Phase.Kind);
        Assert.Null(model.State(RecordId).Failure);
    }

    // SkillLearningTests.swift:628-654.
    [Fact]
    public async Task TwoSessionWorkflowsStayIsolated()
    {
        SkillLearningModel model = Model(_daemon);

        await Task.WhenAll(model.LearnSkillAsync(RecordId), model.LearnSkillAsync(SecondId));
        Assert.NotNull(CandidateOf(model, RecordId));
        Assert.NotNull(CandidateOf(model, SecondId));

        SkillCandidate firstCandidate = Some(CandidateOf(model, RecordId));
        await model.ReviewSkillAsync(RecordId, firstCandidate.Draft);
        Assert.NotNull(ReviewOf(model, RecordId));

        Assert.Equal(SkillLearningPhaseKind.Reviewed, model.State(RecordId).Phase.Kind);
        Assert.Equal(SkillLearningPhaseKind.Candidate, model.State(SecondId).Phase.Kind);
        Assert.Null(model.State(SecondId).Failure);
        Assert.False(model.State(SecondId).IsWorking);
    }

    // SkillLearningTests.swift:656-690.
    [Fact]
    public async Task CustomNameInstallationRecoversAndRollsBackBySourceLineage()
    {
        const string customName = "repair-generated-artifacts";
        _daemon.SeedInstalledSkill(customName, RecordId);
        SkillLearningModel model = Model(_daemon);

        await model.LearnSkillAsync(RecordId);
        SkillLearningInstall installed = Some(InstalledSkillOf(model, RecordId));
        Assert.Equal(customName, installed.Name);
        Assert.Equal(RecordId, installed.SourceSubmissionId);

        await model.RollbackSkillAsync(RecordId);
        Assert.Equal(SkillLearningPhaseKind.Idle, model.State(RecordId).Phase.Kind);
        Assert.False(model.State(RecordId).IsWorking);
        IReadOnlyList<SkillLearningFakeDaemon.Call> calls = _daemon.Calls;
        SkillLearningFakeDaemon.Call status = calls.First(call => call.Method == "skill_install_status");
        SkillLearningFakeDaemon.Call rollback = calls.First(call => call.Method == "skill_install_rollback");
        Assert.Contains(RecordId, status.Params, StringComparison.Ordinal);
        Assert.DoesNotContain(customName, status.Params, StringComparison.Ordinal);
        Assert.Contains(RecordId, rollback.Params, StringComparison.Ordinal);
        Assert.Contains(installed.InstallId, rollback.Params, StringComparison.Ordinal);
    }

    // SkillLearningTests.swift:692-758, without the two captured renders
    // (the session detail and the History inspector), which are SwiftUI.
    [Fact]
    public async Task WithdrawnSessionRecoversLocalInstallAndRollsBackToIdle()
    {
        _daemon.SeedInstalledSkill("repair-generated-artifacts", RecordId);
        SkillLearningModel model = Model(_daemon);

        await model.EnsureLocalInstalledSkillStatusAsync(RecordId);
        SkillLearningSessionState recovered = model.State(RecordId);
        Assert.False(recovered.IsWorking);
        SkillLearningInstall installed = Some(recovered.SkillLearningInstall);
        Assert.Equal(SkillLearningPhaseKind.Idle, recovered.Phase.Kind);
        Assert.Equal(RecordId, installed.SourceSubmissionId);
        Assert.Equal(new[] { "skill_install_status" }, _daemon.Calls.Select(call => call.Method));

        await model.RollbackSkillAsync(RecordId);
        Assert.Equal(SkillLearningPhaseKind.Idle, model.State(RecordId).Phase.Kind);
        Assert.False(model.State(RecordId).IsWorking);
        Assert.Null(model.State(RecordId).Failure);
        Assert.Contains(_daemon.Calls, call => call.Method == "skill_install_rollback");
        Assert.DoesNotContain(_daemon.Calls, call => call.Method == "skill_candidate");
    }

    // SkillLearningClientTests.swift:20-142.
    [Fact]
    public async Task ClientUsesOpaqueIdsAndTheExactReviewedDigestAcrossEveryMutation()
    {
        var client = new SkillLearningClient(_daemon.CallAsync);
        SkillCandidate candidate = await client.SkillCandidateAsync(RecordId);
        Assert.Equal(8, candidate.EvaluationContract.TaskCount);
        Assert.Equal(6, candidate.EvaluationContract.PlanTaskCount);
        Assert.Equal(7, candidate.EvaluationContract.FixturePoolCount);
        Assert.Equal(7, candidate.EvaluationContract.ClusterCount);
        Assert.Equal(3, candidate.EvaluationContract.ArmCount);
        Assert.Equal(24, candidate.EvaluationContract.TotalRequests);
        Assert.Equal("nearai", candidate.EvaluationContract.RequiredModelOwner);
        Assert.Null(candidate.ReplacesReviewId);
        SkillReview review = await client.ReviewSkillAsync(candidate.CandidateId, candidate.Draft);
        SkillEvaluationReport evaluation = await client.EvaluateSkillAsync(review.ReviewId, review.SkillSha256);
        Assert.Equal(8, evaluation.TaskCount);
        Assert.Equal(6, evaluation.PlanTaskCount);
        Assert.Equal(7, evaluation.FixturePoolCount);
        Assert.Equal(7, evaluation.ClusterCount);
        Assert.Equal(candidate.EvaluationContract, evaluation.EvaluationContract);
        Assert.Equal(3, evaluation.ArmCount);
        Assert.Equal(new string('e', 64), evaluation.SourceTaskFingerprintSha256);
        Assert.Equal(24, evaluation.Trials.Count);
        Assert.Equal(24, evaluation.Trials.Select(trial => trial.Id).Distinct().Count());
        Assert.Equal(7, evaluation.Trials.Select(trial => trial.Cluster).Distinct().Count());
        Assert.All(evaluation.PlanSummaries, summary => Assert.Equal(6, summary.Total));
        Assert.All(evaluation.ApplicabilitySummaries, summary => Assert.Equal(2, summary.Total));
        Assert.Equal(2, evaluation.ApplicabilitySummaries.First(s => s.Arm == SkillEvaluationArm.CandidateSkill).Passed);
        Assert.All(evaluation.Trials, trial =>
        {
            Assert.Equal("https", trial.SourceUrl.Scheme);
            Assert.NotEmpty(trial.RawOutput);
            Assert.False(string.IsNullOrEmpty(trial.RequestId));
        });
        var trialsByTask = evaluation.Trials.GroupBy(trial => trial.TaskId).ToList();
        Assert.Equal(8, trialsByTask.Count);
        foreach (var trials in trialsByTask)
        {
            Assert.Equal(
                new[] { SkillEvaluationArm.Baseline, SkillEvaluationArm.ManualInstruction, SkillEvaluationArm.CandidateSkill },
                trials.Select(trial => trial.Arm).Distinct().OrderBy(arm => arm));
        }

        var applicabilityTrials = evaluation.Trials.Where(trial => trial.Cluster == "metadata-applicability").ToList();
        Assert.Equal(6, applicabilityTrials.Count);
        Assert.All(applicabilityTrials, trial => Assert.Null(trial.Answer));
        var planTrials = evaluation.Trials.Where(trial => trial.Cluster != "metadata-applicability").ToList();
        Assert.Equal(18, planTrials.Count);
        Assert.All(planTrials, trial => Assert.NotNull(trial.Answer));
        foreach (SkillArmSummary summary in evaluation.Summaries)
        {
            var matching = evaluation.Trials.Where(trial => trial.Arm == summary.Arm).ToList();
            Assert.Equal(matching.Count, summary.Total);
            Assert.Equal(matching.Count(trial => trial.Passed), summary.Passed);
        }

        var summaries = evaluation.Summaries.ToDictionary(summary => summary.Arm, summary => summary.Passed);
        int candidatePasses = summaries[SkillEvaluationArm.CandidateSkill];
        Assert.True(candidatePasses > summaries[SkillEvaluationArm.Baseline]);
        Assert.True(candidatePasses > summaries[SkillEvaluationArm.ManualInstruction]);
        Assert.Equal(
            "https://github.com/TraceCommons/trace-commons/commit/b6722426bb4b83d90425494b664ac468d67943b5",
            evaluation.Trials.First(trial => trial.TaskId == "mark-msix-scale-ladder").SourceUrl.OriginalString);
        SkillInstallPlan plan = await client.SkillInstallPlanAsync(evaluation.EvaluationId);
        Assert.Equal(plan.TargetLocation + "/SKILL.md", plan.SkillLocation);
        Assert.Equal(plan.TargetLocation + "/.trace-commons-install.json", plan.MarkerLocation);
        Assert.DoesNotContain("/Users/", plan.TargetLocation, StringComparison.Ordinal);
        Assert.Equal(64, plan.MarkerFileSha256.Length);
        Assert.Contains("\"source_submission_id\"", plan.MarkerJson, StringComparison.Ordinal);
        Assert.Contains(candidate.SourceSubmissionId, plan.MarkerJson, StringComparison.Ordinal);
        SkillLearningInstall installed = await client.InstallSkillAsync(plan.PlanId, plan.SkillSha256, plan.MarkerFileSha256);
        SkillInstallStatus status = await client.SkillInstallStatusAsync(candidate.SourceSubmissionId);
        Assert.Equal(installed, status.Skill);
        Assert.True((await client.RollbackSkillAsync(installed.InstallId, installed.SourceSubmissionId)).Removed);

        IReadOnlyList<SkillLearningFakeDaemon.Call> calls = _daemon.Calls;
        Assert.Equal(
            new[]
            {
                "skill_candidate", "skill_review", "skill_evaluate", "skill_install_plan",
                "skill_install_commit", "skill_install_status", "skill_install_rollback",
            },
            calls.Select(call => call.Method));
        var parameterSets = calls.Select(Params).ToList();
        Assert.Equal(Sorted("submission_id"), Keys(parameterSets[0]));
        Assert.Equal(Sorted("review_id", "skill_sha256"), Keys(parameterSets[2]));
        Assert.Equal(
            Sorted("plan_id", "as_previewed_sha256", "as_previewed_marker_sha256"),
            Keys(parameterSets[4]));
        Assert.Equal(plan.MarkerFileSha256, Text(parameterSets[4], "as_previewed_marker_sha256"));
        Assert.Equal(Sorted("source_submission_id"), Keys(parameterSets[5]));
        Assert.Equal(Sorted("install_id", "source_submission_id"), Keys(parameterSets[6]));
        Assert.DoesNotContain(calls.Skip(1), call => call.Params.Contains(candidate.SourceCorrection, StringComparison.Ordinal));
    }

    // SkillLearningRenderingTests.swift:51-94: the stage renders dropped,
    // the state each settles in kept.
    [Fact]
    public async Task InstallCommitReplyLossRecoversReceiptAndSupportsRollback()
    {
        SkillLearningModel model = Model(_daemon);
        await ThroughEvaluation(model);
        await model.ReviewSkillInstallAsync(RecordId);
        Assert.NotNull(InstallPlanOf(model, RecordId));

        int statusCallsBeforeCommit = StatusCallSubmissionIds().Count;
        _daemon.SetInstallCommitReplyFailure("skill-install-write-failed");
        await model.InstallSkillAsync(RecordId);
        Assert.False(model.State(RecordId).IsWorking);

        SkillLearningInstall recovered = Some(InstalledSkillOf(model, RecordId));
        Assert.Null(model.State(RecordId).Failure);
        Assert.Equal(statusCallsBeforeCommit + 1, StatusCallSubmissionIds().Count);
        Assert.Equal(
            new[] { "skill_install_commit", "skill_install_status" },
            _daemon.Calls.TakeLast(2).Select(call => call.Method));

        await model.RollbackSkillAsync(RecordId);
        Assert.Null(InstalledSkillOf(model, RecordId));
        Assert.False(model.State(RecordId).IsWorking);
        SkillLearningFakeDaemon.Call last = _daemon.Calls[^1];
        Assert.Equal("skill_install_rollback", last.Method);
        Assert.Contains(recovered.InstallId, last.Params, StringComparison.Ordinal);
    }

    // SkillLearningRenderingTests.swift:96-138. The Mac hosts the view for
    // each session, whose appearance reads the status
    // (SkillLearningView.swift:52-54); here that call is made directly.
    [Fact]
    public async Task AcceptedSessionFirstOpenRecoversInstallAfterRestartPerSession()
    {
        _daemon.SeedInstalledSkill("repair-generated-artifacts", RecordId);
        SkillLearningModel restartedModel = Model(_daemon);

        await restartedModel.EnsureLocalInstalledSkillStatusAsync(RecordId);
        Assert.NotNull(InstalledSkillOf(restartedModel, RecordId));

        await restartedModel.EnsureLocalInstalledSkillStatusAsync(SecondId);
        Assert.False(restartedModel.State(SecondId).IsWorking);

        Assert.Equal("repair-generated-artifacts", InstalledSkillOf(restartedModel, RecordId)?.Name);
        Assert.Equal(SkillLearningPhaseKind.Idle, restartedModel.State(SecondId).Phase.Kind);
        Assert.Equal(new[] { RecordId, SecondId }, StatusCallSubmissionIds());
        Assert.DoesNotContain(_daemon.Calls, call => call.Method == "skill_candidate");
    }

    // New (no Mac counterpart): a commit the daemon refuses, with nothing
    // written, settles through the status read to nothing installed, back on
    // the evaluation, showing the refusal's line (AppModel+SkillLearning.swift:228-240).
    [Fact]
    public async Task RefusedCommitLeavesNothingInstalled()
    {
        SkillLearningModel model = Model(_daemon);
        await ThroughEvaluation(model);
        await model.ReviewSkillInstallAsync(RecordId);
        Assert.NotNull(InstallPlanOf(model, RecordId));

        _daemon.SetFailure("skill_install_commit", "skill-install-write-failed");
        await model.InstallSkillAsync(RecordId);

        SkillLearningSessionState state = model.State(RecordId);
        Assert.False(state.IsWorking);
        Assert.Equal(SkillLearningPhaseKind.Evaluated, state.Phase.Kind);
        Assert.Null(state.SkillLearningInstall);
        Assert.True(state.Installation.IsAbsent);
        Assert.Equal(Line("skill-install-write-failed"), state.Failure);
        Assert.Equal(
            new[] { "skill_install_commit", "skill_install_status" },
            _daemon.Calls.TakeLast(2).Select(call => call.Method));
    }

    // New (no Mac counterpart): a commit the daemon answered is not taken on
    // its word. When the status read after it says nothing is installed,
    // nothing is shown installed (AppModel+SkillLearning.swift:198-252).
    [Fact]
    public async Task AnAnsweredCommitTheStatusDoesNotConfirmLeavesNothingInstalled()
    {
        SkillLearningModel model = Model(_daemon);
        await ThroughEvaluation(model);
        await model.ReviewSkillInstallAsync(RecordId);
        _daemon.SetInstallStatusShape(installed: false, includesSkill: false);
        await model.InstallSkillAsync(RecordId);

        SkillLearningSessionState state = model.State(RecordId);
        Assert.False(state.IsWorking);
        Assert.Null(state.SkillLearningInstall);
        Assert.Equal(Line("skill-source-lineage-invalid"), state.Failure);
        Assert.Equal(
            new[] { "skill_install_commit", "skill_install_status" },
            _daemon.Calls.TakeLast(2).Select(call => call.Method));
    }

    // New (no Mac counterpart): every parameter object the model sends is
    // exactly the Mac's (DaemonClient+SkillLearning.swift:7-90), values
    // included; a first review carries no replaces_review_id at all.
    [Fact]
    public async Task TheModelSendsExactlyTheMacsParameters()
    {
        SkillLearningModel model = Model(_daemon);
        await ThroughInstall(model);
        SkillLearningInstall installed = Some(InstalledSkillOf(model, RecordId));
        await model.RollbackSkillAsync(RecordId);

        IReadOnlyList<SkillLearningFakeDaemon.Call> calls = _daemon.Calls;
        Assert.Equal(
            new[]
            {
                "skill_install_status", "skill_candidate", "skill_review", "skill_evaluate", "skill_install_plan",
                "skill_install_commit", "skill_install_status", "skill_install_rollback",
            },
            calls.Select(call => call.Method));
        string sha = new('a', 64);
        AssertParams(calls[0], ("source_submission_id", RecordId));
        AssertParams(calls[1], ("submission_id", RecordId));
        JsonObject review = Params(calls[2]);
        Assert.Equal(Sorted("candidate_id", "draft"), Keys(review));
        Assert.Equal("11111111-1111-4111-8111-111111111111", Text(review, "candidate_id"));
        JsonObject draft = Assert.IsType<JsonObject>(review["draft"]);
        Assert.Equal(Sorted("name", "description", "procedure"), Keys(draft));
        Assert.Equal("repair-generated-sources", Text(draft, "name"));
        AssertParams(calls[3], ("review_id", "44444444-4444-4444-8444-444444444444"), ("skill_sha256", sha));
        AssertParams(calls[4], ("evaluation_id", "55555555-5555-4555-8555-555555555555"));
        AssertParams(
            calls[5],
            ("plan_id", "66666666-6666-4666-8666-666666666666"),
            ("as_previewed_sha256", sha),
            ("as_previewed_marker_sha256", new string('b', 64)));
        AssertParams(calls[6], ("source_submission_id", RecordId));
        AssertParams(calls[7], ("install_id", installed.InstallId), ("source_submission_id", RecordId));
        Assert.Equal(new DateTimeOffset(2026, 9, 11, 5, 0, 0, TimeSpan.Zero), installed.InstalledAt);
    }

    // New (no Mac counterpart): the copy is refused whole when a word is
    // missing, as the Mac's Decodable refuses it (SkillLearningModels.swift:75-80).
    [Fact]
    public void TheCopyIsRefusedWhenAnyWordIsMissing()
    {
        Assert.Equal("word:rollback_incomplete", Copy.RollbackIncomplete);
        Assert.Equal("word:unavailable", Copy.Unavailable);
        var partial = new JsonObject(SkillLearningCopy.RequiredWords.Skip(1).Select(key =>
            KeyValuePair.Create(key, (JsonNode?)JsonValue.Create(key))));
        Assert.Null(SkillLearningCopy.Parse(partial.ToJsonString()));
        Assert.Null(SkillLearningCopy.Parse(null));
    }

    // ComputeSkillsParityTests.swift:177-198.
    [Fact]
    public void LearningIsOfferedOnlyOnAnActiveCorrectedContribution()
    {
        static bool Offers(SessionDetail? detail, string recordStatus = "accepted", bool withdrawnHere = false)
        {
            bool done = ContributionGate.WithdrawalCompleted(recordStatus, withdrawnHere);
            return SkillLearningGate.OffersLearning(detail, recordStatus, done);
        }

        Assert.True(Offers(Detail()));
        Assert.False(Offers(null));
        Assert.False(Offers(Detail(correction: null)));
        Assert.False(Offers(Detail(taskSuccess: null)));
        Assert.False(Offers(Detail(status: "rejected"), recordStatus: "rejected"));
        Assert.False(Offers(Detail(status: null)));
        Assert.False(Offers(Detail(status: "withdrawn")));
        Assert.False(Offers(Detail(), recordStatus: "withdrawn"));
        Assert.False(Offers(Detail(), withdrawnHere: true));
        // An unknown status reads "Status unavailable" and is terminal.
        Assert.False(Offers(Detail(), recordStatus: "status-from-a-newer-daemon"));
        Assert.False(Offers(Detail(status: "status-from-a-newer-daemon")));
    }

    // ComputeSkillsParityTests.swift:200-216.
    [Fact]
    public void TheInstalledSurfaceShowsOutsideTheLearningRule()
    {
        Assert.True(SkillLearningGate.ShowsInstalledSurface(null, withdrawalCompleted: false));
        Assert.True(SkillLearningGate.ShowsInstalledSurface(Detail(), withdrawalCompleted: true));
        Assert.True(SkillLearningGate.ShowsInstalledSurface(Detail(status: "revoked"), withdrawalCompleted: false));
        Assert.False(SkillLearningGate.ShowsInstalledSurface(Detail(), withdrawalCompleted: false));

        var failed = new SkillLearningSessionState { WorkflowFailure = "read failed" };
        Assert.True(SkillLearningGate.OffersInstallStatusRetry(failed, null, "withdrawn"));
        Assert.True(SkillLearningGate.OffersInstallStatusRetry(failed, Detail(status: "rejected"), "rejected"));
        Assert.False(SkillLearningGate.OffersInstallStatusRetry(failed, Detail(), "accepted"));
        Assert.False(SkillLearningGate.OffersInstallStatusRetry(new SkillLearningSessionState(), null, "withdrawn"));
    }

    /// <summary>The Mac's <c>detail(status:taskSuccess:correction:)</c> (ComputeSkillsParityTests.swift:164-175).</summary>
    private static SessionDetail Detail(
        string? status = "accepted", string? taskSuccess = "success", string? correction = "Use the fixture.")
    {
        var json = new JsonObject
        {
            ["evidence"] = new JsonArray(),
            ["contributed_version"] = "1",
            ["consent_policy_version"] = "1",
            ["redaction_pipeline_version"] = "1",
            ["publication_version"] = 0,
        };
        if (status is not null)
        {
            json["contribution_status"] = status;
        }

        if (taskSuccess is not null)
        {
            json["task_success"] = taskSuccess;
        }

        if (correction is not null)
        {
            json["human_correction"] = correction;
        }

        using JsonDocument doc = JsonDocument.Parse(json.ToJsonString());
        return Some(SessionDetail.Parse(doc.RootElement));
    }

    private static async Task ThroughEvaluation(SkillLearningModel model)
    {
        await model.LearnSkillAsync(RecordId);
        await model.ReviewSkillAsync(RecordId, Some(CandidateOf(model, RecordId)).Draft);
        Assert.NotNull(ReviewOf(model, RecordId));
        await model.TestSkillAsync(RecordId);
        Assert.NotNull(EvaluationOf(model, RecordId));
    }

    private static async Task ThroughInstall(SkillLearningModel model)
    {
        await ThroughEvaluation(model);
        await model.ReviewSkillInstallAsync(RecordId);
        Assert.NotNull(InstallPlanOf(model, RecordId));
        await model.InstallSkillAsync(RecordId);
    }

    private static T Some<T>(T? value)
        where T : class
    {
        Assert.NotNull(value);
        return value!;
    }

    private static SkillCandidate? CandidateOf(SkillLearningModel model, string id) => model.State(id).Phase.Candidate;

    private static SkillReview? ReviewOf(SkillLearningModel model, string id) => model.State(id).Phase.Review;

    private static SkillEvaluationReport? EvaluationOf(SkillLearningModel model, string id) => model.State(id).Phase.Report;

    private static SkillInstallPlan? InstallPlanOf(SkillLearningModel model, string id) =>
        model.State(id).Phase.Kind == SkillLearningPhaseKind.Planned ? model.State(id).Phase.Plan : null;

    private static SkillLearningInstall? InstalledSkillOf(SkillLearningModel model, string id) => model.State(id).SkillLearningInstall;

    private List<string> StatusCallSubmissionIds() =>
        _daemon.Calls
            .Where(call => call.Method == "skill_install_status")
            .Select(call => Text(Params(call), "source_submission_id"))
            .OfType<string>()
            .ToList();

    private static JsonObject Params(SkillLearningFakeDaemon.Call call) =>
        Assert.IsType<JsonObject>(JsonNode.Parse(call.Params));

    private static string? Text(JsonObject parameters, string key) =>
        parameters[key] is JsonValue value && value.TryGetValue(out string? text) ? text : null;

    private static string[] Keys(JsonObject parameters) =>
        parameters.Select(pair => pair.Key).OrderBy(key => key, StringComparer.Ordinal).ToArray();

    private static string[] Sorted(params string[] keys) => keys.OrderBy(key => key, StringComparer.Ordinal).ToArray();

    private static void AssertParams(SkillLearningFakeDaemon.Call call, params (string Key, string Value)[] expected)
    {
        JsonObject parameters = Params(call);
        Assert.Equal(Sorted(expected.Select(pair => pair.Key).ToArray()), Keys(parameters));
        foreach ((string key, string value) in expected)
        {
            Assert.Equal(value, Text(parameters, key));
        }
    }
}

/// <summary>
/// The daemon double the Mac's tested-skill tests share
/// (<c>SkillLearningDaemon</c>, SkillLearningTestSupport.swift:9-568):
/// records each call, answers each method with the Mac's fixtures as a JSON
/// frame, and can be scripted to refuse, to lose a commit's reply, to fail
/// the gate, to find the target occupied, or to override one reply field.
/// Every reply is built from fresh nodes.
/// </summary>
internal sealed class SkillLearningFakeDaemon
{
    private const string DefaultSource = "22222222-2222-4222-8222-222222222222";

    private readonly object _lock = new();
    private readonly List<Call> _recorded = new();
    private readonly Dictionary<string, string> _failures = new(StringComparer.Ordinal);
    private readonly Dictionary<string, Dictionary<string, string>> _overrides = new(StringComparer.Ordinal);
    private string? _installCommitReplyFailure;
    private bool _installed;
    private string _installedName = "repair-generated-sources";
    private string _installedSourceSubmissionId = DefaultSource;
    private string _lastSourceSubmissionId = DefaultSource;
    private bool _gatePasses = true;
    private bool _installOccupied;
    private bool? _reportedInstallStatus;
    private bool? _reportedInstallSkillPresence;
    private SkillDraft? _recoveredDraft;
    private string? _recoveredReviewId;

    internal sealed record Call(string Method, string Params);

    public IReadOnlyList<Call> Calls
    {
        get
        {
            lock (_lock)
            {
                return _recorded.ToList();
            }
        }
    }

    public void SetFailure(string method, string? label)
    {
        lock (_lock)
        {
            if (label is null)
            {
                _failures.Remove(method);
            }
            else
            {
                _failures[method] = label;
            }
        }
    }

    /// <summary>A daemon that commits the skill to disk, then loses the reply before the client receives its receipt.</summary>
    public void SetInstallCommitReplyFailure(string? label)
    {
        lock (_lock)
        {
            _installCommitReplyFailure = label;
        }
    }

    public void SetGatePasses(bool passes)
    {
        lock (_lock)
        {
            _gatePasses = passes;
        }
    }

    public void SetInstallOccupied(bool occupied)
    {
        lock (_lock)
        {
            _installOccupied = occupied;
        }
    }

    public void SetResponseField(string method, string field, string? value)
    {
        lock (_lock)
        {
            if (value is not null)
            {
                if (!_overrides.TryGetValue(method, out Dictionary<string, string>? fields))
                {
                    fields = new Dictionary<string, string>(StringComparer.Ordinal);
                    _overrides[method] = fields;
                }

                fields[field] = value;
            }
            else if (_overrides.TryGetValue(method, out Dictionary<string, string>? fields))
            {
                fields.Remove(field);
                if (fields.Count == 0)
                {
                    _overrides.Remove(method);
                }
            }
        }
    }

    public void SetInstalled(bool value)
    {
        lock (_lock)
        {
            _installed = value;
        }
    }

    public void SetInstallStatusShape(bool? installed, bool? includesSkill)
    {
        lock (_lock)
        {
            _reportedInstallStatus = installed;
            _reportedInstallSkillPresence = includesSkill;
        }
    }

    public void SeedInstalledSkill(string name, string sourceSubmissionId)
    {
        lock (_lock)
        {
            _installed = true;
            _installedName = name;
            _installedSourceSubmissionId = sourceSubmissionId;
        }
    }

    public void SeedRecoveredReview(SkillDraft draft, string reviewId)
    {
        lock (_lock)
        {
            _recoveredDraft = draft;
            _recoveredReviewId = reviewId;
        }
    }

    public Task<DaemonResponse> CallAsync(string method, string parameters) =>
        Task.FromResult(DaemonResponse.Parse(Frame(method, parameters)));

    private string Frame(string method, string paramsJson)
    {
        lock (_lock)
        {
            _recorded.Add(new Call(method, paramsJson));
            if (_failures.TryGetValue(method, out string? label))
            {
                return ErrorFrame(label);
            }

            JsonObject parameters = JsonNode.Parse(paramsJson) as JsonObject ?? new JsonObject();
            JsonNode result;
            switch (method)
            {
                case "skill_candidate":
                    string source = Text(parameters, "submission_id") ?? _lastSourceSubmissionId;
                    _lastSourceSubmissionId = source;
                    result = Candidate(source);
                    break;
                case "skill_install_status":
                    bool matches = _installed && Text(parameters, "source_submission_id") == _installedSourceSubmissionId;
                    bool includesSkill = _reportedInstallSkillPresence ?? matches;
                    result = new JsonObject
                    {
                        ["installed"] = _reportedInstallStatus ?? matches,
                        ["skill"] = includesSkill ? InstalledSkillNode() : null,
                    };
                    break;
                case "skill_review":
                    JsonObject review = Review();
                    if (parameters["draft"] is JsonObject submitted)
                    {
                        review["draft"] = JsonNode.Parse(submitted.ToJsonString());
                    }

                    result = review;
                    break;
                case "skill_evaluate":
                    result = Evaluation();
                    break;
                case "skill_install_plan":
                    result = InstallPlan();
                    break;
                case "skill_install_commit":
                    _installed = true;
                    _installedSourceSubmissionId = _lastSourceSubmissionId;
                    if (_installCommitReplyFailure is { } lost)
                    {
                        return ErrorFrame(lost);
                    }

                    result = InstalledSkillNode();
                    break;
                case "skill_install_rollback":
                    bool owned = Text(parameters, "source_submission_id") == _installedSourceSubmissionId;
                    if (owned)
                    {
                        _installed = false;
                    }

                    result = new JsonObject { ["removed"] = owned, ["retained_directory"] = false };
                    break;
                default:
                    return ErrorFrame("unexpected-test-method");
            }

            if (_overrides.TryGetValue(method, out Dictionary<string, string>? fields) && result is JsonObject replaced)
            {
                foreach (KeyValuePair<string, string> field in fields)
                {
                    replaced[field.Key] = field.Value;
                }
            }

            return new JsonObject { ["id"] = 1, ["result"] = result }.ToJsonString();
        }
    }

    private static string ErrorFrame(string label) =>
        new JsonObject
        {
            ["id"] = 1,
            ["error"] = new JsonObject { ["code"] = "unavailable", ["message"] = label },
        }.ToJsonString();

    private static string? Text(JsonObject parameters, string key) =>
        parameters[key] is JsonValue value && value.TryGetValue(out string? text) ? text : null;

    private static JsonArray Strings(params string[] items) =>
        new(items.Select(item => (JsonNode?)JsonValue.Create(item)).ToArray());

    private const string SkillMd = "---\nname: repair-generated-sources\ndescription: test\n---\n\nEdit the source.";

    private static JsonObject Draft() => new()
    {
        ["name"] = "repair-generated-sources",
        ["description"] = "Use when a generated file has an authoritative source.",
        ["procedure"] = "# Procedure\n\nEdit the source, regenerate, and verify drift.",
    };

    private static JsonObject EvaluationContract() => new()
    {
        ["task_count"] = 8,
        ["plan_task_count"] = 6,
        ["fixture_pool_count"] = 7,
        ["cluster_count"] = 7,
        ["arm_count"] = 3,
        ["total_requests"] = 24,
        ["output_token_limit"] = 900,
        ["request_timeout_seconds"] = 90,
        ["required_model_owner"] = "nearai",
        ["fixture_scope"] = "Six source-excluded client-shipped public repository incidents plus two metadata-only applicability probes; no session task, correction, or evidence is sent.",
        ["selection_policy"] = "Select six incidents from the fixed public fixture pool after excluding source-task overlap.",
    };

    private JsonObject Candidate(string sourceSubmissionId)
    {
        var value = new JsonObject
        {
            ["candidate_id"] = "11111111-1111-4111-8111-111111111111",
            ["family"] = "generated-source-repair",
            ["source_submission_id"] = sourceSubmissionId,
            ["source_correction"] = "Change the source schema and regenerate the generated file.",
            ["source_evidence"] = new JsonArray(new JsonObject
            {
                ["event_id"] = "33333333-3333-4333-8333-333333333333",
                ["kind"] = "feedback",
                ["excerpt"] = "The source schema owns this generated output.",
            }),
            ["draft"] = Draft(),
            ["manual_control_instruction"] = "Edit the source and regenerate generated files.",
            ["evaluation_contract"] = EvaluationContract(),
        };
        if (_recoveredDraft is { } recovered)
        {
            value["draft"] = recovered.ToNode();
        }

        if (_recoveredReviewId is { } reviewId)
        {
            value["replaces_review_id"] = reviewId;
        }

        return value;
    }

    private static JsonObject Review() => new()
    {
        ["review_id"] = "44444444-4444-4444-8444-444444444444",
        ["candidate_id"] = "11111111-1111-4111-8111-111111111111",
        ["family"] = "generated-source-repair",
        ["source_submission_id"] = "22222222-2222-4222-8222-222222222222",
        ["source_evidence_ids"] = Strings("33333333-3333-4333-8333-333333333333"),
        ["draft"] = Draft(),
        ["skill_md"] = SkillMd,
        ["skill_sha256"] = new string('a', 64),
    };

    private sealed record TrialRow(string TaskId, string Cluster, string Arm, bool Passed, JsonObject Node);

    private JsonObject Evaluation()
    {
        List<TrialRow> trials = TrialRows();
        var arms = new[]
        {
            (Arm: "baseline", Label: "Baseline"),
            (Arm: "manual_instruction", Label: "Simple instruction"),
            (Arm: "candidate_skill", Label: "Candidate skill"),
        };
        JsonArray Summarize(IEnumerable<TrialRow> selected)
        {
            List<TrialRow> rows = selected.ToList();
            return new JsonArray(arms.Select(arm =>
            {
                List<TrialRow> matching = rows.Where(row => row.Arm == arm.Arm).ToList();
                return (JsonNode?)new JsonObject
                {
                    ["arm"] = arm.Arm,
                    ["label"] = arm.Label,
                    ["passed"] = matching.Count(row => row.Passed),
                    ["total"] = matching.Count,
                };
            }).ToArray());
        }

        bool IsApplicability(TrialRow row) => row.Cluster == "metadata-applicability";

        JsonObject value = EvaluationContract();
        value["evaluation_id"] = "55555555-5555-4555-8555-555555555555";
        value["review_id"] = "44444444-4444-4444-8444-444444444444";
        value["skill_sha256"] = new string('a', 64);
        value["requested_model"] = "deepseek-ai/DeepSeek-V4-Flash";
        value["served_model"] = "deepseek-ai/DeepSeek-V4-Flash";
        value["model_owned_by"] = "nearai";
        value["source_task_fingerprint_sha256"] = new string('e', 64);
        value["selected_plan_task_ids"] = Strings(trials.Where(row => !IsApplicability(row)).Select(row => row.TaskId).Distinct().ToArray());
        value["excluded_source_overlap_task_ids"] = new JsonArray();
        value["reserve_plan_task_ids"] = Strings("contributor-release-version-stamp");
        value["ambient_context"] = Strings("public immutable repository evidence", "metadata-only applicability probes");
        value["summaries"] = Summarize(trials);
        value["plan_summaries"] = Summarize(trials.Where(row => !IsApplicability(row)));
        value["applicability_summaries"] = Summarize(trials.Where(IsApplicability));
        value["trials"] = new JsonArray(trials.Select(row => (JsonNode?)row.Node).ToArray());
        value["regressions"] = new JsonArray();
        value["install_allowed"] = _gatePasses;
        value["gate_reason"] = _gatePasses ? "skill-improved-over-both-controls" : "skill-did-not-beat-both-controls";
        return value;
    }

    private List<TrialRow> TrialRows()
    {
        var rows = new List<TrialRow>();
        foreach (TrialFixture fixture in TrialFixtures())
        {
            foreach (string arm in new[] { "baseline", "manual_instruction", "candidate_skill" })
            {
                bool passed = arm switch
                {
                    "baseline" => fixture.BaselinePasses,
                    "manual_instruction" => fixture.ManualPasses,
                    _ => _gatePasses || fixture.CandidatePassesWhenGateFails,
                };

                JsonNode? answer;
                string rawOutput;
                if (fixture.GuidanceShouldApply is { } guidanceShouldApply)
                {
                    answer = null;
                    bool applicable = arm != "baseline" && guidanceShouldApply;
                    rawOutput = new JsonObject { ["applicable"] = applicable }.ToJsonString();
                }
                else
                {
                    PlanData? plan = passed ? fixture.PassingAnswer : fixture.FailingAnswer;
                    answer = PlanNode(plan);
                    rawOutput = PlanNode(plan).ToJsonString();
                }

                var node = new JsonObject
                {
                    ["task_id"] = fixture.TaskId,
                    ["cluster"] = fixture.Cluster,
                    ["task"] = fixture.Task,
                    ["source_url"] = fixture.SourceUrl,
                    ["arm"] = arm,
                    ["passed"] = passed,
                    ["failure_reasons"] = passed
                        ? new JsonArray()
                        : Strings("The plan omitted the authoritative source or a required regeneration step."),
                    ["answer"] = answer,
                    ["raw_output"] = rawOutput,
                    ["request_id"] = "request-" + fixture.TaskId + "-" + arm,
                    ["served_model"] = "deepseek-ai/DeepSeek-V4-Flash",
                    ["finish_reason"] = "stop",
                    ["usage"] = new JsonObject
                    {
                        ["prompt_tokens"] = 200,
                        ["completion_tokens"] = 100,
                        ["reasoning_tokens"] = 0,
                        ["total_tokens"] = 300,
                    },
                };
                rows.Add(new TrialRow(fixture.TaskId, fixture.Cluster, arm, passed, node));
            }
        }

        return rows;
    }

    /// <summary>A proposed plan; null is the Mac's empty <c>noPlan</c>.</summary>
    private sealed record PlanData(string Diagnosis, string[] EditPaths, string[] Commands, string[] Verification);

    private static JsonObject PlanNode(PlanData? plan) => plan is null
        ? new JsonObject()
        : new JsonObject
        {
            ["diagnosis"] = plan.Diagnosis,
            ["edit_paths"] = Strings(plan.EditPaths),
            ["commands"] = Strings(plan.Commands),
            ["verification"] = Strings(plan.Verification),
        };

    private sealed record TrialFixture(
        string TaskId,
        string Cluster,
        string Task,
        string SourceUrl,
        bool? GuidanceShouldApply,
        PlanData? PassingAnswer,
        PlanData? FailingAnswer,
        bool BaselinePasses,
        bool ManualPasses,
        bool CandidatePassesWhenGateFails);

    private static IReadOnlyList<TrialFixture> TrialFixtures()
    {
        var incompletePlan = new PlanData(
            "Edit the checked-in output directly.", new[] { "generated/output" }, Array.Empty<string>(), Array.Empty<string>());
        return new[]
        {
            new TrialFixture(
                "discovery-positive-generated-source",
                "metadata-applicability",
                "Agent Skill metadata applicability: A checked-in Windows package image is generated from shared Rust source; change its background and regenerate every scale variant.",
                "https://github.com/TraceCommons/trace-commons",
                true, null, null, true, true, true),
            new TrialFixture(
                "discovery-negative-handwritten-docs",
                "metadata-applicability",
                "Agent Skill metadata applicability: Correct punctuation in a hand-written README paragraph. The file is not generated and no derived output changes.",
                "https://github.com/TraceCommons/trace-commons",
                false, null, null, true, true, true),
            new TrialFixture(
                "mark-msix-scale-ladder",
                "mark-assets",
                "Change the Windows package tile background while keeping all three base assets and their four display-scale variants consistent.",
                "https://github.com/TraceCommons/trace-commons/commit/b6722426bb4b83d90425494b664ac468d67943b5",
                null,
                new PlanData(
                    "The shared mark library generates every committed tile variant.",
                    new[] { "crates/trace-commons-mark/src/lib.rs" },
                    new[] { "cargo run -p trace-commons-mark --bin mark-export -- assets/mark --repo-root ." },
                    new[] { "scripts/mark/check-drift.sh" }),
                incompletePlan, false, true, true),
            new TrialFixture(
                "witness-compose-image-pin",
                "witness-compose",
                "Replace the witness image digest and keep the measured app-compose deployment document current.",
                "https://github.com/TraceCommons/trace-commons/commit/c443326564264b27f5c65cd385b89762e7a81ea1",
                null,
                new PlanData(
                    "The compose file owns the image pin and generates app-compose.json.",
                    new[] { "deploy/witness/docker-compose.yml" },
                    new[] { "deploy/witness/build-app-compose.sh" },
                    new[] { "deploy/witness/build-app-compose.sh --check" }),
                incompletePlan, false, true, true),
            new TrialFixture(
                "flatpak-ironwire-pin",
                "flatpak-sources",
                "Advance the pinned IronWire proxy dependency to a reviewed revision and keep both lockfiles and the offline Flatpak vendor source set synchronized.",
                "https://github.com/TraceCommons/trace-commons/commit/e89a628bf4be8b1e31dd278793b00391e9781f1c",
                null,
                new PlanData(
                    "The contributor dependency pin owns two lockfiles and the generated offline source set.",
                    new[] { "crates/trace-commons-contributor/Cargo.toml" },
                    new[]
                    {
                        "cargo update -p ironwire_proxy",
                        "cargo update --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml -p ironwire_proxy",
                        "python3 flatpak-cargo-generator.py crates/trace-commons-contributor-gtk/Cargo.lock -o crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json",
                    },
                    new[] { "cargo test -p trace-commons-contributor --test release_pipeline" }),
                incompletePlan, false, false, false),
            new TrialFixture(
                "winget-publisher",
                "winget-manifests",
                "Change the publisher shown in the generated WinGet release manifests without allowing the singleton, version, and locale documents to drift apart.",
                "https://github.com/TraceCommons/trace-commons/commit/c5786be30bf4135ca0d5e63e5b8ee04f02acbc04",
                null,
                new PlanData(
                    "The generator owns all three versioned WinGet manifests.",
                    new[] { "scripts/winget/generate-manifests.sh" },
                    new[] { "scripts/winget/generate-manifests.sh 0.12.2" },
                    new[] { "winget validate --manifest manifests/t/TraceCommons/Contributor/0.12.2" }),
                incompletePlan, false, false, false),
            new TrialFixture(
                "png-encoder-foreign-decoder",
                "png-encoding",
                "Reduce the generated Windows tile PNG size by changing the shared encoder, regenerate all committed base tiles, and verify the bytes with an independent decoder.",
                "https://github.com/TraceCommons/trace-commons/commit/13f53d4e2c453a0ba15de2ce7d8c6f77eb33a77b",
                null,
                new PlanData(
                    "The shared raster encoder owns every committed base tile.",
                    new[] { "crates/trace-commons-mark/src/raster.rs" },
                    new[] { "cargo run -p trace-commons-mark --bin mark-export -- assets/mark --repo-root ." },
                    new[] { "python3 scripts/mark/verify-png.py target/mark-verify" }),
                incompletePlan, false, true, false),
            new TrialFixture(
                "update-conformance-platform",
                "update-conformance",
                "Change the artifact URL origin in the deterministic update-conformance generator and regenerate every signed manifest fixture without hand-editing its signatures.",
                "https://github.com/TraceCommons/trace-commons/commit/ddbfd593338541cf67af6cbd50d3af50de492ee2",
                null,
                new PlanData(
                    "The conformance generator owns each manifest and signature fixture.",
                    new[] { "tests/fixtures/update-conformance/regenerate.sh" },
                    new[] { "tests/fixtures/update-conformance/regenerate.sh" },
                    new[] { "cargo test -p trace-commons-contributor --test update_conformance" }),
                incompletePlan, false, false, false),
        };
    }

    private JsonObject InstallPlan()
    {
        const string target = "$CODEX_HOME/skills/repair-generated-sources";
        return new JsonObject
        {
            ["plan_id"] = "66666666-6666-4666-8666-666666666666",
            ["evaluation_id"] = "55555555-5555-4555-8555-555555555555",
            ["tool"] = "Codex",
            ["target_location"] = target,
            ["skill_location"] = target + "/SKILL.md",
            ["marker_location"] = target + "/.trace-commons-install.json",
            ["occupied"] = _installOccupied,
            ["can_install"] = !_installOccupied,
            ["skill_md"] = SkillMd,
            ["skill_sha256"] = new string('a', 64),
            ["marker_json"] = MarkerJson(),
            ["marker_file_sha256"] = new string('b', 64),
        };
    }

    private string MarkerJson() => new JsonObject
    {
        ["schema_version"] = 2,
        ["install_id"] = "77777777-7777-4777-8777-777777777777",
        ["evaluation_id"] = "55555555-5555-4555-8555-555555555555",
        ["tool"] = "Codex",
        ["name"] = "repair-generated-sources",
        ["skill_sha256"] = new string('a', 64),
        ["source_submission_id"] = _lastSourceSubmissionId,
        ["source_evidence_ids"] = Strings("33333333-3333-4333-8333-333333333333"),
        ["owner_scope_sha256"] = "sha256:" + new string('d', 64),
        ["device_key_id"] = "device-key-test",
        ["installed_at"] = "2026-09-11T05:00:00Z",
        ["marker_sha256"] = new string('c', 64),
        ["device_signature_b64"] = "synthetic-signature",
    }.ToJsonString(new JsonSerializerOptions { WriteIndented = true });

    private JsonObject InstalledSkillNode() => new()
    {
        ["install_id"] = "77777777-7777-4777-8777-777777777777",
        ["evaluation_id"] = "55555555-5555-4555-8555-555555555555",
        ["source_submission_id"] = _installedSourceSubmissionId,
        ["tool"] = "Codex",
        ["name"] = _installedName,
        ["target_location"] = "$CODEX_HOME/skills/" + _installedName,
        ["skill_sha256"] = new string('a', 64),
        ["marker_sha256"] = new string('c', 64),
        ["installed_at"] = "2026-09-11T05:00:00Z",
    };
}

/// <summary>
/// The skill surface on Windows: the core's live words, its positional
/// templates filled as Swift fills them, and the History inspector drawing
/// the flow behind the Mac's gates (SessionDetailView.swift:108-149).
/// </summary>
public sealed class SkillLearningSurfaceTests
{
    [Fact]
    public void TheLiveCopyDecodesAndItsTemplatesFillAsOnTheMac()
    {
        SkillLearningCopy copy = SkillLearningCopy.Current ?? throw new InvalidOperationException("tc_skill_learning_copy did not decode");
        string summary = SkillLearningCopy.Format(copy.ContractSummaryFormat, 12, 4, 36, 2048, 90);
        Assert.DoesNotContain("%", summary);
        Assert.Contains("12", summary);
        Assert.Contains("2048", summary);
        string budget = SkillLearningCopy.Format(copy.ReviewBudgetFormat, 36, 2048, "owner-x");
        Assert.Contains("owner-x", budget);
        Assert.DoesNotContain("%", budget);
        Assert.DoesNotContain("%", SkillLearningCopy.Format(copy.ModelBudgetFormat, "owner-x", 12, 2048));
    }

    [Fact]
    public void PositionalHolesTakeTheirOwnArgument()
    {
        Assert.Equal("b a 3 %4$d", SkillLearningCopy.Format("%2$@ %1$@ %3$d %4$d", "a", "b", 3));
    }

    [Fact]
    public void TheInspectorDrawsTheFlowBehindTheMacsGates()
    {
        string inspector = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App", "Controls",
            "HistoryDetailInspector.cs.txt"));
        int overview = inspector.IndexOf("Overview(copy, detail);", StringComparison.Ordinal);
        int learning = inspector.IndexOf("SkillLearningGate.OffersLearning(detail, Record.Status, completed)", StringComparison.Ordinal);
        int editor = inspector.IndexOf("if (ContributionGate.IsActive(detail, Record.Status, completed))", StringComparison.Ordinal);
        int installed = inspector.IndexOf("InstalledSkillSurface(copy, withdrawn);", StringComparison.Ordinal);
        Assert.True(overview > 0 && learning > overview && editor > learning && installed > editor);
        Assert.Contains("SkillLearningGate.ShowsInstalledSurface(CurrentDetail, withdrawalCompleted)", inspector);
        Assert.Contains("SkillLearningGate.OffersInstallStatusRetry(state, CurrentDetail, Record.Status)", inspector);

        string panel = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App", "Controls",
            "SkillLearningPanel.cs.txt"));
        // A rollback asks first, and only its explicit choice rolls back.
        int ask = panel.IndexOf("DialogGuard.ShowOnceAsync(dialog) == ContentDialogResult.Primary", StringComparison.Ordinal);
        int rollback = panel.IndexOf("await _model.RollbackSkillAsync(_id);", StringComparison.Ordinal);
        Assert.True(ask > 0 && rollback > ask);
        Assert.Equal(2, panel.Split("RollbackSkillAsync").Length);
        Assert.Contains("DefaultButton = ContentDialogButton.Close,", panel);
        // Review waits for the core's verdict on the draft.
        Assert.Contains("_reviewButton.IsEnabled = !working && validation?.Valid == true;", panel);
    }
}
