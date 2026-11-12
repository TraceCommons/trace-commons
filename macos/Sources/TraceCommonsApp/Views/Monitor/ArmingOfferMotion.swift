import SwiftUI
import TCDesign
import TCShellCore

/// Where the arming offer's Enable is: sent to the daemon, then armed.
enum ArmingOfferStep: Equatable, Sendable {
    case arming, armed
}

/// How the arming offer answers its Enable (owner, 2026-11-12). Enable is
/// locked while the daemon arms the project; once it has, the card's bolt
/// turns to a check with a pulse, and a beat later the card fades and the
/// list below slides up. Before this the card vanished on the daemon's
/// answer with nothing to say it had worked. No words are added: the
/// button keeps the core's label and the change is drawn, not said.
enum ArmingOfferMotion {
    /// How long the armed card stays before it leaves.
    static let hold: Duration = .milliseconds(900)

    /// The mark's symbol: the offer's bolt, a check once armed.
    static func symbol(_ step: ArmingOfferStep?) -> String {
        step == .armed ? "checkmark" : "bolt"
    }

    /// The mark's tint: the accent, the "on" status colour once armed.
    static func tint(_ step: ArmingOfferStep?) -> Color {
        step == .armed ? GlassStatus.on.color : GlassColor.accentText
    }

    /// Enable is pressed once: locked from the press until the card goes.
    static func enableLocked(_ step: ArmingOfferStep?) -> Bool {
        step != nil
    }

    /// The card's exit; none under Reduce Motion, where it simply goes.
    static func exit(reduceMotion: Bool) -> Animation? {
        reduceMotion ? nil : .spring(response: 0.42, dampingFraction: 0.9)
    }

    /// The mark's change from bolt to check.
    static let markChange: Animation = .spring(response: 0.32, dampingFraction: 0.7)

    /// The offer after a daemon refresh: the card the person pressed stays
    /// until its own exit, so a refresh that already reads the project as
    /// armed does not pull it from under the check.
    static func offerAfterRefresh(
        current: ArmingOffer?, answered: ArmingOffer?, step: ArmingOfferStep?
    ) -> ArmingOffer? {
        step == nil ? answered : current
    }
}

/// A ring that spreads from the mark once the project is armed. Never
/// under Reduce Motion.
struct ArmedPulse: View {
    let armed: Bool
    let tint: Color
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var spread = false

    var body: some View {
        Circle()
            .strokeBorder(tint, lineWidth: 2)
            .scaleEffect(spread ? 1.9 : 1)
            .opacity(armed && !reduceMotion && !spread ? 0.8 : 0)
            .allowsHitTesting(false)
            .accessibilityHidden(true)
            .onChange(of: armed) { _, now in
                guard now, !reduceMotion else { return }
                withAnimation(.easeOut(duration: 0.7)) { spread = true }
            }
    }
}
