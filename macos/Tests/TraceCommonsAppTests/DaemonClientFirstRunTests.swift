import Foundation
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// Answers one canned frame per method and records what was sent, so the
/// method name and the parameter bytes the picker puts on the socket have
/// somewhere to be asserted.
private final class PickerDaemon: DaemonCalling, @unchecked Sendable {
    private let lock = NSLock()
    private var recorded: [(method: String, params: String)] = []
    private var table: [String: String] = [:]

    var calls: [(method: String, params: String)] {
        lock.lock()
        defer { lock.unlock() }
        return recorded
    }

    func answer(_ method: String, with frame: String) {
        lock.lock()
        defer { lock.unlock() }
        table[method] = frame
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        lock.lock()
        defer { lock.unlock() }
        recorded.append((method: method, params: paramsJSON))
        return table[method] ?? #"{"id":1,"result":{}}"#
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }

    func openPreview(entryID: String) throws -> TCPreview {
        throw TCDaemon.TCError.daemonGone
    }

    func params(of method: String) -> [String: Any]? {
        guard let call = calls.last(where: { $0.method == method }),
              let data = call.params.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        return object
    }
}

/// Carries the daemon into the teardown block; shutdown is idempotent.
private struct Owned: @unchecked Sendable {
    let daemon: TCDaemon
}

/// The first-run picker's two daemon calls, `list_past_sessions` and
/// `include_past_sessions`, as `DaemonClient` sends and reads them.
final class DaemonClientFirstRunTests: XCTestCase {
    private static let listFrame = """
    {"id":1,"result":{"total":3,"project_mode":"notify_only","sessions":[
      {"session_id":"sess_00000000000000000000000000000001","entry_id":"8d3c1f9e-0000-4000-8000-000000000001",
       "state":"pending","selectable":true,"started_at":"2026-10-01T09:30:00.412518838Z",
       "duration_secs":1800,"title":"t","size_bytes":4096,"source":"claude"},
      {"session_id":"sess_00000000000000000000000000000002","entry_id":null,
       "state":"not_queued","selectable":true,"started_at":"2026-09-30T08:00:00Z",
       "duration_secs":null,"title":null,"size_bytes":2048,"source":"codex"},
      {"session_id":"sess_00000000000000000000000000000003","entry_id":null,
       "state":"still_active","selectable":false,"started_at":null,
       "duration_secs":null,"title":null,"size_bytes":10,"source":"claude"}
    ]}}
    """

    func test_listPastSessionsSendsItsMethodAndProjectID() throws {
        let daemon = PickerDaemon()
        daemon.answer("list_past_sessions", with: Self.listFrame)
        let client = DaemonClient(daemon: daemon)

        let list = try client.listPastSessions(projectID: "proj_0123456789abcdef")

        XCTAssertEqual(daemon.calls.map(\.method), ["list_past_sessions"])
        let params = try XCTUnwrap(daemon.params(of: "list_past_sessions"))
        XCTAssertEqual(params.count, 1)
        XCTAssertEqual(params["project_id"] as? String, "proj_0123456789abcdef")

        XCTAssertEqual(list.total, 3)
        XCTAssertEqual(list.projectMode, "notify_only")
        XCTAssertEqual(list.sessions.map(\.id), [
            "sess_00000000000000000000000000000001",
            "sess_00000000000000000000000000000002",
            "sess_00000000000000000000000000000003",
        ])
        XCTAssertEqual(list.sessions.map(\.state), [.pending, .notQueued, .stillActive])
        XCTAssertEqual(list.sessions.map(\.selectable), [true, true, false])

        let queued = list.sessions[0]
        XCTAssertEqual(queued.entryID, "8d3c1f9e-0000-4000-8000-000000000001")
        XCTAssertEqual(queued.durationSecs, 1800)
        XCTAssertEqual(queued.title, "t")
        XCTAssertEqual(queued.sizeBytes, 4096)
        XCTAssertEqual(queued.source, "claude")
        // Nanosecond precision, as chrono writes it, still dates the row.
        let started = try XCTUnwrap(queued.startedAt)
        XCTAssertEqual(started.timeIntervalSince1970, 1_790_847_000.412, accuracy: 0.001)

        let unqueued = list.sessions[1]
        XCTAssertNil(unqueued.entryID)
        XCTAssertNil(unqueued.title)
        XCTAssertNil(unqueued.durationSecs)
        XCTAssertNotNil(unqueued.startedAt)
        XCTAssertNil(list.sessions[2].startedAt)
    }

    /// A state this shell does not know is a row it cannot reason about:
    /// it reads as `never` and is unselectable whatever the wire claimed.
    func test_anUnknownStateIsUnselectable() throws {
        let daemon = PickerDaemon()
        daemon.answer("list_past_sessions", with: """
        {"id":1,"result":{"total":1,"project_mode":null,"sessions":[
          {"session_id":"sess_00000000000000000000000000000009","entry_id":null,
           "state":"something_new","selectable":true,"started_at":null,
           "duration_secs":null,"title":null,"size_bytes":1,"source":"claude"}
        ]}}
        """)
        let list = try DaemonClient(daemon: daemon).listPastSessions(projectID: "proj_x")

        XCTAssertNil(list.projectMode)
        let row = try XCTUnwrap(list.sessions.first)
        XCTAssertEqual(row.state, .never)
        XCTAssertFalse(row.selectable)
    }

    /// The ids go out exactly as chosen: no sort, no dedup -- the daemon
    /// owns both, and a reordered list is not the one the person ticked.
    func test_includeSendsTheChosenIDsInOrder() throws {
        let daemon = PickerDaemon()
        daemon.answer("include_past_sessions", with: """
        {"id":1,"result":{"approved":2,"skipped":[
          {"session_id":"sess_a","label":"session-still-active"}]}}
        """)
        let client = DaemonClient(daemon: daemon)
        let chosen = ["sess_c", "sess_a", "sess_b", "sess_a"]

        let outcome = try client.includePastSessions(projectID: "proj_0123", sessionIDs: chosen)

        XCTAssertEqual(daemon.calls.map(\.method), ["include_past_sessions"])
        let params = try XCTUnwrap(daemon.params(of: "include_past_sessions"))
        XCTAssertEqual(Set(params.keys), ["project_id", "session_ids"])
        XCTAssertEqual(params["project_id"] as? String, "proj_0123")
        XCTAssertEqual(params["session_ids"] as? [String], chosen)

        XCTAssertEqual(outcome.approved, 2)
        XCTAssertEqual(outcome.skipped.map(\.sessionID), ["sess_a"])
        XCTAssertEqual(outcome.skipped.map(\.label), ["session-still-active"])
    }

    /// Against the real dylib and a real daemon: a project the daemon does
    /// not know is its refusal, never an empty picker.
    func test_anUnknownProjectIsRefusedNotEmpty() throws {
        // Short enough for the daemon's Unix socket path on macOS.
        let directory = URL(fileURLWithPath: "/private/tmp/tc-fr-\(UUID().uuidString.prefix(8))")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let daemon = try TCDaemon(
            configDir: directory.path,
            settingsJSON: #"{"claude_source":{"mode":"off"},"codex_source":{"mode":"off"}}"#
        )
        let owned = Owned(daemon: daemon)
        addTeardownBlock {
            _ = owned.daemon.shutdown()
            try? FileManager.default.removeItem(at: directory)
        }
        let client = DaemonClient(daemon: daemon)

        do {
            let list = try client.listPastSessions(projectID: "proj_does_not_exist")
            XCTFail("an unknown project answered with \(list.sessions.count) rows")
        } catch let failure as DaemonClient.Failure {
            XCTAssertEqual(failure.message, "project-id-unrecognized")
        }
        do {
            _ = try client.includePastSessions(
                projectID: "proj_does_not_exist",
                sessionIDs: ["sess_00000000000000000000000000000001"]
            )
            XCTFail("an unknown project was included")
        } catch let failure as DaemonClient.Failure {
            XCTAssertEqual(failure.message, "project-id-unrecognized")
        }
    }
}
