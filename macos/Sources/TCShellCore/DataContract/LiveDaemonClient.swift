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
/// Wired for every method that exists on main. The PROVISIONAL network
/// methods (Zaki's C3) throw `notAvailableYet` until their IPC lands;
/// `previewUnsureSpans` does too, because the synchronous `tc_call` entry
/// point refuses it (`preview-unsure-spans-requires-async`) and its ABI
/// twin `tc_preview_unsure_spans_json` is routed in K1.
///
/// `@unchecked Sendable`: the only state is the transport, and the C ABI
/// is safe to call from any thread (`DaemonClient` already calls it off the
/// main actor). Calls block for as long as the daemon takes to answer.
public final class LiveDaemonClient: DaemonDataClient, @unchecked Sendable {
    private let transport: any DaemonTransport
    private let lock = NSLock()
    private var continuations: [UUID: AsyncStream<DaemonDataEvent>.Continuation] = [:]

    public init(transport: any DaemonTransport) {
        self.transport = transport
    }

    // MARK: Status and the queue

    public func status() async throws -> DaemonData.Status {
        try call("status", as: DaemonData.Status.self)
    }

    public func listPending(projectId: String?) async throws -> [DaemonData.QueueEntry] {
        var params: [String: Any] = [:]
        if let projectId { params["project_id"] = projectId }
        return try call("list_pending", params: params, as: DaemonData.PendingList.self).pending
    }

    public func listKept() async throws -> [DaemonData.QueueEntry] {
        try call("list_kept", as: DaemonData.KeptList.self).kept
    }

    public func preview(entryId: String) async throws -> DaemonData.PreviewSummary {
        try call("preview", params: ["entry_id": entryId], as: DaemonData.PreviewSummary.self)
    }

    public func previewUnsureSpans(entryId: String, bodyDigest: String) async throws -> DaemonData.UnsureSpans {
        throw DaemonDataError.notAvailableYet(method: "preview_unsure_spans")
    }

    public func approve(entryId: String) async throws -> DaemonData.ApproveResult {
        try call("approve", params: ["entry_id": entryId], as: DaemonData.ApproveResult.self)
    }

    public func keep(entryId: String) async throws -> DaemonData.KeepResult {
        try call("keep", params: ["entry_id": entryId], as: DaemonData.KeepResult.self)
    }

    public func undoKeep(entryId: String) async throws -> DaemonData.KeepResult {
        try call("undo_keep", params: ["entry_id": entryId], as: DaemonData.KeepResult.self)
    }

    public func dismiss(entryId: String) async throws {
        _ = try rawResult("dismiss", params: ["entry_id": entryId])
    }

    // MARK: Projects and tools

    public func listProjects() async throws -> DaemonData.ProjectList {
        try call("list_projects", as: DaemonData.ProjectList.self)
    }

    public func setProjectMode(projectId: String, mode: ProjectMode, includeBacklog: Bool?) async throws
        -> DaemonData.ProjectModeResult
    {
        var params: [String: Any] = ["project_id": projectId, "mode": mode.rawValue]
        if let includeBacklog { params["include_backlog"] = includeBacklog }
        return try call("set_project_mode", params: params, as: DaemonData.ProjectModeResult.self)
    }

    public func harnessList() async throws -> HarnessList {
        try call("harness_list", as: HarnessList.self)
    }

    // MARK: Settings

    public func settings() async throws -> DaemonData.Settings {
        try call("get_settings", as: DaemonData.Settings.self)
    }

    public func setScrubCheck(_ mode: DaemonData.ScrubCheckMode) async throws -> DaemonData.Settings {
        try call("set_settings", params: ["scrub_check": mode.rawValue], as: DaemonData.Settings.self)
    }

    public func setLocalNotifications(_ on: Bool) async throws -> DaemonData.Settings {
        try call("set_settings", params: ["local_notifications": on], as: DaemonData.Settings.self)
    }

    public func setDigestSchedule(_ schedule: DaemonData.DigestSchedule) async throws -> DaemonData.Settings {
        var value: [String: Any] = ["mode": schedule.mode]
        if let hour = schedule.hour { value["hour"] = hour }
        return try call("set_settings", params: ["digest_schedule": value], as: DaemonData.Settings.self)
    }

    // MARK: History and credit

    public func listHistory(limit: Int) async throws -> [DaemonData.HistoryRow] {
        try call("list_history", params: ["limit": limit], as: DaemonData.HistoryList.self).history
    }

    public func historyRollup() async throws -> DaemonData.HistoryRollup {
        try call("history_rollup", as: DaemonData.HistoryRollup.self)
    }

    public func commonsCreditSummary() async throws -> DaemonData.CommonsCreditSummary {
        try call("commons_credit_summary", as: DaemonData.CommonsCreditSummary.self)
    }

    // MARK: The map and the Inference tab

    public func toolDestinations() async throws -> DaemonData.ToolDestinations {
        try call("tool_destinations", as: DaemonData.ToolDestinations.self)
    }

    public func inferenceCalls(limit: Int, cursor: String?) async throws -> DaemonData.InferenceCallPage {
        var params: [String: Any] = ["limit": limit]
        if let cursor { params["cursor"] = cursor }
        return try call("inference_calls", params: params, as: DaemonData.InferenceCallPage.self)
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

    private func call<T: Decodable>(_ method: String, params: [String: Any] = [:], as type: T.Type) throws -> T {
        let data = try rawResult(method, params: params)
        do {
            return try DaemonDataDecoding.decoder().decode(T.self, from: data)
        } catch {
            throw DaemonDataError.undecodable(method: method)
        }
    }

    private func rawResult(_ method: String, params: [String: Any]) throws -> Data {
        let paramsJSON: String
        if params.isEmpty {
            paramsJSON = "{}"
        } else {
            let data = try JSONSerialization.data(withJSONObject: params)
            paramsJSON = String(decoding: data, as: UTF8.self)
        }
        return try DaemonFrame.result(of: transport.call(method, params: paramsJSON), method: method)
    }
}

/// Unwraps one `tc_call` frame: `{"result": ...}` or `{"error": {code, message}}`.
public enum DaemonFrame {
    /// The fixed labels that mean the core is not there to answer.
    static let unreachableMessages: Set<String> = [
        "daemon-stopped", "attached-transport-failed", "null-handle", "invalid-handle-pointer",
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
