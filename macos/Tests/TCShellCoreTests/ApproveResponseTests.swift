import TCShellCore
import XCTest

/// `ApproveResponse` decodes the daemon's `approve` reply exactly, against a
/// literal JSON fixture -- the shape it must match is
/// `crates/trace-commons-contributor/src/daemon/ipc.rs`, the `approve`
/// handler's `Response::ok` body:
///
/// ```
/// {"approved", "hold_secs", "hold_until", "flagged", "redactions", "skipped"}
/// ```
///
/// A decode bug here is invisible until a contributor hits it -- this shell
/// has already shipped one, in `DaemonClient.listProjects`, where `ask` was
/// spelled `"ask"` while the daemon actually sent `"notify_only"` and every
/// row silently failed to decode. Asserting a literal fixture, field by
/// field, is what catches the same class of mistake here.
final class ApproveResponseTests: XCTestCase {
    private func decode(_ json: String) throws -> ApproveResponse {
        try JSONDecoder().decode(ApproveResponse.self, from: Data(json.utf8))
    }

    func testDecodesTheDaemonsFullShape() throws {
        let response = try decode("""
        {
            "approved": 44,
            "hold_secs": 5,
            "hold_until": "2026-08-20T17:00:05Z",
            "flagged": 3,
            "redactions": {"aws_secret_key": 1, "local_path": 212},
            "skipped": [
                {"entry_id": "entry_a", "reason_label": "envelope-too-large"},
                {"entry_id": "entry_b", "reason_label": "envelope-too-large"},
                {"entry_id": "entry_c", "reason_label": "envelope-too-large"}
            ]
        }
        """)

        XCTAssertEqual(response.approved, 44)
        XCTAssertEqual(response.holdSecs, 5)
        XCTAssertEqual(response.holdUntil, "2026-08-20T17:00:05Z")
        XCTAssertEqual(response.flagged, 3)
        XCTAssertEqual(response.redactions, ["aws_secret_key": 1, "local_path": 212])
        XCTAssertEqual(
            response.skipped,
            [
                ApproveSkip(entryID: "entry_a", reasonLabel: "envelope-too-large"),
                ApproveSkip(entryID: "entry_b", reasonLabel: "envelope-too-large"),
                ApproveSkip(entryID: "entry_c", reasonLabel: "envelope-too-large"),
            ]
        )
    }

    /// `hold_until` is `null` whenever nothing was approved, or the hold is
    /// configured off -- the daemon says so explicitly (`ipc.rs`, the
    /// `hold_until` comment in the `approve` handler). Decoding that as a
    /// present-but-empty string instead of `nil` would tell a caller undo
    /// has a deadline that does not exist.
    func testHoldUntilIsNilRatherThanAnEmptyString() throws {
        let response = try decode("""
        {
            "approved": 0,
            "hold_secs": 5,
            "hold_until": null,
            "flagged": 0,
            "redactions": {},
            "skipped": [
                {"entry_id": "entry_a", "reason_label": "not-pending"},
                {"entry_id": "entry_b", "reason_label": "not-pending"}
            ]
        }
        """)

        XCTAssertNil(response.holdUntil)
        XCTAssertEqual(response.approved, 0)
        XCTAssertEqual(response.redactions, [:])
    }

    func testTotalRedactionsSumsTheMapRatherThanCountingCategories() throws {
        let response = try decode("""
        {"approved": 1, "hold_secs": 5, "hold_until": null, "flagged": 0,
         "redactions": {"aws_secret_key": 1, "local_path": 3}, "skipped": []}
        """)

        XCTAssertEqual(response.totalRedactions, 4)
    }

    func testToastMatchesSubmitToastRenderedDirectly() throws {
        let response = try decode("""
        {"approved": 44, "hold_secs": 5, "hold_until": "2026-08-20T17:00:05Z", "flagged": 3,
         "redactions": {"aws_secret_key": 1, "local_path": 212},
         "skipped": [
            {"entry_id": "entry_a", "reason_label": "envelope-too-large"},
            {"entry_id": "entry_b", "reason_label": "envelope-too-large"},
            {"entry_id": "entry_c", "reason_label": "envelope-too-large"}
         ]}
        """)

        // Asserted against the renderer's own output, never a literal: the
        // wording is `SubmitToast`'s contract to keep, not this file's --
        // see the note on that type.
        XCTAssertEqual(
            response.toast,
            SubmitToast.render(
                approved: 44,
                redactions: 213,
                flagged: 3,
                skipped: ["envelope-too-large", "envelope-too-large", "envelope-too-large"]
            )
        )
        XCTAssertTrue(response.toast.offerUndo)
    }

    /// `approve` reports a count, not a list -- `approvedEntryIDs` is what a
    /// caller uses to recover which of the ids it attempted actually went
    /// through, for `cancel` to drive.
    func testApprovedEntryIDsIsAttemptedMinusSkipped() throws {
        let response = try decode("""
        {"approved": 2, "hold_secs": 5, "hold_until": "2026-08-20T17:00:05Z", "flagged": 0,
         "redactions": {},
         "skipped": [{"entry_id": "entry_b", "reason_label": "not-pending"}]}
        """)

        XCTAssertEqual(
            response.approvedEntryIDs(attempted: ["entry_a", "entry_b", "entry_c"]),
            ["entry_a", "entry_c"]
        )
    }

    func testApprovedEntryIDsIsEmptyWhenEverythingAttemptedWasSkipped() throws {
        let response = try decode("""
        {"approved": 0, "hold_secs": 5, "hold_until": null, "flagged": 0,
         "redactions": {},
         "skipped": [
            {"entry_id": "entry_a", "reason_label": "not-pending"},
            {"entry_id": "entry_b", "reason_label": "not-pending"}
         ]}
        """)

        XCTAssertEqual(response.approvedEntryIDs(attempted: ["entry_a", "entry_b"]), [])
    }

    /// A label this build has never been taught still decodes -- the
    /// forward-compatibility row in the spec's skip table
    /// (`docs/superpowers/specs/2026-08-20-one-click-submit-design.md`)
    /// applies to rendering, not decoding, and decoding must not throw on a
    /// daemon newer than the shell.
    func testAnUnrecognisedSkipReasonStillDecodes() throws {
        let response = try decode("""
        {"approved": 0, "hold_secs": 5, "hold_until": null, "flagged": 0,
         "redactions": {},
         "skipped": [{"entry_id": "entry_a", "reason_label": "some-future-reason"}]}
        """)

        XCTAssertEqual(response.skipped.first?.reasonLabel, "some-future-reason")
        // Against the renderer, not a literal -- see the note above.
        XCTAssertEqual(
            response.toast,
            SubmitToast.render(approved: 0, redactions: 0, flagged: 0, skipped: ["some-future-reason"])
        )
    }
}
