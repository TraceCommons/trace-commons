using System;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The three values the daemon accepts for <c>outcome</c>, and the
/// distinction between "the contributor did not answer" and "the contributor
/// answered something unrecognised" -- which are not the same event: the
/// first approves normally, the second approves nothing.
/// </summary>
public sealed class VerdictTests
{
    [Fact]
    public void TheWireValuesAreTheThreeTheDaemonAccepts()
    {
        Assert.Equal("worked", Verdict.Worked);
        Assert.Equal("partly", Verdict.Partly);
        Assert.Equal("failed", Verdict.Failed);
    }

    [Fact]
    public void OnlyThoseThreeAreRecognised()
    {
        Assert.True(Verdict.IsRecognised(Verdict.Worked));
        Assert.True(Verdict.IsRecognised(Verdict.Partly));
        Assert.True(Verdict.IsRecognised(Verdict.Failed));

        Assert.False(Verdict.IsRecognised(null));
        Assert.False(Verdict.IsRecognised(string.Empty));
        Assert.False(Verdict.IsRecognised("   "));
        Assert.False(Verdict.IsRecognised("Worked"));
        Assert.False(Verdict.IsRecognised("succeeded"));
    }

    /// <summary>
    /// Absence is null and nothing else. An empty string is a value, and a
    /// value the daemon refuses.
    /// </summary>
    [Fact]
    public void OnlyNullMeansNoAnswer()
    {
        Assert.True(Verdict.IsAbsent(null));
        Assert.False(Verdict.IsAbsent(string.Empty));
        Assert.False(Verdict.IsAbsent(Verdict.Worked));
    }

    [Fact]
    public void RequireReturnsARecognisedValueAndRejectsEverythingElse()
    {
        Assert.Equal("worked", Verdict.Require(Verdict.Worked));
        Assert.Throws<ArgumentException>(() => Verdict.Require(string.Empty));
        Assert.Throws<ArgumentException>(() => Verdict.Require("partially"));
    }

    /// <summary>
    /// The strings a contributor reads, word for word as the Linux and macOS
    /// shells print them. The caption is the disclosure that the outcome
    /// fields sit outside the "exactly what would be sent" guarantee, and it
    /// is asserted in full here so that shortening it in the XAML cannot pass
    /// unnoticed.
    /// </summary>
    [Fact]
    public void TheCopyMatchesTheOtherShellsWordForWord()
    {
        Assert.Equal("Did this session do what you asked?", VerdictCopy.Question);
        Assert.Equal("Worked", VerdictCopy.Worked);
        Assert.Equal("Partly", VerdictCopy.Partly);
        Assert.Equal("Failed", VerdictCopy.Failed);
        Assert.Equal(
            "Optional. This is recorded as the trace outcome; the preview above does not show it.",
            VerdictCopy.Caption);
        Assert.Equal("Submit all as...", VerdictCopy.SubmitAllAs);
        Assert.Equal(
            "Record the same outcome for every session in this group.",
            VerdictCopy.SubmitAllAsTooltip);
    }
}
