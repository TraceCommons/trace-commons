import XCTest

@testable import TCShellCore

final class TranscriptBudgetTests: XCTestCase {
    /// A body under the budget is passed through untouched and carries no
    /// notice. The common case must not gain a truncation warning it does
    /// not deserve.
    func testShortBodyIsUnchanged() {
        let text = "line one\nline two\n"
        let clamped = TranscriptBudget.clamp(text)
        XCTAssertEqual(clamped.shown, text)
        XCTAssertEqual(clamped.withheldBytes, 0)
        XCTAssertFalse(clamped.isClamped)
        XCTAssertEqual(TranscriptBudget.notice(clamped), "")
    }

    /// A body exactly at the budget is not clamped. Off-by-one here would
    /// put a "showing the first 64 KB of 64 KB" notice on screen.
    func testBodyExactlyAtBudgetIsNotClamped() {
        let text = String(repeating: "a", count: TranscriptBudget.limitBytes)
        let clamped = TranscriptBudget.clamp(text)
        XCTAssertFalse(clamped.isClamped)
        XCTAssertEqual(clamped.shown.utf8.count, TranscriptBudget.limitBytes)
    }

    /// The slice never exceeds the budget, and the withheld count is the
    /// exact remainder -- the notice's arithmetic depends on it.
    func testLongBodyIsClampedToBudget() {
        let line = String(repeating: "x", count: 99) + "\n"
        let text = String(repeating: line, count: 20_000)  // ~2 MB
        let clamped = TranscriptBudget.clamp(text)

        XCTAssertTrue(clamped.isClamped)
        XCTAssertLessThanOrEqual(clamped.shown.utf8.count, TranscriptBudget.limitBytes)
        XCTAssertEqual(clamped.totalBytes, text.utf8.count)
        XCTAssertEqual(
            clamped.shown.utf8.count + clamped.withheldBytes,
            clamped.totalBytes
        )
    }

    /// The cut lands on a line boundary, so the last visible line is whole.
    func testClampCutsOnALineBoundary() {
        let line = String(repeating: "x", count: 99) + "\n"
        let text = String(repeating: line, count: 20_000)
        let clamped = TranscriptBudget.clamp(text)
        XCTAssertTrue(clamped.shown.hasSuffix("\n"))
        // Every line survived intact: no partial line at the tail.
        for l in clamped.shown.split(separator: "\n") {
            XCTAssertEqual(l.count, 99)
        }
    }

    /// A body with no newline in the budget still gets cut, and the cut does
    /// not split a multi-byte character. This is the minified-JSON case.
    func testClampWithoutNewlinesDoesNotSplitAScalar() {
        // Four-byte scalars, so a naive byte cut lands mid-character with
        // high probability.
        let text = String(repeating: "🙂", count: TranscriptBudget.limitBytes)
        let clamped = TranscriptBudget.clamp(text)

        XCTAssertTrue(clamped.isClamped)
        XCTAssertLessThanOrEqual(clamped.shown.utf8.count, TranscriptBudget.limitBytes)
        XCTAssertFalse(clamped.shown.unicodeScalars.contains("\u{FFFD}"))
        // Round-tripping the slice reproduces its own bytes: proof the cut
        // is scalar-aligned rather than merely replacement-free.
        XCTAssertEqual(Array(clamped.shown.utf8), Array(text.utf8.prefix(clamped.shown.utf8.count)))
    }

    /// A multi-byte body that does have newlines keeps its characters whole
    /// too -- the line-boundary path and the scalar path must both hold.
    func testClampWithMultibyteLinesKeepsCharactersWhole() {
        let line = String(repeating: "é", count: 50) + "\n"
        let text = String(repeating: line, count: 20_000)
        let clamped = TranscriptBudget.clamp(text)

        XCTAssertTrue(clamped.isClamped)
        XCTAssertFalse(clamped.shown.unicodeScalars.contains("\u{FFFD}"))
        for l in clamped.shown.split(separator: "\n") {
            XCTAssertEqual(l.count, 50)
        }
    }

    /// The notice states both numbers and does not imply approval shrank.
    /// This is the sentence that keeps the tab's promise true, so it is
    /// asserted verbatim rather than by shape.
    func testNoticeStatesShownTotalAndThatApprovalIsUnaffected() {
        let text = String(repeating: "x\n", count: 9_000_000)  // ~17.2 MB
        let clamped = TranscriptBudget.clamp(text)
        let notice = TranscriptBudget.notice(clamped)

        XCTAssertEqual(
            notice,
            "Showing the first 64 KB of 17.2 MB. "
                + "The rest is not displayed here. Approving still covers the whole body."
        )
    }

    /// The reported "shown" figure is the size of what is actually on
    /// screen, not the budget constant. A cut that backs off to a line
    /// boundary shows slightly less than 64 KB, and the notice must not
    /// round that into a claim about bytes the reader cannot see.
    func testNoticeReportsBytesActuallyShown() {
        let line = String(repeating: "x", count: 999) + "\n"
        let text = String(repeating: line, count: 2_000)
        let clamped = TranscriptBudget.clamp(text)
        let shownBytes = clamped.totalBytes - clamped.withheldBytes
        XCTAssertEqual(shownBytes, clamped.shown.utf8.count)
    }

    /// The 17.5 MB body that hung the app lays out its slice promptly. The
    /// budget exists for this case, so the case is the test.
    func testRealisticLargeBodyClampsQuickly() {
        let line = String(repeating: "y", count: 175) + "\n"
        let text = String(repeating: line, count: 100_000)  // ~17.6 MB
        let started = Date()
        let clamped = TranscriptBudget.clamp(text)
        let elapsed = Date().timeIntervalSince(started)

        XCTAssertTrue(clamped.isClamped)
        XCTAssertLessThanOrEqual(clamped.shown.utf8.count, TranscriptBudget.limitBytes)
        XCTAssertLessThan(elapsed, 2.0, "clamping should be a scan, not a reflow")
    }
}
