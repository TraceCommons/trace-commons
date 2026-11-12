import Foundation
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// An `account_sign_in` account (an invite or wallet enrolment, not NEAR
/// AI): the first sign-in waits on a browser tab that was closed, and the
/// daemon ends it when a second sign-in replaces it
/// (`account-sign-in-replaced`). Every value is synthetic.
final class ReplacedAccountSignInDaemon: DaemonCalling, @unchecked Sendable {
    private let lock = NSLock()
    private var signIns = 0
    private var signedIn = false
    /// Released when the second sign-in arrives, as the daemon's
    /// replacement ends the first call's wait.
    let firstReplaced = DispatchSemaphore(value: 0)
    var signInCount: Int { lock.withLock { signIns } }

    func call(_ method: String, params: String) -> String {
        switch method {
        case "hello":
            return Self.ok(["methods": ["account_session_status", "account_sign_in"]])
        case "account_session_status":
            let now = lock.withLock { signedIn }
            return Self.ok(["state": "known", "signed_in": now, "expires_at": NSNull(),
                            "renewal": NSNull(), "renewal_failure": NSNull(), "sign_in": NSNull()])
        case "account_sign_in":
            let number = lock.withLock { () -> Int in signIns += 1; return signIns }
            if number == 1 {
                firstReplaced.wait()
                return #"{"error":{"code":"unavailable","message":"account-sign-in-replaced"}}"#
            }
            firstReplaced.signal()
            lock.withLock { signedIn = true }
            return Self.ok(["signed_in": true, "expires_at": NSNull()])
        default:
            return #"{"error":{"code":"unknown_method","message":"unknown-method"}}"#
        }
    }

    private static func ok(_ result: [String: Any]) -> String {
        String(decoding: try! JSONSerialization.data(withJSONObject: ["result": result]), as: UTF8.self)
    }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
}

final class AccountSignInRetryTests: XCTestCase {
    /// Sign in, close the tab, Sign in again: the second press is offered
    /// and goes through, and the replaced first call reports no failure.
    @MainActor
    func test_signInAgainWhileTheBrowserIsOutReplacesTheWait() async throws {
        let daemon = ReplacedAccountSignInDaemon()
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))

        let first = Task { await model.signInToWithdraw() }
        let deadline = Date().addingTimeInterval(5)
        while daemon.signInCount < 1, Date() < deadline { try await Task.sleep(for: .milliseconds(20)) }
        XCTAssertTrue(model.accountSigningIn)
        XCTAssertTrue(model.canRetryAccountSignIn, "Sign in stays pressable while the browser is out")

        await model.signInToWithdraw()
        await first.value

        XCTAssertEqual(daemon.signInCount, 2)
        XCTAssertNil(model.accountSignInFailure, "the replaced sign-in is not a failure")
        XCTAssertFalse(model.accountSigningIn)
        XCTAssertFalse(model.canRetryAccountSignIn)
        XCTAssertEqual(model.accountSession, .signedIn)
    }
}
