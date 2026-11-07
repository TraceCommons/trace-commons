import Foundation
import os
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// Actual AppModel callbacks, with synthetic daemon replies parked after capture.
private final class ScopedReadReply: Sendable {
    let response: String
    let started = DispatchSemaphore(value: 0)
    let resume = DispatchSemaphore(value: 0)
    let parked: Bool

    init(_ response: String, parked: Bool = true) {
        self.response = response
        self.parked = parked
    }

    func waitUntilStarted() async -> Bool {
        await wait(started)
    }

    private func wait(_ signal: DispatchSemaphore) async -> Bool {
        await withCheckedContinuation { continuation in
            DispatchQueue.global().async {
                continuation.resume(returning: signal.wait(timeout: .now() + 3) == .success)
            }
        }
    }

    @MainActor
    func complete(_ task: Task<Void, Never>?) async {
        resume.signal()
        await task?.value
    }

}

private final class ScopedReadDaemon: DaemonCalling {
    private struct State: Sendable {
        var scripted: [String: [ScopedReadReply]] = [:]
        var defaults: [String: String] = [:]
        var calls: [String] = []
    }
    private let state = OSAllocatedUnfairLock(initialState: State())

    func enqueue(_ method: String, _ reply: ScopedReadReply) {
        state.withLock { $0.scripted[method, default: []].append(reply) }
    }
    func setDefault(_ method: String, _ response: String) {
        state.withLock { $0.defaults[method] = response }
    }
    func count(_ method: String) -> Int { state.withLock { $0.calls.filter { $0 == method }.count } }
    func call(_ method: String, params: String) -> String {
        let reply = state.withLock { state -> ScopedReadReply in
            state.calls.append(method)
            if var replies = state.scripted[method], !replies.isEmpty {
                let reply = replies.removeFirst()
                state.scripted[method] = replies
                return reply
            }
            let fallback: String
            switch method {
            case "list_history": fallback = Self.history("default")
            case "history_rollup": fallback = Self.rollup(0)
            case "automatic_grant": fallback = Self.grant(false)
            case "list_projects": fallback = Self.frame(#"{"projects":[]}"#)
            case "arming_suggestion": fallback = Self.frame("{}")
            case "hello": fallback = Self.frame(#"{"methods":["account_sign_out","account_session_status","unenroll"]}"#)
            case "account_session_status": fallback = Self.frame(#"{"signed_in":false}"#)
            case "status": fallback = Self.frame(#"{"logged_in":true,"account_scope":"a"}"#)
            case "withdraw_automatic_grant": fallback = Self.frame(#"{"withdrawn":true}"#)
            case "unenroll": fallback = Self.frame(#"{"unenrolled":true,"near_ai_cleared":true,"near_ai_revoked":true}"#)
            case "account_sign_out": fallback = Self.frame(#"{"signed_out":true,"near_ai_cleared":true,"near_ai_revoked":true}"#)
            default: fallback = Self.frame("{}")
            }
            return ScopedReadReply(state.defaults[method] ?? fallback, parked: false)
        }
        reply.started.signal()
        if reply.parked, reply.resume.wait(timeout: .now() + 8) != .success { return Self.failure }
        return reply.response
    }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }

    static func frame(_ result: String) -> String { #"{"id":1,"result":\#(result)}"# }
    static let failure = #"{"id":1,"error":{"code":"unavailable","message":"synthetic_refusal"}}"#
    static func grant(_ granted: Bool) -> String {
        frame(granted ? #"{"granted":true,"include_existing":true,"granted_at":"2026-11-06T12:00:00Z"}"# : #"{"granted":false}"#)
    }
    static func history(_ id: String) -> String {
        frame(#"{"history":[{"submission_id":"\#(id)","submitted_at":"2026-11-06T12:00:00Z","project_label":"synthetic","source":"codex","status":"accepted","consent_scopes":[],"credit_points_pending":0,"explanations":[]}]}"#)
    }
    static func rollup(_ value: Int) -> String {
        let counts = #"{"submitted":\#(value),"accepted":\#(value),"quarantined":0,"other":0}"#
        return frame(#"{"week":\#(counts),"month":\#(counts),"all_time":\#(counts),"credit_pending":0,"credit_final":0,"quarantined":0}"#)
    }
}

final class ScopedGrantHistoryReadsTests: XCTestCase {
    @MainActor
    private func makeModel(_ daemon: ScopedReadDaemon) -> AppModel {
        let model = AppModel()
        scope("a", on: model)
        model.setClientForTesting(DaemonClient(daemon: daemon))
        return model
    }
    @MainActor
    private func scope(_ value: String?, on model: AppModel) {
        var status = DaemonStatus.unknown
        status.accountScope = value
        model.setStatusForTesting(status)
    }
    @MainActor
    private func assertStarted(_ reply: ScopedReadReply, file: StaticString = #filePath, line: UInt = #line) async {
        let started = await reply.waitUntilStarted()
        XCTAssertTrue(started, file: file, line: line)
    }
    @MainActor
    private func waitUntil(_ predicate: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(3)
        while !predicate(), ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(5))
        }
        XCTAssertTrue(predicate())
    }

    private func statusFrame(_ scope: String, depth: Int) -> String {
        ScopedReadDaemon.frame(#"{"logged_in":true,"account_scope":"\#(scope)","queue_depth":\#(depth)}"#)
    }

    @MainActor
    func testCurrentStatusFailureThenRecoveryUpdatesActualReadState() async throws {
        let daemon = ScopedReadDaemon()
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        defer { model.shutdown() }
        XCTAssertFalse(model.statusReadFailed)
        XCTAssertFalse(model.failedReads.contains("status"))
        XCTAssertFalse(model.answeredReads.contains("status"))
        daemon.setDefault("status", ScopedReadDaemon.failure)
        let failed = model.refreshStatus(); await failed?.value
        XCTAssertTrue(model.statusReadFailed)
        XCTAssertTrue(model.failedReads.contains("status"))
        XCTAssertFalse(model.answeredReads.contains("status"))
        daemon.setDefault("status", statusFrame("a", depth: 2))
        let recovered = model.refreshStatus(); await recovered?.value
        XCTAssertFalse(model.statusReadFailed)
        XCTAssertFalse(model.failedReads.contains("status"))
        XCTAssertTrue(model.answeredReads.contains("status"))
        XCTAssertEqual(model.status.accountScope, "a")
        XCTAssertEqual(model.status.queueDepth, 2)
    }

    @MainActor
    func testStatusKeepsItsNewestSameAccountReadAndDiscardsOldFailure() async throws {
        for oldFrame in [statusFrame("a", depth: 1), ScopedReadDaemon.failure] {
            let daemon = ScopedReadDaemon()
            let model = makeModel(daemon)
            defer { model.shutdown() }
            let old = ScopedReadReply(oldFrame); defer { old.resume.signal() }
            daemon.enqueue("status", old)
            let oldRead = model.refreshStatus(); await assertStarted(old)
            daemon.setDefault("status", statusFrame("a", depth: 2))
            let current = model.refreshStatus(); await current?.value
            XCTAssertEqual(model.status.queueDepth, 2)
            model.lastActionError = "current-action"
            await old.complete(oldRead)
            XCTAssertEqual(model.status.queueDepth, 2)
            XCTAssertFalse(model.statusReadFailed)
            XCTAssertFalse(model.failedReads.contains("status"))
            XCTAssertEqual(model.lastActionError, "current-action")
        }
    }

    @MainActor
    func testOldStatusFailureCannotFailANewerStillLoadingStatus() async throws {
        let daemon = ScopedReadDaemon()
        let model = makeModel(daemon)
        defer { model.shutdown() }
        let old = ScopedReadReply(ScopedReadDaemon.failure)
        let fresh = ScopedReadReply(statusFrame("a", depth: 2))
        defer { old.resume.signal(); fresh.resume.signal() }
        daemon.enqueue("status", old)
        let oldRead = model.refreshStatus(); await assertStarted(old)
        daemon.enqueue("status", fresh)
        let current = model.refreshStatus(); await assertStarted(fresh)
        model.lastActionError = "current-action"
        await old.complete(oldRead)
        XCTAssertFalse(model.statusReadFailed)
        XCTAssertFalse(model.failedReads.contains("status"))
        XCTAssertEqual(model.status.queueDepth, 0)
        XCTAssertEqual(model.lastActionError, "current-action")
        await fresh.complete(current)
        XCTAssertEqual(model.status.queueDepth, 2)
    }

    /// Status events can arrive faster than the daemon answers. They share
    /// one read at a time, plus one after it, so the newest-read-wins fence in
    /// `refreshStatus` cannot retire every reply before any publishes.
    @MainActor
    func testStatusEventBurstSharesOneReadAndPublishesTheLatest() async throws {
        let daemon = ScopedReadDaemon()
        let model = makeModel(daemon)
        defer { model.shutdown() }
        let first = ScopedReadReply(statusFrame("a", depth: 1)); defer { first.resume.signal() }
        daemon.enqueue("status", first)
        daemon.setDefault("status", statusFrame("a", depth: 7))
        model.handle(event: .statusChanged)
        await assertStarted(first)
        for _ in 0..<20 { model.handle(event: .statusChanged) }
        XCTAssertEqual(daemon.count("status"), 1, "a burst waits for the read in flight")
        first.resume.signal()
        try await waitUntil { model.status.queueDepth == 7 }
        XCTAssertEqual(daemon.count("status"), 2, "one more read follows the burst")
        XCTAssertFalse(model.statusReadFailed)
    }

    /// Daemon calls block their thread until the daemon answers. Many of them
    /// waiting at once must not stall unrelated reads: they run on the
    /// daemon-call queue, not Swift's cooperative pool (about one thread per
    /// core), which a handful of parked calls would otherwise exhaust.
    @MainActor
    func testManyParkedDaemonCallsDoNotStallOtherReads() async throws {
        let daemon = ScopedReadDaemon()
        let model = makeModel(daemon)
        defer { model.shutdown() }
        let parked = (0..<16).map { _ in ScopedReadReply(statusFrame("a", depth: 9)) }
        defer { parked.forEach { $0.resume.signal() } }
        parked.forEach { daemon.enqueue("status", $0) }
        let reads = parked.map { _ in model.refreshStatus() }
        for reply in parked { await assertStarted(reply) }
        daemon.setDefault("list_history", ScopedReadDaemon.history("unblocked"))
        let history = model.refreshHistory()
        await history.history?.value
        XCTAssertEqual(model.history.first?.id, "unblocked")
        parked.forEach { $0.resume.signal() }
        for read in reads { await read?.value }
    }

    @MainActor
    func testOldStatusCannotReopenAnAccountAfterNewStatusAndHistory() async throws {
        let daemon = ScopedReadDaemon()
        let model = makeModel(daemon)
        defer { model.shutdown() }
        let old = ScopedReadReply(statusFrame("a", depth: 1)); defer { old.resume.signal() }
        daemon.enqueue("status", old)
        let oldRead = model.refreshStatus(); await assertStarted(old)
        daemon.setDefault("status", statusFrame("b", depth: 2))
        daemon.setDefault("list_history", ScopedReadDaemon.history("account-b"))
        let current = model.refreshStatus(); await current?.value
        let history = model.refreshHistory(); await history.history?.value; await history.rollup?.value
        XCTAssertEqual(model.status.accountScope, "b")
        XCTAssertEqual(model.history.first?.id, "account-b")
        await old.complete(oldRead)
        XCTAssertEqual(model.status.accountScope, "b")
        XCTAssertEqual(model.history.first?.id, "account-b")
    }

    @MainActor
    func testDirectSnapshotInvalidatesStatusEvenWhenItsAccountScopeIsUnchanged() async throws {
        let daemon = ScopedReadDaemon()
        let model = makeModel(daemon)
        defer { model.shutdown() }
        let old = ScopedReadReply(statusFrame("b", depth: 1)); defer { old.resume.signal() }
        daemon.enqueue("status", old)
        let oldRead = model.refreshStatus(); await assertStarted(old)
        let snapshot = try DaemonDecoding.decoder().decode(DaemonStatus.self,
            from: Data(#"{"logged_in":true,"account_scope":"a","queue_depth":2}"#.utf8))
        model.handle(event: .snapshot(pending: [], status: snapshot))
        await old.complete(oldRead)
        XCTAssertEqual(model.status.accountScope, "a")
        XCTAssertEqual(model.status.queueDepth, 2)
    }

    @MainActor
    func testStatusReplyFromAReplacedClientCannotChangeTheCurrentAccount() async throws {
        let daemon = ScopedReadDaemon()
        let model = makeModel(daemon)
        defer { model.shutdown() }
        let old = ScopedReadReply(statusFrame("a", depth: 1)); defer { old.resume.signal() }
        daemon.enqueue("status", old)
        let oldRead = model.refreshStatus(); await assertStarted(old)
        let currentDaemon = ScopedReadDaemon()
        currentDaemon.setDefault("status", statusFrame("b", depth: 2))
        model.setClientForTesting(DaemonClient(daemon: currentDaemon))
        let current = model.refreshStatus(); await current?.value
        await old.complete(oldRead)
        XCTAssertEqual(model.status.accountScope, "b")
        XCTAssertEqual(model.status.queueDepth, 2)
    }

    @MainActor
    func testHistoryAndRollupKeepTheNewestSameAccountRead() async throws {
        let daemon = ScopedReadDaemon()
        let model = makeModel(daemon)
        model.setClientForTesting(DaemonClient(daemon: daemon))
        defer { model.shutdown() }
        let old = ScopedReadReply(ScopedReadDaemon.history("old"))
        let oldRollup = ScopedReadReply(ScopedReadDaemon.rollup(1))
        defer { old.resume.signal(); oldRollup.resume.signal() }
        daemon.enqueue("list_history", old); daemon.enqueue("history_rollup", oldRollup)
        let oldReads = model.refreshHistory()
        await assertStarted(old); await assertStarted(oldRollup)
        daemon.setDefault("list_history", ScopedReadDaemon.history("new"))
        daemon.setDefault("history_rollup", ScopedReadDaemon.rollup(2))
        let current = model.refreshHistory()
        await current.history?.value; await current.rollup?.value
        XCTAssertEqual(model.history.first?.id, "new")
        await old.complete(oldReads.history); await oldRollup.complete(oldReads.rollup)
        XCTAssertEqual(model.history.first?.id, "new")
        XCTAssertEqual(model.rollup?.allTime.accepted, 2)
    }

    @MainActor
    func testHistoryRejectsAccountABAAndDisconnectEvenWithTheSameClientAgain() async throws {
        for boundary in ["account", "aba", "disconnect"] {
            let daemon = ScopedReadDaemon()
            let model = makeModel(daemon)
            let client = DaemonClient(daemon: daemon)
            model.setClientForTesting(client)
            defer { model.shutdown() }
            let old = ScopedReadReply(ScopedReadDaemon.history("old"))
            let oldRollup = ScopedReadReply(ScopedReadDaemon.rollup(1))
            defer { old.resume.signal(); oldRollup.resume.signal() }
            daemon.enqueue("list_history", old); daemon.enqueue("history_rollup", oldRollup)
            let oldReads = model.refreshHistory()
            await assertStarted(old); await assertStarted(oldRollup)
            daemon.setDefault("list_history", ScopedReadDaemon.history("current"))
            daemon.setDefault("history_rollup", ScopedReadDaemon.rollup(2))
            if boundary == "disconnect" {
                model.shutdown(); model.setClientForTesting(client)
            } else {
                scope("b", on: model)
                if boundary == "aba" { scope("a", on: model) }
            }
            let current = model.refreshHistory()
            await current.history?.value; await current.rollup?.value
            await old.complete(oldReads.history); await oldRollup.complete(oldReads.rollup)
            XCTAssertEqual(model.history.first?.id, "current", boundary)
            XCTAssertEqual(model.rollup?.allTime.accepted, 2, boundary)
        }
    }

    @MainActor
    func testAccountScopeChangeRefetchesWithoutAnOldReplyCompletingTheNewLoad() async throws {
        let daemon = ScopedReadDaemon()
        let model = makeModel(daemon)
        model.setClientForTesting(DaemonClient(daemon: daemon)); defer { model.shutdown() }
        let old = ScopedReadReply(ScopedReadDaemon.history("old"))
        let oldRollup = ScopedReadReply(ScopedReadDaemon.rollup(1))
        let fresh = ScopedReadReply(ScopedReadDaemon.history("fresh"))
        let freshRollup = ScopedReadReply(ScopedReadDaemon.rollup(2))
        defer { old.resume.signal(); oldRollup.resume.signal(); fresh.resume.signal(); freshRollup.resume.signal() }
        daemon.enqueue("list_history", old); daemon.enqueue("history_rollup", oldRollup)
        let oldReads = model.refreshHistory()
        await assertStarted(old); await assertStarted(oldRollup)
        daemon.enqueue("list_history", fresh); daemon.enqueue("history_rollup", freshRollup)
        scope("b", on: model)
        await assertStarted(fresh); await assertStarted(freshRollup)
        await old.complete(oldReads.history); await oldRollup.complete(oldReads.rollup)
        XCTAssertTrue(model.history.isEmpty)
        XCTAssertNil(model.rollup)
        XCTAssertFalse(model.historyAnswered)
        fresh.resume.signal(); freshRollup.resume.signal()
        try await waitUntil { model.history.first?.id == "fresh" && model.rollup?.allTime.accepted == 2 }
    }

    @MainActor
    func testLateHistoryFailureDoesNotChangeNewReadBookkeepingOrError() async throws {
        for currentIsLoading in [false, true] {
            let daemon = ScopedReadDaemon()
            let model = makeModel(daemon)
            model.setClientForTesting(DaemonClient(daemon: daemon)); defer { model.shutdown() }
            let old = ScopedReadReply(ScopedReadDaemon.failure)
            let oldRollup = ScopedReadReply(ScopedReadDaemon.failure)
            let fresh = ScopedReadReply(ScopedReadDaemon.history("current"), parked: currentIsLoading)
            let freshRollup = ScopedReadReply(ScopedReadDaemon.rollup(2), parked: currentIsLoading)
            defer { old.resume.signal(); oldRollup.resume.signal(); fresh.resume.signal(); freshRollup.resume.signal() }
            daemon.enqueue("list_history", old); daemon.enqueue("history_rollup", oldRollup)
            let oldReads = model.refreshHistory()
            await assertStarted(old); await assertStarted(oldRollup)
            daemon.enqueue("list_history", fresh); daemon.enqueue("history_rollup", freshRollup)
            let current = model.refreshHistory()
            await assertStarted(fresh); await assertStarted(freshRollup)
            if !currentIsLoading { await current.history?.value; await current.rollup?.value }
            model.lastActionError = "current-action"
            await old.complete(oldReads.history); await oldRollup.complete(oldReads.rollup)
            XCTAssertFalse(model.failedReads.contains("list_history"))
            XCTAssertFalse(model.failedReads.contains("history_rollup"))
            XCTAssertEqual(model.historyAnswered, !currentIsLoading)
            XCTAssertEqual(model.lastActionError, "current-action")
            if currentIsLoading { await fresh.complete(current.history); await freshRollup.complete(current.rollup) }
        }
    }

    @MainActor
    func testSignOutInvalidatesReadsBeforeAndDuringTheMutation() async throws {
        let daemon = ScopedReadDaemon()
        let model = makeModel(daemon)
        model.setClientForTesting(DaemonClient(daemon: daemon)); defer { model.shutdown() }
        let before = ScopedReadReply(ScopedReadDaemon.history("before-signout"))
        let during = ScopedReadReply(ScopedReadDaemon.history("during-signout"))
        let signOut = ScopedReadReply(ScopedReadDaemon.frame(#"{"signed_out":true,"near_ai_cleared":true,"near_ai_revoked":true}"#))
        defer { before.resume.signal(); during.resume.signal(); signOut.resume.signal() }
        daemon.enqueue("list_history", before); daemon.enqueue("account_sign_out", signOut)
        let oldReads = model.refreshHistory(); await assertStarted(before)
        let write = Task { await model.signOut() }
        await assertStarted(signOut)
        await before.complete(oldReads.history)
        XCTAssertTrue(model.history.isEmpty)
        daemon.enqueue("list_history", during)
        let duringReads = model.refreshHistory(); await assertStarted(during)
        daemon.setDefault("list_history", ScopedReadDaemon.history("after-signout"))
        signOut.resume.signal(); let signedOut = await write.value; XCTAssertTrue(signedOut)
        try await waitUntil { model.history.first?.id == "after-signout" }
        await during.complete(duringReads.history)
        XCTAssertEqual(model.history.first?.id, "after-signout")
    }

    @MainActor
    func testOlderGrantReadCannotRestoreAutomaticAfterAskCompletes() async throws {
        let daemon = ScopedReadDaemon()
        let model = makeModel(daemon)
        model.setClientForTesting(DaemonClient(daemon: daemon)); defer { model.shutdown() }
        let old = ScopedReadReply(ScopedReadDaemon.grant(true)); defer { old.resume.signal() }
        daemon.enqueue("automatic_grant", old)
        let oldRead = model.refreshProjects(); await assertStarted(old)
        let asked = await model.setAskEachTime(); XCTAssertTrue(asked)
        try await waitUntil { model.automaticGrant?.granted == false }
        await old.complete(oldRead)
        XCTAssertEqual(SharingChoice.current(model.automaticGrant), .askMe)
    }

    @MainActor
    func testGrantWriteInvalidatesOldReadsAndDefersReadsUntilCompletion() async throws {
        let daemon = ScopedReadDaemon()
        let model = makeModel(daemon)
        model.setClientForTesting(DaemonClient(daemon: daemon)); defer { model.shutdown() }
        let old = ScopedReadReply(ScopedReadDaemon.grant(false))
        let grant = ScopedReadReply(ScopedReadDaemon.grant(true))
        defer { old.resume.signal(); grant.resume.signal() }
        daemon.enqueue("automatic_grant", old); daemon.enqueue("grant_automatic_including_existing", grant)
        let oldRead = model.refreshProjects(); await assertStarted(old)
        let write = Task { await model.grantAutomatic(witness: nil) }
        await assertStarted(grant)
        await old.complete(oldRead)
        XCTAssertNil(model.automaticGrant)
        let duringWrite = model.refreshProjects()
        XCTAssertNil(duringWrite)
        XCTAssertEqual(daemon.count("automatic_grant"), 1)
        daemon.setDefault("automatic_grant", ScopedReadDaemon.grant(true))
        grant.resume.signal(); let granted = await write.value; XCTAssertEqual(granted, .granted)
        try await waitUntil { model.automaticGrant?.granted == true }
    }

    @MainActor
    func testGrantReadLatestWinsAndLateFailureHasNoCurrentEffects() async throws {
        for oldFrame in [ScopedReadDaemon.grant(true), ScopedReadDaemon.failure] {
            let daemon = ScopedReadDaemon()
            let model = makeModel(daemon)
            model.setClientForTesting(DaemonClient(daemon: daemon)); defer { model.shutdown() }
            let old = ScopedReadReply(oldFrame); defer { old.resume.signal() }
            daemon.enqueue("automatic_grant", old)
            let oldRead = model.refreshProjects(); await assertStarted(old)
            let current = model.refreshProjects(); await current?.value
            XCTAssertEqual(model.automaticGrant?.granted, false)
            model.lastActionError = "current-action"
            await old.complete(oldRead)
            XCTAssertEqual(SharingChoice.current(model.automaticGrant), .askMe)
            XCTAssertFalse(model.failedReads.contains("automatic_grant"))
            XCTAssertEqual(model.lastActionError, "current-action")
        }
    }

    @MainActor
    func testGrantReadsRejectAccountABAAndDisconnect() async throws {
        for boundary in ["account", "aba", "disconnect"] {
            let daemon = ScopedReadDaemon()
            let model = makeModel(daemon)
            let client = DaemonClient(daemon: daemon)
            model.setClientForTesting(client); defer { model.shutdown() }
            let old = ScopedReadReply(ScopedReadDaemon.grant(true)); defer { old.resume.signal() }
            daemon.enqueue("automatic_grant", old)
            let oldRead = model.refreshProjects(); await assertStarted(old)
            if boundary == "disconnect" {
                model.shutdown(); model.setClientForTesting(client)
            } else {
                scope("b", on: model)
                if boundary == "aba" { scope("a", on: model) }
            }
            let current = model.refreshProjects(); await current?.value
            await old.complete(oldRead)
            XCTAssertEqual(SharingChoice.current(model.automaticGrant), .askMe, boundary)
        }
    }

    @MainActor
    func testFinalOverlappingMutationFailureStillReadsTheCompletedGrantState() async throws {
        let daemon = ScopedReadDaemon()
        let model = makeModel(daemon)
        model.setClientForTesting(DaemonClient(daemon: daemon)); defer { model.shutdown() }
        let ask = ScopedReadReply(ScopedReadDaemon.frame(#"{"withdrawn":true}"#))
        let grant = ScopedReadReply(ScopedReadDaemon.failure)
        defer { ask.resume.signal(); grant.resume.signal() }
        daemon.enqueue("withdraw_automatic_grant", ask); daemon.enqueue("grant_automatic_including_existing", grant)
        let first = Task { await model.setAskEachTime() }; await assertStarted(ask)
        let second = Task { await model.grantAutomatic(witness: nil) }; await assertStarted(grant)
        ask.resume.signal(); let asked = await first.value; XCTAssertTrue(asked)
        XCTAssertEqual(daemon.count("automatic_grant"), 0)
        grant.resume.signal(); let outcome = await second.value
        if case .granted = outcome { XCTFail("The last mutation was refused") }
        try await waitUntil { model.automaticGrant?.granted == false }
    }

    @MainActor
    func testAccountChangeReleasesAnOldGrantMutationWithoutChangingTheNewAccount() async throws {
        let daemon = ScopedReadDaemon()
        let model = makeModel(daemon)
        model.setClientForTesting(DaemonClient(daemon: daemon)); defer { model.shutdown() }
        let grant = ScopedReadReply(ScopedReadDaemon.grant(true)); defer { grant.resume.signal() }
        daemon.enqueue("grant_automatic_including_existing", grant)
        let write = Task { await model.grantAutomatic(witness: nil) }; await assertStarted(grant)
        scope("b", on: model); scope("a", on: model)
        let current = model.refreshProjects(); await current?.value
        grant.resume.signal(); let outcome = await write.value
        if case .granted = outcome { XCTFail("An old account's completion must not finish the current grant flow") }
        XCTAssertEqual(SharingChoice.current(model.automaticGrant), .askMe)
        let later = model.refreshProjects(); XCTAssertNotNil(later); await later?.value
    }

    @MainActor
    func testAnOldMutationCompletionCannotReleaseTheCurrentAccountsWriteGate() async throws {
        for boundary in ["account", "aba", "client"] {
            let daemon = ScopedReadDaemon()
            let model = makeModel(daemon)
            defer { model.shutdown() }
            let old = ScopedReadReply(ScopedReadDaemon.grant(true))
            let fresh = ScopedReadReply(ScopedReadDaemon.grant(true))
            defer { old.resume.signal(); fresh.resume.signal() }
            daemon.enqueue("grant_automatic_including_existing", old)
            let oldWrite = Task { await model.grantAutomatic(witness: nil) }
            await assertStarted(old)
            if boundary == "client" {
                model.setClientForTesting(DaemonClient(daemon: daemon))
            } else {
                scope("b", on: model)
                if boundary == "aba" { scope("a", on: model) }
            }
            let current = model.refreshProjects(); await current?.value
            XCTAssertEqual(model.automaticGrant?.granted, false)
            daemon.enqueue("grant_automatic_including_existing", fresh)
            let freshWrite = Task { await model.grantAutomatic(witness: nil) }
            await assertStarted(fresh)
            model.lastActionError = "current-action"
            old.resume.signal()
            let oldOutcome = await oldWrite.value
            if case .granted = oldOutcome { XCTFail("An old owner cannot finish the current flow") }
            XCTAssertNil(model.refreshProjects(), boundary)
            XCTAssertEqual(model.lastActionError, "current-action", boundary)
            daemon.setDefault("automatic_grant", ScopedReadDaemon.grant(true))
            let currentScope = boundary == "account" ? "b" : "a"
            daemon.setDefault("status", ScopedReadDaemon.frame(#"{"logged_in":true,"account_scope":"\#(currentScope)"}"#))
            fresh.resume.signal()
            let freshOutcome = await freshWrite.value
            XCTAssertEqual(freshOutcome, .granted)
            try await waitUntil { model.automaticGrant?.granted == true }
        }
    }

    @MainActor
    func testGrantNotificationsRefreshTheCurrentGrantAndInvalidatePriorReplies() async throws {
        for event: DaemonEvent in [.statusChanged, .resyncRequired, .lagged(skipped: 1)] {
            let daemon = ScopedReadDaemon()
            let model = makeModel(daemon)
            model.setClientForTesting(DaemonClient(daemon: daemon)); defer { model.shutdown() }
            daemon.setDefault("automatic_grant", ScopedReadDaemon.grant(true))
            let initial = model.refreshProjects(); await initial?.value
            XCTAssertEqual(SharingChoice.current(model.automaticGrant), .automatic)
            let old = ScopedReadReply(ScopedReadDaemon.grant(true)); defer { old.resume.signal() }
            daemon.enqueue("automatic_grant", old)
            let oldRead = model.refreshProjects(); await assertStarted(old)
            daemon.setDefault("automatic_grant", ScopedReadDaemon.grant(false))
            model.handle(event: event)
            try await waitUntil { model.automaticGrant?.granted == false }
            await old.complete(oldRead)
            XCTAssertEqual(SharingChoice.current(model.automaticGrant), .askMe)
        }
    }

    @MainActor
    func testFirstRunConsumersDiscardStaleAwaitedStatusWithoutPublishingOrErrors() async throws {
        for consumer in ["complete", "watch-only", "unenroll", "scopes-fallback"] {
            for oldFrame in [statusFrame("a", depth: 1), ScopedReadDaemon.failure] {
                let daemon = ScopedReadDaemon()
                let model = makeModel(daemon)
                defer { model.shutdown() }
                daemon.setDefault("set_consent_scopes", ScopedReadDaemon.failure)
                let old = ScopedReadReply(oldFrame); defer { old.resume.signal() }
                daemon.enqueue("status", old)
                let operation = Task { @MainActor in
                    switch consumer {
                    case "complete": _ = await model.markComplete()
                    case "watch-only": _ = await model.markWatchOnlyComplete()
                    case "unenroll": _ = await model.unenroll()
                    default: _ = await model.setConsentScopes([])
                    }
                }
                await assertStarted(old)
                daemon.setDefault("list_history", ScopedReadDaemon.history("current-b"))
                let snapshot = try DaemonDecoding.decoder().decode(DaemonStatus.self,
                    from: Data(#"{"logged_in":true,"account_scope":"b","queue_depth":2}"#.utf8))
                model.handle(event: .snapshot(pending: [], status: snapshot))
                let current = model.refreshHistory(); await current.history?.value
                model.lastActionError = "current-action"
                old.resume.signal(); await operation.value
                XCTAssertEqual(model.status.accountScope, "b", consumer)
                XCTAssertTrue(model.status.loggedIn, consumer)
                XCTAssertEqual(model.status.queueDepth, 2, consumer)
                XCTAssertEqual(model.history.first?.submissionID, "current-b", consumer)
                XCTAssertEqual(model.lastActionError, "current-action", consumer)
                XCTAssertFalse(model.statusReadFailed, consumer)
            }
        }
    }

    @MainActor
    func testActualEventReceiverDiscardsReplacedAndReusedConnections() async throws {
        let daemon = ScopedReadDaemon()
        let model = makeModel(daemon)
        defer { model.shutdown() }
        let firstClient = DaemonClient(daemon: daemon)
        model.setClientForTesting(firstClient)
        let oldReceiver = try XCTUnwrap(model.eventReceiver())
        model.shutdown()
        model.setClientForTesting(firstClient)
        let current = model.refreshHistory(); await current.history?.value
        let snapshot = try DaemonDecoding.decoder().decode(DaemonStatus.self,
            from: Data(#"{"logged_in":true,"account_scope":"obsolete","queue_depth":1}"#.utf8))
        oldReceiver(.snapshot(pending: [], status: snapshot))
        XCTAssertEqual(model.status.accountScope, "a")
        model.setClientForTesting(DaemonClient(daemon: daemon))
        oldReceiver(.snapshot(pending: [], status: snapshot))
        XCTAssertEqual(model.status.accountScope, "a")
    }

    @MainActor
    func testActualEventReceiverRemainsValidAcrossAccountChangesOnSameConnection() async throws {
        let daemon = ScopedReadDaemon()
        let model = makeModel(daemon)
        defer { model.shutdown() }
        let receive = try XCTUnwrap(model.eventReceiver())
        scope("b", on: model)
        let snapshot = try DaemonDecoding.decoder().decode(DaemonStatus.self,
            from: Data(#"{"logged_in":true,"account_scope":"c","queue_depth":3}"#.utf8))
        receive(.snapshot(pending: [], status: snapshot))
        XCTAssertEqual(model.status.accountScope, "c")
        XCTAssertEqual(model.status.queueDepth, 3)
    }


    @MainActor
    func testInitialOwnerDiscoverySettlesBeforeGrantOrAskMutation() async throws {
        for ask in [false, true] {
            let daemon = ScopedReadDaemon()
            let model = AppModel()
            model.setClientForTesting(DaemonClient(daemon: daemon))
            defer { model.shutdown() }
            let initial = ScopedReadReply(statusFrame("a", depth: 1))
            let preflight = ScopedReadReply(statusFrame("a", depth: 2))
            let mutation = ScopedReadReply(ask ? ScopedReadDaemon.frame(#"{"withdrawn":true}"#) : ScopedReadDaemon.grant(true))
            defer { initial.resume.signal(); preflight.resume.signal(); mutation.resume.signal() }
            daemon.enqueue("status", initial)
            let initialTask = model.refreshStatus(); await assertStarted(initial)
            daemon.enqueue("status", preflight)
            let method = ask ? "withdraw_automatic_grant" : "grant_automatic_including_existing"
            daemon.enqueue(method, mutation)
            let operation = Task { @MainActor in
                if ask { return await model.setAskEachTime() }
                return await model.grantAutomatic(witness: nil) == .granted
            }
            try await waitUntil { daemon.count(method) > 0 || daemon.count("status") > 1 }
            XCTAssertEqual(daemon.count(method), 0, "The owner must settle before the write starts")
            XCTAssertEqual(daemon.count("status"), 2)
            await initial.complete(initialTask)
            preflight.resume.signal()
            await assertStarted(mutation)
            XCTAssertEqual(model.status.accountScope, "a")
            XCTAssertNil(model.refreshProjects(), "No read may escape the pending write gate")
            mutation.resume.signal()
            let succeeded = await operation.value
            XCTAssertTrue(succeeded, "Initial owner discovery must not turn a confirmed write into refusal")
        }
    }

    @MainActor
    func testUnknownOwnerMutationsRequireSuccessfulStatusButAllowConfirmedLegacyNilScope() async throws {
        for ask in [false, true] {
            for statusFrame in [ScopedReadDaemon.failure, ScopedReadDaemon.frame(#"{"logged_in":false}"#)] {
                let daemon = ScopedReadDaemon()
                let model = AppModel()
                model.setClientForTesting(DaemonClient(daemon: daemon))
                defer { model.shutdown() }
                daemon.setDefault("status", statusFrame)
                daemon.setDefault("grant_automatic_including_existing", ScopedReadDaemon.grant(true))
                let succeeded: Bool
                if ask { succeeded = await model.setAskEachTime() }
                else { succeeded = await model.grantAutomatic(witness: nil) == .granted }
                let expected = statusFrame != ScopedReadDaemon.failure
                XCTAssertEqual(succeeded, expected)
                XCTAssertEqual(daemon.count(ask ? "withdraw_automatic_grant" : "grant_automatic_including_existing"), expected ? 1 : 0)
            }
        }
    }

}
