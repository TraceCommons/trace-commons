import SwiftUI
import TCDesign
import TCShellCore

/// Marks NEAR AI credit arriving on the Balance tile: the figure counts up
/// from the one last shown to the new one while the card's edge lights in
/// the status green, then the light fades. Under Reduce Motion there is no
/// count and no fade: the edge is lit, still, for a moment.
///
/// Placeholder, pending Ron's design for credit settlement flourishes. It is
/// one self-contained view so his version can replace it whole: `content`
/// draws the tile, given the figure to show while counting (nil otherwise).
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
    @State private var lit = false

    /// How long the count runs, in frames of `frame`.
    static var frames: Int { 24 }
    static var frame: Duration { .milliseconds(50) }
    /// How long the edge stays lit once the count is done.
    static var hold: Duration { .milliseconds(1600) }

    var body: some View {
        content(countingFigure)
            .overlay {
                RoundedRectangle(cornerRadius: GlassTier.card.radius, style: .continuous)
                    .strokeBorder(GlassTokens.Color.statusOnText.color, lineWidth: 1.5)
                    .opacity(lit ? 0.9 : 0)
                    .allowsHitTesting(false)
                    .accessibilityHidden(true)
            }
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
            await light()
        case .countUp(let from, let to):
            withAnimation(GlassMotion.standard(reduceMotion)) { lit = true }
            for step in BalanceArrival.countSteps(fromNanos: from, toNanos: to, frames: Self.frames) {
                guard !Task.isCancelled else { break }
                countingFigure = calls.amount(1, step, status.scale).flatMap { $0.isEmpty ? nil : $0 }
                try? await Task.sleep(for: Self.frame)
            }
            countingFigure = nil
            await light()
        }
    }

    /// Light the edge, hold, and let it go: faded with motion, at once
    /// without.
    private func light() async {
        withAnimation(GlassMotion.standard(reduceMotion)) { lit = true }
        try? await Task.sleep(for: Self.hold)
        withAnimation(GlassMotion.standard(reduceMotion)) { lit = false }
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
