using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Tests for reading the daemon's answer to <c>quiesce</c>.
///
/// This mapping is the gate in front of an update: everything except a
/// confirmed quiesce must refuse, because the alternative is App Installer
/// terminating the process while an upload is on the wire. So each refusal
/// shape gets its own test rather than being lumped into a catch-all --
/// a refusal misread as success is the one failure that costs a contributor
/// a half-uploaded trace.
///
/// Pure JSON, no native library and no daemon, so these run anywhere.
/// </summary>
public class UpdateProtocolTests
{
    [Fact]
    public void AConfirmedQuiesceAllowsTheUpdate()
    {
        QuiesceOutcome outcome = UpdateProtocol.ReadQuiesce(
            DaemonResponse.Parse("{\"id\":1,\"result\":{\"quiesced\":true,\"waited_ms\":1200}}"));

        Assert.Equal(TcQuiesceOutcome.Quiesced, outcome.Outcome);
        Assert.Equal(1200, outcome.WaitedMs);
        Assert.True(outcome.CanUpdate);
    }

    [Fact]
    public void ATimeoutIsDistinguishedFromAPlainRefusal()
    {
        QuiesceOutcome outcome = UpdateProtocol.ReadQuiesce(
            DaemonResponse.Parse(
                "{\"id\":1,\"error\":{\"code\":\"busy\",\"message\":\"quiesce-timeout\"}}"));

        Assert.Equal(TcQuiesceOutcome.TimedOut, outcome.Outcome);
        Assert.False(outcome.CanUpdate);
    }

    [Fact]
    public void ABusyDaemonRefusesWithoutTimingOut()
    {
        QuiesceOutcome outcome = UpdateProtocol.ReadQuiesce(
            DaemonResponse.Parse(
                "{\"id\":1,\"error\":{\"code\":\"busy\",\"message\":\"another-quiesce-in-flight\"}}"));

        Assert.Equal(TcQuiesceOutcome.Busy, outcome.Outcome);
        Assert.False(outcome.CanUpdate);
    }

    [Fact]
    public void ADaemonTooOldToKnowTheMethodIsUnsupportedNotBroken()
    {
        QuiesceOutcome outcome = UpdateProtocol.ReadQuiesce(
            DaemonResponse.Parse(
                "{\"id\":1,\"error\":{\"code\":\"unknown_method\",\"message\":\"quiesce\"}}"));

        Assert.Equal(TcQuiesceOutcome.Unsupported, outcome.Outcome);
        Assert.False(outcome.CanUpdate);
    }

    [Fact]
    public void TheSynchronousRefusalIsAlsoUnsupported()
    {
        QuiesceOutcome outcome = UpdateProtocol.ReadQuiesce(
            DaemonResponse.Parse(
                "{\"id\":1,\"error\":{\"code\":\"bad_params\",\"message\":\"quiesce-requires-async\"}}"));

        Assert.Equal(TcQuiesceOutcome.Unsupported, outcome.Outcome);
    }

    [Fact]
    public void AStoppedDaemonIsUnavailable()
    {
        QuiesceOutcome outcome = UpdateProtocol.ReadQuiesce(
            DaemonResponse.Parse(
                "{\"error\":{\"code\":\"unavailable\",\"message\":\"daemon-not-started\"}}"));

        Assert.Equal(TcQuiesceOutcome.Unavailable, outcome.Outcome);
        Assert.False(outcome.CanUpdate);
    }

    [Fact]
    public void AResultFrameThatDoesNotClaimQuiescedIsNotTreatedAsSuccess()
    {
        QuiesceOutcome outcome = UpdateProtocol.ReadQuiesce(
            DaemonResponse.Parse("{\"id\":1,\"result\":{\"quiesced\":false,\"waited_ms\":0}}"));

        Assert.Equal(TcQuiesceOutcome.Unavailable, outcome.Outcome);
        Assert.False(outcome.CanUpdate);
    }

    [Fact]
    public void MalformedJsonRefusesRatherThanThrowing()
    {
        QuiesceOutcome outcome = UpdateProtocol.ReadQuiesce(DaemonResponse.Parse("not json"));

        Assert.Equal(TcQuiesceOutcome.Unavailable, outcome.Outcome);
    }

    [Fact]
    public void EveryRefusalHasAContributorFacingSentence()
    {
        foreach (TcQuiesceOutcome value in new[]
                 {
                     TcQuiesceOutcome.Busy,
                     TcQuiesceOutcome.TimedOut,
                     TcQuiesceOutcome.Unsupported,
                     TcQuiesceOutcome.Unavailable,
                 })
        {
            string text = UpdateProtocol.DescribeRefusal(value);
            Assert.False(string.IsNullOrWhiteSpace(text));
            Assert.EndsWith(".", text);
        }
    }

    [Fact]
    public void TheQuiesceMethodNameMatchesTheDaemonContract()
    {
        Assert.Equal("quiesce", DaemonProtocol.Methods.Quiesce);
    }
}
