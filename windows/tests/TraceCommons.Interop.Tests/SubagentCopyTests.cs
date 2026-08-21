using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The condition: a very large Claude Code conversation had its largest
/// delegated transcripts left out to fit the source's byte budget, and no
/// client said so -- a contributor consenting to a conversation nobody told
/// them was trimmed. The contract calls surfacing that a <c>must</c>. These
/// pin what the card now says, and that the two counts are read off the wire
/// at all.
/// </summary>
public class SubagentCopyTests
{
    /// <summary>
    /// Never a "0 dropped" row: a line that is always present is a line
    /// nobody reads, and the one case that matters would be lost in it.
    /// </summary>
    [Fact]
    public void ACardCoveringNothingDelegatedSaysNothingAtAll()
    {
        Assert.Equal(string.Empty, SubagentCopy.Line(0, 0));
    }

    [Fact]
    public void TheExtentLineCountsInWordsAPersonCanRead()
    {
        Assert.Equal("Includes 1 delegated subagent transcript.", SubagentCopy.Line(1, 0));
        Assert.Equal("Includes 42 delegated subagent transcripts.", SubagentCopy.Line(42, 0));
    }

    /// <summary>
    /// The contract's one <c>must</c>. Every shape with a drop in it says so,
    /// says what survived, and says neither in a word that reads as a fault --
    /// trimming is a normal consequence of a very large session.
    /// </summary>
    [Theory]
    [InlineData(0, 1)]
    [InlineData(0, 7)]
    [InlineData(3, 1)]
    [InlineData(42, 3)]
    public void ADroppedTranscriptIsAlwaysStated(int kept, int dropped)
    {
        string line = SubagentCopy.Line(kept, dropped);
        Assert.NotEqual(string.Empty, line);
        Assert.True(
            line.Contains(dropped.ToString(System.Globalization.CultureInfo.CurrentCulture))
                || (dropped == 1 && line.Contains("largest")),
            $"the count of what was left out has to appear: {line}");
        Assert.Contains("the conversation itself is complete", line);
        foreach (string alarming in new[]
                 { "error", "failed", "corrupt", "incomplete", "lost", "missing" })
        {
            Assert.DoesNotContain(alarming, line.ToLowerInvariant());
        }
    }

    /// <summary>"The 1 largest" is a bug; one dropped transcript is "the largest".</summary>
    [Fact]
    public void OneDroppedTranscriptIsNotDescribedInThePlural()
    {
        Assert.Equal(
            "Includes 42 delegated subagent transcripts. The largest was left out to keep this "
                + "session within its size limit; the conversation itself is complete.",
            SubagentCopy.Line(42, 1));
        Assert.Equal(
            "Includes 42 delegated subagent transcripts. The 3 largest were left out to keep "
                + "this session within its size limit; the conversation itself is complete.",
            SubagentCopy.Line(42, 3));
    }

    /// <summary>
    /// Everything delegated was dropped: there is no kept count to open with,
    /// so the sentence starts from what was left out rather than claiming to
    /// include nothing.
    /// </summary>
    [Fact]
    public void EverythingDroppedStartsFromWhatWasLeftOut()
    {
        Assert.Equal(
            "1 delegated subagent transcript was left out to keep this session within its size "
                + "limit; the conversation itself is complete.",
            SubagentCopy.Line(0, 1));
        Assert.Equal(
            "2 delegated subagent transcripts were left out to keep this session within its "
                + "size limit; the conversation itself is complete.",
            SubagentCopy.Line(0, 2));
    }

    /// <summary>
    /// The copy is worth nothing if the fields never leave the socket. Both
    /// are read off <c>entry_value</c>'s snake_case names.
    /// </summary>
    [Fact]
    public void BothCountsAreDecodedFromTheQueueEntry()
    {
        QueueEntry? entry = JsonSerializer.Deserialize<QueueEntry>(
            """
            {"entry_id":"e1","subagent_count":42,"subagents_dropped":3}
            """);
        Assert.NotNull(entry);
        Assert.Equal(42, entry!.SubagentCount);
        Assert.Equal(3, entry.SubagentsDropped);
    }

    /// <summary>
    /// A daemon predating the fields sends neither, and silence reads as
    /// zero -- which renders nothing, not a wrong number.
    /// </summary>
    [Fact]
    public void AnOlderDaemonReportsNeitherAndTheCardStaysQuiet()
    {
        QueueEntry? entry = JsonSerializer.Deserialize<QueueEntry>(
            """
            {"entry_id":"e1"}
            """);
        Assert.NotNull(entry);
        Assert.Equal(0, entry!.SubagentCount);
        Assert.Equal(0, entry.SubagentsDropped);
        Assert.Equal(string.Empty, SubagentCopy.Line(entry.SubagentCount, entry.SubagentsDropped));
    }
}
