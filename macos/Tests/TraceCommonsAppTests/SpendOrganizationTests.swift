import XCTest

import TCBridge
@testable import TCShellCore
@testable import TraceCommonsApp

/// The spending-organization picker and the Private AI prompt, through the
/// real `AppModel` and `DaemonClient` over a scripted daemon.
@MainActor
final class SpendOrganizationTests: XCTestCase {
    /// Answers the two spending-organization methods and refuses the rest.
    private final class Daemon: DaemonCalling {
        struct Call: Equatable {
            let method: String
            let params: String
        }

        private let lock = NSLock()
        private var recorded: [Call] = []
        var listing: String
        var selectReply: String

        init(listing: String, selectReply: String = #"{"id":1,"result":{"switched":true,"revoke_pending":false}}"#) {
            self.listing = listing
            self.selectReply = selectReply
        }

        var calls: [Call] {
            lock.lock()
            defer { lock.unlock() }
            return recorded
        }

        func call(_ method: String, params paramsJSON: String) -> String {
            lock.lock()
            recorded.append(Call(method: method, params: paramsJSON))
            let listing = self.listing
            let selectReply = self.selectReply
            lock.unlock()
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
    nonisolated(unsafe) static let view: [String: Any] = [
        "title": "TITLE", "what": "WHAT", "picker": "PICKER", "switching": "SWITCHING", "failed": "FAILED",
        "session_expired": "EXPIRED", "refused": "REFUSED", "revoke_pending": "PENDING",
        "prompt_title": "PROMPT-TITLE", "prompt_line": "PROMPT-LINE",
        "prompt_action": "PROMPT-ACTION", "prompt_action_accessibility": "PROMPT-ACTION-AX",
    ]

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
