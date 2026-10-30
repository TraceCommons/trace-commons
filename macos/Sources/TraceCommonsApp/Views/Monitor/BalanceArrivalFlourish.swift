import SwiftUI
import TCDesign
import TCShellCore

/// Marks NEAR AI credit arriving on the Balance tile: gold coins burst out
/// of the card's centre like a firework while the figure counts up from the
/// one last shown to the new one (owner, 2026-10-27, in place of the edge
/// that lit in the status green). Under Reduce Motion there is no count and
/// the coins are drawn still, spread round the centre, for a moment. The
/// tile is drawn above what follows it (`zIndex`, where it is placed), so
/// the coins fall over the cards below.
///
/// One self-contained view: `content` draws the tile, given the figure to
/// show while counting (nil otherwise).
///
/// Whether there is anything to mark is `BalanceArrival.effect`'s call, from
/// the daemon's `received_total_nanos` -- never from the balance itself,
/// which spending also moves. What was last shown is kept in this app's
/// defaults, so credit that arrived while the app was closed is marked the
/// next time the tile is drawn.
struct BalanceArrivalFlourish<Content: View>: View {
    let status: BalanceStatus
    let calls: BalanceCalls
    @ViewBuilder let content: (_ countingFigure: String?) -> Content

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var countingFigure: String?
    /// When the coins burst; nil while none fly.
    @State private var burst: Date?
    /// The count under way, kept apart from the balance check so that a
    /// refresh mid-count does not cancel it: it runs on to the figure it
    /// was counting toward.
    @State private var count: Count?

    /// How long the count runs, in frames of `frame`.
    static var frames: Int { 24 }
    static var frame: Duration { .milliseconds(50) }
    /// How long the still coins stay under Reduce Motion.
    static var hold: Duration { .milliseconds(1600) }

    /// How long the coins stay: their flight, or under Reduce Motion the
    /// moment they are held still.
    static func coinsStay(reduceMotion: Bool) -> Duration {
        reduceMotion ? hold : .seconds(GlassCoinBurst.duration)
    }

    var body: some View {
        content(countingFigure)
            .glassCoinBurst(start: burst, still: reduceMotion)
            .task(id: Run(received: status.receivedTotalNanos, remaining: status.remainingNanos)) {
                check()
            }
            // The coins clear once their time has passed. A refresh does
            // not touch `burst`, so it leaves them to finish; a newer burst
            // restarts the wait; the tile going away ends it.
            .task(id: burst) {
                guard burst != nil else { return }
                try? await Task.sleep(for: Self.coinsStay(reduceMotion: reduceMotion))
                guard !Task.isCancelled else { return }
                burst = nil
            }
            .task(id: count) {
                guard let count else { return }
                for step in BalanceArrival.countSteps(fromNanos: count.from, toNanos: count.to, frames: Self.frames) {
                    countingFigure = calls.amount(1, step, status.scale).flatMap { $0.isEmpty ? nil : $0 }
                    try? await Task.sleep(for: Self.frame)
                    guard !Task.isCancelled else { return }
                }
                countingFigure = nil
                self.count = nil
            }
    }

    /// What restarts the check: the counted total or the figure moving.
    private struct Run: Equatable {
        let received: Int64?
        let remaining: Int64?
    }

    /// A count from one figure to another.
    private struct Count: Equatable {
        let from: Int64
        let to: Int64
    }

    private func check() {
        let now = BalanceArrival.Reading(status)
        let effect = BalanceArrival.effect(shown: Self.shown(), now: now, reduceMotion: reduceMotion)
        if now.receivedNanos != nil { Self.remember(now) }
        switch effect {
        case .none:
            return
        case .highlight:
            burst = Date()
        case .countUp(let from, let to):
            // The coins burst as the count starts and fly while it runs.
            // Never under Reduce Motion, which is `.highlight`.
            burst = Date()
            count = Count(from: from, to: to)
        }
    }

    // MARK: - What was last shown

    private static var receivedKey: String { "BalanceArrivalShownReceivedNanos" }
    private static var remainingKey: String { "BalanceArrivalShownRemainingNanos" }

    private static func shown() -> BalanceArrival.Reading? {
        let defaults = UserDefaults.standard
        guard let received = defaults.string(forKey: receivedKey).flatMap(Int64.init) else { return nil }
        return .init(receivedNanos: received,
                     remainingNanos: defaults.string(forKey: remainingKey).flatMap(Int64.init))
    }

    private static func remember(_ reading: BalanceArrival.Reading) {
        let defaults = UserDefaults.standard
        defaults.set(reading.receivedNanos.map(String.init), forKey: receivedKey)
        defaults.set(reading.remainingNanos.map(String.init), forKey: remainingKey)
    }
}
