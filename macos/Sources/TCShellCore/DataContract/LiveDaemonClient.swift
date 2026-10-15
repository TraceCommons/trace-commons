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

/// The real `DaemonDataClient`, over `tc_call` (K1 of #1173).
///
/// Wired for every method that exists on main, `preview_unsure_spans`
/// included: `tc_call` answers through `ipc::handle_local`, which runs the
/// async dispatcher (`handle_request_async`), and that serves it. The
/// `*-requires-async` refusals belong to the synchronous `handle_request`
/// path only, which `tc_call` does not use. The network methods (C3, #1187)
/// are routed too. Only the two PROVISIONAL methods the screens still read,
/// `inferenceSummary()` and `missionCatalogue()`, throw `notAvailableYet`
/// and send nothing: the daemon's real reply has a different shape, served
/// by `networkInferenceSummary()` and `networkMissionCatalogue(limit:before:)`.
///
/// Every call runs on `workQueue`, never on the caller's thread. The C ABI
/// blocks for as long as the daemon takes (a preview is a redaction pass),
/// and blocking a Swift concurrency thread for that long starves the
/// cooperative pool -- the reason `TCDaemon` gives for not being an actor.
///
/// Events: `deliver(eventJSON:)` is fed by the app's one `tc_subscribe`
/// callback (`AppModel.subscribe`), so the data client and the existing
/// `AppModel` handlers see the same frames from the same subscription.
/// `tc_subscribe` sends no `snapshot` on the in-process path, so each
/// `events()` stream opens with one built here from `status` and
/// `list_pending`, as the contract promises; frames that arrive while it is
/// being built are held and follow it in order.
///
/// `@unchecked Sendable`: the state is the transport, which is safe to call
/// from any thread (`DaemonClient` already calls it off the main actor),
/// and the subscriber table, guarded by `lock`.
public final class LiveDaemonClient: DaemonDataClient, @unchecked Sendable {
    private let transport: any DaemonTransport
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

    public init(transport: any DaemonTransport) {
        self.transport = transport
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

    public func listPending(
        projectId: String?, filter: DaemonData.PendingFilter?, order: DaemonData.PendingOrder?
    ) async throws -> [DaemonData.QueueEntry] {
        var params: [String: Any] = [:]
        if let projectId { params["project_id"] = projectId }
        if let filter { params["filter"] = filter.rawValue }
        if let order, order != .queue { params["order"] = order.rawValue }
        return try await call("list_pending", params: params, as: DaemonData.PendingList.self).pending
    }

    // MARK: Nudges

    public func nudgeOpened(_ kind: NudgeSurface.Kind) async throws {
        _ = try await call("nudge_opened", params: ["kind": kind.rawValue], as: DaemonData.NudgeAck.self)
    }

    public func nudgeDecline(_ kind: NudgeSurface.Kind) async throws {
        _ = try await call("nudge_decline", params: ["kind": kind.rawValue], as: DaemonData.NudgeAck.self)
    }

    public func setSuggestionsEnabled(_ on: Bool) async throws {
        _ = try await call("set_suggestions_enabled", params: ["on": on], as: DaemonData.NudgeAck.self)
    }

    public func setMenuBarMarkEnabled(_ on: Bool) async throws {
        _ = try await call("set_menu_bar_mark_enabled", params: ["on": on], as: DaemonData.NudgeAck.self)
    }

    public func setNotificationsEnabled(_ on: Bool) async throws {
        _ = try await call("set_notifications_enabled", params: ["on": on], as: DaemonData.NudgeAck.self)
    }

    public func setNotifyKind(_ kind: String, on: Bool) async throws {
        _ = try await call("set_notify_kind", params: ["kind": kind, "on": on], as: DaemonData.NudgeAck.self)
    }

    public func dismissNotifyOffer(kind: String) async throws {
        guard let key = NudgeSettings.offerMarker(kind: kind) else { throw NudgeSettings.noOfferForKind }
        _ = try await call("set_settings", params: [key: false], as: DaemonData.NudgeAck.self)
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

    public func approve(entryId: String, verdict: ContributorVerdict?, correction: String?) async throws
        -> ApproveResponse
    {
        var params: [String: Any] = ["entry_id": entryId]
        if let verdict { params["outcome"] = verdict.rawValue }
        if let correction { params["correction"] = correction }
        return try await call("approve", params: params, as: ApproveResponse.self)
            .requireApproved(entryId: entryId)
    }

    public func cancel(entryId: String) async throws {
        _ = try await call("cancel", params: ["entry_id": entryId], as: DaemonData.OkReply.self)
    }

    public func cancelFolder(projectId: String) async throws -> Int {
        try await call("cancel", params: ["project_id": projectId], as: DaemonData.CancelFolderResult.self).canceled
    }

    public func approveFolder(
        projectId: String, verdict: ContributorVerdict?, filter: DaemonData.PendingFilter?
    ) async throws -> ApproveResponse {
        var params: [String: Any] = ["project_id": projectId]
        if let verdict { params["outcome"] = verdict.rawValue }
        if let filter { params["filter"] = filter.rawValue }
        return try await call("approve", params: params, as: ApproveResponse.self)
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

    public func setContributionOverride(mode: ProjectMode, confirm: Bool) async throws
        -> DaemonData.ContributionOverrideResult
    {
        var params: [String: Any] = ["mode": mode.rawValue]
        if mode == .autoUpload, confirm { params["confirm"] = true }
        return try await call("set_contribution_override", params: params, as: DaemonData.ContributionOverrideResult.self)
    }

    public func clearContributionOverride() async throws -> DaemonData.ContributionOverrideClearResult {
        try await call("clear_contribution_override", as: DaemonData.ContributionOverrideClearResult.self)
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

    public func privateAI() async throws -> DaemonData.PrivateAISwitch {
        DaemonData.PrivateAISwitch(settings: try await settings())
    }

    public func setPrivateAI(on: Bool) async throws -> DaemonData.PrivateAISwitch {
        DaemonData.PrivateAISwitch(
            settings: try await call(
                "set_settings", params: PrivateInferenceSurface.settingsParams(on: on), as: DaemonData.Settings.self))
    }

    public func setScrubCheck(_ mode: DaemonData.ScrubCheckMode) async throws -> DaemonData.Settings {
        try await call("set_settings", params: ["scrub_check": mode.rawValue], as: DaemonData.Settings.self)
    }

    public func setLocalNotifications(_ on: Bool) async throws -> DaemonData.Settings {
        try await call("set_settings", params: ["local_notifications": on], as: DaemonData.Settings.self)
    }

    public func setInsightsRecapCard(_ on: Bool) async throws -> DaemonData.Settings {
        try await call("set_settings", params: ["insights_recap_card_enabled": on], as: DaemonData.Settings.self)
    }

    public func setInsightsLedgerFeed(_ on: Bool) async throws -> DaemonData.Settings {
        try await call("set_settings", params: ["insights_ledger_feed": on], as: DaemonData.Settings.self)
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

    // MARK: Insights

    public func insightsWeek(isoWeek: String?) async throws -> DaemonData.InsightsWeek {
        var params: [String: Any] = ["tz": TimeZone.current.secondsFromGMT()]
        if let isoWeek { params["iso_week"] = isoWeek }
        return try await call("insights_week", params: params, as: DaemonData.InsightsWeek.self)
    }

    public func insightsGlance(tzSeconds: Int) async throws -> DaemonData.InsightsGlance {
        try await call("insights_glance", params: ["tz": tzSeconds], as: DaemonData.InsightsGlance.self)
    }

    // MARK: Network methods (C3, #1187)

    public func networkInferenceSummary() async throws -> DaemonData.NetworkInferenceSummary {
        try await call("inference_summary", as: DaemonData.NetworkInferenceSummary.self)
    }

    public func inferenceCallProof(callId: Int64) async throws -> DaemonData.InferenceProofDetail {
        try await call("inference_call_proof", params: ["call_id": callId], as: DaemonData.InferenceProofDetail.self)
    }

    public func modelSpend() async throws -> DaemonData.ModelSpend {
        try await call("model_spend", as: DaemonData.ModelSpend.self)
    }

    public func networkPrivateAI() async throws -> DaemonData.NetworkPrivateAISwitch {
        try await call("private_ai", as: DaemonData.NetworkPrivateAISwitch.self)
    }

    public func setNetworkPrivateAI(on: Bool, consent: DaemonData.PrivateAIConsent?) async throws
        -> DaemonData.NetworkPrivateAISwitch
    {
        // Refused here, before `tc_call`: an enable with no consent never
        // reaches the daemon. The label is the daemon's own refusal.
        guard !on || consent != nil else {
            throw DaemonDataError.daemon(code: "bad_params", message: "confirmation-required")
        }
        return try await call(
            "set_private_ai", params: ["on": on, "confirmed": consent != nil],
            as: DaemonData.NetworkPrivateAISwitch.self)
    }

    public func networkMissionCatalogue(limit: Int?, before: String?) async throws
        -> DaemonData.NetworkMissionCatalogue
    {
        var params: [String: Any] = [:]
        if let limit { params["limit"] = limit }
        if let before { params["before"] = before }
        return try await call("mission_catalogue", params: params, as: DaemonData.NetworkMissionCatalogue.self)
    }

    public func lookupInvite(code: String) async throws -> DaemonData.InviteLookup {
        try await call("invite_lookup", params: ["code": code], as: DaemonData.InviteLookup.self)
    }

    public func passkeyState() async throws -> DaemonData.PasskeyState {
        try await call("passkey_state", as: DaemonData.PasskeyState.self)
    }

    public func accountState() async throws -> DaemonData.AccountState {
        try await call("account_session_status", as: DaemonData.AccountState.self)
    }

    public func activityMissionsCatalogue() async throws -> DaemonData.ActivityMissionsCatalogue {
        try await call("activity_missions_catalogue", as: DaemonData.ActivityMissionsCatalogue.self)
    }

    public func activityMissionsStatus() async throws -> DaemonData.ActivityMissionsStatus {
        try await call("activity_missions_status", as: DaemonData.ActivityMissionsStatus.self)
    }

    // MARK: PROVISIONAL shapes the screens still read

    public func inferenceSummary() async throws -> DaemonData.InferenceSummary {
        throw DaemonDataError.notAvailableYet(method: "inference_summary")
    }

    public func missionCatalogue() async throws -> DaemonData.MissionCatalogue {
        throw DaemonDataError.notAvailableYet(method: "mission_catalogue")
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
            // A reply that is not a frame, or a refusal, throws: the stream
            // opens with `.resyncRequired`. Only a well-formed `status` body
            // this build cannot read is unknown, which is not a reason to
            // withhold the queue.
            let statusBody = try DaemonFrame.result(of: transport.call("status", params: "{}"), method: "status")
            let status = try? DaemonDataDecoding.decoder().decode(DaemonData.Status.self, from: statusBody)
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
