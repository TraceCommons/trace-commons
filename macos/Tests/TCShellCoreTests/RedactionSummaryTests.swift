import XCTest

@testable import TCShellCore

/// The scrubbing tab's "what left, and what didn't" panel.
///
/// The label vocabulary is open and namespaced -- `secret:{pattern}`,
/// `privacy_filter:{label}`, `tool_sensitive_field:{action}` are all
/// generated -- so this type can only reason about families, and it must
/// never claim to know what an unfamiliar one means.
final class RedactionSummaryTests: XCTestCase {
    func testAnEmptyMapProducesNoRows() {
        let out = RedactionSummary.rows(occurrences: [:], distinct: [:])
        XCTAssertTrue(out.removed.isEmpty)
        XCTAssertTrue(out.stillPresent.isEmpty)
    }

    func testOneFamilyBecomesOneRow() {
        let out = RedactionSummary.rows(
            occurrences: ["local_path": 185],
            distinct: ["local_path": 12]
        )
        XCTAssertEqual(out.removed.count, 1)
        XCTAssertEqual(out.removed[0].family, "local_path")
        XCTAssertEqual(out.removed[0].display, "local path")
        XCTAssertEqual(out.removed[0].occurrences, 185)
        XCTAssertEqual(out.removed[0].distinct, 12)
        XCTAssertFalse(out.removed[0].description.isEmpty)
    }

    /// Nine secret patterns are one `secret` row, not nine rows. The
    /// sub-labels go on a detail line.
    func testSubLabelsCollapseIntoTheirFamily() {
        let out = RedactionSummary.rows(
            occurrences: ["secret:contextual_entropy": 3, "secret:pem_private_key": 1, "secret": 2],
            distinct: ["secret:contextual_entropy": 2, "secret:pem_private_key": 1, "secret": 2]
        )
        XCTAssertEqual(out.removed.count, 1)
        XCTAssertEqual(out.removed[0].family, "secret")
        XCTAssertEqual(out.removed[0].occurrences, 6)
        XCTAssertEqual(out.removed[0].distinct, 5)
        XCTAssertEqual(
            out.removed[0].detail,
            ["contextual entropy", "pem private key"]
        )
    }

    /// A secret that was DETECTED AND NOT REMOVED. Putting it in `removed`
    /// would state the exact opposite of what happened.
    func testAResidualSurvivorIsReportedAsStillPresent() {
        let out = RedactionSummary.rows(
            occurrences: ["local_path": 3, "residual_secret_at:events.correction": 1],
            distinct: [:]
        )
        XCTAssertEqual(out.removed.map(\.family), ["local_path"])
        XCTAssertEqual(out.stillPresent.map(\.family), ["residual_secret_at"])
        XCTAssertEqual(out.stillPresent[0].detail, ["events.correction"])
    }

    /// An unfamiliar family gets a neutral description and is NEVER dropped.
    /// Hiding a category because this build has no words for it would
    /// understate what happened, which is the one direction this panel must
    /// not fail in.
    func testAnUnknownFamilyIsKeptWithANeutralDescription() {
        let out = RedactionSummary.rows(occurrences: ["future_category": 4], distinct: [:])
        XCTAssertEqual(out.removed.count, 1)
        XCTAssertEqual(out.removed[0].family, "future_category")
        XCTAssertFalse(out.removed[0].description.isEmpty)
        XCTAssertFalse(
            out.removed[0].description.contains("future"),
            "a neutral description must not pretend to know the category"
        )
    }

    func testRowsAreOrderedByOccurrencesThenFamily() {
        let out = RedactionSummary.rows(
            occurrences: ["secret": 3, "local_path": 185, "email": 3],
            distinct: [:]
        )
        XCTAssertEqual(out.removed.map(\.family), ["local_path", "email", "secret"])
    }

    /// The panel names kinds, never values. There is no value left to name.
    func testARowCarriesNoMatchedText() {
        let out = RedactionSummary.rows(occurrences: ["secret": 1], distinct: [:])
        XCTAssertEqual(out.removed[0].detail, [])
    }
}
