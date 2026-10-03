#if DEBUG
import Foundation
import Observation
import TCShellCore

/// The Home tab's and History's data (R9 of #1173), read through
/// `DaemonDataClient`.
///
/// Each read stands alone, as in `InferenceStore`: unreadable credit
/// leaves the history standing. A failed read keeps the last good value
/// under its failure notice, except the status and the tool routes: those
/// drive what Home says is happening now, so a failed read clears them and
/// Home says unknown rather than the last state.
@MainActor
@Observable
final class HomeStore {
    private(set) var status: DaemonData.Status?
    /// `tool_destinations`: which tools the core reads now.
    private(set) var destinations: DaemonData.ToolDestinations?
    /// `list_history`, newest first.
    private(set) var history: [DaemonData.HistoryRow]?
    private(set) var rollup: DaemonData.HistoryRollup?
    /// The commons' own credit figures. Its `unknown` halves are nil, never zero.
    private(set) var credit: DaemonData.CommonsCreditSummary?
    /// The last read that failed, by method; cleared when it next succeeds.
    private(set) var failures: [String: DaemonDataError] = [:]

    /// History rows read at once: the page, not the whole record.
    static let historyLimit = 100

    let client: any DaemonDataClient

    init(client: any DaemonDataClient) {
        self.client = client
    }

    /// Loads, then follows the event stream for as long as the calling task
    /// runs (a view's `.task`). History changes when the queue or the
    /// status does, so either rereads it all.
    func run() async {
        await load()
        for await event in client.events() {
            if Task.isCancelled { break }
            switch event {
            case .snapshot, .queueChanged, .statusChanged, .resyncRequired:
                await load()
            case .digestDue, .previewReady, .inferenceCallAdded, .unknown:
                break
            }
        }
    }

    func load() async {
        async let status: Void = loadStatus()
        async let history: Void = loadHistory()
        async let rollup: Void = loadRollup()
        async let credit: Void = loadCredit()
        async let destinations: Void = loadDestinations()
        _ = await (status, history, rollup, credit, destinations)
    }

    private func loadStatus() async {
        status = await read("status", { try await $0.status() })
    }

    private func loadDestinations() async {
        destinations = await read("tool_destinations", { try await $0.toolDestinations() })
    }

    private func loadHistory() async {
        if let value = await read("list_history", { try await $0.listHistory(limit: Self.historyLimit) }) {
            history = Self.newestFirst(value)
        }
    }

    private func loadRollup() async {
        if let value = await read("history_rollup", { try await $0.historyRollup() }) { rollup = value }
    }

    private func loadCredit() async {
        if let value = await read("commons_credit_summary", { try await $0.commonsCreditSummary() }) { credit = value }
    }

    /// Newest first; a row with no date sorts last rather than first.
    static func newestFirst(_ rows: [DaemonData.HistoryRow]) -> [DaemonData.HistoryRow] {
        rows.sorted { ($0.submittedAt ?? .distantPast) > ($1.submittedAt ?? .distantPast) }
    }

    private func read<T: Sendable>(
        _ method: String, _ call: @Sendable (any DaemonDataClient) async throws -> T
    ) async -> T? {
        do {
            let value = try await call(client)
            failures[method] = nil
            return value
        } catch let error as DaemonDataError {
            if case .notAvailableYet = error {
                failures[method] = nil
            } else {
                failures[method] = error
            }
            return nil
        } catch {
            failures[method] = .undecodable(method: method)
            return nil
        }
    }
}
#endif
