import XCTest

@testable import TCShellCore

/// Three answers, and they are not interchangeable. "0 matches" against the
/// redacted body means either "it was never here" or "we took it out", and
/// a contributor checking whether their employer's name is in a trace needs
/// to know which.
final class OriginalSearchOutcomeTests: XCTestCase {
    func testNowhereInEitherTextIsAbsent() {
        XCTAssertEqual(OriginalSearchOutcome.classify(remaining: 0, original: 0), .absent)
    }

    func testPresentOriginallyAndGoneNowIsAllRemoved() {
        XCTAssertEqual(
            OriginalSearchOutcome.classify(remaining: 0, original: 3),
            .allRemoved(3)
        )
    }

    func testStillPresentIsSomeRemain() {
        XCTAssertEqual(
            OriginalSearchOutcome.classify(remaining: 2, original: 5),
            .someRemain(remaining: 2, total: 5)
        )
    }

    func testAFailedOriginalSearchIsUnknownNotAbsent() {
        // Reporting "not in this trace" because a call failed would be the
        // single most dangerous wrong answer this tab can give.
        XCTAssertEqual(OriginalSearchOutcome.classify(remaining: 0, original: nil), .unknown)
        XCTAssertEqual(
            OriginalSearchOutcome.classify(remaining: 2, original: nil),
            .someRemain(remaining: 2, total: 2)
        )
    }

    func testAnOriginalCountBelowTheRemainingCountFallsBackToWhatIsCertain() {
        // Impossible from a correct daemon. The certain half is that 2 are
        // still there.
        XCTAssertEqual(
            OriginalSearchOutcome.classify(remaining: 2, original: 1),
            .someRemain(remaining: 2, total: 2)
        )
    }

    func testTheSentencesSayWhichCaseItIs() {
        XCTAssertEqual(OriginalSearchOutcome.absent.sentence, "0 matches -- not in this session")
        XCTAssertEqual(
            OriginalSearchOutcome.allRemoved(3).sentence,
            "3 matches -- all 3 were removed"
        )
        XCTAssertEqual(
            OriginalSearchOutcome.someRemain(remaining: 2, total: 5).sentence,
            "5 matches -- 2 would still be sent"
        )
        XCTAssertEqual(
            OriginalSearchOutcome.unknown.sentence,
            "0 matches in what would be sent. Couldn't check the original."
        )
    }

    /// A failed check must not draw as a clean answer.
    ///
    /// `unknown` used to share `isAlarming == false` with `absent` and
    /// `allRemoved`, and the sheet drew that single bit as clear-or-
    /// attention. The result was the app's all-clear glyph beside the
    /// sentence "Couldn't check the original." -- the one outcome meaning
    /// *no answer* rendered identically to the one meaning *nothing found*.
    func testAnUncheckedOriginalIsNeitherClearNorAlarming() {
        XCTAssertEqual(OriginalSearchOutcome.unknown.emphasis, .unchecked)
        XCTAssertFalse(OriginalSearchOutcome.unknown.isAlarming)
        XCTAssertEqual(OriginalSearchOutcome.absent.emphasis, .clear)
        XCTAssertEqual(OriginalSearchOutcome.allRemoved(3).emphasis, .clear)
        XCTAssertEqual(
            OriginalSearchOutcome.someRemain(remaining: 2, total: 5).emphasis,
            .attention
        )
    }
}
