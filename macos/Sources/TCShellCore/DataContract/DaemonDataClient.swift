import Foundation

/// C1 of #1173: the one protocol every screen reads daemon data through.
///
/// Screens hold an `any DaemonDataClient` and never know which one they
/// have. During development and in previews that is `SampleDaemonClient`
/// (Debug only); in the app it is `LiveDaemonClient`, over the same
/// `tc_call` path `DaemonClient` uses today. Swapping one for the other
/// changes no screen code.
///
/// Named `DaemonDataClient`, not `DaemonClient`, because `DaemonClient` is
/// already the app target's concrete, blocking typed layer, and every
/// existing screen depends on it. This protocol sits beside it rather than
/// replacing it: K1 moves the remaining calls across, and Ron's screens
/// are written against this one from the start.
///
/// One `async throws` method per IPC method a screen needs. Unsupported
/// daemon methods retain the daemon's `unknown_method` refusal; no sample
/// data substitutes for unavailable live data.
public protocol DaemonDataClient: Sendable {
    // MARK: Status and the queue

    /// `status`.
    func status() async throws -> DaemonData.Status
    /// `list_pending`, optionally narrowed to one project (the past-session picker).
    func listPending(projectId: String?) async throws -> [DaemonData.QueueEntry]
    /// `list_kept`.
    func listKept() async throws -> [DaemonData.QueueEntry]
    /// `preview`: the summary only, with `title` and `unsure_spans`.
    func preview(entryId: String) async throws -> DaemonData.PreviewSummary
    /// `preview_unsure_spans` for the body `preview_body` returned.
    func previewUnsureSpans(entryId: String, bodyDigest: String) async throws -> DaemonData.UnsureSpans
    /// `approve` for one entry.
    func approve(entryId: String) async throws -> DaemonData.ApproveResult
    /// `keep`: Keep on this Mac.
    func keep(entryId: String) async throws -> DaemonData.KeepResult
    /// `undo_keep`.
    func undoKeep(entryId: String) async throws -> DaemonData.KeepResult
    /// `dismiss`: permanent.
    func dismiss(entryId: String) async throws

    // MARK: Projects and tools

    /// `list_projects`.
    func listProjects() async throws -> DaemonData.ProjectList
    /// `set_project_mode`. `includeBacklog` only with `.autoUpload`.
    func setProjectMode(projectId: String, mode: ProjectMode, includeBacklog: Bool?) async throws
        -> DaemonData.ProjectModeResult
    /// `harness_list`.
    func harnessList() async throws -> HarnessList

    // MARK: Settings

    /// `get_settings`.
    func settings() async throws -> DaemonData.Settings
    /// `set_settings` with `scrub_check`.
    func setScrubCheck(_ mode: DaemonData.ScrubCheckMode) async throws -> DaemonData.Settings
    /// `set_settings` with `local_notifications`.
    func setLocalNotifications(_ on: Bool) async throws -> DaemonData.Settings
    /// `set_settings` with `digest_schedule`.
    func setDigestSchedule(_ schedule: DaemonData.DigestSchedule) async throws -> DaemonData.Settings

    // MARK: History and credit

    /// `list_history`.
    func listHistory(limit: Int) async throws -> [DaemonData.HistoryRow]
    /// `history_rollup`.
    func historyRollup() async throws -> DaemonData.HistoryRollup
    /// `commons_credit_summary`: always succeeds on the daemon; halves it
    /// could not read are `unknown`.
    func commonsCreditSummary() async throws -> DaemonData.CommonsCreditSummary

    // MARK: The map and the Inference tab

    /// `tool_destinations`.
    func toolDestinations() async throws -> DaemonData.ToolDestinations
    /// `inference_calls`. `limit` 1-200; `cursor` is the previous page's `nextCursor`.
    func inferenceCalls(limit: Int, cursor: String?) async throws -> DaemonData.InferenceCallPage

    // MARK: Network methods (C3)

    /// Z1.1, upstream grouped summary; registry-priced cost is not billed spend.
    func inferenceSummary() async throws -> DaemonData.InferenceSummary
    /// Z1.2, `inference_call_proof`.
    func inferenceCallProof(callId: Int64) async throws -> DaemonData.InferenceProofDetail
    /// Z1.3, billed spend per model.
    func modelSpend() async throws -> DaemonData.ModelSpend
    /// Z1.5, the Private AI switch's state and disclosure.
    func privateAI() async throws -> DaemonData.PrivateAISwitch
    /// Z1.5, turning Private AI on or off.
    /// `confirmed` is the caller's explicit answer after the core disclosure.
    /// Enabling with false is refused; no implicit acknowledgement is supplied.
    func setPrivateAI(on: Bool, confirmed: Bool) async throws -> DaemonData.PrivateAISwitch
    /// Z2.2, the mission catalogue.
    func missionCatalogue() async throws -> DaemonData.MissionCatalogue
    /// Z3.1, invite lookup. `code` carries the full invite URL.
    func lookupInvite(code: String) async throws -> DaemonData.InviteLookup
    /// Z3.2, passkey binding state.
    func passkeyState() async throws -> DaemonData.PasskeyState
    /// Z3.4, `account_session_status`.
    func accountState() async throws -> DaemonData.AccountState

    /// Read-only shared trace_activity policy; no matching/profile input leaves the Mac.
    func activityMissionsCatalogue() async throws -> DaemonData.ActivityMissionsCatalogue
    /// Authenticated server contribution facts; unavailable never becomes zero progress.
    func activityMissionsStatus() async throws -> DaemonData.ActivityMissionsStatus

    // MARK: Live updates

    /// The daemon's events, for screens that refresh live. Each call returns
    /// a fresh stream. On `.resyncRequired`, refetch `status` and `listPending`.
    func events() -> AsyncStream<DaemonDataEvent>
}

/// What the event stream carries: the contract's events, plus a
/// provisional per-call event for the map pulse.
public enum DaemonDataEvent: Equatable, Sendable {
    /// Sent first on subscribe: the whole queue and status.
    case snapshot(pending: [DaemonData.QueueEntry], status: DaemonData.Status?)
    case queueChanged
    case statusChanged
    case digestDue(pending: Int?, text: String?)
    /// A scheduled preview finished; refetch it with `preview(entryId:)`.
    case previewReady(entryId: String)
    /// Fell behind: refetch `status` and `listPending`.
    case resyncRequired
    /// `inference_call_added`, so the map pulses per real call.
    // PROVISIONAL: event not on main yet; shape follows `inference_calls` rows.
    case inferenceCallAdded(DaemonData.InferenceCall)
    case unknown(String)
}

public enum DaemonDataEventParser {
    /// One `{"event": ..., "data": {...}}` frame, as `tc_subscribe` delivers it.
    public static func parse(_ json: String) -> DaemonDataEvent {
        guard let data = json.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let name = object["event"] as? String
        else { return .unknown("unparseable") }
        let payload = object["data"] as? [String: Any] ?? [:]
        let payloadData = (try? JSONSerialization.data(withJSONObject: payload)) ?? Data("{}".utf8)
        let decoder = DaemonDataDecoding.decoder()
        switch name {
        case "snapshot":
            struct Snapshot: Decodable {
                let pending: [DaemonData.QueueEntry]
                let status: DaemonData.Status?
            }
            guard let snap = try? decoder.decode(Snapshot.self, from: payloadData) else {
                return .resyncRequired
            }
            return .snapshot(pending: snap.pending, status: snap.status)
        case "queue_changed": return .queueChanged
        case "status_changed": return .statusChanged
        case "digest_due":
            return .digestDue(pending: payload["pending"] as? Int, text: payload["text"] as? String)
        case "preview_ready":
            guard let id = payload["entry_id"] as? String else { return .unknown(name) }
            return .previewReady(entryId: id)
        case "resync_required", "lagged": return .resyncRequired
        case "inference_call_added":
            guard let call = try? decoder.decode(DaemonData.InferenceCall.self, from: payloadData) else {
                return .unknown(name)
            }
            return .inferenceCallAdded(call)
        default: return .unknown(name)
        }
    }
}
