import SwiftUI
import TCDesign
import TCShellCore

/// Welcome's choreography (Ron's 2026-10-21 canvas, rulings through
/// 2026-10-22), apart from the view so it can be tested: the N drawn like a
/// marker in one second, the glow behind it fading in as it draws, no
/// pulse, a light band crossing the mark and fully off it just after the
/// draw and then every 15 seconds, and the words rising one at a time.
enum WelcomeScreenLayout {
    static let drawDelay: Double = 0.2
    static let drawDuration: Double = 1
    static let shineFirst: Double = 1.5
    static let shineEvery: Double = 15
    static let shineCrossing: Double = 1.35
    static let wordDelays: [Double] = [1.05, 1.14, 1.23]
    static let bodyDelay: Double = 1.45
    static let buttonDelay: Double = 1.8
    static let riseDuration: Double = 0.76
    /// The hero title over the type scale's largest step (44 over 26).
    static let heroScale: CGFloat = 44.0 / 26

    /// How much of the mark is inked, `elapsed` seconds after Welcome opens.
    static func drawn(after elapsed: Double, reduceMotion: Bool) -> Double {
        guard !reduceMotion else { return 1 }
        return ease((elapsed - drawDelay) / drawDuration)
    }

    /// Where the shine's band is across the mark, or nil between crossings.
    static func shine(after elapsed: Double, reduceMotion: Bool) -> Double? {
        guard !reduceMotion, elapsed >= shineFirst else { return nil }
        let into = (elapsed - shineFirst).truncatingRemainder(dividingBy: shineEvery)
        guard into < shineCrossing else { return nil }
        return GlassMotionCurve.value(into / shineCrossing)
    }

    /// How far a line has risen into place (0 hidden and lowered, 1 in
    /// place), `elapsed` seconds after Welcome opens.
    static func risen(after elapsed: Double, delay: Double, reduceMotion: Bool) -> Double {
        guard !reduceMotion else { return 1 }
        return FirstRunStageLayout.burst(elapsed - delay, duration: riseDuration) * 0.98
            + (elapsed - delay >= riseDuration ? 0.02 : 0)
    }

    /// The glow behind the mark, fading in while it draws.
    static func glow(after elapsed: Double, reduceMotion: Bool) -> Double {
        guard !reduceMotion else { return 1 }
        return ease((elapsed - 0.15) / 1.1)
    }

    /// The marker's ease: slow in, quick through the middle, slow out.
    static func ease(_ t: Double) -> Double {
        let p = min(1, max(0, t))
        return p < 0.5 ? 4 * p * p * p : 1 - pow(-2 * p + 2, 3) / 2
    }

    /// The title's words, raised one at a time.
    static func words(_ title: String) -> [String] {
        title.split(separator: " ").map(String.init)
    }
}

/// The curve the step thread and the shine share, as a plain function of
/// progress: the token's ease (0.2, 0.8, 0.2, 1) sampled once for a value.
enum GlassMotionCurve {
    static func value(_ t: Double) -> Double {
        let p = min(1, max(0, t))
        // A cubic Bezier from (0,0) to (1,1) through the token's control
        // points, solved for x by Newton's method.
        let x1 = GlassTokens.Motion.easeX1, y1 = GlassTokens.Motion.easeY1
        let x2 = GlassTokens.Motion.easeX2, y2 = GlassTokens.Motion.easeY2
        func bezier(_ s: Double, _ a: Double, _ b: Double) -> Double {
            3 * (1 - s) * (1 - s) * s * a + 3 * (1 - s) * s * s * b + s * s * s
        }
        var s = p
        for _ in 0..<8 {
            let x = bezier(s, x1, x2) - p
            let dx = 3 * (1 - s) * (1 - s) * x1 + 6 * (1 - s) * s * (x2 - x1) + 3 * s * s * (1 - x2)
            guard abs(dx) > 1e-6 else { break }
            s = min(1, max(0, s - x / dx))
        }
        return bezier(s, y1, y2)
    }
}

/// Welcome, the first step: the NEAR mark drawn like a marker, "Welcome to
/// Near AI" rising a word at a time, Ron's line under it, and Get started.
/// No step thread and no Back: nothing comes before it.
struct WelcomeScreen: View {
    let copy: FirstRunCopy
    @ObservedObject var runner: FirstRunRunner
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var openedAt = Date()

    var body: some View {
        GlassAnimationTimeline(paused: reduceMotion) { date in
            let elapsed = date.timeIntervalSince(openedAt)
            VStack(spacing: 0) {
                Spacer(minLength: 0)
                FirstRunHub(
                    drawn: WelcomeScreenLayout.drawn(after: elapsed, reduceMotion: reduceMotion),
                    shine: WelcomeScreenLayout.shine(after: elapsed, reduceMotion: reduceMotion),
                    showsTip: true,
                    glow: WelcomeScreenLayout.glow(after: elapsed, reduceMotion: reduceMotion))
                    .firstRunActor(FirstRunStageLayout.hubActor)
                    .frame(width: 132, height: 132)
                    .padding(.bottom, 52)
                title(elapsed)
                    .padding(.bottom, GlassTokens.Space.s7)
                Text(copy.welcome.body)
                    .glassType(GlassTokens.TypeScale.title.weight(.regular))
                    .foregroundStyle(GlassColor.textSecondary)
                    .multilineTextAlignment(.center)
                    .modifier(Rise(amount: WelcomeScreenLayout.risen(
                        after: elapsed, delay: WelcomeScreenLayout.bodyDelay, reduceMotion: reduceMotion)))
                    .padding(.bottom, 38)
                Button(copy.welcome.start) {
                    runner.state = FirstRunNavigation.next(runner.state)
                }
                .buttonStyle(GlassButtonStyle(.primary, size: .stage))
                .modifier(Rise(amount: WelcomeScreenLayout.risen(
                    after: elapsed, delay: WelcomeScreenLayout.buttonDelay, reduceMotion: reduceMotion)))
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .padding(.horizontal, FirstRunFrameLayout.barInset)
        }
        .onAppear { openedAt = Date() }
    }

    private func title(_ elapsed: Double) -> some View {
        let words = WelcomeScreenLayout.words(copy.welcome.title)
        return HStack(spacing: 0) {
            ForEach(Array(words.enumerated()), id: \.offset) { index, word in
                let delay = WelcomeScreenLayout.wordDelays[min(index, WelcomeScreenLayout.wordDelays.count - 1)]
                Text(index < words.count - 1 ? word + " " : word)
                    .modifier(Rise(amount: WelcomeScreenLayout.risen(
                        after: elapsed, delay: delay, reduceMotion: reduceMotion)))
            }
        }
        // The canvas's 44pt hero is the largest step of the type scale
        // drawn larger, so it still follows the system text size; the frame
        // holds the room the larger line takes.
        .glassType(GlassTokens.TypeScale.number)
        .foregroundStyle(GlassColor.textPrimary)
        .scaleEffect(WelcomeScreenLayout.heroScale)
        .frame(height: GlassTokens.TypeScale.number.lineHeight * WelcomeScreenLayout.heroScale)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(copy.welcome.title)
        .accessibilityAddTraits(.isHeader)
    }
}

/// A line rising 18 points into place as it fades in.
private struct Rise: ViewModifier {
    let amount: Double

    func body(content: Content) -> some View {
        content
            .opacity(min(1, max(0, amount)))
            .offset(y: 18 * (1 - amount))
    }
}
