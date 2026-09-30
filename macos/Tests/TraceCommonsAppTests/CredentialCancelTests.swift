import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// Records what the model actually put on the wire.
private final class RecordingDaemon: DaemonCalling {
    /// One press fans out into several `Task.detached` calls -- the cancel
    /// itself, then the re-reads it triggers -- so this double is called
    /// from more than one thread at once, and the test reads `calls` from
    /// the main actor while they are still in flight. An unsynchronised
    /// `Array.append` under that corrupts the buffer's refcount and takes
    /// the whole test process down with SIGSEGV, after every test has
    /// reported passing. The lock is what makes the recording safe.
    private let lock = NSLock()
    private var recorded: [(method: String, params: String)] = []
    private var responseValue = #"{"id":1,"result":{}}"#

    var calls: [(method: String, params: String)] {
        lock.lock()
        defer { lock.unlock() }
        return recorded
    }

    var response: String {
        get {
            lock.lock()
            defer { lock.unlock() }
            return responseValue
        }
        set {
            lock.lock()
            defer { lock.unlock() }
            responseValue = newValue
        }
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        lock.lock()
        defer { lock.unlock() }
        recorded.append((method: method, params: paramsJSON))
        return responseValue
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }

    func openPreview(entryID: String) throws -> TCPreview {
        throw TCDaemon.TCError.daemonGone
    }

    var lastParams: [String: Any]? {
        guard let last = calls.last,
              let data = last.params.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        return object
    }
}

/// The Cancel this shell was told to draw is the Cancel it sends.
///
/// `near_ai_credential_status` resolves `obtaining` from the ceremony the
/// daemon holds, needing no attempt id, so an app restarted while the daemon
/// kept running is offered Cancel and holds nothing to name. The daemon
/// accepts an unnamed cancel, so the model sends one rather than dropping the
/// press on the floor.
final class CredentialCancelTests: XCTestCase {
    @MainActor
    func testCancelWithNoAttemptIDStillReachesTheDaemon() async throws {
        let daemon = RecordingDaemon()
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        XCTAssertNil(model.credentialAttempt)

        model.cancelNearAiCredential()
        for _ in 0..<200 where model.credentialBusy {
            try await Task.sleep(nanoseconds: 10_000_000)
        }

        // Busy is not quiescence. `submitNearAiCredential` clears
        // `credentialBusy` and only THEN issues the three re-reads the cancel
        // triggers -- refreshNearAiCredential, refreshSettings and
        // refreshHarnesses -- each on its own detached task. So the loop above
        // can exit with three calls still landing in `daemon.calls`, and
        // reading it here would be reading an array another thread is still
        // appending to. Wait for the fan-out to go quiet first.
        var lastCount = daemon.calls.count
        var quietTicks = 0
        for _ in 0..<200 {
            try await Task.sleep(nanoseconds: 10_000_000)
            let count = daemon.calls.count
            if count == lastCount {
                quietTicks += 1
                if quietTicks >= 3 { break }
            } else {
                lastCount = count
                quietTicks = 0
            }
        }

        let cancels = daemon.calls.filter { $0.method == CredentialSurface.cancelMethod }
        XCTAssertEqual(cancels.count, 1, "the press must reach the daemon")

        // Omitted, never sent empty: an empty id names no running attempt and
        // would be refused exactly as a wrong one is.
        let params = try XCTUnwrap(
            cancels.first?.params.data(using: .utf8)
                .flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] })
        XCTAssertNil(params["attempt_id"])
    }
}
