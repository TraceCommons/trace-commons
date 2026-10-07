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

    /// The pill's override writes (#1208): `confirm` goes only with Auto
    /// contribute, only as `true`; Ask me and Never carry the mode alone;
    /// clear carries nothing. Each reply decodes as the daemon sends it.
    func testTheContributionOverrideSendsConfirmOnlyForAutoContribute() async throws {
        let transport = ScriptedTransport { method, _ in
            switch method {
            case "set_contribution_override":
                return #"{"id":0,"result":{"changed":true,"contribution_override":{"mode":"ignore","since":"2026-10-02T09:00:00Z"},"returned":2}}"#
            case "clear_contribution_override":
                return #"{"id":0,"result":{"cleared":true,"returned":1}}"#
            default: return nil
            }
        }
        let client = LiveDaemonClient(transport: transport)
        let set = try await client.setContributionOverride(mode: .ignore, confirm: false)
        XCTAssertEqual(set.changed, true)
        XCTAssertEqual(set.returned, 2)
        XCTAssertEqual(set.contributionOverride?.mode, "ignore")
        XCTAssertNotNil(set.contributionOverride?.since)
        _ = try await client.setContributionOverride(mode: .ask, confirm: true)
        _ = try await client.setContributionOverride(mode: .autoUpload, confirm: true)
        _ = try await client.setContributionOverride(mode: .autoUpload, confirm: false)
        let cleared = try await client.clearContributionOverride()
        XCTAssertEqual(cleared, DaemonData.ContributionOverrideClearResult(cleared: true, returned: 1))
        XCTAssertEqual(transport.calls.map(\.method), [
            "set_contribution_override", "set_contribution_override", "set_contribution_override",
            "set_contribution_override", "clear_contribution_override",
        ])
        XCTAssertEqual(transport.calls.map(\.params), [
            #"{"mode":"ignore"}"#,
            #"{"mode":"notify_only"}"#,
            #"{"confirm":true,"mode":"auto_upload"}"#,
            // Unconfirmed: nothing claims a confirmation, and the daemon
            // refuses it as `confirm-required`.
            #"{"mode":"auto_upload"}"#,
            "{}",
        ])
    }

    /// A refusal keeps its label; a stopped daemon is unreachable.
    func testAContributionOverrideRefusalKeepsItsLabel() async {
        let refusing = ScriptedTransport { _, _ in
            #"{"id":0,"error":{"code":"unavailable","message":"arming-terms-unavailable"}}"#
        }
        do {
            _ = try await LiveDaemonClient(transport: refusing).setContributionOverride(mode: .autoUpload, confirm: true)
            XCTFail("a refusal answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .daemon(code: "unavailable", message: "arming-terms-unavailable"))
        }
        let stopped = ScriptedTransport { _, _ in #"{"error":{"code":"unavailable","message":"daemon-stopped"}}"# }
        do {
            _ = try await LiveDaemonClient(transport: stopped).clearContributionOverride()
            XCTFail("a stopped daemon answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .unreachable)
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

    /// The Private AI switch (Z1.5) over `tc_call`: the read is
    /// `get_settings`, the write is `set_settings` carrying the switch AND
    /// the offer marker, so a contributor who found the switch is never put
    /// the first-run question again. Nothing is confirmed here; the caller
    /// confirms with the core's rule.
    func testThePrivateAISwitchReadsAndWritesTheSettings() async throws {
        let transport = ScriptedTransport { method, _ in
            switch method {
            case "get_settings":
                return #"{"id":0,"result":{"private_inference":false,"private_inference_offer_seen":false,"private_inference_state":{"state":"off","port":null}}}"#
            case "set_settings":
                return #"{"id":0,"result":{"private_inference":true,"private_inference_offer_seen":true,"private_inference_state":{"state":"running","port":4100}}}"#
            default: return nil
            }
        }
        let client = LiveDaemonClient(transport: transport)
        let read = try await client.privateAI()
        XCTAssertEqual(read, DaemonData.PrivateAISwitch(
            on: false, offerSeen: false, state: DaemonData.PrivateInferenceState(state: "off", port: nil)))
        let written = try await client.setPrivateAI(on: true)
        XCTAssertEqual(written.on, true)
        XCTAssertEqual(written.offerSeen, true)
        XCTAssertEqual(written.state?.state, "running")
        XCTAssertEqual(transport.calls.map(\.method), ["get_settings", "set_settings"])
        XCTAssertEqual(transport.calls.map(\.params), [
            "{}",
            #"{"private_inference":true,"private_inference_offer_seen":true}"#,
        ])
    }

    /// A stopped daemon is unreachable for the switch too; a refusal keeps its label.
    func testAPrivateAIRefusalKeepsItsLabel() async {
        let refusing = ScriptedTransport { _, _ in #"{"id":0,"error":{"code":"bad_params","message":"settings-invalid-value"}}"# }
        do {
            _ = try await LiveDaemonClient(transport: refusing).setPrivateAI(on: true)
            XCTFail("a refusal answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .daemon(code: "bad_params", message: "settings-invalid-value"))
        }
        let stopped = ScriptedTransport { _, _ in #"{"error":{"code":"unavailable","message":"daemon-stopped"}}"# }
        do {
            _ = try await LiveDaemonClient(transport: stopped).privateAI()
            XCTFail("a stopped daemon answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .unreachable)
        }
    }

    /// Only the two PROVISIONAL shapes the screens still read throw
    /// `notAvailableYet`; the daemon's real replies for those methods are
    /// `networkInferenceSummary()` and `networkMissionCatalogue()`, and every
    /// other network method is routed (`DaemonNetworkContractTests`).
    func testProvisionalMethodsSendNothing() async {
        let transport = ScriptedTransport.sample(.normalDay)
        let client = LiveDaemonClient(transport: transport)
        let provisional: [(String, () async throws -> Void)] = [
            ("inference_summary", { _ = try await client.inferenceSummary() }),
            ("mission_catalogue", { _ = try await client.missionCatalogue() }),
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

    // MARK: - previewUnsureSpans goes through tc_call

    func testUnsureSpansGoThroughTcCallAndDecode() async throws {
        let transport = ScriptedTransport { method, _ in
            method == "preview_unsure_spans"
                ? #"{"id":0,"result":{"entry_id":"e1","body_digest":"sha256:ab","envelope_digest":"sha256:cd","span_count":3,"spans":[{"label":"looks-like-email","byte_offset":10,"byte_len":17}],"spans_truncated":true}}"#
                : nil
        }
        let spans = try await LiveDaemonClient(transport: transport)
            .previewUnsureSpans(entryId: "e1", bodyDigest: "sha256:ab")
        XCTAssertEqual(spans.spanCount, 3)
        XCTAssertTrue(spans.spansTruncated)
        XCTAssertEqual(spans.spans.first?.label, "looks-like-email")
        XCTAssertEqual(transport.calls.map(\.method), ["preview_unsure_spans"])
        XCTAssertEqual(transport.calls.first?.params, #"{"body_digest":"sha256:ab","entry_id":"e1"}"#)
    }

    func testUnsureSpansRefusalsKeepTheirLabelAndAStoppedDaemonIsUnreachable() async {
        let changed = ScriptedTransport { _, _ in #"{"error":{"code":"unavailable","message":"preview-body-changed"}}"# }
        let stopped = ScriptedTransport { _, _ in #"{"error":{"code":"unavailable","message":"daemon-stopped"}}"# }
        do {
            _ = try await LiveDaemonClient(transport: changed).previewUnsureSpans(entryId: "e", bodyDigest: "d")
            XCTFail("a changed body answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .daemon(code: "unavailable", message: "preview-body-changed"))
        }
        do {
            _ = try await LiveDaemonClient(transport: stopped).previewUnsureSpans(entryId: "e", bodyDigest: "d")
            XCTFail("a stopped daemon answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .unreachable)
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
            XCTAssertEqual(error as? DaemonDataError, .undecodable(method: "list_projects", codingPath: "projects"))
        }
    }

    func testCallsDoNotRunOnTheCallersThread() async throws {
        let transport = ScriptedTransport.sample(.normalDay)
        _ = try await LiveDaemonClient(transport: transport).status()
        XCTAssertEqual(transport.calls.first?.onQueue, true)
    }

    /// The network methods (C3) wait on a server round trip, so each runs on
    /// the client's work queue like every local call, never holding a Swift
    /// concurrency thread for the trip.
    func testNetworkCallsDoNotRunOnTheCallersThread() async throws {
        let transport = ScriptedTransport.sample(.normalDay)
        let client = LiveDaemonClient(transport: transport)
        _ = try await client.networkInferenceSummary()
        _ = try await client.inferenceCallProof(callId: 414)
        _ = try await client.modelSpend()
        let shown = try await client.networkPrivateAI()
        _ = try await client.setNetworkPrivateAI(on: false, consent: DaemonData.PrivateAIConsent(acknowledging: shown))
        _ = try await client.networkMissionCatalogue(limit: 1, before: nil)
        _ = try await client.lookupInvite(code: "c")
        _ = try await client.passkeyState()
        _ = try await client.accountState()
        _ = try await client.activityMissionsCatalogue()
        XCTAssertEqual(transport.calls.map(\.method), [
            "inference_summary", "inference_call_proof", "model_spend", "private_ai", "set_private_ai",
            "mission_catalogue", "invite_lookup", "passkey_state", "account_session_status",
            "activity_missions_catalogue",
        ])
        XCTAssertEqual(transport.calls.filter { !$0.onQueue }.map(\.method), [])
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
            .digestDue(DaemonData.DigestDue(pending: 2, text: "2 sessions are waiting")),
            .previewReady(PreviewRequestResult(entryID: "e7", state: .ready)),
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

    /// A reply that is not a frame at all is a broken stream, not a status
    /// that is unknown: the stream opens with `.resyncRequired`, never with
    /// a snapshot that draws "unknown" over it.
    func testAStatusReplyThatIsNotAFrameIsAResyncNotAnUnknownStatus() async {
        let sample = ScriptedTransport.sample(.normalDay)
        let transport = ScriptedTransport { method, params in
            method == "status" ? "not a frame" : sample.call(method, params: params)
        }
        var iterator = LiveDaemonClient(transport: transport).events().makeAsyncIterator()
        let first = await iterator.next()
        XCTAssertEqual(first, .resyncRequired)
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
    /// `tc_call` result frame. `dismiss` answers `ok: true`, as the daemon does;
    /// a write answers with what its read returns.
    static func sample(_ set: SampleDaemonClient.SampleSet) -> ScriptedTransport {
        ScriptedTransport { method, _ in
            if method == "dismiss" { return #"{"id":0,"result":{"ok":true}}"# }
            let read = ["set_settings": "get_settings", "set_private_ai": "private_ai"][method] ?? method
            let reply = SampleDaemonData.reply(read, in: set)
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
