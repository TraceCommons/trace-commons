using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using TraceCommons.Interop.FirstRun;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Debug-only first-run preview opens only at a named step and can
/// never confirm a daemon call, so a screenshot run cannot start a daemon,
/// enroll, accept terms or finish this machine's first run.
/// </summary>
public sealed class FirstRunPreviewTests
{
    [Theory]
    [InlineData("Welcome", FirstRunStep.Welcome)]
    [InlineData("tools", FirstRunStep.Tools)]
    [InlineData(" USES ", FirstRunStep.Uses)]
    [InlineData("traces", FirstRunStep.Traces)]
    [InlineData("privateai", FirstRunStep.PrivateAI)]
    [InlineData("Terms", FirstRunStep.Terms)]
    [InlineData("join", FirstRunStep.Join)]
    public void NamedStepsOpen(string value, FirstRunStep expected)
    {
        Assert.Equal(expected, FirstRunPreview.StepFrom(value));
    }

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("  ")]
    [InlineData("0")]
    [InlineData("3")]
    [InlineData("connect")]
    [InlineData("private ai")]
    public void AnythingElseOpensNothing(string? value)
    {
        Assert.Null(FirstRunPreview.StepFrom(value));
    }

    [Fact]
    public void TheVariableIsNamedForTheSampleLaunch()
    {
        Assert.Equal("TRACE_COMMONS_FIRST_RUN_STEP", FirstRunPreview.EnvironmentVariable);
    }

    [Fact]
    public async Task EveryCallIsRefusedAndRecorded()
    {
        var daemon = new RefusingFirstRunDaemon();
        Assert.False(await daemon.StartDaemon("{}"));
        Assert.False(await daemon.SetSourceSettings("{}"));
        Assert.IsType<FirstRunLookup.Unavailable>(await daemon.LookupInvite("x"));
        Assert.False(await daemon.EnrollInvite("x"));
        Assert.False(await daemon.SignInNearAI());
        Assert.Equal(NearAILoginOutcome.Failed, await daemon.NearAILoginOutcome(default));
        Assert.False(await daemon.CancelNearAILogin());
        Assert.False((await daemon.EnrollNearAI(null)).Enrolled);
        Assert.False(await daemon.SetAskEachTime());
        Assert.False(await daemon.SaveConsentScopes(new[] { "a" }));
        Assert.False(await daemon.SetProjectMode("p", default));
        Assert.False(await daemon.IncludePastSessions("p", Array.Empty<string>()));
        Assert.False(await daemon.SetPrivateAI(true));
        Assert.False((await daemon.GrantAutomatic(null)).Granted);
        Assert.False(await daemon.MarkComplete());
        Assert.False(await daemon.MarkWatchOnlyComplete());
        Assert.False(await daemon.RecordTermsAcceptance(new AcceptedTerms("v", "s", DateTimeOffset.UnixEpoch)));
        daemon.FirstRunFinished(null);
        Assert.Equal(18, daemon.Calls.Count);
    }

    [Fact]
    public async Task ARunnerOnItNeverLeavesToolsOrStarts()
    {
        var daemon = new RefusingFirstRunDaemon();
        var runner = new FirstRunRunner(new FirstRunState { Step = FirstRunStep.Tools }, daemon);
        await runner.Commit(CommitPoint.LeaveTools);
        Assert.Equal(FirstRunStep.Tools, runner.State.Step);
        Assert.False(runner.State.DaemonStarted);
        Assert.DoesNotContain(nameof(IFirstRunDaemon.MarkComplete), daemon.Calls);
    }
}
