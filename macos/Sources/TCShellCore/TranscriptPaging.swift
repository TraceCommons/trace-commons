import Foundation

/// How the transcript tab shows a whole body without laying all of it out.
///
/// The tab used to hand its whole body to one text run. On a real 17.5 MB
/// Claude Code session that pinned the main thread inside CoreText at 197%
/// CPU and 2.97 GB resident until the app had to be force-quit; every
/// main-thread sample landed in
/// `__NSCoreTypesetterCreateBaseLineFromAttributedString`. The first fix was
/// a 64 KB cap with a notice saying the rest was not displayed. That bounded
/// the damage at the cost of making the tab's promise -- "exactly what would
/// be sent" -- something the tab could no longer keep.
///
/// This is the cap moved rather than removed. Every byte is reachable; what
/// is bounded is how much text is *laid out and retained at once*. The body
/// is cut into chunks, only the chunks near the viewport are typeset, and
/// chunks that scroll away are evicted. Eviction is the load-bearing half: a
/// window that only ever adds chunks reaches the same out-of-memory failure
/// as the original, just further down the scrollbar.
///
/// ## Why layout has to be chunked at all
///
/// SwiftUI's `Text` sizes an attributed string through
/// `NSAttributedString.boundingRect(with:options:.usesLineFragmentOrigin)`,
/// and that call is quadratic in the length of the string. Measured on an
/// M-series laptop, 13pt monospaced, 720pt wide, transcript-shaped lines:
///
///     size     plain      with redaction chips
///       4 KB   0.005 s    0.005 s
///       8 KB   0.007 s    0.018 s
///      16 KB   0.020 s    0.055 s
///      32 KB   0.088 s    0.234 s
///      64 KB   0.379 s    1.138 s
///     128 KB   1.727 s    4.882 s
///
/// Each doubling costs about four times as much, and the chip attributes
/// that mark where scrubbing fired roughly triple the constant. Extrapolated
/// to 17.5 MB that is hours, which is exactly the observed "window never
/// comes back".
///
/// The consequence for chunking is the whole design: laying out a body of
/// `B` bytes in chunks of `c` costs `(B/c) * k*c^2 = k*B*c`. Linear in the
/// body, and *proportional to the chunk size*. Smaller chunks are strictly
/// cheaper, so the chunk size is set by the smallest unit that is still
/// worth being a view, not by how much text looks reasonable.
public enum TranscriptPaging {
    /// Target chunk size, in bytes of UTF-8.
    ///
    /// Measured cost of laying out one chunk with chips, and of refilling a
    /// full 128 KB resident window from cold, at each candidate size:
    ///
    ///     chunk    per chunk   full window
    ///       2 KB    0.0018 s     0.114 s
    ///       4 KB    0.0064 s     0.221 s
    ///       8 KB    0.0252 s     0.383 s
    ///      16 KB    0.0690 s     0.547 s
    ///      32 KB    0.2202 s     0.980 s
    ///
    /// A 60 Hz frame is 0.0167 s. 4 KB is the largest chunk whose layout
    /// still fits inside one frame, so materialising a chunk during a scroll
    /// costs at most a frame of jitter rather than a visible stall; 8 KB
    /// drops a frame and a half on every chunk that comes into view. 2 KB is
    /// cheaper still but doubles the view count -- 8,960 rows for a 17.5 MB
    /// body against 4,480 -- for a difference nobody can perceive.
    public static let targetChunkBytes = 4 * 1024

    /// The hard ceiling on a single chunk.
    ///
    /// A chunk normally ends at a newline at or before the target. When a
    /// body has no newline to cut on -- minified JSON on one line -- the cut
    /// is taken at the target and then pushed off any redaction marker it
    /// landed inside, which can carry it up to `maxMarkerBytes` further. The
    /// ceiling is what the tests assert against, so it is written down
    /// rather than implied.
    public static let maxChunkBytes = targetChunkBytes + maxMarkerBytes

    /// The ceiling on text laid out and retained at once, in bytes of UTF-8.
    ///
    /// This is the number that replaces `TranscriptBudget.limitBytes`. It
    /// bounds glyph storage rather than what the reader can reach, and it is
    /// constant in the size of the body: a 17.5 MB trace retains exactly as
    /// much as a 200 KB one.
    ///
    /// Sized from the viewport, measured rather than guessed. At 13pt
    /// monospaced the advance is 8.036 pt and the line box is 16 pt, so a
    /// screenful of transcript is:
    ///
    ///     640 x 420 pt sheet    79 cols x 22 rows   1.7 KB
    ///     720 x 560 pt sheet    89 cols x 29 rows   2.5 KB
    ///    1000 x 1100 pt sheet  124 cols x 57 rows   6.9 KB
    ///
    /// 128 KB is at least 18 screenfuls even on a full-height display: the
    /// visible page plus roughly nine screenfuls of overscan in each
    /// direction, which is what keeps a flick-scroll from outrunning the
    /// window and showing blank space. Refilling all of it from cold costs
    /// the measured 0.221 s, and that only happens on a jump, never on a
    /// drag.
    ///
    /// In memory: the incident measured 2.97 GB resident for a live 17.5 MB
    /// single run, about 170 bytes of process memory per source byte. At
    /// that ratio 128 KB of resident text is roughly 22 MB -- and stays
    /// there, because the window evicts.
    public static let retainedLimitBytes = 128 * 1024

    /// The longest redaction marker the chunker will refuse to split.
    ///
    /// Markers are short by construction (`<PRIVATE_SECRET_1>`,
    /// `[REDACTED:aws_secret_key]`). This is the look-back window used when
    /// a cut has to land in the middle of a line; a marker longer than this
    /// is not protected, which is stated here rather than left to be
    /// discovered.
    public static let maxMarkerBytes = 256
}

/// A body cut into chunks, holding its own bytes so that slicing a chunk or
/// a search snippet is an array slice rather than a walk.
///
/// The byte array is a second copy of the body -- 17.5 MB for the trace that
/// started this -- and it is deliberate. It replaces the copy the search tab
/// used to make of the entire body on *every keystroke* (`Array(transcript.utf8)`
/// inside a computed property, re-evaluated per character typed), so the
/// steady-state cost goes from one full copy per keystroke to one for the
/// lifetime of the sheet.
public struct TranscriptDocument: Sendable {
    /// One unit of layout: a byte range of the body that can be typeset on
    /// its own without splitting a character or a redaction marker.
    public struct Chunk: Equatable, Sendable {
        public let byteOffset: Int
        public let byteCount: Int
        /// Newlines in the chunk, used to place the chunk's stand-in while
        /// it is not resident. A chunk that ends at a newline counts that
        /// newline; the count is therefore the number of display rows when
        /// no line wraps.
        public let lineCount: Int

        public var byteRange: Range<Int> { byteOffset..<(byteOffset + byteCount) }
    }

    private let bytes: [UInt8]
    public let chunks: [Chunk]

    public var totalBytes: Int { bytes.count }
    public var chunkCount: Int { chunks.count }

    /// Cuts the body once, when it arrives.
    ///
    /// Measured on a 17.5 MB body: 0.0064 s in a release build, 0.663 s in a
    /// debug one. At 6 ms this does not need to leave the main actor in a
    /// shipping build; the debug figure is why the test's bound is 2 s
    /// rather than 50 ms.
    public init(_ body: String) {
        self.bytes = Array(body.utf8)
        self.chunks = TranscriptDocument.cut(bytes)
    }

    /// The text of one chunk. Always valid UTF-8: the cut is scalar-aligned.
    public func text(of index: Int) -> String {
        guard chunks.indices.contains(index) else { return "" }
        let chunk = chunks[index]
        return String(decoding: bytes[chunk.byteRange], as: UTF8.self)
    }

    /// The whole body, decoded back out of the bytes.
    ///
    /// Building a 17.5 MB string is a copy, not a layout, so this is cheap
    /// in the way that matters -- it is what "Copy everything" hands to the
    /// pasteboard. Nothing lays it out.
    public func wholeText() -> String {
        String(decoding: bytes, as: UTF8.self)
    }

    /// A window of context around a byte offset, for the search tab.
    ///
    /// The FFI reports UTF-8 *byte* offsets, so this cuts from the byte
    /// array and decodes back. The cut ends are scalar-aligned so a snippet
    /// never opens or closes with a replacement character.
    public func snippet(around byteOffset: Int, matchBytes: Int, window: Int) -> (
        text: String, elidedBefore: Bool, elidedAfter: Bool
    ) {
        guard byteOffset >= 0, byteOffset <= bytes.count else { return ("", false, false) }
        var start = max(0, byteOffset - window)
        var end = min(bytes.count, byteOffset + matchBytes + window)
        while start > 0, TranscriptDocument.isContinuation(bytes[start]) { start -= 1 }
        while end < bytes.count, TranscriptDocument.isContinuation(bytes[end]) { end -= 1 }
        guard start < end else { return ("", false, false) }
        return (
            String(decoding: bytes[start..<end], as: UTF8.self),
            start > 0,
            end < bytes.count
        )
    }

    // MARK: - Cutting

    private static func isContinuation(_ byte: UInt8) -> Bool { byte & 0xC0 == 0x80 }

    /// Cuts the body into chunks.
    ///
    /// Rules, in order of preference:
    ///
    /// 1. End at the last newline at or before the target, provided that
    ///    leaves a chunk of at least half the target. A whole number of
    ///    lines is what a reader expects, and a newline can never be inside
    ///    a redaction marker, so this path is safe by construction.
    /// 2. Otherwise cut at the target, then push the cut off any marker it
    ///    landed inside -- back to the marker's start if that leaves a
    ///    non-empty chunk, forward past its end if it did not.
    /// 3. Then back the cut off any UTF-8 continuation byte, so the chunk
    ///    ends on a scalar boundary.
    private static func cut(_ bytes: [UInt8]) -> [Chunk] {
        guard !bytes.isEmpty else { return [] }
        let target = TranscriptPaging.targetChunkBytes
        let minimum = target / 2
        var chunks: [Chunk] = []
        chunks.reserveCapacity(bytes.count / target + 1)

        var start = 0
        while start < bytes.count {
            var cut = min(start + target, bytes.count)
            if cut < bytes.count {
                if let newline = bytes[start..<cut].lastIndex(of: 0x0A),
                    newline + 1 - start >= minimum
                {
                    cut = newline + 1
                } else {
                    cut = pushOffMarker(bytes, cut: cut, start: start)
                    while cut > start + 1, isContinuation(bytes[cut]) { cut -= 1 }
                }
            }
            let range = start..<cut
            chunks.append(
                Chunk(
                    byteOffset: start,
                    byteCount: cut - start,
                    lineCount: bytes[range].reduce(into: 0) { $0 += ($1 == 0x0A ? 1 : 0) }
                )
            )
            start = cut
        }
        return chunks
    }

    /// Moves `cut` out of the middle of a redaction marker, if it is in one.
    ///
    /// A marker rendered as two halves in two separately-typeset chunks --
    /// `<PRIVATE_SEC` in one and `RET_1>` in the next -- is not a cosmetic
    /// problem. The chips are how a contributor sees *where* scrubbing
    /// fired, and half a marker in body type reads as content that was not
    /// scrubbed.
    private static func pushOffMarker(_ bytes: [UInt8], cut: Int, start: Int) -> Int {
        let lookBack = max(start, cut - TranscriptPaging.maxMarkerBytes)
        guard lookBack < cut else { return cut }
        let lookAhead = min(bytes.count, cut + TranscriptPaging.maxMarkerBytes)
        let text = String(decoding: bytes[lookBack..<lookAhead], as: UTF8.self)
        for span in TranscriptMarkerScan.byteSpans(in: text) {
            let markerStart = lookBack + span.lowerBound
            let markerEnd = lookBack + span.upperBound
            guard markerStart < cut, cut < markerEnd else { continue }
            return markerStart > start ? markerStart : min(markerEnd, bytes.count)
        }
        return cut
    }
}

/// Finds the redaction markers the scrubbing pipeline leaves behind.
///
/// Shared by the chunker, which must not cut through one, and by the view,
/// which draws each one as a chip. One pattern, one place: a chunker that
/// protected a different set of markers than the view highlights would split
/// exactly the ones the view cares about.
public enum TranscriptMarkerScan {
    /// Both marker families the pipeline emits, including the
    /// `[REDACTED:aws_secret_key]` form that carries a category label.
    ///
    /// The `[REDACTED...]` arm excludes newlines as well as `]`. Without
    /// that, an unclosed bracket anywhere in the body would let a "marker"
    /// run to the end of the file, and the chunker would then refuse to cut
    /// there.
    public static let pattern = "<PRIVATE_[A-Za-z0-9_]+>|\\[REDACTED[^\\]\\n]*\\]"

    private static let regex = try? NSRegularExpression(pattern: pattern)

    /// Marker ranges in `text`, as string ranges, in order.
    public static func spans(in text: String) -> [Range<String.Index>] {
        guard let regex else { return [] }
        let whole = NSRange(text.startIndex..<text.endIndex, in: text)
        return regex.matches(in: text, range: whole).compactMap { Range($0.range, in: text) }
    }

    /// Marker ranges in `text`, as UTF-8 byte offsets from its start.
    public static func byteSpans(in text: String) -> [Range<Int>] {
        spans(in: text).map { span in
            let lower = text.utf8.distance(from: text.startIndex, to: span.lowerBound)
            let upper = text.utf8.distance(from: text.startIndex, to: span.upperBound)
            return lower..<upper
        }
    }
}

/// Where each chunk sits vertically, so a chunk that is not resident can
/// still hold its place in the scroll.
///
/// Rows are estimated from bytes and newlines rather than measured, because
/// measuring is the thing this whole design exists to avoid. In a monospaced
/// font the estimate is exact for any chunk whose lines all fit the width.
/// It is high by at most one row per chunk for a wrapped chunk -- the
/// wrapped count is rounded up at the chunk edge, so an unbroken 8,900-byte
/// line at 89 columns estimates 102 rows against a true 100 -- and low by at
/// most one row per line for a chunk that mixes wrapped and short lines.
/// Either way the error is bounded per chunk, not per body: at most 16 pt of
/// scroll extent per 4 KB, which shows up as the scrollbar settling slightly
/// as chunks materialise rather than as a body whose length is unknown.
public struct TranscriptRowIndex: Sendable {
    /// Characters per display row at the current width. Never less than 1.
    public let columns: Int
    /// Cumulative row offsets, `chunkCount + 1` entries.
    public let rowStarts: [Int]

    public var totalRows: Int { rowStarts.last ?? 0 }

    public init(_ document: TranscriptDocument, columns: Int) {
        let columns = max(1, columns)
        self.columns = columns
        var starts: [Int] = [0]
        starts.reserveCapacity(document.chunkCount + 1)
        var running = 0
        for chunk in document.chunks {
            running += TranscriptRowIndex.rows(of: chunk, columns: columns)
            starts.append(running)
        }
        self.rowStarts = starts
    }

    public static func rows(of chunk: TranscriptDocument.Chunk, columns: Int) -> Int {
        let wrapped = (chunk.byteCount + columns - 1) / max(1, columns)
        return max(1, max(chunk.lineCount, wrapped))
    }

    public func rows(of index: Int) -> Int {
        guard index >= 0, index + 1 < rowStarts.count else { return 0 }
        return rowStarts[index + 1] - rowStarts[index]
    }

    /// The chunk containing a display row, clamped to the document.
    public func chunk(containingRow row: Int) -> Int {
        guard rowStarts.count > 1 else { return 0 }
        let target = min(max(0, row), max(0, totalRows - 1))
        var low = 0
        var high = rowStarts.count - 2
        while low < high {
            let mid = (low + high + 1) / 2
            if rowStarts[mid] <= target { low = mid } else { high = mid - 1 }
        }
        return low
    }
}

/// Which chunks are laid out right now, and which have been let go.
public enum TranscriptResidency {
    /// The chunks to keep typeset, given what is on screen.
    ///
    /// The visible range comes first and is never dropped for overscan; if
    /// the visible range alone somehow exceeded the ceiling it is trimmed
    /// from its far end, so the returned window is under the ceiling
    /// unconditionally. That trim is not expected to fire -- the ceiling is
    /// 128 KB and the largest measured screenful is 6.9 KB -- but "not
    /// expected" is not a bound.
    ///
    /// Overscan is then added one chunk at a time, alternating below and
    /// above, so a reader scrolling in either direction has the same amount
    /// of already-typeset text ahead of them.
    public static func window(
        _ document: TranscriptDocument,
        visible: Range<Int>,
        limitBytes: Int = TranscriptPaging.retainedLimitBytes
    ) -> Range<Int> {
        let count = document.chunkCount
        guard count > 0 else { return 0..<0 }

        var lower = min(max(0, visible.lowerBound), count - 1)
        var upper = min(max(lower + 1, visible.upperBound), count)
        var bytes = document.chunks[lower..<upper].reduce(0) { $0 + $1.byteCount }
        while upper - lower > 1, bytes > limitBytes {
            upper -= 1
            bytes -= document.chunks[upper].byteCount
        }

        var growDown = true
        while true {
            let canDown = lower > 0 && bytes + document.chunks[lower - 1].byteCount <= limitBytes
            let canUp = upper < count && bytes + document.chunks[upper].byteCount <= limitBytes
            if !canDown && !canUp { break }
            if growDown && canDown {
                lower -= 1
                bytes += document.chunks[lower].byteCount
            } else if canUp {
                bytes += document.chunks[upper].byteCount
                upper += 1
            } else {
                lower -= 1
                bytes += document.chunks[lower].byteCount
            }
            growDown.toggle()
        }
        return lower..<upper
    }
}

/// The typeset chunks the view is holding, and the eviction that keeps that
/// set bounded.
///
/// Generic over what a chunk is rendered into so the policy can be tested
/// for what it is -- an accounting rule over byte counts -- without a view,
/// a font, or a running app. The view instantiates it with `AttributedString`;
/// the tests instantiate it with `String` and assert the same `retainedBytes`
/// the view is subject to.
public struct TranscriptResidentChunks<Rendered> {
    public private(set) var window: Range<Int> = 0..<0
    public private(set) var rendered: [Int: Rendered] = [:]
    /// UTF-8 bytes of body currently typeset. The number the ceiling is on.
    public private(set) var retainedBytes: Int = 0
    /// How many chunks have been evicted since this set was created. Exists
    /// so a test can prove eviction happened rather than infer it from a
    /// count that merely stopped growing.
    public private(set) var evictions: Int = 0

    public init() {}

    public var residentCount: Int { rendered.count }

    /// Moves the window to cover `visible`, typesetting what came into it
    /// and dropping what fell out.
    ///
    /// `make` is called only for chunks that are not already rendered, so a
    /// scroll of one chunk costs one chunk of layout, not a window's worth.
    public mutating func update(
        document: TranscriptDocument,
        visible: Range<Int>,
        limitBytes: Int = TranscriptPaging.retainedLimitBytes,
        make: (Int) -> Rendered
    ) {
        let next = TranscriptResidency.window(document, visible: visible, limitBytes: limitBytes)
        guard next != window || rendered.count != next.count else { return }

        let dropped = rendered.keys.filter { !next.contains($0) }
        for index in dropped {
            rendered.removeValue(forKey: index)
            retainedBytes -= document.chunks[index].byteCount
            evictions += 1
        }
        for index in next where rendered[index] == nil {
            rendered[index] = make(index)
            retainedBytes += document.chunks[index].byteCount
        }
        window = next
    }
}
