import Combine
import CoreGraphics
import Foundation
import TCShellCore

/// A trace the Traces map shows moving (owner, 2026-10-16): one that began
/// to leave this Mac for the commons, or one kept here; or one the person
/// dismissed, which only its card shows (owner, 2026-10-19). Built only
/// from what the core reported (`MapTransferDetector`), never from a guess.
struct MapTransfer: Equatable, Identifiable {
    enum Action: Equatable { case contribute, keep, dismiss }

    /// The History row or the queue entry.
    let id: String
    /// Its folder (`project_id`) and its tool (the adapter's source name).
    let folder: String
    let tool: String
    let action: Action
    /// The time the card gives: when a contribution was sent, or when a
    /// kept or dismissed trace started; nil when unrecorded.
    var at: Date? = nil
    /// The folder's name as the core gave it, for the card; nil when it
    /// gave none, and the card then looks the folder up on the map.
    var folderLabel: String? = nil
    /// How many traces the card stands for: more than one for a batch, a
    /// run of traces sent together (owner, 2026-10-19: one card for a
    /// batch, not a stack of them).
    var count: Int = 1
    /// For a batch, whether its traces came from more than one folder or
    /// tool; the card then names neither.
    var mixedFolders = false
    var mixedTools = false

    /// One card for a run of several traces: the first trace's folder,
    /// tool and time, and how many there are.
    static func batch(_ transfers: [MapTransfer]) -> MapTransfer? {
        guard let first = transfers.first else { return nil }
        guard transfers.count > 1 else { return first }
        return MapTransfer(
            id: "batch:" + first.id, folder: first.folder, tool: first.tool, action: first.action,
            at: first.at, folderLabel: first.folderLabel, count: transfers.count,
            mixedFolders: Set(transfers.map(\.folder)).count > 1,
            mixedTools: Set(transfers.map(\.tool)).count > 1)
    }
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
                return MapTransfer(id: row.submissionId, folder: folder, tool: tool, action: .contribute, at: sent,
                                   folderLabel: row.projectLabel)
            }
            .sorted { ($0.at ?? .distantPast) < ($1.at ?? .distantPast) }
    }

    /// The entry the window has just kept; nil when it is not among
    /// `sessions` to be found.
    static func kept(_ entryId: String?, in sessions: [DaemonData.QueueEntry]) -> MapTransfer? {
        decided(entryId, .keep, in: sessions)
    }

    /// The entry the window has just dismissed, likewise.
    static func dismissed(_ entryId: String?, in sessions: [DaemonData.QueueEntry]) -> MapTransfer? {
        decided(entryId, .dismiss, in: sessions)
    }

    private static func decided(
        _ entryId: String?, _ action: MapTransfer.Action, in sessions: [DaemonData.QueueEntry]
    ) -> MapTransfer? {
        guard let entryId, let entry = sessions.first(where: { $0.entryId == entryId }) else { return nil }
        return MapTransfer(id: entry.entryId, folder: entry.projectId, tool: entry.source,
                           action: action, at: entry.startedAt, folderLabel: entry.projectLabel)
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
/// On the standard map (`TracesMapStyle.standard`, owner, 2026-10-19) there
/// is no rocket: each contributed trace goes into the laptop and rises on
/// its own from the laptop's screen to the library's orb.
///
/// Everything here is a function of the seconds since the run began, so
/// the map draws any moment of it and the timings are tested.
struct MapTransferRun: Equatable {
    let transfers: [MapTransfer]
    let start: Date
    var style: TracesMapStyle = .launch

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
    /// A contributed trace's rise from the laptop to the orb, on the
    /// standard map.
    static let ascent = 1.1

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
        /// Rising from the laptop to the library's orb, 0...1 (the
        /// standard map's contributions only).
        case ascent(Double)
        /// Gone into the rocket, the pad or the orb, this many seconds ago.
        case arrived(Double)
    }

    /// Whether each contributed trace rises to the orb on its own.
    var rises: Bool { style == .standard && action == .contribute }

    func leg(_ index: Int, at t: Double) -> Leg {
        var local = t - Double(index) * Self.step
        if local < 0 { return .waiting }
        if local < Self.inbound { return .inbound(local / Self.inbound) }
        local -= Self.inbound
        if local < Self.orbit { return .orbit(local / Self.orbit) }
        local -= Self.orbit
        if local < Self.outbound { return .outbound(local / Self.outbound) }
        local -= Self.outbound
        if rises {
            if local < Self.ascent { return .ascent(local / Self.ascent) }
            local -= Self.ascent
        }
        return .arrived(local)
    }

    /// When the last trace reaches the pad, or on the standard map the orb.
    var allArrived: Double {
        let legs: Double = Self.inbound + Self.orbit + Self.outbound + (rises ? Self.ascent : 0)
        return Double(max(transfers.count - 1, 0)) * Self.step + legs
    }

    /// When the rocket lifts off (a contribution's run on the launch map).
    var liftOff: Double { allArrived + Self.settle }

    /// When the run is over.
    var end: Double {
        guard action == .contribute else { return allArrived + Self.keptLinger }
        return rises ? allArrived + Self.linger : liftOff + Self.flight + Self.linger
    }

    /// How many traces have arrived by `t`.
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
    /// pad (a contribution's run) or there is no rocket (a kept run, or the
    /// standard map).
    func flight(at t: Double) -> Double? {
        guard action == .contribute, !rises, t >= liftOff else { return nil }
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
/// one action. `cards` are the traces the cards describe (owner,
/// 2026-10-19: floating at the foot of the Traces map and over the top of
/// the Traces tab), oldest first, stacked with the newest in front: each
/// trace of a run joins as it sets off and stays `hold` seconds past its
/// run's end; a dismissed trace joins at once and stays `hold` seconds.
/// Each card's X closes that card alone. Under Reduce Motion the runs still
/// play, so the cards still show; the map draws them still.
@MainActor
final class MapTransferPlayer: ObservableObject {
    @Published private(set) var run: MapTransferRun?
    @Published private(set) var cards: [MapTransfer] = []
    private var queue: [MapTransfer] = []
    private var played = Set<String>()
    private var timer: Task<Void, Never>?
    private var expiries: [String: Task<Void, Never>] = [:]
    /// The map's style for the runs still to play; one already playing
    /// keeps its own.
    var style: TracesMapStyle = .launch

    /// How long a card stays after its run ends, or after a dismissal.
    static let hold: Double = 5
    /// The most cards kept in the stack; the oldest go first past it.
    static let stackLimit = 6

    /// The card in front of the stack, the newest.
    var front: MapTransfer? { cards.last }

    func enqueue(_ transfers: [MapTransfer]) {
        let fresh = transfers.filter { $0.action != .dismiss && played.insert($0.id).inserted }
        guard !fresh.isEmpty else { return }
        queue += fresh
        if run == nil { next() }
    }

    /// A card's X: that card goes now; the rest of the stack, and a run
    /// still playing on the map, carry on.
    func close(_ id: String) {
        expiries.removeValue(forKey: id)?.cancel()
        cards.removeAll { $0.id == id }
    }

    /// A dismissed trace: nothing moves on the map, its card joins the
    /// stack.
    func announce(_ transfer: MapTransfer) {
        guard played.insert(transfer.id).inserted else { return }
        add(transfer, for: Self.hold)
    }

    private func next() {
        guard let first = queue.first else {
            run = nil
            return
        }
        let batch = queue.prefix { $0.action == first.action }
        queue.removeFirst(batch.count)
        let run = MapTransferRun(transfers: Array(batch), start: .now, style: style)
        self.run = run
        feed(run)
        timer?.cancel()
        timer = Task { [weak self] in
            try? await Task.sleep(for: .seconds(run.end))
            guard !Task.isCancelled else { return }
            self?.next()
        }
    }

    /// Adds `run` to the stack as it sets off: one card for its trace, or
    /// one card for the whole batch when several go together (owner,
    /// 2026-10-19: a stack of one card per trace was too noisy). The map
    /// still moves each trace in turn.
    private func feed(_ run: MapTransferRun) {
        let until: Double = run.end + Self.hold
        if let card = MapTransfer.batch(run.transfers) { add(card, for: until) }
    }

    /// Puts `transfer` in front of the stack for `seconds`.
    private func add(_ transfer: MapTransfer, for seconds: Double) {
        cards.removeAll { $0.id == transfer.id }
        cards.append(transfer)
        while cards.count > Self.stackLimit {
            let oldest = cards.removeFirst()
            expiries.removeValue(forKey: oldest.id)?.cancel()
        }
        let id = transfer.id
        expiries[id]?.cancel()
        expiries[id] = Task { [weak self] in
            try? await Task.sleep(for: .seconds(max(0, seconds)))
            guard !Task.isCancelled else { return }
            self?.cards.removeAll { $0.id == id }
            self?.expiries[id] = nil
        }
    }
}
