import SwiftUI

/// The treatments for a card that should stand out from the cards around it,
/// as a promotion does (owner, 2026-10-11: Home's Missions card, explored as
/// an advert). Each sits on the card's own tier fill under a brand tint, so
/// the card's words keep the card's contrast; only the light around them
/// moves. Under Reduce Motion every treatment holds still on one frame.
///
/// For comparison, `TC_PROMO_TREATMENT` or the `TCPromoTreatment` default
/// picks one; the gallery draws all of them side by side.
public enum GlassPromoTreatment: String, Sendable, CaseIterable, Identifiable {
    /// Soft brand-coloured light drifting slowly behind the card.
    case aurora
    /// A diagonal band of light that sweeps across the card now and then.
    case shimmer
    /// A brand gradient running round the card's edge, with a faint glow.
    case glowBorder
    /// A pool of light that follows the pointer over the card.
    case spotlight
    /// The brand tint alone, still.
    case tint

    public var id: String { rawValue }

    /// The default (`spotlight`, owner, 2026-10-11) unless the environment
    /// or the defaults name another.
    public static var stored: GlassPromoTreatment {
        resolve(
            environment: ProcessInfo.processInfo.environment["TC_PROMO_TREATMENT"],
            defaults: UserDefaults.standard.string(forKey: defaultsKey))
    }

    public static let defaultsKey = "TCPromoTreatment"

    /// The environment wins over the defaults; an unknown name is the
    /// default.
    static func resolve(environment: String?, defaults: String?) -> GlassPromoTreatment {
        for value in [environment, defaults] {
            if let value, let treatment = GlassPromoTreatment(rawValue: value) { return treatment }
        }
        return .spotlight
    }

    /// Whether the treatment draws anything that moves on its own.
    var animates: Bool {
        switch self {
        case .aurora, .shimmer, .glowBorder: true
        case .spotlight, .tint: false
        }
    }
}

/// A card on a promotion treatment: the card tier's padding, radius and
/// edge, with the treatment drawn between the tier fill and the content.
/// `interactive` lifts it under the pointer as an interactive `GlassCard`
/// does.
public struct GlassPromoCard<Content: View>: View {
    private let treatment: GlassPromoTreatment
    private let interactive: Bool
    private let content: Content
    @State private var pointer: CGPoint?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.colorScheme) private var colorScheme

    public init(_ treatment: GlassPromoTreatment, interactive: Bool = false, @ViewBuilder content: () -> Content) {
        self.treatment = treatment
        self.interactive = interactive
        self.content = content()
    }

    /// How strongly the brand colours are laid over the card. A little
    /// lower in light mode, where the tint sits under dark text, but not so
    /// low it reads pastel on the grey pane (owner, 2026-10-12).
    static func strength(_ scheme: ColorScheme) -> Double {
        scheme == .dark ? 1 : 0.85
    }

    /// The lift under the pointer: a faint brightening in the dark theme,
    /// and in light a faint deepening of the brand tint, never a wash of
    /// white over the tint (owner, 2026-10-12: the light hover was far too
    /// bright).
    @ViewBuilder
    static func hover(_ scheme: ColorScheme, shape: RoundedRectangle) -> some View {
        if scheme == .dark {
            shape.fill(Color.white.opacity(Self.hoverDark)).blendMode(.plusLighter)
        } else {
            shape.fill(GlassTokens.Color.purpleSoft.color.opacity(Self.hoverLight))
        }
    }

    static var hoverDark: Double { 0.06 }
    static var hoverLight: Double { 0.07 }

    /// The ground under the tint: the card's own fill in the dark theme,
    /// and white in light, so the brand tint reads clean rather than muddy
    /// over the grey pane (owner, 2026-10-12).
    @ViewBuilder
    static func base(_ scheme: ColorScheme, shape: RoundedRectangle) -> some View {
        if scheme == .dark {
            GlassTier.card.fill(in: shape)
        } else {
            shape.fill(Color.white)
        }
    }

    public var body: some View {
        let shape = RoundedRectangle(cornerRadius: GlassTier.card.radius, style: .continuous)
        let strength = Self.strength(colorScheme)
        content
            .padding(.vertical, GlassTokens.Space.cardPaddingVertical)
            .padding(.horizontal, GlassTokens.Space.cardPaddingHorizontal)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background {
                ZStack {
                    Self.base(colorScheme, shape: shape)
                    GlassPromoTint(strength: strength)
                    GlassPromoLight(
                        treatment: treatment, pointer: pointer, strength: strength, still: reduceMotion)
                    if interactive, pointer != nil {
                        Self.hover(colorScheme, shape: shape)
                    }
                }
                .glassPressedFill()
                .animation(GlassMotion.fast(reduceMotion), value: pointer == nil)
            }
            .clipShape(shape)
            .overlay {
                GlassPromoEdge(treatment: treatment, strength: strength, still: reduceMotion, shape: shape)
            }
            .glassEdge(GlassTier.card.edge, in: shape)
            .contentShape(shape)
            .onContinuousHover { phase in
                switch phase {
                case .active(let location): pointer = location
                case .ended: pointer = nil
                }
            }
    }
}

/// The brand tint every treatment starts from: the accent at the top
/// leading corner fading to `promoTintEnd` at the bottom trailing one
/// (purple to blue in the dark theme, orange to amber in light).
private struct GlassPromoTint: View {
    let strength: Double

    var body: some View {
        LinearGradient(
            colors: [
                GlassTokens.Color.purpleSoft.color.opacity(0.34 * strength),
                GlassTokens.Color.purpleSoft.color.opacity(0.2 * strength),
                GlassTokens.Color.promoTintEnd.color.opacity(0.22 * strength),
            ],
            startPoint: .topLeading, endPoint: .bottomTrailing)
    }
}

/// The light inside the card: what moves, by treatment.
private struct GlassPromoLight: View {
    let treatment: GlassPromoTreatment
    let pointer: CGPoint?
    let strength: Double
    let still: Bool

    var body: some View {
        switch treatment {
        case .aurora:
            TimelineView(.animation(minimumInterval: 1 / 30, paused: still)) { context in
                aurora(at: still ? 0 : context.date.timeIntervalSinceReferenceDate)
            }
        case .shimmer:
            TimelineView(.animation(minimumInterval: 1 / 60, paused: still)) { context in
                shimmer(at: still ? nil : context.date.timeIntervalSinceReferenceDate)
            }
        case .spotlight:
            spotlight
        case .glowBorder, .tint:
            EmptyView()
        }
    }

    /// Three soft blobs of brand light on slow, unrelated orbits, so the
    /// pattern does not visibly repeat.
    private func aurora(at time: TimeInterval) -> some View {
        GeometryReader { box in
            let size = box.size
            let blobs: [(Color, Double, Double, Double, CGFloat)] = [
                (GlassTokens.Color.purpleSoft.color, 0.11, 0.0, 0.62, 0.9),
                (GlassTokens.Color.promoTintEnd.color, 0.07, 2.1, 0.5, 0.75),
                (GlassTokens.Color.purpleText.color, 0.05, 4.2, 0.32, 0.6),
            ]
            ZStack {
                ForEach(0..<blobs.count, id: \.self) { index in
                    let (color, speed, phase, alpha, scale) = blobs[index]
                    let angle = time * speed * 2 * .pi + phase
                    let diameter = max(size.width, size.height) * scale
                    Circle()
                        .fill(color.opacity(alpha * strength))
                        .frame(width: diameter, height: diameter)
                        .position(
                            x: size.width * (0.5 + 0.42 * cos(angle)),
                            y: size.height * (0.5 + 0.38 * sin(angle * 1.3)))
                }
            }
            .blur(radius: 34)
        }
        .allowsHitTesting(false)
    }

    /// A band of light crossing the card for 1.4s in every 6s; held off
    /// the card when still.
    private func shimmer(at time: TimeInterval?) -> some View {
        GeometryReader { box in
            let period = 6.0
            let sweep = 1.4
            let phase = time.map { $0.truncatingRemainder(dividingBy: period) / sweep } ?? 2
            let progress = min(max(phase, 0), 1)
            let band = box.size.width * 0.45
            LinearGradient(
                colors: [.clear, Color.white.opacity(0.22 * strength), .clear],
                startPoint: .leading, endPoint: .trailing)
                .frame(width: band, height: box.size.height * 2)
                .rotationEffect(.degrees(20))
                .offset(x: -band + (box.size.width + band * 2) * progress, y: -box.size.height / 2)
                .opacity(phase <= 1 ? 1 : 0)
        }
        .allowsHitTesting(false)
    }

    /// A soft pool of light under the pointer; a faint one at the top
    /// leading corner while the pointer is elsewhere.
    private var spotlight: some View {
        GeometryReader { box in
            let centre = pointer ?? CGPoint(x: box.size.width * 0.15, y: 0)
            RadialGradient(
                colors: [
                    GlassTokens.Color.purpleSoft.color.opacity((pointer == nil ? 0.25 : 0.5) * strength),
                    .clear,
                ],
                center: UnitPoint(
                    x: centre.x / max(box.size.width, 1), y: centre.y / max(box.size.height, 1)),
                startRadius: 0, endRadius: max(box.size.width, box.size.height) * 0.6)
        }
        .allowsHitTesting(false)
    }
}

/// The card's edge on a treatment: a brand gradient running round it for
/// `glowBorder`, and a thin brand rule on the others, so a promotion card
/// is told apart from its neighbours without its motion.
private struct GlassPromoEdge: View {
    let treatment: GlassPromoTreatment
    let strength: Double
    let still: Bool
    let shape: RoundedRectangle

    private static var ring: [Color] {
        [
            GlassTokens.Color.purpleSoft.color, GlassTokens.Color.promoTintEnd.color,
            GlassTokens.Color.purpleText.color, GlassTokens.Color.purpleSoft.color,
        ]
    }

    var body: some View {
        switch treatment {
        case .glowBorder:
            TimelineView(.animation(minimumInterval: 1 / 30, paused: still)) { context in
                let turn = still ? 0 : (context.date.timeIntervalSinceReferenceDate / 8).truncatingRemainder(dividingBy: 1)
                let gradient = AngularGradient(colors: Self.ring, center: .center, angle: .degrees(turn * 360))
                ZStack {
                    shape.strokeBorder(gradient, lineWidth: 1.5)
                    shape.strokeBorder(gradient, lineWidth: 4)
                        .blur(radius: 6)
                        .opacity(0.55 * strength)
                }
            }
            .allowsHitTesting(false)
        default:
            shape.strokeBorder(
                LinearGradient(
                    colors: [
                        GlassTokens.Color.purpleSoft.color.opacity(0.7 * strength),
                        GlassTokens.Color.promoTintEnd.color.opacity(0.35 * strength),
                    ],
                    startPoint: .topLeading, endPoint: .bottomTrailing),
                lineWidth: 1)
                .allowsHitTesting(false)
        }
    }
}
