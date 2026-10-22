import Combine
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// A daemon whose sign-in the test finishes, cancels or times out by hand.
private final class SignInDaemon: DaemonCalling {
    enum Phase { case absent, waiting, signedIn }

    private let lock = NSLock()
    private var phase: Phase = .absent
    private var recorded: [String] = []

    var methods: [String] {
        lock.lock()
        defer { lock.unlock() }
        return recorded
    }

    func count(_ method: String) -> Int { methods.filter { $0 == method }.count }

    func set(_ next: Phase) {
        lock.lock()
        defer { lock.unlock() }
        phase = next
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        lock.lock()
        defer { lock.unlock() }
        recorded.append(method)
        switch method {
        case "near_ai_credential_status":
            switch phase {
            case .absent:
                return #"{"id":1,"result":{"state":"absent","session_state":"absent"}}"#
            case .waiting:
                return #"{"id":1,"result":{"state":"obtaining","session_state":"obtaining","attempt_id":"synthetic-attempt","attempt_status":"waiting_for_browser"}}"#
            case .signedIn:
                return #"{"id":1,"result":{"state":"present","session_state":"present"}}"#
            }
        case "near_ai_credential_start":
            phase = .waiting
            return #"{"id":1,"result":{"attempt_id":"synthetic-attempt","browser_url":"https://cloud.example.invalid/native-login","status":"waiting_for_browser"}}"#
        case "near_ai_credential_cancel":
            phase = .absent
            return #"{"id":1,"result":{}}"#
        case "near_ai_balance":
            return #"{"id":1,"result":{"state":"no_session","remaining_nanos":null,"spend_limit_nanos":null,"total_spent_nanos":null,"scale":9,"observed_at":null}}"#
        default:
            return #"{"id":1,"error":{"code":"unavailable","message":"synthetic_refusal"}}"#
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
        _ = await model.startNearAiCredential()
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

    @MainActor
    func testCancelStopsThePollWithoutReReadingTheKeyDependents() async throws {
        let daemon = SignInDaemon()
        let model = makeModel(daemon)
        _ = await model.startNearAiCredential()
        let poll = try XCTUnwrap(model.credentialPoll)
        model.cancelNearAiCredential()
        await poll.value
        for _ in 0..<300 where model.credentialAttempt != nil { try await Task.sleep(nanoseconds: 10_000_000) }

        XCTAssertNil(model.credentialAttempt)
        XCTAssertEqual(daemon.count("near_ai_credential_cancel"), 1)
        let settled = daemon.count("near_ai_credential_status")
        try await Task.sleep(nanoseconds: 60_000_000)
        XCTAssertEqual(daemon.count("near_ai_credential_status"), settled, "Nothing may poll a cancelled sign-in")
    }

    @MainActor
    func testTimedOutOrFailedSignInStopsThePoll() async throws {
        let daemon = SignInDaemon()
        let model = makeModel(daemon)
        _ = await model.startNearAiCredential()
        let poll = try XCTUnwrap(model.credentialPoll)
        // The daemon gives up on the ceremony: no key, nothing pending.
        daemon.set(.absent)
        await poll.value

        XCTAssertNil(model.credentialAttempt)
        XCTAssertNil(model.credentialPoll)
        XCTAssertEqual(model.credentialStatus.sessionState, "absent")
        XCTAssertEqual(daemon.count("get_settings"), 0, "No key arrived, so nothing behind it is re-read")
    }

    @MainActor
    func testASecondStartDoesNotCreateASecondLoop() async throws {
        let daemon = SignInDaemon()
        let model = makeModel(daemon)
        _ = await model.startNearAiCredential()
        let first = try XCTUnwrap(model.credentialPoll)
        _ = await model.startNearAiCredential()
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
        _ = await model.startNearAiCredential()
        model.credentialPollInterval = .seconds(60)
        let before = daemon.count("near_ai_credential_status")
        XCTAssertTrue(model.refreshCredentialOnActivation())
        for _ in 0..<300 where daemon.count("near_ai_credential_status") == before { try await Task.sleep(nanoseconds: 10_000_000) }
        XCTAssertGreaterThan(daemon.count("near_ai_credential_status"), before)
        model.shutdown()
    }
}
