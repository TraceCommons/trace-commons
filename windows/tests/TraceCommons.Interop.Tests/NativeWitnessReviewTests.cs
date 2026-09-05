using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

public sealed class NativeWitnessReviewTests
{
    [Fact]
    public void UnknownCapabilitiesAndIncompleteResponsesFailClosed()
    {
        Assert.False(NativeWitnessReview.Supports(DaemonResponse.Parse("{\"result\":{}}")));
        Assert.True(NativeWitnessReview.Supports(DaemonResponse.Parse("{\"result\":{\"methods\":[\"witness_preview_request\"]}}")));
        Assert.False(NativeWitnessReview.IsReady(DaemonResponse.Parse("{\"result\":{\"status\":\"queued\"}}")));
        Assert.True(NativeWitnessReview.IsReady(DaemonResponse.Parse("{\"result\":{\"status\":\"ready\"}}")));
    }

    [Fact]
    public void ConfirmedRequestDoesNotApproveOrChangeOutcome()
    {
        using var json = JsonDocument.Parse(NativeWitnessReview.ConfirmedRequest("entry"));
        Assert.Equal("entry", json.RootElement.GetProperty("entry_id").GetString());
        Assert.True(json.RootElement.GetProperty("raw_session_confirmed").GetBoolean());
        Assert.False(json.RootElement.TryGetProperty("outcome", out _));
        Assert.False(json.RootElement.TryGetProperty("correction", out _));
    }

    [Fact]
    public void SharedDisclosureIsCompleteAndDistinguishesReviewFromApproval()
    {
        var copy = WitnessSurface.Copy();
        Assert.NotNull(copy?.Review);
        Assert.True(copy!.Review!.IsComplete);
        Assert.Contains("before you approve", copy.Review.Disclosure);
        Assert.Contains("may already", copy.Review.Failed);
        Assert.Contains("not a spendable", copy.Onboarding!.FollowUp);
    }
}
