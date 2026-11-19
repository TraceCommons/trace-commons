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
/// The window's buttons, driven through <see cref="FirstRunController"/>,
/// the one thing FirstRunWindow's click handlers call (checklist B5): each
/// test fails if the controller stops making the call it names.
/// </summary>
public sealed class FirstRunControllerTests
{
    private static readonly FirstRunCopy Copy = FirstRunCopy.Load() ?? throw new InvalidOperationException("the core's first-run copy did not decode");

    /// <summary>The core's own automatic contribution copy, for a computer with no configuration yet.</summary>
    private static readonly AutomaticGrantCopy Grant = LoadGrant();

    private static AutomaticGrantCopy LoadGrant()
    {
        string dir = Path.Combine(Path.GetTempPath(), "tc-firstrun-grant-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(dir);
        try
        {
            return AutomaticGrantCopy.Load(dir) ?? throw new InvalidOperationException("the core's automatic contribution copy did not decode");
        }
        finally
        {
            Directory.Delete(dir, true);
        }
    }

    private sealed class Daemon : IFirstRunDaemon
    {
        public readonly List<string> Calls = new();
        public NearAILoginOutcome Login = FirstRun.NearAILoginOutcome.SignedIn;

        public Task<bool> StartDaemon(string settingsJson) { Calls.Add("start"); return Task.FromResult(true); }
        public Task<bool> SetSourceSettings(string settingsJson) { Calls.Add("settings"); return Task.FromResult(true); }
        public Task<FirstRunLookup> LookupInvite(string invite) { Calls.Add("lookup"); return Task.FromResult<FirstRunLookup>(new FirstRunLookup.Found("{}")); }
        public Task<bool> EnrollInvite(string invite) { Calls.Add("enroll"); return Task.FromResult(true); }
        public Task<bool> SignInNearAI() { Calls.Add("signin"); return Task.FromResult(true); }
        public Task<NearAILoginOutcome> NearAILoginOutcome(CancellationToken cancel) { Calls.Add("login"); return Task.FromResult(Login); }
        public Task<bool> CancelNearAILogin() { Calls.Add("cancel"); return Task.FromResult(true); }
        public Task<NearAIEnrolment> EnrollNearAI(string? referralCode) { Calls.Add("enroll-near"); return Task.FromResult(new NearAIEnrolment(true, null)); }
        public Task<bool> SetAskEachTime() { Calls.Add("ask"); return Task.FromResult(true); }
        public Task<bool> SaveConsentScopes(IReadOnlyList<string> scopes) { Calls.Add("scopes"); return Task.FromResult(true); }
        public Task<bool> SetProjectMode(string projectId, ProjectRule mode) { Calls.Add("mode"); return Task.FromResult(true); }
        public Task<bool> IncludePastSessions(string projectId, IReadOnlyList<string> sessionIds) { Calls.Add("include"); return Task.FromResult(true); }
        public Task<bool> SetPrivateAI(bool on) { Calls.Add("private"); return Task.FromResult(true); }
        public Task<GrantAnswer> GrantAutomatic(string? witness) { Calls.Add("grant " + (witness ?? "-")); return Task.FromResult(new GrantAnswer(true, null)); }
        public Task<bool> MarkComplete() { Calls.Add("mark"); return Task.FromResult(true); }
        public Task<bool> MarkWatchOnlyComplete() { Calls.Add("mark-watch"); return Task.FromResult(true); }
        public Task<bool> RecordTermsAcceptance(AcceptedTerms terms) { Calls.Add("terms"); return Task.FromResult(true); }
        public void FirstRunFinished(FirstRunFailure? notice) => Calls.Add("finished " + (notice?.GetType().Name ?? "-"));
    }

    /// <summary>The window's sheets and reads, recorded; each answers as told.</summary>
    private sealed class Sheets : IFirstRunSheets
    {
        public readonly List<string> Shown = new();
        public bool Pass = true;
        public string? Witness = "0xwitness";
        public bool LoggedIn = true;
        public Flow1GrantRequest? Answer = new(true, Array.Empty<string>(), "0xwitness");
        public string? Progress;

        public Task<bool> ShowDisclosures(SharingDisclosureFlow flow, AutomaticGrantCopy grant)
        {
            Shown.Add("scrub:" + string.Join("|", grant.Lines));
            if (!Pass)
            {
                return Task.FromResult(false);
            }

            flow.AcknowledgeScrub();
            Shown.Add("witness");
            flow.AcknowledgeWitness(Witness);
            return Task.FromResult(true);
        }

        public Task<bool> Connected() => Task.FromResult(LoggedIn);

        public Task<Flow1GrantRequest?> GrantRequest(string progressJson)
        {
            Progress = progressJson;
            return Task.FromResult(Answer);
        }
    }

    private static FirstRunState OnJoin(SharingPath sharing)
    {
        var state = new FirstRunState { Step = FirstRunStep.Join, Account = AccountAnswer.Enrolled, NearAIEnrolled = true, SignedIn = true, DaemonStarted = true };
        state.ToolAnswers["claude-code"] = ToolChoice.Watch(@"C:\a");
        state.ToolAnswers["codex"] = ToolChoice.Off;
        state.StartedSettingsJson = state.SessionRoots.SettingsJson();
        state.Sharing = sharing;
        state.SharingChosen = true;
        return state;
    }

    private static (FirstRunController Controller, Daemon Daemon, Sheets Sheets) Make(FirstRunState state, AutomaticGrantCopy? grant)
    {
        var daemon = new Daemon();
        var sheets = new Sheets();
        var controller = new FirstRunController(new FirstRunRunner(state, daemon), sheets, () => Task.FromResult(grant), Copy.Uses)
        {
            RequiredScope = "debugging_evaluation",
        };
        return (controller, daemon, sheets);
    }

    [Fact]
    public async Task ContinueOnToolsStartsTheDaemonAndMovesOn()
    {
        var state = new FirstRunState { Step = FirstRunStep.Tools };
        state.ToolAnswers["claude-code"] = ToolChoice.Off;
        var (controller, daemon, _) = Make(state, Grant);
        Assert.False(controller.CanContinue);
        controller.AnswerTool("claude-code", @"C:\a", true);
        controller.AnswerTool("codex", @"C:\b", false);
        Assert.True(controller.CanContinue);

        await controller.Continue();
        Assert.Equal(new[] { "start" }, daemon.Calls);
        Assert.Equal(FirstRunStep.PrivateAI, controller.Runner.State.Step);
    }

    [Fact]
    public async Task ContinueAndBackWalkTheSteps()
    {
        var (controller, _, _) = Make(new FirstRunState { Step = FirstRunStep.PrivateAI }, Grant);
        await controller.Continue();
        Assert.Equal(FirstRunStep.Traces, controller.Runner.State.Step);
        await controller.Continue();
        Assert.Equal(FirstRunStep.Uses, controller.Runner.State.Step);
        await controller.Back();
        Assert.Equal(FirstRunStep.Traces, controller.Runner.State.Step);
        controller.BackTo(FirstRunStep.Tools);
        Assert.Equal(FirstRunStep.Tools, controller.Runner.State.Step);
    }

    [Fact]
    public async Task UsesChoicesLandInTheState()
    {
        var (controller, _, _) = Make(new FirstRunState { Step = FirstRunStep.Uses }, Grant);
        controller.ChooseSharing(SharingPath.Automatic);
        Assert.True(controller.Runner.State.SharingChosen);
        Assert.Equal(SharingPath.Automatic, controller.Runner.State.Sharing);
        controller.SetScope("research", true);
        Assert.Contains("research", controller.Runner.State.Scopes);
        controller.SetScope("research", false);
        Assert.DoesNotContain("research", controller.Runner.State.Scopes);
        await Task.CompletedTask;
    }

    [Fact]
    public async Task TermsAreAcceptedForTheDocumentShownAndContinueTakesThem()
    {
        var (controller, _, _) = Make(new FirstRunState { Step = FirstRunStep.Terms }, Grant);
        controller.Terms = new FetchedTerms("v1", "e", true, "T", Array.Empty<TermsBlock>(), "abc");
        Assert.False(controller.CanContinue);
        controller.AcceptTerms(true, DateTimeOffset.UnixEpoch);
        Assert.True(controller.CanContinue);
        await controller.Continue();
        Assert.Equal(FirstRunStep.Join, controller.Runner.State.Step);
        controller.AcceptTerms(false, DateTimeOffset.UnixEpoch);
        Assert.Null(controller.Runner.AcceptedTerms);
    }

    [Fact]
    public async Task SignInChoosesNearAIAndCommitsTheSignIn()
    {
        var state = OnJoin(SharingPath.AskMe);
        state.Account = AccountAnswer.None;
        state.NearAIEnrolled = false;
        state.SignedIn = false;
        state.SharingChosen = false;
        var (controller, daemon, _) = Make(state, Grant);
        await controller.SignIn();
        Assert.Equal(AccountAnswer.NearAI, controller.Runner.State.Account);
        Assert.Contains("login", daemon.Calls);
        Assert.Contains("enroll-near", daemon.Calls);
        Assert.True(controller.Runner.State.SignedIn);
    }

    [Fact]
    public async Task StartOnAutomaticShowsBothSheetsThenCommitsTheCoresAnswer()
    {
        var (controller, daemon, sheets) = Make(OnJoin(SharingPath.Automatic), Grant);
        await controller.ReadGrantCopy();
        Assert.True(controller.CanContinue);

        await controller.Continue();
        Assert.Equal(new[] { "scrub:" + string.Join("|", Grant.Lines), "witness" }, sheets.Shown);
        using (JsonDocument progress = JsonDocument.Parse(sheets.Progress!))
        {
            Assert.True(progress.RootElement.GetProperty("witness_disclosure_seen").GetBoolean());
            Assert.Equal("0xwitness", progress.RootElement.GetProperty("witness_shown").GetString());
            Assert.Contains("debugging_evaluation", progress.RootElement.GetProperty("scopes_saved").EnumerateArray().Select(e => e.GetString()));
        }

        Assert.Contains("grant 0xwitness", daemon.Calls);
        Assert.Equal("finished -", daemon.Calls[^1]);
        Assert.True(controller.Runner.Completed);
    }

    [Fact]
    public async Task ACancelledSheetStartsNothing()
    {
        var (controller, daemon, sheets) = Make(OnJoin(SharingPath.Automatic), Grant);
        await controller.ReadGrantCopy();
        sheets.Pass = false;
        await controller.Continue();
        Assert.Empty(daemon.Calls);
        Assert.False(controller.Starting);
        Assert.Equal(SharingPath.Automatic, controller.Runner.State.Sharing);
    }

    [Fact]
    public async Task ANotReadyAnswerFinishesOnAskAndCarriesTheRefusal()
    {
        var (controller, daemon, sheets) = Make(OnJoin(SharingPath.Automatic), Grant);
        await controller.ReadGrantCopy();
        sheets.Answer = new Flow1GrantRequest(false, new[] { "connect" }, null);
        await controller.Continue();
        Assert.DoesNotContain(daemon.Calls, c => c.StartsWith("grant", StringComparison.Ordinal));
        Assert.Equal("finished GrantRefused", daemon.Calls[^1]);
    }

    [Fact]
    public async Task UnreadableScrubCopyShowsNoSheetAndStartsNothing()
    {
        // The Mac (UsesScreen.swift:100-108, :151-157, :705-709;
        // JoinScreen.swift:417-422): with the automatic contribution copy
        // unreadable, the Sharing card says it is unavailable, Start is off,
        // and no sheet shows. Windows used to finish Automatic on Ask and
        // say sharing_refused.
        foreach (SharingPath path in new[] { SharingPath.Automatic, SharingPath.AskMe })
        {
            var (controller, daemon, sheets) = Make(OnJoin(path), null);
            await controller.ReadGrantCopy();
            Assert.True(controller.SharingUnavailable);
            Assert.False(controller.CanContinue, path + ": Start is off without the copy");

            await controller.Continue();
            Assert.Empty(sheets.Shown);
            Assert.Empty(daemon.Calls);
            Assert.Null(controller.Runner.Failure);
            Assert.False(controller.Runner.Completed);
            Assert.Equal(path, controller.Runner.State.Sharing);
        }
    }

    [Fact]
    public async Task StartWaitsForTheCopyToBeRead()
    {
        var (controller, _, _) = Make(OnJoin(SharingPath.AskMe), Grant);
        Assert.False(controller.SharingUnavailable, "not read is not unavailable");
        Assert.False(controller.CanContinue, "the Mac's sharingLine is the fallback until the copy is read");
        await controller.ReadGrantCopy();
        Assert.True(controller.CanContinue);
    }

    [Fact]
    public async Task BackDuringTheBrowserWaitGoesThroughTheRunner()
    {
        var waiting = new TaskCompletionSource<NearAILoginOutcome>();
        var state = OnJoin(SharingPath.AskMe);
        state.Account = AccountAnswer.NearAI;
        state.NearAIEnrolled = false;
        state.SignedIn = false;
        var daemon = new HoldingDaemon(waiting);
        var controller = new FirstRunController(new FirstRunRunner(state, daemon), new Sheets(), () => Task.FromResult<AutomaticGrantCopy?>(Grant), Copy.Uses);
        Task signIn = controller.SignIn();
        Assert.True(controller.BackWhileCommitting);
        Assert.True(controller.BackEnabled);
        await controller.Back();
        await signIn;
        Assert.Equal(1, daemon.Cancels);
        Assert.Equal(FirstRunStep.Terms, controller.Runner.State.Step);
    }

    private sealed class HoldingDaemon : IFirstRunDaemon
    {
        private readonly TaskCompletionSource<NearAILoginOutcome> _outcome;
        public int Cancels;

        public HoldingDaemon(TaskCompletionSource<NearAILoginOutcome> outcome) => _outcome = outcome;

        public Task<bool> StartDaemon(string settingsJson) => Task.FromResult(true);
        public Task<bool> SetSourceSettings(string settingsJson) => Task.FromResult(true);
        public Task<FirstRunLookup> LookupInvite(string invite) => Task.FromResult<FirstRunLookup>(new FirstRunLookup.Unavailable());
        public Task<bool> EnrollInvite(string invite) => Task.FromResult(true);
        public Task<bool> SignInNearAI() => Task.FromResult(true);
        public Task<NearAILoginOutcome> NearAILoginOutcome(CancellationToken cancel)
        {
            cancel.Register(() => _outcome.TrySetResult(FirstRun.NearAILoginOutcome.Cancelled));
            return _outcome.Task;
        }

        public Task<bool> CancelNearAILogin() { Cancels++; return Task.FromResult(true); }
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

    // ---- The Sharing info button (UsesScreen.swift:159-185) -------------------

    [Fact]
    public void TheCoresCopyCarriesBothPathsWords()
    {
        // Decoded from the core's own answer: every path_* field the Mac
        // reads is there and not empty, except Ask's cut first line.
        Assert.NotNull(Grant.PathAskFirstTitle);
        Assert.False(string.IsNullOrEmpty(Grant.PathAskFirstDetail));
        Assert.False(string.IsNullOrEmpty(Grant.PathAutomaticTitle));
        Assert.False(string.IsNullOrEmpty(Grant.PathAutomaticDetail));
        Assert.NotNull(Grant.Chosen);
    }

    [Fact]
    public void TheInfoButtonSaysThePathsDetail()
    {
        Assert.Equal(Grant.PathAskFirstDetail, UsesSharing.Detail(SharingPath.AskMe, Grant, Copy.Uses));
        string automatic = UsesSharing.Detail(SharingPath.Automatic, Grant, Copy.Uses)!;
        string lead = Copy.Uses.AutomaticScope.Length > 0 ? Copy.Uses.AutomaticScope : Grant.PathAutomaticDetail!;
        Assert.Equal(string.Join(" ", lead, Grant.Chosen!.Scope, Grant.Chosen.Limit), automatic);
        Assert.Null(UsesSharing.Detail(SharingPath.AskMe, null, Copy.Uses));
        Assert.Null(UsesSharing.Detail(SharingPath.Automatic, Grant with { PathAutomaticDetail = "" }, Copy.Uses));
        Assert.Null(UsesSharing.Detail(SharingPath.AskMe, Grant with { PathAskFirstTitle = null }, Copy.Uses));
    }

    [Fact]
    public async Task TheControllerOffersTheChosenPathsDetail()
    {
        var state = new FirstRunState { Step = FirstRunStep.Uses };
        var (controller, _, _) = Make(state, Grant);
        Assert.Null(controller.SharingDetail);
        await controller.ReadGrantCopy();
        Assert.Equal(UsesSharing.Detail(state.Sharing, Grant, Copy.Uses), controller.SharingDetail);
        controller.ChooseSharing(SharingPath.Automatic);
        Assert.Equal(UsesSharing.Detail(SharingPath.Automatic, Grant, Copy.Uses), controller.SharingDetail);
    }
}
