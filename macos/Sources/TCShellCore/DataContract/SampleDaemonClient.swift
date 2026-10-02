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
        return try decode(json, method: method, as: T.self)
    }

    private func decode<T: Decodable>(_ json: String, method: String, as type: T.Type) throws -> T {
        do {
            return try DaemonDataDecoding.decoder().decode(T.self, from: Data(json.utf8))
        } catch {
            throw DaemonDataError.undecodable(method: method, from: error)
        }
    }

    /// Every pending and kept entry in this set.
    private func allEntries() throws -> [DaemonData.QueueEntry] {
        try serve("list_pending", as: DaemonData.PendingList.self).pending
            + serve("list_kept", as: DaemonData.KeptList.self).kept
    }

    /// `preview` and a ready `preview_request` share this summary.
    private func summary(for entryId: String, method: String) throws -> DaemonData.PreviewSummary {
        guard let entry = try allEntries().first(where: { $0.entryId == entryId }) else {
            throw DaemonDataError.daemon(code: "bad_params", message: "unknown-entry-id")
        }
        return try decode(SampleDaemonData.previewSummary(for: entry), method: method, as: DaemonData.PreviewSummary.self)
    }

    // MARK: Status and the queue

    public func status() async throws -> DaemonData.Status {
        try serve("status", as: DaemonData.Status.self)
    }

    public func listPending(projectId: String?) async throws -> [DaemonData.QueueEntry] {
        let all = try serve("list_pending", as: DaemonData.PendingList.self).pending
        guard let projectId else { return all }
        // The daemon knows a project from its policy (the `list_projects`
        // rows) and from the queue; anything else it refuses, as here.
        let known = Set(try serve("list_projects", as: DaemonData.ProjectList.self).projects.map(\.projectId))
            .union(try allEntries().map(\.projectId))
        guard known.contains(projectId) else {
            throw DaemonDataError.daemon(code: "bad_params", message: "project-id-unrecognized")
        }
        return all.filter { $0.projectId == projectId }
    }

    public func listKept() async throws -> [DaemonData.QueueEntry] {
        try serve("list_kept", as: DaemonData.KeptList.self).kept
    }

    // MARK: Previews

    /// Every sample preview is already built: `ready`, from cache, so no
    /// `previewReady` event follows (as the daemon does for a cached one).
    public func requestPreview(entryId: String) async throws -> DaemonData.PreviewRequestOutcome {
        PreviewRequestResult(entryID: entryId, state: .ready, summary: try summary(for: entryId, method: "preview_request"))
    }

    public func setVisiblePreviews(entryIds: [String]) async throws -> Int {
        guard set != .coreDown else { throw DaemonDataError.unreachable }
        return entryIds.count
    }

    /// Nothing is ever scheduled, so there is never anything to drop.
    public func cancelPreview(entryId: String) async throws -> DaemonData.PreviewCancelResult {
        guard set != .coreDown else { throw DaemonDataError.unreachable }
        return DaemonData.PreviewCancelResult(entryId: entryId, dropped: false)
    }

    public func preview(entryId: String) async throws -> DaemonData.PreviewSummary {
        try summary(for: entryId, method: "preview")
    }

    public func previewUnsureSpans(entryId: String, bodyDigest: String) async throws -> DaemonData.UnsureSpans {
        guard set != .coreDown else { throw DaemonDataError.unreachable }
        let json = SampleDaemonData.unsureSpans(entryId: entryId, bodyDigest: bodyDigest)
        return try decode(json, method: "preview_unsure_spans", as: DaemonData.UnsureSpans.self)
    }

    // MARK: Queue actions

    /// Approves a pending entry of this set. Any other id answers the way
    /// the daemon answers an entry it cannot act on: OK, `approved: 0`, and
    /// a `not-pending` skip, which `approve(entryId:)` throws as
    /// `notApproved`.
    public func approve(entryId: String) async throws -> ApproveResponse {
        let pending = try serve("list_pending", as: DaemonData.PendingList.self).pending
        let json = pending.contains(where: { $0.entryId == entryId })
            ? SampleDaemonData.approved
            : SampleDaemonData.approveSkipped(entryId: entryId, reason: "not-pending")
        return try decode(json, method: "approve", as: ApproveResponse.self).requireApproved(entryId: entryId)
    }

    /// Approves every pending entry of that folder in this set, held ones
    /// excepted, as the daemon's group selector does.
    public func approveFolder(projectId: String) async throws -> ApproveResponse {
        let pending = try await listPending(projectId: projectId)
        let held = pending.filter { $0.heldForSecondLook || $0.heldByManualScrubCheck }.count
        let json = SampleDaemonData.approvedGroup(approved: pending.count - held, excludedHeld: held)
        return try decode(json, method: "approve", as: ApproveResponse.self)
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

    public func setSource(_ kind: SourceKind, _ choice: SourceChoice) async throws -> DaemonData.Settings {
        guard choice.settingsParams(for: kind) != nil else { throw DaemonData.unansweredSource }
        return try serve("get_settings", as: DaemonData.Settings.self)
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

    // MARK: PROVISIONAL network methods (Zaki's C3)

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

    public func setPrivateAI(on: Bool) async throws -> DaemonData.PrivateAISwitch {
        try serve("private_ai", as: DaemonData.PrivateAISwitch.self)
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
