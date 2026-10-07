import TCShellCore
import XCTest

/// The three tables the monitor reads from the disclosure bundle
/// (`tc_contributor_disclosure_copy_json`): the verdict and correction
/// words, History's words, and the folder modes' names. Decoded from the
/// bundle's shape here; `ContributorDisclosureCopyExportTests` decodes the
/// real export.
final class ContributorDisclosureCopyTests: XCTestCase {
    private static let fixture = #"""
    {
      "witness_review": {"heading": "ignored by this decoder"},
      "outcome": {
        "verdict_question": "Q", "worked": "W", "partly": "P", "failed": "F",
        "verdict_caption": "C", "correction_question": "CQ", "correction_placeholder": "CP",
        "correction_caption": "CC", "correction_credential_headline": "CH",
        "correction_credential_body": "CB", "submit_all_as": "S", "submit_all_as_tooltip": "ST",
        "max_correction_chars": 2000
      },
      "history_ui": {
        "held_row_body": "H", "status_awaiting_pii_backstop": "A",
        "status_unavailable": "U", "status_labels": {"submitted": "L"}
      },
      "folder_mode_labels": {"notify_only": "M1", "auto_upload": "M2", "ignore": "M3"}
    }
    """#

    func testTheMonitorsThreeTablesDecode() throws {
        let copy = try XCTUnwrap(ContributorDisclosureCopy.decode(fromJSON: Self.fixture))
        XCTAssertEqual(copy.outcome.verdictQuestion, "Q")
        XCTAssertEqual(copy.outcome.worked, "W")
        XCTAssertEqual(copy.outcome.partly, "P")
        XCTAssertEqual(copy.outcome.failed, "F")
        XCTAssertEqual(copy.outcome.correctionQuestion, "CQ")
        XCTAssertEqual(copy.outcome.submitAllAs, "S")
        XCTAssertEqual(copy.outcome.maxCorrectionChars, 2000)
        XCTAssertEqual(copy.historyUi.statusUnavailable, "U")
        XCTAssertEqual(copy.historyUi.statusAwaitingPiiBackstop, "A")
        XCTAssertEqual(copy.historyUi.statusLabels["submitted"], "L")
        XCTAssertEqual(copy.folderModeLabels["auto_upload"], "M2")
    }

    /// The backstop label is optional in the core (`Option`); absent is not
    /// a broken bundle.
    func testAnAbsentBackstopLabelStillDecodes() throws {
        let json = Self.fixture.replacingOccurrences(of: #""status_awaiting_pii_backstop": "A","#, with: "")
        let copy = try XCTUnwrap(ContributorDisclosureCopy.decode(fromJSON: json))
        XCTAssertNil(copy.historyUi.statusAwaitingPiiBackstop)
    }

    /// Fail closed: an empty verdict word, or a zero correction limit, is a
    /// bundle this shell cannot render, never a blank button.
    func testAnEmptyWordOrNoLimitDecodesToNothing() {
        XCTAssertNil(ContributorDisclosureCopy.decode(fromJSON: Self.fixture.replacingOccurrences(of: #""worked": "W""#, with: #""worked": """#)))
        XCTAssertNil(ContributorDisclosureCopy.decode(fromJSON: Self.fixture.replacingOccurrences(of: "2000", with: "0")))
        XCTAssertNil(ContributorDisclosureCopy.decode(fromJSON: "{}"))
        XCTAssertNil(ContributorDisclosureCopy.decode(fromJSON: nil))
    }
}
