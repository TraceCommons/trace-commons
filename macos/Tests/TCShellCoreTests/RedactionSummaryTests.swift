import XCTest

@testable import TCShellCore

/// Decoding the core's scrubbing panel, and the one rule this shell still
/// applies to a row: its count line. The grouping, the split and the
/// descriptions are the core's, asserted against the real export in
/// `TCBridgeTests/CoreCopyExportTests`.
final class RedactionSummaryTests: XCTestCase {
    private func row(_ family: String, _ occurrences: Int, _ distinct: Int) -> RedactionSummaryRow {
        RedactionSummaryRow(
            family: family,
            display: family.replacingOccurrences(of: "_", with: " "),
            description: "D",
            occurrences: occurrences,
            distinct: distinct,
            detail: []
        )
    }

    func testDecodesBothListsByName() throws {
        let json = """
            {"removed": [{"family": "local_path", "display": "local path", "description": "D",
                          "occurrences": 3, "distinct": 1, "detail": []}],
             "still_present": [{"family": "residual_secret_at", "display": "residual secret at",
                                "description": "S", "occurrences": 1, "distinct": 0,
                                "detail": ["events.3.tool_result"]}]}
            """
        let out = try XCTUnwrap(RedactionSummary.rows(fromJSON: json))
        XCTAssertEqual(out.removed.map(\.family), ["local_path"])
        XCTAssertEqual(out.stillPresent.map(\.family), ["residual_secret_at"])
        XCTAssertEqual(out.stillPresent[0].detail, ["events.3.tool_result"])
    }

    /// An unreadable answer is not "no rows": the panel must not claim
    /// nothing matched because it could not read what did.
    func testAnUnreadablePayloadIsNilNotEmpty() {
        XCTAssertNil(RedactionSummary.rows(fromJSON: nil))
        XCTAssertNil(RedactionSummary.rows(fromJSON: "not json"))
        XCTAssertNil(RedactionSummary.rows(fromJSON: #"{"removed": []}"#))
    }

    /// `(0 distinct)` beside a non-zero occurrence count reads as "nothing
    /// was removed", the exact opposite of what happened.
    func testACountLineOmitsAZeroDistinctFigure() {
        XCTAssertEqual(row("secret", 2, 0).countLine, "2 secret")
    }

    func testACountLineCarriesTheDistinctFigureWhenItAddsSomething() {
        XCTAssertEqual(row("local_path", 185, 12).countLine, "185 local path (12 distinct)")
    }

    /// Equal counts say the same thing twice.
    func testACountLineOmitsADistinctFigureThatMatchesItsOccurrences() {
        XCTAssertEqual(row("local_path", 3, 3).countLine, "3 local path")
    }
}
