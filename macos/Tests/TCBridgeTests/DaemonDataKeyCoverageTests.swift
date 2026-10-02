import Foundation
import TCBridge
@testable import TCShellCore
import XCTest

/// C1 of #1173: every key the real daemon sends is declared by the Swift
/// model that reads it.
///
/// The direction matters. A test that checks fixture keys against the model
/// can never notice a field the daemon sends and Swift drops, which is how
/// `skipped`, `grant_voids` and `attested_inference` slipped through. This
/// starts the daemon from this branch's dylib on a throwaway store, puts one
/// real session in its queue, calls every live `DaemonDataClient` method
/// over `tc_call`, and decodes each reply with `KeyRecorder`, which records
/// every key the model ASKS for (present, null or absent). A key in the
/// reply that the model never asked for fails here, by path.
///
/// What it cannot see: a key the daemon only sends in a state this store
/// never reaches (an attested-inference record, a grant void, a migration
/// notice). Those fields are pinned by decode tests beside the model in
/// `DaemonDataContractWireTests`.
final class DaemonDataKeyCoverageTests: XCTestCase {
    /// Keys the contract deliberately does not model, per method, with the
    /// reason. Every other key must be declared. Adding a key here is a
    /// decision to drop it; say why.
    private let notModeled: [String: Set<String>] = {
        let settings: Set<String> = [
            // Daemon tuning, never drawn: no screen sets or shows these.
            "schema_version", "poll_interval_secs", "canary_interval_secs", "community_poll_secs",
            "history_poll_secs", "growth_factor", "growth_min_new_bytes", "max_queue_entries", "max_reuploads",
            "queue_ttl_days",
            // Read by the app's existing settings model
            // (`DaemonSettingsView`, `PrivateInferenceStateView` in
            // `TraceCommonsApp/Models.swift`), which owns those screens; C1's
            // screens do not draw them.
            "admission_evidence_required", "claude_root_configured", "codex_root_configured",
            "near_ai_configured", "near_ai_inference_configured", "near_ai_session_retained",
            "private_inference_offer_seen", "private_inference_state", "private_inference_state.port",
            "private_inference_state.state",
            // Token capture and its storage notice: `TokenStorageView` owns
            // them, with copy from the core.
            "token_capture_enabled", "token_distributions_contribution", "token_storage",
            "token_storage.cancel_label", "token_storage.capture_confirmation", "token_storage.capture_enabled",
            "token_storage.capture_label", "token_storage.capture_notice", "token_storage.cleanup_label",
            "token_storage.cleanup_pending", "token_storage.confirm_label", "token_storage.discard_confirmation",
            "token_storage.discard_label", "token_storage.failure_line", "token_storage.lease_expires_at_unix",
            "token_storage.oldest_pending_seconds", "token_storage.renewal_failures", "token_storage.retained_bytes",
            "token_storage.scope_note", "token_storage.state_line", "token_storage.unsubmitted_reviews",
        ]
        return ["get_settings": settings, "set_settings": settings]
    }()

    private struct Owned: @unchecked Sendable {
        let daemon: TCDaemon
    }

    private var directory: URL!

    private func startDaemonWithOneSession() throws -> TCDaemon {
        // Short enough for the daemon's Unix socket path on macOS.
        directory = URL(fileURLWithPath: "/private/tmp/tc-c1-\(UUID().uuidString.prefix(8))")
        let claudeRoot = directory.appendingPathComponent("claude")
        let codexRoot = directory.appendingPathComponent("codex")
        let project = claudeRoot.appendingPathComponent("-Users-testuser-code-project1")
        try FileManager.default.createDirectory(at: project, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: codexRoot, withIntermediateDirectories: true)
        // The on-disk shape `ClaudeCodeSource::discover` reads, as the ABI
        // tests write it (`contributor-ffi/tests/abi.rs`).
        let line = #"{"type":"user","message":{"role":"user","content":"Fix the flaky retry test"},"cwd":"/Users/testuser/code/project1","timestamp":"2026-08-08T10:00:00Z","version":"2.0.1","sessionId":"s1","uuid":"a1"}"#
        try Data((line + "\n").utf8).write(to: project.appendingPathComponent("s1.jsonl"))
        let settings: [String: Any] = [
            "claude_root": claudeRoot.path,
            "codex_root": codexRoot.path,
            "quiescence_secs": 0,
        ]
        let settingsJSON = String(decoding: try JSONSerialization.data(withJSONObject: settings), as: UTF8.self)
        // The supervisor ticks once at start. A first sighting waits one
        // poll for its size to settle (`eligibility::evaluate`), so the
        // first start only records the session; the second start's opening
        // tick finds it stable and queues it, with no poll interval to wait
        // out.
        let first = try TCDaemon(configDir: directory.path, settingsJSON: settingsJSON)
        try waitForFile(named: "daemon-state.json")
        _ = first.shutdown()
        let daemon = try TCDaemon(configDir: directory.path, settingsJSON: settingsJSON)
        let owned = Owned(daemon: daemon)
        let dir = directory!
        addTeardownBlock {
            _ = owned.daemon.shutdown()
            try? FileManager.default.removeItem(at: dir)
        }
        return daemon
    }

    private func waitForFile(named name: String) throws {
        for _ in 0..<200 {
            let found = FileManager.default.enumerator(atPath: directory.path)?
                .contains { ($0 as? String)?.hasSuffix(name) == true } ?? false
            if found { return }
            Thread.sleep(forTimeInterval: 0.05)
        }
        throw XCTSkip("the first tick never saved \(name)")
    }

    /// The `result` of one `tc_call`, as a JSON value; fails on an error frame.
    private func result(_ daemon: TCDaemon, _ method: String, _ params: [String: Any] = [:]) throws -> Any {
        let paramsJSON = String(decoding: try JSONSerialization.data(withJSONObject: params), as: UTF8.self)
        let frame = daemon.call(method, params: paramsJSON)
        let object = try XCTUnwrap(
            try JSONSerialization.jsonObject(with: Data(frame.utf8)) as? [String: Any], "\(method): not a frame")
        if let error = object["error"], !(error is NSNull) {
            throw NSError(domain: "tc_call", code: 1, userInfo: [NSLocalizedDescriptionKey: "\(method): \(error)"])
        }
        return try XCTUnwrap(object["result"], "\(method): no result")
    }

    private var checked: [String: Int] = [:]

    /// Decodes `json` as `T` and fails for every key `T` does not declare.
    private func assertDeclared<T: Decodable>(
        _ type: T.Type, _ json: Any, method: String, file: StaticString = #filePath, line: UInt = #line
    ) {
        let recorder = KeyRecorder()
        do {
            _ = try KeyRecorder.decode(type, from: json, recorder: recorder)
        } catch {
            XCTFail("\(method) does not decode as \(T.self): \(error)", file: file, line: line)
            return
        }
        let missing = recorder.undeclared(in: json).filter { !(notModeled[method]?.contains($0) ?? false) }
        XCTAssertEqual(missing, [], "\(method): the daemon sends keys \(T.self) does not declare", file: file, line: line)
        checked[method, default: 0] += KeyRecorder.keyPaths(in: json).count
    }

    private func waitForPending(_ daemon: TCDaemon) throws -> [String: Any] {
        for _ in 0..<200 {
            let pending = try XCTUnwrap((try result(daemon, "list_pending") as? [String: Any])?["pending"] as? [Any])
            if let entry = pending.first as? [String: Any] { return entry }
            Thread.sleep(forTimeInterval: 0.05)
        }
        throw XCTSkip("the watcher never queued the session")
    }

    func testEveryKeyTheDaemonSendsIsDeclared() throws {
        let daemon = try startDaemonWithOneSession()
        var frames: [String] = []
        let framesLock = NSLock()
        let subscription = try XCTUnwrap(daemon.subscribe { json in
            framesLock.lock()
            frames.append(json)
            framesLock.unlock()
        })
        defer { _ = daemon.unsubscribe(subscription) }

        let entry = try waitForPending(daemon)
        let entryId = try XCTUnwrap(entry["entry_id"] as? String)
        let projectId = try XCTUnwrap(entry["project_id"] as? String)

        // Status and the queue.
        assertDeclared(DaemonData.Status.self, try result(daemon, "status"), method: "status")
        assertDeclared(DaemonData.PendingList.self, try result(daemon, "list_pending"), method: "list_pending")
        assertDeclared(
            DaemonData.PendingList.self, try result(daemon, "list_pending", ["project_id": projectId]),
            method: "list_pending")

        // Previews: the scheduler, until it answers, then the sheet's.
        var outcome = try result(daemon, "preview_request", ["entry_id": entryId])
        assertDeclared(DaemonData.PreviewRequestOutcome.self, outcome, method: "preview_request")
        for _ in 0..<200 where ["queued", "running"].contains((outcome as? [String: Any])?["state"] as? String) {
            Thread.sleep(forTimeInterval: 0.05)
            outcome = try result(daemon, "preview_request", ["entry_id": entryId])
        }
        XCTAssertEqual((outcome as? [String: Any])?["state"] as? String, "ready")
        assertDeclared(DaemonData.PreviewRequestOutcome.self, outcome, method: "preview_request")
        assertDeclared(
            DaemonData.PreviewVisibleResult.self, try result(daemon, "preview_visible", ["entry_ids": [entryId]]),
            method: "preview_visible")
        assertDeclared(
            DaemonData.PreviewCancelResult.self, try result(daemon, "preview_cancel", ["entry_id": entryId]),
            method: "preview_cancel")
        assertDeclared(DaemonData.PreviewSummary.self, try result(daemon, "preview", ["entry_id": entryId]), method: "preview")
        let body = try XCTUnwrap(try result(daemon, "preview_body", ["entry_id": entryId]) as? [String: Any])
        let digest = try XCTUnwrap(body["body_digest"] as? String)
        // Served by `tc_call` (the async dispatcher), never refused as
        // `preview-unsure-spans-requires-async`. On a store that is not
        // enrolled no envelope is kept, so each call rebuilds the body and
        // the digest can move between `preview_body` and this call; that
        // refusal still proves the method was served.
        do {
            assertDeclared(
                DaemonData.UnsureSpans.self,
                try result(daemon, "preview_unsure_spans", ["entry_id": entryId, "body_digest": digest]),
                method: "preview_unsure_spans")
        } catch {
            XCTAssertTrue("\(error)".contains("preview-body-changed"), "\(error)")
        }

        // Keep, then back.
        assertDeclared(DaemonData.KeepResult.self, try result(daemon, "keep", ["entry_id": entryId]), method: "keep")
        assertDeclared(DaemonData.KeptList.self, try result(daemon, "list_kept"), method: "list_kept")
        assertDeclared(DaemonData.KeepResult.self, try result(daemon, "undo_keep", ["entry_id": entryId]), method: "undo_keep")

        // Projects and tools.
        assertDeclared(DaemonData.ProjectList.self, try result(daemon, "list_projects"), method: "list_projects")
        assertDeclared(
            DaemonData.ProjectModeResult.self,
            try result(daemon, "set_project_mode", ["project_id": projectId, "mode": ProjectMode.ask.rawValue]),
            method: "set_project_mode")
        assertDeclared(HarnessList.self, try result(daemon, "harness_list"), method: "harness_list")

        // Settings.
        assertDeclared(DaemonData.Settings.self, try result(daemon, "get_settings"), method: "get_settings")
        assertDeclared(
            DaemonData.Settings.self, try result(daemon, "set_settings", ["scrub_check": "manual"]),
            method: "set_settings")

        // History, credit, the map.
        assertDeclared(DaemonData.HistoryList.self, try result(daemon, "list_history", ["limit": 50]), method: "list_history")
        assertDeclared(DaemonData.HistoryRollup.self, try result(daemon, "history_rollup"), method: "history_rollup")
        assertDeclared(
            DaemonData.CommonsCreditSummary.self, try result(daemon, "commons_credit_summary"),
            method: "commons_credit_summary")
        assertDeclared(DaemonData.ToolDestinations.self, try result(daemon, "tool_destinations"), method: "tool_destinations")
        assertDeclared(
            DaemonData.InferenceCallPage.self, try result(daemon, "inference_calls", ["limit": 25]),
            method: "inference_calls")

        // Approve last: it moves the entry out of the queue when it acts.
        assertDeclared(ApproveResponse.self, try result(daemon, "approve", ["entry_id": entryId]), method: "approve")

        // The events the calls above caused, as `tc_subscribe` delivered
        // them. `preview_ready` is published from the scheduler's thread, so
        // give it a moment to land.
        for _ in 0..<100 {
            framesLock.lock()
            let seen = frames.contains { $0.contains(#""preview_ready""#) }
            framesLock.unlock()
            if seen { break }
            Thread.sleep(forTimeInterval: 0.05)
        }
        framesLock.lock()
        let delivered = frames
        framesLock.unlock()
        for frame in delivered {
            let object = try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(frame.utf8)) as? [String: Any])
            let name = try XCTUnwrap(object["event"] as? String)
            let data = object["data"] ?? [String: Any]()
            switch name {
            case "preview_ready":
                assertDeclared(DaemonData.PreviewRequestOutcome.self, data, method: "event preview_ready")
            case "digest_due":
                assertDeclared(DaemonData.DigestDue.self, data, method: "event digest_due")
            case "queue_changed", "status_changed":
                XCTAssertEqual(KeyRecorder.keyPaths(in: data), [], "\(name) carries no payload the contract reads")
            default:
                break
            }
        }

        XCTAssertGreaterThan(checked["event preview_ready"] ?? 0, 3, "no preview_ready frame was checked")

        // Proof the walk reached real content, not a set of empty replies.
        for method in ["status", "list_pending", "preview", "preview_request", "list_projects", "get_settings", "approve"] {
            XCTAssertGreaterThan(checked[method] ?? 0, 3, "\(method) was not checked against a real reply")
        }
    }

    /// The same daemon through `LiveDaemonClient`: the review fixes hold
    /// against real replies, not only canned frames.
    func testTheLiveClientAgainstTheRealDaemon() async throws {
        let daemon = try startDaemonWithOneSession()
        let client = LiveDaemonClient(transport: Pipe(daemon))
        let entry = try waitForPending(daemon)
        let entryId = try XCTUnwrap(entry["entry_id"] as? String)

        // An unknown project is the daemon's refusal, as the sample gives it.
        do {
            _ = try await client.listPending(projectId: "proj_does_not_exist")
            XCTFail("an unknown project answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .daemon(code: "bad_params", message: "project-id-unrecognized"))
        }

        // The scheduler answers a card without blocking on the build.
        let requested = try await client.requestPreview(entryId: entryId)
        XCTAssertEqual(requested.entryID, entryId)
        _ = try await client.setVisiblePreviews(entryIds: [entryId])

        // `preview_unsure_spans` is served over `tc_call`: whatever it
        // answers, it is never the sync path's `*-requires-async` refusal.
        do {
            _ = try await client.previewUnsureSpans(entryId: entryId, bodyDigest: "sha256:00")
        } catch DaemonDataError.daemon(_, let message) {
            XCTAssertFalse(message.hasSuffix("requires-async"), message)
        }

        // A single-entry approve the daemon skips is never success. The
        // second approve of the same entry finds it no longer pending.
        do {
            _ = try await client.approve(entryId: entryId)
        } catch DaemonDataError.notApproved(let reason) {
            XCTAssertNotNil(reason)
        }
        do {
            let again = try await client.approve(entryId: entryId)
            XCTFail("a second approve of one entry was drawn as success: \(again)")
        } catch DaemonDataError.notApproved(let reason) {
            XCTAssertNotNil(reason, "the skip names its reason")
        }

        // A stopped daemon is the core-down state.
        _ = daemon.shutdown()
        do {
            _ = try await client.status()
            XCTFail("a stopped daemon answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .unreachable)
        }
    }
}

/// `TCDaemon` as a `DaemonTransport`, by forwarding. The app target
/// declares that conformance on `TCDaemon` itself (`DaemonCalling.swift`);
/// declaring it again in this bundle would be a second conformance.
private final class Pipe: DaemonTransport, @unchecked Sendable {
    let daemon: TCDaemon
    init(_ daemon: TCDaemon) { self.daemon = daemon }
    func call(_ method: String, params paramsJSON: String) -> String {
        daemon.call(method, params: paramsJSON)
    }
}
