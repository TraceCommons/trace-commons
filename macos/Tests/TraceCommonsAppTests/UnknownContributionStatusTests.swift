import TCBridge
import XCTest

@testable import TraceCommonsApp

/// Every contribution status reads as the core's word for it
/// (`history_copy::STATUS_LABELS`, carried by `tc_public_run_copy` as
/// `history_status_labels`), and an unrecognised one as "Status
/// unavailable" -- the core's `history_copy::STATUS_UNAVAILABLE` -- on every
/// macOS surface. It is never "Not in the commons" (a claim about where the
/// trace is) and never the raw wire token.
final class UnknownContributionStatusTests: XCTestCase {
    private func sharedCopy() throws -> PublicRunCopy {
        try XCTUnwrap(PublicRunCopy.decode(fromJSON: TCPublicRun.copyJSON() ?? ""))
    }

    func testLegacyHistoryLabelsAnUnknownStatusFromTheCore() throws {
        let copy = try sharedCopy()
        for status in ["future_state", "", "pending"] {
            XCTAssertEqual(
                HistoryRow.statusSentence(for: status, copy: copy), "Status unavailable", status)
            // And it is terminal: no Withdraw beside it.
            XCTAssertFalse(ContributionStatusPresentation.offersWithdraw(status), status)
        }
    }

    func testLegacyHistoryKeepsItsKnownLabels() throws {
        let copy = try sharedCopy()
        XCTAssertEqual(HistoryRow.statusSentence(for: "accepted", copy: copy), "In the commons")
        XCTAssertEqual(
            HistoryRow.statusSentence(for: "submitted", copy: copy), "Waiting to be scored")
        XCTAssertEqual(
            HistoryRow.statusSentence(for: "quarantined", copy: copy), "Held for privacy review")
        XCTAssertEqual(
            HistoryRow.statusSentence(for: "withdrawn", copy: copy), "Withdrawn by you")
    }

    /// These read "Status unavailable" before the core had a table -- a
    /// rejected row beside a Withdraw button among them.
    func testLegacyHistoryLabelsTheStatusesItHadNoWordFor() throws {
        let copy = try sharedCopy()
        let expected = [
            "received": "Received",
            "rejected": "Rejected",
            "revoked": "Withdrawn",
            "awaiting_pii_backstop": "Waiting for privacy review",
            "processing": "Waiting to be scored",
            "expired": "Expired",
            "purged": "Purged",
        ]
        for (status, label) in expected {
            XCTAssertEqual(HistoryRow.statusSentence(for: status, copy: copy), label, status)
        }
    }

    func testEveryStatusInTheCoresTableReadsAsItsLabel() throws {
        let copy = try sharedCopy()
        XCTAssertEqual(copy.historyStatusLabels.count, 11)
        for row in copy.historyStatusLabels {
            XCTAssertEqual(copy.historyStatusLabel(for: row.value), row.label, row.value)
            XCTAssertNotEqual(row.label, copy.contributionStatusUnavailable, row.value)
        }
    }

    /// A Withdraw button never sits beside "Status unavailable".
    func testEveryWithdrawableStatusHasALabel() throws {
        let copy = try sharedCopy()
        for status in [
            "submitted", "received", "accepted", "quarantined", "awaiting_pii_backstop", "rejected",
        ] {
            XCTAssertTrue(ContributionStatusPresentation.offersWithdraw(status), status)
            XCTAssertNotEqual(
                copy.historyStatusLabel(for: status), copy.contributionStatusUnavailable, status)
        }
    }

    func testLegacyHistoryShowsNoTagRatherThanATypedOneWithoutTheCore() {
        XCTAssertNil(HistoryRow.statusSentence(for: "future_state", copy: nil))
        XCTAssertNil(HistoryRow.statusSentence(for: "accepted", copy: nil))
    }

    func testAnUnknownStatusOffersNoWithdraw() {
        XCTAssertFalse(ContributionStatusPresentation.offersWithdraw("future_state"))
        XCTAssertTrue(ContributionStatusPresentation.offersWithdraw("received"))
        XCTAssertTrue(ContributionStatusPresentation.offersWithdraw("rejected"))
    }
}
