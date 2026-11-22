using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>The words the shell says when the clipboard or the background service fails.</summary>
public sealed class ShellFailureCopyTests
{
    [Fact]
    public void BothLinesParse()
    {
        ShellFailureWords? words = ShellFailureCopy.Parse("""{"clipboard_failed":"a","daemon_start_failed":"b"}""");
        Assert.NotNull(words);
        Assert.Equal("a", words!.ClipboardFailed);
        Assert.Equal("b", words.DaemonStartFailed);
    }

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("  ")]
    [InlineData("not json")]
    [InlineData("{}")]
    [InlineData("""{"clipboard_failed":"a"}""")]
    [InlineData("""{"clipboard_failed":"","daemon_start_failed":"b"}""")]
    [InlineData("null")]
    public void AnythingLessThanBothLinesIsNothing(string? json)
    {
        Assert.Null(ShellFailureCopy.Parse(json));
    }

    /// <summary>The real core's answer, through the real ABI: the approved wording.</summary>
    [Fact]
    public void TheCoresWordsAreTheApprovedOnes()
    {
        Assert.Equal("Couldn't copy. Try again.", ShellFailureCopy.ClipboardFailed);
        Assert.Contains("couldn't start its background service", ShellFailureCopy.DaemonStartFailed);
        Assert.EndsWith("Restart the app; if it keeps happening, reinstall it.", ShellFailureCopy.DaemonStartFailed);
    }
}
