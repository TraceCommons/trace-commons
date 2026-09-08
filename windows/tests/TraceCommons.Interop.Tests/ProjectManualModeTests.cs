using System;
using System.IO;
using System.Linq;
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

    /// <summary>
    /// Settings reports the mode the daemon stored, not the one it sent, and
    /// re-reads on the failure path as well as the success one.
    ///
    /// Asserted about the call site rather than about
    /// <see cref="ProjectManualMode.NoticeFor"/>: the helper was already
    /// right and already covered while this screen went on setting its row
    /// from the value it had just requested, which is the state this change
    /// found. A shell that writes a consent row from its own request is
    /// asserting something it never observed, and the failure path is where
    /// the two disagree -- a refused toggle that still flips the row is a
    /// lie the contributor then acts on.
    ///
    /// Read from the copied source because <c>TraceCommons.App</c> is a WinUI
    /// project that cannot be built, let alone referenced, on the machines
    /// this suite runs on. That limit is worth naming: this catches the
    /// optimistic write coming back and the re-read being made conditional,
    /// and it does not prove the rebuilt rows reach the screen.
    /// </summary>
    [Fact]
    public void TheSettingsToggleReadsBackTheStoredModeOnBothPaths()
    {
        string body = SettingsMethodBody("public async Task ToggleProjectAsync");

        Assert.Contains("await LoadProjectsAsync()", body, StringComparison.Ordinal);
        Assert.Contains("ProjectManualMode.NoticeFor(", body, StringComparison.Ordinal);

        // The optimism itself: nothing here may set a row's mode from the
        // value that was sent.
        Assert.DoesNotContain("SetMode", body, StringComparison.Ordinal);

        // The re-read is unconditional, and it happens before anything is
        // said about the outcome. A reload placed after the notice would be
        // a reload the notice never saw.
        Assert.True(
            body.IndexOf("await LoadProjectsAsync()", StringComparison.Ordinal)
                < body.IndexOf("ProjectManualMode.NoticeFor(", StringComparison.Ordinal),
            "the re-read must run before the outcome is reported");

        // The daemon's answer is mentioned exactly once in this method, as
        // the argument handed to NoticeFor. Any second mention is a branch,
        // and a branch here is a path on which the write is believed rather
        // than checked.
        Assert.Equal(1, Occurrences(body, "IsError"));
        Assert.Matches(@"NoticeFor\(\s*response\.IsError", body);
    }

    /// <summary>
    /// The setter that made the optimistic write possible is gone from the
    /// row, for the reason onboarding's was: a row's mode arrives from
    /// <c>list_projects</c> and from nowhere else, and removing the door is
    /// what stops the next caller opening it again.
    /// </summary>
    [Fact]
    public void TheSettingsRowHasNoModeSetterLeftToWriteOptimisticallyWith()
    {
        Assert.DoesNotContain("public void SetMode", SettingsSource(), StringComparison.Ordinal);
    }

    private static int Occurrences(string haystack, string needle) =>
        haystack.Split(needle).Length - 1;

    /// <summary>
    /// The settings view model, copied beside the test assembly, with its C#
    /// comments stripped -- prose about the rule names the very identifiers
    /// the rule forbids.
    /// </summary>
    private static string SettingsSource()
    {
        string path = Path.Combine(AppContext.BaseDirectory, "ContributorSettingsViewModel.cs.txt");
        Assert.True(File.Exists(path), $"the app source was not copied to {path}");

        return string.Join(
            "\n",
            File.ReadAllLines(path)
                .Where(line => !line.TrimStart().StartsWith("//", StringComparison.Ordinal)));
    }

    /// <summary>
    /// One method's body, from its signature to the next member declared at
    /// the same indentation. Scoped so a call living in some other method
    /// cannot satisfy an assertion made about this one.
    /// </summary>
    private static string SettingsMethodBody(string signature)
    {
        string source = SettingsSource();
        int start = source.IndexOf(signature, StringComparison.Ordinal);
        Assert.True(start >= 0, $"{signature} was not found in the settings view model");

        int from = start + signature.Length;
        int next = source.IndexOf("\n    public ", from, StringComparison.Ordinal);
        int alsoNext = source.IndexOf("\n    private ", from, StringComparison.Ordinal);
        if (alsoNext >= 0 && (next < 0 || alsoNext < next))
        {
            next = alsoNext;
        }

        return next < 0 ? source.Substring(start) : source.Substring(start, next - start);
    }
}
