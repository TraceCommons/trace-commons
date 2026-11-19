using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using TraceCommons.Interop.FirstRun;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The first run's steps, Continue rules and daemon plan, pinned to the
/// Mac's own tests (macos/Tests/TCShellCoreTests/FirstRunNavigationTests.swift,
/// FirstRunPlanTests.swift, macos/Tests/TraceCommonsAppTests/FirstRunRunnerTests.swift
/// at origin/main 409066936): each expectation below is the Swift test's own
/// literal, cited by test name.
/// </summary>
public class FirstRunFlowTests
{
    private static FirstRunState Answered()
    {
        var s = new FirstRunState { Step = FirstRunStep.Join, Invite = "INVITE-1", Account = AccountAnswer.NearAI };
        s.ToolAnswers["claude-code"] = ToolChoice.Watch("/Users/someone/.claude/projects");
        s.ToolAnswers["codex"] = ToolChoice.Off;
        return s;
    }

    private static FirstRunState SignedIn()
    {
        FirstRunState s = Answered();
        s.DaemonStarted = true;
        s.EnrolledInvite = "INVITE-1";
        s.SignedIn = true;
        return s;
    }

    [Fact]
    public void TheStepsRunFromWelcomeToJoin()
    {
        // FirstRunNavigation.swift:15; test_theStepsRunFromWelcomeToJoin.
        Assert.Equal(
            new[] { FirstRunStep.Welcome, FirstRunStep.Tools, FirstRunStep.PrivateAI, FirstRunStep.Traces, FirstRunStep.Uses, FirstRunStep.Terms, FirstRunStep.Join },
            FirstRunNavigation.Steps);
        var s = new FirstRunState();
        var walked = new List<FirstRunStep> { s.Step };
        for (int i = 0; i < 8; i++)
        {
            s = FirstRunNavigation.Next(s);
            walked.Add(s.Step);
        }

        Assert.Equal(FirstRunNavigation.Steps.Concat(new[] { FirstRunStep.Join, FirstRunStep.Join }), walked);
        Assert.Equal(FirstRunNavigation.Steps.Skip(1), FirstRunNavigation.ThreadSteps);
        Assert.Null(FirstRunStageLayout.ThreadIndex(FirstRunStep.Welcome));
        Assert.Equal(0, FirstRunStageLayout.ThreadIndex(FirstRunStep.Tools));
    }

    [Fact]
    public void TheThreadGoesBackButNeverForward()
    {
        var s = new FirstRunState { Step = FirstRunStep.Uses, GrantReady = true };
        Assert.Equal(FirstRunStep.Uses, FirstRunNavigation.BackTo(FirstRunStep.Join, s).Step);
        FirstRunState back = FirstRunNavigation.BackTo(FirstRunStep.Tools, s);
        Assert.Equal(FirstRunStep.Tools, back.Step);
        Assert.False(back.GrantReady);
        Assert.False(FirstRunNavigation.Back(new FirstRunState { Step = FirstRunStep.Join, GrantReady = true }).GrantReady);
    }

    [Fact]
    public void NothingIsPreselected()
    {
        // FirstRunState.swift:157-178 defaults; owner 2026-10-27: no sharing
        // path chosen; Ron 2026-10-22: every tool check starts unticked.
        var s = new FirstRunState();
        Assert.False(s.SharingChosen);
        Assert.Empty(s.Scopes);
        Assert.Equal(AccountAnswer.None, s.Account);
        Assert.False(s.PrivateAI);
        s.RecordDiscovery(new[] { ("claude-code", true), ("codex", false), ("cline", true) });
        Assert.Equal(ToolChoice.Off, s.RowAnswer("claude-code"));
        Assert.Equal(ToolChoice.Off, s.RowAnswer("cline"));
        Assert.Equal(ToolChoice.Undecided, s.RowAnswer("codex"));
        Assert.False(s.WatchesAnything);
        s.Step = FirstRunStep.Tools;
        Assert.False(FirstRunNavigation.CanContinue(s, "required"));
        // An added tool folder starts unanswered; exported traces read Watch.
        Assert.Null(new AddedFolder("codex", "C:\\x").Watched);
        Assert.True(new AddedFolder(null, "C:\\exports").Watched);
    }

    [Fact]
    public void CanContinuePerStepIsTheMacs()
    {
        var s = new FirstRunState();
        foreach (FirstRunStep step in new[] { FirstRunStep.Welcome, FirstRunStep.Traces, FirstRunStep.PrivateAI, FirstRunStep.Terms })
        {
            s.Step = step;
            Assert.True(FirstRunNavigation.CanContinue(s, null));
        }

        s.Step = FirstRunStep.Uses;
        Assert.False(FirstRunNavigation.CanContinue(s, null));
        Assert.True(FirstRunNavigation.CanContinue(s, "required"));

        // Tools: something watched, nothing watched twice, a declaration.
        s.Step = FirstRunStep.Tools;
        s.RecordDiscovery(new[] { ("claude-code", true), ("codex", true) });
        s.Answer("claude-code", ToolChoice.Watch("C:\\Users\\a\\.claude\\projects"));
        Assert.True(FirstRunNavigation.CanContinue(s, null));
        s.Add(new AddedFolder("claude-code", "D:\\claude"));
        s.AnswerAdded("D:\\claude", true);
        Assert.Equal(new[] { "claude-code" }, s.WatchedTwice);
        Assert.False(FirstRunNavigation.CanContinue(s, null));

        // Join: the required use known, and watch only (no enrollment held)
        // or a held enrollment (test_joinWaitsForSomethingStartCanFinish).
        var join = Answered();
        Assert.False(FirstRunNavigation.CanContinue(join, "required"), "near.ai chosen is not an enrollment");
        join.Account = AccountAnswer.WatchOnly;
        Assert.True(FirstRunNavigation.CanContinue(join, "required"));
        Assert.False(FirstRunNavigation.CanContinue(join, null));
        join.EnrolledInvite = "INVITE-1";
        Assert.False(FirstRunNavigation.CanContinue(join, "required"), "watch only under a held enrollment");
        Assert.True(FirstRunNavigation.CanContinue(SignedIn(), "required"));
    }

    [Fact]
    public void LeavingToolsDeclaresTheRootsOnly()
    {
        foreach (CommitPoint point in new[] { CommitPoint.LeaveTools, CommitPoint.DeclareRoots })
        {
            var calls = FirstRunPlan.Calls(Answered(), point);
            var start = Assert.IsType<FirstRunCall.StartDaemon>(Assert.Single(calls));
            var decoded = JsonSerializer.Deserialize<Dictionary<string, Dictionary<string, string>>>(start.SettingsJson)!;
            Assert.Equal(new Dictionary<string, string> { ["mode"] = "off" }, decoded["codex_source"]);
        }

        FirstRunState started = Answered();
        started.DaemonStarted = true;
        started.StartedSettingsJson = started.SessionRoots.SettingsJson();
        Assert.Empty(FirstRunPlan.Calls(started, CommitPoint.LeaveTools));
        started.Answer("codex", ToolChoice.Watch("/Users/someone/.codex/sessions"));
        var changed = Assert.IsType<FirstRunCall.SetSourceSettings>(Assert.Single(FirstRunPlan.Calls(started, CommitPoint.DeclareRoots)));
        Assert.Equal(new[] { "codex_source" }, JsonSerializer.Deserialize<Dictionary<string, object>>(changed.SettingsJson)!.Keys);
    }

    [Fact]
    public void TheDaemonStartsBeforeAnyJoinCall()
    {
        var calls = FirstRunPlan.Calls(Answered(), CommitPoint.SignIn);
        Assert.Equal(4, calls.Count);
        Assert.IsType<FirstRunCall.StartDaemon>(calls[0]);
        Assert.Equal(new FirstRunCall[] { new FirstRunCall.LookupInvite("INVITE-1"), new FirstRunCall.Enroll("INVITE-1"), new FirstRunCall.SignInNearAI() }, calls.Skip(1));
        FirstRunState unfinished = Answered();
        unfinished.ToolAnswers.Remove("codex");
        Assert.Empty(FirstRunPlan.Calls(unfinished, CommitPoint.SignIn));
    }

    [Fact]
    public void AFolderWithdrawnAfterStartIsTurnedOff()
    {
        FirstRunState s = Answered();
        s.Add(new AddedFolder(null, "/Users/someone/exports"));
        s.DaemonStarted = true;
        s.StartedSettingsJson = s.SessionRoots.SettingsJson();
        s.WithdrawTrajectory();
        var call = Assert.IsType<FirstRunCall.SetSourceSettings>(Assert.Single(FirstRunPlan.Calls(s, CommitPoint.DeclareRoots)));
        Assert.Equal("{\"trajectory_source\":{\"mode\":\"off\"}}", call.SettingsJson);
    }

    [Fact]
    public void ScopesAreSavedBeforeTheGrant()
    {
        // test_scopesAreSavedBeforeTheGrant.
        FirstRunState s = SignedIn();
        s.Scopes = new SortedSet<string>(new[] { "research", "evaluation" });
        s.Rules["p2"] = ProjectRule.Ask;
        s.Rules["p1"] = ProjectRule.AutoUpload;
        s.PastSelections["p1"] = new SortedSet<string>(new[] { "s2", "s1" });
        s.PrivateAI = true;
        s.Sharing = SharingPath.Automatic;
        s.WitnessSigningAddress = "witness-1";
        s.GrantReady = true;
        Assert.Equal(new FirstRunCall[]
        {
            new FirstRunCall.SetConsentScopes(new[] { "evaluation", "research" }),
            new FirstRunCall.SetProjectMode("p1", ProjectRule.AutoUpload),
            new FirstRunCall.IncludePastSessions("p1", new[] { "s1", "s2" }),
            new FirstRunCall.SetPrivateAI(true),
            new FirstRunCall.GrantAutomatic("witness-1"),
            new FirstRunCall.SetProjectMode("p2", ProjectRule.Ask),
            new FirstRunCall.MarkComplete(),
        }, FirstRunPlan.Calls(s, CommitPoint.Start));
    }

    [Fact]
    public void AutomaticWithoutTheCoresReadyAnswerNeverGrants()
    {
        FirstRunState s = SignedIn();
        s.Scopes.Add("research");
        s.Sharing = SharingPath.Automatic;
        s.WitnessSigningAddress = "witness-1";
        Assert.Equal(new FirstRunCall[] { new FirstRunCall.SetConsentScopes(new[] { "research" }), new FirstRunCall.MarkComplete() }, FirstRunPlan.Calls(s, CommitPoint.Start));
        s.GrantReady = true;
        Assert.Contains(new FirstRunCall.GrantAutomatic("witness-1"), FirstRunPlan.Calls(s, CommitPoint.Start));
        s.Sharing = SharingPath.Automatic;
        Assert.False(s.GrantReady);
    }

    [Fact]
    public void AskMeNeverGrantsAndWatchOnlyHoldsNoEnrolment()
    {
        FirstRunState s = SignedIn();
        s.Scopes.Add("research");
        Assert.Equal(new FirstRunCall[] { new FirstRunCall.SetAskEachTime(), new FirstRunCall.SetConsentScopes(new[] { "research" }), new FirstRunCall.MarkComplete() }, FirstRunPlan.Calls(s, CommitPoint.Start));

        FirstRunState plain = Answered();
        plain.Account = AccountAnswer.WatchOnly;
        plain.Scopes.Add("research");
        plain.Sharing = SharingPath.Automatic;
        plain.GrantReady = true;
        Assert.Equal(new FirstRunCall[] { new FirstRunCall.MarkWatchOnlyComplete() }, FirstRunPlan.Calls(plain, CommitPoint.Start));
        plain.EnrolledInvite = "INVITE-1";
        Assert.Empty(FirstRunPlan.Calls(plain, CommitPoint.Start));
        Assert.Empty(FirstRunPlan.Calls(Answered(), CommitPoint.Start));
    }

    private sealed class Recorder : IFirstRunDaemon
    {
        public readonly List<string> Calls = new();
        public bool Starts = true;

        public Task<bool> StartDaemon(string settingsJson) { Calls.Add("start"); return Task.FromResult(Starts); }
        public Task<bool> SetSourceSettings(string settingsJson) { Calls.Add("settings"); return Task.FromResult(true); }
        public Task<FirstRunLookup> LookupInvite(string invite) { Calls.Add("lookup"); return Task.FromResult<FirstRunLookup>(new FirstRunLookup.Found("{}")); }
        public Task<bool> EnrollInvite(string invite) { Calls.Add("enroll"); return Task.FromResult(true); }
        public Task<bool> SignInNearAI() { Calls.Add("signin"); return Task.FromResult(true); }
        public Task<NearAILoginOutcome> NearAILoginOutcome(CancellationToken cancel) { Calls.Add("login"); return Task.FromResult(FirstRun.NearAILoginOutcome.SignedIn); }
        public Task<bool> CancelNearAILogin() { Calls.Add("cancel"); return Task.FromResult(true); }
        public Task<NearAIEnrolment> EnrollNearAI(string? referralCode) { Calls.Add("enroll-near"); return Task.FromResult(new NearAIEnrolment(true, null)); }
        public Task<bool> SetAskEachTime() { Calls.Add("ask"); return Task.FromResult(true); }
        public Task<bool> SaveConsentScopes(IReadOnlyList<string> scopes) { Calls.Add("scopes"); return Task.FromResult(true); }
        public Task<bool> SetProjectMode(string projectId, ProjectRule mode) { Calls.Add("mode"); return Task.FromResult(true); }
        public Task<bool> IncludePastSessions(string projectId, IReadOnlyList<string> sessionIds) { Calls.Add("include"); return Task.FromResult(true); }
        public Task<bool> SetPrivateAI(bool on) { Calls.Add("private"); return Task.FromResult(true); }
        public Task<GrantAnswer> GrantAutomatic(string? witness) { Calls.Add("grant"); return Task.FromResult(new GrantAnswer(true, null)); }
        public Task<bool> MarkComplete() { Calls.Add("mark"); return Task.FromResult(true); }
        public Task<bool> MarkWatchOnlyComplete() { Calls.Add("mark-watch"); return Task.FromResult(true); }
        public Task<bool> RecordTermsAcceptance(AcceptedTerms terms) { Calls.Add("terms"); return Task.FromResult(true); }
        public void FirstRunFinished(FirstRunFailure? notice) => Calls.Add("finished");
    }

    [Fact]
    public async Task TheTermsAreSentOnceToTheStartedDaemon()
    {
        // FirstRunRunner.swift:263-272: handed over at the first commit after
        // acceptance with the daemon started, once per first run.
        var daemon = new Recorder();
        FirstRunState state = Answered();
        state.Step = FirstRunStep.Tools;
        var runner = new FirstRunRunner(state, daemon) { AcceptedTerms = new AcceptedTerms("v1", "abc", default) };
        await runner.Commit(CommitPoint.LeaveTools);
        Assert.Equal(new[] { "start", "terms" }, daemon.Calls);
        Assert.Equal(FirstRunStep.PrivateAI, runner.State.Step);
        runner.State.Step = FirstRunStep.Join;
        await runner.Commit(CommitPoint.SignIn);
        await runner.Commit(CommitPoint.Start);
        Assert.Equal(1, daemon.Calls.Count(c => c == "terms"));
        Assert.Equal(new[] { "start", "terms", "lookup", "enroll", "signin", "scopes", "mark", "finished" }, daemon.Calls.Where(c => c != "ask"));
        Assert.True(runner.Completed);
    }

    [Fact]
    public async Task TermsAcceptedOnlyForTheDocumentOnScreen()
    {
        var runner = new FirstRunRunner(new FirstRunState { Step = FirstRunStep.Terms }, new Recorder());
        Assert.False(runner.HasAcceptedTerms(true, "v1", "abc"));
        runner.AcceptedTerms = new AcceptedTerms("v1", "abc", default);
        Assert.False(runner.HasAcceptedTerms(false, "v1", "abc"));
        Assert.False(runner.HasAcceptedTerms(true, "v2", "abc"));
        Assert.False(runner.HasAcceptedTerms(true, null, null));
        runner.ContinueFromTerms(true, "v2", "abc");
        Assert.Equal(FirstRunStep.Terms, runner.State.Step);
        runner.ContinueFromTerms(true, "v1", "abc");
        Assert.Equal(FirstRunStep.Join, runner.State.Step);
        await Task.CompletedTask;
    }

    [Fact]
    public async Task AFailedStartStaysOnToolsAndSaysSo()
    {
        var daemon = new Recorder { Starts = false };
        FirstRunState state = Answered();
        state.Step = FirstRunStep.Tools;
        var runner = new FirstRunRunner(state, daemon) { AcceptedTerms = new AcceptedTerms("v1", "abc", default) };
        await runner.Commit(CommitPoint.LeaveTools);
        Assert.IsType<FirstRunFailure.StartFailed>(runner.Failure);
        Assert.Equal(FirstRunStep.Tools, runner.State.Step);
        Assert.Equal(new[] { "start" }, daemon.Calls);
        // The failure clears on the next commit that succeeds (L3).
        daemon.Starts = true;
        await runner.Commit(CommitPoint.LeaveTools);
        Assert.Null(runner.Failure);
        Assert.Equal(FirstRunStep.PrivateAI, runner.State.Step);
    }

    [Fact]
    public void AnEnrolmentTheDaemonHoldsIsRecordedNotJoinedAgain()
    {
        // OnboardingNavigation.swift:36-48.
        FirstRunState s = Answered();
        s.Account = AccountAnswer.None;
        FirstRunState recorded = FirstRunNavigation.RecordEnrolment(s);
        Assert.Equal(AccountAnswer.Enrolled, recorded.Account);
        Assert.Equal("", recorded.EnrolledInvite);
        Assert.Equal("", recorded.Invite);
        Assert.True(recorded.HoldsEnrolment);
        Assert.DoesNotContain(FirstRunPlan.Calls(recorded, CommitPoint.SignIn), c => c is FirstRunCall.Enroll or FirstRunCall.LookupInvite);
        FirstRunState signedOut = Answered();
        signedOut.SignedOutOfEnrolment = true;
        Assert.Same(signedOut, FirstRunNavigation.RecordEnrolment(signedOut));
        FirstRunState nearAI = Answered();
        Assert.Equal(AccountAnswer.NearAI, FirstRunNavigation.RecordEnrolment(nearAI).Account);
    }
}
