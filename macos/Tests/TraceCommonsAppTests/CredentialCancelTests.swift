import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// Records what the model actually put on the wire.
private final class RecordingDaemon: DaemonCalling {
    private(set) var calls: [(method: String, params: String)] = []
    var response = #"{"id":1,"result":{}}"#

    func call(_ method: String, params paramsJSON: String) -> String {
        calls.append((method: method, params: paramsJSON))
        return response
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
