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
}
