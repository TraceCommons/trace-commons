using System.Collections.Generic;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The submit toast, checked against the spec's own worked examples.
///
/// These are the same assertions the Linux shell holds in
/// <c>crates/trace-commons-contributor-gtk/src/toast.rs</c> and the macOS
/// shell in <c>macos/Tests/TCShellCoreTests/SubmitToastTests.swift</c>.
/// Three shells render this sentence, the whole point of the
/// "The toast: normative copy" section of
/// <c>docs/superpowers/specs/2026-08-20-one-click-submit-design.md</c> is
/// that they render it identically, and the only thing that actually holds
/// three languages to one sentence is the same examples asserted in each of
/// them.
///
/// Checked here, they are checked on a machine that cannot build WinUI at
/// all -- which is why the renderer lives in the interop assembly.
/// </summary>
public sealed class SubmitToastTests
{
    [Fact]
    public void TheSpecWorkedExamplesRenderExactly()
    {
        Assert.Equal(
            "Approved. Scrubbing removed 4. 1 flagged.",
            SubmitToast.Render(1, 4, 1, new List<string>()).Line);
        Assert.Equal(
            "Approved 47. Scrubbing removed 213. 3 flagged.",
            SubmitToast.Render(47, 213, 3, new List<string>()).Line);
        Assert.Equal(
            "Approved 44. Scrubbing removed 213. 3 flagged, 3 not approved: too large to send.",
            SubmitToast.Render(
                44, 213, 3,
                new List<string> { "envelope-too-large", "envelope-too-large", "envelope-too-large" })
                .Line);
        Assert.Equal(
            "Nothing approved. Scrubbing matched nothing. 2 not approved: already decided.",
            SubmitToast.Render(0, 0, 0, new List<string> { "not-pending", "not-pending" }).Line);
    }

    [Fact]
    public void UndoIsOfferedOnlyWhenSomethingWasApproved()
    {
        Assert.True(SubmitToast.Render(1, 0, 0, new List<string>()).OfferUndo);
        Assert.False(SubmitToast.Render(0, 0, 0, new List<string> { "not-pending" }).OfferUndo);
    }

    [Fact]
    public void AWireLabelNeverReachesTheContributor()
    {
        var line = SubmitToast.Render(0, 0, 0, new List<string> { "envelope-too-large" }).Line;
        Assert.DoesNotContain("envelope-too-large", line);
        Assert.Contains("too large to send", line);
    }

    /// <summary>
    /// Every wire label the daemon can send, and one it cannot.
    ///
    /// The unknown case is the one that matters: a label this shell has not
    /// been taught is still a label, and echoing it would put daemon
    /// vocabulary in front of a contributor at exactly the moment the shell
    /// is least sure what happened.
    /// </summary>
    [Fact]
    public void EveryWireLabelMapsToAHumanOne()
    {
        var wireLabels = new[]
        {
            "not-enrolled",
            "not-pending",
            "not-pinned",
            "envelope-too-large",
            "session-file-vanished",
            "preview-failed",
            "some-label-this-shell-has-never-seen",
        };

        foreach (var wire in wireLabels)
        {
            var line = SubmitToast.Render(0, 0, 0, new List<string> { wire }).Line;
            Assert.DoesNotContain(wire, line);
        }
    }

    /// <summary>
    /// The clause is a count of entries and a list of distinct reasons, and
    /// those are two different numbers whenever entries share a reason.
    /// </summary>
    [Fact]
    public void DistinctReasonsAreListedOnceInTheSpecsOrder()
    {
        Assert.Equal(
            "Nothing approved. Scrubbing matched nothing. 4 not approved: not connected to a commons, "
                + "already decided, could not be read.",
            SubmitToast.Render(
                0, 0, 0,
                new List<string> { "preview-failed", "not-pending", "not-enrolled", "not-pending" })
                .Line);
    }

    /// <summary>
    /// The unrecognised-label fallback happens to share its rendered text
    /// with <c>not-pinned</c>'s own label ("could not be prepared"). A batch
    /// that skips entries for both reasons must still print that text only
    /// once, and must still count every skipped entry.
    /// </summary>
    [Fact]
    public void TheUnknownFallbackDoesNotDuplicateAcollidingLabel()
    {
        Assert.Equal(
            "Nothing approved. Scrubbing matched nothing. 2 not approved: could not be prepared.",
            SubmitToast.Render(
                0, 0, 0,
                new List<string> { "not-pinned", "some-label-this-shell-has-never-seen" })
                .Line);
    }

    /// <summary>
    /// Clauses 3 and 4 appear only when non-zero; clauses 1 and 2 always do.
    /// </summary>
    [Fact]
    public void TheOptionalClausesAreAbsentWhenEmpty()
    {
        Assert.Equal(
            "Approved. Scrubbing matched nothing.",
            SubmitToast.Render(1, 0, 0, new List<string>()).Line);
        Assert.Equal(
            "Approved 2. Scrubbing removed 1.",
            SubmitToast.Render(2, 1, 0, new List<string>()).Line);
    }

    /// <summary>
    /// A zero redaction count is a fact the contributor is owed, not an
    /// absence to omit -- a session that obviously touched a <c>.env</c> and
    /// reports nothing removed is a signal.
    /// </summary>
    [Fact]
    public void ScrubbingReportsZeroRatherThanSayingNothing()
    {
        Assert.Contains(
            "Scrubbing matched nothing",
            SubmitToast.Render(1, 0, 0, new List<string>()).Line);
    }

    /// <summary>
    /// Mutation guard: dropping the flagged half of the joined clause leaves
    /// compiling, wrong code. See the follow-up report for the mutation
    /// applied and reverted while verifying this test.
    /// </summary>
    [Fact]
    public void TheJoinedClauseCarriesBothHalvesWhenBothApply()
    {
        Assert.Equal(
            "Approved 44. Scrubbing removed 213. 3 flagged, 1 not approved: too large to send.",
            SubmitToast.Render(44, 213, 3, new List<string> { "envelope-too-large" }).Line);
    }
}
