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
/// shape and change nothing: the sets are fixtures, not a simulator. The
/// one exception is the contribution override, which `status` reflects
/// until it is cleared, so the menu-bar pill can be driven end to end.
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

    /// The contribution override in force, as the daemon keeps it: the one
    /// piece of state a sample set holds, so the pill can be driven end to
    /// end (`set_contribution_override` / `clear_contribution_override`).
    private var contributionOverride: (mode: ProjectMode, since: String)?

    /// Every override write this client answered, as `method params`, in
    /// order, refusals included. Tests read it to prove what was (and was
    /// not) sent.
    public var overrideCalls: [String] { lock.withLock { recordedOverrideCalls } }
    private var recordedOverrideCalls: [String] = []

    /// The raw JSON this set answers `method` with: the IPC `result`
    /// object, or `nil` for a set where the core is down. `status` carries
    /// the override in force, as the daemon's `status_value` does.
    public func json(for method: String) -> String? {
        guard set != .coreDown, let reply = SampleDaemonData.reply(method, in: set) else { return nil }
        guard method == "status" else { return reply }
        lock.lock()
        let active = contributionOverride
        lock.unlock()
        guard let active else { return reply }
        return statusWithOverride(reply, mode: active.mode, since: active.since)
    }

    /// The recorded `status` with the override's three fields, as the
    /// daemon sets them: `contribution_override: {mode, since}`, the roll-up
    /// is the override's mode, and partial is true only for Automatic
    /// beside a folder set to Never. (The daemon also counts the unresolved
    /// bucket; no sample set queues a session there.)
    private func statusWithOverride(_ status: String, mode: ProjectMode, since: String) -> String {
        guard var object = (try? JSONSerialization.jsonObject(with: Data(status.utf8))) as? [String: Any] else {
            return status
        }
        object["contribution_override"] = ["mode": mode.rawValue, "since": since]
        object["contribution_mode"] = mode.rawValue
        object["contribution_mode_partial"] = mode == .autoUpload && hasNeverFolder()
        guard let data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]) else {
            return status
        }
        return String(decoding: data, as: UTF8.self)
    }

    private func hasNeverFolder() -> Bool {
        guard let json = SampleDaemonData.reply("list_projects", in: set),
            let object = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any],
            let rows = object["projects"] as? [[String: Any]]
        else { return false }
        return rows.contains { (($0["folder_mode"] ?? $0["mode"]) as? String) == ProjectMode.ignore.rawValue }
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

    /// The card `preview` and a ready `preview_request` share, as the
    /// daemon's `preview_card_value` does. Only `preview` adds the entry.
    private func summary(for entryId: String, method: String, withEntry: Bool) throws -> DaemonData.PreviewSummary {
        guard let entry = try allEntries().first(where: { $0.entryId == entryId }) else {
            throw DaemonDataError.daemon(code: "bad_params", message: "unknown-entry-id")
        }
        let json = SampleDaemonData.previewCard(for: entry, withEntry: withEntry)
        return try decode(json, method: method, as: DaemonData.PreviewSummary.self)
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
        PreviewRequestResult(
            entryID: entryId, state: .ready, summary: try summary(for: entryId, method: "preview_request", withEntry: false))
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
        try summary(for: entryId, method: "preview", withEntry: true)
    }

    public func previewUnsureSpans(entryId: String, bodyDigest: String) async throws -> DaemonData.UnsureSpans {
        guard set != .coreDown else { throw DaemonDataError.unreachable }
        let json = SampleDaemonData.unsureSpans(entryId: entryId, bodyDigest: bodyDigest)
        return try decode(json, method: "preview_unsure_spans", as: DaemonData.UnsureSpans.self)
    }

    // MARK: Queue actions

    /// Approves a pending entry of this set. A kept entry answers the way
    /// the daemon answers an entry it holds but cannot act on: OK,
    /// `approved: 0`, and a `not-pending` skip, which `approve(entryId:)`
    /// throws as `notApproved`. An id this set never held is refused up
    /// front with `unknown-entry-id`, as the daemon refuses it.
    public func approve(entryId: String, verdict: ContributorVerdict?, correction: String?) async throws
        -> ApproveResponse
    {
        guard set != .coreDown else { throw DaemonDataError.unreachable }
        // The daemon's own checks, in its order: a correction needs a
        // `partly` or `failed` verdict.
        if let correction, !correction.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
            verdict != .partly, verdict != .failed
        {
            throw DaemonDataError.daemon(code: "bad_params", message: "correction-needs-outcome")
        }
        guard try allEntries().contains(where: { $0.entryId == entryId }) else {
            throw DaemonDataError.daemon(code: "bad_params", message: "unknown-entry-id")
        }
        let pending = try serve("list_pending", as: DaemonData.PendingList.self).pending
        let json = pending.contains(where: { $0.entryId == entryId })
            ? SampleDaemonData.approved
            : SampleDaemonData.approveSkipped(entryId: entryId, reason: "not-pending")
        return try decode(json, method: "approve", as: ApproveResponse.self).requireApproved(entryId: entryId)
    }

    /// Nothing is ever held in a sample set, so an Undo is always refused
    /// the way the daemon refuses an entry that is not approved.
    public func cancel(entryId: String) async throws {
        guard set != .coreDown else { throw DaemonDataError.unreachable }
        throw DaemonDataError.daemon(code: "bad_params", message: "not-cancelable")
    }

    public func cancelFolder(projectId: String) async throws -> Int {
        _ = try await listPending(projectId: projectId)
        return 0
    }

    /// Approves every pending entry of that folder in this set, except
    /// those held for a person (`heldForReview`), as the daemon's group
    /// selector does. A Manual Scrub check hold is approved with the rest.
    public func approveFolder(projectId: String, verdict: ContributorVerdict?) async throws -> ApproveResponse {
        let pending = try await listPending(projectId: projectId)
        let held = pending.filter(\.heldForReview).count
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

    /// Answers as `handle_set_contribution_override` does, in its order:
    /// Automatic without `confirm` is `bad_params` /
    /// `confirm-required`; without grant terms -- a set that is not
    /// enrolled (`status.logged_in` false: `empty`, whose store has no
    /// contributor configuration) -- it is `unavailable` /
    /// `arming-terms-unavailable`, and nothing changes; the override already
    /// in force answers `changed: false`. Nothing is ever returned to
    /// waiting, since no sample entry was approved unattended.
    public func setContributionOverride(mode: ProjectMode, confirm: Bool) async throws
        -> DaemonData.ContributionOverrideResult
    {
        guard set != .coreDown else { throw DaemonDataError.unreachable }
        record("set_contribution_override \(mode.rawValue)\(mode == .autoUpload && confirm ? " confirm" : "")")
        guard mode != .autoUpload || confirm else {
            throw DaemonDataError.daemon(code: "bad_params", message: "confirm-required")
        }
        if mode == .autoUpload, try serve("status", as: DaemonData.Status.self).loggedIn != true {
            throw DaemonDataError.daemon(code: "unavailable", message: "arming-terms-unavailable")
        }
        let (changed, since) = lock.withLock {
            let changed = contributionOverride?.mode != mode
            if changed {
                contributionOverride = (mode, ISO8601DateFormatter().string(from: Date()))
            }
            return (changed, contributionOverride?.since)
        }
        let value = #"{"mode":"\#(mode.rawValue)","since":"\#(since ?? "")"}"#
        let current = try decode(value, method: "set_contribution_override", as: DaemonData.ContributionOverride.self)
        if changed { emit(.statusChanged) }
        return DaemonData.ContributionOverrideResult(changed: changed, contributionOverride: current, returned: 0)
    }

    /// Answers as `handle_clear_contribution_override` does: the recorded
    /// status comes back exactly, and `cleared: false` when none was in force.
    public func clearContributionOverride() async throws -> DaemonData.ContributionOverrideClearResult {
        guard set != .coreDown else { throw DaemonDataError.unreachable }
        record("clear_contribution_override")
        let cleared = lock.withLock {
            defer { contributionOverride = nil }
            return contributionOverride != nil
        }
        if cleared { emit(.statusChanged) }
        return DaemonData.ContributionOverrideClearResult(cleared: cleared, returned: 0)
    }

    private func record(_ call: String) {
        lock.withLock { recordedOverrideCalls.append(call) }
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

    /// The Private AI value last written, as the daemon keeps it; `nil` is
    /// the recorded `get_settings` value.
    private var privateAIOn: Bool?

    /// Every Private AI write this client answered, in order. Tests read it
    /// to prove what was (and was not) sent.
    public var privateAICalls: [Bool] { lock.withLock { recordedPrivateAICalls } }
    private var recordedPrivateAICalls: [Bool] = []

    public func privateAI() async throws -> DaemonData.PrivateAISwitch {
        let recorded = DaemonData.PrivateAISwitch(settings: try serve("get_settings", as: DaemonData.Settings.self))
        guard let written = lock.withLock({ privateAIOn }) else { return recorded }
        return DaemonData.PrivateAISwitch(on: written, offerSeen: true, state: recorded.state)
    }

    public func setPrivateAI(on: Bool) async throws -> DaemonData.PrivateAISwitch {
        guard set != .coreDown else { throw DaemonDataError.unreachable }
        lock.withLock {
            recordedPrivateAICalls.append(on)
            privateAIOn = on
        }
        let answer = try await privateAI()
        emit(.statusChanged)
        return answer
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

    // MARK: Insights

    public func insightsWeek(isoWeek: String?) async throws -> DaemonData.InsightsWeek {
        guard set != .coreDown else { throw DaemonDataError.unreachable }
        // SAMPLE: no counter pass is recorded, so the window shows the
        // saved-imports feed, as it would against a daemon that predates it.
        throw DaemonDataError.notAvailableYet(method: "insights_week")
    }

    // MARK: Network methods (C3, #1187): hand-written samples

    public func networkInferenceSummary() async throws -> DaemonData.NetworkInferenceSummary {
        try serve("inference_summary", as: DaemonData.NetworkInferenceSummary.self)
    }

    public func inferenceCallProof(callId: Int64) async throws -> DaemonData.InferenceProofDetail {
        try serve("inference_call_proof", as: DaemonData.InferenceProofDetail.self)
    }

    public func modelSpend() async throws -> DaemonData.ModelSpend {
        try serve("model_spend", as: DaemonData.ModelSpend.self)
    }

    public func networkPrivateAI() async throws -> DaemonData.NetworkPrivateAISwitch {
        try serve("private_ai", as: DaemonData.NetworkPrivateAISwitch.self)
    }

    /// Answers the switch as asked -- off with no port, or running on the
    /// sample port -- keeping the set's disclosure. Not remembered: a later
    /// `networkPrivateAI()` still reads the set's own state.
    public func setNetworkPrivateAI(on: Bool, consent: DaemonData.PrivateAIConsent?) async throws
        -> DaemonData.NetworkPrivateAISwitch
    {
        guard set != .coreDown else { throw DaemonDataError.unreachable }
        guard !on || consent != nil else {
            throw DaemonDataError.daemon(code: "bad_params", message: "confirmation-required")
        }
        let shown = try serve("private_ai", as: DaemonData.NetworkPrivateAISwitch.self)
        let answer: [String: Any] = [
            "on": on, "state": on ? "running" : "off", "port": on ? 3128 : NSNull(), "disclosure": shown.disclosure,
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: answer) else {
            throw DaemonDataError.undecodable(method: "set_private_ai")
        }
        return try decode(String(decoding: data, as: UTF8.self), method: "set_private_ai",
                          as: DaemonData.NetworkPrivateAISwitch.self)
    }

    /// One fixed page: the sample has no second page to ask for.
    public func networkMissionCatalogue(limit: Int?, before: String?) async throws
        -> DaemonData.NetworkMissionCatalogue
    {
        try serve("mission_catalogue", as: DaemonData.NetworkMissionCatalogue.self)
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

    public func activityMissionsCatalogue() async throws -> DaemonData.ActivityMissionsCatalogue {
        guard set != .coreDown else { throw DaemonDataError.unreachable }
        guard set != .unknownCounts else {
            throw DaemonDataError.daemon(code: "unavailable", message: "activity-missions-unavailable")
        }
        return try serve("activity_missions_catalogue", as: DaemonData.ActivityMissionsCatalogue.self)
    }

    public func activityMissionsStatus() async throws -> DaemonData.ActivityMissionsStatus {
        guard set != .coreDown else { throw DaemonDataError.unreachable }
        // SAMPLE: no official configured policy or authoritative progress recording yet.
        throw DaemonDataError.daemon(code: "unavailable", message: "activity-missions-unavailable")
    }

    // MARK: PROVISIONAL shapes the screens still read

    /// The Inference screen's earlier shape, from `SampleDaemonData.provisional`;
    /// the wire's `inference_summary` is `networkInferenceSummary()`.
    public func inferenceSummary() async throws -> DaemonData.InferenceSummary {
        try serveProvisional("inference_summary", as: DaemonData.InferenceSummary.self)
    }

    /// The Missions screen's earlier shape, from `SampleDaemonData.provisional`;
    /// the wire's `mission_catalogue` is `networkMissionCatalogue(limit:before:)`.
    public func missionCatalogue() async throws -> DaemonData.MissionCatalogue {
        try serveProvisional("mission_catalogue", as: DaemonData.MissionCatalogue.self)
    }

    private func serveProvisional<T: Decodable>(_ method: String, as type: T.Type) throws -> T {
        guard set != .coreDown, let json = SampleDaemonData.provisional(method, in: set) else {
            throw DaemonDataError.unreachable
        }
        return try decode(json, method: method, as: T.self)
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
