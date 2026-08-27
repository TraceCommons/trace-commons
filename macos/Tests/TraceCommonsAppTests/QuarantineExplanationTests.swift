import XCTest

@testable import TraceCommonsApp

/// Covers what the held-for-privacy-review section is allowed to print.
///
/// The section spans every held trace at once, while the server writes its
/// explanation lines per RECORD. A flat list of them therefore repeated the
/// same two sentences once per trace -- on a real account with 210 held
/// traces that was 420 lines, two of them distinct, and every other line an
/// opaque tenant digest.
final class QuarantineExplanationTests: XCTestCase {
    private func record(_ explanations: [String]) -> HistoryRecord {
        HistoryRecord(
            submissionID: UUID().uuidString,
            submittedAt: Date(timeIntervalSince1970: 0),
            projectLabel: "demo",
            source: "claude-code",
            status: "quarantined",
            consentScopes: [],
            creditPointsPending: 0,
            creditPointsFinal: nil,
            explanations: explanations,
            lastRefreshedAt: nil
        )
    }

    /// The repetition this section actually shipped: the same server sentence
    /// once per held trace.
    func testIdenticalLinesAcrossRecordsCollapseToOne() {
        let held = (0..<210).map { _ in
            record(["Quarantined for privacy review; credit is pending review."])
        }
        let shown = HistoryView.contributorFacingExplanations(in: held)
        XCTAssertEqual(
            shown, ["Quarantined for privacy review; credit is pending review."],
            "210 records saying the same thing must print that thing once")
    }

    /// `Attributed to tenant tenant_sha256:<64 hex>` rides on every receipt.
    /// It is true and unreadable, and it was printed above the sentence that
    /// says what happened.
    func testOpaqueDigestLinesAreNotShown() {
        let shown = HistoryView.contributorFacingExplanations(in: [
            record([
                "Quarantined for privacy review; credit is pending review.",
                "Attributed to tenant tenant_sha256:8719ab8d740b9882d27c80f473bfe5b1",
            ])
        ])
        XCTAssertEqual(shown, ["Quarantined for privacy review; credit is pending review."])
    }

    /// Distinct reasons are all worth showing -- the filter is for repetition
    /// and digests, not for brevity. Order is first-seen so the earliest
    /// reason reads first.
    func testDistinctReasonsAreAllKeptInFirstSeenOrder() {
        let shown = HistoryView.contributorFacingExplanations(in: [
            record(["Quarantined for privacy review; credit is pending review."]),
            record(["Held pending an automated privacy backstop verdict; not yet in the corpus."]),
            record(["Quarantined for privacy review; credit is pending review."]),
        ])
        XCTAssertEqual(
            shown,
            [
                "Quarantined for privacy review; credit is pending review.",
                "Held pending an automated privacy backstop verdict; not yet in the corpus.",
            ])
    }

    /// A record carrying nothing but a digest contributes nothing, rather
    /// than contributing an empty line.
    func testARecordOfOnlyDigestsContributesNothing() {
        let shown = HistoryView.contributorFacingExplanations(in: [
            record(["Attributed to tenant tenant_sha256:8719ab8d740b9882d27c80f473bfe5b1"])
        ])
        XCTAssertTrue(shown.isEmpty)
    }

    /// The held copy must not tell a contributor that a person is reading
    /// their session. An agent inspects these; saying otherwise is both wrong
    /// and the more alarming of the two readings.
    func testHeldCopyDoesNotClaimAHumanReader() {
        let copy = HistoryView.heldReviewBody.lowercased()
        for forbidden in ["a person at", "someone at", "our team", "a human", "staff", "the reviewer"]
        {
            XCTAssertFalse(
                copy.contains(forbidden),
                "held copy must not imply a human reads these: \(forbidden)")
        }
        XCTAssertTrue(
            copy.contains("agent"), "held copy must say what actually inspects a held trace")
        // The denial this section exists to carry, and no promised wait.
        XCTAssertTrue(copy.contains("have not been rejected"))
        for forbidden in ["48 hours", "business days", "within a week", "usually takes"] {
            XCTAssertFalse(copy.contains(forbidden), "no turnaround time may be stated")
        }
    }
}
