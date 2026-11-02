import Combine
import os
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// A daemon whose sign-in the test finishes, cancels or times out by hand.
private final class SignInDaemon: DaemonCalling {
    enum Phase: Sendable { case absent, waiting, signedIn, failedWithRetainedKey, cancelledWithRetainedSession }

    private struct State: Sendable {
        var phase: Phase = .absent
        var recorded: [String] = []
        var holdNextStatus = false
        var heldStatus = false
    }
    private let state = OSAllocatedUnfairLock(initialState: State())
    let releaseStatus = DispatchSemaphore(value: 0)

    var statusHeld: Bool { state.withLock { $0.heldStatus } }
    func holdNextStatusReply() { state.withLock { $0.holdNextStatus = true } }

    var methods: [String] {
        state.withLock { $0.recorded }
    }

    func count(_ method: String) -> Int { methods.filter { $0 == method }.count }

    func set(_ next: Phase) {
        state.withLock { $0.phase = next }
    }

    private static func statusReply(_ phase: Phase) -> String {
        switch phase {
        case .absent:
            return #"{"id":1,"result":{"state":"absent","session_state":"absent"}}"#
        case .waiting:
            return #"{"id":1,"result":{"state":"obtaining","session_state":"obtaining","attempt_id":"synthetic-attempt","attempt_status":"waiting_for_browser"}}"#
        case .signedIn:
            return #"{"id":1,"result":{"state":"present","session_state":"present","attempt_id":"synthetic-attempt","attempt_status":"complete"}}"#
        case .failedWithRetainedKey:
            return #"{"id":1,"result":{"state":"present","session_state":"absent","attempt_id":"synthetic-attempt","attempt_status":"failed"}}"#
        case .cancelledWithRetainedSession:
            return #"{"id":1,"result":{"state":"present","session_state":"present","attempt_id":"synthetic-attempt","attempt_status":"cancelled"}}"#
        }
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        if method == "near_ai_credential_status" {
            let (reply, held) = state.withLock {
                $0.recorded.append(method)
                let held = $0.holdNextStatus
                $0.holdNextStatus = false
                $0.heldStatus = held
                return (Self.statusReply($0.phase), held)
            }
            // Freeze the reply without holding the daemon lock: a newer
            // read can finish the ceremony before this old answer arrives.
            if held {
                releaseStatus.wait()
                state.withLock { $0.heldStatus = false }
            }
            return reply
        }
        return state.withLock { state in
            state.recorded.append(method)
            switch method {
            case "near_ai_credential_start":
                state.phase = .waiting
                return #"{"id":1,"result":{"attempt_id":"synthetic-attempt","browser_url":"https://cloud.example.invalid/native-login","status":"waiting_for_browser"}}"#
            case "near_ai_credential_cancel":
                state.phase = .absent
                return #"{"id":1,"result":{}}"#
            case "account_session_status":
                return #"{"id":1,"result":{"state":"known","signed_in":false,"sign_in":null}}"#
            case "near_ai_balance":
                return #"{"id":1,"result":{"state":"no_session","remaining_nanos":null,"spend_limit_nanos":null,"total_spent_nanos":null,"scale":9,"observed_at":null}}"#
            default:
                return #"{"id":1,"error":{"code":"unavailable","message":"synthetic_refusal"}}"#
            }
        }
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}

/// The browser sign-in finishes where this app is not told, so the model
/// polls it for itself, whichever view started it and whichever is on screen.
/// No view is created anywhere in this file.
final class NearAiSignInPollTests: XCTestCase {
    @MainActor
    private func makeModel(_ daemon: SignInDaemon) -> AppModel {
        let model = AppModel()
        model.credentialPollInterval = .milliseconds(5)
        model.setClientForTesting(DaemonClient(daemon: daemon))
        return model
    }

    /// Resolves when the model publishes a credential status satisfying `matches`.
    @MainActor
    private func published(_ model: AppModel, _ label: String, where matches: @escaping (CredentialStatus) -> Bool) -> (XCTestExpectation, AnyCancellable) {
        let expectation = expectation(description: label)
        let subscription = model.$credentialStatus
            .filter(matches)
            .prefix(1)
            .sink { _ in expectation.fulfill() }
        return (expectation, subscription)
    }

    @MainActor
    func testHeldAttemptIsPolledToCompletionWithNoViewOnScreen() async throws {
        let daemon = SignInDaemon()
        let model = makeModel(daemon)
        let (waiting, waitingSubscription) = published(model, "waiting") { $0.sessionState == "obtaining" }
        _ = await model.startNearAiCredential(provider: nil)
        await fulfillment(of: [waiting], timeout: 3)
        waitingSubscription.cancel()
        XCTAssertEqual(model.credentialAttempt?.attemptID, "synthetic-attempt")

        // The browser finishes. Nothing here presses a refresh.
        let (landed, landedSubscription) = published(model, "landed") { $0.sessionState == CredentialSurface.statePresent }
        daemon.set(.signedIn)
        await fulfillment(of: [landed], timeout: 3)
        landedSubscription.cancel()
        let poll = try XCTUnwrap(model.credentialPoll, "the model holds the poll for a held attempt")
        await poll.value

        XCTAssertNil(model.credentialAttempt, "A finished sign-in must stop the poll")
        XCTAssertNil(model.credentialPoll)
        XCTAssertEqual(model.accountSessionGeneration, 1, "billing and payout must re-read after the shared browser sign-in")
        // Everything the key sits behind is re-read once the sign-in lands.
        for method in ["get_settings", "harness_list", "managed_snapshot"] {
            for _ in 0..<300 where daemon.count(method) == 0 { try await Task.sleep(nanoseconds: 10_000_000) }
            XCTAssertGreaterThan(daemon.count(method), 0, "\(method) must be re-read after the sign-in lands")
        }
        // A poll that has stopped asks nothing more.
        let settled = daemon.count("near_ai_credential_status")
        try await Task.sleep(nanoseconds: 60_000_000)
        XCTAssertEqual(daemon.count("near_ai_credential_status"), settled)
    }

    /// The daemon keeps its browser ceremony across an app restart. A
    /// status read resumes that wait even though this app holds no attempt.
    @MainActor
    func testRecoveredDaemonAttemptNotifiesAccountDependentsWhenItCompletes() async throws {
        let daemon = SignInDaemon()
        daemon.set(.waiting)
        let model = makeModel(daemon)
        let (waiting, waitingSubscription) = published(model, "recovered waiting") { $0.sessionState == "obtaining" }
        model.refreshNearAiCredential()
        await fulfillment(of: [waiting], timeout: 3)
        waitingSubscription.cancel()
        let poll = try XCTUnwrap(model.credentialPoll)
        XCTAssertNil(model.credentialAttempt, "the app did not start this ceremony")
        XCTAssertEqual(model.accountSessionGeneration, 0)

        daemon.set(.signedIn)
        await poll.value

        XCTAssertNil(model.credentialPoll)
        XCTAssertEqual(model.credentialStatus.sessionState, CredentialSurface.statePresent)
        XCTAssertEqual(model.accountSessionGeneration, 1, "billing and payout must follow a recovered sign-in too")
    }

    /// A normal read of an already kept login is not a new sign-in.
    @MainActor
    func testAnAlreadyPresentLoginDoesNotPublishAnotherSignIn() async throws {
        let daemon = SignInDaemon()
        daemon.set(.signedIn)
        let model = makeModel(daemon)
        let (landed, subscription) = published(model, "already present") { $0.sessionState == CredentialSurface.statePresent }
        model.refreshNearAiCredential()
        await fulfillment(of: [landed], timeout: 3)
        subscription.cancel()
        XCTAssertNil(model.credentialPoll)
        XCTAssertEqual(model.accountSessionGeneration, 0)
    }

    @MainActor
    func testCancelStopsThePollWithoutReReadingTheKeyDependents() async throws {
        let daemon = SignInDaemon()
        let model = makeModel(daemon)
        _ = await model.startNearAiCredential(provider: nil)
        let poll = try XCTUnwrap(model.credentialPoll)
        model.cancelNearAiCredential()
        await poll.value
        for _ in 0..<300 where model.credentialAttempt != nil { try await Task.sleep(nanoseconds: 10_000_000) }

        XCTAssertNil(model.credentialAttempt)
        XCTAssertEqual(daemon.count("near_ai_credential_cancel"), 1)
        XCTAssertEqual(model.accountSessionGeneration, 0, "cancelling must not publish a successful sign-in")
        let settled = daemon.count("near_ai_credential_status")
        try await Task.sleep(nanoseconds: 60_000_000)
        XCTAssertEqual(daemon.count("near_ai_credential_status"), settled, "Nothing may poll a cancelled sign-in")
    }

    @MainActor
    func testTimedOutOrFailedSignInStopsThePoll() async throws {
        let daemon = SignInDaemon()
        let model = makeModel(daemon)
        _ = await model.startNearAiCredential(provider: nil)
        let poll = try XCTUnwrap(model.credentialPoll)
        // The daemon gives up on the ceremony: no key, nothing pending.
        daemon.set(.absent)
        await poll.value

        XCTAssertNil(model.credentialAttempt)
        XCTAssertNil(model.credentialPoll)
        XCTAssertEqual(model.credentialStatus.sessionState, "absent")
        XCTAssertEqual(daemon.count("get_settings"), 0, "No key arrived, so nothing behind it is re-read")
        XCTAssertEqual(model.accountSessionGeneration, 0, "a failed browser attempt must not refresh signed-in sections")
    }

    @MainActor
    func testAFailedFreshAttemptWithARetainedKeyDoesNotPublishSignIn() async throws {
        let daemon = SignInDaemon()
        let model = makeModel(daemon)
        _ = await model.startNearAiCredential(provider: nil)
        let poll = try XCTUnwrap(model.credentialPoll)
        daemon.set(.failedWithRetainedKey)
        await poll.value
        XCTAssertEqual(model.credentialStatus.state, CredentialSurface.statePresent)
        XCTAssertEqual(model.credentialStatus.sessionState, "absent")
        XCTAssertEqual(model.accountSessionGeneration, 0, "an inference key is not a signed-in account session")
    }

    @MainActor
    func testACancelledFreshAttemptWithARetainedSessionDoesNotPublishSignIn() async throws {
        let daemon = SignInDaemon()
        let model = makeModel(daemon)
        _ = await model.startNearAiCredential(provider: nil)
        let poll = try XCTUnwrap(model.credentialPoll)
        daemon.set(.cancelledWithRetainedSession)
        await poll.value
        XCTAssertEqual(model.credentialStatus.sessionState, CredentialSurface.statePresent)
        XCTAssertEqual(model.accountSessionGeneration, 0, "the named attempt was cancelled despite an older retained session")
    }

    @MainActor
    func testALateWaitingReplyCannotRearmACompletedSignIn() async throws {
        let daemon = SignInDaemon()
        let model = makeModel(daemon)
        model.credentialPollInterval = .seconds(60)
        defer { daemon.releaseStatus.signal(); model.shutdown() }
        let (waiting, waitingSubscription) = published(model, "initial browser wait") { $0.sessionState == "obtaining" }
        _ = await model.startNearAiCredential(provider: nil)
        await fulfillment(of: [waiting], timeout: 3)
        waitingSubscription.cancel()

        daemon.holdNextStatusReply()
        model.refreshNearAiCredential()
        for _ in 0..<300 where !daemon.statusHeld { try await Task.sleep(for: .milliseconds(5)) }
        XCTAssertTrue(daemon.statusHeld)

        let completed = expectation(description: "shared sign-in completed")
        let completionSubscription = model.$accountSessionGeneration.filter { $0 == 1 }.prefix(1)
            .sink { _ in completed.fulfill() }
        daemon.set(.signedIn)
        model.refreshNearAiCredential()
        await fulfillment(of: [completed], timeout: 3)
        completionSubscription.cancel()

        let stale = expectation(description: "old waiting reply must stay discarded")
        stale.isInverted = true
        let staleSubscription = model.$credentialStatus.dropFirst().filter { $0.sessionState == "obtaining" }
            .sink { _ in stale.fulfill() }
        daemon.releaseStatus.signal()
        await fulfillment(of: [stale], timeout: 0.3)
        staleSubscription.cancel()
        XCTAssertEqual(model.credentialStatus.sessionState, CredentialSurface.statePresent)
        XCTAssertNil(model.credentialAttempt)

        // A subsequent normal read cannot turn that late waiting reply
        // into another successful sign-in notification.
        let reads = daemon.count("near_ai_credential_status")
        model.refreshNearAiCredential()
        for _ in 0..<300 where daemon.count("near_ai_credential_status") == reads {
            try await Task.sleep(for: .milliseconds(5))
        }
        try await Task.sleep(for: .milliseconds(30))
        XCTAssertEqual(model.accountSessionGeneration, 1)
    }

    /// Settings can read the same ceremony while the shared Sign in flow
    /// owns it. Only that flow may announce successful completion.
    @MainActor
    func testObservingTheNewerBrowserFlowDoesNotPublishASecondCompletion() async throws {
        let daemon = SignInDaemon()
        let model = makeModel(daemon)
        var browserOpened = false
        model.openBrowser = { _ in browserOpened = true }
        let signIn = Task { await model.signInToNearAI(provider: nil) }
        let deadline = Date().addingTimeInterval(3)
        while !browserOpened, Date() < deadline { try await Task.sleep(for: .milliseconds(5)) }
        XCTAssertTrue(browserOpened)
        let (waiting, subscription) = published(model, "observing browser wait") { $0.sessionState == "obtaining" }
        model.refreshNearAiCredential()
        await fulfillment(of: [waiting], timeout: 3)
        subscription.cancel()
        let poll = try XCTUnwrap(model.credentialPoll)
        daemon.set(.signedIn)
        await poll.value
        await signIn.value
        XCTAssertFalse(model.nearAiSignInFailed)
        XCTAssertEqual(model.accountSessionGeneration, 1, "the observer must not duplicate the shared flow's notification")
    }

    @MainActor
    func testASecondStartDoesNotCreateASecondLoop() async throws {
        let daemon = SignInDaemon()
        let model = makeModel(daemon)
        _ = await model.startNearAiCredential(provider: nil)
        let first = try XCTUnwrap(model.credentialPoll)
        _ = await model.startNearAiCredential(provider: nil)
        XCTAssertEqual(model.credentialPoll, first, "One sign-in is one loop")
        daemon.set(.absent)
        await first.value
        XCTAssertNil(model.credentialPoll)
    }

    @MainActor
    func testNothingIsPolledWhenNoAttemptIsHeld() async throws {
        let daemon = SignInDaemon()
        let model = makeModel(daemon)
        model.refreshNearAiCredential()
        for _ in 0..<300 where daemon.count("near_ai_credential_status") == 0 { try await Task.sleep(nanoseconds: 10_000_000) }
        XCTAssertNil(model.credentialPoll, "A read that finds no sign-in in flight starts no loop")
    }

    @MainActor
    func testComingForwardRereadsTheSignInOnlyWhileAnAttemptIsHeld() async throws {
        let daemon = SignInDaemon()
        let model = makeModel(daemon)
        XCTAssertFalse(model.refreshCredentialOnActivation(), "Nothing is waiting, so nothing is read")
        _ = await model.startNearAiCredential(provider: nil)
        model.credentialPollInterval = .seconds(60)
        let before = daemon.count("near_ai_credential_status")
        XCTAssertTrue(model.refreshCredentialOnActivation())
        for _ in 0..<300 where daemon.count("near_ai_credential_status") == before { try await Task.sleep(nanoseconds: 10_000_000) }
        XCTAssertGreaterThan(daemon.count("near_ai_credential_status"), before)
        model.shutdown()
    }
}
