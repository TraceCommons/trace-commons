import XCTest
import TCBridge

@testable import TraceCommonsApp

/// Shared queue wording must preserve admission outcomes and unknown send state.
final class RefusedOutcomeTests: XCTestCase {
    private static let refusalLabels = [
        "admission_refused",
        "admission_limit_reached",
        "admission_in_progress",
        "admission_identity_conflict",
        "admission_evidence_refused",
    ]

    func testARefusedContributionDoesNotReadAsUnsentOrAsHeld() {
        let held = TCOutcome.line(label: "a-label-this-shell-has-never-seen")
        XCTAssertEqual(held, "Status unavailable", "the default arm moved; this test's baseline is stale")

        for label in Self.refusalLabels {
            let line = TCOutcome.line(label: label)
            XCTAssertNotEqual(
                line, held,
                "\(label) fell through to the default arm, so nothing was fixed for it")
            XCTAssertFalse(
                line.lowercased().contains("nothing was sent"),
                "\(label) tells a contributor their work never left: \(line)")
        }
    }

    /// A lease another attempt holds is not a refusal and must not read as
    /// one. It is the label most easily swept in with the others.
    func testALeaseAnotherAttemptHoldsDoesNotReadAsARefusal() {
        let line = TCOutcome.line(label: "admission_in_progress").lowercased()
        for refused in ["declined", "refused", "turned away", "rejected"] {
            XCTAssertFalse(line.contains(refused), "a held lease reads as a refusal: \(line)")
        }
    }

    /// The lookup is additive: labels this shell already answered are
    /// untouched.
    func testTheExistingOutcomeSentencesAreUnchanged() {
        XCTAssertEqual(TCOutcome.line(label: "dismissed-by-contributor"), "Skipped; not sent")
        XCTAssertEqual(TCOutcome.line(label: "queue-full"), "Queue was full")
    }
}
