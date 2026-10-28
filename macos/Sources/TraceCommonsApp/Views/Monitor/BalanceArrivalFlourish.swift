import SwiftUI
import TCDesign
import TCShellCore

/// Marks NEAR AI credit arriving on the Balance tile: gold coins burst out
/// of the card's centre like a firework while the figure counts up from the
/// one last shown to the new one (owner, 2026-10-27, in place of the edge
/// that lit in the status green). Under Reduce Motion there is no count and
/// the coins are drawn still, spread round the centre, for a moment.
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

    /// How long the count runs, in frames of `frame`.
    static var frames: Int { 24 }
    static var frame: Duration { .milliseconds(50) }
    /// How long the still coins stay under Reduce Motion.
    static var hold: Duration { .milliseconds(1600) }

    var body: some View {
        content(countingFigure)
            .glassCoinBurst(start: burst, still: reduceMotion)
            .task(id: Run(received: status.receivedTotalNanos, remaining: status.remainingNanos)) {
                await run()
            }
    }

    /// What restarts the check: the counted total or the figure moving.
    private struct Run: Equatable {
        let received: Int64?
        let remaining: Int64?
    }

    private func run() async {
        let now = BalanceArrival.Reading(status)
        let effect = BalanceArrival.effect(shown: Self.shown(), now: now, reduceMotion: reduceMotion)
        if now.receivedNanos != nil { Self.remember(now) }
        switch effect {
        case .none:
            return
        case .highlight:
            await celebrate()
        case .countUp(let from, let to):
            // The coins burst as the count starts and fly while it runs.
            let ends = ContinuousClock.now + Self.burstLength(reduceMotion)
            burst = .now
            for step in BalanceArrival.countSteps(fromNanos: from, toNanos: to, frames: Self.frames) {
                guard !Task.isCancelled else { break }
                countingFigure = calls.amount(1, step, status.scale).flatMap { $0.isEmpty ? nil : $0 }
                try? await Task.sleep(for: Self.frame)
            }
            countingFigure = nil
            try? await Task.sleep(until: ends, clock: .continuous)
            burst = nil
        }
    }

    /// Burst the coins and let them fall away; still ones held a moment
    /// under Reduce Motion.
    private func celebrate() async {
        burst = .now
        try? await Task.sleep(for: Self.burstLength(reduceMotion))
        burst = nil
    }

    /// How long the coins are drawn: one burst, or `hold` still.
    private static func burstLength(_ reduceMotion: Bool) -> Duration {
        reduceMotion ? hold : .seconds(GlassCoinBurst.duration)
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
