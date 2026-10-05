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
        XCTAssertEqual(failure, .grantRefused(label: "connect"))
        XCTAssertFalse(FirstRunPlan.calls(for: state, at: .start).contains { if case .grantAutomatic = $0 { true } else { false } })

        // An unreadable answer is not ready either.
        let (unread, unreadFailure) = SharingDisclosureFlow.resolve(onUses(.automatic), request: nil)
        XCTAssertEqual(unread.sharing, .askMe)
        XCTAssertNotNil(unreadFailure)
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
}
