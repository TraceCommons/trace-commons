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
    /// The mission catalogue (R10): one list for the whole commons, the same
    /// request for every contributor (#1174 M1). PROVISIONAL (Zaki's C3): the
    /// live client throws `notAvailableYet`, which is not a failure to show.
    private(set) var missions: DaemonData.MissionCatalogue?
    /// The last read that failed, by method; cleared when it next succeeds.
    private(set) var failures: [String: DaemonDataError] = [:]

    /// History rows read at once: the page, not the whole record.
    static let historyLimit = 100

    /// The app's live client (`AppModel.daemonData`), attached by the
    /// window when the daemon starts; nil while it is not running.
    private(set) var client: (any DaemonDataClient)?
    /// Bumped by each attach; a read that started against an older client
    /// is dropped when it answers, so it never writes over the new one.
    private var generation = 0

    init(client: (any DaemonDataClient)?) {
        self.client = client
    }

    /// Follows a new client (or none): nothing read from the old one is
    /// drawn as current, so Home is loading until the new one is read.
    func attach(_ client: (any DaemonDataClient)?) {
        self.client = client
        generation += 1
        status = nil
        destinations = nil
        history = nil
        rollup = nil
        credit = nil
        missions = nil
        failures = [:]
    }

    /// Loads, then follows the event stream for as long as the calling task
    /// runs (a view's `.task`). History changes when the queue or the
    /// status does, so either rereads it all. With no client the core is
    /// down: every read says so.
    func run() async {
        await load()
        guard let client else { return }
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
        async let missions: Void = loadMissions()
        async let destinations: Void = loadDestinations()
        _ = await (status, history, rollup, credit, missions, destinations)
    }

    private func loadStatus() async {
        await read("status", { try await $0.status() }) { self.status = $0 }
    }

    private func loadDestinations() async {
        await read("tool_destinations", { try await $0.toolDestinations() }) { self.destinations = $0 }
    }

    private func loadHistory() async {
        await read("list_history", { try await $0.listHistory(limit: Self.historyLimit) }) {
            if let value = $0 { self.history = Self.newestFirst(value) }
        }
    }

    private func loadRollup() async {
        await read("history_rollup", { try await $0.historyRollup() }) { if let value = $0 { self.rollup = value } }
    }

    private func loadCredit() async {
        await read("commons_credit_summary", { try await $0.commonsCreditSummary() }) {
            if let value = $0 { self.credit = value }
        }
    }

    private func loadMissions() async {
        await read("mission_catalogue", { try await $0.missionCatalogue() }) {
            if let value = $0 { self.missions = value }
        }
    }

    /// Newest first; a row with no date sorts last rather than first.
    static func newestFirst(_ rows: [DaemonData.HistoryRow]) -> [DaemonData.HistoryRow] {
        rows.sorted { ($0.submittedAt ?? .distantPast) > ($1.submittedAt ?? .distantPast) }
    }

    /// One read, recording its failure by method, then `apply` with its
    /// value (nil when it failed or is not there). A read answered after
    /// another client was attached changes nothing.
    private func read<T: Sendable>(
        _ method: String, _ call: @Sendable (any DaemonDataClient) async throws -> T, apply: (T?) -> Void
    ) async {
        guard let client else {
            failures[method] = .unreachable
            apply(nil)
            return
        }
        let mine = generation
        let result: Result<T, DaemonDataError>
        do {
            result = .success(try await call(client))
        } catch {
            result = .failure(error as? DaemonDataError ?? .undecodable(method: method))
        }
        guard mine == generation else { return }
        switch result {
        case .success(let value):
            failures[method] = nil
            apply(value)
        case .failure(.notAvailableYet):
            failures[method] = nil
            apply(nil)
        case .failure(let error):
            failures[method] = error
            apply(nil)
        }
    }
}
#endif
