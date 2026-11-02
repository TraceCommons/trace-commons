import Foundation
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// A NEAR AI account whose commons session ended: what the staging app met
/// on 2026-10-24, with no way to sign in. Every value is synthetic.
final class NearAiAccountDaemon: DaemonCalling, @unchecked Sendable {
    private let lock = NSLock()
    private var _calls: [String] = []
    var calls: [String] { lock.withLock { _calls } }
    func count(_ method: String) -> Int { calls.filter { $0 == method }.count }

    /// NEAR AI refuses the stored login until the browser sign-in finishes.
    var loginRefused = false
    var signedIn = false
    var balanceState = "ready"
    /// How long `near_ai_account_sign_in` blocks its thread, as a ceremony
    /// waiting on the network does.
    var signInDelay: TimeInterval = 0
    /// `sign_in` on the status read: `near_ai`, or nil (null) for an
    /// account that signs in through `account_sign_in`.
    var signInPath: String? = "near_ai"
    /// The browser sign-in ends without a login (closed, timed out).
    var attemptFails = false
    private var attempt = "absent"
    var cancelFails = false
    var startFails = false
    var cancelDelay: TimeInterval = 0
    private var attemptNumber = 0
    private var holdStatus = false
    private var _statusHeld = false
    var statusHeld: Bool { lock.withLock { _statusHeld } }
    let releaseStatus = DispatchSemaphore(value: 0)
    func holdNextSuccessfulStatus() { lock.withLock { holdStatus = true } }
    private var _cancelledIDs: [String] = []
    var cancelledIDs: [String] { lock.withLock { _cancelledIDs } }
    /// The `provider` each `near_ai_credential_start` named, nil for none.
    private var _startedProviders: [String?] = []
    var startedProviders: [String?] { lock.withLock { _startedProviders } }

    func call(_ method: String, params: String) -> String {
        lock.withLock { _calls.append(method) }
        switch method {
        case "hello":
            return Self.ok(["methods": ["account_session_status", "account_sign_in", "near_ai_account_sign_in"]])
        case "near_ai_balance":
            return Self.ok(["state": balanceState])
        case "account_session_status":
            let now = lock.withLock { signedIn }
            return Self.ok(["state": "known", "signed_in": now, "expires_at": NSNull(),
                            "renewal": now ? NSNull() : "failed",
                            "renewal_failure": now ? NSNull() : "near_ai_sign_in_required",
                            "sign_in": signInPath ?? NSNull()])
        case "near_ai_account_sign_in":
            if signInDelay > 0 { Thread.sleep(forTimeInterval: signInDelay) }
            return lock.withLock {
                if loginRefused {
                    return #"{"error":{"code":"unavailable","message":"near_ai_sign_in_required"}}"#
                }
                signedIn = true
                return Self.ok(["signed_in": true, "expires_at": NSNull()])
            }
        case "near_ai_credential_start":
            let named = (try? JSONSerialization.jsonObject(with: Data(params.utf8)) as? [String: Any])?["provider"]
            lock.withLock { _startedProviders.append(named as? String) }
            if startFails { return #"{"error":{"code":"unavailable","message":"start-failed"}}"# }
            return lock.withLock {
                attemptNumber += 1
                attempt = attemptFails ? "failed" : "waiting_for_browser"
                return Self.ok(["attempt_id": "synthetic-attempt-\(attemptNumber)",
                                "browser_url": "https://cloud.example.invalid/native-login/\(attemptNumber)",
                                "status": "waiting_for_browser"])
            }
        case "near_ai_credential_cancel":
            if cancelDelay > 0 { Thread.sleep(forTimeInterval: cancelDelay) }
            return lock.withLock {
                let object = try? JSONSerialization.jsonObject(with: Data(params.utf8)) as? [String: Any]
                _cancelledIDs.append(object?["attempt_id"] as? String ?? "unnamed")
                if cancelFails { return #"{"error":{"code":"unavailable","message":"cancel-failed"}}"# }
                if object?["attempt_id"] as? String == "synthetic-attempt-\(attemptNumber)" {
                    attempt = "cancelled"
                }
                return Self.ok([:])
            }
        case "near_ai_credential_status":
            // The old session is still kept while the browser sign-in runs:
            // a poll that trusted it would end the wait at once.
            let snapshot = lock.withLock { () -> (String, Int, Bool) in
                let held = holdStatus && params.contains("attempt_id")
                if held { holdStatus = false; _statusHeld = true }
                return (held ? "complete" : attempt, attemptNumber, held)
            }
            if snapshot.2 { releaseStatus.wait() }
            return Self.ok(["state": "present", "session_state": "present",
                            "attempt_id": "synthetic-attempt-\(snapshot.1)", "attempt_status": snapshot.0])
        case "account_sign_in":
            return #"{"error":{"code":"unavailable","message":"account-sign-in-not-for-this-account"}}"#
        default:
            return #"{"error":{"code":"unknown_method","message":"unknown-method"}}"#
        }
    }

    /// The browser sign-in finishes: NEAR AI accepts the new login.
    func finishBrowserSignIn() {
        lock.withLock {
            attempt = "complete"
            loginRefused = false
        }
    }

    private static func ok(_ result: [String: Any]) -> String {
        String(decoding: try! JSONSerialization.data(withJSONObject: ["result": result]), as: UTF8.self)
    }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
}

final class SignInAgainTests: XCTestCase {
    @MainActor
    private func model(_ daemon: NearAiAccountDaemon) -> (AppModel, () -> [URL]) {
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        let opened = NSLock()
        var urls: [URL] = []
        model.openBrowser = { url in opened.withLock { urls.append(url) } }
        return (model, { opened.withLock { urls } })
    }

    /// Sign in again on a NEAR AI account runs the NEAR AI path, never
    /// `account_sign_in`, and trusts only the status read after it.
    @MainActor
    func test_aNearAiAccountSignsInAgainWithItsNearAiLogin() async {
        let daemon = NearAiAccountDaemon()
        daemon.signedIn = false
        daemon.loginRefused = false
        let (model, opened) = model(daemon)
        let generation = model.accountSessionGeneration

        await model.signInToAccount()

        XCTAssertEqual(model.accountSession, .signedIn)
        XCTAssertNil(model.accountSignInFailure)
        XCTAssertEqual(model.accountSignInPath, "near_ai")
        XCTAssertEqual(daemon.count("account_sign_in"), 0, "a NEAR AI account never uses account_sign_in")
        XCTAssertEqual(daemon.count("near_ai_account_sign_in"), 1)
        XCTAssertEqual(daemon.calls.last, "account_session_status", "the session is read again after")
        XCTAssertEqual(daemon.count("near_ai_credential_start"), 0, "Trace-only sign-in reuses the healthy NEAR AI login")
        XCTAssertTrue(opened().isEmpty, "a usable NEAR AI login needs no browser")
        XCTAssertEqual(model.accountSessionGeneration, generation + 1, "the sections that said Sign in again read again")
    }

    /// NEAR AI refused the stored login: Sign in again signs in to NEAR AI
    /// in the browser first, waits for that attempt (not the old session,
    /// which is still kept), and then asks the daemon once more.
    @MainActor
    func test_aRefusedNearAiLoginSignsInToNearAiFirst() async throws {
        let daemon = NearAiAccountDaemon()
        daemon.loginRefused = true
        let (model, opened) = model(daemon)

        let signIn = Task { await model.signInToAccount() }
        let deadline = Date().addingTimeInterval(5)
        while opened().isEmpty, Date() < deadline { try await Task.sleep(for: .milliseconds(20)) }
        XCTAssertEqual(opened().map(\.host), ["cloud.example.invalid"], "the NEAR AI sign-in opened")
        // The old session is still present; the wait must not end on it.
        try await Task.sleep(for: .milliseconds(1_500))
        XCTAssertEqual(daemon.count("near_ai_account_sign_in"), 1, "not asked again before the browser finished")
        daemon.finishBrowserSignIn()
        await signIn.value

        XCTAssertEqual(model.accountSession, .signedIn)
        XCTAssertNil(model.accountSignInFailure)
        XCTAssertEqual(daemon.count("near_ai_credential_start"), 1)
        XCTAssertEqual(daemon.count("near_ai_account_sign_in"), 2)
    }

    /// The freeze: a daemon call that blocks on the network must not hold
    /// the main thread or the cooperative pool.
    @MainActor
    func test_aSlowSignInLeavesTheMainActorAndThePoolFree() async throws {
        let daemon = NearAiAccountDaemon()
        daemon.signInDelay = 1.5
        let (model, _) = model(daemon)

        // More blocked calls than the cooperative pool has threads.
        let blocked = ProcessInfo.processInfo.activeProcessorCount * 2
        let slow = (0..<blocked).map { _ in Task { await DaemonWork.run { Thread.sleep(forTimeInterval: 1.5) } } }
        let signIn = Task { await model.signInToAccount() }

        let started = Date()
        // The main actor answers, and a detached task (the pool) runs.
        await Task.yield()
        let pooled = await Task.detached { 1 }.value
        let waited = Date().timeIntervalSince(started)

        XCTAssertEqual(pooled, 1)
        XCTAssertLessThan(waited, 0.5, "the pool waited \(waited)s behind blocked daemon calls")
        XCTAssertNotEqual(model.accountSession, .signedIn, "the main actor answered while the sign-in was still blocked")
        await signIn.value
        for task in slow { await task.value }
        XCTAssertEqual(model.accountSession, .signedIn)
    }
}
