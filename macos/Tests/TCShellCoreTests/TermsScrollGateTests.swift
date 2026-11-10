import CoreGraphics
import TCShellCore
import XCTest

/// The accept checkbox unlocks only once the end of the terms has been in
/// view, and never locks again.
final class TermsScrollGateTests: XCTestCase {
    func testStartsLocked() {
        XCTAssertFalse(TermsScrollGate().reachedEnd)
    }

    /// A document that fits without scrolling (or very large text in a
    /// short document) unlocks on its first observation.
    func testShortDocumentUnlocksImmediately() {
        var gate = TermsScrollGate()
        gate.observe(contentHeight: 200, viewportHeight: 260, offset: 0)
        XCTAssertTrue(gate.reachedEnd)

        var exact = TermsScrollGate()
        exact.observe(contentHeight: 268, viewportHeight: 260, offset: 0)
        XCTAssertTrue(exact.reachedEnd, "content <= viewport + 8pt counts as fitting")
    }

    func testUnlocksAtEndWithinTolerance() {
        var gate = TermsScrollGate()
        gate.observe(contentHeight: 2000, viewportHeight: 260, offset: 0)
        XCTAssertFalse(gate.reachedEnd)
        gate.observe(contentHeight: 2000, viewportHeight: 260, offset: 1000)
        XCTAssertFalse(gate.reachedEnd)
        gate.observe(contentHeight: 2000, viewportHeight: 260, offset: 1731)
        XCTAssertFalse(gate.reachedEnd, "9pt short of the end is not the end")
        gate.observe(contentHeight: 2000, viewportHeight: 260, offset: 1732)
        XCTAssertTrue(gate.reachedEnd, "within 8pt of the end is the end")
    }

    func testScrollingBackDoesNotRelock() {
        var gate = TermsScrollGate()
        gate.observe(contentHeight: 2000, viewportHeight: 260, offset: 1740)
        XCTAssertTrue(gate.reachedEnd)
        gate.observe(contentHeight: 2000, viewportHeight: 260, offset: 0)
        XCTAssertTrue(gate.reachedEnd)
        // A later layout pass that grows the content does not relock either.
        gate.observe(contentHeight: 4000, viewportHeight: 260, offset: 0)
        XCTAssertTrue(gate.reachedEnd)
    }

    /// Before layout, a zero-height measurement says nothing.
    func testZeroHeightsSayNothing() {
        var gate = TermsScrollGate()
        gate.observe(contentHeight: 0, viewportHeight: 260, offset: 0)
        gate.observe(contentHeight: 2000, viewportHeight: 0, offset: 0)
        XCTAssertFalse(gate.reachedEnd)
    }

    /// VoiceOver reaching the last block counts as the end.
    func testLastBlockCountsAsTheEnd() {
        var gate = TermsScrollGate()
        gate.reachedLastBlock()
        XCTAssertTrue(gate.reachedEnd)
    }
}
