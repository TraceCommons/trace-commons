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
/// What one account's skill work leaves behind when another account, or no
/// account, takes over (<c>reconcileAccountOwnedContent</c> and
/// <c>clearAccountOwnedContent</c>, AppModel.swift:4195-4261, and
/// SessionDetailReadOrderTests.swift:79-102). The Mac keeps the owner on its
/// one app model; Windows builds an inspector per record, so the owner is
/// kept per window in <see cref="AccountOwnedContent"/> and every inspector's
/// read reports to it.
/// </summary>
public sealed class AccountOwnedContentTests
{
    private const string RecordId = "22222222-2222-4222-8222-222222222222";
    private const string SecondId = "88888888-8888-4888-8888-888888888888";
    private const string OwnerA = "sha256:owner-a";
    private const string OwnerB = "sha256:owner-b";

    private static readonly SkillLearningCopy Copy = SkillLearningCopy.Parse(
        new JsonObject(SkillLearningCopy.RequiredWords.Select(key =>
            KeyValuePair.Create(key, (JsonNode?)JsonValue.Create("word:" + key)))).ToJsonString())!;

    private static string? Line(string label) => "line:" + label;

    /// <summary>A window's two halves, wired as MainWindow wires them.</summary>
    private static (AccountOwnedContent Content, SkillLearningModel Skills) Window(
        Func<string, string, Task<DaemonResponse>> call)
    {
        var skills = new SkillLearningModel(call, Copy, Line);
        var content = new AccountOwnedContent();
        content.Cleared += skills.ClearAccountOwnedContent;
        return (content, skills);
    }

    /// <summary>Account A's work: one installed skill, and one candidate in review on another record.</summary>
    private static async Task WorkUnderTheFirstAccount(SkillLearningModel skills)
    {
        await skills.LearnSkillAsync(RecordId);
        SkillCandidate candidate = skills.State(RecordId).Phase.Candidate!;
        await skills.ReviewSkillAsync(RecordId, candidate.Draft);
        await skills.TestSkillAsync(RecordId);
        await skills.ReviewSkillInstallAsync(RecordId);
        await skills.InstallSkillAsync(RecordId);
        Assert.NotNull(skills.State(RecordId).SkillLearningInstall);
        await skills.LearnSkillAsync(SecondId);
        Assert.NotNull(skills.State(SecondId).Phase.Candidate);
    }

    private static void AssertNothingLeft(SkillLearningModel skills)
    {
        Assert.Equal(0, skills.RetainedSessionCount);
        foreach (string id in new[] { RecordId, SecondId })
        {
            SkillLearningSessionState state = skills.State(id);
            Assert.Equal(SkillLearningPhaseKind.Idle, state.Phase.Kind);
            Assert.Null(state.Phase.Candidate);
            Assert.Null(state.SkillLearningInstall);
            Assert.Null(state.Failure);
            Assert.Null(state.PreservedDraft);
            Assert.False(state.IsWorking);
        }
    }

    [Fact]
    public async Task ADetailUnderAnotherAccountClearsEverySkillTheFirstLeft()
    {
        var daemon = new SkillLearningFakeDaemon();
        (AccountOwnedContent content, SkillLearningModel skills) = Window(daemon.CallAsync);
        Assert.True(content.ReadSucceeded(content.ReadStarted(), OwnerA));
        await WorkUnderTheFirstAccount(skills);

        // Another record's inspector reads under account B.
        Assert.True(content.ReadSucceeded(content.ReadStarted(), OwnerB));

        AssertNothingLeft(skills);
        Assert.Equal(OwnerB, content.Scope);
    }

    [Fact]
    public async Task TheSameAccountOrNoOwnerClearsNothing()
    {
        var daemon = new SkillLearningFakeDaemon();
        (AccountOwnedContent content, SkillLearningModel skills) = Window(daemon.CallAsync);
        int cleared = 0;
        content.Cleared += () => cleared++;
        Assert.True(content.ReadSucceeded(content.ReadStarted(), OwnerA));
        await WorkUnderTheFirstAccount(skills);

        Assert.True(content.ReadSucceeded(content.ReadStarted(), OwnerA));
        // A core that predates the owner sends none: nothing is decided by it.
        Assert.True(content.ReadSucceeded(content.ReadStarted(), null));

        Assert.Equal(0, cleared);
        Assert.Equal(2, skills.RetainedSessionCount);
        Assert.NotNull(skills.State(RecordId).SkillLearningInstall);
        Assert.Equal(OwnerA, content.Scope);
    }

    // Signing out, or a record no longer this account's (AppModel.swift:4230-4239).
    [Theory]
    [InlineData("account-session-required")]
    [InlineData("session-detail-not-found")]
    [InlineData("session-owner-changed")]
    public async Task AnAccountEndingFailureClearsEverything(string label)
    {
        var daemon = new SkillLearningFakeDaemon();
        (AccountOwnedContent content, SkillLearningModel skills) = Window(daemon.CallAsync);
        Assert.True(content.ReadSucceeded(content.ReadStarted(), OwnerA));
        await WorkUnderTheFirstAccount(skills);

        content.ReadFailed(content.ReadStarted(), label);

        AssertNothingLeft(skills);
        Assert.Null(content.Scope);
    }

    [Theory]
    [InlineData("session-detail-unavailable")]
    [InlineData("daemon-unavailable")]
    [InlineData(null)]
    public async Task AnyOtherFailureClearsNothing(string? label)
    {
        var daemon = new SkillLearningFakeDaemon();
        (AccountOwnedContent content, SkillLearningModel skills) = Window(daemon.CallAsync);
        Assert.True(content.ReadSucceeded(content.ReadStarted(), OwnerA));
        await WorkUnderTheFirstAccount(skills);

        content.ReadFailed(content.ReadStarted(), label);

        Assert.Equal(2, skills.RetainedSessionCount);
        Assert.Equal(OwnerA, content.Scope);
    }

    // SessionDetailReadOrderTests.swift:79-102.
    [Fact]
    public async Task ALateAnswerFromAnEarlierAccountNeverClearsTheNewerOne()
    {
        var daemon = new SkillLearningFakeDaemon();
        (AccountOwnedContent content, SkillLearningModel skills) = Window(daemon.CallAsync);
        ulong first = content.ReadStarted();
        ulong second = content.ReadStarted();
        Assert.True(content.ReadSucceeded(second, OwnerB));
        await WorkUnderTheFirstAccount(skills);

        // Not shown: the caller says the read could not be made.
        Assert.False(content.ReadSucceeded(first, OwnerA));

        Assert.Equal(2, skills.RetainedSessionCount);
        Assert.Equal(OwnerB, content.Scope);
    }

    [Fact]
    public async Task ALateFailureFromAnEarlierReadNeverClearsTheNewerOne()
    {
        var daemon = new SkillLearningFakeDaemon();
        (AccountOwnedContent content, SkillLearningModel skills) = Window(daemon.CallAsync);
        ulong first = content.ReadStarted();
        ulong second = content.ReadStarted();
        Assert.True(content.ReadSucceeded(second, OwnerB));
        await WorkUnderTheFirstAccount(skills);

        content.ReadFailed(first, "account-session-required");

        Assert.Equal(2, skills.RetainedSessionCount);
        Assert.Equal(OwnerB, content.Scope);
    }

    [Fact]
    public async Task AnEarlierAnswerUnderTheSameAccountIsStillShown()
    {
        var daemon = new SkillLearningFakeDaemon();
        (AccountOwnedContent content, SkillLearningModel skills) = Window(daemon.CallAsync);
        ulong first = content.ReadStarted();
        ulong second = content.ReadStarted();
        Assert.True(content.ReadSucceeded(second, OwnerA));
        await WorkUnderTheFirstAccount(skills);

        Assert.True(content.ReadSucceeded(first, OwnerA));
        Assert.Equal(2, skills.RetainedSessionCount);
    }

    /// <summary>
    /// Windows' account boundary (<see cref="AccountGeneration.Changed"/>: a
    /// status naming another account, a sign-out, an unenroll) clears too,
    /// so a record reopened before its read answers draws nothing of the
    /// last account's. The next owner is then a first owner.
    /// </summary>
    [Fact]
    public async Task TheAccountBoundaryClearsEverythingBeforeAnyReadAnswers()
    {
        var daemon = new SkillLearningFakeDaemon();
        (AccountOwnedContent content, SkillLearningModel skills) = Window(daemon.CallAsync);
        var account = new AccountGeneration();
        account.Changed += content.AccountBoundary;
        Assert.True(content.ReadSucceeded(content.ReadStarted(), OwnerA));
        await WorkUnderTheFirstAccount(skills);

        account.Boundary();

        AssertNothingLeft(skills);
        Assert.Null(content.Scope);
        int cleared = 0;
        content.Cleared += () => cleared++;
        Assert.True(content.ReadSucceeded(content.ReadStarted(), OwnerB));
        Assert.Equal(0, cleared);
    }

    [Fact]
    public async Task AReadStartedBeforeTheBoundaryNeverBringsTheOldAccountBack()
    {
        var daemon = new SkillLearningFakeDaemon();
        (AccountOwnedContent content, SkillLearningModel skills) = Window(daemon.CallAsync);
        Assert.True(content.ReadSucceeded(content.ReadStarted(), OwnerA));
        ulong before = content.ReadStarted();
        content.AccountBoundary();
        await WorkUnderTheFirstAccount(skills);

        Assert.False(content.ReadSucceeded(before, OwnerA));
        content.ReadFailed(before, "account-session-required");

        Assert.Null(content.Scope);
        Assert.Equal(2, skills.RetainedSessionCount);
    }

    [Fact]
    public async Task AStatusNamingAnotherAccountClearsEverything()
    {
        var daemon = new SkillLearningFakeDaemon();
        (AccountOwnedContent content, SkillLearningModel skills) = Window(daemon.CallAsync);
        var account = new AccountGeneration();
        account.Changed += content.AccountBoundary;
        account.Observe(OwnerA);
        await WorkUnderTheFirstAccount(skills);

        account.Observe(OwnerB);

        AssertNothingLeft(skills);
    }

    /// <summary>A daemon that holds one method's reply until released.</summary>
    private sealed class Held
    {
        private readonly SkillLearningFakeDaemon _daemon;
        private readonly string _method;
        private readonly TaskCompletionSource _release = new(TaskCreationOptions.RunContinuationsAsynchronously);
        public readonly TaskCompletionSource Reached = new(TaskCreationOptions.RunContinuationsAsynchronously);

        public Held(SkillLearningFakeDaemon daemon, string method)
        {
            _daemon = daemon;
            _method = method;
        }

        public void Release() => _release.TrySetResult();

        public async Task<DaemonResponse> CallAsync(string method, string parameters)
        {
            if (method == _method)
            {
                Reached.TrySetResult();
                await _release.Task;
            }

            return await _daemon.CallAsync(method, parameters);
        }
    }

    [Theory]
    [InlineData("skill_candidate")]
    [InlineData("skill_install_status")]
    public async Task AnAnswerInFlightWhenTheAccountChangesSettlesNothing(string method)
    {
        var daemon = new SkillLearningFakeDaemon();
        var held = new Held(daemon, method);
        (AccountOwnedContent content, SkillLearningModel skills) = Window(held.CallAsync);
        Assert.True(content.ReadSucceeded(content.ReadStarted(), OwnerA));

        Task learning = skills.LearnSkillAsync(RecordId);
        await held.Reached.Task;
        Assert.True(skills.State(RecordId).IsWorking);
        Assert.True(content.ReadSucceeded(content.ReadStarted(), OwnerB));
        held.Release();
        await learning;

        Assert.Equal(0, skills.RetainedSessionCount);
        Assert.Null(skills.State(RecordId).Phase.Candidate);
        Assert.Null(skills.State(RecordId).Failure);
    }

    [Fact]
    public async Task AnInstallInFlightWhenTheAccountChangesSettlesNothing()
    {
        var daemon = new SkillLearningFakeDaemon();
        var held = new Held(daemon, "skill_install_commit");
        (AccountOwnedContent content, SkillLearningModel skills) = Window(held.CallAsync);
        Assert.True(content.ReadSucceeded(content.ReadStarted(), OwnerA));
        await skills.LearnSkillAsync(RecordId);
        await skills.ReviewSkillAsync(RecordId, skills.State(RecordId).Phase.Candidate!.Draft);
        await skills.TestSkillAsync(RecordId);
        await skills.ReviewSkillInstallAsync(RecordId);

        Task installing = skills.InstallSkillAsync(RecordId);
        await held.Reached.Task;
        content.ReadFailed(content.ReadStarted(), "account-session-required");
        held.Release();
        await installing;

        Assert.Equal(0, skills.RetainedSessionCount);
        Assert.Null(skills.State(RecordId).SkillLearningInstall);
    }

    [Fact]
    public async Task EverySessionTheClearRemovesIsAnnounced()
    {
        var daemon = new SkillLearningFakeDaemon();
        (AccountOwnedContent content, SkillLearningModel skills) = Window(daemon.CallAsync);
        Assert.True(content.ReadSucceeded(content.ReadStarted(), OwnerA));
        await WorkUnderTheFirstAccount(skills);
        var changed = new List<string>();
        skills.SessionChanged += changed.Add;

        content.AccountBoundary();

        Assert.Equal(new[] { RecordId, SecondId }.OrderBy(id => id, StringComparer.Ordinal), changed.OrderBy(id => id, StringComparer.Ordinal));
    }

    // ---- The record's own detail (HistoryDetailState) --------------------

    private static SessionDetail Detail(string owner)
    {
        using JsonDocument doc = JsonDocument.Parse(
            "{\"owner_scope_sha256\":\"" + owner + "\",\"task\":\"t\",\"task_success\":\"partial\",\"user_feedback\":\"correction\","
            + "\"human_correction\":\"c\",\"evidence\":[],\"contribution_status\":\"accepted\",\"permitted_uses\":[],"
            + "\"contributed_version\":\"trace-contribution/1\",\"consent_policy_version\":\"consent/1\","
            + "\"redaction_pipeline_version\":\"redaction/3\",\"publication\":null,\"publication_version\":0}");
        return SessionDetail.Parse(doc.RootElement)!;
    }

    [Fact]
    public void AnAccountEndingFailureAlsoDropsThePublicationError()
    {
        var state = new HistoryDetailState();
        state.ReadSucceeded(Detail(OwnerA));
        state.PublicationFailed("public-run-publication-refused");
        Assert.NotNull(state.PublicationError);

        state.ReadFailed("account-session-required");

        Assert.Null(state.Detail);
        Assert.Null(state.PublicationError);
    }

    [Fact]
    public void TheWindowsClearDropsTheDetailAndItsErrors()
    {
        var state = new HistoryDetailState();
        state.ReadSucceeded(Detail(OwnerA));
        state.PublicationFailed("public-run-publication-refused");

        state.AccountCleared();

        Assert.Null(state.Detail);
        Assert.Null(state.ReadError);
        Assert.Null(state.PublicationError);
    }

    [Fact]
    public void AStaleReadIsNotShownAndSaysSo()
    {
        var state = new HistoryDetailState();
        state.ReadSucceeded(Detail(OwnerB));

        state.ReadStale();

        Assert.Equal(OwnerB, state.Detail?.OwnerScopeSha256);
        Assert.Equal(PublicRunErrors.SessionDetailLine("session-owner-changed"), state.ReadError);
    }

    // ---- Wiring in the app (source-pinned, as the shell is not built here) --

    private static string AppSource(string relative) => File.ReadAllText(
        Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App", relative + ".txt")).Replace("\r\n", "\n", StringComparison.Ordinal);

    [Fact]
    public void TheWindowClearsItsSkillsAndInspectorOnEitherSignal()
    {
        string window = AppSource("MainWindow.SentTracesTraining.cs");
        Assert.Contains("_accountContent.Cleared += ClearAccountOwnedContent;", window);
        Assert.Contains("_host.Account.Changed += _accountContent.AccountBoundary;", window);
        Assert.Contains("_skillLearning?.ClearAccountOwnedContent();", window);
        Assert.Contains("_historyDetail?.ClearAccountOwnedContent();", window);
        string opened = AppSource("MainWindow.HistoryDetail.cs");
        Assert.Contains("new HistoryDetailInspector(_host, record, SentTraining, SkillLearning, AccountContent)", opened);
    }

    [Fact]
    public void EveryInspectorReadReportsToTheWindowsOwner()
    {
        string inspector = AppSource(Path.Combine("Controls", "HistoryDetailInspector.cs"));
        Assert.Contains("ulong sequence = _account?.ReadStarted() ?? 0;", inspector);
        Assert.Contains("_account?.ReadFailed(sequence, ", inspector);
        Assert.Contains("_account.ReadSucceeded(sequence, detail.OwnerScopeSha256)", inspector);
        Assert.Contains("_state.ReadStale();", inspector);
        // The inspector's own clear drops the typed skill draft with the panel.
        int clear = inspector.IndexOf("public void ClearAccountOwnedContent()", StringComparison.Ordinal);
        Assert.True(clear >= 0);
        string body = inspector.Substring(clear, inspector.IndexOf("\n    }", clear, StringComparison.Ordinal) - clear);
        Assert.Contains("_state.AccountCleared();", body);
        Assert.Contains("ResetEditor();", body);
        Assert.Contains("_skillPanel = null;", body);
    }
}
