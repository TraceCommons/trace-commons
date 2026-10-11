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
/// One `async throws` method per IPC method a screen needs. The network
/// methods (C3, #1187) are routed. Two methods marked PROVISIONAL keep the
/// earlier shapes the screens were written against, and the live client
/// throws `DaemonDataError.notAvailableYet` for them; their real replies are
/// `networkInferenceSummary()` and `networkMissionCatalogue(limit:before:)`.
/// A method an older attached daemon does not have keeps the daemon's
/// `unknown_method` refusal; no sample data substitutes for live data.
public protocol DaemonDataClient: Sendable {
    // MARK: Status and the queue

    /// `status`.
    func status() async throws -> DaemonData.Status
    /// `list_pending`, optionally narrowed to one project (the past-session
    /// picker). An id the daemon does not know is refused with
    /// `.daemon(code: "bad_params", message: "project-id-unrecognized")`,
    /// never answered with `[]`, so a stale id cannot read as "nothing
    /// waiting".
    ///
    /// `filter` narrows it to the idle candidates the idle card named;
    /// `order` sorts what is selected (`.suggested`: mission fit, then the
    /// estimate tier, then size and recency, all decided by the daemon).
    /// `nil` and `.queue` send neither, so the request is the one before
    /// these existed.
    func listPending(
        projectId: String?, filter: DaemonData.PendingFilter?, order: DaemonData.PendingOrder?
    ) async throws -> [DaemonData.QueueEntry]
    /// `list_kept`.
    func listKept() async throws -> [DaemonData.QueueEntry]

    // MARK: Previews
    //
    // THE RULE: a list of cards gets its previews from the scheduler, never
    // from `preview`. Each `preview` call runs a full read-parse-redact pass
    // on the caller's time with nothing bounding how many run at once, so a
    // 40-card queue calling it per card starts 40 passes. The scheduler
    // (`preview_request` / `preview_visible` / `preview_cancel`) bounds the
    // work, builds what is on screen first, and answers each card through
    // the `previewReady` event.
    //
    // - Queue and folder cards: `requestPreview` once per card, then wait
    //   for `previewReady`; `setVisiblePreviews` when a scroll settles;
    //   `cancelPreview` for a card that leaves the queue.
    // - The review sheet, for the ONE session the contributor opened:
    //   `preview`.

    /// `preview_request`: asks the scheduler for one card's preview and
    /// returns at once. A `ready`, `too_large` or `failed` answer came from
    /// cache and no event follows; `queued` or `running` means a
    /// `previewReady` event will carry the outcome.
    func requestPreview(entryId: String) async throws -> DaemonData.PreviewRequestOutcome
    /// `preview_visible`: replaces the set of entries on screen, which
    /// decides build ORDER, never membership. Returns how many ids the
    /// daemon now holds as visible.
    func setVisiblePreviews(entryIds: [String]) async throws -> Int
    /// `preview_cancel`: drops a scheduled preview. `dropped == false` is a
    /// defined no-op, not an error.
    func cancelPreview(entryId: String) async throws -> DaemonData.PreviewCancelResult
    /// `preview`: the summary for the review sheet's one opened session,
    /// with `title` and `unsure_spans`. Blocking and unbounded on the
    /// daemon: NEVER call it per card (see the rule above).
    func preview(entryId: String) async throws -> DaemonData.PreviewSummary
    /// `preview_unsure_spans` for the body `preview_body` returned. Served
    /// by `tc_call` like every other method here: `tc_call` answers through
    /// the async dispatcher (`ipc::handle_local`), which serves it.
    func previewUnsureSpans(entryId: String, bodyDigest: String) async throws -> DaemonData.UnsureSpans

    // MARK: Queue actions

    /// `approve` for one entry, with the contributor's verdict and, for a
    /// `partly` or `failed` verdict, their written correction (R7). `nil`
    /// omits the parameter, which is "no answer", never `null` or `""`
    /// (those are refused as `outcome-invalid`). A correction without a
    /// `partly` / `failed` verdict is refused as `correction-needs-outcome`,
    /// one over the daemon's length cap as `correction-too-long`; either
    /// refusal approves nothing.
    ///
    /// Returns the daemon's whole reply (`ApproveResponse`: `approved`,
    /// `flagged`, `redactions`, `skipped`, the hold) when the entry was
    /// approved, so the screen can say "scrubbing removed N, M flagged".
    /// When the daemon answers OK but approved nothing -- the entry is
    /// `not-pending`, `not-enrolled`, too large, `witness-review-stale`, ...
    /// -- this THROWS `DaemonDataError.notApproved(reasonLabel:)` with the
    /// `skipped` row's label, so a skip can never be drawn as success.
    func approve(entryId: String, verdict: ContributorVerdict?, correction: String?) async throws -> ApproveResponse
    /// `approve` with `project_id`: Contribute for a whole folder (R6).
    ///
    /// A group call means "every pending session here that can go", so it
    /// does NOT throw when it approves nothing: the reply says what became
    /// of the rest -- `skipped[]` per entry, `excludedHeld` (held for a
    /// person's review) and `excludedIneligible` (cannot be contributed),
    /// neither of which is part of `skipped`. An id the daemon does not
    /// know is refused with `project-id-unrecognized`. A `verdict` is the
    /// opt-in "Submit all as" answer, sent as `outcome`; `nil` sends none.
    /// A `filter` narrows the folder to what `listPending` lists under the
    /// same filter, so Submit under the idle filter sends what is shown;
    /// `nil` sends none and the whole folder is meant.
    func approveFolder(
        projectId: String, verdict: ContributorVerdict?, filter: DaemonData.PendingFilter?
    ) async throws -> ApproveResponse
    /// `cancel` for one entry: Undo inside the hold window (R7). Only an
    /// entry still `approved` can be cancelled, which the hold guarantees
    /// until it ends; any other is refused with `not-cancelable`. The entry
    /// goes back to waiting, and its verdict and correction go with the
    /// approval.
    func cancel(entryId: String) async throws
    /// `cancel` with `project_id`: Undo for a folder approve. Returns how
    /// many approved entries went back to waiting; `0` is a real answer
    /// (nothing in that folder was still cancelable), not an error.
    func cancelFolder(projectId: String) async throws -> Int
    /// `keep`: Keep on this Mac.
    func keep(entryId: String) async throws -> DaemonData.KeepResult
    /// `undo_keep`.
    func undoKeep(entryId: String) async throws -> DaemonData.KeepResult
    /// `dismiss`: permanent.
    func dismiss(entryId: String) async throws

    // MARK: Nudges

    /// `nudge_opened {kind}`: sent only from a suggestion's own action
    /// (Review, See history, the panel row), never for being shown. For
    /// `verdicts_landed` it acknowledges the news.
    func nudgeOpened(_ kind: NudgeSurface.Kind) async throws
    /// `nudge_decline {kind}`: the in-app "Not now". `verdicts_landed` is
    /// refused by the daemon (`nudge-kind-not-declinable`).
    func nudgeDecline(_ kind: NudgeSurface.Kind) async throws
    /// `set_suggestions_enabled {on}`: the cards, the panel row and the mark.
    func setSuggestionsEnabled(_ on: Bool) async throws
    /// `set_menu_bar_mark_enabled {on}`: the news ring and the idle halo.
    func setMenuBarMarkEnabled(_ on: Bool) async throws
    /// `set_notifications_enabled {on}`: "Notifications from Trace Commons".
    func setNotificationsEnabled(_ on: Bool) async throws
    /// `set_notify_kind {kind, on}`. Answering `verdicts_landed` or
    /// `idle_sessions` either way also ends that kind's one-time offer.
    func setNotifyKind(_ kind: String, on: Bool) async throws
    /// `set_settings {<kind>_offer_pending: false}`: a one-time offer
    /// dismissed without turning the kind on. Only `verdicts_landed` and
    /// `idle_sessions` have one; any other kind throws and sends nothing.
    func dismissNotifyOffer(kind: String) async throws

    // MARK: Projects and tools

    /// `list_projects`.
    func listProjects() async throws -> DaemonData.ProjectList
    /// `set_project_mode`. `includeBacklog` only with `.autoUpload`.
    func setProjectMode(projectId: String, mode: ProjectMode, includeBacklog: Bool?) async throws
        -> DaemonData.ProjectModeResult
    /// `set_contribution_override`: the menu-bar pill's global override
    /// (#1173, #1208). Per-folder modes are never written.
    ///
    /// `confirm` is sent only with `.autoUpload`, and only as `true`: it says
    /// the contributor confirmed the core's confirmation, which carries the
    /// arming disclosure. Automatic without it is refused
    /// (`bad_params` / `confirm-required`), and is refused without grant
    /// terms in force (`unavailable` / `arming-terms-unavailable`) before
    /// anything is recorded. Ask me and Never take no confirmation on the
    /// wire. Read `status` afterwards; the pill never shows a local guess.
    func setContributionOverride(mode: ProjectMode, confirm: Bool) async throws
        -> DaemonData.ContributionOverrideResult
    /// `clear_contribution_override`: every folder back on its own mode.
    /// `cleared == false` is "none was in force", not an error.
    func clearContributionOverride() async throws -> DaemonData.ContributionOverrideClearResult
    /// `harness_list`.
    func harnessList() async throws -> HarnessList
    /// `set_settings` with `<tool>_source`: the tool switch (R6). `.off` is
    /// "I do not use this tool" (watch nothing, no fallback); `.watch` names
    /// the folder. `.undecided`, or `.watch` with an empty path, is not an
    /// answer: nothing is sent and this throws the daemon's own
    /// `settings-invalid-value` refusal. The reply's `<tool>_source_mode`
    /// reads back `off` / `watch` (`unset` is never drawn as off).
    func setSource(_ kind: SourceKind, _ choice: SourceChoice) async throws -> DaemonData.Settings

    // MARK: Settings

    /// `get_settings`.
    func settings() async throws -> DaemonData.Settings
    /// Z1.5, the Private AI switch: `get_settings`' `private_inference`,
    /// its offer marker and the listener's state.
    func privateAI() async throws -> DaemonData.PrivateAISwitch
    /// Z1.5, `set_settings` with `private_inference` and the offer marker.
    /// Answers what the daemon echoed; nothing is confirmed here, the caller
    /// confirms with the core's rule (`TCPrivateInference.writeConfirmed`).
    func setPrivateAI(on: Bool) async throws -> DaemonData.PrivateAISwitch
    /// `set_settings` with `scrub_check`.
    func setScrubCheck(_ mode: DaemonData.ScrubCheckMode) async throws -> DaemonData.Settings
    /// `set_settings` with `local_notifications`.
    func setLocalNotifications(_ on: Bool) async throws -> DaemonData.Settings
    /// `set_settings { insights_recap_card_enabled }`: the Insights weekly
    /// summary card's only switch (its "Turn off").
    func setInsightsRecapCard(_ on: Bool) async throws -> DaemonData.Settings
    /// `set_settings { insights_ledger_feed }`: whether Insights may read
    /// the proxy ledger for the glance and per-call tokens (owner decision D3, settled
    /// 2026-10-09: on by default, this turns it off). Draw only the confirmed value from the reply.
    func setInsightsLedgerFeed(_ on: Bool) async throws -> DaemonData.Settings
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

    // MARK: Insights

    /// `insights_week`: one local ISO week (`YYYY-Www`; `nil` is the current
    /// week) from the daemon's counter pass, Insights feed T (owner decision
    /// D4, open). Answers `enabled: false` while the setting is off; an older
    /// daemon refuses it as `unknown_method`. Either way, and on any failed
    /// read, the window shows the saved-imports feed instead, never both.
    func insightsWeek(isoWeek: String?) async throws -> DaemonData.InsightsWeek
    /// `insights_glance`: today's routed calls per tool from the proxy
    /// ledger (on by default; owner decision D3, settled 2026-10-09). `tzSeconds` is the local UTC offset
    /// (`TimeZone.current.secondsFromGMT()`). `enabled: false`, an older
    /// daemon's `unknown_method` and any failed read all mean no glance,
    /// never a zero and never a failed popover.
    func insightsGlance(tzSeconds: Int) async throws -> DaemonData.InsightsGlance

    // MARK: Network methods (C3, #1187)

    /// Z1.1, `inference_summary`: IronWire's upstream grouped summary;
    /// registry-priced cost is not billed spend.
    func networkInferenceSummary() async throws -> DaemonData.NetworkInferenceSummary
    /// Z1.2, `inference_call_proof`.
    func inferenceCallProof(callId: Int64) async throws -> DaemonData.InferenceProofDetail
    /// Z1.3, billed spend by model for the entire NEAR AI organization, not this Mac.
    func modelSpend() async throws -> DaemonData.ModelSpend
    /// Z1.5, `private_ai`: the Private AI switch's state with the core's
    /// disclosure.
    func networkPrivateAI() async throws -> DaemonData.NetworkPrivateAISwitch
    /// Z1.5, `set_private_ai`: turning Private AI on or off.
    /// `consent` is built only from the `NetworkPrivateAISwitch` whose
    /// disclosure the caller showed (`PrivateAIConsent(acknowledging:)`).
    /// Enabling with `nil` is refused before anything is sent; the daemon
    /// gets `confirmed: true` only when a consent is held. Turning off needs
    /// none.
    func setNetworkPrivateAI(on: Bool, consent: DaemonData.PrivateAIConsent?) async throws
        -> DaemonData.NetworkPrivateAISwitch
    /// Z2.2, one page of the mission catalogue (`MissionCatalogQuery`):
    /// at most `limit` entries (the daemon's default when `nil`), older than
    /// the mission id `before`. Pass a page's `catalogue.nextCursor` as
    /// `before` for the next; `nil` there means the last page.
    func networkMissionCatalogue(limit: Int?, before: String?) async throws -> DaemonData.NetworkMissionCatalogue
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

    // MARK: PROVISIONAL shapes the screens still read

    /// Z1.1, per-model summary. PROVISIONAL: the Inference screen's shape.
    /// The live client throws `notAvailableYet`; the real reply is
    /// `networkInferenceSummary()`.
    func inferenceSummary() async throws -> DaemonData.InferenceSummary
    /// Z2.2, the mission catalogue. PROVISIONAL: the Missions screen's
    /// shape. The live client throws `notAvailableYet`; the real reply is
    /// `networkMissionCatalogue(limit:before:)`.
    func missionCatalogue() async throws -> DaemonData.MissionCatalogue

    // MARK: Live updates

    /// The daemon's events, for screens that refresh live. Each call returns
    /// a fresh stream. On `.resyncRequired`, refetch `status` and `listPending`.
    ///
    /// A stream that finishes means the core is down (or the app is tearing
    /// down): an unreachable daemon ends it with no events at all, not with
    /// an empty snapshot. Draw a finished stream as core-down, never as
    /// "nothing to show".
    func events() -> AsyncStream<DaemonDataEvent>
}

extension DaemonDataClient {
    /// `list_pending` in queue order, every pending entry or one project's.
    public func listPending(projectId: String?) async throws -> [DaemonData.QueueEntry] {
        try await listPending(projectId: projectId, filter: nil, order: nil)
    }

    /// `approve` for one entry with no verdict.
    public func approve(entryId: String) async throws -> ApproveResponse {
        try await approve(entryId: entryId, verdict: nil, correction: nil)
    }

    /// `approveFolder` with no verdict.
    public func approveFolder(projectId: String) async throws -> ApproveResponse {
        try await approveFolder(projectId: projectId, verdict: nil)
    }

    /// `approveFolder` over the whole folder.
    public func approveFolder(projectId: String, verdict: ContributorVerdict?) async throws -> ApproveResponse {
        try await approveFolder(projectId: projectId, verdict: verdict, filter: nil)
    }

    /// The first page of the mission catalogue, at the daemon's default size.
    public func networkMissionCatalogue() async throws -> DaemonData.NetworkMissionCatalogue {
        try await networkMissionCatalogue(limit: nil, before: nil)
    }
}

/// What the event stream carries: the contract's events, plus a
/// provisional per-call event for the map pulse.
public enum DaemonDataEvent: Equatable, Sendable {
    /// Sent first on subscribe: the whole queue and status.
    case snapshot(pending: [DaemonData.QueueEntry], status: DaemonData.Status?)
    case queueChanged
    case statusChanged
    /// `digest_due`, with the contribution counts a notification is built
    /// from.
    case digestDue(DaemonData.DigestDue)
    /// `reengage_due`: one standalone re-engagement notification, in the
    /// daemon's words. Delivered only to a subscriber that accepts it.
    case reengageDue(DaemonData.ReengageDue)
    /// A scheduled preview reached a terminal state. Read `state`: only
    /// `.ready` carries a summary; `.tooLarge` and `.failed` are answers
    /// too, and must be drawn as what they are, never as ready.
    case previewReady(DaemonData.PreviewRequestOutcome)
    /// Fell behind: refetch `status` and `listPending`.
    case resyncRequired
    /// `inference_call_added`, so the map pulses per real call.
    // PROVISIONAL: event not on main yet; shape follows #1203's `call_added`.
    case inferenceCallAdded(DaemonData.InferenceCallAdded)
    /// `usage_changed`: the ledger read added a call this tick, while the
    /// Insights ledger feed is on. A pulse with no payload: re-read
    /// `insights_glance`.
    case usageChanged
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
            // Decoded in two halves, as `DaemonEventParser` does. A queue
            // this build cannot read is `.queueChanged` (refetch the list),
            // never `.resyncRequired`: a resubscribe would get the same
            // snapshot back and loop. A status it cannot read is unknown.
            struct Pending: Decodable { let pending: [DaemonData.QueueEntry] }
            struct StatusHalf: Decodable { let status: DaemonData.Status? }
            guard let pending = try? decoder.decode(Pending.self, from: payloadData) else {
                return .queueChanged
            }
            let status = (try? decoder.decode(StatusHalf.self, from: payloadData))?.status
            return .snapshot(pending: pending.pending, status: status)
        case "queue_changed": return .queueChanged
        case "status_changed": return .statusChanged
        case "digest_due":
            guard let digest = try? decoder.decode(DaemonData.DigestDue.self, from: payloadData) else {
                return .unknown(name)
            }
            return .digestDue(digest)
        case "reengage_due":
            guard let due = try? decoder.decode(DaemonData.ReengageDue.self, from: payloadData) else {
                return .unknown(name)
            }
            return .reengageDue(due)
        case "preview_ready":
            guard let outcome = try? decoder.decode(DaemonData.PreviewRequestOutcome.self, from: payloadData)
            else { return .unknown(name) }
            return .previewReady(outcome)
        case "resync_required", "lagged": return .resyncRequired
        case "inference_call_added":
            guard let call = try? decoder.decode(DaemonData.InferenceCallAdded.self, from: payloadData) else {
                return .unknown(name)
            }
            return .inferenceCallAdded(call)
        case "usage_changed": return .usageChanged
        default: return .unknown(name)
        }
    }
}
