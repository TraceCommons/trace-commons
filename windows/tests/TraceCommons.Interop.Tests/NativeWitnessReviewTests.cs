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
    public void ABusyWitnessCarriesItsSentenceAndTryAgainLineAndNothingElseDoes()
    {
        const string busy = "{\"error\":{\"code\":\"unavailable\",\"message\":\"witness_saturated\"},"
            + "\"result\":{\"view\":{\"state\":\"Busy\",\"message\":\"m\","
            + "\"retry_at\":\"2030-01-01T00:01:00Z\",\"retry_label\":\"Try again after\"}}}";
        var response = DaemonResponse.Parse(busy);
        Assert.Equal("m", NativeWitnessReview.Refusal(response));
        Assert.Equal("Try again after: 2030-01-01T00:01:00Z",
            NativeWitnessReview.RetryLine(response, d => d.UtcDateTime.ToString("yyyy-MM-ddTHH:mm:ssZ")));
        Assert.Null(NativeWitnessReview.RetryLine(DaemonResponse.Parse(busy.Replace("Busy", "Refused"))));
        Assert.Null(NativeWitnessReview.RetryLine(DaemonResponse.Parse(busy.Replace("2030-01-01T00:01:00Z", "soon"))));
        Assert.Null(NativeWitnessReview.RetryLine(DaemonResponse.Parse("{\"result\":{\"status\":\"ready\"}}")));
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
