using System.Collections.Generic;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Parsing of the scrubber's detector-name payload.
///
/// The native call itself is not exercised here: <c>tc_scrub_detector_names</c>
/// does not exist in the cdylib on this branch, so <see cref="ScrubDetectors.Names"/>
/// throws <c>EntryPointNotFoundException</c> until that export lands. What is
/// tested is everything downstream of the boundary, which is where a defect
/// would actually be.
/// </summary>
public class ScrubDetectorsTests
{
    [Fact]
    public void AWellFormedPayloadKeepsItsOrder()
    {
        IReadOnlyList<string> names =
            ScrubDetectors.ParseNames("[\"github_token\",\"jwt\",\"npm_token\"]");

        Assert.Equal(new[] { "github_token", "jwt", "npm_token" }, names);
    }

    /// <summary>
    /// The dialog is reference material opened during onboarding. A transient
    /// fault should leave a contributor reading the concession over an empty
    /// list, not staring at a crash on the first screen.
    /// </summary>
    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("   ")]
    [InlineData("not json")]
    [InlineData("{\"unexpected\":\"shape\"}")]
    public void AnUnusablePayloadIsEmptyRatherThanFatal(string? payload)
    {
        Assert.Empty(ScrubDetectors.ParseNames(payload));
    }

    /// <summary>
    /// A blank entry would render as an empty bullet, which reads as a detector
    /// nobody bothered to name.
    /// </summary>
    [Fact]
    public void BlankEntriesAreDropped()
    {
        IReadOnlyList<string> names = ScrubDetectors.ParseNames("[\"jwt\",\"\",\"  \"]");

        Assert.Equal(new[] { "jwt" }, names);
    }
}
