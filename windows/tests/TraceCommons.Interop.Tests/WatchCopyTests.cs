using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Screen 5's copy and the unresolvable-bucket rule.
///
/// These run on a machine that cannot build WinUI, which is the whole reason
/// the logic lives in the interop assembly. Nothing below touches the view.
/// </summary>
public class WatchCopyTests
{
    /// <summary>
    /// The literal here is not a guess. It was read out of the daemon by
    /// calling <c>policy::project_id_for(UNKNOWN_PROJECT_KEY)</c> directly in a
    /// throwaway Rust test, which printed <c>proj_a49d2e499850e74c</c>.
    ///
    /// Pinning it is what makes the C# derivation trustworthy: if
    /// <see cref="WatchCopy.DeriveProjectId"/>'s mirror of the Rust ever drifts
    /// -- wrong hash, wrong prefix, wrong nibble count -- this fails. It cannot
    /// catch the Rust side changing its own algorithm; see the remark on
    /// <see cref="WatchCopy.UnknownProjectId"/>.
    /// </summary>
    [Fact]
    public void AnUnresolvableBucketIsRecognisedByItsDaemonMintedId()
    {
        Assert.Equal("proj_a49d2e499850e74c", WatchCopy.UnknownProjectId);
        Assert.True(WatchCopy.IsUnresolvable("proj_a49d2e499850e74c"));
    }

    /// <summary>
    /// Recognition goes through the id, never the label. A shell that matched
    /// on the label would claim any project a contributor happened to name
    /// "unknown-project" could never be armed, which is a lie about their own
    /// repository.
    /// </summary>
    [Fact]
    public void RecognitionIsByIdAndNeverByLabel()
    {
        const string impostorId = "proj_0000000000000000";

        Assert.False(WatchCopy.IsUnresolvable(impostorId));
        Assert.Equal("unknown-project", WatchCopy.LabelFor(impostorId, "unknown-project"));
        Assert.Equal(WatchCopy.AskMeFirst, WatchCopy.SubLineFor(impostorId, "ask"));
    }

    /// <summary>
    /// The wire carries the slug as this row's label because the daemon refuses
    /// to degrade it into something that might contain a path. A slug is not a
    /// project name, so the screen must not show one.
    /// </summary>
    [Fact]
    public void TheBucketNeverRendersTheRawSlugAsItsName()
    {
        string shown = WatchCopy.LabelFor(WatchCopy.UnknownProjectId, "unknown-project");

        Assert.Equal("Sessions with no project", shown);
        Assert.DoesNotContain("unknown-project", shown, System.StringComparison.Ordinal);
    }

    /// <summary>
    /// The note REPLACES the state line rather than joining it: "you'll always
    /// be asked" already says what "Ask me first" says, and a row carrying both
    /// says the same thing twice.
    /// </summary>
    [Fact]
    public void TheNoteReplacesTheStateLineOnTheBucket()
    {
        string sub = WatchCopy.SubLineFor(WatchCopy.UnknownProjectId, "ask");

        Assert.Equal(WatchCopy.UnknownNote, sub);
        Assert.DoesNotContain(WatchCopy.AskMeFirst, sub, System.StringComparison.Ordinal);
    }

    /// <summary>
    /// The bucket can be silenced even though it cannot be armed, so its state
    /// line stays the note whichever mode the daemon reports. Ignoring it is a
    /// real action; the note is about arming, not about silence.
    /// </summary>
    [Fact]
    public void TheBucketKeepsItsNoteEvenWhenIgnored()
    {
        Assert.Equal(WatchCopy.UnknownNote, WatchCopy.SubLineFor(WatchCopy.UnknownProjectId, "ignore"));
    }

    [Theory]
    [InlineData("ask", "Ask me first")]
    [InlineData("notify_only", "Ask me first")]
    [InlineData("ignore", "Ignored")]
    public void AnOrdinaryRowShowsItsModeInSettingsVocabulary(string mode, string expected)
    {
        Assert.Equal(expected, WatchCopy.SubLineFor("proj_1111111111111111", mode));
    }

    /// <summary>
    /// A blank label is the daemon telling us nothing useful, which is the same
    /// situation the bucket describes -- so it gets the same words rather than
    /// an empty row.
    /// </summary>
    [Fact]
    public void ABlankLabelFallsBackRatherThanRenderingNothing()
    {
        Assert.Equal(WatchCopy.UnknownLabel, WatchCopy.LabelFor("proj_2222222222222222", ""));
        Assert.Equal(WatchCopy.UnknownLabel, WatchCopy.LabelFor("proj_2222222222222222", null));
    }

    /// <summary>
    /// The subtitle states the default before the exception. If someone
    /// reverses it, a contributor who reads only the first clause learns the
    /// escape hatch instead of what happens by default.
    /// </summary>
    [Fact]
    public void TheSubtitleStatesTheDefaultBeforeTheException()
    {
        int askFirst = WatchCopy.Subtitle.IndexOf("ask-first", System.StringComparison.Ordinal);
        int ignore = WatchCopy.Subtitle.IndexOf("Ignore a project", System.StringComparison.Ordinal);

        Assert.True(askFirst >= 0, "the subtitle must state the ask-first default");
        Assert.True(ignore > askFirst, "the default must come before the exception");
    }
}
