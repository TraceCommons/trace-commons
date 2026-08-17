using System;
using System.Collections.Generic;
using System.Text;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Tests for the UTF-8 byte offset to UTF-16 index conversion.
///
/// This is the one piece of real logic in the binding, and it is the one place
/// a wrong answer is silent: a mis-shifted highlight looks like a highlight.
/// The ABI reports byte offsets; a C# string is UTF-16; the gap between them
/// opens the moment a transcript contains anything outside ASCII.
///
/// These tests are pure -- no native library, no daemon -- so they run
/// anywhere.
/// </summary>
public class OffsetConversionTests
{
    /// <summary>
    /// The property under test, stated independently of the implementation:
    /// converting the byte offset of a needle must yield an index at which
    /// that needle actually appears in the managed string.
    /// </summary>
    private static void AssertRoundTrip(string text, string needle)
    {
        byte[] utf8 = Encoding.UTF8.GetBytes(text);
        byte[] needleUtf8 = Encoding.UTF8.GetBytes(needle);

        var byteOffsets = new List<int>();
        for (int i = 0; i + needleUtf8.Length <= utf8.Length; i++)
        {
            bool match = true;
            for (int j = 0; j < needleUtf8.Length; j++)
            {
                if (utf8[i + j] != needleUtf8[j])
                {
                    match = false;
                    break;
                }
            }

            if (match)
            {
                byteOffsets.Add(i);
                i += needleUtf8.Length - 1;
            }
        }

        IReadOnlyList<int> utf16 =
            TcPreview.ConvertUtf8OffsetsToUtf16(text, byteOffsets.ToArray());

        Assert.Equal(byteOffsets.Count, utf16.Count);
        foreach (int index in utf16)
        {
            Assert.True(
                index + needle.Length <= text.Length,
                $"index {index} runs past the end of a {text.Length}-unit string");
            Assert.Equal(needle, text.Substring(index, needle.Length));
        }
    }

    [Fact]
    public void PureAsciiOffsetsAreUnchanged()
    {
        // The degenerate case the naive implementation also gets right, kept
        // so a regression that breaks ASCII is caught immediately rather than
        // only via the harder cases.
        const string text = "the quick brown fox";
        IReadOnlyList<int> converted =
            TcPreview.ConvertUtf8OffsetsToUtf16(text, new[] { 0, 4, 10 });

        Assert.Equal(new[] { 0, 4, 10 }, converted);
    }

    [Fact]
    public void TwoByteCharactersShiftSubsequentOffsets()
    {
        // "é" is 2 UTF-8 bytes but 1 UTF-16 unit, so every later match sits at
        // a lower index than its byte offset. Handing the raw offset to a
        // highlighter would run one character long.
        const string text = "café bar";
        AssertRoundTrip(text, "bar");

        IReadOnlyList<int> converted =
            TcPreview.ConvertUtf8OffsetsToUtf16(text, new[] { 6 });
        Assert.Equal(new[] { 5 }, converted);
    }

    [Fact]
    public void ThreeByteCharactersShiftSubsequentOffsets()
    {
        // CJK: 3 UTF-8 bytes, 1 UTF-16 unit. Two characters in, the drift is
        // already 4.
        const string text = "日本 test";
        AssertRoundTrip(text, "test");
    }

    [Fact]
    public void SurrogatePairsAdvanceFourBytesAndTwoUnits()
    {
        // The case that breaks implementations which assume one UTF-16 unit
        // per character: an emoji is 4 UTF-8 bytes AND 2 UTF-16 units, so the
        // drift is +2 rather than +3.
        const string text = "ok 🎯 target";
        AssertRoundTrip(text, "target");

        Assert.Equal(2, "🎯".Length);
        Assert.Equal(4, Encoding.UTF8.GetByteCount("🎯"));
    }

    [Fact]
    public void MixedWidthTranscriptRoundTripsEveryMatch()
    {
        // A realistic worst case: every width class in one string, with the
        // needle appearing repeatedly after each.
        const string text =
            "start x café x 日本語 x 🎯🎯 x done x";
        AssertRoundTrip(text, "x");
    }

    [Fact]
    public void OffsetsAreSortedBeforeWalking()
    {
        // The ABI documents matches as left-to-right, but the walk is
        // single-pass and forward-only, so unsorted input must not silently
        // drop matches.
        const string text = "café bar café baz";
        IReadOnlyList<int> converted =
            TcPreview.ConvertUtf8OffsetsToUtf16(text, new[] { 11, 0 });

        Assert.Equal(new[] { 0, 10 }, converted);
    }

    [Fact]
    public void OffsetInsideAMultiByteSequenceResolvesToItsCharacterStart()
    {
        // Not expected from the ABI, but an offset that lands mid-character
        // must resolve to a real index rather than being dropped or throwing.
        const string text = "café";
        IReadOnlyList<int> converted =
            TcPreview.ConvertUtf8OffsetsToUtf16(text, new[] { 4 });

        Assert.Single(converted);
        Assert.Equal(3, converted[0]);
    }

    [Fact]
    public void OffsetPastTheEndIsDroppedNotClamped()
    {
        // A match that cannot be located is not a match at position zero, and
        // it is not a match at the last character either. Dropping it is the
        // only answer that does not invent a highlight.
        const string text = "short";
        IReadOnlyList<int> converted =
            TcPreview.ConvertUtf8OffsetsToUtf16(text, new[] { 0, 999 });

        Assert.Equal(new[] { 0 }, converted);
    }

    [Fact]
    public void EmptyOffsetsYieldEmptyResult()
    {
        Assert.Empty(TcPreview.ConvertUtf8OffsetsToUtf16("anything", Array.Empty<int>()));
    }

    [Fact]
    public void EmptyTextWithZeroOffsetYieldsIndexZero()
    {
        // Guards the loop bound: utf16Index <= text.Length must admit the
        // zero-length case rather than skipping the loop entirely.
        Assert.Equal(new[] { 0 }, TcPreview.ConvertUtf8OffsetsToUtf16(string.Empty, new[] { 0 }));
    }
}
