using System;
using System.Collections.Generic;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// What arms Contribute, and what the sheet says while it does, in the one
/// place either can be tested off Windows.
///
/// Everything the preview sheet decides -- whether Contribute may be pressed,
/// what the summary says, where the redaction chips go, what an excerpt shows,
/// how long an undo lasts -- lives in the interop assembly precisely so these
/// exist. The XAML that draws them cannot be built on this machine; the rules
/// it obeys can be.
///
/// These tests used to encode a two-step read gate: transcript shown, then an
/// acknowledgement ticked, in that order. Both steps were removed as friction
/// (see <see cref="ReadGate"/> for why), so the tests that asserted them are
/// gone and the ones below assert the rule that replaced them, including the
/// one that is easiest to break by accident -- a loaded preview arms
/// Contribute on its own.
/// </summary>
public sealed class ReadGateTests
{
    [Fact]
    public void NothingIsArmedToBeginWith()
    {
        var gate = new ReadGate();

        Assert.False(gate.CanContribute);
        Assert.False(gate.HasPinnedPreview);
    }

    [Fact]
    public void APinnedPreviewIsTheOnlyThingContributeWaitsOn()
    {
        // The friction change, stated as a value: no transcript tab, no
        // acknowledgement, no second step.
        var gate = new ReadGate();
        gate.SetPinnedPreview(true);

        Assert.True(gate.CanContribute);
        Assert.Equal(ReadGate.ReadyHelp, gate.Help);
    }

    [Fact]
    public void AnUnenrolledPreviewCanNeverBeContributed()
    {
        // Nothing was pinned, so there is no envelope for an approval to bind
        // to. This is the one condition that was never friction, and it is
        // the one that stayed.
        var gate = new ReadGate();

        Assert.False(gate.CanContribute);
        Assert.Equal(ReadGate.UnenrolledHelp, gate.Help);
    }

    [Fact]
    public void ResetUnpinsSoTheNextSessionStartsFromZero()
    {
        // The whole point: an approval covers the bytes THIS sheet pinned. A
        // pin carried into the next session would let it be approved against
        // the previous one's envelope.
        var gate = new ReadGate();
        gate.SetPinnedPreview(true);

        gate.Reset();

        Assert.False(gate.CanContribute);
        Assert.False(gate.HasPinnedPreview);
    }

    [Fact]
    public void EveryTransitionRaisesChanged()
    {
        // The sheet re-evaluates Contribute from this event alone, so a
        // silent transition would leave the button disagreeing with the gate.
        var gate = new ReadGate();
        int raised = 0;
        gate.Changed += () => raised++;

        gate.SetPinnedPreview(true);
        gate.Reset();

        Assert.Equal(2, raised);
    }

    [Fact]
    public void RepeatedTransitionsDoNotRaiseChanged()
    {
        var gate = new ReadGate();
        gate.SetPinnedPreview(true);
        int raised = 0;
        gate.Changed += () => raised++;

        gate.SetPinnedPreview(true);
        gate.Reset();
        gate.Reset();

        Assert.Equal(1, raised);
    }
}

/// <summary>
/// The sentence that replaced the acknowledgement checkbox.
///
/// The same assertions the Linux shell holds in
/// <c>crates/trace-commons-contributor-gtk/src/copy.rs</c> and the macOS shell
/// in <c>macos/Tests/TCShellCoreTests/ReadGateTests.swift</c>. Three shells
/// print this above one irreversible button; the only thing that holds three
/// languages to one sentence is the same text asserted in each of them, plus
/// the Rust test that reads all three sources.
/// </summary>
public sealed class ReadGateCopyTests
{
    private const string Statement =
        "\"Exactly what would be sent\" is the exact text that would leave this machine. "
        + "Pattern-based scrubbing may have missed something in it, and nothing here checks "
        + "that you looked.";

    [Fact]
    public void TheConsentStatementIsExactlyWhatWasAgreed()
    {
        Assert.Equal(Statement, ReadGate.Statement);
    }

    [Fact]
    public void TheStatementKeepsBothHalvesOfWhatTheCheckboxUsedToSay()
    {
        // The acknowledgement made a contributor assert these two things by
        // hand. Neither may quietly drop out now that nobody is asked to tick
        // anything.
        Assert.Contains("Pattern-based scrubbing may have missed something", ReadGate.Statement);
        Assert.Contains("nothing here checks that you looked", ReadGate.Statement);
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

    /// <summary>
    /// An unrecognised <c>project_id</c> is refused the same way as an
    /// unrecognised <c>entry_id</c>: a <c>bad_params</c> error frame, with no
    /// result to build a toast from. A caller must show that as a refusal,
    /// not as a toast claiming zero sessions were sent.
    /// </summary>
    [Fact]
    public void AnUnrecognisedProjectIsRefusedNotSkipped()
    {
        DaemonResponse response = DaemonResponse.Parse(
            """{"id":1,"error":{"code":"bad_params","message":"unknown-project"}}""");

        Assert.True(response.IsError);
        Assert.Null(ApprovalHold.Parse(response));
    }

    /// <summary>
    /// Two shapes that must never be confused: a project_id naming no
    /// project the daemon knows at all (bad_params / project_id_unrecognized,
    /// an error frame -- ipc.rs:1241) versus one naming a real project that
    /// simply has nothing pending right now (an ordinary success,
    /// approved: 0). The first is a client bug -- a typo'd or stale id --
    /// and must read as a refusal. The second is a race a client can win
    /// completely honestly (a sweep or another device cleared the project
    /// first) and must read as the ordinary "nothing sent" toast, not as an
    /// error.
    /// </summary>
    [Fact]
    public void AnUnrecognisedProjectIsDistinctFromAKnownEmptyOne()
    {
        DaemonResponse refused = DaemonResponse.Parse(
            """{"id":1,"error":{"code":"bad_params","message":"project_id_unrecognized"}}""");
        DaemonResponse knownButEmpty = Response(
            """{"approved":0,"flagged":0,"redactions":{},"skipped":[],"hold_secs":0,"hold_until":null}""");

        Assert.Null(ApprovalHold.Parse(refused));

        ApprovalHold hold = Assert.IsType<ApprovalHold>(ApprovalHold.Parse(knownButEmpty));
        Assert.Equal(0UL, hold.Approved);
        Assert.Equal(SubmitToast.Render(0, 0, 0, new List<string>()).Line, hold.Toast.Line);
    }

    /// <summary>
    /// The full shape a one-click submit reports, decoded whole: counts,
    /// redaction categories, and the skipped list -- everything
    /// <see cref="ApprovalHold.Toast"/> and <see cref="ApprovalHold.ApprovedEntryIds"/>
    /// are built from.
    /// </summary>
    [Fact]
    public void TheFullSubmitResponseDecodes()
    {
        ApprovalHold hold = Assert.IsType<ApprovalHold>(ApprovalHold.Parse(Response(
            """
            {
                "approved": 44,
                "flagged": 3,
                "redactions": {"aws_key": 200, "email": 13},
                "skipped": [
                    {"entry_id": "e1", "reason_label": "envelope-too-large"},
                    {"entry_id": "e2", "reason_label": "envelope-too-large"},
                    {"entry_id": "e3", "reason_label": "envelope-too-large"}
                ],
                "hold_secs": 5,
                "hold_until": "2026-08-20T12:00:05Z"
            }
            """)));

        Assert.Equal(44UL, hold.Approved);
        Assert.Equal(3UL, hold.Flagged);
        Assert.Equal(213UL, hold.RedactionsTotal);
        Assert.Equal(3, hold.Skipped.Count);
        Assert.Equal("e1", hold.Skipped[0].EntryId);
        Assert.Equal("envelope-too-large", hold.Skipped[0].ReasonLabel);
        // Asserted against the renderer's own output, not a literal English
        // sentence: SubmitToast.cs is the shared normative-copy contract and
        // is not this file's to pin a snapshot of -- only that ApprovalHold
        // feeds it the right numbers and labels.
        Assert.Equal(
            SubmitToast.Render(44, 213, 3, new[] { "envelope-too-large", "envelope-too-large", "envelope-too-large" }).Line,
            hold.Toast.Line);
        Assert.True(hold.Toast.OfferUndo);
    }

    /// <summary>
    /// The response never lists which entries it approved, only which it
    /// skipped -- so a project-group submit's Undo has to recover that set by
    /// subtracting the skipped ids from the ones it offered.
    /// </summary>
    [Fact]
    public void ApprovedEntryIdsIsTheCandidatesMinusTheSkipped()
    {
        ApprovalHold hold = Assert.IsType<ApprovalHold>(ApprovalHold.Parse(Response(
            """
            {
                "approved": 2,
                "flagged": 0,
                "redactions": {},
                "skipped": [{"entry_id": "b", "reason_label": "not-pending"}],
                "hold_secs": 5,
                "hold_until": "2026-08-20T12:00:05Z"
            }
            """)));

        IReadOnlyList<string> approvedIds =
            hold.ApprovedEntryIds(new[] { "a", "b", "c" });

        Assert.Equal(new[] { "a", "c" }, approvedIds);
    }

    /// <summary>
    /// A zero-approval response offers no undo, whatever the hold fields say --
    /// the defect Task 1's renderer exists to fix, exercised here on the real
    /// decoded type rather than on bare counts.
    /// </summary>
    [Fact]
    public void ZeroApprovedOffersNoUndoEvenWithSkips()
    {
        ApprovalHold hold = Assert.IsType<ApprovalHold>(ApprovalHold.Parse(Response(
            """
            {
                "approved": 0,
                "flagged": 0,
                "redactions": {},
                "skipped": [
                    {"entry_id": "a", "reason_label": "not-pending"},
                    {"entry_id": "b", "reason_label": "not-pending"}
                ],
                "hold_secs": 0,
                "hold_until": null
            }
            """)));

        Assert.False(hold.Toast.OfferUndo);
        Assert.Equal(
            SubmitToast.Render(0, 0, 0, new[] { "not-pending", "not-pending" }).Line,
            hold.Toast.Line);
    }
}
