using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Screen 5's copy and the unresolvable-bucket rule.
///
/// These run on a machine that cannot build WinUI, which is the whole reason
/// the logic lives in the interop assembly. Nothing below touches the view.
///
/// Which row is the unresolvable bucket is NOT decided here and is not tested
/// here: the daemon marks it with <c>is_unresolved_bucket</c>, and two Rust
/// tests pin that on the side that owns it. What these assert is this shell's
/// CONSUMPTION of the flag.
/// </summary>
public class WatchCopyTests
{
    /// <summary>
    /// The flag is read off the wire, not inferred. If the field were dropped
    /// or renamed, every bucket row would silently render as an ordinary
    /// project and lose the note explaining why it can never be armed.
    ///
    /// The ids below are arbitrary on purpose. This shell used to re-derive the
    /// daemon's opaque id to find this row, and that duplicate is gone: nothing
    /// here depends on what the id happens to be.
    /// </summary>
    [Fact]
    public void TheUnresolvedFlagIsDecodedFromThePayload()
    {
        const string json = """
        {"projects":[
          {"project_id":"proj_1111111111111111","project_label":"frobnicator","mode":"ask","configured":true,"is_unresolved_bucket":false},
          {"project_id":"proj_2222222222222222","project_label":"unknown-project","mode":"ask","configured":false,"is_unresolved_bucket":true}
        ]}
        """;

        ProjectSettingsPayload payload = JsonSerializer.Deserialize<ProjectSettingsPayload>(json)!;

        Assert.False(payload.Projects[0].IsUnresolvedBucket);
        Assert.True(payload.Projects[1].IsUnresolvedBucket);
    }

    /// <summary>
    /// A row the daemon did not mark is an ordinary project, whatever it is
    /// called. The wire carries the slug <c>unknown-project</c> as the bucket's
    /// label, and matching on that would tell a contributor who happened to
    /// name a repository the same thing that it can never be armed.
    /// <c>docs/contributor-daemon-ipc-v1_1.md</c> forbids the label match.
    /// </summary>
    [Fact]
    public void ALabelThatLooksLikeTheBucketIsStillAnOrdinaryProject()
    {
        Assert.Equal("unknown-project", WatchCopy.LabelFor(false, "unknown-project"));
        Assert.Equal(WatchCopy.AskMeFirst, WatchCopy.SubLineFor(false, "ask"));
    }

    /// <summary>
    /// The wire carries the slug as the bucket's label because the daemon
    /// refuses to degrade it into something that might contain a path. A slug
    /// is not a project name, so the screen must not show one.
    /// </summary>
    [Fact]
    public void TheBucketNeverRendersTheRawSlugAsItsName()
    {
        string shown = WatchCopy.LabelFor(true, "unknown-project");

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
        string sub = WatchCopy.SubLineFor(true, "ask");

        Assert.Equal(WatchCopy.UnknownNote, sub);
        Assert.DoesNotContain(WatchCopy.AskMeFirst, sub, System.StringComparison.Ordinal);
    }

    /// <summary>
    /// The bucket can be silenced even though it cannot be armed, so its note
    /// stays whichever mode the daemon reports. This is the client half of what
    /// the Rust test <c>the_unresolvable_flag_survives_being_ruled_on</c> pins:
    /// the row must not lose its explanation exactly when someone acts on it.
    /// </summary>
    [Fact]
    public void TheBucketKeepsItsNoteEvenWhenIgnored()
    {
        Assert.Equal(WatchCopy.UnknownNote, WatchCopy.SubLineFor(true, "ignore"));
    }

    [Theory]
    [InlineData("ask", "Ask me first")]
    [InlineData("notify_only", "Ask me first")]
    [InlineData("ignore", "Ignored")]
    public void AnOrdinaryRowShowsItsModeInSettingsVocabulary(string mode, string expected)
    {
        Assert.Equal(expected, WatchCopy.SubLineFor(false, mode));
    }

    /// <summary>
    /// A blank label is the daemon telling us nothing useful, which is the same
    /// situation the bucket describes -- so it gets the same words rather than
    /// an empty row.
    /// </summary>
    [Fact]
    public void ABlankLabelFallsBackRatherThanRenderingNothing()
    {
        Assert.Equal(WatchCopy.UnknownLabel, WatchCopy.LabelFor(false, ""));
        Assert.Equal(WatchCopy.UnknownLabel, WatchCopy.LabelFor(false, null));
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
