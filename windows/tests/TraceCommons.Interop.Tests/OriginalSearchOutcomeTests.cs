using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Searching the redacted body for a value that was removed returns zero
/// matches, which is indistinguishable from the value never having been there.
/// Those are the two answers a worried contributor most needs to tell apart,
/// and this is the type that tells them apart.
/// </summary>
public class OriginalSearchOutcomeTests
{
    [Fact]
    public void AValueThatWasNeverThereIsAbsent()
        => Assert.Equal(OriginalSearchOutcome.Absent, OriginalSearchOutcome.Classify(0, 0));

    [Fact]
    public void AValueScrubbingTookOutIsReportedAsRemoved()
        => Assert.Equal(
            new OriginalSearchOutcome.AllRemoved(3),
            OriginalSearchOutcome.Classify(0, 3));

    [Fact]
    public void AValueStillInTheBodyReportsBothFigures()
        => Assert.Equal(
            new OriginalSearchOutcome.SomeRemain(2, 5),
            OriginalSearchOutcome.Classify(2, 5));

    /// <summary>
    /// Reporting "not in this session" because a call failed would be the
    /// single most dangerous wrong answer this tab can give.
    /// </summary>
    [Fact]
    public void AFailedOriginalSearchIsUnknownNotAbsent()
    {
        Assert.Equal(OriginalSearchOutcome.Unknown, OriginalSearchOutcome.Classify(0, null));
        Assert.Equal(
            new OriginalSearchOutcome.SomeRemain(2, 2),
            OriginalSearchOutcome.Classify(2, null));
    }

    /// <summary>
    /// Impossible from a correct daemon. Falling back to what is certain
    /// beats reporting a negative number of removals.
    /// </summary>
    [Fact]
    public void AnOriginalCountBelowTheRemainingCountFallsBackToWhatIsCertain()
        => Assert.Equal(
            new OriginalSearchOutcome.SomeRemain(2, 2),
            OriginalSearchOutcome.Classify(2, 1));

    /// <summary>
    /// Every case says something a contributor can act on, and the two that
    /// mean "not proven" must never read as "clean".
    /// </summary>
    [Fact]
    public void EachOutcomeHasItsOwnSentence()
    {
        Assert.Contains("Not in this session", OriginalSearchOutcome.Absent.Sentence);
        Assert.Contains("could not check", OriginalSearchOutcome.Unknown.Sentence);
        Assert.Contains("removed", new OriginalSearchOutcome.AllRemoved(3).Sentence);
        Assert.Contains(
            "still in what would be sent",
            new OriginalSearchOutcome.SomeRemain(2, 5).Sentence);

        // The unknown case must not claim absence in any of its words.
        Assert.DoesNotContain("Not in this session", OriginalSearchOutcome.Unknown.Sentence);
    }

    [Fact]
    public void OnlyAValueStillInThePayloadIsAlarming()
    {
        Assert.False(OriginalSearchOutcome.Absent.IsAlarming);
        Assert.False(OriginalSearchOutcome.Unknown.IsAlarming);
        Assert.False(new OriginalSearchOutcome.AllRemoved(3).IsAlarming);
        Assert.True(new OriginalSearchOutcome.SomeRemain(2, 5).IsAlarming);
    }
}
