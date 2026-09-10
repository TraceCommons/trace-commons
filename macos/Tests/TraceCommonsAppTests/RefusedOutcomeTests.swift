import XCTest

@testable import TraceCommonsApp

/// A contribution the commons refused must not read as one that never left.
///
/// This shell's outcome default is `Held` -- not false, but the wrong shape:
/// it reads as a transient state that will resolve, and a standing refusal
/// will not. The Linux shell's default was outright false. See #810.
///
/// **On the spelling.** These five labels are the server's own, with
/// underscores; `daemon::health`'s constants for the same events are
/// hyphenated. Production code never spells them -- it passes through
/// whatever the daemon sent -- so the only place a wrong spelling could hide
/// is here, and it hides in the safe direction: a label the ABI does not
/// recognise falls through to `Held`, which fails these assertions rather
/// than passing them.
final class RefusedOutcomeTests: XCTestCase {
    private static let refusalLabels = [
        "admission_refused",
        "admission_limit_reached",
        "admission_in_progress",
        "admission_identity_conflict",
        "admission_evidence_refused",
    ]

    func testARefusedContributionDoesNotReadAsUnsentOrAsHeld() {
        let held = OutcomeCopy.sentence(for: "a-label-this-shell-has-never-seen")
        XCTAssertEqual(held, "Held", "the default arm moved; this test's baseline is stale")

        for label in Self.refusalLabels {
            let line = OutcomeCopy.sentence(for: label)
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
        let line = OutcomeCopy.sentence(for: "admission_in_progress").lowercased()
        for refused in ["declined", "refused", "turned away", "rejected"] {
            XCTAssertFalse(line.contains(refused), "a held lease reads as a refusal: \(line)")
        }
    }

    /// The lookup is additive: labels this shell already answered are
    /// untouched.
    func testTheExistingOutcomeSentencesAreUnchanged() {
        XCTAssertEqual(OutcomeCopy.sentence(for: "dismissed-by-contributor"), "You said no thanks")
        XCTAssertEqual(OutcomeCopy.sentence(for: "queue-full"), "Queue was full")
    }
}
