import XCTest
@testable import TCShellCore

@MainActor
final class SingleOrganizationOnboardingTests: XCTestCase {
    @MainActor
    private final class Backend {
        var cloud = ["a"]
        var owned = ["a"]
        var reported = true
        var current = true
        var rail = PayoutRailStatus()
        var fail: String?
        var invalidate: String?
        var writes: [String] = []
        var selected = false
        var selectionAccepted = true
        var confirmed = false
        var confirmMatches = true
        enum Failure: Error { case refused }

        func check(_ call: String) throws {
            if invalidate == call { current = false }
            if fail == call { throw Failure.refused }
        }

        var service: SingleOrganizationOnboarding {
            SingleOrganizationOnboarding(
                organizations: {
                    try self.check(self.selected ? "selectedCloud" : "cloud")
                    return SpendOrganizations(
                        state: self.reported ? SpendOrganizations.stateListed : "",
                        organizations: self.cloud.map { .init(id: $0, name: $0, current: false) },
                        currentOrganizationID: self.selected && self.selectionAccepted ? "a" : nil, explicit: false, prompt: false,
                        revokePending: false, words: nil)
                },
                payoutOrganizations: {
                    try self.check(self.selected ? "refreshedOwned" : "owned")
                    return PayoutOrganizationListing(
                        organizations: self.owned.map { .init(id: $0, name: $0) }, orgsRevision: self.selected ? "new-revision" : "revision")
                },
                status: {
                    try self.check(self.confirmed ? "verified" : "status")
                    return [self.confirmed && self.confirmMatches ? Self.linked("a") : self.rail]
                },
                selectSpend: { id in
                    self.writes.append("spend:\(id)"); try self.check("spend"); self.selected = true
                },
                startLink: { id, revision, version in
                    self.writes.append("start:\(id):\(revision):\(version)"); try self.check("start")
                },
                confirmLink: { id in
                    self.writes.append("confirm:\(id)"); try self.check("confirm"); self.confirmed = true
                },
                isCurrent: { self.current })
        }

        static func linked(_ id: String?, check: String = "match") -> PayoutRailStatus {
            PayoutRailStatus(linked: true, state: "active", version: 2,
                             recipientRefCheck: check, organizationID: id)
        }
    }

    func testZeroManyAndUnknownCloudCountsNeverWrite() async {
        for ids in [[], ["a", "b"]] {
            let backend = Backend()
            backend.cloud = ids
            let result = await backend.service.prepare()
            XCTAssertEqual(result, .needsChoice)
            XCTAssertTrue(backend.writes.isEmpty)
        }
        let backend = Backend()
        backend.reported = false
        let result = await backend.service.prepare()
        XCTAssertEqual(result, .unavailable)
        XCTAssertTrue(backend.writes.isEmpty)
    }

    func testOwnedListingMustContainExactlyTheSameSoleOrganization() async {
        for ids in [[], ["b"], ["a", "b"]] {
            let backend = Backend()
            backend.owned = ids
            let result = await backend.service.prepare()
            XCTAssertEqual(result, .needsChoice)
            XCTAssertTrue(backend.writes.isEmpty)
        }
    }

    func testReadFailuresNeverWrite() async {
        for call in ["cloud", "owned", "status"] {
            let backend = Backend()
            backend.fail = call
            let result = await backend.service.prepare()
            XCTAssertEqual(result, .unavailable)
            XCTAssertTrue(backend.writes.isEmpty)
        }
    }

    func testSingleOrganizationSelectsInferenceAndConfirmsPaymentWithRevisionAndVersion() async {
        let backend = Backend()
        let result = await backend.service.prepare()
        XCTAssertEqual(result, .ready)
        XCTAssertEqual(backend.writes, ["spend:a", "start:a:new-revision:0", "confirm:a"])
    }

    func testUnconfirmedInferenceSelectionDoesNotLinkPaymentOrReportReady() async {
        let backend = Backend()
        backend.selectionAccepted = false
        let result = await backend.service.prepare()
        XCTAssertEqual(result, .unavailable)
        XCTAssertEqual(backend.writes, ["spend:a"])
    }

    func testExistingMatchedLinkSkipsDuplicateConfirmation() async {
        let backend = Backend()
        backend.rail = Backend.linked("a")
        let result = await backend.service.prepare()
        XCTAssertEqual(result, .ready)
        XCTAssertEqual(backend.writes, ["spend:a"])
    }

    func testOtherUnknownMismatchedOrHistoricalLinksNeverRetarget() async {
        for rail in [Backend.linked("b"), Backend.linked(nil),
                     Backend.linked("a", check: "mismatch"),
                     PayoutRailStatus(version: 2),
                     PayoutRailStatus(requestState: "pending_confirmation"),
                     PayoutRailStatus(unlinkRequested: true)] {
            let backend = Backend()
            backend.rail = rail
            let result = await backend.service.prepare()
            XCTAssertEqual(result, .needsChoice)
            XCTAssertTrue(backend.writes.isEmpty)
        }
    }

    func testPartialFailuresNeverReportReadyOrContinueWriting() async {
        for (call, count) in [("spend", 1), ("selectedCloud", 1), ("refreshedOwned", 1),
                              ("start", 2), ("confirm", 3), ("verified", 3)] {
            let backend = Backend()
            backend.fail = call
            let result = await backend.service.prepare()
            XCTAssertEqual(result, .unavailable)
            XCTAssertEqual(backend.writes.count, count)
        }
        let backend = Backend()
        backend.confirmMatches = false
        let result = await backend.service.prepare()
        XCTAssertEqual(result, .unavailable)
    }

    func testAccountChangesStopSubsequentWrites() async {
        for (call, count) in [("cloud", 0), ("owned", 0), ("status", 0),
                              ("spend", 1), ("start", 2), ("confirm", 3)] {
            let backend = Backend()
            backend.invalidate = call
            let result = await backend.service.prepare()
            XCTAssertEqual(result, .unavailable)
            XCTAssertEqual(backend.writes.count, count)
        }
    }

    func testOptionalOrganizationIDDecodesOldAndNewDaemonStatus() throws {
        let old = Data(#"{"rail_id":"near_ai","mode":"live","linked":true,"state":"active","version":2,"recipient_ref_check":"match"}"#.utf8)
        XCTAssertNil(try JSONDecoder().decode(PayoutRailStatus.self, from: old).organizationID)
        let new = Data(#"{"rail_id":"near_ai","mode":"live","linked":true,"state":"active","version":2,"recipient_ref_check":"match","organization_id":"a"}"#.utf8)
        XCTAssertEqual(try JSONDecoder().decode(PayoutRailStatus.self, from: new).organizationID, "a")
    }
}
