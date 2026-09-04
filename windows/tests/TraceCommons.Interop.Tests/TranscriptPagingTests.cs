using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.Linq;
using System.Text;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The transcript tab's promise is "exactly what would be sent", and the
/// button beneath it approves every byte. These tests exist to keep two
/// things true at once: every byte is reachable, and no more than
/// <see cref="TranscriptPaging.RetainedLimitBytes"/> of it is typeset at any
/// moment.
///
/// They mirror <c>macos/Tests/TCShellCoreTests/TranscriptPagingTests.swift</c>
/// assertion for assertion, plus the two hazards that are specific to C#: a
/// UTF-16 string cut on a UTF-8 budget can split a surrogate pair as well as
/// a UTF-8 sequence, and a four-byte emoji is one of each.
///
/// Checked here, they are checked on a machine that cannot build WinUI at
/// all -- which is why the paging lives in the interop assembly rather than
/// in the App project.
/// </summary>
public sealed class TranscriptPagingTests
{
    /// <summary>
    /// A transcript-shaped body: JSON-ish lines around 78 bytes, with a
    /// redaction marker every so often, cut to an exact byte size.
    /// </summary>
    private static string Body(int bytes)
    {
        // Every character here is ASCII, so UTF-16 length and UTF-8 byte
        // count are the same number and the body can be cut to an exact
        // byte size by cutting to a length.
        var builder = new StringBuilder(bytes + 128);
        int line = 0;
        while (builder.Length < bytes)
        {
            string marker = line % 17 == 0 ? "<PRIVATE_SECRET_1>" : "ordinary";
            builder.AppendFormat(
                CultureInfo.InvariantCulture,
                "{{\"turn\":{0,5},\"role\":\"assistant\",\"text\":\"{1} content here padding\"}}\n",
                line,
                marker);
            line += 1;
        }

        return builder.ToString(0, bytes);
    }

    private static string Lines(int count, int width) =>
        string.Concat(Enumerable.Repeat(new string('x', width) + "\n", count));

    // MARK: - Every byte reachable

    /// <summary>
    /// The point of the change. The chunks, concatenated, are the body --
    /// not a prefix of it, not a lossy re-encoding of it.
    /// </summary>
    [Fact]
    public void EveryByteIsReachable()
    {
        string body = Body(600_000);
        var document = new TranscriptDocument(body);

        var rebuilt = new StringBuilder();
        for (int i = 0; i < document.ChunkCount; i++)
        {
            rebuilt.Append(document.TextOf(i));
        }

        Assert.Equal(body, rebuilt.ToString());
        Assert.Equal(body, document.WholeText());
        Assert.Equal(Encoding.UTF8.GetByteCount(body), document.TotalBytes);
    }

    /// <summary>Chunks tile the body exactly: no gap, no overlap, first starts at 0.</summary>
    [Fact]
    public void ChunksTileTheBodyWithoutGapOrOverlap()
    {
        var document = new TranscriptDocument(Body(600_000));

        Assert.Equal(0, document.Chunks[0].ByteOffset);
        for (int i = 1; i < document.ChunkCount; i++)
        {
            Assert.Equal(document.Chunks[i - 1].ByteEnd, document.Chunks[i].ByteOffset);
        }

        Assert.Equal(document.TotalBytes, document.Chunks[^1].ByteEnd);
    }

    [Fact]
    public void EmptyBodyHasNoChunks()
    {
        var document = new TranscriptDocument(string.Empty);

        Assert.Equal(0, document.ChunkCount);
        Assert.Equal(0, document.TotalBytes);
        Assert.Equal(string.Empty, document.WholeText());
    }

    /// <summary>A body smaller than one chunk is one chunk, unchanged.</summary>
    [Fact]
    public void ShortBodyIsASingleChunk()
    {
        const string body = "line one\nline two\n";
        var document = new TranscriptDocument(body);

        Assert.Equal(1, document.ChunkCount);
        Assert.Equal(body, document.TextOf(0));
        Assert.Equal(2, document.Chunks[0].LineCount);
    }

    /// <summary>
    /// Every chunk is inside the declared ceiling, and the ordinary
    /// newline-terminated ones are also above the floor -- otherwise a body
    /// of short lines would degenerate into thousands of tiny chunks and the
    /// per-chunk view overhead would become the new problem.
    /// </summary>
    [Fact]
    public void ChunkSizesStayWithinTheDeclaredBounds()
    {
        var document = new TranscriptDocument(Body(2_000_000));

        Assert.True(document.ChunkCount > 400);
        for (int i = 0; i < document.ChunkCount; i++)
        {
            TranscriptChunk chunk = document.Chunks[i];
            Assert.InRange(chunk.ByteCount, 1, TranscriptPaging.MaxChunkBytes);
            if (i < document.ChunkCount - 1)
            {
                Assert.True(
                    chunk.ByteCount >= TranscriptPaging.TargetChunkBytes / 2,
                    $"chunk {i} is only {chunk.ByteCount} bytes");
            }
        }
    }

    /// <summary>
    /// The line-boundary path is the normal one: every chunk of a
    /// newline-terminated body ends at a newline.
    /// </summary>
    [Fact]
    public void ChunksEndOnLineBoundaries()
    {
        var document = new TranscriptDocument(Lines(30_000, 77));

        for (int i = 0; i < document.ChunkCount; i++)
        {
            Assert.EndsWith("\n", document.TextOf(i), StringComparison.Ordinal);
        }
    }

    // MARK: - Character and surrogate boundaries

    /// <summary>
    /// Four-byte scalars with no newline anywhere: the minified-JSON case,
    /// where a naive byte cut lands mid-character with high probability.
    ///
    /// A four-byte emoji is ONE UTF-8 sequence and ONE UTF-16 surrogate
    /// pair. The round-trip assertion covers both at once: a chunk whose
    /// UTF-8 bytes are exactly the corresponding slice of the body's bytes
    /// cannot contain a lone surrogate, because a lone surrogate does not
    /// survive a UTF-16 to UTF-8 encode.
    /// </summary>
    [Fact]
    public void BoundariesNeverSplitACharacterWithoutNewlines()
    {
        // The two-byte ASCII prefix is what makes this a real test: without
        // it every 4,096-byte target would already sit on a four-byte
        // sequence boundary and the back-off would never run. With it, the
        // first target lands two bytes into an emoji.
        string body = "ab" + string.Concat(Enumerable.Repeat("\U0001F642", 20_000)); // ~80 KB
        var document = new TranscriptDocument(body);
        byte[] whole = Encoding.UTF8.GetBytes(body);

        Assert.True(document.ChunkCount > 15);

        // 4,096 lands at 4,094 + 2 bytes into a sequence, so the cut backs
        // off by exactly two bytes. Asserted as a number, not as "it did not
        // crash".
        Assert.Equal(4_094, document.Chunks[0].ByteCount);

        for (int i = 0; i < document.ChunkCount; i++)
        {
            TranscriptChunk chunk = document.Chunks[i];
            string text = document.TextOf(i);

            Assert.DoesNotContain('�', text);

            // The chunk's bytes are exactly the body's bytes at that range:
            // proof the cut is character-aligned rather than merely
            // replacement-free, and proof no UTF-16 surrogate pair was split,
            // since a lone surrogate cannot survive a UTF-8 round trip.
            Assert.Equal(
                whole.Skip(chunk.ByteOffset).Take(chunk.ByteCount).ToArray(),
                Encoding.UTF8.GetBytes(text));

            // Stated directly as well as implied: no chunk opens with a low
            // surrogate or closes with a high one.
            Assert.False(char.IsLowSurrogate(text[0]));
            Assert.False(char.IsHighSurrogate(text[^1]));
        }
    }

    /// <summary>
    /// Multi-byte characters on newline-terminated lines: both the line path
    /// and the scalar path have to hold.
    /// </summary>
    [Fact]
    public void BoundariesNeverSplitACharacterWithMultibyteLines()
    {
        string body = string.Concat(
            Enumerable.Repeat(new string('é', 50) + "\n", 5_000)); // e-acute, 2 bytes
        var document = new TranscriptDocument(body);

        var rebuilt = new StringBuilder();
        for (int i = 0; i < document.ChunkCount; i++)
        {
            string text = document.TextOf(i);
            Assert.DoesNotContain('�', text);
            rebuilt.Append(text);
        }

        Assert.Equal(body, rebuilt.ToString());
    }

    /// <summary>
    /// Astral characters mixed into newline-terminated lines. The line path
    /// is safe by construction, but a chunk must still never hand the view a
    /// half surrogate pair, so it is asserted rather than reasoned about.
    /// </summary>
    [Fact]
    public void SurrogatePairsSurviveLineTerminatedBodies()
    {
        string body = string.Concat(
            Enumerable.Repeat("prefix \U0001F600\U0001F601 suffix padding here\n", 6_000));
        var document = new TranscriptDocument(body);

        for (int i = 0; i < document.ChunkCount; i++)
        {
            string text = document.TextOf(i);
            Assert.False(char.IsLowSurrogate(text[0]));
            Assert.False(char.IsHighSurrogate(text[^1]));
            Assert.DoesNotContain('�', text);
        }
    }

    // MARK: - Markers across boundaries

    /// <summary>
    /// A marker placed at every byte offset across the first chunk boundary,
    /// on a body with no newline to cut on, must come out whole in exactly
    /// one chunk.
    ///
    /// This is the failure the chip rendering cannot survive:
    /// <c>&lt;PRIVATE_SEC</c> in one chunk and <c>RET_1&gt;</c> in the next
    /// are both drawn as ordinary body text, which reads as content that was
    /// never scrubbed.
    /// </summary>
    [Fact]
    public void MarkerStraddlingABoundaryIsNotSplit()
    {
        AssertMarkerSurvivesEveryOffset("<PRIVATE_SECRET_1>");
    }

    /// <summary>The same for the labelled <c>[REDACTED:...]</c> family.</summary>
    [Fact]
    public void LabelledMarkerStraddlingABoundaryIsNotSplit()
    {
        AssertMarkerSurvivesEveryOffset("[REDACTED:aws_secret_key]");
    }

    /// <summary>The same for the bare <c>[REDACTED]</c> secrets token.</summary>
    [Fact]
    public void BareRedactedMarkerStraddlingABoundaryIsNotSplit()
    {
        AssertMarkerSurvivesEveryOffset("[REDACTED]");
    }

    private static void AssertMarkerSurvivesEveryOffset(string marker)
    {
        int target = TranscriptPaging.TargetChunkBytes;

        for (int offset = target - marker.Length - 2; offset <= target + 2; offset++)
        {
            string body = new string('a', offset)
                + marker
                + new string('b', target);
            var document = new TranscriptDocument(body);

            int whole = 0;
            for (int i = 0; i < document.ChunkCount; i++)
            {
                string text = document.TextOf(i);
                whole += CountOccurrences(text, marker);

                // No chunk may end with a prefix of the marker or begin with
                // a suffix of it: that is the split, seen from the outside.
                for (int cut = 1; cut < marker.Length; cut++)
                {
                    if (i + 1 < document.ChunkCount)
                    {
                        Assert.False(
                            text.EndsWith(marker[..cut], StringComparison.Ordinal),
                            $"chunk {i} ends with the first {cut} chars of {marker} "
                                + $"at offset {offset}");
                    }
                }
            }

            Assert.Equal(1, whole);
        }
    }

    private static int CountOccurrences(string haystack, string needle)
    {
        int count = 0;
        int from = 0;
        while (true)
        {
            int at = haystack.IndexOf(needle, from, StringComparison.Ordinal);
            if (at < 0)
            {
                return count;
            }

            count += 1;
            from = at + needle.Length;
        }
    }

    /// <summary>
    /// Chipping per chunk finds exactly the markers chipping the whole body
    /// would find -- same count, same text, same order. This is the property
    /// the view depends on now that it never scans the whole body.
    /// </summary>
    [Fact]
    public void PerChunkMarkerScanMatchesWholeBodyScan()
    {
        string body = Body(400_000);
        var document = new TranscriptDocument(body);

        List<string> whole = TranscriptMarkers.Split(body)
            .Where(run => run.IsMarker)
            .Select(run => body.Substring(run.Start, run.Length))
            .ToList();

        var perChunk = new List<string>();
        for (int i = 0; i < document.ChunkCount; i++)
        {
            string text = document.TextOf(i);
            perChunk.AddRange(
                TranscriptMarkers.Split(text)
                    .Where(run => run.IsMarker)
                    .Select(run => text.Substring(run.Start, run.Length)));
        }

        Assert.True(whole.Count > 100);
        Assert.Equal(whole, perChunk);
    }

    /// <summary>
    /// Per-chunk runs also cover their chunk exactly once, so no byte is
    /// dropped between two chips at a boundary.
    /// </summary>
    [Fact]
    public void PerChunkRunsCoverTheChunkExactly()
    {
        var document = new TranscriptDocument(Body(200_000));

        for (int i = 0; i < document.ChunkCount; i++)
        {
            string text = document.TextOf(i);
            var rebuilt = new StringBuilder();
            int cursor = 0;
            foreach (TranscriptRun run in TranscriptMarkers.Split(text))
            {
                Assert.Equal(cursor, run.Start);
                rebuilt.Append(text.Substring(run.Start, run.Length));
                cursor += run.Length;
            }

            Assert.Equal(text, rebuilt.ToString());
        }
    }

    /// <summary>
    /// An unclosed bracket does not turn the rest of the body into one
    /// enormous "marker" the chunker then refuses to cut. Without the
    /// newline exclusion in the pattern's <c>[REDACTED...]</c> arm this
    /// produces a single chunk the size of the body -- the original hang,
    /// reached through the fix.
    /// </summary>
    [Fact]
    public void UnclosedBracketDoesNotSwallowTheBody()
    {
        string body = "[REDACTED:unclosed\n" + Lines(3_000, 77);
        var document = new TranscriptDocument(body);

        Assert.True(document.ChunkCount > 40);
        foreach (TranscriptChunk chunk in document.Chunks)
        {
            Assert.InRange(chunk.ByteCount, 1, TranscriptPaging.MaxChunkBytes);
        }
    }

    // MARK: - Retention and eviction

    /// <summary>
    /// The ceiling holds at every step of a scroll through a 17.5 MB body,
    /// and the visible chunk is always among the chunks that are typeset.
    ///
    /// This is the assertion the whole design is for. It is not enough that
    /// the window stops growing: it has to stay under the ceiling from the
    /// first screen to the last.
    /// </summary>
    [Fact]
    public void RetainedBytesCeilingHoldsWhileScrollingSeventeenMegabytes()
    {
        var document = new TranscriptDocument(Body(17_500_000));
        Assert.True(document.ChunkCount > 4_000);

        var resident = new TranscriptResidentChunks<string>();
        int peak = 0;

        for (int index = 0; index < document.ChunkCount; index += 7)
        {
            resident.Update(
                document,
                new ChunkRange(index, index + 1),
                document.TextOf);

            Assert.True(
                resident.RetainedBytes <= TranscriptPaging.RetainedLimitBytes,
                $"retained {resident.RetainedBytes} at chunk {index}");
            Assert.True(resident.Window.Contains(index));
            Assert.Equal(resident.Window.Count, resident.ResidentCount);
            peak = Math.Max(peak, resident.RetainedBytes);
        }

        // The window really does fill: a ceiling that holds because nothing
        // was ever retained would prove nothing.
        Assert.True(
            peak > TranscriptPaging.RetainedLimitBytes - TranscriptPaging.MaxChunkBytes,
            $"peak retention was only {peak}");
        Assert.True(resident.Evictions > 3_000);
    }

    /// <summary>
    /// Eviction, stated directly: after scrolling away, the chunk that was
    /// on screen at the start is no longer typeset. A window that only ever
    /// adds passes every ceiling test above by never reaching the ceiling
    /// early; this is what catches it.
    /// </summary>
    [Fact]
    public void ChunksThatScrollAwayAreEvicted()
    {
        var document = new TranscriptDocument(Body(4_000_000));
        var resident = new TranscriptResidentChunks<string>();

        resident.Update(document, new ChunkRange(0, 1), document.TextOf);
        Assert.True(resident.TryGet(0, out _));
        int retainedAtStart = resident.RetainedBytes;

        resident.Update(document, new ChunkRange(500, 501), document.TextOf);

        Assert.False(resident.TryGet(0, out _));
        Assert.True(resident.Evictions >= retainedAtStart / TranscriptPaging.MaxChunkBytes);
        Assert.True(resident.RetainedBytes <= TranscriptPaging.RetainedLimitBytes);
        Assert.True(resident.Window.Contains(500));
    }

    /// <summary>
    /// Advancing by one chunk typesets one chunk, not a window's worth. The
    /// per-chunk layout cost is only a frame if this holds.
    /// </summary>
    [Fact]
    public void AdvancingOneChunkRendersOneChunk()
    {
        var document = new TranscriptDocument(Body(1_000_000));
        var resident = new TranscriptResidentChunks<string>();

        resident.Update(document, new ChunkRange(100, 101), document.TextOf);
        int rendersAfterFill = resident.Renders;
        int evictionsAfterFill = resident.Evictions;

        resident.Update(document, new ChunkRange(101, 102), document.TextOf);

        Assert.Equal(rendersAfterFill + 1, resident.Renders);
        Assert.Equal(evictionsAfterFill + 1, resident.Evictions);
    }

    /// <summary>
    /// The window is centred on the viewport, so overscan exists in both
    /// directions rather than only ahead.
    /// </summary>
    [Fact]
    public void WindowOverscansBothWays()
    {
        var document = new TranscriptDocument(Body(4_000_000));
        ChunkRange window = TranscriptResidency.Window(document, new ChunkRange(500, 501));

        Assert.True(500 - window.Start > 5);
        Assert.True(window.End - 501 > 5);
        Assert.InRange(Math.Abs((500 - window.Start) - (window.End - 501)), 0, 2);
    }

    /// <summary>
    /// At the ends of the body the window does not shrink: it spends all of
    /// its budget on the side that exists.
    /// </summary>
    [Fact]
    public void WindowAtTheStartAndEndStillFillsItsBudget()
    {
        var document = new TranscriptDocument(Body(4_000_000));

        foreach (ChunkRange visible in new[]
        {
            new ChunkRange(0, 1),
            new ChunkRange(document.ChunkCount - 1, document.ChunkCount),
        })
        {
            ChunkRange window = TranscriptResidency.Window(document, visible);
            int bytes = Enumerable.Range(window.Start, window.Count)
                .Sum(i => document.Chunks[i].ByteCount);

            Assert.True(bytes <= TranscriptPaging.RetainedLimitBytes);
            Assert.True(
                bytes > TranscriptPaging.RetainedLimitBytes - TranscriptPaging.MaxChunkBytes,
                $"window at {visible.Start} only held {bytes} bytes");
        }
    }

    /// <summary>
    /// A body smaller than the ceiling is entirely resident: the chunking
    /// must not cost small traces anything.
    /// </summary>
    [Fact]
    public void SmallBodyIsFullyResident()
    {
        var document = new TranscriptDocument(Body(60_000));
        var resident = new TranscriptResidentChunks<string>();

        resident.Update(document, new ChunkRange(0, 1), document.TextOf);

        Assert.Equal(document.ChunkCount, resident.ResidentCount);
        Assert.Equal(document.TotalBytes, resident.RetainedBytes);
        Assert.Equal(0, resident.Evictions);
    }

    // MARK: - Placing what is not resident

    /// <summary>
    /// Row offsets are cumulative and complete, so a non-resident chunk
    /// holds exactly its own place in the scroll.
    /// </summary>
    [Fact]
    public void RowIndexIsCumulativeAndComplete()
    {
        var document = new TranscriptDocument(Body(400_000));
        var rows = new TranscriptRowIndex(document, 89);

        Assert.Equal(document.ChunkCount + 1, rows.RowStarts.Count);
        Assert.Equal(0, rows.RowStarts[0]);
        for (int i = 0; i < document.ChunkCount; i++)
        {
            Assert.True(rows.RowsOf(i) >= 1);
            Assert.Equal(rows.RowStarts[i] + rows.RowsOf(i), rows.RowStarts[i + 1]);
        }

        Assert.Equal(rows.RowStarts[^1], rows.TotalRows);
    }

    /// <summary>
    /// The estimate is exact when nothing wraps: 78-byte lines at 89 columns
    /// are one row each.
    /// </summary>
    [Fact]
    public void RowEstimateIsExactWhenNothingWraps()
    {
        var document = new TranscriptDocument(Lines(4_000, 77));
        var rows = new TranscriptRowIndex(document, 89);

        Assert.Equal(4_000, rows.TotalRows);
    }

    /// <summary>
    /// And when everything wraps, it is the wrapped count, rounded up once
    /// per chunk. One unbroken 8,900-byte line at 89 columns is 100 rows;
    /// cut into chunks of 4,096, 4,096 and 708 bytes it estimates
    /// 47 + 47 + 8 = 102. Two rows of slack over 100 is the cost of placing
    /// chunks without measuring them, and it is bounded at one row per chunk
    /// -- 16 px of scroll extent per 4 KB.
    /// </summary>
    [Fact]
    public void RowEstimateCountsWrappedRowsRoundedPerChunk()
    {
        var document = new TranscriptDocument(new string('x', 8_900));
        var rows = new TranscriptRowIndex(document, 89);

        Assert.Equal(3, document.ChunkCount);
        Assert.Equal(102, rows.TotalRows);
        Assert.InRange(rows.TotalRows, 100, 100 + document.ChunkCount);
    }

    [Fact]
    public void RowLookupClampsOutOfRangeRows()
    {
        var document = new TranscriptDocument(Lines(2_000, 77));
        var rows = new TranscriptRowIndex(document, 89);

        Assert.Equal(0, rows.ChunkContainingRow(-50));
        Assert.Equal(document.ChunkCount - 1, rows.ChunkContainingRow(rows.TotalRows + 500));
        Assert.Equal(0, rows.RowsOf(-1));
        Assert.Equal(0, rows.RowsOf(document.ChunkCount));
    }

    // MARK: - Viewport arithmetic

    /// <summary>
    /// A scroll offset maps to the chunks that are actually at that height,
    /// checked against the row index rather than against itself.
    /// </summary>
    [Fact]
    public void ViewportMapsScrollOffsetToTheChunksAtThatHeight()
    {
        var document = new TranscriptDocument(Lines(20_000, 77));
        var rows = new TranscriptRowIndex(document, 89);
        const double RowHeight = 16.0;

        // Row 5,000 is 5,000 rows down; a 640 px viewport shows 40 rows.
        double offset = 5_000 * RowHeight;
        ChunkRange visible = TranscriptViewport.VisibleChunks(rows, offset, 640.0, RowHeight);

        Assert.Equal(rows.ChunkContainingRow(5_000), visible.Start);
        Assert.Equal(rows.ChunkContainingRow(5_039), visible.End - 1);
        Assert.True(visible.Count >= 1);

        // The top of the document, and past the bottom of it.
        Assert.Equal(0, TranscriptViewport.VisibleChunks(rows, 0.0, 640.0, RowHeight).Start);
        ChunkRange past = TranscriptViewport.VisibleChunks(
            rows,
            rows.TotalRows * RowHeight * 2,
            640.0,
            RowHeight);
        Assert.Equal(document.ChunkCount, past.End);
    }

    /// <summary>
    /// The spacers plus the resident window are the whole body's scroll
    /// extent. If they were not, the scrollbar would report the document as
    /// only as long as the window and the reader could not reach the rest --
    /// which is the clamp again, wearing a scrollbar.
    /// </summary>
    [Fact]
    public void SpacersPreserveTheWholeBodysScrollExtent()
    {
        var document = new TranscriptDocument(Body(4_000_000));
        var rows = new TranscriptRowIndex(document, 89);
        const double RowHeight = 16.0;

        foreach (int anchor in new[] { 0, 17, 400, document.ChunkCount - 1 })
        {
            ChunkRange window = TranscriptResidency.Window(
                document,
                new ChunkRange(anchor, anchor + 1));
            (double above, double below) = TranscriptViewport.Spacers(rows, window, RowHeight);

            double inside = Enumerable.Range(window.Start, window.Count)
                .Sum(i => rows.RowsOf(i) * RowHeight);

            Assert.Equal(rows.TotalRows * RowHeight, above + inside + below, 3);
            Assert.True(above >= 0.0);
            Assert.True(below >= 0.0);
        }
    }

    [Fact]
    public void ColumnsAreAtLeastOneAndScaleWithWidth()
    {
        Assert.Equal(1, TranscriptViewport.Columns(0.0));
        Assert.Equal(1, TranscriptViewport.Columns(-100.0));
        Assert.Equal(100, TranscriptViewport.Columns(660.0, 6.6));
    }

    // MARK: - Search snippets

    /// <summary>
    /// Snippets are cut from bytes at the offsets the daemon reports, and
    /// the text around the match is the text that is really there.
    /// </summary>
    [Fact]
    public void SnippetIsCutAtTheReportedByteOffset()
    {
        string body = new string('a', 1_000) + "NEEDLE" + new string('b', 1_000);
        var document = new TranscriptDocument(body);

        TranscriptSnippet snippet = document.Snippet(1_000, 6, 10);

        Assert.Equal(new string('a', 10) + "NEEDLE" + new string('b', 10), snippet.Text);
        Assert.True(snippet.ElidedBefore);
        Assert.True(snippet.ElidedAfter);
    }

    /// <summary>
    /// A snippet whose window would land inside a multi-byte character backs
    /// off to the character boundary rather than emitting U+FFFD.
    /// </summary>
    [Fact]
    public void SnippetEndsAreScalarAligned()
    {
        string body = string.Concat(Enumerable.Repeat("\U0001F642", 100))
            + "NEEDLE"
            + string.Concat(Enumerable.Repeat("\U0001F642", 100));
        var document = new TranscriptDocument(body);

        // 400 bytes of emoji before the needle. A 7-byte window from either
        // side lands mid-sequence: the leading cut backs off from 393 to
        // 392, taking two whole emoji rather than one and three quarters,
        // and the trailing cut backs off from 413 to 410, taking one.
        TranscriptSnippet snippet = document.Snippet(400, 6, 7);

        Assert.DoesNotContain('�', snippet.Text);
        Assert.Equal("\U0001F642\U0001F642NEEDLE\U0001F642", snippet.Text);
        Assert.True(snippet.ElidedBefore);
        Assert.True(snippet.ElidedAfter);
    }

    /// <summary>
    /// Snippets at the ends of the body report nothing elided, so the
    /// leading and trailing ellipsis are only drawn when text really is cut.
    /// </summary>
    [Fact]
    public void SnippetAtTheEdgesReportsNothingElided()
    {
        var document = new TranscriptDocument("NEEDLE in a short body");

        TranscriptSnippet snippet = document.Snippet(0, 6, 1_000);

        Assert.Equal("NEEDLE in a short body", snippet.Text);
        Assert.False(snippet.ElidedBefore);
        Assert.False(snippet.ElidedAfter);
    }

    // MARK: - Cost

    /// <summary>
    /// Chunking a 17.5 MB body is a scan, not a reflow. If this ever became
    /// slow it would be the hang again, moved one function along. The bound
    /// is loose because it runs on shared CI hardware; the failure it exists
    /// to catch is an accidental quadratic, which misses it by orders of
    /// magnitude rather than by a factor.
    /// </summary>
    [Fact]
    public void ChunkingSeventeenMegabytesIsAScan()
    {
        string body = Body(17_500_000);

        var clock = Stopwatch.StartNew();
        var document = new TranscriptDocument(body);
        clock.Stop();

        Assert.True(document.ChunkCount > 4_000);
        Assert.True(
            clock.Elapsed.TotalSeconds < 5.0,
            $"chunking took {clock.Elapsed.TotalSeconds:F3} s");
    }

    /// <summary>
    /// Moving the window is independent of how big the body is: the cost of
    /// a scroll must not grow with the trace.
    /// </summary>
    [Fact]
    public void WindowMoveDoesNotWalkTheBody()
    {
        var document = new TranscriptDocument(Body(17_500_000));

        var clock = Stopwatch.StartNew();
        for (int i = 0; i < 2_000; i++)
        {
            int anchor = (i * 37) % document.ChunkCount;
            TranscriptResidency.Window(document, new ChunkRange(anchor, anchor + 1));
        }

        clock.Stop();

        Assert.True(
            clock.Elapsed.TotalSeconds < 2.0,
            $"2,000 window moves took {clock.Elapsed.TotalSeconds:F3} s");
    }
}
