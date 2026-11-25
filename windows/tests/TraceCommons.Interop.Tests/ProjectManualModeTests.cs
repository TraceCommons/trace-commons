using System;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

public sealed class ProjectManualModeTests
{
    [Theory]
    [InlineData("auto_upload", "ask")]
    [InlineData("ignore", "ask")]
    [InlineData("ask", "ignore")]
    public void ManualActionRestoresReviewOrIgnoresWithoutArming(string current, string expected)
    {
        Assert.Equal(expected, ProjectManualMode.Next(current));
        Assert.NotEqual("auto_upload", ProjectManualMode.Next(current));
    }

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("future-mode")]
    public void UnknownModeHasNoImplicitTransition(string? mode)
    {
        Assert.Null(ProjectManualMode.Next(mode));
    }

    /// <summary>
    /// A refused write is reported. Before this, onboarding's row simply
    /// re-enabled: the daemon's refusal and a click that did nothing looked
    /// identical, and the contributor was left believing a consent field had
    /// changed.
    /// </summary>
    [Fact]
    public void ARefusedWriteIsReported()
    {
        string notice = ProjectManualMode.NoticeFor(writeFailed: true, "ignore", persisted: "ask");

        Assert.NotEqual(string.Empty, notice);
        Assert.Equal(WatchCopy.WriteFailed, notice);
    }

    /// <summary>
    /// The whole point of re-reading. A daemon that answers without an error
    /// but stores something other than what was asked for has not made the
    /// change, and a shell that set its row from the value it sent could never
    /// say so.
    /// </summary>
    [Fact]
    public void AWriteThatDidNotLandIsReportedEvenWithoutAnError()
    {
        string notice = ProjectManualMode.NoticeFor(writeFailed: false, "ask", persisted: "ignore");

        Assert.Equal(WatchCopy.WriteFailed, notice);
    }

    /// <summary>
    /// A row that is gone from the re-read leaves both outcomes unknown, so
    /// neither is claimed: saying "couldn't be changed" there would assert a
    /// failure that was never observed.
    /// </summary>
    [Fact]
    public void AnUnreadableStateIsNotReportedAsAFailedWrite()
    {
        string notice = ProjectManualMode.NoticeFor(writeFailed: false, "ask", persisted: null);

        Assert.Equal(WatchCopy.WriteUnconfirmed, notice);
        Assert.NotEqual(WatchCopy.WriteFailed, notice);
    }

    /// <summary>
    /// A write that failed and cannot be re-read is still a failed write: that
    /// much was observed, and it is the more specific of the two sentences.
    /// </summary>
    [Fact]
    public void AFailedWriteOutranksAnUnreadableState()
    {
        Assert.Equal(
            WatchCopy.WriteFailed,
            ProjectManualMode.NoticeFor(writeFailed: true, "ask", persisted: null));
    }

    /// <summary>
    /// Silent when the daemon stored what was asked for. A line that appears
    /// every time to say nothing happened is how the line that matters gets
    /// skipped.
    /// </summary>
    [Theory]
    [InlineData("ask")]
    [InlineData("ignore")]
    public void ALandedWriteSaysNothing(string mode)
    {
        Assert.Equal(string.Empty, ProjectManualMode.NoticeFor(false, mode, mode));
    }
}
