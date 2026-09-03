using System;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The caveat has to be identical everywhere it appears, and the
/// nothing-matched line has to say what to do next.
/// </summary>
public class ScrubbingCaveatCopyTests
{
    [Fact]
    public void TheNothingMatchedLineOffersANextStep()
        => Assert.Contains(
            "search",
            ScrubbingCaveatCopy.RowLine(0),
            StringComparison.OrdinalIgnoreCase);

    /// <summary>
    /// The tone is unchanged: a session where no pattern fired is still the
    /// one worth slowing down on, and the line still says so. Only the next
    /// step was added.
    /// </summary>
    [Fact]
    public void TheNothingMatchedLineStillAsksForASecondLook()
        => Assert.Contains(
            "second look",
            ScrubbingCaveatCopy.RowLine(0),
            StringComparison.Ordinal);

    [Fact]
    public void ASessionThatRemovedSomethingGetsThePlainCaveat()
    {
        Assert.Equal(ScrubbingCaveatCopy.Sentence, ScrubbingCaveatCopy.RowLine(1));
        Assert.Equal(ScrubbingCaveatCopy.Sentence, ScrubbingCaveatCopy.RowLine(185));
    }

    /// <summary>
    /// The caveat is a statement about the mechanism, so it never claims the
    /// scrubbing was complete.
    /// </summary>
    [Fact]
    public void TheCaveatSaysWhatScrubbingCannotDo()
        => Assert.Contains("misses things", ScrubbingCaveatCopy.Sentence, StringComparison.Ordinal);
}
