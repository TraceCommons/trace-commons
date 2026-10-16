import XCTest

@testable import TraceCommonsApp

/// The gold line is a judgement -- "nothing matched" is the case to slow
/// down on -- but it was a judgement with nothing to do about it.
final class ScrubbingCaveatTests: XCTestCase {
    func testTheNothingMatchedLineOffersANextStep() {
        let line = ScrubbingCaveat.rowLine(redactionCount: 0)
        XCTAssertTrue(
            line.lowercased().contains("search"),
            "the line must point at the thing to do about it: \(line)"
        )
    }

    func testALineWithRedactionsIsUnchangedInTone() {
        let line = ScrubbingCaveat.rowLine(redactionCount: 4)
        XCTAssertFalse(line.isEmpty)
    }

    /// The canonical sentence is the core's (#1146 parity, 2026-10-07),
    /// verbatim, and never written here.
    func testTheCanonicalSentenceIsTheCoresVerbatim() {
        XCTAssertEqual(ScrubbingCaveat.canonical, "Scrubbing is pattern-based. It misses things it hasn't seen before.")
        XCTAssertEqual(ScrubbingCaveat.beforeYouContribute, "Before you contribute.")
    }
}
