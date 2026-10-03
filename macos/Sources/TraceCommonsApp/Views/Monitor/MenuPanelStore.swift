#if DEBUG
import Foundation
import Observation
import TCDesign
import TCShellCore

/// The menu-bar popover's data (R13 of #1173), read through
/// `DaemonDataClient` like the other glass screens. The popover's actions
/// (pause, resume, Private AI off) stay on `AppModel`, which the shipping
/// menu already uses for them.
///
/// The client is the app's live one (`AppModel.daemonData`), attached by
/// the menu-bar label when the daemon starts and detached when it stops;
/// sample data reaches this store only from tests. With no client, or after
/// any read fails, the data is `stale` and drawn as unavailable rather than
/// as the last values.
@MainActor
@Observable
final class MenuPanelStore {
    private(set) var projects: [ProjectRow]?
    private(set) var pending: [DaemonData.QueueEntry] = []
    private(set) var kept: [DaemonData.QueueEntry] = []
    private(set) var history: [DaemonData.HistoryRow] = []
    private(set) var calls: [DaemonData.InferenceCall] = []
    /// True until a load has read everything, and again whenever a read
    /// fails, there is no client, or the client's event stream ends: what
    /// is held is no longer current.
    private(set) var stale = true

    private(set) var client: (any DaemonDataClient)?

    init(client: (any DaemonDataClient)?) {
        self.client = client
    }

    /// Follows a new client (or none): the old data is stale until the
    /// new one has been read.
    func attach(_ client: (any DaemonDataClient)?) {
        self.client = client
        stale = true
    }

    func run() async {
        await load()
        guard let client else { return }
        for await event in client.events() {
            if Task.isCancelled { break }
            switch event {
            case .snapshot, .queueChanged, .statusChanged, .resyncRequired, .inferenceCallAdded:
                await load()
            case .digestDue, .previewReady, .unknown:
                break
            }
        }
        // The stream ended: the daemon went away.
        if !Task.isCancelled { stale = true }
    }

    /// Reads everything. A failed read marks the data stale rather than
    /// keeping the last value as if it were current; a method the daemon
    /// does not have yet is not a failure.
    func load() async {
        guard let client else {
            stale = true
            return
        }
        var failed = false
        func read<T>(_ call: () async throws -> T) async -> T? {
            do { return try await call() } catch DaemonDataError.notAvailableYet {
                return nil
            } catch {
                failed = true
                return nil
            }
        }
        if let value = await read({ try await client.listProjects() }) { projects = value.projects }
        if let value = await read({ try await client.listPending(projectId: nil) }) { pending = value }
        if let value = await read({ try await client.listKept() }) { kept = value }
        if let value = await read({ try await client.listHistory(limit: HomeStore.historyLimit) }) { history = value }
        if let value = await read({ try await client.inferenceCalls(limit: 20, cursor: nil) }) {
            calls = value.readable ? value.calls : []
        }
        stale = failed
    }

    /// The graph's columns: contributed (shared) and kept, per day.
    var columns: [GlassDayColumn] {
        MenuPanelData.days(
            shared: history.compactMap(\.submittedAt),
            kept: kept.compactMap { $0.discoveredAt ?? $0.startedAt },
            ending: Date())
    }
}

/// The popover's rules, as pure functions so they are tested.
enum MenuPanelData {
    /// The roll-up of every listed folder's mode, for the mode pill.
    enum ModeRollup: Equatable {
        case ask, armed, never
        /// The folders do not all share one mode.
        case mixed
        /// No folder is listed yet.
        case none
    }

    static func rollup(_ modes: [ProjectMode]) -> ModeRollup {
        guard let first = modes.first else { return .none }
        guard modes.allSatisfy({ $0 == first }) else { return .mixed }
        switch first {
        case .ask: return .ask
        case .autoUpload: return .armed
        case .ignore: return .never
        }
    }

    /// The graph's span in days, and so its number of columns.
    static let days = 36

    /// One column per day, oldest first, ending today: how many sessions
    /// were contributed (up) and kept (down) that day. Counts per day, from
    /// the history and the kept list; the daemon reports no byte rates.
    static func days(
        shared: [Date], kept: [Date], ending now: Date, count: Int = days, calendar: Calendar = .current
    ) -> [GlassDayColumn] {
        let today = calendar.startOfDay(for: now)
        func index(_ date: Date) -> Int? {
            let day = calendar.startOfDay(for: date)
            guard let back = calendar.dateComponents([.day], from: day, to: today).day,
                  back >= 0, back < count else { return nil }
            return count - 1 - back
        }
        var up = Array(repeating: 0, count: count)
        var down = Array(repeating: 0, count: count)
        for date in shared { if let i = index(date) { up[i] += 1 } }
        for date in kept { if let i = index(date) { down[i] += 1 } }
        return (0 ..< count).map { GlassDayColumn(id: String($0), up: up[$0], down: down[$0]) }
    }

    /// Sessions worth a second look: nothing matched in them, or they were
    /// trimmed to fit (`QueueShieldState`'s attention, counted).
    static func flagged(_ pending: [DaemonData.QueueEntry]) -> Int {
        pending.filter { entry in
            let reasons = entry.secondLook ?? []
            return reasons.contains("nothing-matched") || reasons.contains("trimmed-to-fit")
        }.count
    }

    /// One recent-activity row.
    struct Recent: Identifiable, Equatable {
        enum Kind: Equatable { case waiting, contributed, call }
        let id: String
        let kind: Kind
        let at: Date
        let tool: GlassTool?
        let text: String
        /// Set for a call that left without proof.
        let trailing: String?
    }

    /// The newest few things that happened: sessions waiting, contributions
    /// with the core's word for their status, and calls that left for an
    /// outside model with their proof label.
    static func recent(
        pending: [DaemonData.QueueEntry], history: [DaemonData.HistoryRow], calls: [DaemonData.InferenceCall],
        statusLabel: (String) -> String?, limit: Int = 3
    ) -> [Recent] {
        var rows: [Recent] = []
        for entry in pending {
            guard let at = entry.startedAt ?? entry.discoveredAt else { continue }
            rows.append(Recent(
                id: "pending:\(entry.entryId)", kind: .waiting, at: at, tool: tool(entry.declaredSource ?? entry.source),
                text: "\(entry.projectLabel) · \(MonitorWords.waiting)", trailing: nil))
        }
        for row in history {
            guard let at = row.submittedAt, let status = row.status else { continue }
            rows.append(Recent(
                id: "history:\(row.submissionId)", kind: .contributed, at: at, tool: row.source.flatMap(tool),
                text: "\(row.projectLabel ?? "—") · \(statusLabel(status) ?? status)", trailing: nil))
        }
        for call in calls where call.route == "outside" {
            rows.append(Recent(
                id: "call:\(call.id)", kind: .call, at: call.at, tool: tool(call.tool),
                text: "\(InferenceTabView.toolName(call.tool)) · \(call.model)",
                trailing: call.proofLabel.isProof ? nil : InferenceWords.proof(call.proofLabel)))
        }
        return Array(rows.sorted { $0.at > $1.at }.prefix(limit))
    }

    static func tool(_ source: String) -> GlassTool? {
        SourceKind(rawValue: source).map(TracesTreeView.glassTool)
    }
}
#endif
