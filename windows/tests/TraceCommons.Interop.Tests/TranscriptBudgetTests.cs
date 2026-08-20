using System;
using System.Linq;
using System.Text;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The transcript budget, checked against the same worked examples the
/// macOS shell holds in
/// <c>macos/Tests/TCShellCoreTests/TranscriptBudgetTests.swift</c>.
///
/// Checked here, they are checked on a machine that cannot build WinUI at
/// all -- which is why the clamp and its notice live in the interop
/// assembly rather than in the App project.
/// </summary>
public sealed class TranscriptBudgetTests
{
    /// <summary>
    /// A body under the budget is passed through untouched and carries no
    /// notice. The common case must not gain a truncation warning it does
    /// not deserve.
    /// </summary>
    [Fact]
    public void ShortBodyIsUnchanged()
    {
        const string text = "line one\nline two\n";
        var clamped = TranscriptBudget.Clamp(text);

        Assert.Equal(text, clamped.Shown);
        Assert.Equal(0, clamped.WithheldBytes);
        Assert.False(clamped.IsClamped);
        Assert.Equal(string.Empty, TranscriptBudget.Notice(clamped));
    }

    /// <summary>
    /// A body exactly at the budget is not clamped. Off-by-one here would
    /// put a "showing the first 64 KB of 64 KB" notice on screen.
    /// </summary>
    [Fact]
    public void BodyExactlyAtBudgetIsNotClamped()
    {
        string text = new string('a', TranscriptBudget.LimitBytes);
        var clamped = TranscriptBudget.Clamp(text);

        Assert.False(clamped.IsClamped);
        Assert.Equal(TranscriptBudget.LimitBytes, Encoding.UTF8.GetByteCount(clamped.Shown));
    }

    /// <summary>
    /// The slice never exceeds the budget, and the withheld count is the
    /// exact remainder -- the notice's arithmetic depends on it.
    /// </summary>
    [Fact]
    public void LongBodyIsClampedToBudget()
    {
        string line = new string('x', 99) + "\n";
        string text = string.Concat(Enumerable.Repeat(line, 20_000)); // ~2 MB
        var clamped = TranscriptBudget.Clamp(text);

        Assert.True(clamped.IsClamped);
        int shownBytes = Encoding.UTF8.GetByteCount(clamped.Shown);
        Assert.True(shownBytes <= TranscriptBudget.LimitBytes);
        Assert.Equal(Encoding.UTF8.GetByteCount(text), clamped.TotalBytes);
        Assert.Equal(clamped.TotalBytes, shownBytes + clamped.WithheldBytes);
    }

    /// <summary>The cut lands on a line boundary, so the last visible line is whole.</summary>
    [Fact]
    public void ClampCutsOnALineBoundary()
    {
        string line = new string('x', 99) + "\n";
        string text = string.Concat(Enumerable.Repeat(line, 20_000));
        var clamped = TranscriptBudget.Clamp(text);

        Assert.EndsWith("\n", clamped.Shown, StringComparison.Ordinal);
        foreach (string l in clamped.Shown.Split('\n', StringSplitOptions.RemoveEmptyEntries))
        {
            Assert.Equal(99, l.Length);
        }
    }

    /// <summary>
    /// A body with no newline in the budget still gets cut, and the cut
    /// does not split a multi-byte character. This is the minified-JSON
    /// case: a four-byte emoji is one UTF-8 sequence AND one UTF-16
    /// surrogate pair, and a naive byte cut lands mid-character with high
    /// probability.
    /// </summary>
    [Fact]
    public void ClampWithoutNewlinesDoesNotSplitACharacter()
    {
        string text = string.Concat(Enumerable.Repeat("\U0001F642", TranscriptBudget.LimitBytes));
        var clamped = TranscriptBudget.Clamp(text);

        Assert.True(clamped.IsClamped);
        int shownBytes = Encoding.UTF8.GetByteCount(clamped.Shown);
        Assert.True(shownBytes <= TranscriptBudget.LimitBytes);
        Assert.DoesNotContain('�', clamped.Shown);

        // Round-tripping the slice reproduces its own bytes: proof the cut
        // is character-aligned rather than merely replacement-free. It also
        // proves no UTF-16 surrogate pair was split, since a split pair
        // cannot round-trip through UTF-8 encode/decode without becoming
        // replacement characters.
        byte[] wholeBytes = Encoding.UTF8.GetBytes(text);
        byte[] prefix = wholeBytes.Take(shownBytes).ToArray();
        Assert.Equal(prefix, Encoding.UTF8.GetBytes(clamped.Shown));
    }

    /// <summary>
    /// A multi-byte body that does have newlines keeps its characters whole
    /// too -- the line-boundary path and the byte-boundary path must both
    /// hold.
    /// </summary>
    [Fact]
    public void ClampWithMultibyteLinesKeepsCharactersWhole()
    {
        string line = new string('é', 50) + "\n"; // é, two bytes in UTF-8
        string text = string.Concat(Enumerable.Repeat(line, 20_000));
        var clamped = TranscriptBudget.Clamp(text);

        Assert.True(clamped.IsClamped);
        Assert.DoesNotContain('�', clamped.Shown);
        foreach (string l in clamped.Shown.Split('\n', StringSplitOptions.RemoveEmptyEntries))
        {
            Assert.Equal(50, l.Length);
        }
    }

    /// <summary>
    /// The notice states both numbers and does not imply approval shrank.
    /// This is the sentence that keeps the tab's promise true, so it is
    /// asserted verbatim rather than by shape.
    /// </summary>
    [Fact]
    public void NoticeStatesShownTotalAndThatApprovalIsUnaffected()
    {
        string text = string.Concat(Enumerable.Repeat("x\n", 9_000_000)); // ~17.2 MB
        var clamped = TranscriptBudget.Clamp(text);
        string notice = TranscriptBudget.Notice(clamped);

        Assert.Equal(
            "Showing the first 64 KB of 17.2 MB. "
                + "The rest is not displayed here. Approving still covers the whole body.",
            notice);
    }

    /// <summary>
    /// The reported "shown" figure is the size of what is actually on
    /// screen, not the budget constant. A cut that backs off to a line
    /// boundary shows slightly less than 64 KB, and the notice must not
    /// round that into a claim about bytes the reader cannot see.
    /// </summary>
    [Fact]
    public void NoticeReportsBytesActuallyShown()
    {
        string line = new string('x', 999) + "\n";
        string text = string.Concat(Enumerable.Repeat(line, 2_000));
        var clamped = TranscriptBudget.Clamp(text);
        int shownBytes = clamped.TotalBytes - clamped.WithheldBytes;

        Assert.Equal(shownBytes, Encoding.UTF8.GetByteCount(clamped.Shown));
    }
}
