import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The Sharing moment's Automatic path (owner, 2026-09-28 points 2 and 4):
/// the scrub disclosure, then the witness disclosure, then the core decides
/// whether the grant may be asked for (`TCFlow1.grantRequestJSON`) and with
/// which witness. Ask me shows neither and grants nothing.
final class SharingDisclosuresTests: XCTestCase {
    private let scopes: Set<String> = ["debugging_evaluation"]

    private func onUses(_ sharing: SharingPath) -> FirstRunState {
        FirstRunState(
            tier: .quick, step: .uses, account: .nearAI, scopes: scopes, sharing: sharing,
            daemonStarted: true, enrolledInvite: "invite")
    }

    private static func source() throws -> String {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/FirstRun/SharingDisclosures.swift")
        return try String(contentsOf: url, encoding: .utf8)
    }

    func test_automaticShowsBothDisclosuresBeforeTheGrant() throws {
        XCTAssertTrue(SharingDisclosureFlow.isNeeded(for: onUses(.automatic)))

        var flow = SharingDisclosureFlow()
        XCTAssertEqual(flow.step, .scrub)

        // Before either sheet, the core refuses, naming both.
        let none = try XCTUnwrap(SharingDisclosureFlow.grantRequest(flow.progress(connected: true, scopes: scopes)))
        XCTAssertFalse(none.ready)
        XCTAssertEqual(none.blockers, ["scrub_disclosure", "witness_disclosure"])

        // The witness cannot be acknowledged first.
        flow.acknowledgeWitness(shown: "0xwitness")
        XCTAssertEqual(flow.step, .scrub)

        flow.acknowledgeScrub()
        XCTAssertEqual(flow.step, .witness)
        let one = try XCTUnwrap(SharingDisclosureFlow.grantRequest(flow.progress(connected: true, scopes: scopes)))
        XCTAssertEqual(one.blockers, ["witness_disclosure"])

        flow.acknowledgeWitness(shown: "0xwitness")
        XCTAssertEqual(flow.step, .done)
        let both = try XCTUnwrap(SharingDisclosureFlow.grantRequest(flow.progress(connected: true, scopes: scopes)))
        XCTAssertTrue(both.ready)
        XCTAssertEqual(both.witnessSigningAddress, "0xwitness")

        // Ready: Automatic stands and the grant carries the witness shown,
        // after the scopes are saved.
        let (state, failure) = SharingDisclosureFlow.resolve(onUses(.automatic), request: both)
        XCTAssertNil(failure)
        XCTAssertEqual(state.sharing, .automatic)
        XCTAssertTrue(state.grantReady, "only the core's ready answer marks the grant ready")
        let calls = FirstRunPlan.calls(for: state, at: .start)
        let scopesAt = try XCTUnwrap(calls.firstIndex(of: .setConsentScopes(["debugging_evaluation"])))
        let grantAt = try XCTUnwrap(calls.firstIndex(of: .grantAutomatic(witness: "0xwitness")))
        XCTAssertLessThan(scopesAt, grantAt)

        // The sheets are drawn from the core's words: the scrub lines, then
        // the route disclosure and the witness's state line.
        let source = try Self.source()
        XCTAssertTrue(source.contains("grant.lines"))
        XCTAssertTrue(source.contains("RouteDisclosureGlassBody(disclosure:"))
        XCTAssertTrue(source.contains("WitnessSurface.stateLine("))
        XCTAssertTrue(source.contains("TCFlow1.grantRequestJSON("))
    }

    /// Not ready (for example, the account is not connected): setup finishes
    /// on Ask me and says why; it never claims Automatic.
    func test_aGrantTheCoreRefusesFinishesOnAskMe() throws {
        var flow = SharingDisclosureFlow()
        flow.acknowledgeScrub()
        flow.acknowledgeWitness(shown: nil)
        let request = try XCTUnwrap(SharingDisclosureFlow.grantRequest(flow.progress(connected: false, scopes: scopes)))
        XCTAssertFalse(request.ready)

        let (state, failure) = SharingDisclosureFlow.resolve(onUses(.automatic), request: request)
        XCTAssertEqual(state.sharing, .askMe)
        XCTAssertNil(state.witnessSigningAddress)
        XCTAssertFalse(state.grantReady)
        XCTAssertEqual(failure, .grantRefused(label: "connect"))
        XCTAssertFalse(FirstRunPlan.calls(for: state, at: .start).contains { if case .grantAutomatic = $0 { true } else { false } })

        // An unreadable answer is not ready either.
        let (unread, unreadFailure) = SharingDisclosureFlow.resolve(onUses(.automatic), request: nil)
        XCTAssertEqual(unread.sharing, .askMe)
        XCTAssertFalse(unread.grantReady)
        XCTAssertNotNil(unreadFailure)

        // A marker left from an earlier ready answer does not survive a
        // refusal, even on a state that still says Automatic.
        var stale = onUses(.automatic)
        stale.grantReady = true
        let (refusedAgain, _) = SharingDisclosureFlow.resolve(stale, request: request)
        XCTAssertFalse(refusedAgain.grantReady)
    }

    func test_askMeGrantsNothing() {
        let state = onUses(.askMe)
        XCTAssertFalse(SharingDisclosureFlow.isNeeded(for: state))
        let calls = FirstRunPlan.calls(for: state, at: .start)
        XCTAssertFalse(calls.contains { if case .grantAutomatic = $0 { true } else { false } })
        XCTAssertEqual(calls.last, .markComplete)

        // Watching only never reaches the disclosures, whatever the state says.
        var watching = onUses(.automatic)
        watching.account = .watchOnly
        XCTAssertFalse(SharingDisclosureFlow.isNeeded(for: watching))
    }

    /// Spec rule 10 and Ron's review of #1235, item 1: turning on Private AI
    /// is a consent event that leads to the witness disclosure. On the
    /// Automatic path it is the second of the two sheets; on Ask me, or
    /// watching only, it is shown directly, and Start then grants nothing.
    func test_privateAIOnWithoutAutomaticShowsTheWitnessDisclosure() throws {
        var askMe = onUses(.askMe)
        askMe.tier = .custom
        askMe.privateAI = true
        XCTAssertEqual(UsesScreenLayout.startRoute(askMe), .discloseWitness)
        var watching = askMe
        watching.account = .watchOnly
        XCTAssertEqual(UsesScreenLayout.startRoute(watching), .discloseWitness)

        // Automatic already shows the witness sheet, after the scrub sheet.
        var automatic = onUses(.automatic)
        automatic.tier = .custom
        automatic.privateAI = true
        XCTAssertEqual(UsesScreenLayout.startRoute(automatic), .disclose)

        // Private AI off, or not offered (Quick), changes nothing.
        askMe.privateAI = false
        XCTAssertEqual(UsesScreenLayout.startRoute(askMe), .commit)
        var quick = onUses(.askMe)
        quick.privateAI = true
        XCTAssertEqual(UsesScreenLayout.startRoute(quick), .commit)

        // The witness-only flow opens on the witness sheet and needs only it.
        var flow = SharingDisclosureFlow(witnessOnly: true)
        XCTAssertEqual(flow.step, .witness)
        XCTAssertTrue(flow.witnessOnly)
        flow.acknowledgeWitness(shown: "0xwitness")
        XCTAssertEqual(flow.step, .done)

        // The screen opens it, and its end starts without asking for a grant.
        let uses = try String(
            contentsOf: URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
                .deletingLastPathComponent().appendingPathComponent(
                    "Sources/TraceCommonsApp/Views/FirstRun/UsesScreen.swift"), encoding: .utf8)
        XCTAssertTrue(uses.contains("case .discloseWitness:"))
        XCTAssertTrue(uses.contains("SharingDisclosureFlow(witnessOnly: true)"))
        XCTAssertTrue(uses.contains("if flow.witnessOnly"))
    }
}
