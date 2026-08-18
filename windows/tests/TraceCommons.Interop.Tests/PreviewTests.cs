using System;
using System.Collections.Generic;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The consent invariant, in the one place it can be tested off Windows.
///
/// Everything the preview sheet decides -- whether Contribute may be pressed,
/// what the summary says, where the redaction chips go, what an excerpt shows,
/// how long an undo lasts -- lives in the interop assembly precisely so these
/// exist. The XAML that draws them cannot be built on this machine; the rules
/// it obeys can be.
/// </summary>
public sealed class ReadGateTests
{
    [Fact]
    public void NothingIsArmedToBeginWith()
    {
        var gate = new ReadGate();

        Assert.False(gate.CanContribute);
        Assert.False(gate.TranscriptShown);
        Assert.False(gate.Acknowledged);
        Assert.Equal(ReadGate.OpenPrompt, gate.OpenedLine);
    }

    [Fact]
    public void APinnedPreviewAloneDoesNotArmContribute()
    {
        // This is the misclick the gate exists to prevent: a loaded preview
        // is not a read one.
        var gate = new ReadGate();
        gate.SetPinnedPreview(true);

        Assert.False(gate.CanContribute);
        Assert.Equal(ReadGate.BlockedHelp, gate.Help);
    }

    [Fact]
    public void ShowingTheTranscriptAloneDoesNotArmContribute()
    {
        var gate = new ReadGate();
        gate.SetPinnedPreview(true);
        gate.MarkTranscriptShown();

        Assert.False(gate.CanContribute);
        Assert.Equal(ReadGate.Opened, gate.OpenedLine);
    }

    [Fact]
    public void AcknowledgingBeforeTheTranscriptIsShownIsRefused()
    {
        // Order is part of the invariant: ticking a box about bytes nobody
        // has seen acknowledges nothing. The UI disables the control, but the
        // rule is enforced here so it holds whatever drives it.
        var gate = new ReadGate();
        gate.SetPinnedPreview(true);

        gate.Acknowledged = true;

        Assert.False(gate.Acknowledged);
        Assert.False(gate.CanContribute);
    }

    [Fact]
    public void AllThreeConditionsArmContribute()
    {
        var gate = new ReadGate();
        gate.SetPinnedPreview(true);
        gate.MarkTranscriptShown();
        gate.Acknowledged = true;

        Assert.True(gate.CanContribute);
        Assert.Equal(ReadGate.ReadyHelp, gate.Help);
    }

    [Fact]
    public void AnUnenrolledPreviewCanNeverBeContributed()
    {
        // Nothing was pinned, so there is no envelope for an approval to bind
        // to. Reading it all the way through does not change that.
        var gate = new ReadGate();
        gate.MarkTranscriptShown();
        gate.Acknowledged = true;

        Assert.False(gate.CanContribute);
        Assert.Equal(ReadGate.UnenrolledHelp, gate.Help);
    }

    [Fact]
    public void ResetClearsConsentSoTheNextSessionStartsFromZero()
    {
        // The whole point: consent covers the bytes that were shown. A gate
        // carried into the next session would let it inherit the first
        // session's approval.
        var gate = new ReadGate();
        gate.SetPinnedPreview(true);
        gate.MarkTranscriptShown();
        gate.Acknowledged = true;

        gate.Reset();

        Assert.False(gate.CanContribute);
        Assert.False(gate.TranscriptShown);
        Assert.False(gate.Acknowledged);
    }

    [Fact]
    public void EveryConditionRaisesChanged()
    {
        // The sheet re-evaluates Contribute from this event alone, so a
        // silent transition would leave the button disagreeing with the gate.
        var gate = new ReadGate();
        int raised = 0;
        gate.Changed += () => raised++;

        gate.SetPinnedPreview(true);
        gate.MarkTranscriptShown();
        gate.Acknowledged = true;
        gate.Acknowledged = false;
        gate.Reset();

        Assert.Equal(5, raised);
    }

    [Fact]
    public void RepeatedTransitionsDoNotRaiseChanged()
    {
        var gate = new ReadGate();
        gate.SetPinnedPreview(true);
        gate.MarkTranscriptShown();
        int raised = 0;
        gate.Changed += () => raised++;

        gate.SetPinnedPreview(true);
        gate.MarkTranscriptShown();

        Assert.Equal(0, raised);
    }
}

public sealed class PreviewSummaryTests
{
    private const string Json = """
        {
          "would_send_bytes": 86016,
          "raw_session_bytes": 71234,
          "event_count": 148,
          "opening_prompt": "fix the flaky test",
          "redactions": { "secret": 12, "token": 4, "path": 31 },
          "pii_labels_present": ["PERSON"],
          "consent_scopes": ["debugging_evaluation", "model_training"],
          "residual_risk": "pattern_based_only",
          "envelope_digest": "abc",
          "input_fingerprint": "def",
          "enrolled": true
        }
        """;

    [Fact]
    public void TheContractShapeParses()
    {
        PreviewSummary summary = Assert.IsType<PreviewSummary>(PreviewSummary.Parse(Json));

        Assert.Equal(86016, summary.WouldSendBytes);
        Assert.Equal(71234, summary.RawSessionBytes);
        Assert.Equal(148, summary.EventCount);
        Assert.Equal(47, summary.TotalRedactions);
        Assert.True(summary.Enrolled);
        Assert.Equal(2, summary.ConsentScopes.Count);
    }

    [Fact]
    public void TheReceiptIsLargestCategoryFirst()
    {
        PreviewSummary summary = Assert.IsType<PreviewSummary>(PreviewSummary.Parse(Json));

        Assert.Equal("scrubbed: 31 path, 12 secret, 4 token", summary.RedactionReceipt);
    }

    [Fact]
    public void NothingMatchedIsSaidRatherThanShownAsZero()
    {
        // A session where no pattern fired is the session most worth a second
        // look, and "scrubbed: 0" reads as a reassurance.
        PreviewSummary summary = Assert.IsType<PreviewSummary>(
            PreviewSummary.Parse("""{"redactions":{}}"""));

        Assert.Equal("scrubbed: nothing matched", summary.RedactionReceipt);
        Assert.Equal("nothing matched", summary.ScrubbingFound);
    }

    [Theory]
    [InlineData("")]
    [InlineData("   ")]
    [InlineData("not json at all")]
    [InlineData(null)]
    public void AnUnreadableSummaryIsNullRatherThanAThrow(string? json)
    {
        // Null reaches the gate as "no pinned preview", which is the only
        // safe reading of a summary the app cannot understand.
        Assert.Null(PreviewSummary.Parse(json));
    }

    [Fact]
    public void AMissingEnrolledFlagDefaultsToNotEnrolled()
    {
        // Fail closed. An older daemon that does not send the flag must not
        // be read as having pinned an envelope.
        PreviewSummary summary = Assert.IsType<PreviewSummary>(
            PreviewSummary.Parse("""{"would_send_bytes": 10}"""));

        Assert.False(summary.Enrolled);
    }
}

public sealed class TranscriptMarkerTests
{
    [Fact]
    public void MarkersAreSplitOutAndNothingIsDropped()
    {
        const string text = "before <PRIVATE_SECRET_1> middle [REDACTED:aws_key] after";

        IReadOnlyList<TranscriptRun> runs = TranscriptMarkers.Split(text);

        Assert.Equal(5, runs.Count);
        Assert.Equal(new[] { false, true, false, true, false }, Selected(runs));
        Assert.Equal(text, Rebuild(text, runs));
    }

    [Fact]
    public void AMarkerAtEitherEndStillCoversTheWholeBody()
    {
        const string text = "<PRIVATE_TOKEN_2>tail";

        IReadOnlyList<TranscriptRun> runs = TranscriptMarkers.Split(text);

        Assert.Equal(text, Rebuild(text, runs));
        Assert.True(runs[0].IsMarker);
    }

    [Fact]
    public void PlainTextIsOneRun()
    {
        IReadOnlyList<TranscriptRun> runs = TranscriptMarkers.Split("nothing to see");

        Assert.Single(runs);
        Assert.False(runs[0].IsMarker);
    }

    [Fact]
    public void AnEmptyTranscriptHasNoRuns()
    {
        Assert.Empty(TranscriptMarkers.Split(string.Empty));
    }

    /// <summary>
    /// The property that actually matters: the runs reproduce the transcript
    /// exactly. A dropped byte would show the contributor less than the
    /// approval covers.
    /// </summary>
    private static string Rebuild(string text, IReadOnlyList<TranscriptRun> runs)
    {
        var rebuilt = new System.Text.StringBuilder();
        foreach (TranscriptRun run in runs)
        {
            rebuilt.Append(text.AsSpan(run.Start, run.Length));
        }

        return rebuilt.ToString();
    }

    private static bool[] Selected(IReadOnlyList<TranscriptRun> runs)
    {
        var flags = new bool[runs.Count];
        for (int i = 0; i < runs.Count; i++)
        {
            flags[i] = runs[i].IsMarker;
        }

        return flags;
    }
}

public sealed class SearchContextTests
{
    [Fact]
    public void AnExcerptCarriesTheTermAndItsSurroundings()
    {
        string body = new string('a', 300) + "acme-corp" + new string('b', 300);

        IReadOnlyList<string> excerpts = SearchContexts.Build(body, "acme-corp", new[] { 300 });

        string only = Assert.Single(excerpts);
        Assert.Contains("acme-corp", only, StringComparison.Ordinal);
        Assert.StartsWith("…", only, StringComparison.Ordinal);
        Assert.EndsWith("…", only, StringComparison.Ordinal);
    }

    [Fact]
    public void ExcerptsAreCappedButTheCountShownIsNot()
    {
        // The cap is on what is laid out, never on what is reported: the
        // match count the contributor reads is always the full one.
        string body = string.Concat(System.Linq.Enumerable.Repeat("x", 5000));
        var offsets = new List<int>();
        for (int i = 0; i < 100; i++)
        {
            offsets.Add(i * 10);
        }

        IReadOnlyList<string> excerpts = SearchContexts.Build(body, "x", offsets);

        Assert.Equal(SearchContexts.MaxExcerpts, excerpts.Count);
    }

    [Fact]
    public void NewlinesCollapseSoAnExcerptStaysOneLine()
    {
        IReadOnlyList<string> excerpts = SearchContexts.Build("a\nneedle\nb", "needle", new[] { 2 });

        Assert.DoesNotContain('\n', Assert.Single(excerpts));
    }

    [Fact]
    public void AnEmptyNeedleFindsNothing()
    {
        Assert.Empty(SearchContexts.Build("body", string.Empty, new[] { 0 }));
    }

    [Fact]
    public void AnOffsetPastTheEndIsSkippedRatherThanThrowing()
    {
        // TcPreview drops unresolvable offsets, but this must not be the one
        // place a bad number takes the sheet down.
        Assert.Empty(SearchContexts.Build("short", "needle", new[] { 9999, -1 }));
    }

    [Fact]
    public void AWindowBoundaryDoesNotSplitASurrogatePair()
    {
        // Half a surrogate pair renders as a replacement character, in an
        // excerpt whose entire job is showing exactly what is in the trace.
        string body = string.Concat(System.Linq.Enumerable.Repeat("😀", 200)) + "needle";

        IReadOnlyList<string> excerpts = SearchContexts.Build(body, "needle", new[] { 400 });

        string only = Assert.Single(excerpts);
        Assert.DoesNotContain('�', only);
        Assert.False(char.IsLowSurrogate(only[1]));
    }
}

public sealed class ApprovalHoldTests
{
    private static DaemonResponse Response(string result) =>
        DaemonResponse.Parse($$"""{"id":1,"result":{{result}}}""");

    [Fact]
    public void TheDaemonsDeadlineDrivesTheCountdown()
    {
        var now = DateTimeOffset.Parse("2026-08-18T12:00:00Z", System.Globalization.CultureInfo.InvariantCulture);
        ApprovalHold hold = Assert.IsType<ApprovalHold>(ApprovalHold.Parse(
            Response("""{"approved":1,"hold_secs":5,"hold_until":"2026-08-18T12:00:05Z"}""")));

        Assert.Equal(5, hold.RemainingSeconds(now));
        Assert.True(hold.IsLive(now));
        Assert.Equal(0, hold.RemainingSeconds(now.AddSeconds(5)));
        Assert.False(hold.IsLive(now.AddSeconds(5)));
    }

    [Fact]
    public void NoHoldMeansNoUndoIsOffered()
    {
        // Null is a real answer: nothing was approved, or the hold is off.
        // Inventing a deadline would offer an undo that cannot work.
        ApprovalHold hold = Assert.IsType<ApprovalHold>(ApprovalHold.Parse(
            Response("""{"approved":1,"hold_secs":0,"hold_until":null}""")));

        Assert.Null(hold.Deadline);
        Assert.False(hold.IsLive(DateTimeOffset.UtcNow));
    }

    [Fact]
    public void AnErrorFrameYieldsNoHold()
    {
        DaemonResponse response = DaemonResponse.Parse(
            """{"id":1,"error":{"code":"bad_params","message":"unknown-entry"}}""");

        Assert.Null(ApprovalHold.Parse(response));
    }
}
