using System;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The correction control's contract: the copy three shells share, and the
/// omit-versus-empty rule the request has to honour.
///
/// Asserting a literal against a literal looks circular and is not. The
/// published policy page says redaction happens locally and is re-applied on
/// the server; a correction is the one exception and the page does not yet
/// say so. Until it does, the caption is the entire disclosure a contributor
/// gets that their own words are stored verbatim, so a shell that shortens it
/// for layout is shipping the exception undisclosed. A test on the Rust side
/// reads <c>CorrectionCopy.cs</c> itself, which is what holds all three
/// shells to one wording.
/// </summary>
public sealed class CorrectionTests
{
    private const string EntryId = "entry-123";

    [Fact]
    public void ThePromptAndPlaceholderAreTheSharedWording()
    {
        Assert.Equal("What did it get wrong?", CorrectionCopy.Question);
        Assert.Equal("Optional", CorrectionCopy.Placeholder);
    }

    [Fact]
    public void TheDisclosureCaptionIsIntact()
    {
        Assert.Equal(
            "Stored exactly as you write it. Unlike the rest of the trace, a correction is not scrubbed here or on the server -- so leave out anything you would not want in the corpus: someone else's personal information, employer-confidential material, or anything you are not free to share.",
            CorrectionCopy.Caption);
        // The halves that must never quietly drop out: what is different
        // about a correction, and what not to put in one.
        Assert.Contains("Stored exactly as you write it", CorrectionCopy.Caption, StringComparison.Ordinal);
        Assert.Contains("not scrubbed here or on the server", CorrectionCopy.Caption, StringComparison.Ordinal);
        Assert.Contains("personal information", CorrectionCopy.Caption, StringComparison.Ordinal);
        Assert.Contains("employer-confidential", CorrectionCopy.Caption, StringComparison.Ordinal);
        Assert.Contains("not free to share", CorrectionCopy.Caption, StringComparison.Ordinal);
    }

    /// <summary>
    /// The refusal says both things it has to: nothing was sent, and the
    /// credential has to be rotated because it has already been typed.
    /// </summary>
    [Fact]
    public void TheCredentialRefusalSaysNothingWasSentAndToRotate()
    {
        Assert.Equal(
            "Nothing was sent. Your correction looks like it contains a credential.",
            CorrectionCopy.CredentialHeadline);
        Assert.Contains("rotate it", CorrectionCopy.CredentialBody, StringComparison.Ordinal);
        Assert.Contains("already been typed", CorrectionCopy.CredentialBody, StringComparison.Ordinal);
    }

    /// <summary>
    /// The refusal must never render as the generic "could not be prepared":
    /// that would be a false account of something the contributor caused and
    /// can fix.
    /// </summary>
    [Fact]
    public void TheRefusalHasItsOwnSkipLabel()
    {
        Assert.Contains(
            SubmitToast.SkipReasons,
            pair => pair.Wire == CorrectionCopy.CredentialRefusalLabel);
        Assert.NotEqual(
            SubmitToast.SkipReasonUnknown,
            SubmitToast.ReasonLabel(CorrectionCopy.CredentialRefusalLabel));
    }

    [Fact]
    public void ACorrectionRidesAlongWithTheVerdictItWasWrittenUnder()
    {
        string json = SubmitParams.ForEntry(EntryId, Verdict.Failed, "it edited the wrong config");

        using JsonDocument doc = JsonDocument.Parse(json);
        Assert.Equal("failed", doc.RootElement.GetProperty("outcome").GetString());
        Assert.Equal(
            "it edited the wrong config",
            doc.RootElement.GetProperty("correction").GetString());
    }

    /// <summary>
    /// An untouched box and a box holding only whitespace are the same
    /// thing: no correction. The assertion is on the KEY -- an empty string
    /// would declare <c>correction_included</c> on the envelope for content
    /// that is not there.
    /// </summary>
    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("   ")]
    [InlineData("\n\t ")]
    public void ABlankCorrectionOmitsTheParameter(string? blank)
    {
        string json = SubmitParams.ForEntry(EntryId, Verdict.Failed, blank);

        using JsonDocument doc = JsonDocument.Parse(json);
        Assert.False(doc.RootElement.TryGetProperty("correction", out _));
        Assert.DoesNotContain("correction", json, StringComparison.Ordinal);
    }

    [Fact]
    public void ACorrectionIsSentTrimmed()
    {
        string json = SubmitParams.ForEntry(EntryId, Verdict.Partly, "  it stopped halfway  ");

        using JsonDocument doc = JsonDocument.Parse(json);
        Assert.Equal("it stopped halfway", doc.RootElement.GetProperty("correction").GetString());
    }

    /// <summary>
    /// The verdict gate, refused here rather than over the pipe. The sheet
    /// does not offer the field outside partly and failed, so reaching this
    /// is a bug -- and a test failure is a better place to find it than a
    /// submit that silently did not happen.
    /// </summary>
    [Theory]
    [InlineData(null)]
    [InlineData(Verdict.Worked)]
    public void ACorrectionWithoutAPartlyOrFailedVerdictIsRefused(string? outcome)
    {
        Assert.Throws<ArgumentException>(
            () => SubmitParams.ForEntry(EntryId, outcome, "it did the wrong thing"));
    }

    [Fact]
    public void AnOversizedCorrectionIsRefused()
    {
        string tooLong = new('x', CorrectionCopy.MaxCharacters + 1);

        Assert.Throws<ArgumentException>(
            () => SubmitParams.ForEntry(EntryId, Verdict.Failed, tooLong));
    }

    [Fact]
    public void ACorrectionAtTheCapIsAccepted()
    {
        string atCap = new('x', CorrectionCopy.MaxCharacters);
        string json = SubmitParams.ForEntry(EntryId, Verdict.Failed, atCap);

        using JsonDocument doc = JsonDocument.Parse(json);
        Assert.Equal(atCap, doc.RootElement.GetProperty("correction").GetString());
    }

    /// <summary>
    /// A project call has no way to express a correction at all: one written
    /// for a group would describe sessions it was not written about, and
    /// every one of them would carry it into the corpus as the contributor's
    /// own words.
    /// </summary>
    [Fact]
    public void AProjectCallNeverCarriesACorrection()
    {
        string json = SubmitParams.ForProject("proj_abcdef", Verdict.Failed);

        Assert.DoesNotContain("correction", json, StringComparison.Ordinal);
    }

    /// <summary>The refusal is recognised off the response, and nothing else is.</summary>
    [Fact]
    public void OnlyTheCredentialRefusalIsRecognisedAsOne()
    {
        var refused = new ApprovalHold
        {
            Skipped =
            {
                new SubmitSkip
                {
                    EntryId = EntryId,
                    ReasonLabel = CorrectionCopy.CredentialRefusalLabel,
                },
            },
        };
        Assert.True(refused.WasRefusedForACorrectionCredential);

        var otherSkip = new ApprovalHold
        {
            Skipped =
            {
                new SubmitSkip { EntryId = EntryId, ReasonLabel = "envelope-too-large" },
            },
        };
        Assert.False(otherSkip.WasRefusedForACorrectionCredential);

        Assert.False(new ApprovalHold { Approved = 1 }.WasRefusedForACorrectionCredential);
    }
}
