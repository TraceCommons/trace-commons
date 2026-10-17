import Combine
import CoreGraphics
import Foundation
import TCShellCore

/// A trace the Traces map shows moving (owner, 2026-10-16): one that began
/// to leave this Mac for the commons, or one kept here. Built only from
/// what the core reported (`MapTransferDetector`), never from a guess.
struct MapTransfer: Equatable, Identifiable {
    enum Action: Equatable { case contribute, keep }

    /// The History row or the queue entry.
    let id: String
    /// Its folder (`project_id`) and its tool (the adapter's source name).
    let folder: String
    let tool: String
    let action: Action
    /// The time the details card gives: when a contribution was sent, or
    /// when a kept trace started; nil when unrecorded.
    var at: Date? = nil
}

/// Finds transfers between two reads. `list_pending` lists only what
/// waits, so a contribution is found where it lands: a row History gained,
/// sent after everything the earlier read held (a page further back is
/// not new). That is a person's Contribute and an armed upload alike. A
/// keep is the entry the window last kept, found among the traces it was
/// listed with. The first read finds none, so what had already left
/// before the window opened never plays.
enum MapTransferDetector {
    static func contributions(
        from old: [DaemonData.HistoryRow]?, to new: [DaemonData.HistoryRow]?
    ) -> [MapTransfer] {
        guard let old, let new else { return [] }
        let seen = Set(old.map(\.submissionId))
        let newest = old.compactMap(\.submittedAt).max() ?? .distantPast
        return new
            .compactMap { row -> MapTransfer? in
                guard !seen.contains(row.submissionId), let sent = row.submittedAt, sent > newest,
                      let folder = row.projectId, let tool = row.source else { return nil }
                return MapTransfer(id: row.submissionId, folder: folder, tool: tool, action: .contribute, at: sent)
            }
            .sorted { ($0.at ?? .distantPast) < ($1.at ?? .distantPast) }
    }

    /// The entry the window has just kept; nil when it is not among
    /// `sessions` to be found.
    static func kept(_ entryId: String?, in sessions: [DaemonData.QueueEntry]) -> MapTransfer? {
        guard let entryId, let entry = sessions.first(where: { $0.entryId == entryId }) else { return nil }
        return MapTransfer(id: entry.entryId, folder: entry.projectId, tool: entry.source,
                           action: .keep, at: entry.startedAt)
    }
}

/// One run of transfers the map plays together (owner, 2026-10-16), all
/// of one action. Each trace in turn rides its folder's line into the tool
/// group; its tool's coin, shuffled to the front as it set off, grows when
/// it arrives and a light runs once counter-clockwise round the coin's rim
/// from its base; then the coin hands it to the line out, and at the pad
/// it vanishes into the waiting rocket (or, kept, into the pad) with a
/// pulse of light. A contribution's rocket lifts off only once every trace
/// has arrived: slowly, then faster, to the library's station.
///
/// Everything here is a function of the seconds since the run began, so
/// the map draws any moment of it and the timings are tested.
struct MapTransferRun: Equatable {
    let transfers: [MapTransfer]
    let start: Date

    /// The seconds between traces setting off, and each leg's length.
    static let step = 1.05
    static let inbound = 0.70
    static let orbit = 0.45
    static let outbound = 0.70
    /// The pause on the pad before lift-off, the flight, and what follows.
    static let settle = 0.5
    static let flight = 2.6
    static let linger = 1.3
    static let keptLinger = 1.6
    /// The pulse a trace gives off as it vanishes at the pad.
    static let pulse = 0.9

    var action: MapTransfer.Action { transfers.first?.action ?? .contribute }

    /// Where one trace is, `t` seconds into the run.
    enum Leg: Equatable {
        case waiting
        /// Riding its folder's line, 0...1.
        case inbound(Double)
        /// In its coin while the rim light goes round, 0...1.
        case orbit(Double)
        /// Riding the line out to the pad, 0...1.
        case outbound(Double)
        /// Gone into the rocket or the pad, this many seconds ago.
        case arrived(Double)
    }

    func leg(_ index: Int, at t: Double) -> Leg {
        var local = t - Double(index) * Self.step
        if local < 0 { return .waiting }
        if local < Self.inbound { return .inbound(local / Self.inbound) }
        local -= Self.inbound
        if local < Self.orbit { return .orbit(local / Self.orbit) }
        local -= Self.orbit
        if local < Self.outbound { return .outbound(local / Self.outbound) }
        return .arrived(local - Self.outbound)
    }

    /// When the last trace reaches the pad.
    var allArrived: Double {
        Double(max(transfers.count - 1, 0)) * Self.step + Self.inbound + Self.orbit + Self.outbound
    }

    /// When the rocket lifts off (a contribution's run only).
    var liftOff: Double { allArrived + Self.settle }

    /// When the run is over.
    var end: Double {
        action == .contribute ? liftOff + Self.flight + Self.linger : allArrived + Self.keptLinger
    }

    /// How many traces have reached the pad by `t`.
    func arrived(at t: Double) -> Int {
        transfers.indices.filter { if case .arrived = leg($0, at: t) { true } else { false } }.count
    }

    /// The pile's order at `t`, front first: `base` with each trace's tool
    /// moved to the front as that trace set off.
    func order(_ base: [String], at t: Double) -> [String] {
        var order = base
        for (index, transfer) in transfers.enumerated() where leg(index, at: t) != .waiting {
            let id = "tool:" + transfer.tool
            guard let at = order.firstIndex(of: id) else { continue }
            order.remove(at: at)
            order.insert(id, at: 0)
        }
        return order
    }

    /// The tools whose coin is grown at `t` (a trace is in it), with how
    /// far round the rim light has gone.
    func orbits(at t: Double) -> [String: Double] {
        var orbits: [String: Double] = [:]
        for (index, transfer) in transfers.enumerated() {
            if case .orbit(let progress) = leg(index, at: t) { orbits["tool:" + transfer.tool] = progress }
        }
        return orbits
    }

    /// The trace the details card describes at `t`: the latest to set off
    /// that has not yet reached the pad.
    func current(at t: Double) -> MapTransfer? {
        transfers.indices.reversed().first { index in
            switch leg(index, at: t) {
            case .waiting, .arrived: false
            default: true
            }
        }.map { transfers[$0] }
    }

    /// Whether any trace is still moving through the map at `t`.
    func moving(at t: Double) -> Bool { arrived(at: t) < transfers.count }

    /// How far the rocket has flown, 0...1, or nil while it waits on the
    /// pad (a contribution's run) or there is no rocket (a kept run).
    func flight(at t: Double) -> Double? {
        guard action == .contribute, t >= liftOff else { return nil }
        return min(1, (t - liftOff) / Self.flight)
    }

    /// The share of the flight's path the rocket has covered: still for
    /// the ignition's first quarter, then slowly away and faster and
    /// faster, like a real launch.
    static func climb(_ progress: Double) -> Double {
        let ignition = 0.26
        guard progress > ignition else { return 0 }
        let x = (progress - ignition) / (1 - ignition)
        return pow(x, 2.4)
    }
}

/// Plays the runs the window hands it, one at a time (owner, 2026-10-16):
/// what arrives while a run plays waits for the next, and a run is all of
/// one action. Under Reduce Motion nothing is played; the counts the map
/// draws change at once.
@MainActor
final class MapTransferPlayer: ObservableObject {
    @Published private(set) var run: MapTransferRun?
    private var queue: [MapTransfer] = []
    private var played = Set<String>()
    private var timer: Task<Void, Never>?
    var reduceMotion = false

    func enqueue(_ transfers: [MapTransfer]) {
        guard !reduceMotion else { return }
        let fresh = transfers.filter { played.insert($0.id).inserted }
        guard !fresh.isEmpty else { return }
        queue += fresh
        if run == nil { next() }
    }

    private func next() {
        guard let first = queue.first else {
            run = nil
            return
        }
        let batch = queue.prefix { $0.action == first.action }
        queue.removeFirst(batch.count)
        let run = MapTransferRun(transfers: Array(batch), start: .now)
        self.run = run
        timer?.cancel()
        timer = Task { [weak self] in
            try? await Task.sleep(for: .seconds(run.end))
            guard !Task.isCancelled else { return }
            self?.next()
        }
    }
}
