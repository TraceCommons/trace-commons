using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The bucket's shared words and the mode it must never be offered.
///
/// The mode rule is the serious half of this defect: the daemon refuses
/// <c>auto_upload</c> for this row in two independent places, so a shell
/// offering it would have a contributor believe they had armed something that
/// cannot be armed, and the refusal would be silent. It is asserted here rather
/// than in a view because a view is the one place in this codebase that nothing
/// can execute.
/// </summary>
public class UnresolvedBucketCopyTests
{
    [Fact]
    public void TheBucketMayNotBeOfferedAutoUpload()
    {
        Assert.False(UnresolvedBucketCopy.MayOfferAutoUpload(true));
        Assert.DoesNotContain("auto_upload", UnresolvedBucketCopy.OfferableModes(true));
    }

    /// <summary>
    /// Silencing is not arming. Removing <c>ignore</c> along with
    /// <c>auto_upload</c> would take away the one action a contributor can
    /// still take on this row.
    /// </summary>
    [Fact]
    public void TheBucketMayStillBeSilencedOrAsked()
    {
        string[] modes = UnresolvedBucketCopy.OfferableModes(true);

        Assert.Contains("ignore", modes);
        Assert.Contains("ask", modes);
    }

    [Fact]
    public void AnOrdinaryProjectKeepsEveryMode()
    {
        string[] modes = UnresolvedBucketCopy.OfferableModes(false);

        Assert.True(UnresolvedBucketCopy.MayOfferAutoUpload(false));
        Assert.Equal(new[] { "ask", "auto_upload", "ignore" }, modes);
    }

    /// <summary>
    /// Onboarding screen 5 and Settings state one fact on two surfaces, so they
    /// must state it in one set of words. Two copies is how one surface gets
    /// quietly reworded later.
    /// </summary>
    [Fact]
    public void OnboardingAndSettingsShareTheSameWords()
    {
        Assert.Same(UnresolvedBucketCopy.Label, WatchCopy.UnknownLabel);
        Assert.Same(UnresolvedBucketCopy.Note, WatchCopy.UnknownNote);
    }

    /// <summary>
    /// The note is a statement of what the daemon does, not an apology, and
    /// nothing in it is a contributor's to fix.
    /// </summary>
    [Fact]
    public void TheNoteSaysWhatHappensRatherThanAskingForAFix()
    {
        Assert.Contains("You'll always be asked", UnresolvedBucketCopy.Note, System.StringComparison.Ordinal);
        Assert.DoesNotContain("error", UnresolvedBucketCopy.Note, System.StringComparison.OrdinalIgnoreCase);
        Assert.DoesNotContain("failed", UnresolvedBucketCopy.Note, System.StringComparison.OrdinalIgnoreCase);
    }
}
