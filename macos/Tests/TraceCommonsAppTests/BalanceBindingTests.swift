import os
import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Records what the model actually put on the wire.
///
/// Locked for `CredentialCancelTests.RecordingDaemon`'s reason: one refresh
/// fans out into several `Task.detached` calls, and an unsynchronised append
/// under that corrupts the buffer's refcount and takes the whole test process
/// down with SIGSEGV after every test has reported passing.
private final class RecordingDaemon: DaemonCalling {
    private struct State: Sendable {
        var recorded: [String] = []
        var response = #"{"id":1,"result":{}}"#
    }
    private let state = OSAllocatedUnfairLock(initialState: State())

    var methods: [String] {
        state.withLock { $0.recorded }
    }

    var response: String {
        get { state.withLock { $0.response } }
        set { state.withLock { $0.response = newValue } }
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        state.withLock {
            $0.recorded.append(method)
            return $0.response
        }
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }

    func openPreview(entryID: String) throws -> TCPreview {
        throw TCDaemon.TCError.daemonGone
    }
}

/// The balance row as the model really wires it.
///
/// `BalanceExportTests` builds its own `BalanceCalls` from `TCNearAiBalance`,
/// which proves the ABI answers correctly but CANNOT prove the model installs
/// those particular functions. A `balanceCalls` that had been given
/// `TCNearAiCredential.action` -- the neighbouring table, the same signature,
/// the same enum -- would satisfy every other test in this repo and would put
/// a Forget button beside somebody's money.
final class BalanceBindingTests: XCTestCase {
    @MainActor
    func testTheModelSeedsAnUnreportedBalanceRatherThanAnEmptyOne() {
        let model = AppModel()
        XCTAssertEqual(model.balanceStatus.state, "")
        XCTAssertNil(model.balanceStatus.remainingNanos, "nothing has been read yet")
        // Unreported, not "no sign-in kept here": before the first poll this
        // shell has read nothing, and either seed that named a state would be
        // a claim about somebody's account made before anything was asked.
        XCTAssertEqual(model.balanceStatus, BalanceStatus.unreported)
    }

    /// The model's own closures, against the real Rust.
    ///
    /// The states are asserted through `model.balanceCalls` specifically --
    /// not through a locally built struct -- so a mis-wired table is caught.
    @MainActor
    func testTheModelsOwnTablesAnswerTheBalanceContract() throws {
        let calls = AppModel().balanceCalls

        // OBTAIN on exactly the two states that answer it.
        for state in ["no_session", "session_expired"] {
            XCTAssertEqual(
                BalanceSurface.action(BalanceStatus(state: state), calls: calls), .obtain,
                "\(state) offers a sign-in")
        }
        for state in ["known", "no_organization", "unavailable", "", "a_state_from_2027"] {
            XCTAssertEqual(
                BalanceSurface.action(BalanceStatus(state: state), calls: calls),
                CredentialAction.none,
                "\(state) offers nothing")
        }

        // The money formatter is the balance one, and it keeps null apart
        // from zero.
        let zero = BalanceStatus(state: "known", remainingNanos: 0, scale: 9)
        let null = BalanceStatus(state: "known", remainingNanos: nil, scale: 9)
        let payload = try XCTUnwrap(
            PrivateInferenceCopy.decode(fromJSON: try XCTUnwrap(TCPrivateInference.copyJSON())))
        XCTAssertEqual(
            BalanceSurface.remaining(zero, copy: payload, calls: calls), .figure("$0.00"))
        XCTAssertEqual(
            BalanceSurface.remaining(null, copy: payload, calls: calls),
            .sentence(payload.balanceNoRemaining))

        // Only a successful read is settled, and no amount moves that.
        XCTAssertEqual(BalanceSurface.tone(zero, calls: calls), .clear)
        XCTAssertEqual(BalanceSurface.tone(null, calls: calls), .clear)
        XCTAssertNotEqual(
            BalanceSurface.tone(BalanceStatus(state: "unavailable"), calls: calls), .clear)
    }

    /// Reading the key reads the balance too.
    ///
    /// They are one card and one session: signing in, signing in again and
    /// forgetting all move both halves, and a card whose halves disagreed
    /// about whether a session exists would be worse than either alone.
    @MainActor
    func testRefreshingTheCredentialAlsoRefreshesTheBalance() async throws {
        let daemon = RecordingDaemon()
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))

        model.refreshNearAiCredential()

        var lastCount = daemon.methods.count
        var quietTicks = 0
        for _ in 0..<200 {
            try await Task.sleep(nanoseconds: 10_000_000)
            let count = daemon.methods.count
            if count == lastCount {
                quietTicks += 1
                if quietTicks >= 3 { break }
            } else {
                lastCount = count
                quietTicks = 0
            }
        }

        XCTAssertTrue(
            daemon.methods.contains(BalanceSurface.statusMethod),
            "the balance was never asked for: \(daemon.methods)")
        XCTAssertTrue(daemon.methods.contains(CredentialSurface.statusMethod))
    }
}


private final class DelayedBalanceDaemon: DaemonCalling {
    private struct State: Sendable {
        var count = 0
        var freshValues: [Bool] = []
    }
    private let state = OSAllocatedUnfairLock(initialState: State())
    let releaseFirst = DispatchSemaphore(value: 0)
    let firstReply: String
    let laterReply: String
    var count: Int { state.withLock { $0.count } }
    var requestedFresh: [Bool] { state.withLock { $0.freshValues } }

    init(first: String, later: String) { firstReply = first; laterReply = later }
    func call(_ method: String, params: String) -> String {
        guard method == BalanceSurface.statusMethod else { return #"{"id":1,"result":{}}"# }
        let payload = (try? JSONSerialization.jsonObject(with: Data(params.utf8))) as? [String: Any]
        let fresh = payload?["fresh"] as? Bool ?? false
        let index = state.withLock {
            $0.freshValues.append(fresh)
            $0.count += 1
            return $0.count
        }
        if index == 1 {
            _ = releaseFirst.wait(timeout: .now() + 5)
            return firstReply
        }
        return laterReply
    }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}

extension BalanceBindingTests {
    private static let oldBalance = #"{"id":1,"result":{"state":"known","remaining_nanos":1000000000,"scale":9}}"#
    private static let newBalance = #"{"id":1,"result":{"state":"known","remaining_nanos":2000000000,"scale":9}}"#

    @MainActor
    private func waitUntil(_ condition: () -> Bool) async throws {
        let deadline = Date().addingTimeInterval(3)
        while !condition(), Date() < deadline { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertTrue(condition())
    }

    @MainActor
    func testAnOlderAutomaticReadCannotOverwriteAManualRefresh() async throws {
        let daemon = DelayedBalanceDaemon(first: Self.oldBalance, later: Self.newBalance)
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.refreshNearAiBalance()
        try await waitUntil { daemon.count == 1 }
        model.refreshNearAiBalanceFromButton()
        try await waitUntil { model.balanceStatus.remainingNanos == 2_000_000_000 }
        daemon.releaseFirst.signal()
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertEqual(model.balanceStatus.remainingNanos, 2_000_000_000)
    }

    @MainActor
    func testRepeatedManualRefreshUsesOneRequest() async throws {
        let daemon = DelayedBalanceDaemon(first: Self.newBalance, later: Self.oldBalance)
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.refreshNearAiBalanceFromButton()
        XCTAssertTrue(model.balanceRefreshing, "the click must show activity before the daemon starts")
        model.refreshNearAiBalanceFromButton()
        try await waitUntil { daemon.count >= 1 }
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertEqual(daemon.count, 1)
        XCTAssertEqual(daemon.requestedFresh, [true], "the button must bypass the ordinary cache")
        daemon.releaseFirst.signal()
        try await waitUntil { model.balanceStatus.remainingNanos == 2_000_000_000 }
        XCTAssertFalse(model.balanceRefreshing)
    }

    @MainActor
    func testTransportFailureReplacesTheBalanceWithItsUnavailableState() async throws {
        let daemon = DelayedBalanceDaemon(first: Self.oldBalance,
            later: #"{"id":1,"error":{"code":-1,"message":"unavailable"}}"#)
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.refreshNearAiBalance()
        daemon.releaseFirst.signal()
        try await waitUntil { model.balanceStatus.state == "known" }
        model.refreshNearAiBalanceFromButton()
        try await waitUntil { daemon.count == 2 && !model.balanceRefreshing }
        XCTAssertEqual(model.balanceStatus.state, "unavailable")
        XCTAssertNil(model.balanceStatus.remainingNanos)
    }
}


extension BalanceBindingTests {
    @MainActor
    func testCredentialAndFreshRereadsSupersedePendingManualRefresh() async throws {
        let daemon = DelayedBalanceDaemon(first: Self.oldBalance, later: Self.newBalance)
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.refreshNearAiBalanceFromButton()
        try await waitUntil { daemon.count == 1 }
        model.refreshNearAiCredential()
        XCTAssertTrue(model.balanceRefreshing)
        try await waitUntil { model.balanceStatus.remainingNanos == 2_000_000_000 }
        model.refreshNearAiBalance(fresh: true)
        try await waitUntil { daemon.count == 3 }
        try await waitUntil { model.balanceStatus.remainingNanos == 2_000_000_000 }
        XCTAssertFalse(model.balanceRefreshing)
        XCTAssertEqual(daemon.requestedFresh, [true, true, true])
        daemon.releaseFirst.signal()
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertEqual(model.balanceStatus.remainingNanos, 2_000_000_000)
    }

    @MainActor
    func testMissingClientShowsUnavailableWithoutLeavingRefreshBusy() {
        let model = AppModel()
        model.refreshNearAiBalanceFromButton()
        XCTAssertEqual(model.balanceStatus.state, "unavailable")
        XCTAssertFalse(model.balanceRefreshing)
    }

    @MainActor
    func testAReplacedClientCannotPublishButUnlocksRefreshWhenItReturns() async throws {
        let old = DelayedBalanceDaemon(first: Self.oldBalance, later: Self.oldBalance)
        let replacement = DelayedBalanceDaemon(first: Self.newBalance, later: Self.newBalance)
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: old))
        model.refreshNearAiBalanceFromButton()
        XCTAssertTrue(model.balanceRefreshing)
        try await waitUntil { old.count == 1 }
        model.setClientForTesting(DaemonClient(daemon: replacement))
        old.releaseFirst.signal()
        try await waitUntil { !model.balanceRefreshing }
        XCTAssertEqual(model.balanceStatus, .unreported)
        replacement.releaseFirst.signal()
        model.refreshNearAiBalanceFromButton()
        try await waitUntil { !model.balanceRefreshing }
        XCTAssertEqual(model.balanceStatus.remainingNanos, 2_000_000_000)
    }
}
