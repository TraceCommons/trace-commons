#if DEBUG
import Foundation

/// C1 of #1173: a `DaemonDataClient` that needs no daemon, for building and
/// previewing screens. DEBUG ONLY: this file and `SampleDaemonData.swift`
/// compile to nothing in a Release build.
///
/// It serves recorded-shape JSON through the same decoders the live client
/// uses, so a screen that renders a sample renders the real reply. One
/// named set per state a screen must draw:
///
/// ```swift
/// MyScreen(data: SampleDaemonClient(.heldSessions))
/// ```
///
/// Writes (`keep`, `approve`, settings) answer with the daemon's reply
/// shape and change nothing: the sets are fixtures, not a simulator.
/// `emit(_:)` pushes an event to every open `events()` stream, so a preview
/// can show a live update.
public final class SampleDaemonClient: DaemonDataClient, @unchecked Sendable {
    public enum SampleSet: String, CaseIterable, Sendable {
        /// A fresh install: nothing found, nothing decided.
        case empty
        /// A few sessions waiting, some history, Private AI running.
        case normalDay
        /// Many sessions waiting across folders.
        case busyQueue
        /// Every call throws `DaemonDataError.unreachable`.
        case coreDown
        /// `decisions_owed` absent, and every other figure the daemon may
        /// not know reported as unknown.
        case unknownCounts
        /// Sessions held for `second-look-review-required` and `scrub-check-manual`.
        case heldSessions
        /// A folder armed from now: its new sessions go on their own and its
        /// backlog waits.
        case armedFolder
    }

    public let set: SampleSet
    private let lock = NSLock()
    private var continuations: [UUID: AsyncStream<DaemonDataEvent>.Continuation] = [:]

    public init(_ set: SampleSet) {
        self.set = set
    }

    /// The raw JSON this set answers `method` with: the IPC `result`
    /// object, or `nil` for a set where the core is down.
    public func json(for method: String) -> String? {
        set == .coreDown ? nil : SampleDaemonData.reply(method, in: set)
    }

    private func serve<T: Decodable>(_ method: String, as type: T.Type) throws -> T {
        guard let json = json(for: method) else { throw DaemonDataError.unreachable }
        do {
            return try DaemonDataDecoding.decoder().decode(T.self, from: Data(json.utf8))
        } catch {
            throw DaemonDataError.undecodable(method: method)
        }
    }

    // MARK: Status and the queue

    public func status() async throws -> DaemonData.Status {
        try serve("status", as: DaemonData.Status.self)
    }

    public func listPending(projectId: String?) async throws -> [DaemonData.QueueEntry] {
        let all = try serve("list_pending", as: DaemonData.PendingList.self).pending
        guard let projectId else { return all }
        return all.filter { $0.projectId == projectId }
    }

    public func listKept() async throws -> [DaemonData.QueueEntry] {
        try serve("list_kept", as: DaemonData.KeptList.self).kept
    }

    public func preview(entryId: String) async throws -> DaemonData.PreviewSummary {
        let pending = try serve("list_pending", as: DaemonData.PendingList.self).pending
            + serve("list_kept", as: DaemonData.KeptList.self).kept
        guard let entry = pending.first(where: { $0.entryId == entryId }) else {
            throw DaemonDataError.daemon(code: "bad_params", message: "unknown-entry-id")
        }
        let json = SampleDaemonData.previewSummary(for: entry)
        do {
            return try DaemonDataDecoding.decoder().decode(DaemonData.PreviewSummary.self, from: Data(json.utf8))
        } catch {
            throw DaemonDataError.undecodable(method: "preview")
        }
    }

    public func previewUnsureSpans(entryId: String, bodyDigest: String) async throws -> DaemonData.UnsureSpans {
        guard set != .coreDown else { throw DaemonDataError.unreachable }
        let json = SampleDaemonData.unsureSpans(entryId: entryId, bodyDigest: bodyDigest)
        return try DaemonDataDecoding.decoder().decode(DaemonData.UnsureSpans.self, from: Data(json.utf8))
    }

    public func approve(entryId: String) async throws -> DaemonData.ApproveResult {
        try serve("approve", as: DaemonData.ApproveResult.self)
    }

    public func keep(entryId: String) async throws -> DaemonData.KeepResult {
        try serve("keep", as: DaemonData.KeepResult.self)
    }

    public func undoKeep(entryId: String) async throws -> DaemonData.KeepResult {
        try serve("undo_keep", as: DaemonData.KeepResult.self)
    }

    public func dismiss(entryId: String) async throws {
        guard set != .coreDown else { throw DaemonDataError.unreachable }
    }

    // MARK: Projects and tools

    public func listProjects() async throws -> DaemonData.ProjectList {
        try serve("list_projects", as: DaemonData.ProjectList.self)
    }

    public func setProjectMode(projectId: String, mode: ProjectMode, includeBacklog: Bool?) async throws
        -> DaemonData.ProjectModeResult
    {
        try serve("set_project_mode", as: DaemonData.ProjectModeResult.self)
    }

    public func harnessList() async throws -> HarnessList {
        try serve("harness_list", as: HarnessList.self)
    }

    // MARK: Settings

    public func settings() async throws -> DaemonData.Settings {
        try serve("get_settings", as: DaemonData.Settings.self)
    }

    public func setScrubCheck(_ mode: DaemonData.ScrubCheckMode) async throws -> DaemonData.Settings {
        try serve("get_settings", as: DaemonData.Settings.self)
    }

    public func setLocalNotifications(_ on: Bool) async throws -> DaemonData.Settings {
        try serve("get_settings", as: DaemonData.Settings.self)
    }

    public func setDigestSchedule(_ schedule: DaemonData.DigestSchedule) async throws -> DaemonData.Settings {
        try serve("get_settings", as: DaemonData.Settings.self)
    }

    // MARK: History and credit

    public func listHistory(limit: Int) async throws -> [DaemonData.HistoryRow] {
        Array(try serve("list_history", as: DaemonData.HistoryList.self).history.prefix(limit))
    }

    public func historyRollup() async throws -> DaemonData.HistoryRollup {
        try serve("history_rollup", as: DaemonData.HistoryRollup.self)
    }

    public func commonsCreditSummary() async throws -> DaemonData.CommonsCreditSummary {
        try serve("commons_credit_summary", as: DaemonData.CommonsCreditSummary.self)
    }

    // MARK: The map and the Inference tab

    public func toolDestinations() async throws -> DaemonData.ToolDestinations {
        try serve("tool_destinations", as: DaemonData.ToolDestinations.self)
    }

    public func inferenceCalls(limit: Int, cursor: String?) async throws -> DaemonData.InferenceCallPage {
        try serve("inference_calls", as: DaemonData.InferenceCallPage.self)
    }

    // MARK: SAMPLE network methods (C3)

    public func inferenceSummary() async throws -> DaemonData.InferenceSummary {
        try serve("inference_summary", as: DaemonData.InferenceSummary.self)
    }

    public func inferenceCallProof(callId: Int64) async throws -> DaemonData.InferenceProofDetail {
        try serve("inference_call_proof", as: DaemonData.InferenceProofDetail.self)
    }

    public func modelSpend() async throws -> DaemonData.ModelSpend {
        try serve("model_spend", as: DaemonData.ModelSpend.self)
    }

    public func privateAI() async throws -> DaemonData.PrivateAISwitch {
        try serve("private_ai", as: DaemonData.PrivateAISwitch.self)
    }

    public func setPrivateAI(on: Bool, confirmed: Bool) async throws -> DaemonData.PrivateAISwitch {
        guard set != .coreDown else { throw DaemonDataError.unreachable }
        guard !on || confirmed else {
            throw DaemonDataError.daemon(code: "bad_params", message: "confirmation-required")
        }
        return try serve("private_ai", as: DaemonData.PrivateAISwitch.self)
    }

    public func missionCatalogue() async throws -> DaemonData.MissionCatalogue {
        try serve("mission_catalogue", as: DaemonData.MissionCatalogue.self)
    }

    public func lookupInvite(code: String) async throws -> DaemonData.InviteLookup {
        try serve("invite_lookup", as: DaemonData.InviteLookup.self)
    }

    public func passkeyState() async throws -> DaemonData.PasskeyState {
        try serve("passkey_state", as: DaemonData.PasskeyState.self)
    }

    public func accountState() async throws -> DaemonData.AccountState {
        try serve("account_session_status", as: DaemonData.AccountState.self)
    }

    // MARK: Live updates

    /// Opens with a `snapshot` of this set, as `subscribe` does; a core-down
    /// set's stream finishes at once.
    public func events() -> AsyncStream<DaemonDataEvent> {
        let id = UUID()
        let opening: DaemonDataEvent? = json(for: "status").map { status in
            DaemonDataEventParser.parse(
                #"{"event":"snapshot","data":{"pending":\#(SampleDaemonData.pending(set)),"status":\#(status)}}"#)
        }
        return AsyncStream { continuation in
            guard let opening else {
                continuation.finish()
                return
            }
            continuation.yield(opening)
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

    /// Pushes `event` to every open `events()` stream, for previews.
    public func emit(_ event: DaemonDataEvent) {
        lock.lock()
        let targets = Array(continuations.values)
        lock.unlock()
        for continuation in targets { continuation.yield(event) }
    }
}
#endif
