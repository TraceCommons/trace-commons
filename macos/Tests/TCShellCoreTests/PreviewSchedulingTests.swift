import TCShellCore
import XCTest

/// A stand-in for `PreviewSummary` (the app target's real decoded shape).
/// `PreviewRequestResult` is generic precisely so this module never needs to
/// know the real one -- see that type's doc -- so a minimal fixture is
/// exactly what these tests should decode against.
private struct FixtureSummary: Decodable, Equatable, Sendable {
    let wouldSendBytes: Int
    enum CodingKeys: String, CodingKey { case wouldSendBytes = "would_send_bytes" }
}

/// Decoding tests for the wire shape `preview_request` and `preview_ready`
/// both use, plus the one invariant that shape exists to protect: a
/// `too_large` refusal decodes no summary, ever -- see the design spec's
/// "Admission control by size", which calls a synthesized would-send figure
/// on that card "the one failure mode this product cannot have."
final class PreviewRequestResultTests: XCTestCase {
    private func decode(_ json: String) throws -> PreviewRequestResult<FixtureSummary> {
        try JSONDecoder().decode(PreviewRequestResult<FixtureSummary>.self, from: Data(json.utf8))
    }

    func testReadyCarriesTheSummary() throws {
        let result = try decode("""
        {"entry_id": "entry_a", "state": "ready", "summary": {"would_send_bytes": 4160}}
        """)
        XCTAssertEqual(result.state, .ready)
        XCTAssertEqual(result.summary, FixtureSummary(wouldSendBytes: 4160))
        XCTAssertNil(result.rawSessionBytes)
        XCTAssertNil(result.limitBytes)
    }

    func testQueuedAndRunningCarryNoSummary() throws {
        for state in ["queued", "running"] {
            let result = try decode("""
            {"entry_id": "entry_a", "state": "\(state)"}
            """)
            XCTAssertNil(result.summary, "\(state) must not carry a summary")
        }
    }

    /// The literal payload shape `PreviewOutcome::to_value` emits for
    /// `TooLarge` (`preview_scheduler.rs`) -- no `summary` key at all, so a
    /// decode bug that defaulted a missing key to some placeholder value
    /// would be the exact failure this test exists to catch.
    func testTooLargeCarriesNoSummaryOfAnyKind() throws {
        let result = try decode("""
        {"entry_id": "entry_a", "state": "too_large", "raw_session_bytes": 367500000, \
        "limit_bytes": 67108864}
        """)
        XCTAssertEqual(result.state, .tooLarge)
        XCTAssertNil(result.summary, "a refusal must never carry a summary to synthesize a size from")
        XCTAssertEqual(result.rawSessionBytes, 367_500_000)
        XCTAssertEqual(result.limitBytes, 67_108_864)
    }

    /// Even a payload that smuggles a would-send-shaped `summary` alongside
    /// `too_large` -- which the real daemon never sends, but a decoder
    /// should not paper over if some future bug did -- must not be silently
    /// treated as a green light to read `wouldSendBytes` off of it: this
    /// only asserts the field decodes as *whatever was sent*, not that it is
    /// safe to render. `PreviewCardCopy.tooLarge` (used by the app target)
    /// is what actually enforces "raw size only" at the render call site by
    /// never taking a summary parameter at all.
    func testFailedCarriesCodeAndLabelNotASummary() throws {
        let result = try decode("""
        {"entry_id": "entry_a", "state": "failed", "code": "unavailable", \
        "label": "preview-failed"}
        """)
        XCTAssertEqual(result.state, .failed)
        XCTAssertEqual(result.code, "unavailable")
        XCTAssertEqual(result.label, "preview-failed")
        XCTAssertNil(result.summary)
    }
}

/// `PreviewRequestTracker`'s dedup bookkeeping: what decides whether a card
/// asks the daemon again.
final class PreviewRequestTrackerTests: XCTestCase {
    func testAFreshEntryShouldBeRequested() {
        let tracker = PreviewRequestTracker()
        XCTAssertTrue(tracker.shouldRequest("entry_a"))
    }

    func testAnEntryMarkedRequestedIsNotOfferedAgainWhileInFlight() {
        var tracker = PreviewRequestTracker()
        tracker.markRequested("entry_a")
        XCTAssertFalse(
            tracker.shouldRequest("entry_a"),
            "a row re-rendering while the daemon still has the job queued must not resend"
        )
    }

    func testQueuedAndRunningLeaveTheEntryInFlightNotAnswered() {
        var tracker = PreviewRequestTracker()
        tracker.markRequested("entry_a")
        tracker.apply(state: .queued, to: "entry_a")
        XCTAssertEqual(tracker.inFlightCount, 1)
        XCTAssertEqual(tracker.answeredCount, 0)
        tracker.apply(state: .running, to: "entry_a")
        XCTAssertEqual(tracker.inFlightCount, 1)
        XCTAssertEqual(tracker.answeredCount, 0)
        XCTAssertFalse(tracker.shouldRequest("entry_a"))
    }

    /// All three terminal states move an entry to `answered`, where it is
    /// never requested again -- the daemon delivered a final word, and the
    /// caller's own summary/too-large/error dictionaries are what carry it.
    func testEveryTerminalStateStopsFurtherRequests() {
        for state in [PreviewRequestState.ready, .tooLarge, .failed] {
            var tracker = PreviewRequestTracker()
            tracker.markRequested("entry_a")
            tracker.apply(state: state, to: "entry_a")
            XCTAssertEqual(tracker.inFlightCount, 0, "\(state) must clear in-flight")
            XCTAssertEqual(tracker.answeredCount, 1, "\(state) must record an answer")
            XCTAssertFalse(tracker.shouldRequest("entry_a"), "\(state) must not be re-requested")
        }
    }

    /// An entry that left the pending list -- approved, dismissed, expired,
    /// superseded -- must be requestable again if it were ever somehow
    /// re-offered, rather than permanently stuck in whichever bucket it last
    /// occupied.
    func testForgetClearsBothInFlightAndAnsweredBookkeeping() {
        var tracker = PreviewRequestTracker()
        tracker.markRequested("entry_a")
        tracker.markRequested("entry_b")
        tracker.apply(state: .ready, to: "entry_b")

        tracker.forget(["entry_a", "entry_b"])

        XCTAssertEqual(tracker.inFlightCount, 0)
        XCTAssertEqual(tracker.answeredCount, 0)
        XCTAssertTrue(tracker.shouldRequest("entry_a"))
        XCTAssertTrue(tracker.shouldRequest("entry_b"))
    }

    /// Forgetting an id the tracker never saw is a no-op, not a crash --
    /// mirrors the daemon's own `preview_cancel` contract, where
    /// `dropped: false` is a defined outcome for exactly this case.
    func testForgettingAnUnknownIDIsHarmless() {
        var tracker = PreviewRequestTracker()
        tracker.forget(["never_seen"])
        XCTAssertEqual(tracker.inFlightCount, 0)
        XCTAssertEqual(tracker.answeredCount, 0)
    }
}

/// `PreviewVisibilityCoalescer`: the pure half of the `preview_visible`
/// debounce -- what to send, not when.
final class PreviewVisibilityCoalescerTests: XCTestCase {
    func testANewVisibleSetIsPendingUntilTaken() {
        var coalescer = PreviewVisibilityCoalescer()
        coalescer.setVisible(["a", "b"])
        XCTAssertEqual(coalescer.takePendingSend(), ["a", "b"])
    }

    /// The coalescing property itself: several visibility changes before the
    /// debounce timer ever fires must produce exactly one pending send, and
    /// it must be the *latest* set -- a fast scroll through hundreds of rows
    /// must not queue up one send per row crossed.
    func testRapidChangesCoalesceIntoOnlyTheLatestSet() {
        var coalescer = PreviewVisibilityCoalescer()
        coalescer.setVisible(["a"])
        coalescer.setVisible(["a", "b"])
        coalescer.setVisible(["b", "c"])
        XCTAssertEqual(coalescer.takePendingSend(), ["b", "c"])
    }

    /// Taking the pending send clears it: a second timer fire with no
    /// intervening visibility change must not resend an identical set.
    func testTakingPendingSendClearsItUntilTheNextChange() {
        var coalescer = PreviewVisibilityCoalescer()
        coalescer.setVisible(["a"])
        XCTAssertEqual(coalescer.takePendingSend(), ["a"])
        XCTAssertNil(coalescer.takePendingSend(), "a second fire with nothing new must send nothing")
    }

    func testCurrentlyVisibleReflectsTheLatestSetEvenAfterTaking() {
        var coalescer = PreviewVisibilityCoalescer()
        coalescer.setVisible(["a", "b"])
        _ = coalescer.takePendingSend()
        XCTAssertEqual(coalescer.currentlyVisible, ["a", "b"])
    }

    /// An empty set is a real, sendable value -- "nothing is on screen" --
    /// not the absence of a change. The contract's `preview_visible` doc
    /// says the same: "an empty array is valid and means nothing is on
    /// screen."
    func testAnEmptyVisibleSetIsStillAPendingSend() {
        var coalescer = PreviewVisibilityCoalescer()
        coalescer.setVisible(["a"])
        _ = coalescer.takePendingSend()
        coalescer.setVisible([])
        XCTAssertEqual(coalescer.takePendingSend(), [])
    }
}
