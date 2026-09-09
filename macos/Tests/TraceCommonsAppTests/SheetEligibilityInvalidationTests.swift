import Combine
import SwiftUI
import XCTest

@testable import TCShellCore
@testable import TraceCommonsApp

/// Does the open preview sheet actually re-read the eligibility gate when
/// the queue changes?
///
/// Resolving the entry live is only half of it. A correct getter nobody asks
/// again is worth nothing: on a pull-bound shell the footer keeps drawing
/// what it last read, and the button stays visibly armed while the getter
/// would now answer correctly. Windows had to raise a change notification to
/// close that half.
///
/// macOS closes it structurally, and these tests are what establishes that
/// rather than assuming it. `AppModel` is an `ObservableObject`, so its
/// synthesized `objectWillChange` fires on EVERY `@Published` mutation, and
/// `PreviewSheet` holds the model as an `@EnvironmentObject` -- which
/// subscribes to that publisher regardless of which properties the body
/// happens to read. A queue republish therefore invalidates the sheet and
/// its body runs again, which is when `EligibilitySurface.current` is asked
/// a second time.
final class SheetEligibilityInvalidationTests: XCTestCase {
    private var cancellables: Set<AnyCancellable> = []

    private func entry(_ id: String, eligibility: String?, reason: String? = nil) -> QueueEntry {
        QueueEntry(
            entryID: id, sessionHash: "sha256:\(id)", source: "claude_code",
            declaredSource: nil, projectID: "p1", projectLabel: "repo",
            projectPath: "~/code/repo", sessionPath: nil, sizeBytes: 10,
            discoveredAt: Date(timeIntervalSince1970: 0), state: .pending,
            reasonLabel: nil, attempts: 0, subagentCount: nil, subagentsDropped: nil,
            eligibility: eligibility, eligibilityReason: reason,
            // Every entry carries a mark, this fixture's included: the
            // eligibility question it is really about is the one that
            // varies, and the mark does not.
            attestation: "attested", attestationReason: nil)
    }

    /// The signal exists: a queue republish fires the publisher every view
    /// holding this model is subscribed to.
    ///
    /// This is the macOS equivalent of the raise Windows had to add. It is
    /// asserted rather than assumed because the whole finding is that a
    /// gate can be correct and never consulted again.
    @MainActor
    func testAQueueRepublishFiresTheModelsChangePublisher() {
        let model = AppModel()
        var fired = 0
        model.objectWillChange.sink { _ in fired += 1 }.store(in: &cancellables)

        model.applyPendingUpdate([entry("e1", eligibility: "eligible")])
        XCTAssertGreaterThan(fired, 0, "a snapshot must invalidate every view holding this model")
    }

    /// And the value behind the gate really moves.
    ///
    /// Together with the publisher firing, this is the whole chain: the
    /// sheet is invalidated, its body runs again, and when it re-resolves
    /// the held entry it gets a different answer than it got at open time.
    @MainActor
    func testTheGatesAnswerChangesWhenTheQueueDowngradesTheRow() {
        let model = AppModel()
        let held = entry("e1", eligibility: "eligible")
        model.applyPendingUpdate([held])

        // What the sheet resolves at open time.
        let atOpen = EligibilitySurface.current(
            held, in: model.awaitingDecision, id: \.entryID)
        XCTAssertEqual(atOpen.contributionEligibility?.state, "eligible")

        // The snapshot that carries a submit-time failure written back into
        // the row -- Decision 1 of the design.
        model.applyPendingUpdate([
            entry("e1", eligibility: "ineligible_permanent", reason: "digest_mismatch")
        ])

        let afterSnapshot = EligibilitySurface.current(
            held, in: model.awaitingDecision, id: \.entryID)
        XCTAssertEqual(afterSnapshot.contributionEligibility?.state, "ineligible_permanent")
        XCTAssertEqual(afterSnapshot.contributionEligibility?.reason, "digest_mismatch")

        // The held copy is unchanged, so this fails if the sheet ever goes
        // back to reading `entry` directly.
        XCTAssertEqual(held.contributionEligibility?.state, "eligible")
        XCTAssertFalse(
            EligibilitySurface.offersContribute(
                afterSnapshot.contributionEligibility, calls: model.eligibilityCalls),
            "the queue has downgraded this row; the sheet must stop offering to send it")
        XCTAssertTrue(
            EligibilitySurface.offersContribute(
                held.contributionEligibility, calls: model.eligibilityCalls),
            "the opening copy still says yes, which is what made the staleness a bypass")
    }

    // MARK: - The press

    /// **The press decides what is sent.** A single approval is refused when
    /// the queue has downgraded the row since the control was drawn.
    ///
    /// Observed through the completion callback, which is the one thing this
    /// path does synchronously. `perform` needs a daemon client and there is
    /// none here, so an approval that got past the guard reaches the wire
    /// and calls nothing back; one the guard refused calls back at once.
    /// Deleting the guard therefore makes this test hang up on its own
    /// expectation rather than pass quietly.
    @MainActor
    func testASingleApprovalIsRefusedOnARowTheQueueHasDowngraded() {
        let model = AppModel()
        let armed = entry("e1", eligibility: "eligible")
        model.applyPendingUpdate([
            entry("e1", eligibility: "ineligible_permanent", reason: "digest_mismatch")
        ])

        var refused = false
        model.approve(armed) { _ in refused = true }
        XCTAssertTrue(
            refused,
            "the queue downgraded this row after the button was drawn; the press must decline")
    }

    /// And an ordinary press is NOT refused by the guard.
    ///
    /// The other half, so the test above cannot be satisfied by a guard that
    /// refuses everything: here the callback must NOT fire synchronously,
    /// because the approval got through to a wire that is not there.
    @MainActor
    func testAnApprovalOnAStillEligibleRowIsNotRefusedByTheGuard() {
        let model = AppModel()
        let eligible = entry("e1", eligibility: "eligible")
        model.applyPendingUpdate([eligible])

        var refused = false
        model.approve(eligible) { _ in refused = true }
        XCTAssertFalse(refused, "an eligible row must not be declined at the press")
    }

    /// An invited contributor's row has no eligibility question and is never
    /// declined at the press.
    @MainActor
    func testAnInvitedContributorsPressIsNeverRefused() {
        let model = AppModel()
        let invited = entry("e1", eligibility: nil)
        model.applyPendingUpdate([invited])

        var refused = false
        model.approve(invited) { _ in refused = true }
        XCTAssertFalse(refused)
    }

    /// The sheet's body, hosted for real, draws differently before and after
    /// the queue downgrades the row.
    ///
    /// The end-to-end version of the two tests above: not "the getter would
    /// answer correctly" but "the pixels changed". `NSHostingView` runs the
    /// same observation machinery the app does, so a sheet that had no
    /// dependency on the queue would render identically twice and fail here.
    @MainActor
    func testTheHostedSheetRedrawsAfterTheQueueDowngradesTheRow() throws {
        _ = NSApplication.shared
        let model = AppModel()
        let held = entry("e1", eligibility: "eligible")
        model.applyPendingUpdate([held])

        let summary = PreviewSummary(
            wouldSendBytes: 80, rawSessionBytes: 100, eventCount: 2,
            openingPrompt: "Review a synthetic session", redactions: [:], redactionsDistinct: [:],
            piiLabelsPresent: [], consentScopes: [], residualRisk: "low")
        let view = PreviewSheet(
            entry: held,
            preloaded: .init(
                summary: summary, transcript: "Synthetic transcript", needle: "", offsets: [])
        ).environmentObject(model)

        let size = CGSize(width: 900, height: 700)
        let hosting = NSHostingView(rootView: view.frame(width: size.width, height: size.height))
        let bounds = NSRect(origin: .zero, size: size)
        hosting.frame = bounds

        func render() throws -> Data {
            hosting.layoutSubtreeIfNeeded()
            let bitmap = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: bounds))
            hosting.cacheDisplay(in: bounds, to: bitmap)
            return try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        }

        let eligible = try render()
        model.applyPendingUpdate([
            entry("e1", eligibility: "ineligible_permanent", reason: "no_inference_call")
        ])
        let downgraded = try render()

        XCTAssertNotEqual(
            eligible, downgraded,
            "the sheet drew the same thing after the queue downgraded the row -- it is not "
                + "re-reading the eligibility gate, and Contribute stays armed on a session "
                + "that cannot be sent")
    }
}
