using System;
using System.Collections.Generic;
using System.Text;

namespace TraceCommons.Interop;

/// <summary>
/// How the transcript tab shows a whole body without laying all of it out.
///
/// The tab used to hand its whole body to one text run. On a real 17.5 MB
/// Claude Code session that pinned the macOS shell's main thread inside
/// CoreText at 197% CPU and 2.97 GB resident until the app had to be
/// force-quit. The first fix, in all three shells, was a 64 KB cap with a
/// notice saying the rest was not displayed. That bounded the damage at the
/// cost of making the tab's promise -- "exactly what would be sent" --
/// something the tab could no longer keep.
///
/// This is the cap moved rather than removed. Every byte is reachable; what
/// is bounded is how much text is <em>laid out and retained at once</em>.
/// The body is cut into chunks, only the chunks near the viewport are
/// typeset, and chunks that scroll away are evicted. Eviction is the
/// load-bearing half: a window that only ever adds chunks reaches the same
/// out-of-memory failure as the original, just further down the scrollbar.
///
/// The reference implementation is
/// <c>macos/Sources/TCShellCore/TranscriptPaging.swift</c>, and the design
/// with its measurements is
/// <c>docs/superpowers/specs/2026-08-20-chunked-transcript-design.md</c>.
///
/// <para>
/// <b>THE NUMBERS BELOW ARE INHERITED FROM macOS, NOT MEASURED ON WINDOWS.</b>
/// Every figure quoted in this file was taken on an M-series laptop against
/// SwiftUI <c>Text</c> / CoreText. WinUI's <c>RichTextBlock</c> is a
/// different text engine and nobody has run the layout curve against it:
/// the machine this was ported on cannot build the WinUI App project at
/// all. The <em>reasoning</em> is what ports -- chunk size = the largest
/// unit that still lays out inside one frame, retention = enough screenfuls
/// that a flick-scroll cannot outrun the window -- and the reasoning is
/// sound on any engine. The specific constants are a starting point that
/// needs measuring on a Windows box before anyone treats them as tuned. If
/// <c>RichTextBlock</c> turns out to be linear rather than quadratic in the
/// length of a run, <see cref="TargetChunkBytes"/> is free to be much
/// larger; if it is worse, it must be smaller. That is why they are four
/// named constants in one place rather than numbers spread through the
/// view.
/// </para>
/// </summary>
public static class TranscriptPaging
{
    /// <summary>
    /// Target chunk size, in bytes of UTF-8. <b>Inherited from macOS, not
    /// measured on Windows</b> -- see the note on <see cref="TranscriptPaging"/>.
    ///
    /// The macOS measurement it comes from: single-run layout there is
    /// quadratic in the length of the run, so laying out a body of <c>B</c>
    /// bytes in chunks of <c>c</c> costs <c>(B/c) * k*c^2 = k*B*c</c> --
    /// linear in the body and proportional to the chunk size, which means
    /// smaller chunks are strictly cheaper and the size is set by the
    /// smallest unit still worth being a view. One 4 KB chunk with
    /// redaction chips laid out in 0.0064 s there, inside a 60 Hz frame's
    /// 0.0167 s; 8 KB took 0.0252 s and dropped a frame and a half every
    /// time a chunk came into view.
    /// </summary>
    public const int TargetChunkBytes = 4 * 1024;

    /// <summary>
    /// The longest redaction marker the chunker will refuse to split.
    ///
    /// Markers are short by construction (<c>&lt;PRIVATE_SECRET_1&gt;</c>,
    /// <c>[REDACTED:aws_secret_key]</c>). This is the look-back window used
    /// when a cut has to land in the middle of a line; a marker longer than
    /// this is not protected from splitting, which is stated here rather
    /// than left to be discovered.
    /// </summary>
    public const int MaxMarkerBytes = 256;

    /// <summary>
    /// The hard ceiling on a single chunk, which the tests assert against.
    ///
    /// A chunk normally ends at a newline at or before the target. When a
    /// body has no newline to cut on -- minified JSON on one line -- the cut
    /// is taken at the target and then pushed off any redaction marker it
    /// landed inside, which can carry it up to <see cref="MaxMarkerBytes"/>
    /// further.
    /// </summary>
    public const int MaxChunkBytes = TargetChunkBytes + MaxMarkerBytes;

    /// <summary>
    /// The ceiling on text laid out and retained at once, in bytes of UTF-8.
    /// <b>Inherited from macOS, not measured on Windows</b> -- see the note
    /// on <see cref="TranscriptPaging"/>.
    ///
    /// This is the number that replaced <c>TranscriptBudget.LimitBytes</c>.
    /// It bounds glyph storage rather than what the reader can reach, and it
    /// is constant in the size of the body: a 17.5 MB trace retains exactly
    /// as much as a 200 KB one.
    ///
    /// Sized from the viewport on macOS, where a screenful of 13pt
    /// monospaced transcript measured between 1.7 KB (small sheet) and
    /// 6.9 KB (full-height display). 128 KB is at least 18 screenfuls even
    /// at the large end: the visible page plus roughly nine screenfuls of
    /// overscan in each direction, which is what keeps a flick-scroll from
    /// outrunning the window and showing blank space. The Windows sheet's
    /// font is 11px rather than 13pt, so its screenful is smaller and this
    /// buys <em>more</em> overscan here, not less -- which is the one thing
    /// about this constant that transfers without measurement.
    /// </summary>
    public const int RetainedLimitBytes = 128 * 1024;

    /// <summary>
    /// Assumed advance width, in effective pixels, of one character of the
    /// transcript's monospaced font at
    /// <c>TcMonoTranscriptFontSize</c> (11). <b>Not measured.</b>
    ///
    /// Used only to estimate how many rows a chunk occupies while it is not
    /// typeset, so a non-resident chunk holds its place in the scroll.
    ///
    /// Derived, not measured: <c>TcMonoFontFamily</c> is Consolas, whose
    /// advance width is 1126/2048 em, and 11 px of that is 6.048. It is
    /// wrong by whatever font substitution or DPI rounding does to that on a
    /// real machine. The error shows up as the scrollbar settling slightly
    /// as chunks materialise, never as text that cannot be reached: the
    /// estimate only sizes placeholders.
    /// </summary>
    public const double EstimatedColumnWidth = 6.05;

    /// <summary>
    /// Assumed height, in effective pixels, of one display row of the
    /// transcript's monospaced font at 11 px. <b>Not measured.</b> Same role
    /// and same caveat as <see cref="EstimatedColumnWidth"/>: 16 px is
    /// roughly Consolas' 1.17 em line box at 11 px rounded up to a whole
    /// pixel, and a Windows box is the only place to find out what it really
    /// is.
    /// </summary>
    public const double EstimatedRowHeight = 16.0;
}

/// <summary>
/// A half-open range of chunk indices, <c>[Start, End)</c>. C# has no
/// <c>Range&lt;Int&gt;</c> the way Swift does, and <see cref="System.Range"/>
/// carries from-the-end semantics this has no use for.
/// </summary>
public readonly record struct ChunkRange(int Start, int End)
{
    public static readonly ChunkRange Empty = new(0, 0);

    public int Count => Math.Max(0, End - Start);

    public bool IsEmpty => Count == 0;

    public bool Contains(int index) => index >= Start && index < End;
}

/// <summary>
/// One unit of layout: a byte range of the body that can be typeset on its
/// own without splitting a character or a redaction marker.
/// </summary>
/// <param name="ByteOffset">UTF-8 byte offset of the chunk's first byte.</param>
/// <param name="ByteCount">Length of the chunk in UTF-8 bytes.</param>
/// <param name="LineCount">
/// Newlines in the chunk, used to place the chunk's stand-in while it is not
/// resident. A chunk that ends at a newline counts that newline, so the
/// count is the number of display rows when no line wraps.
/// </param>
public readonly record struct TranscriptChunk(int ByteOffset, int ByteCount, int LineCount)
{
    public int ByteEnd => ByteOffset + ByteCount;
}

/// <summary>
/// A body cut into chunks, holding its own bytes so that slicing a chunk or
/// a search snippet is an array slice rather than a walk.
///
/// The byte array is a second copy of the body -- 17.5 MB for the trace that
/// started this -- held for the life of the sheet. That is the trade for
/// never having to re-encode the body to answer "what are the bytes at this
/// offset", which is a question both the chunker and the search tab ask.
/// </summary>
public sealed class TranscriptDocument
{
    private readonly byte[] bytes;

    /// <summary>
    /// Cuts the body once, when it arrives. This is a scan, not a layout:
    /// measured at 0.0064 s for 17.5 MB on the macOS reference. If it ever
    /// became slow it would be the original hang, moved one function along,
    /// which is why there is a test with a wall-clock bound on it.
    /// </summary>
    public TranscriptDocument(string body)
    {
        ArgumentNullException.ThrowIfNull(body);
        bytes = Encoding.UTF8.GetBytes(body);
        Chunks = Cut(bytes);
    }

    /// <summary>The chunks, in order, tiling the body exactly.</summary>
    public IReadOnlyList<TranscriptChunk> Chunks { get; }

    public int ChunkCount => Chunks.Count;

    public int TotalBytes => bytes.Length;

    /// <summary>
    /// The text of one chunk. Always valid UTF-8, and therefore always a
    /// whole number of UTF-16 code points: the cut is scalar-aligned, so a
    /// four-byte character -- one UTF-8 sequence and one UTF-16 surrogate
    /// pair -- is either wholly in this chunk or wholly in the next.
    /// </summary>
    public string TextOf(int index)
    {
        if (index < 0 || index >= Chunks.Count)
        {
            return string.Empty;
        }

        TranscriptChunk chunk = Chunks[index];
        return Encoding.UTF8.GetString(bytes, chunk.ByteOffset, chunk.ByteCount);
    }

    /// <summary>
    /// The whole body, decoded back out of the bytes.
    ///
    /// Building a 17.5 MB string is a copy, not a layout, so this is cheap
    /// in the way that matters: it is what "Copy everything" hands to the
    /// clipboard. Nothing lays it out.
    /// </summary>
    public string WholeText() => Encoding.UTF8.GetString(bytes);

    /// <summary>
    /// A window of context around a UTF-8 byte offset.
    ///
    /// Both cut ends back off any continuation byte, so a snippet never
    /// opens or closes with U+FFFD, and it reports whether it elided text on
    /// each side rather than leaving the caller to guess where the ellipses
    /// go.
    /// </summary>
    public TranscriptSnippet Snippet(int byteOffset, int matchBytes, int window)
    {
        if (byteOffset < 0 || byteOffset > bytes.Length)
        {
            return new TranscriptSnippet(string.Empty, false, false);
        }

        int start = Math.Max(0, byteOffset - window);
        int end = Math.Min(bytes.Length, byteOffset + Math.Max(0, matchBytes) + window);
        while (start > 0 && IsContinuation(bytes[start]))
        {
            start -= 1;
        }

        while (end < bytes.Length && IsContinuation(bytes[end]))
        {
            end -= 1;
        }

        if (start >= end)
        {
            return new TranscriptSnippet(string.Empty, false, false);
        }

        return new TranscriptSnippet(
            Encoding.UTF8.GetString(bytes, start, end - start),
            start > 0,
            end < bytes.Length);
    }

    private static bool IsContinuation(byte value) => (value & 0xC0) == 0x80;

    /// <summary>
    /// Cuts the body into chunks. Rules, in order of preference:
    ///
    /// <list type="number">
    /// <item>End at the last newline at or before the target, provided that
    /// leaves a chunk of at least half the target. A whole number of lines
    /// is what a reader expects, and a newline can never be inside a
    /// redaction marker, so this path is safe by construction. Essentially
    /// every real transcript takes it.</item>
    /// <item>Otherwise cut at the target, then push the cut off any marker
    /// it landed inside -- back to the marker's start if that leaves a
    /// non-empty chunk, forward past its end if it did not.</item>
    /// <item>Then back the cut off any UTF-8 continuation byte, so the chunk
    /// ends on a scalar boundary.</item>
    /// </list>
    /// </summary>
    private static List<TranscriptChunk> Cut(byte[] bytes)
    {
        var chunks = new List<TranscriptChunk>();
        if (bytes.Length == 0)
        {
            return chunks;
        }

        const int Target = TranscriptPaging.TargetChunkBytes;
        const int Minimum = Target / 2;
        chunks.Capacity = (bytes.Length / Target) + 1;

        int start = 0;
        while (start < bytes.Length)
        {
            int cut = Math.Min(start + Target, bytes.Length);
            if (cut < bytes.Length)
            {
                int newline = LastIndexOfNewline(bytes, start, cut);
                if (newline >= 0 && newline + 1 - start >= Minimum)
                {
                    cut = newline + 1;
                }
                else
                {
                    cut = PushOffMarker(bytes, cut, start);
                    while (cut > start + 1 && cut < bytes.Length && IsContinuation(bytes[cut]))
                    {
                        cut -= 1;
                    }
                }
            }

            int lines = 0;
            for (int i = start; i < cut; i++)
            {
                if (bytes[i] == 0x0A)
                {
                    lines += 1;
                }
            }

            chunks.Add(new TranscriptChunk(start, cut - start, lines));
            start = cut;
        }

        return chunks;
    }

    private static int LastIndexOfNewline(byte[] bytes, int start, int endExclusive)
    {
        for (int i = endExclusive - 1; i >= start; i--)
        {
            if (bytes[i] == 0x0A)
            {
                return i;
            }
        }

        return -1;
    }

    /// <summary>
    /// Moves <paramref name="cut"/> out of the middle of a redaction marker,
    /// if it is in one.
    ///
    /// A marker rendered as two halves in two separately-typeset chunks --
    /// <c>&lt;PRIVATE_SEC</c> in one and <c>RET_1&gt;</c> in the next -- is
    /// not a cosmetic problem. The chips are how a contributor sees
    /// <em>where</em> scrubbing fired, and half a marker in body type reads
    /// as content that was never scrubbed.
    /// </summary>
    private static int PushOffMarker(byte[] bytes, int cut, int start)
    {
        int lookBack = Math.Max(start, cut - TranscriptPaging.MaxMarkerBytes);
        if (lookBack >= cut)
        {
            return cut;
        }

        int lookAhead = Math.Min(bytes.Length, cut + TranscriptPaging.MaxMarkerBytes);

        // Decode the window on scalar boundaries. A window edge that lands
        // mid-sequence would decode to U+FFFD and shift every byte offset
        // the scan reports after it.
        while (lookBack > start && IsContinuation(bytes[lookBack]))
        {
            lookBack -= 1;
        }

        while (lookAhead < bytes.Length && IsContinuation(bytes[lookAhead]))
        {
            lookAhead -= 1;
        }

        if (lookBack >= lookAhead)
        {
            return cut;
        }

        string text = Encoding.UTF8.GetString(bytes, lookBack, lookAhead - lookBack);
        foreach ((int Start, int End) span in TranscriptMarkers.ByteSpans(text))
        {
            int markerStart = lookBack + span.Start;
            int markerEnd = lookBack + span.End;
            if (markerStart >= cut || cut >= markerEnd)
            {
                continue;
            }

            return markerStart > start ? markerStart : Math.Min(markerEnd, bytes.Length);
        }

        return cut;
    }
}

/// <summary>A window of context around a search hit.</summary>
/// <param name="Text">The context, always whole characters.</param>
/// <param name="ElidedBefore">True when text was cut from the front.</param>
/// <param name="ElidedAfter">True when text was cut from the end.</param>
public readonly record struct TranscriptSnippet(string Text, bool ElidedBefore, bool ElidedAfter);

/// <summary>
/// Where each chunk sits vertically, so a chunk that is not resident can
/// still hold its place in the scroll.
///
/// Rows are estimated from bytes and newlines rather than measured, because
/// measuring is the thing this whole design exists to avoid. In a monospaced
/// font the estimate is exact for any chunk whose lines all fit the width.
/// It is high by at most one row per chunk when lines wrap and low by at
/// most one row per line for a chunk mixing wrapped and short lines. Either
/// way the error is bounded per chunk, not per body, and shows up as the
/// scrollbar settling slightly as chunks materialise rather than as a body
/// whose length is unknown.
/// </summary>
public sealed class TranscriptRowIndex
{
    private readonly int[] rowStarts;

    public TranscriptRowIndex(TranscriptDocument document, int columns)
    {
        ArgumentNullException.ThrowIfNull(document);

        Columns = Math.Max(1, columns);
        rowStarts = new int[document.ChunkCount + 1];
        int running = 0;
        for (int i = 0; i < document.ChunkCount; i++)
        {
            running += RowsOf(document.Chunks[i], Columns);
            rowStarts[i + 1] = running;
        }
    }

    /// <summary>Characters per display row at the current width. Never less than 1.</summary>
    public int Columns { get; }

    /// <summary>Cumulative row offsets, <c>ChunkCount + 1</c> entries.</summary>
    public IReadOnlyList<int> RowStarts => rowStarts;

    public int TotalRows => rowStarts.Length == 0 ? 0 : rowStarts[^1];

    public static int RowsOf(TranscriptChunk chunk, int columns)
    {
        int width = Math.Max(1, columns);
        int wrapped = (chunk.ByteCount + width - 1) / width;
        return Math.Max(1, Math.Max(chunk.LineCount, wrapped));
    }

    public int RowsOf(int index)
    {
        if (index < 0 || index + 1 >= rowStarts.Length)
        {
            return 0;
        }

        return rowStarts[index + 1] - rowStarts[index];
    }

    /// <summary>The first row of a chunk.</summary>
    public int RowStartOf(int index)
    {
        if (rowStarts.Length == 0)
        {
            return 0;
        }

        return rowStarts[Math.Clamp(index, 0, rowStarts.Length - 1)];
    }

    /// <summary>The chunk containing a display row, clamped to the document.</summary>
    public int ChunkContainingRow(int row)
    {
        if (rowStarts.Length <= 1)
        {
            return 0;
        }

        int target = Math.Clamp(row, 0, Math.Max(0, TotalRows - 1));
        int low = 0;
        int high = rowStarts.Length - 2;
        while (low < high)
        {
            int mid = (low + high + 1) / 2;
            if (rowStarts[mid] <= target)
            {
                low = mid;
            }
            else
            {
                high = mid - 1;
            }
        }

        return low;
    }
}

/// <summary>Which chunks are laid out right now, and which have been let go.</summary>
public static class TranscriptResidency
{
    /// <summary>
    /// The chunks to keep typeset, given what is on screen.
    ///
    /// The visible range comes first and is never dropped for overscan; if
    /// the visible range alone somehow exceeded the ceiling it is trimmed
    /// from its far end, so the returned window is under the ceiling
    /// unconditionally. That trim is not expected to fire -- the ceiling is
    /// 128 KB and the largest measured screenful is single-digit KB -- but
    /// "not expected" is not a bound.
    ///
    /// Overscan is then added one chunk at a time, alternating below and
    /// above, so a reader scrolling in either direction has the same amount
    /// of already-typeset text ahead of them. At the ends of the body the
    /// budget is spent entirely on the side that exists.
    /// </summary>
    public static ChunkRange Window(
        TranscriptDocument document,
        ChunkRange visible,
        int limitBytes = TranscriptPaging.RetainedLimitBytes)
    {
        ArgumentNullException.ThrowIfNull(document);

        int count = document.ChunkCount;
        if (count == 0)
        {
            return ChunkRange.Empty;
        }

        int lower = Math.Clamp(visible.Start, 0, count - 1);
        int upper = Math.Clamp(visible.End, lower + 1, count);

        int bytes = 0;
        for (int i = lower; i < upper; i++)
        {
            bytes += document.Chunks[i].ByteCount;
        }

        while (upper - lower > 1 && bytes > limitBytes)
        {
            upper -= 1;
            bytes -= document.Chunks[upper].ByteCount;
        }

        bool growDown = true;
        while (true)
        {
            bool canDown = lower > 0
                && bytes + document.Chunks[lower - 1].ByteCount <= limitBytes;
            bool canUp = upper < count
                && bytes + document.Chunks[upper].ByteCount <= limitBytes;
            if (!canDown && !canUp)
            {
                break;
            }

            if (growDown && canDown)
            {
                lower -= 1;
                bytes += document.Chunks[lower].ByteCount;
            }
            else if (canUp)
            {
                bytes += document.Chunks[upper].ByteCount;
                upper += 1;
            }
            else
            {
                lower -= 1;
                bytes += document.Chunks[lower].ByteCount;
            }

            growDown = !growDown;
        }

        return new ChunkRange(lower, upper);
    }
}

/// <summary>
/// The typeset chunks the view is holding, and the eviction that keeps that
/// set bounded.
///
/// Generic over what a chunk is rendered into so the policy can be tested
/// for what it is -- an accounting rule over byte counts -- without a view,
/// a font, or a running app. The view instantiates it with the WinUI
/// <c>Paragraph</c> it builds per chunk; the tests instantiate it with
/// <see cref="string"/> and assert the same <see cref="RetainedBytes"/> the
/// view is subject to.
/// </summary>
public sealed class TranscriptResidentChunks<TRendered>
{
    private readonly Dictionary<int, TRendered> rendered = new();

    public ChunkRange Window { get; private set; } = ChunkRange.Empty;

    public IReadOnlyDictionary<int, TRendered> Rendered => rendered;

    /// <summary>UTF-8 bytes of body currently typeset. The number the ceiling is on.</summary>
    public int RetainedBytes { get; private set; }

    /// <summary>
    /// How many chunks have been evicted since this set was created. Exists
    /// so a test can prove eviction happened rather than infer it from a
    /// count that merely stopped growing.
    /// </summary>
    public int Evictions { get; private set; }

    /// <summary>How many chunks have been rendered since this set was created.</summary>
    public int Renders { get; private set; }

    public int ResidentCount => rendered.Count;

    public bool TryGet(int index, out TRendered value) => rendered.TryGetValue(index, out value!);

    /// <summary>
    /// Moves the window to cover <paramref name="visible"/>, typesetting what
    /// came into it and dropping what fell out.
    ///
    /// <paramref name="make"/> is called only for chunks that are not already
    /// rendered, so a scroll of one chunk costs one chunk of layout, not a
    /// window's worth.
    /// </summary>
    public void Update(
        TranscriptDocument document,
        ChunkRange visible,
        Func<int, TRendered> make,
        int limitBytes = TranscriptPaging.RetainedLimitBytes)
    {
        ArgumentNullException.ThrowIfNull(document);
        ArgumentNullException.ThrowIfNull(make);

        ChunkRange next = TranscriptResidency.Window(document, visible, limitBytes);
        if (next == Window && rendered.Count == next.Count)
        {
            return;
        }

        var dropped = new List<int>();
        foreach (int index in rendered.Keys)
        {
            if (!next.Contains(index))
            {
                dropped.Add(index);
            }
        }

        foreach (int index in dropped)
        {
            rendered.Remove(index);
            RetainedBytes -= document.Chunks[index].ByteCount;
            Evictions += 1;
        }

        for (int index = next.Start; index < next.End; index++)
        {
            if (rendered.ContainsKey(index))
            {
                continue;
            }

            rendered[index] = make(index);
            RetainedBytes += document.Chunks[index].ByteCount;
            Renders += 1;
        }

        Window = next;
    }
}

/// <summary>
/// The arithmetic that turns a <c>ScrollViewer</c>'s offset into a range of
/// chunks, and a resident window back into the two spacer heights that keep
/// the scroll extent the whole body's.
///
/// The Windows shell cannot use the macOS shape -- one lazily-built view per
/// chunk -- without putting 4,480 elements in a non-virtualising panel for a
/// 17.5 MB body. Instead the panel holds a top spacer, the resident chunks,
/// and a bottom spacer: the element count is bounded by the retention
/// ceiling regardless of how large the trace is, and the spacers stand in
/// for everything not typeset.
///
/// This is here rather than in the App project for the same reason the rest
/// of the paging is: it is arithmetic, and the App project does not build on
/// the machines these tests run on.
/// </summary>
public static class TranscriptViewport
{
    /// <summary>
    /// The chunks intersecting the viewport, given where the scroll is.
    ///
    /// Row heights are the estimate, not a measurement, so this is
    /// approximate by exactly as much as <see cref="TranscriptRowIndex"/>
    /// is. It does not need to be exact: it feeds
    /// <see cref="TranscriptResidency.Window"/>, whose overscan is several
    /// screenfuls in each direction.
    /// </summary>
    public static ChunkRange VisibleChunks(
        TranscriptRowIndex rows,
        double verticalOffset,
        double viewportHeight,
        double rowHeight = TranscriptPaging.EstimatedRowHeight)
    {
        ArgumentNullException.ThrowIfNull(rows);

        if (rows.TotalRows == 0)
        {
            return ChunkRange.Empty;
        }

        double height = Math.Max(1.0, rowHeight);
        double top = Math.Max(0.0, verticalOffset);
        double bottom = top + Math.Max(height, viewportHeight);

        int firstRow = (int)Math.Floor(top / height);
        int lastRow = (int)Math.Ceiling(bottom / height) - 1;

        int first = rows.ChunkContainingRow(firstRow);
        int last = rows.ChunkContainingRow(lastRow);
        return new ChunkRange(first, Math.Max(first + 1, last + 1));
    }

    /// <summary>
    /// The heights of the two spacers that hold the place of everything
    /// outside <paramref name="window"/>.
    ///
    /// Their sum plus the resident chunks' own heights is the whole body's
    /// scroll extent, which is what stops the scrollbar from claiming the
    /// document is only as long as the window.
    /// </summary>
    public static (double Above, double Below) Spacers(
        TranscriptRowIndex rows,
        ChunkRange window,
        double rowHeight = TranscriptPaging.EstimatedRowHeight)
    {
        ArgumentNullException.ThrowIfNull(rows);

        if (window.IsEmpty)
        {
            return (0.0, rows.TotalRows * rowHeight);
        }

        double above = rows.RowStartOf(window.Start) * rowHeight;
        double below = (rows.TotalRows - rows.RowStartOf(window.End)) * rowHeight;
        return (Math.Max(0.0, above), Math.Max(0.0, below));
    }

    /// <summary>
    /// Characters that fit across <paramref name="usableWidth"/> effective
    /// pixels, for <see cref="TranscriptRowIndex"/>. Never less than 1.
    /// </summary>
    public static int Columns(
        double usableWidth,
        double columnWidth = TranscriptPaging.EstimatedColumnWidth)
    {
        if (columnWidth <= 0 || double.IsNaN(usableWidth) || usableWidth <= 0)
        {
            return 1;
        }

        return Math.Max(1, (int)(usableWidth / columnWidth));
    }
}
