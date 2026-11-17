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
    func testGroupOutcomeExplainsHeldTracesWithoutClaimingApproval() throws {
        let words = try JSONDecoder().decode(MonitorUndoCopy.self, from: Data(#"{"approval_saved":"APPROVAL SAVED","approved":"{label} approved","unavailable":"Undo unavailable","within":"Undo within {seconds}s","may_have_started":"Upload may already have started.","undoing":"Undoing","redaction_begun":"Data redaction has begun.","held_for_review_one":"1 trace needs individual review.","held_for_review":"{count} traces need individual review."}"#.utf8))
        for approved: UInt64 in [0, 2] {
            let response = ApproveResponse(approved: approved, flagged: 0, redactions: [:], skipped: [],
                                           holdSecs: 10, holdUntil: nil, excludedHeld: 3)
            let toast = response.toast(undoCopy: words)
            XCTAssertEqual(
                toast.line,
                approved == 0
                    ? "Nothing approved. 3 traces need individual review."
                    : "Approved 2. Scrubbing matched nothing. 3 traces need individual review.")
            XCTAssertEqual(toast.offerUndo, approved > 0)
        }
        let singular = ApproveResponse(approved: 0, flagged: 0, redactions: [:], skipped: [],
                                       holdSecs: 10, holdUntil: nil, excludedHeld: 1)
        XCTAssertEqual(singular.toast(undoCopy: words).line, "Nothing approved. 1 trace needs individual review.")
    }

    func testHeldOnlyGroupOmitsScrubClaimWithoutCopyAndPreservesOtherOutcomes() {
        let heldOnly = ApproveResponse(approved: 0, flagged: 0, redactions: [:], skipped: [],
                                      holdSecs: 10, holdUntil: nil, excludedHeld: 30)
        XCTAssertEqual(heldOnly.toast(undoCopy: nil), SubmitToast(line: "Nothing approved.", offerUndo: false))

        // These responses include work or an ordinary non-group outcome.
        // They keep the existing scrub accounting and Undo contract.
        let processed: [ApproveResponse] = [
            .init(approved: 1, flagged: 0, redactions: [:], skipped: [], holdSecs: 10,
                  holdUntil: nil, excludedHeld: 30),
            .init(approved: 0, flagged: 1, redactions: [:], skipped: [], holdSecs: 10,
                  holdUntil: nil, excludedHeld: 30),
            .init(approved: 0, flagged: 0, redactions: ["local_path": 1], skipped: [], holdSecs: 10,
                  holdUntil: nil, excludedHeld: 30),
            .init(approved: 0, flagged: 0, redactions: ["local_path": 0], skipped: [], holdSecs: 10,
                  holdUntil: nil, excludedHeld: 30),
            .init(approved: 0, flagged: 0, redactions: [:],
                  skipped: [.init(entryID: "already-decided", reasonLabel: "not-pending")],
                  holdSecs: 10, holdUntil: nil, excludedHeld: 30),
            .init(approved: 0, flagged: 0, redactions: [:], skipped: [], holdSecs: 10,
                  holdUntil: nil, excludedHeld: 0),
            .init(approved: 0, flagged: 0, redactions: [:], skipped: [], holdSecs: 10,
                  holdUntil: nil),
        ]
        for response in processed {
            XCTAssertEqual(response.toast(undoCopy: nil), response.toast)
        }
    }

    func testWitnessReviewWithoutLocalCountsDoesNotClaimAScrubResult() throws {
        let approved = ApproveResponse(approved: 1, flagged: 0, redactions: [:], skipped: [],
                                       holdSecs: 10, holdUntil: nil, witnessReview: true)
        XCTAssertEqual(approved.toast, SubmitToast(line: "Approved.", offerUndo: true))
        XCTAssertEqual(approved.toast(undoCopy: nil), approved.toast)

        let mixed = ApproveResponse(approved: 2, flagged: 1, redactions: [:],
                                    skipped: [.init(entryID: "already-decided", reasonLabel: "not-pending")],
                                    holdSecs: 10, holdUntil: nil, witnessReview: true)
        XCTAssertEqual(mixed.toast.line, "Approved 2. 1 flagged, 1 not approved: already decided.")
        XCTAssertTrue(mixed.toast.offerUndo)

        let words = try JSONDecoder().decode(MonitorUndoCopy.self, from: Data(#"{"approval_saved":"APPROVAL SAVED","approved":"{label} approved","unavailable":"Undo unavailable","within":"Undo within {seconds}s","may_have_started":"Upload may already have started.","undoing":"Undoing","redaction_begun":"Data redaction has begun.","held_for_review_one":"1 trace needs individual review.","held_for_review":"{count} traces need individual review."}"#.utf8))
        let group = ApproveResponse(approved: 1, flagged: 0, redactions: [:], skipped: [],
                                    holdSecs: 10, holdUntil: nil, excludedHeld: 2, witnessReview: true)
        XCTAssertEqual(group.toast(undoCopy: words),
                       SubmitToast(line: "Approved. 2 traces need individual review.", offerUndo: true))
    }

    func testWitnessReviewRetainsAvailableScrubCountsAndOrdinaryZeroResults() {
        let localCounts = ApproveResponse(approved: 1, flagged: 0, redactions: ["local_path": 2], skipped: [],
                                         holdSecs: 10, holdUntil: nil, witnessReview: true)
        XCTAssertEqual(localCounts.toast.line, "Approved. 2 redactions applied.")
        let zeroCount = ApproveResponse(approved: 1, flagged: 0, redactions: ["local_path": 0], skipped: [],
                                       holdSecs: 10, holdUntil: nil, witnessReview: true)
        XCTAssertEqual(zeroCount.toast.line, "Approved. Scrubbing matched nothing.")
        for witness: Bool? in [false, nil] {
            let ordinary = ApproveResponse(approved: 1, flagged: 0, redactions: [:], skipped: [],
                                           holdSecs: 10, holdUntil: nil, witnessReview: witness)
            XCTAssertEqual(ordinary.toast.line, "Approved. Scrubbing matched nothing.")
            XCTAssertTrue(ordinary.toast.offerUndo)
        }
    }

    func testTheSpecWorkedExamplesRenderExactly() {
        XCTAssertEqual(
            SubmitToast.render(approved: 1, redactions: 4, flagged: 1, skipped: []).line,
            "Approved. 4 redactions applied. 1 flagged."
        )
        XCTAssertEqual(
            SubmitToast.render(approved: 47, redactions: 213, flagged: 3, skipped: []).line,
            "Approved 47. 213 redactions applied. 3 flagged."
        )
        XCTAssertEqual(
            SubmitToast.render(
                approved: 44,
                redactions: 213,
                flagged: 3,
                skipped: ["envelope-too-large", "envelope-too-large", "envelope-too-large"]
            ).line,
            "Approved 44. 213 redactions applied. 3 flagged, 3 not approved: too large to send."
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
            "Approved 2. 1 redaction applied."
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
            "Approved 44. 213 redactions applied. 3 flagged, 1 not approved: too large to send."
        )
    }
}
