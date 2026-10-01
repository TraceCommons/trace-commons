import Foundation

/// The one call `LiveDaemonClient` needs: a method name and its parameters
/// as JSON in, one response frame out.
///
/// The app target's `DaemonCalling` refines this (it adds the preview and
/// search calls), and `TCDaemon` already satisfies it, so the live client
/// runs over exactly the `tc_call` path `DaemonClient` uses. Declared here,
/// in a target that does not link the dylib, so the live client's framing
/// and decoding are unit tested against canned frames.
public protocol DaemonTransport: AnyObject {
    func call(_ method: String, params paramsJSON: String) -> String
}

/// The C ABI's `tc_preview_unsure_spans_json`, framed like `tc_call`.
///
/// Kept apart from `DaemonTransport` because it is not a `tc_call` method:
/// the unsure-span index is an overlay on the in-process redacted body, so
/// it has its own export. `TCDaemon.previewUnsureSpans` wraps that export
/// and answers in `tc_call`'s frame (`{"result": ...}` or
/// `{"error": {code, message}}`), so `DaemonFrame` reads both the same way
/// and a stopped daemon is `unreachable` on either path.
public protocol DaemonPreviewIndexTransport: AnyObject {
    func previewUnsureSpans(entryID: String, bodyDigest: String) -> String
}

/// The real `DaemonDataClient`, over `tc_call` (K1 of #1173).
///
/// Wired for every method that exists on main. The PROVISIONAL network
/// methods (Zaki's C3) throw `notAvailableYet` until their IPC lands, and
/// send nothing. `previewUnsureSpans` goes through
/// `tc_preview_unsure_spans_json` (`previewIndex`), not `tc_call`: the
/// synchronous dispatcher refuses `preview_unsure_spans`
/// (`preview-unsure-spans-requires-async`) and the export is the ABI's
/// own route to the same `ipc::open_preview_unsure_spans`.
///
/// Every call runs on `workQueue`, never on the caller's thread. The C ABI
/// blocks for as long as the daemon takes (a preview index is a redaction
/// pass), and blocking a Swift concurrency thread starves the cooperative
/// pool -- the reason `TCDaemon` gives for not being an actor.
///
/// Events: `deliver(eventJSON:)` is fed by the app's one `tc_subscribe`
/// callback (`AppModel.subscribe`), so the data client and the existing
/// `AppModel` handlers see the same frames from the same subscription.
/// `tc_subscribe` sends no `snapshot` on the in-process path, so each
/// `events()` stream opens with one built here from `status` and
/// `list_pending`, as the contract promises; frames that arrive while it is
/// being built are held and follow it in order.
///
/// `@unchecked Sendable`: the state is the transports, which are safe to
/// call from any thread, and the subscriber table, guarded by `lock`.
public final class LiveDaemonClient: DaemonDataClient, @unchecked Sendable {
    private let transport: any DaemonTransport
    private let previewIndex: (any DaemonPreviewIndexTransport)?
    private let workQueue = DispatchQueue(
        label: "trace-commons.live-daemon-client", qos: .userInitiated, attributes: .concurrent)

    private struct Subscriber {
        let continuation: AsyncStream<DaemonDataEvent>.Continuation
        /// Frames delivered before this stream's opening snapshot was sent.
        /// `nil` once the snapshot is out and frames are yielded directly.
        var held: [DaemonDataEvent]? = []
    }

    private let lock = NSLock()
    private var subscribers: [UUID: Subscriber] = [:]
    private var eventsFinished = false

    /// `previewIndex` is the unsure-span export; `nil` (a transport with no
    /// in-process body, as in unit tests) makes `previewUnsureSpans` throw
    /// `notAvailableYet`, never a guess.
    public init(transport: any DaemonTransport, previewIndex: (any DaemonPreviewIndexTransport)? = nil) {
        self.transport = transport
        self.previewIndex = previewIndex
    }

    /// No client, no more frames: a stream outliving its client ends
    /// rather than waiting forever.
    deinit {
        finishEvents()
    }

    // MARK: Status and the queue

    public func status() async throws -> DaemonData.Status {
        try await call("status", as: DaemonData.Status.self)
    }

    public func listPending(projectId: String?) async throws -> [DaemonData.QueueEntry] {
        var params: [String: Any] = [:]
        if let projectId { params["project_id"] = projectId }
        return try await call("list_pending", params: params, as: DaemonData.PendingList.self).pending
    }

    public func listKept() async throws -> [DaemonData.QueueEntry] {
        try await call("list_kept", as: DaemonData.KeptList.self).kept
    }

    public func preview(entryId: String) async throws -> DaemonData.PreviewSummary {
        try await call("preview", params: ["entry_id": entryId], as: DaemonData.PreviewSummary.self)
    }

    public func previewUnsureSpans(entryId: String, bodyDigest: String) async throws -> DaemonData.UnsureSpans {
        let method = "preview_unsure_spans"
        guard previewIndex != nil else { throw DaemonDataError.notAvailableYet(method: method) }
        return try await perform { [self] in
            let frame = self.previewIndex?.previewUnsureSpans(entryID: entryId, bodyDigest: bodyDigest) ?? ""
            return try Self.decode(frame, method: method, as: DaemonData.UnsureSpans.self)
        }
    }

    public func approve(entryId: String) async throws -> DaemonData.ApproveResult {
        try await call("approve", params: ["entry_id": entryId], as: DaemonData.ApproveResult.self)
    }

    public func keep(entryId: String) async throws -> DaemonData.KeepResult {
        try await call("keep", params: ["entry_id": entryId], as: DaemonData.KeepResult.self)
    }

    public func undoKeep(entryId: String) async throws -> DaemonData.KeepResult {
        try await call("undo_keep", params: ["entry_id": entryId], as: DaemonData.KeepResult.self)
    }

    public func dismiss(entryId: String) async throws {
        let paramsJSON = try Self.encode(["entry_id": entryId])
        _ = try await perform { [self] in
            try DaemonFrame.result(of: self.transport.call("dismiss", params: paramsJSON), method: "dismiss")
        }
    }

    // MARK: Projects and tools

    public func listProjects() async throws -> DaemonData.ProjectList {
        try await call("list_projects", as: DaemonData.ProjectList.self)
    }

    public func setProjectMode(projectId: String, mode: ProjectMode, includeBacklog: Bool?) async throws
        -> DaemonData.ProjectModeResult
    {
        var params: [String: Any] = ["project_id": projectId, "mode": mode.rawValue]
        if let includeBacklog { params["include_backlog"] = includeBacklog }
        return try await call("set_project_mode", params: params, as: DaemonData.ProjectModeResult.self)
    }

    public func harnessList() async throws -> HarnessList {
        try await call("harness_list", as: HarnessList.self)
    }

    // MARK: Settings

    public func settings() async throws -> DaemonData.Settings {
        try await call("get_settings", as: DaemonData.Settings.self)
    }

    public func setScrubCheck(_ mode: DaemonData.ScrubCheckMode) async throws -> DaemonData.Settings {
        try await call("set_settings", params: ["scrub_check": mode.rawValue], as: DaemonData.Settings.self)
    }

    public func setLocalNotifications(_ on: Bool) async throws -> DaemonData.Settings {
        try await call("set_settings", params: ["local_notifications": on], as: DaemonData.Settings.self)
    }

    public func setDigestSchedule(_ schedule: DaemonData.DigestSchedule) async throws -> DaemonData.Settings {
        var value: [String: Any] = ["mode": schedule.mode]
        if let hour = schedule.hour { value["hour"] = hour }
        return try await call("set_settings", params: ["digest_schedule": value], as: DaemonData.Settings.self)
    }

    // MARK: History and credit

    public func listHistory(limit: Int) async throws -> [DaemonData.HistoryRow] {
        try await call("list_history", params: ["limit": limit], as: DaemonData.HistoryList.self).history
    }

    public func historyRollup() async throws -> DaemonData.HistoryRollup {
        try await call("history_rollup", as: DaemonData.HistoryRollup.self)
    }

    public func commonsCreditSummary() async throws -> DaemonData.CommonsCreditSummary {
        try await call("commons_credit_summary", as: DaemonData.CommonsCreditSummary.self)
    }

    // MARK: The map and the Inference tab

    public func toolDestinations() async throws -> DaemonData.ToolDestinations {
        try await call("tool_destinations", as: DaemonData.ToolDestinations.self)
    }

    public func inferenceCalls(limit: Int, cursor: String?) async throws -> DaemonData.InferenceCallPage {
        var params: [String: Any] = ["limit": limit]
        if let cursor { params["cursor"] = cursor }
        return try await call("inference_calls", params: params, as: DaemonData.InferenceCallPage.self)
    }

    // MARK: PROVISIONAL network methods (Zaki's C3): not on main yet

    public func inferenceSummary() async throws -> DaemonData.InferenceSummary {
        throw DaemonDataError.notAvailableYet(method: "inference_summary")
    }

    public func inferenceCallProof(callId: Int64) async throws -> DaemonData.InferenceProofDetail {
        throw DaemonDataError.notAvailableYet(method: "inference_call_proof")
    }

    public func modelSpend() async throws -> DaemonData.ModelSpend {
        throw DaemonDataError.notAvailableYet(method: "model_spend")
    }

    public func privateAI() async throws -> DaemonData.PrivateAISwitch {
        throw DaemonDataError.notAvailableYet(method: "private_ai")
    }

    public func setPrivateAI(on: Bool) async throws -> DaemonData.PrivateAISwitch {
        throw DaemonDataError.notAvailableYet(method: "set_private_ai")
    }

    public func missionCatalogue() async throws -> DaemonData.MissionCatalogue {
        throw DaemonDataError.notAvailableYet(method: "mission_catalogue")
    }

    public func lookupInvite(code: String) async throws -> DaemonData.InviteLookup {
        throw DaemonDataError.notAvailableYet(method: "invite_lookup")
    }

    public func passkeyState() async throws -> DaemonData.PasskeyState {
        throw DaemonDataError.notAvailableYet(method: "passkey_state")
    }

    public func accountState() async throws -> DaemonData.AccountState {
        throw DaemonDataError.notAvailableYet(method: "account_session_status")
    }

    // MARK: Live updates

    /// A fresh stream that opens with a `snapshot` (built from `status` and
    /// `list_pending`, since `tc_subscribe` sends none in process) and then
    /// carries every frame `deliver(eventJSON:)` is given.
    ///
    /// A daemon that is unreachable when the snapshot is built finishes the
    /// stream at once, exactly as `SampleDaemonClient(.coreDown)` does. A
    /// snapshot that cannot be built for any other reason opens with
    /// `.resyncRequired` instead, which tells the screen to fetch for itself.
    public func events() -> AsyncStream<DaemonDataEvent> {
        let id = UUID()
        let (stream, continuation) = AsyncStream.makeStream(of: DaemonDataEvent.self)
        lock.lock()
        if eventsFinished {
            lock.unlock()
            continuation.finish()
            return stream
        }
        subscribers[id] = Subscriber(continuation: continuation)
        lock.unlock()
        continuation.onTermination = { [weak self] _ in
            guard let self else { return }
            self.lock.lock()
            self.subscribers[id] = nil
            self.lock.unlock()
        }
        // Strong: the snapshot is built even if the caller let go of the
        // client right after asking for the stream. Once it is sent the
        // stream holds no reference, and `deinit` finishes it.
        workQueue.async { [self] in
            self.open(subscriber: id)
        }
        return stream
    }

    /// Hands one subscription frame to every open `events()` stream. Safe to
    /// call from the Rust thread a `tc_subscribe` callback runs on: it only
    /// parses and yields, and never calls back into the daemon.
    public func deliver(eventJSON: String) {
        let event = DaemonDataEventParser.parse(eventJSON)
        lock.lock()
        defer { lock.unlock() }
        for (id, subscriber) in subscribers {
            if subscriber.held != nil {
                subscribers[id]?.held?.append(event)
            } else {
                subscriber.continuation.yield(event)
            }
        }
    }

    /// Ends every open stream and refuses new ones: the daemon is gone.
    /// Called by the app at teardown, so a screen's `for await` loop ends
    /// rather than waiting on a subscription that no longer exists.
    public func finishEvents() {
        lock.lock()
        eventsFinished = true
        let open = subscribers.values.map(\.continuation)
        subscribers.removeAll()
        lock.unlock()
        for continuation in open { continuation.finish() }
    }

    /// Builds and sends one stream's opening snapshot, then releases the
    /// frames held while it was built. Runs on `workQueue`.
    private func open(subscriber id: UUID) {
        let opening: DaemonDataEvent?
        do {
            let status: DaemonData.Status?
            do {
                status = try Self.decode(transport.call("status", params: "{}"), method: "status",
                                         as: DaemonData.Status.self)
            } catch DaemonDataError.undecodable {
                // A status this build cannot read is unknown, not a reason
                // to withhold the queue.
                status = nil
            }
            let pending = try Self.decode(transport.call("list_pending", params: "{}"), method: "list_pending",
                                          as: DaemonData.PendingList.self).pending
            opening = .snapshot(pending: pending, status: status)
        } catch DaemonDataError.unreachable {
            opening = nil
        } catch {
            opening = .resyncRequired
        }
        lock.lock()
        guard let subscriber = subscribers[id] else {
            lock.unlock()
            return
        }
        guard let opening else {
            subscribers[id] = nil
            lock.unlock()
            // Outside the lock: `finish` runs `onTermination`, which takes it.
            subscriber.continuation.finish()
            return
        }
        defer { lock.unlock() }
        // Yielding under the lock is what keeps held frames behind the
        // snapshot; `yield` never runs `onTermination`.
        subscriber.continuation.yield(opening)
        for event in subscriber.held ?? [] { subscriber.continuation.yield(event) }
        subscribers[id]?.held = nil
    }

    // MARK: Plumbing

    private func call<T: Decodable & Sendable>(
        _ method: String, params: [String: Any] = [:], as type: T.Type
    ) async throws -> T {
        let paramsJSON = try Self.encode(params)
        return try await perform { [self] in
            try Self.decode(self.transport.call(method, params: paramsJSON), method: method, as: T.self)
        }
    }

    /// Runs one blocking daemon call on `workQueue` and resumes with its
    /// answer.
    private func perform<T: Sendable>(_ body: @escaping @Sendable () throws -> T) async throws -> T {
        try await withCheckedThrowingContinuation { continuation in
            workQueue.async {
                continuation.resume(with: Result { try body() })
            }
        }
    }

    static func encode(_ params: [String: Any]) throws -> String {
        guard !params.isEmpty else { return "{}" }
        let data = try JSONSerialization.data(withJSONObject: params, options: [.sortedKeys])
        return String(decoding: data, as: UTF8.self)
    }

    static func decode<T: Decodable>(_ response: String, method: String, as type: T.Type) throws -> T {
        let data = try DaemonFrame.result(of: response, method: method)
        do {
            return try DaemonDataDecoding.decoder().decode(T.self, from: data)
        } catch {
            throw DaemonDataError.undecodable(method: method)
        }
    }
}

/// Unwraps one `tc_call` frame: `{"result": ...}` or `{"error": {code, message}}`.
public enum DaemonFrame {
    /// The fixed labels that mean the core is not there to answer.
    ///
    /// `daemon-stopped`, `daemon-disconnected` and `attached-transport-failed`
    /// are `tc_call`'s own; `handle-freed` is `TCDaemon`'s answer once
    /// teardown has begun; `null-handle` and `invalid-handle-pointer` mean
    /// there is no live handle to ask.
    static let unreachableMessages: Set<String> = [
        "daemon-stopped", "daemon-disconnected", "attached-transport-failed", "handle-freed",
        "null-handle", "invalid-handle-pointer",
    ]

    public static func result(of response: String, method: String) throws -> Data {
        guard let data = response.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { throw DaemonDataError.undecodable(method: method) }
        if let error = object["error"] as? [String: Any] {
            let code = error["code"] as? String ?? "unavailable"
            let message = error["message"] as? String ?? "unknown"
            if unreachableMessages.contains(message) { throw DaemonDataError.unreachable }
            throw DaemonDataError.daemon(code: code, message: message)
        }
        guard let result = object["result"] else { throw DaemonDataError.undecodable(method: method) }
        return try JSONSerialization.data(withJSONObject: result, options: [.fragmentsAllowed])
    }
}
