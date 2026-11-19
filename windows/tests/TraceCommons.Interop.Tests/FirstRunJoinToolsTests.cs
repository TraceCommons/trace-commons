using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using TraceCommons.Interop.FirstRun;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The review fixes for the first run (PR #2210): the add-tool tile, every
/// failure's line, Join's invite card, the Automatic disclosures and the
/// sign-in's Back. Each expectation is the Mac's, cited by file and line at
/// origin/main; the words are the core's own, read through the C ABI.
/// </summary>
public sealed class FirstRunJoinToolsTests
{
    private static readonly FirstRunCopy Copy = FirstRunCopy.Load() ?? throw new InvalidOperationException("the first-run copy did not decode");

    private static string Name(string kind) => kind switch
    {
        "claude-code" => "Claude Code",
        "opencode" => "OpenCode",
        _ => kind,
    };

    private static string Row(string source, string path) =>
        "{\"source\":\"" + source + "\",\"path\":" + JsonSerializer.Serialize(path) + ",\"exists\":true,\"session_count\":2,\"most_recent\":null,\"relocated_by_env\":false}";

    // ---- Tools: add your tool (ToolsScreen.swift:20-160) ----------------------

    [Fact]
    public void AComputerWithNoToolContinuesOnceAnAddedFolderIsWatched()
    {
        // Review finding 1: with none of the five tools found, Tools could
        // not be left. The Mac's tile adds a folder as a row of its own,
        // unanswered (ToolsScreen.swift:57-67), and ticking it watches it.
        var state = new FirstRunState { Step = FirstRunStep.Tools };
        state.RecordDiscovery(FirstRunRoots.SettingsKeys.Keys.Select(k => (k, false)));
        Assert.False(FirstRunNavigation.CanContinue(state, null));

        AddToolOutcome outcome = ToolsAdding.Outcome(@"C:\exports\claude", "[" + Row("claude-code", @"C:\exports\claude") + "]");
        Assert.Equal(new AddToolOutcome.Added(new AddedFolder("claude-code", @"C:\exports\claude")), outcome);
        Assert.True(ToolsAdding.Apply(outcome, state));
        Assert.Null(Assert.Single(ToolsAdding.AddedRows(state)).Watched);
        Assert.False(FirstRunNavigation.CanContinue(state, null));

        state.AnswerAdded(@"C:\exports\claude", true);
        Assert.True(FirstRunNavigation.CanContinue(state, null));
        Assert.Equal(ToolChoice.Watch(@"C:\exports\claude"), state.SessionRoots["claude-code"]);
    }

    [Fact]
    public void TheCoreDescribesAPickedFolder()
    {
        // tc_describe_folder through the binding: a Claude Code store is
        // one match and is added as Claude Code.
        string root = Path.Combine(Path.GetTempPath(), "tc-w7-describe-" + Guid.NewGuid().ToString("N"));
        string project = Path.Combine(root, "-Users-someone-code-app");
        Directory.CreateDirectory(project);
        try
        {
            File.WriteAllText(Path.Combine(project, "6f9619ff-8b86-4011-b42d-00cf4fc964ff.jsonl"), "{}\n");
            File.WriteAllText(Path.Combine(project, "0e1d2c3b-4a59-4687-9b0a-1b2c3d4e5f60.jsonl"), "{}\n");
            AddToolOutcome outcome = ToolsAdding.Outcome(root, ToolsAdding.Describe(root));
            Assert.Equal(new AddToolOutcome.Added(new AddedFolder("claude-code", root)), outcome);
            Assert.Equal(new AddToolOutcome.Refused(), ToolsAdding.Outcome(project + "-missing", ToolsAdding.Describe(project + "-missing")));
        }
        finally
        {
            Directory.Delete(root, recursive: true);
        }
    }

    [Fact]
    public void TwoMatchesAskAndNothingOfferableIsRefused()
    {
        // ToolsScreen.swift:36-52: more than one match is a question, even
        // when one of them cannot be offered; none offerable, a null or an
        // undecodable answer, or an empty list is a refusal.
        string both = "[" + Row("opencode", @"C:\x") + "," + Row("trajectory", @"C:\x") + "]";
        Assert.Equal(new AddToolOutcome.Ask(@"C:\x", new string?[] { "opencode", null }), ToolsAdding.Outcome(@"C:\x", both));
        string oneUnknown = "[" + Row("opencode", @"C:\x") + "," + Row("future-tool", @"C:\x") + "]";
        Assert.Equal(new AddToolOutcome.Ask(@"C:\x", new string?[] { "opencode" }), ToolsAdding.Outcome(@"C:\x", oneUnknown));
        Assert.Equal(new AddToolOutcome.Refused(), ToolsAdding.Outcome(@"C:\x", "[" + Row("future-tool", @"C:\x") + "]"));
        Assert.Equal(new AddToolOutcome.Refused(), ToolsAdding.Outcome(@"C:\x", null));
        Assert.Equal(new AddToolOutcome.Refused(), ToolsAdding.Outcome(@"C:\x", "not json"));
        Assert.Equal(new AddToolOutcome.Refused(), ToolsAdding.Outcome(@"C:\x", "[]"));

        var state = new FirstRunState();
        Assert.True(ToolsAdding.Apply(new AddToolOutcome.Ask(@"C:\x", new string?[] { "opencode", null }), state));
        Assert.Empty(state.AddedFolders);
        Assert.False(ToolsAdding.Apply(new AddToolOutcome.Refused(), state));
    }

    [Fact]
    public void TheQuestionIsAnsweredByKindOrNeither()
    {
        // ToolsScreen.swift:136-150: a kind adds its row; Neither closes the
        // question with nothing added; an index never offered keeps it open.
        var kinds = new string?[] { "opencode", null };
        var state = new FirstRunState();
        Assert.False(ToolsAdding.Answer(2, @"C:\x", kinds, state));
        Assert.Empty(state.AddedFolders);
        Assert.True(ToolsAdding.Answer(null, @"C:\x", kinds, state));
        Assert.Empty(state.AddedFolders);
        Assert.True(ToolsAdding.Answer(1, @"C:\x", kinds, state));
        AddedFolder trajectory = Assert.Single(state.AddedFolders);
        Assert.True(trajectory.IsTrajectory);
        Assert.Equal(true, trajectory.Watched);
        Assert.Same(trajectory, ToolsAdding.TrajectoryFolder(state));
        Assert.Empty(ToolsAdding.AddedRows(state));
        ToolsAdding.SelectTrajectory(false, state);
        Assert.Empty(state.AddedFolders);
    }

    [Fact]
    public void AToolWatchedInTwoRowsIsNamedInTheCoresLine()
    {
        // ToolsScreen.swift:94-98: the line beside an added folder reading
        // Watch while its tool is watched in another row too.
        var state = new FirstRunState { Step = FirstRunStep.Tools };
        state.Answer("claude-code", ToolChoice.Watch(@"C:\a"));
        state.Add(new AddedFolder("claude-code", @"C:\b"));
        AddedFolder added = state.AddedFolders[0];
        Assert.Null(ToolsAdding.Conflict(added, state, Copy.Tools, Name));
        state.AnswerAdded(@"C:\b", true);
        string? line = ToolsAdding.Conflict(state.AddedFolders[0], state, Copy.Tools, Name);
        Assert.Equal(FirstRunCopy.Fill(Copy.Tools.OneFolderPerTool, new Dictionary<string, string> { ["tool"] = "Claude Code" }), line);
        Assert.Contains("Claude Code", line);
        Assert.False(FirstRunNavigation.CanContinue(state, null));
        Assert.Equal("exports", ToolsAdding.FolderName(@"C:\data\exports\"));
        Assert.False(ToolsAdding.AcceptsFolder(FirstRunStep.Tools, isCommitting: true));
        Assert.False(ToolsAdding.AcceptsFolder(FirstRunStep.Uses, isCommitting: false));
        Assert.True(ToolsAdding.AcceptsFolder(FirstRunStep.Tools, isCommitting: false));
    }

    // ---- Failure lines (FoldersScreen.swift:47-73, UsesScreen.swift:224-249, JoinScreen.swift:177-201) ----

    private static string? Enroll(string label) => label == "near_ai_terms_required" ? "core enroll line" : null;

    [Fact]
    public void EveryFailureToolsShowsHasItsLine()
    {
        // Review finding 3: a daemon that would not start said nothing.
        Assert.Equal("watcher line", FirstRunNotices.Tools(new FirstRunFailure.StartFailed(), Copy, "watcher line", Enroll));
        Assert.Null(FirstRunNotices.Tools(new FirstRunFailure.StartFailed(), Copy, null, Enroll));
        Assert.Equal(Copy.Folders.SettingsFailed, FirstRunNotices.Tools(new FirstRunFailure.SettingsFailed(), Copy, null, Enroll));
        Assert.Equal(Copy.Folders.EnrollRefused, FirstRunNotices.Tools(new FirstRunFailure.EnrollFailed(), Copy, null, Enroll));
        Assert.Equal(Copy.Folders.LookupUnavailable, FirstRunNotices.Tools(new FirstRunFailure.LookupUnavailable(), Copy, null, Enroll));
        Assert.Equal(Copy.Folders.SignInFailed, FirstRunNotices.Tools(new FirstRunFailure.SignInFailed(), Copy, null, Enroll));
        Assert.Equal("core enroll line", FirstRunNotices.Tools(new FirstRunFailure.NearAIEnrollFailed("near_ai_terms_required"), Copy, null, Enroll));
        Assert.Equal(Copy.Folders.EnrollRefused, FirstRunNotices.Tools(new FirstRunFailure.NearAIEnrollFailed("unknown"), Copy, null, Enroll));
        Assert.Null(FirstRunNotices.Tools(new FirstRunFailure.ScopesFailed(), Copy, null, Enroll));
        Assert.Null(FirstRunNotices.Tools(null, Copy, null, Enroll));
        Assert.All(new[] { Copy.Folders.SettingsFailed, Copy.Folders.EnrollRefused, Copy.Folders.LookupUnavailable, Copy.Folders.SignInFailed }, l => Assert.NotEmpty(l));
    }

    [Fact]
    public void EveryStartFailureHasItsLineAndARefusedGrantSaysSo()
    {
        UsesWords uses = Copy.Uses;
        Assert.Equal(uses.SharingRefused, FirstRunNotices.Start(new FirstRunFailure.GrantRefused("x"), uses, null));
        Assert.Equal(uses.ScopesFailed, FirstRunNotices.Start(new FirstRunFailure.ScopesFailed(), uses, null));
        Assert.Equal(uses.SharingSaveFailed, FirstRunNotices.Start(new FirstRunFailure.SharingFailed(), uses, null));
        Assert.Equal(uses.RulesFailed, FirstRunNotices.Start(new FirstRunFailure.RulesFailed(), uses, null));
        Assert.Equal("unconfirmed", FirstRunNotices.Start(new FirstRunFailure.PrivateAIFailed(), uses, "unconfirmed"));
        Assert.Equal(uses.PrivateAiFailed, FirstRunNotices.Start(new FirstRunFailure.PrivateAIFailed(), uses, null));
        Assert.Equal(uses.CompleteFailed, FirstRunNotices.Start(new FirstRunFailure.CompleteFailed(), uses, null));
        Assert.Null(FirstRunNotices.Start(new FirstRunFailure.SignInFailed(), uses, null));
        Assert.Equal(uses.SharingRefused, FirstRunNotices.Finished(new FirstRunFailure.GrantRefused("x"), uses));
        Assert.Null(FirstRunNotices.Finished(new FirstRunFailure.CompleteFailed(), uses));
        Assert.Null(FirstRunNotices.Finished(null, uses));
        Assert.All(new[] { uses.SharingRefused, uses.ScopesFailed, uses.SharingSaveFailed, uses.RulesFailed, uses.CompleteFailed }, l => Assert.NotEmpty(l));
    }

    [Fact]
    public void JoinSaysEachFailureOnceAndWhereTheMacSaysIt()
    {
        var none = new FirstRunState { Step = FirstRunStep.Join, Account = AccountAnswer.None };
        var chosen = new FirstRunState { Step = FirstRunStep.Join, Account = AccountAnswer.NearAI };
        // A sign-in that cleared the choice: under the near.ai card, not under the cards.
        Assert.Equal(Copy.Folders.SignInFailed, FirstRunNotices.JoinNearAI(none, new FirstRunFailure.SignInFailed(), Copy, Enroll));
        Assert.Null(FirstRunNotices.Join(new FirstRunFailure.SignInFailed(), none, Copy, null, null, Enroll));
        Assert.Null(FirstRunNotices.JoinNearAI(chosen, new FirstRunFailure.SignInFailed(), Copy, Enroll));
        Assert.Equal(Copy.Folders.SignInFailed, FirstRunNotices.Join(new FirstRunFailure.SignInFailed(), chosen, Copy, null, null, Enroll));
        // A dead invite is said on the invite card.
        Assert.Null(FirstRunNotices.Join(new FirstRunFailure.InviteDead("invite-expired"), chosen, Copy, null, null, Enroll));
        // Start's failures first.
        Assert.Equal(Copy.Uses.CompleteFailed, FirstRunNotices.Join(new FirstRunFailure.CompleteFailed(), chosen, Copy, null, null, Enroll));
        Assert.Equal(Copy.Uses.SharingRefused, FirstRunNotices.Join(new FirstRunFailure.GrantRefused("x"), chosen, Copy, null, null, Enroll));
    }

    // ---- Join's invite card (JoinScreen.swift:203-335) ------------------------

    private static string? Host(string invite) => invite.StartsWith("TC-", StringComparison.Ordinal) ? "issuer.example" : null;

    [Fact]
    public void ALinksInviteIsKeptForTheCardAndJoinedAtSignIn()
    {
        // Review finding 2: an offered invite was held and never shown or sent.
        var state = new FirstRunState();
        FirstRunState offered = JoinInvite.Offer(" TC-1 ", state, Host);
        Assert.Equal("TC-1", offered.Invite);
        Assert.Equal("issuer.example", offered.IssuerHost);
        Assert.True(JoinInvite.ShowsCard(offered));
        Assert.Equal(new JoinInviteLine.Host("issuer.example"), JoinInvite.Line(offered, null, null, Copy.Join, refused: false));
        // Something that is not an invite is never kept; an enrolled one is not replaced.
        Assert.Same(state, JoinInvite.Offer("nonsense", state, Host));
        var enrolled = new FirstRunState { EnrolledInvite = "TC-0" };
        Assert.Same(enrolled, JoinInvite.Offer("TC-1", enrolled, Host));

        // The sign-in looks it up and joins it (FirstRunPlan.swift:160-176).
        offered.Account = AccountAnswer.NearAI;
        offered.ToolAnswers["claude-code"] = ToolChoice.Watch(@"C:\a");
        offered.ToolAnswers["codex"] = ToolChoice.Off;
        offered.DaemonStarted = true;
        offered.StartedSettingsJson = offered.SessionRoots.SettingsJson();
        Assert.Equal(
            new FirstRunCall[] { new FirstRunCall.LookupInvite("TC-1"), new FirstRunCall.Enroll("TC-1"), new FirstRunCall.SignInNearAI() },
            FirstRunPlan.Calls(offered, CommitPoint.SignIn));
    }

    [Fact]
    public void LookUpKeepsWithdrawsAndRefusesAsTheMacs()
    {
        var watchOnly = new FirstRunState { Account = AccountAnswer.WatchOnly };
        var found = JoinInvite.LookUp("TC-2", watchOnly, new FirstRunFailure.InviteDead("x"), Host);
        Assert.Equal(JoinLookUpOutcome.Found, found.Outcome);
        Assert.Equal(AccountAnswer.None, found.State.Account);
        Assert.Null(found.Failure);

        var same = JoinInvite.LookUp("TC-2", found.State, new FirstRunFailure.InviteDead("x"), Host);
        Assert.IsType<FirstRunFailure.InviteDead>(same.Failure);

        var withdrawn = JoinInvite.LookUp("  ", found.State, new FirstRunFailure.InviteDead("x"), Host);
        Assert.Equal(JoinLookUpOutcome.Withdrawn, withdrawn.Outcome);
        Assert.Equal("", withdrawn.State.Invite);
        Assert.Null(withdrawn.State.IssuerHost);
        Assert.Null(withdrawn.Failure);

        var refused = JoinInvite.LookUp("nonsense", found.State, null, Host);
        Assert.Equal(JoinLookUpOutcome.Refused, refused.Outcome);
        Assert.Equal("", refused.State.Invite);
        Assert.Equal(new JoinInviteLine.Error(Copy.Join.InviteError), JoinInvite.Line(refused.State, null, null, Copy.Join, refused: true));
        Assert.Equal(new JoinInviteLine.Error(Copy.Join.InviteDead), JoinInvite.Line(found.State, null, new FirstRunFailure.InviteDead("x"), Copy.Join, refused: false));
        Assert.True(JoinInvite.CanLookUp("", found.State));
        Assert.False(JoinInvite.CanLookUp(" ", new FirstRunState()));
    }

    [Fact]
    public void AJoinedInviteNamesItsHostAndRange()
    {
        var joined = new FirstRunState { EnrolledInvite = "TC-3", IssuerHost = "issuer.example" };
        Assert.False(JoinInvite.IsEditable(joined));
        Assert.False(JoinInvite.ShowsField(joined));
        string lookup = "{\"valid\":true,\"credit_range\":{\"min\":2,\"max\":5,\"unit\":\"points_per_accepted_trace\"}}";
        string range = FirstRunCopy.Fill(Copy.Join.PayRangePoints, new Dictionary<string, string> { ["min"] = "2", ["max"] = "5" });
        Assert.Equal(range, JoinInvite.PayRange(lookup, Copy.Join));
        Assert.Equal(
            new JoinInviteLine.Joined(FirstRunCopy.Fill(Copy.Join.InviteJoined, new Dictionary<string, string> { ["host"] = "issuer.example", ["pay_range"] = range })),
            JoinInvite.Line(joined, lookup, null, Copy.Join, refused: false));
        Assert.Equal(Copy.Join.Unknown, JoinInvite.PayRange("{\"credit_range\":{\"min\":2,\"max\":5,\"unit\":\"usd\"}}", Copy.Join));
        Assert.Equal(Copy.Join.Unknown, JoinInvite.PayRange(null, Copy.Join));
        Assert.Equal(FirstRunCopy.Fill(Copy.Join.PayRangePointsOne, new Dictionary<string, string> { ["min"] = "3" }),
            JoinInvite.PayRange("{\"credit_range\":{\"min\":3,\"max\":3,\"unit\":\"points_per_accepted_trace\"}}", Copy.Join));
        // An enrollment recorded with no invite never says "Joined".
        var recorded = new FirstRunState { EnrolledInvite = "" };
        Assert.True(JoinInvite.ShowsField(recorded));
        Assert.Equal(new JoinInviteLine.Hidden(), JoinInvite.Line(recorded, null, null, Copy.Join, refused: false));
    }

    // ---- Automatic's disclosures (SharingDisclosures.swift) -------------------

    [Fact]
    public void TheDisclosuresRunInOrderAndTheCoreDecidesTheGrant()
    {
        // Review finding 4: Automatic left Start dead. Start now shows the
        // scrub then the witness disclosure, and the core answers.
        var flow = new SharingDisclosureFlow();
        flow.AcknowledgeWitness("ignored before the scrub sheet");
        Assert.Equal(DisclosureStep.Scrub, flow.Step);
        Flow1GrantRequest? early = Flow1GrantRequest.Ask(flow.ProgressJson(true, new[] { "research" }));
        Assert.NotNull(early);
        Assert.False(early!.Ready);
        Assert.Contains("scrub_disclosure", early.Blockers);

        flow.AcknowledgeScrub();
        flow.AcknowledgeWitness(null);
        Assert.Equal(DisclosureStep.Done, flow.Step);
        Flow1GrantRequest? done = Flow1GrantRequest.Ask(flow.ProgressJson(true, new[] { "research" }));
        Assert.NotNull(done);
        Assert.True(done!.Ready, string.Join(",", done.Blockers));
        Assert.Null(done.WitnessSigningAddress);

        Flow1GrantRequest? offline = Flow1GrantRequest.Ask(flow.ProgressJson(false, new[] { "research" }));
        Assert.Equal("connect", offline!.Blockers[0]);
    }

    [Fact]
    public void ANotReadyAnswerFinishesOnAskAndSaysWhy()
    {
        var state = new FirstRunState { Account = AccountAnswer.NearAI, NearAIEnrolled = true, Sharing = SharingPath.Automatic, SharingChosen = true };
        Assert.True(SharingDisclosureFlow.IsNeeded(state));
        Assert.False(SharingDisclosureFlow.IsNeeded(new FirstRunState { Sharing = SharingPath.Automatic }));

        var (ready, none) = SharingDisclosureFlow.Resolve(state, new Flow1GrantRequest(true, Array.Empty<string>(), "0xabc"));
        Assert.Null(none);
        Assert.True(ready.GrantReady);
        Assert.Equal(SharingPath.Automatic, ready.Sharing);
        Assert.Equal("0xabc", ready.WitnessSigningAddress);

        var (asked, refused) = SharingDisclosureFlow.Resolve(state, new Flow1GrantRequest(false, new[] { "connect", "scope" }, null));
        Assert.Equal(SharingPath.AskMe, asked.Sharing);
        Assert.False(asked.GrantReady);
        Assert.Equal(new FirstRunFailure.GrantRefused("connect"), refused);
        Assert.Equal(new FirstRunFailure.GrantRefused(SharingDisclosureFlow.UnreadableLabel), SharingDisclosureFlow.Resolve(state, null).Refused);
        Assert.Null(Flow1GrantRequest.Decode("{\"ready\":\"yes\",\"blockers\":[]}"));
    }

    [Fact]
    public void TheScrubSheetHasTheCoresThreeLines()
    {
        string json = "{\"disclosure\":\"patterns_only\",\"patterns_only\":{\"scope\":\"S\",\"limit\":\"L\"},\"summary\":\"Sum\",\"no_review\":\"N\"}";
        Assert.Equal(new[] { "S", "L", "N" }, AutomaticGrantCopy.Decode(json)!.Lines);
        Assert.Null(AutomaticGrantCopy.Decode("{\"disclosure\":\"model_scrubbed\",\"patterns_only\":{\"scope\":\"S\",\"limit\":\"L\"},\"summary\":\"Sum\",\"no_review\":\"N\"}"));
        Assert.Null(AutomaticGrantCopy.Decode("{\"disclosure\":\"patterns_only\",\"patterns_only\":{\"scope\":\"S\",\"limit\":\"\"},\"summary\":\"Sum\",\"no_review\":\"N\"}"));
    }

    // ---- The sign-in's Back and Cancel (FirstRunRunner.swift:385-414) ---------

    private sealed class WaitingDaemon : IFirstRunDaemon
    {
        public int Cancels;
        public Task<bool> StartDaemon(string settingsJson) => Task.FromResult(true);
        public Task<bool> SetSourceSettings(string settingsJson) => Task.FromResult(true);
        public Task<FirstRunLookup> LookupInvite(string invite) => Task.FromResult<FirstRunLookup>(new FirstRunLookup.Unavailable());
        public Task<bool> EnrollInvite(string invite) => Task.FromResult(true);
        public Task<bool> SignInNearAI() => Task.FromResult(true);

        public async Task<NearAILoginOutcome> NearAILoginOutcome(CancellationToken cancel)
        {
            try
            {
                await Task.Delay(Timeout.Infinite, cancel);
            }
            catch (TaskCanceledException)
            {
            }

            return FirstRun.NearAILoginOutcome.Failed;
        }

        public Task<bool> CancelNearAILogin()
        {
            Cancels++;
            return Task.FromResult(true);
        }

        public Task<NearAIEnrolment> EnrollNearAI(string? referralCode) => Task.FromResult(new NearAIEnrolment(true, null));
        public Task<bool> SetAskEachTime() => Task.FromResult(true);
        public Task<bool> SaveConsentScopes(IReadOnlyList<string> scopes) => Task.FromResult(true);
        public Task<bool> SetProjectMode(string projectId, ProjectRule mode) => Task.FromResult(true);
        public Task<bool> IncludePastSessions(string projectId, IReadOnlyList<string> sessionIds) => Task.FromResult(true);
        public Task<bool> SetPrivateAI(bool on) => Task.FromResult(true);
        public Task<GrantAnswer> GrantAutomatic(string? witness) => Task.FromResult(new GrantAnswer(true, null));
        public Task<bool> MarkComplete() => Task.FromResult(true);
        public Task<bool> MarkWatchOnlyComplete() => Task.FromResult(true);
        public Task<bool> RecordTermsAcceptance(AcceptedTerms terms) => Task.FromResult(true);
        public void FirstRunFinished(FirstRunFailure? notice) { }
    }

    [Fact]
    public async Task BackWhileTheSignInWaitsCancelsItAndGoesBackAStep()
    {
        // Review finding 8: the 600 s wait could not be left.
        var state = new FirstRunState { Step = FirstRunStep.Join, Account = AccountAnswer.NearAI, DaemonStarted = true };
        state.ToolAnswers["claude-code"] = ToolChoice.Watch(@"C:\a");
        state.ToolAnswers["codex"] = ToolChoice.Off;
        state.StartedSettingsJson = state.SessionRoots.SettingsJson();
        var daemon = new WaitingDaemon();
        var runner = new FirstRunRunner(state, daemon);
        Task commit = runner.Commit(CommitPoint.SignIn);
        Assert.True(runner.SignInWaiting);
        Assert.Equal(CommitPoint.SignIn, runner.Committing);

        await runner.BackFromSignIn();
        // The wait ends: a Back that did not cancel would leave it running.
        Assert.Same(commit, await Task.WhenAny(commit, Task.Delay(5000)));
        Assert.Equal(1, daemon.Cancels);
        Assert.False(runner.IsCommitting);
        Assert.False(runner.SignInWaiting);
        Assert.Null(runner.Failure);
        Assert.Equal(FirstRunStep.Terms, runner.State.Step);

        // Back with nothing waiting does nothing.
        await runner.BackFromSignIn();
        Assert.Equal(FirstRunStep.Terms, runner.State.Step);
    }
}
