import Foundation
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

private final class ContributionDaemon: DaemonCalling {
    private let lock = NSLock()
    private var recorded: [(String, String)] = []
    private var refuse: Bool
    private let blockReady: Bool
    private let started = DispatchSemaphore(value: 0)
    let resume = DispatchSemaphore(value: 0)
    init(refuseFirst: Bool = true, blockReady: Bool = false) { refuse = refuseFirst; self.blockReady = blockReady }
    var calls: [(String, String)] { lock.lock(); defer { lock.unlock() }; return recorded }
    func waitUntilStarted() async -> DispatchTimeoutResult {
        let signal = started
        return await withCheckedContinuation { continuation in
            DispatchQueue.global().async {
                continuation.resume(returning: signal.wait(timeout: .now() + 5))
            }
        }
    }
    func call(_ method: String, params: String) -> String {
        lock.lock(); defer { lock.unlock() }
        recorded.append((method, params))
        if refuse { refuse = false; return #"{"id":1,"error":{"code":"unavailable","message":"synthetic_refusal"}}"# }
        if blockReady { started.signal(); resume.wait() }
        let body = (try? JSONSerialization.jsonObject(with: Data(params.utf8))) as? [String: String]
        return "{\"id\":1,\"result\":{\"account_scope\":\"\(body?["account_scope"] ?? "scope-a")\",\"status\":{\"authority\":\"bounded\",\"ready\":true,\"policy_version\":\"v1\"},\"line\":\"Ready to contribute. Accepted contributions may earn pending credit.\"}}"
    }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}
final class ContributionAccountTests: XCTestCase {
    func testAccountScopeWireFieldsDecode() throws {
        let status = try DaemonDecoding.decoder().decode(DaemonStatus.self, from: Data(#"{"logged_in":true,"tenant_id":"same-tenant","account_scope":"scope-a"}"#.utf8))
        XCTAssertEqual(status.accountScope, "scope-a")
        let account = try DaemonDecoding.decoder().decode(DaemonClient.ContributionAccount.self, from: Data(#"{"account_scope":"scope-a","line":"unavailable","status":{"authority":"bounded","ready":false,"policy_version":"v1","refusal_label":"account_limit_reached","retry_after_seconds":2}}"#.utf8))
        XCTAssertEqual(account.accountScope, status.accountScope)
        XCTAssertEqual(account.status.policyVersion, "v1")
        XCTAssertEqual(account.status.refusalLabel, "account_limit_reached")
        XCTAssertEqual(account.status.retryAfterSeconds, 2)
    }

    @MainActor
    func testRedeemRetainsKeyAfterUncertainFailureAndReadsStatus() async throws {
        let daemon = ContributionDaemon()
        let model = AppModel()
        defer { model.shutdown() }
        model.setClientForTesting(DaemonClient(daemon: daemon))
        var status = DaemonStatus.unknown; status.accountScope = "scope-a"; model.setStatusForTesting(status)
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
    @MainActor
    func testSameEnrollmentLifecycleChangeClearsReadinessAndRetryKey() async throws {
        let daemon = ContributionDaemon()
        let model = AppModel(); defer { model.shutdown() }
        model.setClientForTesting(DaemonClient(daemon: daemon))
        var status = DaemonStatus.unknown; status.accountScope = "scope-a"; model.setStatusForTesting(status)
        _ = await model.updateContributionAccount(inviteCode: "SYNTHETICINVITE01")
        _ = await model.updateContributionAccount()
        XCTAssertTrue(model.contributionLine.contains("pending credit"))
        status.accountScope = "scope-b"; model.setStatusForTesting(status)
        XCTAssertEqual(model.contributionLine, model.privateInferenceCopy?.accountContributionRefresh)
        _ = await model.updateContributionAccount(inviteCode: "SYNTHETICINVITE01")
        let calls = daemon.calls.filter { $0.0 == "account_invite_redeem" }
        let bodies = try calls.map { try XCTUnwrap(JSONSerialization.jsonObject(with: Data($0.1.utf8)) as? [String:String]) }
        XCTAssertNotEqual(bodies[0]["idempotency_key"], bodies[1]["idempotency_key"])
        XCTAssertEqual(bodies[1]["account_scope"], "scope-b")
    }
    @MainActor
    func testResponseForOldLifecycleIsDiscardedDespiteSameEnrollment() async throws {
        let daemon = ContributionDaemon(refuseFirst: false, blockReady: true)
        let model = AppModel(); defer { model.shutdown() }
        model.setClientForTesting(DaemonClient(daemon: daemon))
        var status = DaemonStatus.unknown; status.accountScope = "scope-a"; model.setStatusForTesting(status)
        let request = Task { await model.updateContributionAccount() }
        let started = await daemon.waitUntilStarted()
        XCTAssertEqual(started, .success)
        status.accountScope = "scope-b"; model.setStatusForTesting(status)
        daemon.resume.signal()
        let accepted = await request.value
        XCTAssertFalse(accepted)
        XCTAssertEqual(model.contributionLine, model.privateInferenceCopy?.accountContributionRefresh)
    }

}
