import XCTest

@testable import TCShellCore

/// The payout section's pure logic, without the dylib. The words are
/// placeholders here ("T:" prefixed) so a test passes only by reading the
/// payload; `TCBridgeTests` checks the live export's keys.
final class PayoutLinkSurfaceTests: XCTestCase {
    private static let fields = PayoutLinkCopy.consumedFields

    private static func copyJSON(_ override: [String: String] = [:]) -> String {
        var object = [String: String]()
        for field in fields { object[field] = "T:\(field) {org_name}" }
        for (key, value) in override { object[key] = value }
        let data = try! JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        return String(data: data, encoding: .utf8)!
    }

    private let copy = PayoutLinkCopy.decode(fromJSON: copyJSON())!
    private var surface: PayoutLinkSurface { PayoutLinkSurface(copy: copy) }

    private static let a = PayoutOrganization(id: "org-a", name: "Alpha")
    private static let b = PayoutOrganization(id: "org-b", name: "Beta")

    private func listing(_ orgs: [PayoutOrganization]) -> PayoutOrganizationListing {
        PayoutOrganizationListing(organizations: orgs, orgsRevision: String(repeating: "a", count: 64))
    }

    func testNothingIsPreselectedWithTwoOrgs() {
        var picker = PayoutPicker()
        picker.beginLoading()
        picker.listed(listing([Self.a, Self.b]))
        XCTAssertEqual(picker.phase, .choosing)
        XCTAssertEqual(picker.rows.count, 2)
        XCTAssertNil(picker.selected)
        XCTAssertNil(picker.consent(copy: copy))
        picker.beginWorking()
        XCTAssertEqual(picker.phase, .choosing, "nothing is sent with no choice")
    }

    func testOneOrgStillNeedsConfirm() {
        var picker = PayoutPicker()
        picker.listed(listing([Self.a]))
        XCTAssertNil(picker.selected, "a single org is not chosen for the person")
        XCTAssertNil(picker.consent(copy: copy))
        picker.select("org-a")
        let consent = picker.consent(copy: copy)
        XCTAssertEqual(consent?.orgName, "Alpha")
        XCTAssertEqual(consent?.body, "T:body Alpha")
        XCTAssertEqual(consent?.accessibilityName, "T:accessibility_name Alpha")
        picker.beginWorking()
        XCTAssertEqual(picker.phase, .working)
    }

    func testSelectingAnUnlistedOrgDoesNothingAndRelistingClearsTheChoice() {
        var picker = PayoutPicker()
        picker.listed(listing([Self.a, Self.b]))
        picker.select("org-z")
        XCTAssertNil(picker.selected)
        picker.select("org-b")
        XCTAssertEqual(picker.selected, Self.b)
        picker.listed(listing([Self.a, Self.b]))
        XCTAssertNil(picker.selected, "a choice is not carried onto a new list")
        picker.select("org-a")
        picker.dismissSelection()
        XCTAssertNil(picker.selected)
        XCTAssertEqual(picker.phase, .choosing)
    }

    func testNoOwnedOrgAndRefusals() {
        var picker = PayoutPicker()
        picker.listed(listing([]))
        XCTAssertEqual(picker.phase, .noOrganization)
        picker.listed(listing([Self.a]))
        picker.select("org-a")
        picker.refused(.changed)
        XCTAssertEqual(picker.phase, .refused(.changed))
        XCTAssertTrue(picker.rows.isEmpty, "a changed list is dropped")
        XCTAssertNil(picker.selected)
        picker.listed(listing([Self.a]))
        picker.select("org-a")
        picker.refused(.failed)
        XCTAssertEqual(picker.rows.count, 1, "a plain failure keeps the list shown")
        XCTAssertNil(picker.selected)
    }

    func testStatusLinesMapEachState() {
        func line(_ rail: PayoutRailStatus) -> String? { surface.statusLine(for: rail) }
        XCTAssertEqual(line(PayoutRailStatus()), "T:status_none {org_name}")
        XCTAssertEqual(
            line(PayoutRailStatus(linked: true, state: "active", organizationName: "Alpha")),
            "T:status_linked Alpha")
        XCTAssertEqual(
            line(PayoutRailStatus(linked: true, state: "waiting", organizationName: "Alpha")),
            "T:status_waiting Alpha")
        XCTAssertEqual(
            line(PayoutRailStatus(recipientUnavailable: true)), "T:status_unavailable {org_name}")
        XCTAssertEqual(
            line(
                PayoutRailStatus(
                    linked: true, state: "active", recipientRefCheck: "mismatch",
                    organizationName: "Alpha")),
            "T:status_mismatch {org_name}", "a mismatch outranks the friendly state")
        XCTAssertEqual(
            line(PayoutRailStatus(linked: true, state: "active", unlinkRequested: true)),
            "T:status_none {org_name}")
        XCTAssertEqual(
            line(PayoutRailStatus(linked: true, state: "closing", organizationName: "Alpha")),
            "T:status_none {org_name}", "credit is not promised to a closing link")
    }

    func testALinkWhoseNameIsNotInMemoryHasNoLineYet() {
        let rail = PayoutRailStatus(linked: true, state: "active")
        XCTAssertNil(surface.statusLine(for: rail))
        XCTAssertTrue(surface.needsOrgName(for: rail))
        XCTAssertFalse(surface.canUnlink(for: rail))
        XCTAssertFalse(surface.needsOrgName(for: PayoutRailStatus()))
    }

    func testChoosingAndUnlinkingAreOfferedOnlyWhenTheyApply() {
        XCTAssertTrue(surface.canChoose(for: PayoutRailStatus()))
        let active = PayoutRailStatus(linked: true, state: "active", organizationName: "Alpha")
        XCTAssertFalse(surface.canChoose(for: active))
        XCTAssertTrue(surface.canUnlink(for: active))
        XCTAssertEqual(surface.unlinkAccessibilityName(for: active), "T:unlink_accessibility_name Alpha")
        let waiting = PayoutRailStatus(linked: true, state: "waiting", organizationName: "Alpha")
        XCTAssertFalse(surface.canUnlink(for: waiting), "the commons refuses an unlink of a waiting link")
        let mismatch = PayoutRailStatus(
            linked: true, state: "active", recipientRefCheck: "mismatch", organizationName: "Alpha")
        XCTAssertTrue(surface.canChoose(for: mismatch))
        XCTAssertTrue(surface.canChoose(for: PayoutRailStatus(recipientUnavailable: true)))
    }

    /// A direct change: offered on an active link nothing is closing, as
    /// its own button, without stopping the link first.
    func testChangeIsOfferedOnAnActiveLink() {
        let active = PayoutRailStatus(linked: true, state: "active", organizationName: "Alpha")
        XCTAssertTrue(surface.canChange(for: active))
        XCTAssertTrue(
            surface.canChange(for: PayoutRailStatus(linked: true, state: "active")),
            "the change's words name no org, so it does not wait on the name")
        XCTAssertFalse(surface.canChange(for: PayoutRailStatus()), "nothing linked: Choose instead")
        XCTAssertFalse(
            surface.canChange(
                for: PayoutRailStatus(linked: true, state: "active", unlinkRequested: true)))
        XCTAssertFalse(
            surface.canChange(
                for: PayoutRailStatus(linked: true, state: "waiting", organizationName: "Alpha")),
            "a waiting link already has a change in flight")
        XCTAssertFalse(
            surface.canChange(
                for: PayoutRailStatus(
                    linked: true, state: "active", recipientRefCheck: "mismatch",
                    organizationName: "Alpha")),
            "a mismatch is chosen over directly")
        XCTAssertEqual(surface.copy.changeButton, "T:change_button {org_name}")
        XCTAssertEqual(
            surface.copy.changeAccessibilityName, "T:change_accessibility_name {org_name}")
    }

    /// Change shows the org list, then the same consent and Choose as a
    /// first link; Not now, a finish or a lost list ends it.
    func testChangeShowsTheListThenTheSameConsent() {
        let active = PayoutRailStatus(linked: true, state: "active", organizationName: "Alpha")
        var picker = PayoutPicker()
        XCTAssertFalse(surface.showsPicker(for: active, picker))
        picker.beginChange()
        XCTAssertTrue(picker.changing)
        XCTAssertEqual(picker.phase, .loading)
        XCTAssertTrue(surface.showsPicker(for: active, picker))
        picker.beginLoading()
        picker.listed(listing([Self.a, Self.b]))
        XCTAssertTrue(picker.changing, "listing keeps the change open")
        XCTAssertNil(picker.selected, "nothing is preselected on a change either")
        picker.select("org-b")
        XCTAssertEqual(picker.consent(copy: copy)?.body, "T:body Beta")
        picker.beginWorking()
        XCTAssertEqual(picker.phase, .working)
        picker.finished()
        XCTAssertFalse(picker.changing)
        XCTAssertFalse(surface.showsPicker(for: active, picker))

        picker.beginChange()
        picker.listed(listing([Self.a, Self.b]))
        picker.select("org-a")
        picker.dismissSelection()
        XCTAssertFalse(picker.changing, "Not now leaves the change")
        XCTAssertEqual(picker.phase, .idle)
        XCTAssertTrue(picker.rows.isEmpty)

        picker.beginChange()
        picker.listed(listing([Self.a]))
        picker.select("org-a")
        picker.refused(.failed)
        XCTAssertTrue(picker.changing, "a plain failure keeps the list to retry")
        picker.refused(.changed)
        XCTAssertFalse(picker.changing, "a changed list ends it; Change lists again")
        XCTAssertFalse(surface.showsPicker(for: active, picker))
        // Change is hidden only while its list or consent is up; after a
        // refusal or an empty list it is back to list again.
        var shown = PayoutPicker()
        XCTAssertTrue(surface.showsChangeButton(for: active, shown))
        shown.beginChange()
        XCTAssertFalse(surface.showsChangeButton(for: active, shown))
        shown.listed(listing([Self.a]))
        XCTAssertFalse(surface.showsChangeButton(for: active, shown))
        shown.refused(.failed)
        XCTAssertTrue(
            surface.showsChangeButton(for: active, shown), "a failed choose can be retried")
        shown.beginChange()
        shown.listed(listing([]))
        XCTAssertTrue(surface.showsChangeButton(for: active, shown))
        XCTAssertFalse(surface.showsChangeButton(for: PayoutRailStatus(), PayoutPicker()))
        // A first link shows the picker without a change.
        XCTAssertTrue(surface.showsPicker(for: PayoutRailStatus(), PayoutPicker()))
    }

    func testTheSectionIsHiddenUnlessNearAiIsEnabled() {
        XCTAssertNil(PayoutLinkSurface.nearAiRail(in: []))
        XCTAssertNil(PayoutLinkSurface.nearAiRail(in: [PayoutRailStatus(railId: "other")]))
        XCTAssertNil(PayoutLinkSurface.nearAiRail(in: [PayoutRailStatus(mode: "disabled")]))
        XCTAssertNotNil(PayoutLinkSurface.nearAiRail(in: [PayoutRailStatus(mode: "dry_run")]))
        XCTAssertNotNil(PayoutLinkSurface.nearAiRail(in: [PayoutRailStatus(mode: "live")]))
    }

    /// D5 was superseded: no step, state, button or sentence mentions a
    /// passkey, and the refusal routing has no passkey case.
    func testNoPasskeyStepOrWordingExists() {
        for sentence in copy.sentences {
            XCTAssertFalse(sentence.lowercased().contains("passkey"), sentence)
        }
        let phases: [PayoutPicker.Phase] = [
            .idle, .loading, .choosing, .noOrganization, .working, .refused(.signIn),
        ]
        for phase in phases {
            XCTAssertFalse("\(phase)".lowercased().contains("passkey"))
        }
        for label in ["passkey-required", "add-passkey", "payout-link-passkey"] {
            XCTAssertEqual(PayoutLinkRefusal.from(code: label), .failed)
        }
    }

    func testCopyComesFromCoreKeys() {
        let json = Self.copyJSON(["button": "From the core"])
        let decoded = PayoutLinkCopy.decode(fromJSON: json)
        XCTAssertEqual(decoded?.button, "From the core")
        for field in Self.fields {
            var object = try! JSONSerialization.jsonObject(with: Data(json.utf8)) as! [String: String]
            object.removeValue(forKey: field)
            let missing = String(
                data: try! JSONSerialization.data(withJSONObject: object), encoding: .utf8)!
            XCTAssertNil(PayoutLinkCopy.decode(fromJSON: missing), "missing \(field)")
            object[field] = "  "
            let blank = String(
                data: try! JSONSerialization.data(withJSONObject: object), encoding: .utf8)!
            XCTAssertNil(PayoutLinkCopy.decode(fromJSON: blank), "blank \(field)")
        }
        XCTAssertNil(PayoutLinkCopy.decode(fromJSON: "not json"))
    }

    func testRefusalLabelsRoute() {
        for label in [
            "payout-link-near-ai-sign-in-required", "payout-link-near-ai-session-expired",
        ] {
            XCTAssertEqual(PayoutLinkRefusal.from(code: label), .signIn)
        }
        for label in ["payout-link-orgs-changed", "payout-link-changed", "version_conflict"] {
            XCTAssertEqual(PayoutLinkRefusal.from(code: label), .changed)
        }
        XCTAssertEqual(PayoutLinkRefusal.from(code: "subject_mismatch"), .failed)
        XCTAssertEqual(PayoutLinkRefusal.from(code: "anything-new"), .failed)
        // No Trace Commons account session (the daemon could not renew
        // one): its own line, not the NEAR AI sign-in line.
        XCTAssertEqual(PayoutLinkRefusal.from(code: "account-session-required"), .sessionEnded)
        XCTAssertEqual(surface.refusalLine(.sessionEnded), "T:session_ended {org_name}")
        XCTAssertEqual(surface.refusalLine(.signIn), "T:sign_in {org_name}")
        XCTAssertEqual(surface.refusalLine(.changed), "T:changed {org_name}")
        XCTAssertEqual(surface.refusalLine(.failed), "T:failed {org_name}")
    }
}

/// What the section draws when the status read itself is refused.
final class PayoutLinkStatusRefusalTests: XCTestCase {
    /// A missing account session keeps the section on screen with its line,
    /// rather than hiding it; every other refusal still hides it, so no
    /// stale link state is shown.
    func testOnlyAMissingAccountSessionKeepsTheSectionShown() {
        XCTAssertEqual(
            PayoutLinkSurface.shownStatusRefusal(
                PayoutLinkClient.Failure(label: "account-session-required")),
            .sessionEnded)
        for label in [
            "commons_credential_storage_unavailable", "not-logged-in", "payout_unavailable",
            "unparseable-response", "payout-link-near-ai-sign-in-required", "anything-new",
        ] {
            XCTAssertNil(
                PayoutLinkSurface.shownStatusRefusal(PayoutLinkClient.Failure(label: label)), label)
        }
        struct Other: Error {}
        XCTAssertNil(PayoutLinkSurface.shownStatusRefusal(Other()))
    }
}

/// The five IPC calls: the method names and parameters the daemon reads.
final class PayoutLinkClientTests: XCTestCase {
    private final class Fake: DaemonTransport {
        var calls: [(String, [String: Any])] = []
        var replies: [String]
        init(_ replies: [String]) { self.replies = replies }
        func call(_ method: String, params paramsJSON: String) -> String {
            let params = try! JSONSerialization.jsonObject(with: Data(paramsJSON.utf8)) as! [String: Any]
            calls.append((method, params))
            return replies.isEmpty ? #"{"id":1,"result":{}}"# : replies.removeFirst()
        }
    }

    func testStatusDecodesAndToleratesUnknownFields() throws {
        let fake = Fake([
            #"{"id":1,"result":{"rails":[{"rail_id":"near_ai","mode":"live","linked":true,"state":"active","version":3,"request_state":null,"unlink_requested":false,"recipient_unavailable":false,"recipient_ref_check":"match","organization_name":"Alpha","pending_here":null,"future":1}]}}"#
        ])
        let rails = try PayoutLinkClient(transport: fake).status()
        XCTAssertEqual(fake.calls[0].0, "payout_rails_status")
        XCTAssertEqual(rails.first?.version, 3)
        XCTAssertEqual(rails.first?.organizationName, "Alpha")
        XCTAssertEqual(rails.first?.recipientRefCheck, "match")
    }

    func testStartAndConfirmSendWhatTheDaemonRequires() throws {
        let fake = Fake([])
        let client = PayoutLinkClient(transport: fake)
        try client.startLink(
            railId: "near_ai", organizationId: "org-a", orgsRevision: "rev", expectedVersion: 4)
        try client.confirmLink(railId: "near_ai", organizationId: "org-a")
        try client.startUnlink(railId: "near_ai", expectedVersion: 5)
        try client.confirmUnlink(railId: "near_ai")
        XCTAssertEqual(
            fake.calls.map(\.0),
            ["payout_link_start", "payout_link_confirm", "payout_link_unlink", "payout_link_confirm"])
        XCTAssertEqual(fake.calls[0].1["expected_current_version"] as? Int, 4)
        XCTAssertEqual(fake.calls[0].1["orgs_revision"] as? String, "rev")
        XCTAssertEqual(fake.calls[0].1["organization_id"] as? String, "org-a")
        XCTAssertEqual(fake.calls[1].1["confirmed"] as? Bool, true)
        XCTAssertEqual(fake.calls[1].1["action"] as? String, "link")
        XCTAssertEqual(fake.calls[2].1["expected_current_version"] as? Int, 5)
        XCTAssertNil(fake.calls[2].1["organization_id"])
        XCTAssertEqual(fake.calls[3].1["action"] as? String, "unlink")
        XCTAssertNil(fake.calls[3].1["organization_id"])
        for (_, params) in fake.calls {
            XCTAssertNil(params["token"])
            XCTAssertNil(params["near_ai_access_token"])
        }
    }

    func testARefusalKeepsTheDaemonsLabel() {
        let fake = Fake([
            #"{"id":1,"error":{"code":"unavailable","message":"payout-link-orgs-changed"}}"#,
            "garbage",
        ])
        let client = PayoutLinkClient(transport: fake)
        XCTAssertThrowsError(try client.organizations()) { error in
            XCTAssertEqual((error as? PayoutLinkClient.Failure)?.refusal, .changed)
        }
        XCTAssertThrowsError(try client.organizations()) { error in
            XCTAssertEqual((error as? PayoutLinkClient.Failure)?.label, "unparseable-response")
        }
    }

    func testOrganizationsDecode() throws {
        let fake = Fake([
            #"{"id":1,"result":{"organizations":[{"id":"org-a","name":"Alpha"}],"orgs_revision":"rev"}}"#
        ])
        let listing = try PayoutLinkClient(transport: fake).organizations()
        XCTAssertEqual(listing.organizations, [PayoutOrganization(id: "org-a", name: "Alpha")])
        XCTAssertEqual(listing.orgsRevision, "rev")
        XCTAssertEqual(fake.calls[0].0, "payout_link_orgs")
    }
}
