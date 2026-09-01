import XCTest

@testable import TCShellCore

/// The transcript tab's promise is "exactly what would be sent", and the
/// button beneath it approves every byte. These tests exist to keep two
/// things true at once: every byte is reachable, and no more than
/// `retainedLimitBytes` of it is typeset at any moment.
final class TranscriptPagingTests: XCTestCase {
    // MARK: - Fixtures

    /// A transcript-shaped body: JSON-ish lines around 78 bytes, with a
    /// redaction marker every so often, cut to an exact byte size.
    private func body(bytes: Int, markerEvery: Int = 40) -> String {
        var out = ""
        out.reserveCapacity(bytes + 128)
        var line = 0
        while out.utf8.count < bytes {
            if line % markerEvery == markerEvery - 1 {
                out += "  \"key\": \"<PRIVATE_SECRET_\(line)>\",\n"
            } else {
                out += "  \"content\": \"turn \(line) the quick brown fox 0123456789ABCDEF\",\n"
            }
            line += 1
        }
        return out
    }

    private func totalRendered(_ document: TranscriptDocument) -> String {
        (0..<document.chunkCount).map { document.text(of: $0) }.joined()
    }

    // MARK: - Every byte is reachable

    /// The point of the change. The chunks, concatenated, are the body --
    /// not a prefix of it, not a lossy re-encoding of it.
    func testEveryByteIsReachable() {
        let text = body(bytes: 3 * 1024 * 1024)
        let document = TranscriptDocument(text)
        XCTAssertEqual(document.totalBytes, text.utf8.count)
        XCTAssertEqual(totalRendered(document), text)
        XCTAssertEqual(
            document.chunks.reduce(0) { $0 + $1.byteCount },
            text.utf8.count
        )
    }

    /// Chunks tile the body exactly: no gap, no overlap, first starts at 0.
    func testChunksTileTheBodyWithoutGapOrOverlap() {
        let document = TranscriptDocument(body(bytes: 512 * 1024))
        XCTAssertEqual(document.chunks.first?.byteOffset, 0)
        var expected = 0
        for chunk in document.chunks {
            XCTAssertEqual(chunk.byteOffset, expected)
            XCTAssertGreaterThan(chunk.byteCount, 0)
            expected += chunk.byteCount
        }
        XCTAssertEqual(expected, document.totalBytes)
    }

    func testEmptyBodyHasNoChunks() {
        let document = TranscriptDocument("")
        XCTAssertEqual(document.chunkCount, 0)
        XCTAssertEqual(document.totalBytes, 0)
        XCTAssertEqual(TranscriptResidency.window(document, visible: 0..<1), 0..<0)
    }

    /// A body smaller than one chunk is one chunk, unchanged.
    func testShortBodyIsASingleChunk() {
        let text = "line one\nline two\n"
        let document = TranscriptDocument(text)
        XCTAssertEqual(document.chunkCount, 1)
        XCTAssertEqual(document.text(of: 0), text)
    }

    // MARK: - Chunk size

    /// Every chunk is inside the declared ceiling, and the ordinary
    /// newline-terminated ones are also above the floor -- otherwise a body
    /// of short lines would degenerate into thousands of tiny chunks.
    func testChunkSizesStayWithinTheDeclaredBounds() {
        let document = TranscriptDocument(body(bytes: 2 * 1024 * 1024))
        for (index, chunk) in document.chunks.enumerated() {
            XCTAssertLessThanOrEqual(
                chunk.byteCount, TranscriptPaging.maxChunkBytes,
                "chunk \(index) is \(chunk.byteCount) bytes"
            )
            if index < document.chunkCount - 1 {
                XCTAssertGreaterThanOrEqual(
                    chunk.byteCount, TranscriptPaging.targetChunkBytes / 2,
                    "chunk \(index) is \(chunk.byteCount) bytes"
                )
            }
        }
        // 2 MB of ~4 KB chunks. The count is the measured one, not
        // 2 MB / 4 KB: cutting back to the last newline before the target
        // costs an average of 48 bytes a chunk, so 518 chunks averaging
        // 4,048 bytes rather than 512 averaging 4,096.
        XCTAssertEqual(document.chunkCount, 518)
        XCTAssertEqual(document.totalBytes / document.chunkCount, 4_048)
    }

    /// The line-boundary path is the normal one: a chunk of a
    /// newline-terminated body ends at a newline.
    func testChunksEndOnLineBoundaries() {
        let document = TranscriptDocument(body(bytes: 256 * 1024))
        for index in 0..<(document.chunkCount - 1) {
            XCTAssertTrue(
                document.text(of: index).hasSuffix("\n"),
                "chunk \(index) does not end at a line"
            )
        }
    }

    // MARK: - Boundaries never split a character

    /// Four-byte scalars with no newline anywhere: the minified-JSON case,
    /// where a naive byte cut lands mid-character with high probability.
    func testBoundariesNeverSplitACharacterWithoutNewlines() {
        let text = String(repeating: "🙂", count: 200_000)  // 800 KB, one line
        let document = TranscriptDocument(text)
        XCTAssertGreaterThan(document.chunkCount, 190)

        for index in 0..<document.chunkCount {
            let chunk = document.text(of: index)
            XCTAssertFalse(
                chunk.unicodeScalars.contains("\u{FFFD}"),
                "chunk \(index) decoded with a replacement character"
            )
            // Whole emoji only: the count is exact, not "roughly right".
            XCTAssertEqual(chunk.utf8.count % 4, 0)
            XCTAssertEqual(chunk.count, chunk.utf8.count / 4)
        }
        XCTAssertEqual(totalRendered(document), text)
    }

    /// Multi-byte characters on newline-terminated lines: both the line
    /// path and the scalar path have to hold.
    func testBoundariesNeverSplitACharacterWithMultibyteLines() {
        let text = String(repeating: String(repeating: "é", count: 50) + "\n", count: 20_000)
        let document = TranscriptDocument(text)
        for index in 0..<document.chunkCount {
            let chunk = document.text(of: index)
            XCTAssertFalse(chunk.unicodeScalars.contains("\u{FFFD}"))
            for line in chunk.split(separator: "\n") {
                XCTAssertEqual(line.count, 50)
            }
        }
        XCTAssertEqual(totalRendered(document), text)
    }

    // MARK: - Markers survive the boundary

    /// A marker placed exactly across the first chunk boundary, on a body
    /// with no newline to cut on, must come out whole in one chunk.
    ///
    /// This is the failure the chip rendering cannot survive: `<PRIVATE_SEC`
    /// in one chunk and `RET_1>` in the next are both drawn as ordinary
    /// body text, which reads as content that was never scrubbed.
    func testMarkerStraddlingABoundaryIsNotSplit() {
        let target = TranscriptPaging.targetChunkBytes
        let marker = "<PRIVATE_SECRET_1>"
        for offsetIntoMarker in 1..<marker.utf8.count {
            let prefix = String(repeating: "x", count: target - offsetIntoMarker)
            let text = prefix + marker + String(repeating: "y", count: 4096)
            let document = TranscriptDocument(text)

            let carriers = (0..<document.chunkCount).filter {
                document.text(of: $0).contains(marker)
            }
            XCTAssertEqual(
                carriers.count, 1,
                "marker split at offset \(offsetIntoMarker); chunks: "
                    + "\(document.chunks.map(\.byteCount))"
            )
            for index in 0..<document.chunkCount {
                let chunk = document.text(of: index)
                XCTAssertFalse(chunk.hasSuffix("<PRIVATE_SEC"))
                XCTAssertFalse(chunk.hasPrefix("RET_1>"))
            }
            XCTAssertEqual(totalRendered(document), text)
        }
    }

    /// The same for the labelled `[REDACTED:...]` family.
    func testLabelledMarkerStraddlingABoundaryIsNotSplit() {
        let target = TranscriptPaging.targetChunkBytes
        let marker = "[REDACTED:aws_secret_key]"
        for offsetIntoMarker in 1..<marker.utf8.count {
            let text =
                String(repeating: "x", count: target - offsetIntoMarker) + marker
                + String(repeating: "y", count: 4096)
            let document = TranscriptDocument(text)
            let carriers = (0..<document.chunkCount).filter {
                document.text(of: $0).contains(marker)
            }
            XCTAssertEqual(carriers.count, 1, "marker split at offset \(offsetIntoMarker)")
        }
    }

    /// Chipping per chunk finds exactly the markers chipping the whole body
    /// would find -- same count, same text, same order. This is the
    /// property the view depends on now that it never scans the whole body.
    func testPerChunkMarkerScanMatchesWholeBodyScan() {
        let text = body(bytes: 512 * 1024, markerEvery: 7)
        let document = TranscriptDocument(text)

        let whole = TranscriptMarkerScan.spans(in: text).map { String(text[$0]) }
        let perChunk = (0..<document.chunkCount).flatMap { index -> [String] in
            let chunk = document.text(of: index)
            return TranscriptMarkerScan.spans(in: chunk).map { String(chunk[$0]) }
        }

        XCTAssertGreaterThan(whole.count, 800)
        XCTAssertEqual(perChunk, whole)
    }

    /// An unclosed bracket does not turn the rest of the body into one
    /// enormous "marker" the chunker then refuses to cut.
    func testUnclosedBracketDoesNotSwallowTheBody() {
        let text = "[REDACTED:oops\n" + String(repeating: "z", count: 64 * 1024)
        let document = TranscriptDocument(text)
        XCTAssertEqual(TranscriptMarkerScan.spans(in: text).count, 0)
        for chunk in document.chunks {
            XCTAssertLessThanOrEqual(chunk.byteCount, TranscriptPaging.maxChunkBytes)
        }
    }

    // MARK: - The retained-byte ceiling

    /// The ceiling holds at every step of a scroll through a 17.5 MB body,
    /// and the visible chunk is always among the chunks that are typeset.
    ///
    /// This is the assertion the whole design is for. It is not enough that
    /// the window stops growing: it has to stay under the ceiling from the
    /// first screen to the last.
    func testRetainedBytesCeilingHoldsWhileScrollingSeventeenMegabytes() {
        let text = body(bytes: 17_500_000)
        let document = TranscriptDocument(text)
        XCTAssertGreaterThan(document.totalBytes, 17_000_000)

        var resident = TranscriptResidentChunks<String>()
        var peak = 0
        var step = 0
        // Every 8th chunk: a fast drag, several screenfuls per frame.
        while step < document.chunkCount {
            let visible = step..<min(step + 2, document.chunkCount)
            resident.update(document: document, visible: visible) { document.text(of: $0) }

            XCTAssertLessThanOrEqual(
                resident.retainedBytes, TranscriptPaging.retainedLimitBytes,
                "retained \(resident.retainedBytes) at chunk \(step)"
            )
            XCTAssertTrue(
                resident.window.contains(step),
                "visible chunk \(step) was not resident"
            )
            XCTAssertEqual(
                resident.retainedBytes,
                resident.window.reduce(0) { $0 + document.chunks[$1].byteCount },
                "accounting drifted from the window at chunk \(step)"
            )
            XCTAssertEqual(resident.residentCount, resident.window.count)
            peak = max(peak, resident.retainedBytes)
            step += 8
        }

        // The window is actually being filled, not trivially empty: at the
        // ceiling it holds a real number of chunks.
        XCTAssertGreaterThan(peak, TranscriptPaging.retainedLimitBytes - TranscriptPaging.maxChunkBytes)
        // And it evicted its way there rather than accumulating.
        XCTAssertGreaterThan(resident.evictions, 4_000)
        XCTAssertLessThanOrEqual(
            resident.retainedBytes, TranscriptPaging.retainedLimitBytes)
    }

    /// Eviction, stated directly: after scrolling away, the chunk that was
    /// on screen at the start is no longer typeset. A window that only ever
    /// adds passes every ceiling test above by never reaching the ceiling
    /// early; this is what catches it.
    func testChunksThatScrollAwayAreEvicted() {
        let document = TranscriptDocument(body(bytes: 4 * 1024 * 1024))
        var resident = TranscriptResidentChunks<String>()

        resident.update(document: document, visible: 0..<1) { document.text(of: $0) }
        XCTAssertNotNil(resident.rendered[0])
        let firstWindow = resident.window
        XCTAssertEqual(resident.evictions, 0)

        resident.update(document: document, visible: 600..<601) { document.text(of: $0) }
        XCTAssertNil(resident.rendered[0], "chunk 0 was still typeset after scrolling away")
        XCTAssertFalse(resident.window.overlaps(firstWindow))
        XCTAssertEqual(resident.evictions, firstWindow.count)
        XCTAssertLessThanOrEqual(resident.retainedBytes, TranscriptPaging.retainedLimitBytes)

        // Scrolling back re-typesets it: eviction is not a one-way loss of
        // reachability.
        resident.update(document: document, visible: 0..<1) { document.text(of: $0) }
        XCTAssertEqual(resident.rendered[0], document.text(of: 0))
    }

    /// Advancing by one chunk typesets one chunk, not a window's worth. The
    /// measured per-chunk layout cost is only a frame if this holds.
    func testAdvancingOneChunkRendersOneChunk() {
        let document = TranscriptDocument(body(bytes: 2 * 1024 * 1024))
        var resident = TranscriptResidentChunks<String>()
        resident.update(document: document, visible: 200..<201) { document.text(of: $0) }

        var made: [Int] = []
        resident.update(document: document, visible: 201..<202) { index in
            made.append(index)
            return document.text(of: index)
        }
        XCTAssertEqual(made.count, 1)
        XCTAssertEqual(resident.evictions, 1)
    }

    /// The window is centred on the viewport, so overscan exists in both
    /// directions rather than only ahead.
    func testWindowOverscansBothWays() {
        let document = TranscriptDocument(body(bytes: 4 * 1024 * 1024))
        let window = TranscriptResidency.window(document, visible: 500..<501)
        XCTAssertLessThan(window.lowerBound, 500)
        XCTAssertGreaterThan(window.upperBound, 501)
        let ahead = window.upperBound - 501
        let behind = 500 - window.lowerBound
        XCTAssertLessThanOrEqual(abs(ahead - behind), 1)
        XCTAssertLessThanOrEqual(
            window.reduce(0) { $0 + document.chunks[$1].byteCount },
            TranscriptPaging.retainedLimitBytes
        )
    }

    /// At the ends of the body the window does not shrink: it spends all of
    /// its budget on the side that exists.
    func testWindowAtTheStartAndEndStillFillsItsBudget() {
        let document = TranscriptDocument(body(bytes: 4 * 1024 * 1024))
        for visible in [0..<1, (document.chunkCount - 1)..<document.chunkCount] {
            let window = TranscriptResidency.window(document, visible: visible)
            let bytes = window.reduce(0) { $0 + document.chunks[$1].byteCount }
            XCTAssertLessThanOrEqual(bytes, TranscriptPaging.retainedLimitBytes)
            XCTAssertGreaterThan(
                bytes, TranscriptPaging.retainedLimitBytes - TranscriptPaging.maxChunkBytes,
                "the window left budget unspent at \(visible)"
            )
        }
    }

    /// A body smaller than the ceiling is entirely resident: the chunking
    /// must not cost small traces anything.
    func testSmallBodyIsFullyResident() {
        let text = body(bytes: 32 * 1024)
        let document = TranscriptDocument(text)
        var resident = TranscriptResidentChunks<String>()
        resident.update(document: document, visible: 0..<1) { document.text(of: $0) }
        XCTAssertEqual(resident.window, 0..<document.chunkCount)
        XCTAssertEqual(resident.retainedBytes, document.totalBytes)
        XCTAssertEqual(
            (0..<document.chunkCount).compactMap { resident.rendered[$0] }.joined(), text)
    }

    // MARK: - Placement of chunks that are not resident

    /// Row offsets are cumulative and complete, so a non-resident chunk
    /// holds exactly its own place in the scroll.
    func testRowIndexIsCumulativeAndComplete() {
        let document = TranscriptDocument(body(bytes: 256 * 1024))
        let index = TranscriptRowIndex(document, columns: 89)
        XCTAssertEqual(index.rowStarts.count, document.chunkCount + 1)
        XCTAssertEqual(index.rowStarts.first, 0)
        XCTAssertEqual(
            index.totalRows,
            (0..<document.chunkCount).reduce(0) { $0 + index.rows(of: $1) }
        )
        for chunk in 0..<document.chunkCount {
            XCTAssertGreaterThan(index.rows(of: chunk), 0)
            XCTAssertEqual(index.chunk(containingRow: index.rowStarts[chunk]), chunk)
            XCTAssertEqual(
                index.chunk(containingRow: index.rowStarts[chunk + 1] - 1), chunk)
        }
    }

    /// The estimate is exact when nothing wraps: 78-byte lines at 89
    /// columns are one row each.
    func testRowEstimateIsExactWhenNothingWraps() {
        let text = String(repeating: String(repeating: "x", count: 60) + "\n", count: 1_000)
        let document = TranscriptDocument(text)
        let index = TranscriptRowIndex(document, columns: 89)
        XCTAssertEqual(index.totalRows, 1_000)
    }

    /// And when everything wraps, it is the wrapped count, rounded up once
    /// per chunk. One unbroken 8,900-byte line at 89 columns is 100 rows;
    /// cut into chunks of 4,096, 4,096 and 708 bytes it estimates
    /// 47 + 47 + 8 = 102. Two rows of slack over 100 is the cost of
    /// placing chunks without measuring them, and it is bounded at one row
    /// per chunk -- 16 pt of scroll extent per 4 KB.
    func testRowEstimateCountsWrappedRowsRoundedPerChunk() {
        let text = String(repeating: "x", count: 8_900)
        let document = TranscriptDocument(text)
        XCTAssertEqual(document.chunks.map(\.byteCount), [4_096, 4_096, 708])
        let index = TranscriptRowIndex(document, columns: 89)
        XCTAssertEqual(index.totalRows, 102)
    }

    func testRowLookupClampsOutOfRangeRows() {
        let document = TranscriptDocument(body(bytes: 64 * 1024))
        let index = TranscriptRowIndex(document, columns: 89)
        XCTAssertEqual(index.chunk(containingRow: -50), 0)
        XCTAssertEqual(index.chunk(containingRow: index.totalRows + 500), document.chunkCount - 1)
    }

    // MARK: - Search snippets

    /// Snippets are cut from bytes at the offsets the FFI reports, and the
    /// text around the match is the text that is really there.
    func testSnippetIsCutAtTheReportedByteOffset() {
        let text = "aaaa\nNEEDLE_HERE\nbbbb\n"
        let document = TranscriptDocument(text)
        let offset = Array(text.utf8).firstRange(of: Array("NEEDLE_HERE".utf8))!.lowerBound

        let snippet = document.snippet(around: offset, matchBytes: 11, window: 3)
        XCTAssertEqual(snippet.text, "aa\nNEEDLE_HERE\nbb")
        XCTAssertTrue(snippet.elidedBefore)
        XCTAssertTrue(snippet.elidedAfter)
    }

    /// A snippet whose window would land inside a multi-byte character
    /// backs off to the character boundary rather than emitting U+FFFD.
    func testSnippetEndsAreScalarAligned() {
        let text = String(repeating: "🙂", count: 40) + "NEEDLE" + String(repeating: "🙂", count: 40)
        let document = TranscriptDocument(text)
        let offset = Array(text.utf8).firstRange(of: Array("NEEDLE".utf8))!.lowerBound

        // 6 is not a multiple of 4: a naive cut splits an emoji at both ends.
        // Both ends back off to the nearest scalar boundary, so the window
        // grows to 8 bytes before the match and shrinks to 4 after it.
        let snippet = document.snippet(around: offset, matchBytes: 6, window: 6)
        XCTAssertFalse(snippet.text.unicodeScalars.contains("\u{FFFD}"))
        XCTAssertEqual(snippet.text, "🙂🙂NEEDLE🙂")
    }

    /// Snippets at the ends of the body report nothing elided, so the
    /// leading and trailing ellipsis are only drawn when text really is cut.
    func testSnippetAtTheEdgesReportsNothingElided() {
        let text = "NEEDLE tail"
        let document = TranscriptDocument(text)
        let snippet = document.snippet(around: 0, matchBytes: 6, window: 500)
        XCTAssertEqual(snippet.text, text)
        XCTAssertFalse(snippet.elidedBefore)
        XCTAssertFalse(snippet.elidedAfter)
    }

    // MARK: - Cost

    /// Fastest of `runs` attempts.
    ///
    /// Load only ever adds time, never removes it, so the minimum is the
    /// least noisy estimate available on a shared runner. The assertions
    /// below then compare two minima against each other, which cancels how
    /// fast the machine is: what is being asserted is the shape of the cost
    /// curve, and that is a property of the code rather than of the runner.
    private func fastest(_ runs: Int = 5, _ work: () -> Void) -> TimeInterval {
        var best = TimeInterval.infinity
        for _ in 0..<runs {
            let started = Date()
            work()
            best = min(best, Date().timeIntervalSince(started))
        }
        return best
    }

    /// Chunking is a scan, not a reflow. If this ever became quadratic it
    /// would be the hang again, moved one function along.
    ///
    /// This used to assert `elapsed < 2.0` for a single 17.5 MB body, which
    /// says "this machine is fast enough" rather than "this algorithm is
    /// linear". It failed on a loaded runner at 3.36s while the code was
    /// perfectly linear, and it would equally have passed on a fast machine
    /// running a quadratic implementation over a smaller body -- wrong in
    /// both directions.
    ///
    /// Measuring two sizes and comparing them tests the actual claim. Four
    /// times the bytes costs about four times as long when the work is a
    /// scan and about sixteen when it is a reflow, so a threshold at twice
    /// the linear ratio sits an octave clear of both answers.
    func testChunkingCostGrowsWithTheBodyNotWithItsSquare() {
        let small = body(bytes: 3_000_000)
        let large = body(bytes: 12_000_000)

        // `body(bytes:)` fills until it passes the mark, so the real ratio is
        // near four rather than exactly four. Derive it instead of assuming.
        let linearRatio = Double(large.utf8.count) / Double(small.utf8.count)

        // Accumulated so the optimiser cannot discard the construction whose
        // cost is the entire point of the measurement.
        var sink = 0
        let smallTime = fastest(3) { sink &+= TranscriptDocument(small).chunkCount }
        let largeTime = fastest(3) { sink &+= TranscriptDocument(large).chunkCount }

        XCTAssertGreaterThan(sink, 0)
        XCTAssertGreaterThan(
            TranscriptDocument(large).chunkCount, 2_000,
            "a 12 MB body must still chunk into thousands, or the timing measures nothing"
        )
        XCTAssertGreaterThan(
            smallTime, 0,
            "the small case was too fast to time; raise the sizes rather than trusting the ratio"
        )

        let ratio = largeTime / smallTime
        XCTAssertLessThan(
            ratio, linearRatio * 2,
            """
            \(linearRatio)x the bytes cost \(ratio)x the time \
            (\(smallTime)s -> \(largeTime)s). A scan costs about \(linearRatio)x; \
            this is closer to the \(linearRatio * linearRatio)x of a reflow.
            """
        )
    }

    /// Moving the window is independent of how big the body is: the cost of
    /// a scroll must not grow with the trace.
    ///
    /// Same rework as above, and the same reason. The old form asserted that
    /// 2000 window moves over one 17.5 MB body finished within 2.0s, which a
    /// loaded runner can miss without anything being wrong. Independence
    /// from body size is the actual claim, so measure the same moves against
    /// two bodies an order of magnitude apart and require the cost not to
    /// track the difference.
    func testWindowMoveCostDoesNotTrackBodySize() {
        let smallDocument = TranscriptDocument(body(bytes: 1_500_000))
        let largeDocument = TranscriptDocument(body(bytes: 17_500_000))

        func scroll(_ document: TranscriptDocument) -> TranscriptResidentChunks<Int> {
            var resident = TranscriptResidentChunks<Int>()
            resident.update(document: document, visible: 0..<1) { $0 }
            for step in 1..<2_000 {
                resident.update(document: document, visible: step..<(step + 1)) { $0 }
            }
            return resident
        }

        var retained = 0
        let smallTime = fastest(3) { retained = scroll(smallDocument).retainedBytes }
        let largeTime = fastest(3) { retained = scroll(largeDocument).retainedBytes }

        XCTAssertLessThanOrEqual(retained, TranscriptPaging.retainedLimitBytes)
        XCTAssertGreaterThan(
            smallTime, 0,
            "the small case was too fast to time; raise the move count rather than trusting the ratio"
        )

        // The bodies differ by about 12x, so scrolling that walked the body
        // would cost about 12x more.
        //
        // The threshold is 6x rather than something near 1x because the
        // measured ratio is not near 1x -- it is about 2.6x. Scrolling is
        // not literally independent of body size: twelve times the body is
        // twelve times the chunks, and the per-update bookkeeping over that
        // chunk list is not free. It is sublinear, not constant, and at
        // these absolute times (single-digit milliseconds for 2000 moves)
        // fixed setup cost is a visible share of both measurements.
        //
        // So assert the claim that is actually true and actually matters:
        // the cost must stay far short of proportional. 6x sits at half of
        // proportional and better than twice the measured value, which
        // leaves real margin on both sides rather than the 16% a
        // near-1x threshold would have left -- that would have been a fresh
        // flake dressed as a tighter test.
        let ratio = largeTime / smallTime
        XCTAssertLessThan(
            ratio, 6.0,
            """
            2000 window moves cost \(ratio)x more on a 12x larger body \
            (\(smallTime)s -> \(largeTime)s), which is close enough to \
            proportional that the scroll is walking the trace
            """
        )
    }
}
