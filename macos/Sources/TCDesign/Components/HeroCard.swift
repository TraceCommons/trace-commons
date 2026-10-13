import SwiftUI

/// A hero promotion (owner, 2026-10-12): a saturated gradient ground, a
/// large graphic overlaid on the right and running off the card's edge,
/// and on the left an eyebrow, a bold title, a line under it, the caller's
/// content, and a white pill naming the action. The same in both
/// appearances: the ground is its own, and every word on it is `heroInk`.
///
/// The whole card is one button (the pill draws the action, it is not a
/// second control), so VoiceOver reads it once: the title, the line and
/// the action. The graphic is decoration and hidden from assistive tech.
public struct GlassHeroCard<Graphic: View, Content: View>: View {
    private let gradient: GlassGradient
    private let eyebrow: String?
    private let title: String
    private let subtitle: String?
    private let actionTitle: String?
    private let action: () -> Void
    private let graphic: Graphic
    private let content: Content
    @State private var hovering = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    public init(
        gradient: GlassGradient, eyebrow: String? = nil, title: String, subtitle: String? = nil,
        actionTitle: String? = nil, action: @escaping () -> Void,
        @ViewBuilder graphic: () -> Graphic, @ViewBuilder content: () -> Content
    ) {
        self.gradient = gradient
        self.eyebrow = eyebrow
        self.title = title
        self.subtitle = subtitle
        self.actionTitle = actionTitle
        self.action = action
        self.graphic = graphic()
        self.content = content()
    }

    /// The card's least height, and how much of its width the words take:
    /// the rest is the graphic's.
    static var minHeight: CGFloat { 156 }
    static var textShare: CGFloat { 0.62 }
    /// The title's face: larger and heavier than a card title, as a hero's
    /// is (the references' display type, on the system face).
    static var titleType: GlassTypeStyle { GlassTokens.TypeScale.display.weight(.heavy) }

    public var body: some View {
        let shape = RoundedRectangle(cornerRadius: GlassTier.card.radius, style: .continuous)
        Button(action: action) {
            GeometryReader { proxy in
                ZStack(alignment: .topLeading) {
                    graphic
                        .frame(width: proxy.size.width * (1 - Self.textShare) + 40, height: proxy.size.height)
                        .frame(maxWidth: .infinity, alignment: .trailing)
                        .offset(x: 24)
                        .accessibilityHidden(true)
                    words
                        .frame(width: proxy.size.width * Self.textShare, alignment: .leading)
                }
            }
            .frame(minHeight: Self.minHeight)
            .background {
                ZStack {
                    shape.fill(gradient.linear)
                    // A soft light behind the graphic, so it sits in the
                    // ground rather than on it.
                    RadialGradient(
                        colors: [GlassTokens.Color.heroInk.color.opacity(0.22), .clear],
                        center: UnitPoint(x: 0.82, y: 0.4), startRadius: 0, endRadius: 170)
                    if hovering {
                        shape.fill(GlassTokens.Color.heroInk.color.opacity(0.06))
                    }
                }
            }
            .clipShape(shape)
            .contentShape(shape)
        }
        .buttonStyle(GlassPressStyle())
        .onHover { inside in
            withAnimation(GlassMotion.fast(reduceMotion)) { hovering = inside }
        }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
    }

    private var words: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            if let eyebrow {
                Text(eyebrow)
                    .glassType(GlassTokens.TypeScale.eyebrow)
                    .foregroundStyle(GlassTokens.Color.heroInk.color)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 3)
                    .background(Capsule().fill(GlassTokens.Color.heroInk.color.opacity(0.18)))
            }
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                Text(title)
                    .glassType(Self.titleType)
                    .foregroundStyle(GlassTokens.Color.heroInk.color)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityAddTraits(.isHeader)
                if let subtitle {
                    Text(subtitle)
                        .glassType(GlassTokens.TypeScale.label.weight(.regular))
                        .foregroundStyle(GlassTokens.Color.heroInk.color.opacity(0.88))
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            content
            if let actionTitle {
                Text(actionTitle)
                    .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                    .foregroundStyle(GlassTokens.Color.heroButtonInk.color)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 7)
                    .background(Capsule().fill(GlassTokens.Color.heroButtonFill.color))
                    .padding(.top, GlassTokens.Space.s1)
            }
        }
        .padding(GlassTokens.Space.cardPaddingHorizontal)
    }
}

/// A hero's graphic built from a symbol: the symbol large and bright,
/// tilted, with two smaller echoes of `accent` around it, all in heroInk.
public struct GlassHeroSymbol: View {
    private let systemImage: String
    private let accent: String

    public init(_ systemImage: String, accent: String = "sparkle") {
        self.systemImage = systemImage
        self.accent = accent
    }

    public var body: some View {
        let ink = GlassTokens.Color.heroInk.color
        ZStack {
            Image(systemName: systemImage)
                .glassGlyph(108, weight: .bold)
                .foregroundStyle(LinearGradient(
                    colors: [ink.opacity(0.95), ink.opacity(0.35)], startPoint: .top, endPoint: .bottom))
                .rotationEffect(.degrees(-12))
                .shadow(color: .black.opacity(0.25), radius: 18, y: 10)
            Image(systemName: accent)
                .glassGlyph(30, weight: .semibold)
                .foregroundStyle(ink.opacity(0.8))
                .offset(x: -70, y: -46)
            Image(systemName: accent)
                .glassGlyph(16, weight: .semibold)
                .foregroundStyle(ink.opacity(0.6))
                .offset(x: 58, y: 52)
        }
    }
}
