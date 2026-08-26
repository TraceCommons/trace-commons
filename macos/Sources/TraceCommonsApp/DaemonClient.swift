import Foundation
import TCBridge
import TCShellCore

/// The typed layer over the daemon's JSON. Every method here is one
/// `trace_commons.daemon.v1_1` call, in and out of Swift types.
///
/// Deliberately free of pointers: `TCBridge` owns those. Deliberately free
/// of UI: the views own that. Calls block (the ABI is synchronous), so
/// callers run them off the main actor.
final class DaemonClient {
    struct Failure: Error, CustomStringConvertible {
        let code: String
        let message: String
        var description: String { "\(code): \(message)" }
    }

    private let daemon: TCDaemon

    init(daemon: TCDaemon) {
        self.daemon = daemon
    }

    // MARK: - Read

    func status() throws -> DaemonStatus {
        try call("status", as: DaemonStatus.self)
    }

    func listPending() throws -> [QueueEntry] {
        try DaemonDecoding.pendingEntries(from: rawResult("list_pending"))
    }

    /// The socket `preview`: summary only. The redacted body is in-process
    /// only, via `openPreview` below.
    ///
    /// Kept for reference and for anything that still wants a blocking
    /// preview -- the queue's own cards no longer call this. It starts a
    /// full read-parse-redact-serialize pass on the connection's time with
    /// nothing bounding how many run at once, which is exactly the fan-out
    /// `requestPreview` below replaces; see the scheduler design doc.
    func previewSummary(entryID: String) throws -> PreviewSummary {
        let raw = try rawResult("preview", params: ["entry_id": entryID])
        return try DaemonDecoding.decoder().decode(PreviewSummary.self, from: raw)
    }

    /// The daemon's bounded preview scheduler: `preview_request`. Returns
    /// immediately, always -- `queued`/`running` mean a `preview_ready`
    /// event will follow; `ready` and `too_large` are answered here with no
    /// event to come. See `docs/contributor-daemon-ipc-v1_1.md`, "Scheduled
    /// previews".
    func requestPreview(entryID: String) throws -> PreviewRequestResult {
        let raw = try rawResult("preview_request", params: ["entry_id": entryID])
        return try DaemonDecoding.decoder().decode(PreviewRequestResult.self, from: raw)
    }

    /// Replaces the daemon's on-screen set wholesale, deciding preview
    /// *order* -- never membership. Intended to be sent once a scroll
    /// settles, not on every frame; see `AppModel`'s debounce.
    func setVisiblePreviews(entryIDs: [String]) throws {
        _ = try rawResult("preview_visible", params: ["entry_ids": entryIDs])
    }

    /// Drops a queued preview, or discards a running one's result. A
    /// `dropped: false` response is a defined no-op (already finished,
    /// never requested, already cancelled), not an error -- callers here
    /// discard it rather than throw on it.
    func cancelPreview(entryID: String) throws {
        _ = try rawResult("preview_cancel", params: ["entry_id": entryID])
    }

    func listHistory(limit: Int = 200) throws -> [HistoryRecord] {
        struct Wrapper: Decodable { let history: [HistoryRecord] }
        return try call("list_history", params: ["limit": limit], as: Wrapper.self).history
    }

    func historyRollup() throws -> HistoryRollup {
        try call("history_rollup", as: HistoryRollup.self)
    }

    func refreshHistory() throws {
        _ = try rawResult("refresh_history")
    }

    /// The local change log, newest first. `limit` defaults to 20 here
    /// rather than the contract's 50 to match the Linux shell, which asks
    /// for 20 on the same screen -- this is a "what did I change lately"
    /// surface, not an archive, and the daemon caps the log independently
    /// of what any client asks for.
    func listAudit(limit: Int = 20) throws -> [AuditEntry] {
        struct Wrapper: Decodable { let entries: [AuditEntry] }
        return try call("list_audit", params: ["limit": limit], as: Wrapper.self).entries
    }

    func consentOptions() throws -> [ConsentScope] {
        struct Wrapper: Decodable { let scopes: [ConsentScope] }
        return try call("consent_options", as: Wrapper.self).scopes
    }

    func listProjects() throws -> [ProjectRow] {
        struct Wrapper: Decodable { let projects: [ProjectRow] }
        return try call("list_projects", as: Wrapper.self).projects
    }

    /// Sets a project's mode, naming it by the opaque id `list_projects`
    /// gave us.
    ///
    /// The daemon accepts `project_id` OR `project_key`, and the id is the
    /// only one this app can honestly produce. A `project_key` is a full
    /// local path, and by the never-a-path rule neither `list_projects` nor
    /// `list_pending` ever puts one on the wire -- so this call used to send
    /// `project_label`, a final path segment, which the daemon refuses with
    /// `project-key-unrecognized` because it is not a key at all. The old
    /// doc comment said as much: the app had no source for a key that would
    /// satisfy the check. It had a source for an id all along.
    ///
    /// The daemon still accepts a `label` parameter from older clients and
    /// always ignores it -- labels are derived from the key inside the
    /// daemon, never accepted from a caller -- so this wrapper sends none.
    /// Returns how many waiting entries the daemon removed, which is only
    /// ever non-zero for `.ignore`. A daemon older than this field sends none
    /// and the answer is 0 — the caller must read that as "nothing to
    /// reconcile", never as "nothing was removed".
    @discardableResult
    func setProjectMode(projectID: String, mode: ProjectMode) throws -> Int {
        let data = try rawResult(
            "set_project_mode",
            params: ["project_id": projectID, "mode": mode.rawValue]
        )
        let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
        return (object?["purged"] as? Int) ?? 0
    }

    func settings() throws -> DaemonSettingsView {
        try call("get_settings", as: DaemonSettingsView.self)
    }

    /// Redeems `invite` for enrollment. Deliberately never sends
    /// `allowed_hosts` -- the contract's `enroll` entry says the daemon does
    /// not accept one from a socket caller at all (unlike the CLI's
    /// `--allowed-hosts` flag), so there is no parameter here to set one
    /// with.
    ///
    /// On failure this always throws the same `Failure` shape as every
    /// other call, but callers should not surface `failure.message` here:
    /// the contract only ever reports `unavailable` / `enroll-failed` for
    /// this method, on purpose, because the underlying issuer response can
    /// carry a URL or a response body that must never reach a UI. See
    /// `OnboardingConnectView`.
    func enroll(invite: String, scopes: [String] = []) throws -> EnrollResult {
        var params: [String: Any] = ["invite": invite]
        if !scopes.isEmpty { params["scopes"] = scopes }
        return try call("enroll", params: params, as: EnrollResult.self)
    }

    /// Records that the NEAR AI first-use notice was actually shown to the
    /// person in this UI. Callers must not call this without having shown
    /// that notice text first -- see "### `acknowledge_near_ai_notice`" in
    /// the contract: it is audited on the caller's unverified word.
    func acknowledgeNearAINotice() throws {
        _ = try rawResult("acknowledge_near_ai_notice")
    }

    /// Replaces the enrolled device's consent scopes. Local config write
    /// only -- no network I/O -- and requires an existing enrollment
    /// (`unavailable` / `not-logged-in` otherwise, per the contract). Used
    /// by the onboarding consent screen: `enroll` is always called with no
    /// scopes (floor scope only), and this call is what actually applies
    /// whatever the contributor ticked on `ConsentScopesView`, once they
    /// confirm it -- see "### `set_consent_scopes`" in the contract.
    @discardableResult
    func setConsentScopes(_ scopes: [String]) throws -> [String] {
        struct Wrapper: Decodable {
            let consentScopes: [String]
            enum CodingKeys: String, CodingKey { case consentScopes = "consent_scopes" }
        }
        let params: [String: Any] = scopes.isEmpty ? [:] : ["scopes": scopes]
        return try call("set_consent_scopes", params: params, as: Wrapper.self).consentScopes
    }

    /// Counts by `reason_label` across entries that ARE on the queue. It does
    /// not explain sessions the watcher never queued, and the UI must not
    /// claim it does.
    func queueOutcomeCounts() throws -> [String: Int] {
        struct Wrapper: Decodable { let reasons: [String: Int] }
        return try call("queue_outcome_counts", as: Wrapper.self).reasons
    }

    // MARK: - Decide

    /// Approves exactly one entry, by the id `list_pending` gave the caller.
    ///
    /// One-click submit means this no longer requires a preview: where none
    /// was pinned, the daemon builds and pins the envelope itself before
    /// approving (see `docs/superpowers/specs/2026-08-20-one-click-submit-design.md`,
    /// "What this changes"). The full response is returned rather than just
    /// `approved` -- `ApproveResponse.toast` is how a caller renders it.
    @discardableResult
    func approve(entryID: String) throws -> ApproveResponse {
        try call("approve", params: ["entry_id": entryID], as: ApproveResponse.self)
    }

    /// Approves every pending entry in one project, by the id `entry_value`
    /// publishes on each queue row (`project_id`) -- never `project_label`,
    /// which the daemon does not accept for this call and which is not
    /// guaranteed unique across projects in the first place. An id naming no
    /// project the daemon knows comes back as `bad_params` /
    /// `project-id-unrecognized`, thrown as a `Failure` like any other
    /// refusal -- a caller must not fold that into a skip.
    @discardableResult
    func approve(projectID: String) throws -> ApproveResponse {
        try call("approve", params: ["project_id": projectID], as: ApproveResponse.self)
    }

    /// Returns an approved entry to `pending`. This is what the five-second
    /// undo is built on.
    func cancel(entryID: String) throws {
        _ = try rawResult("cancel", params: ["entry_id": entryID])
    }

    func dismiss(entryID: String) throws {
        _ = try rawResult("dismiss", params: ["entry_id": entryID])
    }

    // MARK: - Withdraw

    /// Withdraws one already-submitted trace. Real network I/O, and the one
    /// irreversible thing this app can ask the server to do.
    ///
    /// Returns the tier the server applied (`distribution_reach`), which is
    /// the whole point of the call: withdrawal means different things
    /// depending on how far a trace travelled, and a shell that reports a
    /// generic "withdrawn" lets a contributor believe in an erasure they did
    /// not get. See `WithdrawalReach`.
    ///
    /// **This always fails today**, with `unavailable` /
    /// `account-session-required`, before any request leaves the machine.
    /// Withdrawal is authenticated by an account session -- deliberately not
    /// the device key that authenticates every other call here, so that
    /// withdrawal survives losing the device that submitted the trace -- and
    /// the daemon only ever holds a device key
    /// (`crates/trace-commons-contributor/src/daemon/withdraw.rs`). That
    /// label is distinct on purpose so a shell can say what is missing
    /// instead of showing a bare failure; callers must route it separately
    /// rather than letting it read as "we tried and something went wrong".
    ///
    /// There is no bulk wrapper here. The contract's `withdraw_bulk` reports
    /// only `withdrawn`/`failed` counts and never a per-trace tier, and it
    /// selects its targets from the local history cache's status, which can
    /// be stale -- so it cannot support an honest confirmation. See
    /// `docs/superpowers/plans/macos-withdrawal-ui-report.md`.
    func withdraw(submissionID: String) throws -> WithdrawalOutcome {
        try call("withdraw", params: ["submission_id": submissionID], as: WithdrawalOutcome.self)
    }

    // MARK: - Public profile

    /// The daemon's answer to all three profile calls. They share one shape
    /// on purpose, so a client parses one thing whichever call it made.
    ///
    /// `handlePersisted` is present on `set` and `clear` only, and it is
    /// **not** whether the call worked -- see `AppModel.claimHandle`. It
    /// reports whether the daemon managed to write its local copy of a
    /// profile the server has already accepted.
    ///
    /// `publicURL` is null by contract today: the daemon knows the origin
    /// it uploads to, not the origin the community site serves profiles
    /// from, and will not invent a link that does not resolve.
    struct PublicProfile: Decodable {
        let onRoster: Bool
        let handle: String?
        let bio: String?
        let publicSince: Date?
        let publicURL: String?
        let handlePersisted: Bool?

        enum CodingKeys: String, CodingKey {
            case onRoster = "on_roster"
            case handle
            case bio
            case publicSince = "public_since"
            case publicURL = "public_url"
            case handlePersisted = "handle_persisted"
        }
    }

    /// The locally cached profile. No network I/O: there is no
    /// `GET /v1/community/profile` for a contributor's own row, so this is
    /// what this device last published rather than what the roster holds
    /// this second. Fails with `not-logged-in` on an unenrolled device.
    func publicProfile() throws -> PublicProfile {
        try call("get_public_profile", as: PublicProfile.self)
    }

    /// Claims or updates the public handle. Real network I/O.
    ///
    /// `bio` is sent explicitly as null when there is none: the `PUT`
    /// replaces the whole profile, so "leave the bio alone" is not
    /// something the server can be asked for, and the daemon refuses an
    /// omitted `bio` rather than guessing which was meant.
    ///
    /// The handle is not validated here. The daemon and the server share
    /// one copy of those rules; a second copy in this app is how a handle
    /// this app accepts becomes one the server refuses.
    func setPublicProfile(handle: String, bio: String?) throws -> PublicProfile {
        try call(
            "set_public_profile",
            params: ["handle": handle, "bio": bio ?? NSNull()],
            as: PublicProfile.self
        )
    }

    /// Withdraws the public handle from the roster. Real network I/O.
    func clearPublicProfile() throws -> PublicProfile {
        try call("clear_public_profile", as: PublicProfile.self)
    }

    // MARK: - Pause

    struct PauseResult: Decodable {
        let paused: Bool
        let pausedUntil: Date?

        enum CodingKeys: String, CodingKey {
            case paused
            case pausedUntil = "paused_until"
        }
    }

    @discardableResult
    func pause(until: Date? = nil) throws -> PauseResult {
        var params: [String: Any] = [:]
        if let until {
            let formatter = ISO8601DateFormatter()
            formatter.formatOptions = [.withInternetDateTime]
            params["until"] = formatter.string(from: until)
        }
        return try call("pause", params: params, as: PauseResult.self)
    }

    func resume() throws {
        _ = try rawResult("resume")
    }

    // MARK: - Preview body (in-process only)

    /// Opens the redacted body for `entryID`. Blocks for the redaction pass.
    func openPreview(entryID: String) throws -> TCPreview {
        try daemon.openPreview(entryID: entryID)
    }

    // MARK: - Plumbing

    private func call<T: Decodable>(
        _ method: String,
        params: [String: Any] = [:],
        as type: T.Type
    ) throws -> T {
        let raw = try rawResult(method, params: params)
        return try DaemonDecoding.decoder().decode(T.self, from: raw)
    }

    /// Issues one call and unwraps `{"id":..,"result":{..}}`, turning
    /// `{"error":{"code":..,"message":..}}` into a thrown `Failure`. The
    /// message is always a fixed label by contract, so it is safe to show.
    private func rawResult(_ method: String, params: [String: Any] = [:]) throws -> Data {
        let paramsJSON: String
        if params.isEmpty {
            paramsJSON = "{}"
        } else {
            let data = try JSONSerialization.data(withJSONObject: params)
            paramsJSON = String(decoding: data, as: UTF8.self)
        }
        let response = daemon.call(method, params: paramsJSON)
        guard let data = response.data(using: .utf8),
              let object = try JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            throw Failure(code: "unavailable", message: "unparseable-response")
        }
        if let error = object["error"] as? [String: Any] {
            throw Failure(
                code: error["code"] as? String ?? "unavailable",
                message: error["message"] as? String ?? "unknown"
            )
        }
        guard let result = object["result"] else {
            throw Failure(code: "unavailable", message: "missing-result")
        }
        return try JSONSerialization.data(withJSONObject: result)
    }
}

// MARK: - Event parsing

enum DaemonEventParser {
    static func parse(_ json: String) -> DaemonEvent {
        guard let data = json.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let name = object["event"] as? String
        else { return .unknown("unparseable") }
        let payload = object["data"] as? [String: Any] ?? [:]
        switch name {
        case "snapshot":
            guard let data = try? JSONSerialization.data(withJSONObject: payload),
                  let pending = try? DaemonDecoding.pendingEntries(from: data)
            else { return .queueChanged }
            struct StatusWrapper: Decodable { let status: DaemonStatus }
            let status = (try? DaemonDecoding.decoder().decode(StatusWrapper.self, from: data))?.status
            return .snapshot(pending: pending, status: status ?? .unknown)
        case "queue_changed":
            return .queueChanged
        case "status_changed":
            return .statusChanged
        case "digest_due":
            return .digestDue(
                pending: payload["pending"] as? Int ?? 0,
                text: payload["text"] as? String ?? ""
            )
        case "resync_required":
            return .resyncRequired
        case "lagged":
            return .lagged(skipped: payload["skipped"] as? Int ?? 0)
        case "preview_ready":
            guard let payloadData = try? JSONSerialization.data(withJSONObject: payload),
                  let result = try? DaemonDecoding.decoder()
                      .decode(PreviewRequestResult.self, from: payloadData)
            else { return .unknown(name) }
            return .previewReady(result)
        default:
            return .unknown(name)
        }
    }
}
