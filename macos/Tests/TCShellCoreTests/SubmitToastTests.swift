import TCShellCore
import XCTest

/// The submit toast, checked against the spec's own worked examples.
///
/// These are the same assertions the Linux shell holds in
/// `crates/trace-commons-contributor-gtk/src/toast.rs` and the Windows shell
/// in `windows/tests/TraceCommons.Interop.Tests/SubmitToastTests.cs`. Three
/// shells render this sentence, the whole point of the spec's "The toast:
/// normative copy" section is that they render it identically, and the only
/// thing that actually holds three languages to one sentence is the same
/// examples asserted in each of them.
///
/// Nothing here needs a display, a bundle or the FFI dylib, which is why the
/// renderer lives in `TCShellCore`.
final class SubmitToastTests: XCTestCase {
    func testTheSpecWorkedExamplesRenderExactly() {
        XCTAssertEqual(
            SubmitToast.render(approved: 1, redactions: 4, flagged: 1, skipped: []).line,
            "Approved. Scrubbing removed 4. 1 flagged."
        )
        XCTAssertEqual(
            SubmitToast.render(approved: 47, redactions: 213, flagged: 3, skipped: []).line,
            "Approved 47. Scrubbing removed 213. 3 flagged."
        )
        XCTAssertEqual(
            SubmitToast.render(
                approved: 44,
                redactions: 213,
                flagged: 3,
                skipped: ["envelope-too-large", "envelope-too-large", "envelope-too-large"]
            ).line,
            "Approved 44. Scrubbing removed 213. 3 flagged, 3 not approved: too large to send."
        )
        XCTAssertEqual(
            SubmitToast.render(
                approved: 0, redactions: 0, flagged: 0, skipped: ["not-pending", "not-pending"]
            ).line,
            "Nothing approved. Scrubbing matched nothing. 2 not approved: already decided."
        )
    }

    func testUndoIsOfferedOnlyWhenSomethingWasApproved() {
        XCTAssertTrue(SubmitToast.render(approved: 1, redactions: 0, flagged: 0, skipped: []).offerUndo)
        XCTAssertFalse(
            SubmitToast.render(approved: 0, redactions: 0, flagged: 0, skipped: ["not-pending"])
                .offerUndo
        )
    }

    func testAWireLabelNeverReachesTheContributor() {
        let line = SubmitToast.render(
            approved: 0, redactions: 0, flagged: 0, skipped: ["envelope-too-large"]
        ).line
        XCTAssertFalse(line.contains("envelope-too-large"), "wire label leaked: \(line)")
        XCTAssertTrue(line.contains("too large to send"))
    }

    /// Every wire label the daemon can send, and one it cannot.
    ///
    /// The unknown case is the one that matters: a label this shell has not
    /// been taught is still a label, and echoing it would put daemon
    /// vocabulary in front of a contributor at exactly the moment the shell
    /// is least sure what happened.
    func testEveryWireLabelMapsToAHumanOne() {
        for wire in [
            "not-enrolled",
            "not-pending",
            "not-pinned",
            "envelope-too-large",
            "session-file-vanished",
            "preview-failed",
            "some-label-this-shell-has-never-seen",
        ] {
            let line = SubmitToast.render(approved: 0, redactions: 0, flagged: 0, skipped: [wire])
                .line
            XCTAssertFalse(line.contains(wire), "wire label leaked: \(line)")
        }
    }

    /// The clause is a count of entries and a list of distinct reasons, and
    /// those are two different numbers whenever entries share a reason.
    func testDistinctReasonsAreListedOnceInTheSpecsOrder() {
        XCTAssertEqual(
            SubmitToast.render(
                approved: 0,
                redactions: 0,
                flagged: 0,
                skipped: ["preview-failed", "not-pending", "not-enrolled", "not-pending"]
            ).line,
            "Nothing approved. Scrubbing matched nothing. 4 not approved: not connected to a commons, "
                + "already decided, could not be read."
        )
    }

    /// The unrecognised-label fallback happens to share its rendered text
    /// with `not-pinned`'s own label ("could not be prepared"). A batch
    /// that skips entries for both reasons must still print that text only
    /// once, and must still count every skipped entry.
    func testTheUnknownFallbackDoesNotDuplicateAcollidingLabel() {
        XCTAssertEqual(
            SubmitToast.render(
                approved: 0,
                redactions: 0,
                flagged: 0,
                skipped: ["not-pinned", "some-label-this-shell-has-never-seen"]
            ).line,
            "Nothing approved. Scrubbing matched nothing. 2 not approved: could not be prepared."
        )
    }

    /// Clauses 3 and 4 appear only when non-zero; clauses 1 and 2 always do.
    func testTheOptionalClausesAreAbsentWhenEmpty() {
        XCTAssertEqual(
            SubmitToast.render(approved: 1, redactions: 0, flagged: 0, skipped: []).line,
            "Approved. Scrubbing matched nothing."
        )
        XCTAssertEqual(
            SubmitToast.render(approved: 2, redactions: 1, flagged: 0, skipped: []).line,
            "Approved 2. Scrubbing removed 1."
        )
    }

    /// A zero redaction count is a fact the contributor is owed, not an
    /// absence to omit -- a session that obviously touched a `.env` and
    /// reports nothing removed is a signal.
    func testScrubbingReportsZeroRatherThanSayingNothing() {
        XCTAssertTrue(
            SubmitToast.render(approved: 1, redactions: 0, flagged: 0, skipped: []).line
                .contains("Scrubbing matched nothing")
        )
    }

    /// Mutation guard: dropping the flagged half of the joined clause
    /// leaves compiling, wrong code. See the follow-up report for the
    /// mutation applied and reverted while verifying this test.
    func testTheJoinedClauseCarriesBothHalvesWhenBothApply() {
        let line = SubmitToast.render(
            approved: 44, redactions: 213, flagged: 3, skipped: ["envelope-too-large"]
        ).line
        XCTAssertEqual(
            line,
            "Approved 44. Scrubbing removed 213. 3 flagged, 1 not approved: too large to send."
        )
    }
}
