import TCBridge
import XCTest

@testable import TraceCommonsApp

/// An unrecognised contribution status reads "Status unavailable" -- the
/// core's `history_copy::STATUS_UNAVAILABLE`, carried by
/// `tc_public_run_copy` -- on every macOS surface. It is never "Not in the
/// commons" (a claim about where the trace is) and never the raw wire token.
final class UnknownContributionStatusTests: XCTestCase {
    private func sharedCopy() throws -> PublicRunCopy {
        try XCTUnwrap(PublicRunCopy.decode(fromJSON: TCPublicRun.copyJSON() ?? ""))
    }

    func testLegacyHistoryLabelsAnUnknownStatusFromTheCore() throws {
        let unavailable = try sharedCopy().contributionStatusUnavailable
        for status in ["future_state", "rejected", "revoked", ""] {
            XCTAssertEqual(
                HistoryRow.statusSentence(for: status, unavailable: unavailable),
                "Status unavailable", status)
        }
    }

    func testLegacyHistoryKeepsItsKnownLabels() {
        XCTAssertEqual(HistoryRow.statusSentence(for: "accepted", unavailable: nil), "In the commons")
        XCTAssertEqual(
            HistoryRow.statusSentence(for: "submitted", unavailable: nil), "Waiting to be scored")
        XCTAssertEqual(
            HistoryRow.statusSentence(for: "quarantined", unavailable: nil), "Held for privacy review")
        XCTAssertEqual(
            HistoryRow.statusSentence(for: "withdrawn", unavailable: nil), "Withdrawn by you")
    }

    func testLegacyHistoryShowsNoTagRatherThanATypedOneWithoutTheCore() {
        XCTAssertNil(HistoryRow.statusSentence(for: "future_state", unavailable: nil))
    }

    func testGlassHomeLabelsAnUnknownStatusFromTheCore() throws {
        let copy = try sharedCopy()
        XCTAssertEqual(
            HomeFormat.statusWord(
                "future_state", table: nil, fallback: { copy.contributionStatusLabel(for: $0) }),
            "Status unavailable")
    }

    func testGlassHomeNeverShowsTheRawToken() {
        // With no core copy decoded, the row draws no tag at all.
        XCTAssertNil(HomeFormat.statusWord("future_state", table: nil, fallback: { _ in nil }))
    }

    func testAnUnknownStatusOffersNoWithdraw() {
        XCTAssertFalse(ContributionStatusPresentation.offersWithdraw("future_state"))
        XCTAssertTrue(ContributionStatusPresentation.offersWithdraw("received"))
        XCTAssertTrue(ContributionStatusPresentation.offersWithdraw("rejected"))
    }
}
