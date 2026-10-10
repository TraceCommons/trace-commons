import Foundation
import TCBridge
import TCShellCore
import XCTest

/// `TCDaemon` as a transport, by forwarding. The app target declares the
/// conformance on `TCDaemon` itself (`DaemonCalling.swift`); declaring it
/// again here would be a second conformance in the one test bundle
/// `swift test` links, so this forwards instead.
private final class DaemonPipe: DaemonTransport, @unchecked Sendable {
    let daemon: TCDaemon
    init(_ daemon: TCDaemon) { self.daemon = daemon }
    func call(_ method: String, params paramsJSON: String) -> String {
        daemon.call(method, params: paramsJSON)
    }
}

/// Carries the daemon into the teardown block. `TCDaemon` is safe to use
/// from any thread (its own doc), and shutdown is idempotent.
private struct Owned: @unchecked Sendable {
    let daemon: TCDaemon
    init(_ daemon: TCDaemon) { self.daemon = daemon }
}

private func live(_ daemon: TCDaemon) -> LiveDaemonClient {
    LiveDaemonClient(transport: DaemonPipe(daemon))
}

/// K1 of #1173: `LiveDaemonClient` against the real dylib and a real daemon
/// on a throwaway state directory. The unit tests prove what the client
/// sends and how it reads canned frames; this proves the daemon on this
/// branch answers those requests in shapes the contract decodes.
final class LiveDaemonClientIntegrationTests: XCTestCase {
    /// Both session roots declared off, so the daemon starts without ever
    /// reading a real ~/.claude or ~/.codex tree.
    private let settings = #"{"claude_source":{"mode":"off"},"codex_source":{"mode":"off"}}"#

    private func startDaemon() throws -> TCDaemon {
        // Short enough for the daemon's Unix socket path on macOS.
        let directory = URL(fileURLWithPath: "/private/tmp/tc-k1-\(UUID().uuidString.prefix(8))")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let daemon = try TCDaemon(configDir: directory.path, settingsJSON: settings)
        let owned = Owned(daemon)
        addTeardownBlock {
            _ = owned.daemon.shutdown()
            try? FileManager.default.removeItem(at: directory)
        }
        return daemon
    }

    func testStatusQueueProjectsHarnessesAndSettingsDecodeFromTheRealDaemon() async throws {
        let daemon = try startDaemon()
        let client = live(daemon)

        let status = try await client.status()
        XCTAssertEqual(status.queueDepth, 0)
        // A fresh store owes nothing, and says so; it is not left unknown.
        XCTAssertEqual(status.decisionsOwed, 0)

        let pending = try await client.listPending(projectId: nil)
        XCTAssertEqual(pending, [])
        let kept = try await client.listKept()
        XCTAssertEqual(kept, [])

        let projects = try await client.listProjects()
        XCTAssertEqual(projects.projects, [])

        _ = try await client.harnessList()

        let settings = try await client.settings()
        XCTAssertNotNil(settings.scrubCheckMode)
    }

    func testASettingsWriteRoundTripsThroughTheRealDaemon() async throws {
        let daemon = try startDaemon()
        let client = live(daemon)
        let updated = try await client.setScrubCheck(.manual)
        XCTAssertEqual(updated.scrubCheckMode, .manual)
        let reread = try await client.settings()
        XCTAssertEqual(reread.scrubCheckMode, .manual)
    }

    func testAnUnknownProjectIsTheDaemonsRefusalNotUnreachable() async throws {
        let client = live(try startDaemon())
        do {
            _ = try await client.listPending(projectId: "proj_does_not_exist")
            XCTFail("an unknown project answered")
        } catch DaemonDataError.daemon(_, let message) {
            XCTAssertEqual(message, "project-id-unrecognized")
        }
    }

    /// `tc_call` serves it: a bad entry id is the daemon's refusal, which
    /// proves the call reached the async dispatcher rather than being
    /// refused on the way as `preview-unsure-spans-requires-async`.
    func testUnsureSpansReachTheDaemonOverTcCall() async throws {
        let daemon = try startDaemon()
        let client = live(daemon)
        do {
            _ = try await client.previewUnsureSpans(entryId: "not-a-uuid", bodyDigest: "sha256:00")
            XCTFail("a bad entry id answered")
        } catch DaemonDataError.daemon(_, let message) {
            XCTAssertEqual(message, "entry_id-invalid")
        }
    }

    func testAStoppedDaemonIsUnreachable() async throws {
        let daemon = try startDaemon()
        let client = live(daemon)
        _ = daemon.shutdown()
        for call in [
            { _ = try await client.status() },
            { _ = try await client.listProjects() },
            { _ = try await client.previewUnsureSpans(entryId: "e", bodyDigest: "d") },
        ] as [() async throws -> Void] {
            do {
                try await call()
                XCTFail("a stopped daemon answered")
            } catch {
                XCTAssertEqual(error as? DaemonDataError, .unreachable)
            }
        }
    }

    /// Every nudge request in the shape the real daemon accepts, and the
    /// settings it reads back afterwards.
    func testNudgeRequestsAreAcceptedByTheRealDaemon() async throws {
        let daemon = try startDaemon()
        let client = live(daemon)
        let idle = try await client.listPending(projectId: nil, filter: .idleSessions, order: .suggested)
        XCTAssertEqual(idle, [])
        try await client.nudgeOpened(.verdictsLanded)
        try await client.nudgeDecline(.idleSessions)
        try await client.setSuggestionsEnabled(false)
        try await client.setMenuBarMarkEnabled(false)
        try await client.setNotificationsEnabled(false)
        try await client.setNotifyKind("idle_sessions", on: true)
        try await client.dismissNotifyOffer(kind: "verdicts_landed")
        let settings = try await client.settings()
        XCTAssertEqual(settings.suggestionsEnabled, false)
        XCTAssertEqual(settings.menuBarMarkEnabled, false)
        XCTAssertEqual(settings.notificationsEnabled, false)
        XCTAssertEqual(settings.notify?.idleSessions, true)
        XCTAssertNotEqual(settings.verdictsOfferPending, true)
        // Verdict news has no "Not now"; the daemon refuses it.
        do {
            try await client.nudgeDecline(.verdictsLanded)
            XCTFail("declined verdict news")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .daemon(code: "bad_params", message: "nudge-kind-not-declinable"))
        }
    }

    /// A subscriber that declares it renders `reengage_due` is registered
    /// and still receives the ordinary frames.
    func testASubscriberThatAcceptsReengagementReceivesOrdinaryFrames() async throws {
        let daemon = try startDaemon()
        let client = live(daemon)
        let subscription = try XCTUnwrap(daemon.subscribe(accepts: ["reengage_due"]) { client.deliver(eventJSON: $0) })
        defer { daemon.unsubscribe(subscription) }
        let watchdog = Task {
            try? await Task.sleep(for: .seconds(30))
            client.finishEvents()
        }
        defer { watchdog.cancel() }
        var iterator = client.events().makeAsyncIterator()
        guard case .snapshot? = await iterator.next() else { return XCTFail("first event is not a snapshot") }
        _ = daemon.call("pause", params: "{}")
        let next = await iterator.next()
        XCTAssertEqual(next, .statusChanged)
        client.finishEvents()
    }

    /// In process, a declaration changes by registering the same handler
    /// anew and ending the old registration: afterwards the handler gets
    /// each frame exactly as often as an untouched subscription does, not
    /// twice as often and not never.
    func testRedeclaringInProcessDeliversEachFrameOnce() throws {
        let daemon = try startDaemon()
        let seen = FrameLog()
        let control = FrameLog()
        let plain = try XCTUnwrap(daemon.subscribe { seen.add($0) })
        let declared = try XCTUnwrap(daemon.redeclare(plain, accepts: ["reengage_due"]))
        defer { daemon.unsubscribe(declared) }
        let untouched = try XCTUnwrap(daemon.subscribe { control.add($0) })
        defer { daemon.unsubscribe(untouched) }
        _ = daemon.call("pause", params: "{}")
        let deadline = Date().addingTimeInterval(10)
        while control.count(of: "status_changed") == 0, Date() < deadline {
            RunLoop.current.run(until: Date().addingTimeInterval(0.05))
        }
        // Give a late copy, if there were one, time to land.
        RunLoop.current.run(until: Date().addingTimeInterval(0.6))
        XCTAssertGreaterThan(control.count(of: "status_changed"), 0)
        XCTAssertEqual(seen.count(of: "status_changed"), control.count(of: "status_changed"))
    }

    /// The stream the app feeds from its one `tc_subscribe` callback: opens
    /// with a snapshot of the real queue, then carries the daemon's frames.
    func testEventsOpenWithASnapshotAndCarryRealFrames() async throws {
        let daemon = try startDaemon()
        let client = live(daemon)
        let subscription = try XCTUnwrap(daemon.subscribe { client.deliver(eventJSON: $0) })
        defer { daemon.unsubscribe(subscription) }
        // A daemon that never sends a frame would otherwise hang the job
        // until its timeout. Ending the stream turns that into a nil event,
        // which fails the assertions below.
        let watchdog = Task {
            try? await Task.sleep(for: .seconds(30))
            client.finishEvents()
        }
        defer { watchdog.cancel() }

        var iterator = client.events().makeAsyncIterator()
        guard case .snapshot(let pending, let status)? = await iterator.next() else {
            return XCTFail("first event is not a snapshot")
        }
        XCTAssertEqual(pending, [])
        XCTAssertEqual(status?.decisionsOwed, 0)

        // `pause` moves status and nothing else; the daemon says so.
        _ = daemon.call("pause", params: "{}")
        let next = await iterator.next()
        XCTAssertEqual(next, .statusChanged)
        client.finishEvents()
    }
}

/// Event names a subscription callback received, from the Rust thread.
private final class FrameLog: @unchecked Sendable {
    private let lock = NSLock()
    private var names: [String] = []

    func add(_ json: String) {
        let name = (try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])?["event"] as? String
        lock.lock()
        names.append(name ?? "")
        lock.unlock()
    }

    func count(of name: String) -> Int {
        lock.lock()
        defer { lock.unlock() }
        return names.filter { $0 == name }.count
    }
}
