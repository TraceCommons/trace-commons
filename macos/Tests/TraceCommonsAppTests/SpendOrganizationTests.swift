import XCTest
import os

import TCBridge
@testable import TCShellCore
@testable import TraceCommonsApp

/// The spending-organization picker and the Private AI prompt, through the
/// real `AppModel` and `DaemonClient` over a scripted daemon.
@MainActor
final class SpendOrganizationTests: XCTestCase {
    /// Answers the two spending-organization methods and refuses the rest.
    private final class Daemon: DaemonCalling {
        struct Call: Equatable, Sendable {
            let method: String
            let params: String
        }

        private struct State: Sendable {
            var recorded: [Call] = []
            var listing: String
            var selectReply: String
        }
        private let state: OSAllocatedUnfairLock<State>

        var listing: String {
            get { state.withLock { $0.listing } }
            set { state.withLock { $0.listing = newValue } }
        }
        var selectReply: String {
            get { state.withLock { $0.selectReply } }
            set { state.withLock { $0.selectReply = newValue } }
        }

        init(listing: String, selectReply: String = #"{"id":1,"result":{"switched":true,"revoke_pending":false}}"#) {
            state = OSAllocatedUnfairLock(initialState: State(listing: listing, selectReply: selectReply))
        }

        var calls: [Call] {
            state.withLock { $0.recorded }
        }

        func call(_ method: String, params paramsJSON: String) -> String {
            let (listing, selectReply) = state.withLock {
                $0.recorded.append(Call(method: method, params: paramsJSON))
                return ($0.listing, $0.selectReply)
            }
            switch method {
            case "near_ai_spend_organizations": return listing
            case "near_ai_spend_organization_select": return selectReply
            default: return #"{"id":1,"error":{"code":"unavailable","message":"synthetic_refusal"}}"#
            }
        }

        func searchOriginal(entryID: String, needle: String) -> Int? { nil }
        func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
    }

    private static func listing(_ organizations: [(String, Bool)], explicit: Bool = false) -> String {
        let body = SpendOrganizationSurfaceFixture.payload(organizations: organizations, explicit: explicit)
        return #"{"id":1,"result":"# + body + "}"
    }

    private func model(_ daemon: Daemon) -> AppModel {
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        return model
    }

    private func settle(_ model: AppModel, until condition: @MainActor () -> Bool) async {
        for _ in 0..<200 where !condition() {
            try? await Task.sleep(for: .milliseconds(10))
        }
    }

    func testASingleOrganizationDrawsNothingNew() async {
        let daemon = Daemon(listing: Self.listing([("org-a", true)]))
        let model = model(daemon)
        model.refreshSpendOrganizations()
        await settle(model) { model.spendOrganizations.state == "listed" }
        XCTAssertEqual(model.spendOrganizations.organizations.count, 1)
        XCTAssertFalse(model.spendOrganizations.showsPicker)
        XCTAssertFalse(model.spendOrganizations.showsPrompt)
        XCTAssertNil(SpendOrganizationCard.stateLine(model.spendOrganizations, switching: false, failure: nil))
    }

    func testSeveralOrganizationsWithNoChoicePromptAndOfferThePicker() async {
        let daemon = Daemon(listing: Self.listing([("org-a", true), ("org-b", false)]))
        let model = model(daemon)
        model.refreshSpendOrganizations()
        await settle(model) { model.spendOrganizations.state == "listed" }
        XCTAssertTrue(model.spendOrganizations.showsPicker)
        XCTAssertTrue(model.spendOrganizations.showsPrompt)
        XCTAssertEqual(
            SpendOrganizationCard.options(model.spendOrganizations).map(\.value), ["org-a", "org-b"])
    }

    /// A daemon too old to know the method draws nothing and raises nothing.
    func testAnOlderDaemonDrawsNothingAndRaisesNoError() async {
        let daemon = Daemon(listing: #"{"id":1,"error":{"code":"unknown_method","message":"unknown_method"}}"#)
        let model = model(daemon)
        model.refreshSpendOrganizations()
        await settle(model) { !daemon.calls.isEmpty }
        try? await Task.sleep(for: .milliseconds(50))
        XCTAssertEqual(model.spendOrganizations, .unreported)
        XCTAssertNil(model.lastActionError)
    }

    func testPickingSendsTheOrganizationAndRereadsTheListing() async {
        let daemon = Daemon(listing: Self.listing([("org-a", true), ("org-b", false)]))
        let model = model(daemon)
        model.refreshSpendOrganizations()
        await settle(model) { model.spendOrganizations.state == "listed" }
        daemon.listing = Self.listing([("org-a", false), ("org-b", true)], explicit: true)
        await model.selectSpendOrganization("org-b")
        XCTAssertFalse(model.spendOrganizationSwitching)
        XCTAssertNil(model.spendOrganizationFailure)
        let select = daemon.calls.first { $0.method == "near_ai_spend_organization_select" }
        XCTAssertEqual(select?.params, #"{"organization_id":"org-b"}"#)
        await settle(model) { model.spendOrganizations.currentOrganizationID == "org-b" }
        XCTAssertFalse(model.spendOrganizations.showsPrompt)
        XCTAssertTrue(
            daemon.calls.contains { $0.method == "near_ai_spend_organizations" && $0.params.contains("fresh") })
    }

    /// Choosing the organization already in use ends the prompt at once,
    /// before the re-read lands, even if that re-read still says otherwise.
    func testChoosingTheCurrentOrganizationEndsThePromptImmediately() async {
        let daemon = Daemon(
            listing: Self.listing([("org-a", true), ("org-b", false)]),
            selectReply: #"{"id":1,"result":{"switched":false,"revoke_pending":false}}"#)
        let model = model(daemon)
        model.refreshSpendOrganizations()
        await settle(model) { model.spendOrganizations.state == "listed" }
        XCTAssertTrue(model.spendOrganizations.showsPrompt)
        await model.selectSpendOrganization("org-a")
        XCTAssertFalse(model.spendOrganizations.showsPrompt)
        XCTAssertTrue(model.spendOrganizations.showsPicker)
        XCTAssertNil(model.spendOrganizationFailure)
    }

    /// "Not now" records the organization already in use, so nothing is
    /// minted or switched, and ends the prompt at once even if the re-read
    /// still says otherwise.
    func testNotNowKeepsTheCurrentOrganizationAndEndsThePromptImmediately() async {
        let daemon = Daemon(
            listing: Self.listing([("org-a", true), ("org-b", false)]),
            selectReply: #"{"id":1,"result":{"switched":false,"revoke_pending":false}}"#)
        let model = model(daemon)
        model.refreshSpendOrganizations()
        await settle(model) { model.spendOrganizations.state == "listed" }
        XCTAssertTrue(model.spendOrganizations.showsPrompt)
        XCTAssertEqual(model.spendOrganizations.words?.promptDismiss, "PROMPT-DISMISS")
        await model.dismissSpendOrganizationPrompt()
        let selects = daemon.calls.filter { $0.method == "near_ai_spend_organization_select" }
        XCTAssertEqual(selects.map(\.params), [#"{"organization_id":"org-a"}"#])
        XCTAssertFalse(model.spendOrganizations.showsPrompt)
        XCTAssertTrue(model.spendOrganizations.showsPicker)
        XCTAssertEqual(model.spendOrganizations.currentOrganizationID, "org-a")
        XCTAssertEqual(model.spendOrganizations.organizations.map(\.current), [true, false])
        XCTAssertNil(model.spendOrganizationFailure)
        XCTAssertFalse(model.spendOrganizationSwitching)
    }

    /// A failed record keeps the prompt and says nothing: the next re-read
    /// shows the truth, and the Settings card's failure line is for switches.
    func testNotNowFailingKeepsThePromptAndSaysNothing() async {
        let daemon = Daemon(
            listing: Self.listing([("org-a", true), ("org-b", false)]),
            selectReply: #"{"id":1,"error":{"code":"unavailable","message":"near_ai_spend_org_refused"}}"#)
        let model = model(daemon)
        model.refreshSpendOrganizations()
        await settle(model) { model.spendOrganizations.state == "listed" }
        await model.dismissSpendOrganizationPrompt()
        XCTAssertTrue(model.spendOrganizations.showsPrompt)
        XCTAssertNil(model.spendOrganizationFailure)
        XCTAssertNil(model.lastActionError)
        XCTAssertEqual(model.spendOrganizations.currentOrganizationID, "org-a")
    }

    /// With no current organization there is nothing to keep: no call.
    func testNotNowWithoutACurrentOrganizationCallsNothing() async {
        let daemon = Daemon(listing: Self.listing([("org-a", false), ("org-b", false)]))
        let model = model(daemon)
        await model.dismissSpendOrganizationPrompt()
        XCTAssertTrue(daemon.calls.filter { $0.method == "near_ai_spend_organization_select" }.isEmpty)
    }

    func testARefusedSwitchSaysSoInTheDaemonsWords() async {
        let daemon = Daemon(
            listing: Self.listing([("org-a", true), ("org-b", false)]),
            selectReply: #"{"id":1,"error":{"code":"unavailable","message":"near_ai_spend_org_refused"}}"#)
        let model = model(daemon)
        model.refreshSpendOrganizations()
        await settle(model) { model.spendOrganizations.state == "listed" }
        await model.selectSpendOrganization("org-b")
        XCTAssertFalse(model.spendOrganizationSwitching)
        XCTAssertEqual(model.spendOrganizationFailure, "REFUSED")
        XCTAssertEqual(
            SpendOrganizationCard.stateLine(
                model.spendOrganizations, switching: false, failure: model.spendOrganizationFailure),
            .init(text: "REFUSED", tone: .outside))
    }

    func testTheLineUnderThePickerPrefersSwitchingThenFailureThenPendingRevoke() {
        let status = SpendOrganizations.parse(fromJSON: SpendOrganizationSurfaceFixture.payload(
            organizations: [("org-a", true), ("org-b", false)], revokePending: true))
        XCTAssertEqual(
            SpendOrganizationCard.stateLine(status, switching: true, failure: "FAILED")?.text, "SWITCHING")
        XCTAssertEqual(SpendOrganizationCard.stateLine(status, switching: false, failure: "FAILED")?.text, "FAILED")
        XCTAssertEqual(SpendOrganizationCard.stateLine(status, switching: false, failure: nil)?.text, "PENDING")
    }
}

/// The daemon's answer, shaped as `near_ai_spend_organizations` sends it.
enum SpendOrganizationSurfaceFixture {
    static var view: [String: Any] { [
        "title": "TITLE", "what": "WHAT", "picker": "PICKER", "switching": "SWITCHING", "failed": "FAILED",
        "session_expired": "EXPIRED", "refused": "REFUSED", "revoke_pending": "PENDING",
        "prompt_title": "PROMPT-TITLE", "prompt_line": "PROMPT-LINE",
        "prompt_action": "PROMPT-ACTION", "prompt_action_accessibility": "PROMPT-ACTION-AX",
        "prompt_dismiss": "PROMPT-DISMISS", "prompt_dismiss_accessibility": "PROMPT-DISMISS-AX",
    ] }

    static func payload(
        organizations: [(String, Bool)], explicit: Bool = false, revokePending: Bool = false
    ) -> String {
        let object: [String: Any] = [
            "state": "listed",
            "organizations": organizations.map { ["id": $0.0, "name": "Name \($0.0)", "role": "owner", "current": $0.1] },
            "current_organization_id": organizations.first { $0.1 }?.0 ?? "org-a",
            "explicit": explicit,
            "multiple": organizations.count >= 2,
            "prompt": organizations.count >= 2 && !explicit,
            "revoke_pending": revokePending,
            "view": view,
        ]
        let data = try! JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        return String(decoding: data, as: UTF8.self)
    }
}
