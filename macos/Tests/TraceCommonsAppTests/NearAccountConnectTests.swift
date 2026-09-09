import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

private final class NearRecordingDaemon: DaemonCalling {
    /// Locked pre-emptively, not because a crash was seen here. These tests
    /// drive `DaemonClient` directly and synchronously on the test thread
    /// and never go through `AppModel`, so no `Task.detached` fan-out
    /// reaches this double today. Routing it through `AppModel` would make
    /// it race exactly as CredentialCancelTests did; the guard is free and
    /// removes the trap rather than leaving it for whoever does that.
    private let lock = NSLock()
    private var responseValue = #"{"id":1,"result":{"ready":true}}"#
    private var recorded: [(String, String)] = []

    var response: String {
        get { lock.lock(); defer { lock.unlock() }; return responseValue }
        set { lock.lock(); defer { lock.unlock() }; responseValue = newValue }
    }

    var calls: [(String, String)] {
        lock.lock()
        defer { lock.unlock() }
        return recorded
    }

    func call(_ method: String, params: String) -> String {
        lock.lock()
        defer { lock.unlock() }
        recorded.append((method, params))
        return responseValue
    }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}

final class NearAccountConnectTests: XCTestCase {
    func testMissingCoreViewDoesNotGrantReadiness() {
        let daemon = NearRecordingDaemon()
        XCTAssertThrowsError(try DaemonClient(daemon: daemon).nativeWalletFlow(action: "open", flowID: "", commons: "", account: ""))
        XCTAssertEqual(daemon.calls.map(\.0), ["native_wallet_flow"])
    }
    /// A refused preparation reaches the view as the sentence the daemon
    /// chose, not as the one generic line.
    ///
    /// The sentences come from the **live dylib**, not from this file. Writing
    /// the expected words here would be a second copy of the daemon's mapping
    /// wearing a test's clothes: it would agree with the daemon on the day it
    /// was written and never again.
    ///
    /// The `assertNotEqual` against `failed` is the one that matters. Sixteen
    /// causes used to arrive here and leave as that single sentence, and a
    /// test that only checked a sentence arrived would have passed throughout.
    func testARefusedPreparationCarriesTheDaemonsOwnSentence() throws {
        let copy = try XCTUnwrap(
            WitnessCopy.decode(fromJSON: TCWitness.copyJSON() ?? "")?.admission
        )
        let specific = copy.failedProxyMissing
        XCTAssertNotEqual(
            specific, copy.failed,
            "the dylib itself must distinguish these, or this test proves nothing"
        )

        let daemon = NearRecordingDaemon()
        daemon.response = """
        {"id":1,"error":{"code":"unavailable","message":"admission_setup_proxy_missing"},
         "result":{"view":{"ready":false,"message":"\(specific)","tone":"refused","glyph":"x"}}}
        """
        let outcome = AppModel.admissionOutcome(
            from: DaemonClient(daemon: daemon),
            entryID: "selected",
            backend: "near-funded"
        )

        XCTAssertFalse(outcome.succeeded)
        XCTAssertEqual(outcome.sentence, specific)
        XCTAssertNotEqual(
            outcome.sentence, copy.failed,
            "the refusal was replaced by the generic fallback, which is #819"
        )
    }

    /// No view means no sentence, and the caller keeps its own fallback.
    ///
    /// A transport failure or a daemon older than this shell. The sentence
    /// must be absent rather than invented, so the view can tell the two
    /// apart.
    func testARefusalWithNoViewYieldsNoSentence() throws {
        let daemon = NearRecordingDaemon()
        daemon.response = #"{"id":1,"error":{"code":"unavailable","message":"admission_setup_unavailable"}}"#
        let outcome = AppModel.admissionOutcome(
            from: DaemonClient(daemon: daemon),
            entryID: "selected",
            backend: "near-funded"
        )
        XCTAssertFalse(outcome.succeeded)
        XCTAssertNil(outcome.sentence)
    }

    func testPreparationRequiresExplicitSessionBackendAndConfirmation() throws {
        let daemon = NearRecordingDaemon()
        daemon.response = #"{"id":1,"result":{"status":"ready_for_next_inference","expires_at":123}}"#
        let result = try DaemonClient(daemon: daemon).prepareAdmissionSession(entryID: "selected", backend: "near-funded")
        XCTAssertEqual(result.status, "ready_for_next_inference")
        XCTAssertNil(result.view, "an old raw success is not authoritative readiness")
        XCTAssertEqual(daemon.calls.map(\.0), ["prepare_admission_session"])
        let params = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(daemon.calls[0].1.utf8)) as? [String: Any])
        XCTAssertEqual(Set(params.keys), ["entry_id", "backend", "confirmed"])
        XCTAssertEqual(params["entry_id"] as? String, "selected")
        XCTAssertEqual(params["backend"] as? String, "near-funded")
        XCTAssertEqual(params["confirmed"] as? Bool, true)
    }
}
