import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// New Terms of Service after onboarding (spec section 5): Home's notice and
/// the menu bar's row appear only while the daemon holds uploads for them,
/// have no close action, and raise the sheet; accepting closes the sheet,
/// and a version no longer published keeps it up and loads the terms again.
@MainActor
final class TermsHoldTests: XCTestCase {
    private func held(_ on: Bool) -> DaemonStatus {
        DaemonStatus(
            schemaVersion: "1.1", loggedIn: true, tenantID: "tenant-a", consentScopes: [],
            paused: false, termsHold: on, queueDepth: 0, nextDigestAt: nil,
            health: DaemonHealth(lastErrorLabel: nil, since: nil))
    }

    private static let accepted = AcceptedTerms(
        version: "2026-12-01", contentSHA256: String(repeating: "c", count: 64),
        acceptedAt: Date(timeIntervalSince1970: 1_762_689_600))

    private func words() throws -> FirstRunCopy.TermsHold {
        try XCTUnwrap(TermsWords.hold)
    }

    func testNoticeShownOnlyWhileHeld() throws {
        let model = AppModel()
        model.setStatusForTesting(held(false))
        XCTAssertNil(model.termsHold)
        model.setStatusForTesting(held(true))
        let hold = try XCTUnwrap(model.termsHold)
        let words = try words()
        XCTAssertEqual(hold.title, words.noticeTitle)
        XCTAssertEqual(hold.body, words.noticeBody)
        XCTAssertEqual(hold.review, words.review)
        XCTAssertNil(TermsHoldSurface.of(held: true, words: nil), "no words, no notice")
    }

    /// Like the gate hold's notice, it goes only when its cause does, and
    /// it is not the user's pause.
    func testNoticeHasNoCloseAction() throws {
        let hold = try XCTUnwrap(TermsHoldSurface.of(held: true, words: try words()))
        XCTAssertFalse(hold.closable)
        XCTAssertEqual(TermsHoldSurface.noticeID, "terms-hold")
        let model = AppModel()
        model.setStatusForTesting(held(true))
        XCTAssertFalse(model.status.paused)
    }

    func testReviewOpensSheet() {
        let model = AppModel()
        model.setStatusForTesting(held(true))
        XCTAssertFalse(model.termsSheetOpen)
        model.reviewNewTerms()
        XCTAssertTrue(model.termsSheetOpen)
    }

    func testMenuRowShownOnlyWhileHeldAndOpensHomeWithSheet() throws {
        OpenMonitor.reset()
        addTeardownBlock { @MainActor in OpenMonitor.reset() }
        var opened: [MonitorDestination?] = []
        OpenMonitor.handler = { destination, _ in opened.append(destination) }

        let model = AppModel()
        model.setStatusForTesting(held(false))
        XCTAssertNil(model.termsHold?.menuRow)
        model.setStatusForTesting(held(true))
        XCTAssertEqual(model.termsHold?.menuRow, try words().menuRow)

        model.reviewNewTermsFromMenu()
        XCTAssertTrue(model.termsSheetOpen)
        XCTAssertEqual(opened, [.home(.overview)])
    }

    func testAcceptClosesSheetAndNoticeGoesWithHold() async {
        let model = AppModel()
        model.setStatusForTesting(held(true))
        var sent: [AcceptedTerms] = []
        model.termsAcceptForTesting = { terms in
            sent.append(terms)
            return false
        }
        model.reviewNewTerms()
        let done = await model.acceptNewTerms(Self.accepted)
        XCTAssertTrue(done)
        XCTAssertFalse(model.termsSheetOpen)
        XCTAssertEqual(sent, [Self.accepted])
        // The daemon lifted the hold; its next status takes the notice and
        // the menu row away.
        model.setStatusForTesting(held(false))
        XCTAssertNil(model.termsHold)
    }

    func testStaleAcceptKeepsSheetAndReloads() async {
        let model = AppModel()
        model.setStatusForTesting(held(true))
        var fetches = 0
        let document = TermsDocument(
            version: "2026-12-02", effective: "2026-12-02", inEffect: true, title: "T",
            blocks: [.paragraph("p")])
        let fresh = FetchedTerms(document: document, contentSHA256: String(repeating: "d", count: 64))
        model.termsHoldLoader = TermsLoader(fetch: {
            fetches += 1
            return .success(fresh)
        })
        model.termsAcceptForTesting = { _ in
            throw DaemonClient.Failure(code: "bad_params", message: "terms-version-stale")
        }
        model.reviewNewTerms()
        let done = await model.acceptNewTerms(Self.accepted)
        XCTAssertFalse(done)
        XCTAssertTrue(model.termsSheetOpen, "the sheet stays up")
        XCTAssertEqual(fetches, 1, "the terms are fetched again")
        XCTAssertEqual(model.termsHoldLoader.state, .loaded(fresh))
        XCTAssertNotNil(model.termsHold, "the hold stays")
    }

    /// Any other refusal keeps the sheet and fetches nothing.
    func testRefusedAcceptKeepsSheet() async {
        let model = AppModel()
        model.setStatusForTesting(held(true))
        var fetches = 0
        model.termsHoldLoader = TermsLoader(fetch: {
            fetches += 1
            return .failure(TCTermsRefusal(label: "terms-unavailable"))
        })
        model.termsAcceptForTesting = { _ in
            throw DaemonClient.Failure(code: "unavailable", message: "terms-state-write-failed")
        }
        model.reviewNewTerms()
        let done = await model.acceptNewTerms(Self.accepted)
        XCTAssertFalse(done)
        XCTAssertTrue(model.termsSheetOpen)
        XCTAssertEqual(fetches, 0)
    }
}
