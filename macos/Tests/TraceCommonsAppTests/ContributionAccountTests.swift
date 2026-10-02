import Foundation
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

private final class ContributionDaemon: DaemonCalling {
    private let lock = NSLock()
    private var recorded: [(String, String)] = []
    private var refuse = true
    var calls: [(String, String)] { lock.lock(); defer { lock.unlock() }; return recorded }
    func call(_ method: String, params: String) -> String {
        lock.lock(); defer { lock.unlock() }
        recorded.append((method, params))
        if refuse { refuse = false; return #"{"id":1,"error":{"code":"unavailable","message":"synthetic_refusal"}}"# }
        return #"{"id":1,"result":{"status":{"authority":"bounded","ready":true,"policy_version":"v1"},"line":"Ready to contribute. Accepted contributions may earn pending credit."}}"#
    }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}
final class ContributionAccountTests: XCTestCase {
    @MainActor
    func testRedeemRetainsKeyAfterUncertainFailureAndReadsStatus() async throws {
        let daemon = ContributionDaemon()
        let model = AppModel()
        defer { model.shutdown() }
        model.setClientForTesting(DaemonClient(daemon: daemon))
        let first = await model.updateContributionAccount(inviteCode: "SYNTHETICINVITE01")
        XCTAssertFalse(first)
        let second = await model.updateContributionAccount(inviteCode: "SYNTHETICINVITE01")
        XCTAssertTrue(second)
        let calls = daemon.calls
        XCTAssertEqual(calls.map { $0.0 }, ["account_invite_redeem", "account_invite_redeem"])
        let firstBody = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(calls[0].1.utf8)) as? [String: String])
        let secondBody = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(calls[1].1.utf8)) as? [String: String])
        XCTAssertEqual(firstBody["idempotency_key"], secondBody["idempotency_key"])
        XCTAssertNotNil(UUID(uuidString: try XCTUnwrap(firstBody["idempotency_key"])))
        XCTAssertTrue(model.contributionLine.contains("pending credit"))
        let refreshed = await model.updateContributionAccount()
        XCTAssertTrue(refreshed)
        XCTAssertEqual(daemon.calls.last?.0, "account_contribution_status")
    }
}
