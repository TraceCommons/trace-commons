using System.Collections.Generic;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The daemon-side preview scheduler is exercised in
/// <c>crates/trace-commons-contributor/src/daemon/preview_scheduler.rs</c> and
/// <c>tests/daemon_ipc_contract.rs</c>. This file covers the Windows shell's
/// half of the contract: decoding what the scheduler sends, and the pure
/// logic that decides when to call <c>preview_visible</c> and which entries
/// have left the queue for good. None of it touches WinUI, so it runs on a
/// machine that cannot build the App project at all.
/// </summary>
public sealed class PreviewCardOutcomeTests
{
    [Fact]
    public void AQueuedStateCarriesNoData()
    {
        PreviewCardOutcome outcome = Assert.IsType<PreviewCardOutcome>(
            PreviewCardOutcome.Parse("""{"entry_id":"e1","state":"queued"}"""));

        Assert.Equal("e1", outcome.EntryId);
        Assert.True(outcome.IsPending);
        Assert.False(outcome.IsReady);
        Assert.Null(outcome.Summary);
    }

    [Fact]
    public void ARunningStateIsAlsoPending()
    {
        PreviewCardOutcome outcome = Assert.IsType<PreviewCardOutcome>(
            PreviewCardOutcome.Parse("""{"entry_id":"e1","state":"running"}"""));

        Assert.True(outcome.IsPending);
    }

    [Fact]
    public void AReadyStateCarriesTheSummary()
    {
        const string json = """
            {
              "entry_id": "e1",
              "state": "ready",
              "summary": {
                "would_send_bytes": 4096,
                "raw_session_bytes": 3000,
                "event_count": 12,
                "opening_prompt": "fix the flaky test",
                "redactions": {"secret": 2},
                "pii_labels_present": [],
                "consent_scopes": ["model_training"],
                "residual_risk": "pattern_based_only",
                "input_fingerprint": "abc",
                "enrolled": true
              }
            }
            """;

        PreviewCardOutcome outcome = Assert.IsType<PreviewCardOutcome>(PreviewCardOutcome.Parse(json));

        Assert.True(outcome.IsReady);
        Assert.False(outcome.IsPending);
        PreviewSummary summary = Assert.IsType<PreviewSummary>(outcome.Summary);
        Assert.Equal(4096, summary.WouldSendBytes);
        Assert.Equal("fix the flaky test", summary.OpeningPrompt);
    }

    /// <summary>
    /// The one rule that must never regress: a too-large refusal carries a
    /// raw byte count and NOTHING that could be read as what would have been
    /// sent. There is no <c>would_send_bytes</c> and no <c>summary</c> in the
    /// wire shape at all, and this asserts both are absent from the decoded
    /// type too, not merely that the JSON lacks the field.
    /// </summary>
    [Fact]
    public void ATooLargeRefusalCarriesOnlyTheRawSize()
    {
        const string json = """
            {"entry_id":"e1","state":"too_large","raw_session_bytes":385351680,"limit_bytes":67108864}
            """;

        PreviewCardOutcome outcome = Assert.IsType<PreviewCardOutcome>(PreviewCardOutcome.Parse(json));

        Assert.True(outcome.IsTooLarge);
        Assert.Equal(385351680, outcome.RawSessionBytes);
        Assert.Equal(67108864, outcome.LimitBytes);
        Assert.Null(outcome.Summary);
        Assert.Equal("too large to preview", PreviewCardOutcome.TooLargeText);
    }

    [Fact]
    public void AFailedStateCarriesTheFixedLabel()
    {
        PreviewCardOutcome outcome = Assert.IsType<PreviewCardOutcome>(PreviewCardOutcome.Parse(
            """{"entry_id":"e1","state":"failed","code":"internal","label":"scrub-failed"}"""));

        Assert.True(outcome.IsFailed);
        Assert.Equal("internal", outcome.Code);
        Assert.Equal("scrub-failed", outcome.Label);
    }

    [Theory]
    [InlineData("")]
    [InlineData("not json")]
    [InlineData("""{"entry_id":"e1"}""")]
    [InlineData("""{"state":"ready"}""")]
    [InlineData(null)]
    public void AnUnreadableOutcomeIsNullRatherThanAThrow(string? json)
    {
        Assert.Null(PreviewCardOutcome.Parse(json));
    }

    /// <summary>
    /// <see cref="DaemonEvent.PreviewOutcome"/> is the seam <c>DaemonHost</c>
    /// reads a <c>preview_ready</c> frame's payload through -- asserted here
    /// on the full event envelope, not just the inner object, since that is
    /// what actually arrives at the subscription callback.
    /// </summary>
    [Fact]
    public void APreviewReadyEventDecodesThroughDaemonEvent()
    {
        DaemonEvent evt = Assert.IsType<DaemonEvent>(DaemonEvent.Parse(
            """{"event":"preview_ready","data":{"entry_id":"e1","state":"ready","summary":{"would_send_bytes":1}}}"""));

        Assert.Equal(DaemonProtocol.Events.PreviewReady, evt.Event);
        PreviewCardOutcome outcome = Assert.IsType<PreviewCardOutcome>(evt.PreviewOutcome);
        Assert.Equal("e1", outcome.EntryId);
        Assert.True(outcome.IsReady);
    }

    [Fact]
    public void ANonPreviewEventHasNoPreviewOutcome()
    {
        DaemonEvent evt = Assert.IsType<DaemonEvent>(DaemonEvent.Parse("""{"event":"queue_changed"}"""));

        Assert.Null(evt.PreviewOutcome);
    }
}

public sealed class PreviewVisibilityTrackerTests
{
    [Fact]
    public void TheFirstSettleAlwaysSendsEvenAnEmptySet()
    {
        var tracker = new PreviewVisibilityTracker();

        string? sent = tracker.OnSettled(System.Array.Empty<string>());

        Assert.NotNull(sent);
        using JsonDocument doc = JsonDocument.Parse(sent!);
        Assert.Empty(doc.RootElement.GetProperty("entry_ids").EnumerateArray());
    }

    [Fact]
    public void TheParamsCarryExactlyTheVisibleIdsSortedForDeterminism()
    {
        var tracker = new PreviewVisibilityTracker();

        string? sent = tracker.OnSettled(new[] { "c", "a", "b" });

        using JsonDocument doc = JsonDocument.Parse(sent!);
        var ids = new List<string>();
        foreach (JsonElement el in doc.RootElement.GetProperty("entry_ids").EnumerateArray())
        {
            ids.Add(el.GetString()!);
        }

        Assert.Equal(new[] { "a", "b", "c" }, ids);
    }

    /// <summary>
    /// The whole point of the tracker: a repeated settle over an unchanged
    /// set produces nothing to send, even though the daemon would answer it
    /// happily. This is what makes "debounced, not per frame" a fact about
    /// wire traffic and not just about how often the recompute runs.
    /// </summary>
    [Fact]
    public void AnUnchangedSetProducesNothingToSend()
    {
        var tracker = new PreviewVisibilityTracker();
        tracker.OnSettled(new[] { "a", "b" });

        string? second = tracker.OnSettled(new[] { "b", "a" });

        Assert.Null(second);
    }

    [Fact]
    public void AChangedSetSendsAgain()
    {
        var tracker = new PreviewVisibilityTracker();
        tracker.OnSettled(new[] { "a", "b" });

        string? second = tracker.OnSettled(new[] { "a", "c" });

        Assert.NotNull(second);
    }

    [Fact]
    public void ResetForcesTheNextCallToSendEvenIfUnchanged()
    {
        var tracker = new PreviewVisibilityTracker();
        tracker.OnSettled(new[] { "a" });

        tracker.Reset();
        string? afterReset = tracker.OnSettled(new[] { "a" });

        Assert.NotNull(afterReset);
    }
}

public sealed class PreviewCancellationTests
{
    [Fact]
    public void EntriesThatDroppedOutOfTheQueueAreReported()
    {
        IReadOnlyList<string> removed = PreviewCancellation.EntriesRemoved(
            previousIds: new[] { "a", "b", "c" },
            currentIds: new[] { "a", "c" });

        Assert.Equal(new[] { "b" }, removed);
    }

    [Fact]
    public void NothingRemovedWhenTheSetIsUnchanged()
    {
        IReadOnlyList<string> removed = PreviewCancellation.EntriesRemoved(
            previousIds: new[] { "a", "b" },
            currentIds: new[] { "b", "a" });

        Assert.Empty(removed);
    }

    /// <summary>
    /// Scrolling an entry off screen must never look like this diff: this
    /// type only ever sees the two full entry-id sets of the WHOLE pending
    /// queue, and an entry still in that queue is never "removed" no matter
    /// what a scroll position says. The visibility tracker is a completely
    /// separate signal.
    /// </summary>
    [Fact]
    public void AnEntryStillInTheQueueIsNeverReportedAsRemoved()
    {
        IReadOnlyList<string> removed = PreviewCancellation.EntriesRemoved(
            previousIds: new[] { "a", "b" },
            currentIds: new[] { "a", "b" });

        Assert.Empty(removed);
    }

    [Fact]
    public void ADuplicateInPreviousIdsIsReportedOnceOnly()
    {
        IReadOnlyList<string> removed = PreviewCancellation.EntriesRemoved(
            previousIds: new[] { "a", "a", "b" },
            currentIds: new[] { "b" });

        Assert.Equal(new[] { "a" }, removed);
    }

    [Fact]
    public void AnEmptyPreviousSetRemovesNothing()
    {
        Assert.Empty(PreviewCancellation.EntriesRemoved(
            previousIds: System.Array.Empty<string>(),
            currentIds: new[] { "a" }));
    }
}
