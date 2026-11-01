import Foundation
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

@MainActor
final class PayoutLinkModelTests: XCTestCase {
    func testOlderOrganizationRefusalCannotReplaceSuccessfulRefresh() async throws {
        let daemon = PayoutReadGate(method: "payout_link_orgs", ordinal: 1, delayed: PayoutReadGate.signInRefusal)
        let model = try model(daemon)
        let old = Task { await model.refresh() }
        defer { daemon.release() }
        await fulfillment(of: [daemon.entered], timeout: 3)
        await model.refresh()
        XCTAssertEqual(model.picker.rows.map(\.id), ["org-current"])
        daemon.release()
        await old.value
        XCTAssertNil(model.refusal)
        XCTAssertEqual(model.picker.phase, .choosing)
        XCTAssertEqual(model.picker.rows.map(\.id), ["org-current"])
        XCTAssertEqual(daemon.count("payout_rails_status"), 3, "A superseded refresh must not start its names reload")
    }

    func testOlderStatusRefusalCannotHideSuccessfulRefresh() async throws {
        let daemon = PayoutReadGate(method: "payout_rails_status", ordinal: 1, delayed: PayoutReadGate.sessionRefusal)
        let model = try model(daemon)
        let old = Task { await model.refresh() }
        defer { daemon.release() }
        await fulfillment(of: [daemon.entered], timeout: 3)
        await model.refresh()
        daemon.release()
        await old.value
        XCTAssertEqual(model.rail?.version, 2)
        XCTAssertNil(model.statusRefusal)
        XCTAssertEqual(model.picker.rows.map(\.id), ["org-current"])
    }

    func testOlderNamesReloadCannotReplaceNewerStatus() async throws {
        let daemon = PayoutReadGate(method: "payout_rails_status", ordinal: 2,
                                   delayed: PayoutReadGate.status(version: 1))
        let model = try model(daemon)
        let old = Task { await model.refresh() }
        defer { daemon.release() }
        await fulfillment(of: [daemon.entered], timeout: 3)
        await model.refresh()
        daemon.release()
        await old.value
        XCTAssertEqual(model.rail?.version, 2)
    }

    func testAttachingAnotherClientInvalidatesPendingOrganizationRead() async throws {
        let daemon = PayoutReadGate(method: "payout_link_orgs", ordinal: 1, delayed: PayoutReadGate.signInRefusal)
        let model = try model(daemon)
        let old = Task { await model.loadOrganizations() }
        defer { daemon.release() }
        await fulfillment(of: [daemon.entered], timeout: 3)
        model.attach(DaemonClient(daemon: PayoutReadGate()))
        await model.refresh()
        daemon.release()
        await old.value
        XCTAssertNil(model.refusal)
        XCTAssertEqual(model.picker.rows.map(\.id), ["org-current"])
    }

    func testRefreshDoesNotAbandonAnExplicitChangeListing() async throws {
        let daemon = PayoutReadGate(method: "payout_link_orgs", ordinal: 1, delayed: PayoutReadGate.organizations)
        let model = try model(daemon)
        let change = Task { await model.change() }
        defer { daemon.release() }
        await fulfillment(of: [daemon.entered], timeout: 3)
        await model.refresh()
        daemon.release()
        await change.value
        XCTAssertTrue(model.picker.changing)
        XCTAssertEqual(model.picker.phase, .choosing)
        model.select("org-current")
        XCTAssertEqual(model.picker.selected?.id, "org-current")
    }

    func testRefreshRecoversAChangeThatRequiredSignIn() async throws {
        let daemon = PayoutReadGate(method: "payout_link_orgs", ordinal: 1, delayed: PayoutReadGate.signInRefusal)
        let model = try model(daemon)
        // Let the first listing return its refusal immediately; the next
        // listing represents the usable session after sign-in completed.
        daemon.release()
        await model.change()
        XCTAssertEqual(model.refusal, .signIn)
        XCTAssertEqual(model.picker.phase, .refused(.signIn))
        XCTAssertFalse(model.picker.changing, "A lost sign-in drops the old change and its list")
        await model.refresh()
        XCTAssertNil(model.refusal)
        XCTAssertEqual(model.picker.phase, .choosing)
        XCTAssertEqual(model.picker.rows.map(\.id), ["org-current"])
        XCTAssertNil(model.picker.selected, "Renewing sign-in must not preselect an organization")
    }

    private func model(_ daemon: PayoutReadGate) throws -> PayoutLinkModel {
        let model = PayoutLinkModel()
        XCTAssertNotNil(model.surface)
        model.attach(DaemonClient(daemon: daemon))
        return model
    }
}

private final class PayoutReadGate: DaemonCalling, @unchecked Sendable {
    let entered = XCTestExpectation(description: "Older read reached gate")
    private let gate = DispatchSemaphore(value: 0)
    private let lock = NSLock()
    private var counts: [String: Int] = [:]
    private let method: String?
    private let ordinal: Int
    private let delayed: String

    init(method: String? = nil, ordinal: Int = 1, delayed: String = "") {
        self.method = method; self.ordinal = ordinal; self.delayed = delayed
    }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
    func release() { gate.signal() }
    func count(_ method: String) -> Int { lock.withLock { counts[method, default: 0] } }
    func call(_ method: String, params: String) -> String {
        let index = lock.withLock { counts[method, default: 0] += 1; return counts[method, default: 0] }
        if method == self.method && index == ordinal {
            entered.fulfill()
            gate.wait()
            return delayed
        }
        switch method {
        case "payout_rails_status": return Self.status(version: 2)
        case "payout_link_orgs": return Self.organizations
        default: return #"{"id":0,"result":{}}"#
        }
    }
    static let signInRefusal = #"{"id":0,"error":{"code":"refused","message":"payout-link-near-ai-sign-in-required"}}"#
    static let sessionRefusal = #"{"id":0,"error":{"code":"refused","message":"account-session-required"}}"#
    static let organizations = #"{"id":0,"result":{"organizations":[{"id":"org-current","name":"Current"}],"orgs_revision":"current-revision"}}"#
    static func status(version: Int) -> String {
        "{\"id\":0,\"result\":{\"rails\":[{\"rail_id\":\"near_ai\",\"mode\":\"live\",\"linked\":false,\"version\":\(version),\"unlink_requested\":false,\"recipient_unavailable\":false}]}}"
    }
}
