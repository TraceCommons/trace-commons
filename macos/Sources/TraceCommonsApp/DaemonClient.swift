import Foundation
import TCBridge

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
    func previewSummary(entryID: String) throws -> PreviewSummary {
        let raw = try rawResult("preview", params: ["entry_id": entryID])
        return try DaemonDecoding.decoder().decode(PreviewSummary.self, from: raw)
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

    func consentOptions() throws -> [ConsentScope] {
        struct Wrapper: Decodable { let scopes: [ConsentScope] }
        return try call("consent_options", as: Wrapper.self).scopes
    }

    func listProjects() throws -> [ProjectRow] {
        struct Wrapper: Decodable { let projects: [ProjectRow] }
        return try call("list_projects", as: Wrapper.self).projects
    }

    func settings() throws -> DaemonSettingsView {
        try call("get_settings", as: DaemonSettingsView.self)
    }

    /// Counts by `reason_label` across entries that ARE on the queue. It does
    /// not explain sessions the watcher never queued, and the UI must not
    /// claim it does.
    func queueOutcomeCounts() throws -> [String: Int] {
        struct Wrapper: Decodable { let reasons: [String: Int] }
        return try call("queue_outcome_counts", as: Wrapper.self).reasons
    }

    // MARK: - Decide

    /// Approves exactly one entry. There is no bulk path in this shell: the
    /// contract allows `all: true`, and the product deliberately does not
    /// offer it -- approval follows a preview, one session at a time.
    @discardableResult
    func approve(entryID: String) throws -> Int {
        struct Wrapper: Decodable { let approved: Int }
        return try call("approve", params: ["entry_id": entryID], as: Wrapper.self).approved
    }

    /// Returns an approved entry to `pending`. This is what the five-second
    /// undo is built on.
    func cancel(entryID: String) throws {
        _ = try rawResult("cancel", params: ["entry_id": entryID])
    }

    func dismiss(entryID: String) throws {
        _ = try rawResult("dismiss", params: ["entry_id": entryID])
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
        default:
            return .unknown(name)
        }
    }
}
