import XCTest
@testable import TCShellCore

/// K1 of #1173: `LiveDaemonClient` over a scripted transport. What it sends
/// (method and parameter bytes, which the daemon refuses as a whole when
/// either is wrong), how it reads every reply, and how its event streams
/// open, order and end.
final class LiveDaemonClientTests: XCTestCase {
    // MARK: - Requests

    /// Every live method's IPC name and parameters, as the daemon reads
    /// them (`daemon/ipc.rs`, `docs/contributor-daemon-ipc-v1_1.md`).
    func testEveryLiveMethodSendsItsIPCNameAndParams() async throws {
        let transport = ScriptedTransport.sample(.normalDay)
        let client = LiveDaemonClient(transport: transport)
        let expected: [(String, String, () async throws -> Void)] = [
            ("status", "{}", { _ = try await client.status() }),
            ("list_pending", "{}", { _ = try await client.listPending(projectId: nil) }),
            ("list_pending", #"{"project_id":"proj_1"}"#, { _ = try await client.listPending(projectId: "proj_1") }),
            ("list_kept", "{}", { _ = try await client.listKept() }),
            ("approve", #"{"entry_id":"e1"}"#, { _ = try await client.approve(entryId: "e1") }),
            ("keep", #"{"entry_id":"e1"}"#, { _ = try await client.keep(entryId: "e1") }),
            ("undo_keep", #"{"entry_id":"e1"}"#, { _ = try await client.undoKeep(entryId: "e1") }),
            ("dismiss", #"{"entry_id":"e1"}"#, { try await client.dismiss(entryId: "e1") }),
            ("list_projects", "{}", { _ = try await client.listProjects() }),
            ("set_project_mode", #"{"mode":"auto_upload","project_id":"p"}"#,
             { _ = try await client.setProjectMode(projectId: "p", mode: .autoUpload, includeBacklog: nil) }),
            ("set_project_mode", #"{"include_backlog":true,"mode":"auto_upload","project_id":"p"}"#,
             { _ = try await client.setProjectMode(projectId: "p", mode: .autoUpload, includeBacklog: true) }),
            ("harness_list", "{}", { _ = try await client.harnessList() }),
            ("get_settings", "{}", { _ = try await client.settings() }),
            ("set_settings", #"{"scrub_check":"automatic"}"#, { _ = try await client.setScrubCheck(.automatic) }),
            ("set_settings", #"{"local_notifications":false}"#, { _ = try await client.setLocalNotifications(false) }),
            ("set_settings", #"{"digest_schedule":{"hour":18,"mode":"evening"}}"#,
             { _ = try await client.setDigestSchedule(.evening(hour: 18)) }),
            ("set_settings", #"{"digest_schedule":{"mode":"interval"}}"#,
             { _ = try await client.setDigestSchedule(.interval) }),
            ("list_history", #"{"limit":50}"#, { _ = try await client.listHistory(limit: 50) }),
            ("history_rollup", "{}", { _ = try await client.historyRollup() }),
            ("commons_credit_summary", "{}", { _ = try await client.commonsCreditSummary() }),
            ("tool_destinations", "{}", { _ = try await client.toolDestinations() }),
            ("inference_calls", #"{"limit":25}"#, { _ = try await client.inferenceCalls(limit: 25, cursor: nil) }),
            ("inference_calls", #"{"cursor":"c2","limit":25}"#,
             { _ = try await client.inferenceCalls(limit: 25, cursor: "c2") }),
        ]
        for (method, params, call) in expected {
            let before = transport.calls.count
            do {
                try await call()
            } catch {
                XCTFail("\(method) \(params): \(error)")
            }
            let sent = transport.calls.dropFirst(before)
            XCTAssertEqual(sent.count, 1, method)
            XCTAssertEqual(sent.first?.method, method)
            XCTAssertEqual(sent.first?.params, params, method)
        }
    }

    func testPreviewSendsTheEntryAndDecodesTheSummary() async throws {
        let transport = ScriptedTransport { method, _ in
            method == "preview"
                ? #"{"id":0,"result":{"title":"Fix the login race","unsure_spans":2}}"#
                : nil
        }
        let summary = try await LiveDaemonClient(transport: transport).preview(entryId: "e9")
        XCTAssertEqual(summary.title, "Fix the login race")
        XCTAssertEqual(transport.calls.map(\.params), [#"{"entry_id":"e9"}"#])
    }

    func testProvisionalMethodsSendNothing() async {
        let transport = ScriptedTransport.sample(.normalDay)
        let client = LiveDaemonClient(transport: transport)
        let provisional: [(String, () async throws -> Void)] = [
            ("inference_summary", { _ = try await client.inferenceSummary() }),
            ("inference_call_proof", { _ = try await client.inferenceCallProof(callId: 1) }),
            ("model_spend", { _ = try await client.modelSpend() }),
            ("private_ai", { _ = try await client.privateAI() }),
            ("set_private_ai", { _ = try await client.setPrivateAI(on: true) }),
            ("mission_catalogue", { _ = try await client.missionCatalogue() }),
            ("invite_lookup", { _ = try await client.lookupInvite(code: "c") }),
            ("passkey_state", { _ = try await client.passkeyState() }),
            ("account_session_status", { _ = try await client.accountState() }),
        ]
        for (method, call) in provisional {
            do {
                try await call()
                XCTFail("\(method) answered")
            } catch {
                XCTAssertEqual(error as? DaemonDataError, .notAvailableYet(method: method))
            }
        }
        XCTAssertTrue(transport.calls.isEmpty)
    }

    // MARK: - previewUnsureSpans goes through the export, not tc_call

    func testUnsureSpansUseTheExportAndDecode() async throws {
        let transport = ScriptedTransport.sample(.normalDay)
        let index = ScriptedIndex(
            frame: #"{"result":{"entry_id":"e1","body_digest":"sha256:ab","envelope_digest":"sha256:cd","span_count":3,"spans":[{"label":"looks-like-email","byte_offset":10,"byte_len":17}],"spans_truncated":true}}"#)
        let spans = try await LiveDaemonClient(transport: transport, previewIndex: index)
            .previewUnsureSpans(entryId: "e1", bodyDigest: "sha256:ab")
        XCTAssertEqual(spans.spanCount, 3)
        XCTAssertTrue(spans.spansTruncated)
        XCTAssertEqual(spans.spans.first?.label, "looks-like-email")
        XCTAssertEqual(index.asked.first?.0, "e1")
        XCTAssertEqual(index.asked.first?.1, "sha256:ab")
        XCTAssertTrue(transport.calls.isEmpty, "preview_unsure_spans must not go through tc_call")
    }

    func testUnsureSpansRefusalsKeepTheirLabelAndAStoppedDaemonIsUnreachable() async {
        let changed = ScriptedIndex(frame: #"{"error":{"code":"unavailable","message":"preview-body-changed"}}"#)
        let stopped = ScriptedIndex(frame: #"{"error":{"code":"unavailable","message":"daemon-stopped"}}"#)
        let transport = ScriptedTransport.sample(.normalDay)
        do {
            _ = try await LiveDaemonClient(transport: transport, previewIndex: changed)
                .previewUnsureSpans(entryId: "e", bodyDigest: "d")
            XCTFail("a changed body answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .daemon(code: "unavailable", message: "preview-body-changed"))
        }
        do {
            _ = try await LiveDaemonClient(transport: transport, previewIndex: stopped)
                .previewUnsureSpans(entryId: "e", bodyDigest: "d")
            XCTFail("a stopped daemon answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .unreachable)
        }
    }

    func testUnsureSpansWithoutTheExportAreNotAvailableYet() async {
        do {
            _ = try await LiveDaemonClient(transport: ScriptedTransport.sample(.normalDay))
                .previewUnsureSpans(entryId: "e", bodyDigest: "d")
            XCTFail("answered with no export")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .notAvailableYet(method: "preview_unsure_spans"))
        }
    }

    // MARK: - Replies

    /// Every live method decodes the daemon-shaped reply the sample sets
    /// hold, through the live client's own framing.
    func testEveryLiveMethodDecodesEverySampleSet() async {
        for set in SampleDaemonClient.SampleSet.allCases where set != .coreDown {
            let client = LiveDaemonClient(transport: ScriptedTransport.sample(set))
            do {
                _ = try await client.status()
                _ = try await client.listPending(projectId: nil)
                _ = try await client.listKept()
                _ = try await client.approve(entryId: "e")
                _ = try await client.keep(entryId: "e")
                _ = try await client.undoKeep(entryId: "e")
                try await client.dismiss(entryId: "e")
                _ = try await client.listProjects()
                _ = try await client.setProjectMode(projectId: "p", mode: .ask, includeBacklog: nil)
                _ = try await client.harnessList()
                _ = try await client.settings()
                _ = try await client.setScrubCheck(.manual)
                _ = try await client.listHistory(limit: 10)
                _ = try await client.historyRollup()
                _ = try await client.commonsCreditSummary()
                _ = try await client.toolDestinations()
                _ = try await client.inferenceCalls(limit: 10, cursor: nil)
            } catch {
                XCTFail("\(set): \(error)")
            }
        }
    }

    func testAbsentDecisionsOwedStaysNilBesideAQueue() async throws {
        let transport = ScriptedTransport { method, _ in
            switch method {
            case "status": return #"{"id":0,"result":{"queue_depth":4,"paused":false}}"#
            case "list_pending": return #"{"id":0,"result":{"pending":[]}}"#
            default: return nil
            }
        }
        let status = try await LiveDaemonClient(transport: transport).status()
        XCTAssertNil(status.decisionsOwed, "never queue_depth, never a list length")
        XCTAssertEqual(status.queueDepth, 4)

        let null = ScriptedTransport { _, _ in #"{"id":0,"result":{"queue_depth":4,"decisions_owed":null}}"# }
        let nullStatus = try await LiveDaemonClient(transport: null).status()
        XCTAssertNil(nullStatus.decisionsOwed)
    }

    func testEveryUnreachableLabelIsUnreachable() async {
        for label in ["daemon-stopped", "daemon-disconnected", "attached-transport-failed", "handle-freed",
                      "null-handle", "invalid-handle-pointer"]
        {
            let transport = ScriptedTransport { _, _ in #"{"error":{"code":"unavailable","message":"\#(label)"}}"# }
            do {
                _ = try await LiveDaemonClient(transport: transport).listProjects()
                XCTFail("\(label) answered")
            } catch {
                XCTAssertEqual(error as? DaemonDataError, .unreachable, label)
            }
        }
    }

    func testARefusalIsNotUnreachable() async {
        let transport = ScriptedTransport { _, _ in
            #"{"id":0,"error":{"code":"bad_params","message":"project-id-unrecognized"}}"#
        }
        do {
            _ = try await LiveDaemonClient(transport: transport).listPending(projectId: "nope")
            XCTFail("a refusal answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .daemon(code: "bad_params", message: "project-id-unrecognized"))
        }
    }

    func testAnUnreadableReplyIsUndecodable() async {
        let transport = ScriptedTransport { _, _ in #"{"id":0,"result":{"projects":"not a list"}}"# }
        do {
            _ = try await LiveDaemonClient(transport: transport).listProjects()
            XCTFail("an unreadable reply answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .undecodable(method: "list_projects"))
        }
    }

    func testCallsDoNotRunOnTheCallersThread() async throws {
        let transport = ScriptedTransport.sample(.normalDay)
        _ = try await LiveDaemonClient(transport: transport).status()
        XCTAssertEqual(transport.calls.first?.onQueue, true)
    }

    // MARK: - Events

    func testAStreamOpensWithASnapshotBuiltFromStatusAndTheQueue() async throws {
        let client = LiveDaemonClient(transport: ScriptedTransport.sample(.normalDay))
        var iterator = client.events().makeAsyncIterator()
        guard case .snapshot(let pending, let status)? = await iterator.next() else {
            return XCTFail("first event is not a snapshot")
        }
        XCTAssertEqual(pending.count, 3)
        XCTAssertEqual(status?.decisionsOwed, 3)
    }

    func testFramesDeliveredWhileTheSnapshotIsBuiltFollowItInOrder() async throws {
        let gate = DispatchSemaphore(value: 0)
        let sample = ScriptedTransport.sample(.normalDay)
        let transport = ScriptedTransport { method, params in
            if method == "status" { gate.wait() }
            return sample.call(method, params: params)
        }
        let client = LiveDaemonClient(transport: transport)
        let stream = client.events()
        let frames = [
            #"{"event":"status_changed","data":{}}"#,
            #"{"event":"queue_changed","data":{}}"#,
            #"{"event":"digest_due","data":{"pending":2,"text":"2 sessions are waiting"}}"#,
            #"{"event":"preview_ready","data":{"entry_id":"e7","state":"ready"}}"#,
            #"{"event":"resync_required","data":{}}"#,
            #"{"event":"lagged","data":{"skipped":12}}"#,
        ]
        for frame in frames { client.deliver(eventJSON: frame) }
        gate.signal()
        var received: [DaemonDataEvent] = []
        for await event in stream {
            received.append(event)
            if received.count == frames.count + 1 { break }
        }
        guard case .snapshot? = received.first else { return XCTFail("snapshot not first: \(received)") }
        XCTAssertEqual(Array(received.dropFirst()), [
            .statusChanged,
            .queueChanged,
            .digestDue(pending: 2, text: "2 sessions are waiting"),
            .previewReady(entryId: "e7"),
            .resyncRequired,
            .resyncRequired,
        ])
    }

    func testEveryOpenStreamGetsEachFrame() async {
        let client = LiveDaemonClient(transport: ScriptedTransport.sample(.normalDay))
        var first = client.events().makeAsyncIterator()
        var second = client.events().makeAsyncIterator()
        _ = await first.next()
        _ = await second.next()
        client.deliver(eventJSON: #"{"event":"queue_changed","data":{}}"#)
        let a = await first.next()
        let b = await second.next()
        XCTAssertEqual(a, .queueChanged)
        XCTAssertEqual(b, .queueChanged)
    }

    func testAnUnreachableDaemonFinishesTheStreamAtOnce() async {
        let transport = ScriptedTransport { _, _ in #"{"error":{"code":"unavailable","message":"daemon-stopped"}}"# }
        var count = 0
        for await _ in LiveDaemonClient(transport: transport).events() { count += 1 }
        XCTAssertEqual(count, 0)
    }

    func testAnUnreadableStatusStillOpensWithTheQueue() async {
        let sample = ScriptedTransport.sample(.normalDay)
        let transport = ScriptedTransport { method, params in
            method == "status" ? #"{"result":{"decisions_owed":"three"}}"# : sample.call(method, params: params)
        }
        var iterator = LiveDaemonClient(transport: transport).events().makeAsyncIterator()
        guard case .snapshot(let pending, let status)? = await iterator.next() else {
            return XCTFail("first event is not a snapshot")
        }
        XCTAssertEqual(pending.count, 3)
        XCTAssertNil(status)
    }

    func testFinishEventsEndsOpenStreamsAndRefusesNewOnes() async {
        let client = LiveDaemonClient(transport: ScriptedTransport.sample(.normalDay))
        let stream = client.events()
        var iterator = stream.makeAsyncIterator()
        _ = await iterator.next()
        client.finishEvents()
        let afterFinish = await iterator.next()
        XCTAssertNil(afterFinish)
        var count = 0
        for await _ in client.events() { count += 1 }
        XCTAssertEqual(count, 0)
        client.deliver(eventJSON: #"{"event":"queue_changed","data":{}}"#)
    }
}

/// A transport that answers each method from a script and records what it
/// was sent, and from which thread.
private final class ScriptedTransport: DaemonTransport, @unchecked Sendable {
    struct Call {
        let method: String
        let params: String
        let onQueue: Bool
    }

    private let answer: @Sendable (String, String) -> String?
    private let lock = NSLock()
    private var recorded: [Call] = []

    var calls: [Call] {
        lock.lock()
        defer { lock.unlock() }
        return recorded
    }

    init(_ answer: @escaping @Sendable (String, String) -> String?) {
        self.answer = answer
    }

    /// Answers from a sample set's daemon-shaped replies, wrapped in a
    /// `tc_call` result frame. `dismiss` answers `ok: true`, as the daemon does.
    static func sample(_ set: SampleDaemonClient.SampleSet) -> ScriptedTransport {
        ScriptedTransport { method, _ in
            if method == "dismiss" { return #"{"id":0,"result":{"ok":true}}"# }
            let reply = method == "set_settings" ? SampleDaemonData.reply("get_settings", in: set)
                : SampleDaemonData.reply(method, in: set)
            return reply.map { #"{"id":0,"result":\#($0)}"# }
        }
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        let onQueue = String(cString: __dispatch_queue_get_label(nil)) == "trace-commons.live-daemon-client"
        lock.lock()
        recorded.append(Call(method: method, params: paramsJSON, onQueue: onQueue))
        lock.unlock()
        return answer(method, paramsJSON)
            ?? #"{"id":0,"error":{"code":"bad_params","message":"unknown-method"}}"#
    }
}

private final class ScriptedIndex: DaemonPreviewIndexTransport, @unchecked Sendable {
    let frame: String
    private let lock = NSLock()
    private var recorded: [(String, String)] = []

    var asked: [(String, String)] {
        lock.lock()
        defer { lock.unlock() }
        return recorded
    }

    init(frame: String) {
        self.frame = frame
    }

    func previewUnsureSpans(entryID: String, bodyDigest: String) -> String {
        lock.lock()
        recorded.append((entryID, bodyDigest))
        lock.unlock()
        return frame
    }
}
