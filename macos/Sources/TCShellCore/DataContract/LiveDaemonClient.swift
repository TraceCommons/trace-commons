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

/// The real `DaemonDataClient`, over `tc_call`.
///
/// Wired for every method that exists on main, `preview_unsure_spans`
/// included: `tc_call` answers through `ipc::handle_local`, which runs the
/// async dispatcher (`handle_request_async`), and that serves it. The
/// `*-requires-async` refusals belong to the synchronous `handle_request`
/// path only, which `tc_call` does not use. The PROVISIONAL network methods
/// (Zaki's C3) throw `notAvailableYet` until their IPC lands, and send
/// nothing.
///
/// Every call runs on `workQueue`, never on the caller's thread. The C ABI
/// blocks for as long as the daemon takes (a preview is a redaction pass),
/// and blocking a Swift concurrency thread for that long starves the
/// cooperative pool -- the reason `TCDaemon` gives for not being an actor.
///
/// `@unchecked Sendable`: the state is the transport, which is safe to call
/// from any thread (`DaemonClient` already calls it off the main actor),
/// and the subscriber table, guarded by `lock`.
public final class LiveDaemonClient: DaemonDataClient, @unchecked Sendable {
    private let transport: any DaemonTransport
    private let workQueue = DispatchQueue(
        label: "trace-commons.live-daemon-client", qos: .userInitiated, attributes: .concurrent)
    private let lock = NSLock()
    private var continuations: [UUID: AsyncStream<DaemonDataEvent>.Continuation] = [:]

    public init(transport: any DaemonTransport) {
        self.transport = transport
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

    // MARK: Previews

    public func requestPreview(entryId: String) async throws -> DaemonData.PreviewRequestOutcome {
        try await call("preview_request", params: ["entry_id": entryId], as: DaemonData.PreviewRequestOutcome.self)
    }

    public func setVisiblePreviews(entryIds: [String]) async throws -> Int {
        try await call("preview_visible", params: ["entry_ids": entryIds], as: DaemonData.PreviewVisibleResult.self)
            .visible
    }

    public func cancelPreview(entryId: String) async throws -> DaemonData.PreviewCancelResult {
        try await call("preview_cancel", params: ["entry_id": entryId], as: DaemonData.PreviewCancelResult.self)
    }

    public func preview(entryId: String) async throws -> DaemonData.PreviewSummary {
        try await call("preview", params: ["entry_id": entryId], as: DaemonData.PreviewSummary.self)
    }

    public func previewUnsureSpans(entryId: String, bodyDigest: String) async throws -> DaemonData.UnsureSpans {
        try await call(
            "preview_unsure_spans", params: ["entry_id": entryId, "body_digest": bodyDigest],
            as: DaemonData.UnsureSpans.self)
    }

    // MARK: Queue actions

    public func approve(entryId: String) async throws -> ApproveResponse {
        try await call("approve", params: ["entry_id": entryId], as: ApproveResponse.self)
            .requireApproved(entryId: entryId)
    }

    public func approveFolder(projectId: String) async throws -> ApproveResponse {
        try await call("approve", params: ["project_id": projectId], as: ApproveResponse.self)
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

    public func setSource(_ kind: SourceKind, _ choice: SourceChoice) async throws -> DaemonData.Settings {
        guard let params = choice.settingsParams(for: kind) else { throw DaemonData.unansweredSource }
        return try await call("set_settings", params: params, as: DaemonData.Settings.self)
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

    /// A stream fed by `deliver(eventJSON:)`. The app's existing
    /// `tc_subscribe` callback forwards each frame there (K1 wires it).
    public func events() -> AsyncStream<DaemonDataEvent> {
        let id = UUID()
        return AsyncStream { continuation in
            lock.lock()
            continuations[id] = continuation
            lock.unlock()
            continuation.onTermination = { [weak self] _ in
                guard let self else { return }
                self.lock.lock()
                self.continuations[id] = nil
                self.lock.unlock()
            }
        }
    }

    /// Hands one subscription frame to every open `events()` stream.
    public func deliver(eventJSON: String) {
        let event = DaemonDataEventParser.parse(eventJSON)
        lock.lock()
        let targets = Array(continuations.values)
        lock.unlock()
        for continuation in targets { continuation.yield(event) }
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
    /// answer, so no Swift concurrency thread waits on the daemon.
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
            throw DaemonDataError.undecodable(method: method, from: error)
        }
    }
}

extension ApproveResponse {
    /// The single-entry rule of `DaemonDataClient.approve(entryId:)`: an OK
    /// reply that approved nothing is a refusal, carried as
    /// `notApproved` with the `skipped` row's fixed label.
    func requireApproved(entryId: String) throws -> ApproveResponse {
        guard approved == 0 else { return self }
        let row = skipped.first { $0.entryID == entryId } ?? skipped.first
        throw DaemonDataError.notApproved(reasonLabel: row?.reasonLabel)
    }
}

/// Unwraps one `tc_call` frame: `{"result": ...}` or `{"error": {code, message}}`.
public enum DaemonFrame {
    /// The fixed labels that mean the core is not there to answer.
    ///
    /// `daemon-stopped`, `daemon-disconnected` (an attached handle whose
    /// daemon stopped listening, disconnected or timed out) and
    /// `attached-transport-failed` are `tc_call`'s own; `handle-freed` is
    /// `TCDaemon`'s answer once teardown has begun; `null-handle` and
    /// `invalid-handle-pointer` mean there is no live handle to ask.
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
