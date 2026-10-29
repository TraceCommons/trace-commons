import SwiftUI

/// A hero promotion (owner, 2026-10-12): a saturated gradient ground, a
/// large graphic overlaid on the right and running off the card's edge,
/// and on the left a bold title, a line under it, the caller's content,
/// and a white pill naming the action (no eyebrow, no logo: owner,
/// 2026-10-12). The same in both
/// appearances: the ground is its own, and every word on it is `heroInk`.
///
/// The whole card is one button (the pill draws the action, it is not a
/// second control), so VoiceOver reads it once: the title, the line and
/// the action. The graphic is decoration and hidden from assistive tech.
/// Given `onClose`, a small X in the top trailing corner closes the card
/// (owner, 2026-10-27): a control of its own, over the card's button.
public struct GlassHeroCard<Graphic: View, Content: View>: View {
    private let gradient: GlassGradient
    private let title: String
    private let subtitle: String?
    private let actionTitle: String?
    /// `nil` draws the card as a plain panel: not a button, no hover.
    private let action: (() -> Void)?
    private let closeLabel: String?
    private let onClose: (() -> Void)?
    /// The caller's content runs the card's full width, under the graphic,
    /// while the title and line still stop short of it (owner, 2026-10-29:
    /// Home's Traces hero draws its three steps across the whole card).
    private let contentSpansGraphic: Bool
    private let graphic: Graphic
    private let content: Content
    @State private var hovering = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    public init(
        gradient: GlassGradient, title: String, subtitle: String? = nil,
        actionTitle: String? = nil, action: (() -> Void)? = nil,
        closeLabel: String? = nil, onClose: (() -> Void)? = nil,
        contentSpansGraphic: Bool = false,
        @ViewBuilder graphic: () -> Graphic, @ViewBuilder content: () -> Content
    ) {
        self.gradient = gradient
        self.title = title
        self.subtitle = subtitle
        self.actionTitle = actionTitle
        self.action = action
        self.closeLabel = closeLabel
        self.onClose = onClose
        self.contentSpansGraphic = contentSpansGraphic
        self.graphic = graphic()
        self.content = content()
    }

    /// The card's least height, and how much of its width the words take:
    /// the rest is the graphic's. The words set the card's height, so a
    /// longer title or a wrapped line never pushes the action off the card.
    static var minHeight: CGFloat { 156 }
    static var graphicReserve: CGFloat { 150 }
    /// How much of the large graphic shows through (owner, 2026-10-19: at
    /// full strength it competed with the words).
    static var graphicOpacity: Double { 0.5 }

    /// The title's face: larger and heavier than a card title, as a hero's
    /// is (the references' display type, on the system face).
    static var titleType: GlassTypeStyle { GlassTokens.TypeScale.display.weight(.heavy) }

    public var body: some View {
        Group {
            if let action {
                Button(action: action) { card }
                    .buttonStyle(GlassPressStyle())
                    .onHover { inside in
                        withAnimation(GlassMotion.fast(reduceMotion)) { hovering = inside }
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityAddTraits(.isButton)
            } else {
                card.accessibilityElement(children: .combine)
            }
        }
        .overlay(alignment: .topTrailing) {
            if let onClose {
                GlassHeroClose(label: closeLabel ?? "", action: onClose)
            }
        }
    }

    private var card: some View {
        let shape = RoundedRectangle(cornerRadius: GlassTier.card.radius, style: .continuous)
        return words
            .padding(.trailing, contentSpansGraphic ? 0 : Self.graphicReserve)
            .frame(maxWidth: .infinity, minHeight: Self.minHeight, alignment: .leading)
            .background(alignment: .trailing) {
                // Behind the words, sized to the card, past its edges.
                graphic
                    .frame(width: Self.graphicReserve + 60)
                    .frame(maxHeight: .infinity)
                    .opacity(Self.graphicOpacity)
                    .accessibilityHidden(true)
            }
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

    private var words: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
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
            .padding(.trailing, contentSpansGraphic ? Self.graphicReserve : 0)
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

/// A hero card's X: small, in heroInk on the card's own ground so it reads
/// in both appearances, brighter under the pointer, with a control-sized
/// target (the Traces cards' X, on a hero's ground).
struct GlassHeroClose: View {
    let label: String
    let action: () -> Void
    @State private var hovering = false

    /// The glyph's side, and its inset from the card's top and trailing edges.
    static var glyph: CGFloat { 12 }
    static var inset: CGFloat { GlassTokens.Space.s4 }

    var body: some View {
        Button(action: action) {
            Image(systemName: "xmark")
                .glassGlyph(Self.glyph, weight: .semibold)
                .foregroundStyle(GlassTokens.Color.heroInk.color.opacity(hovering ? 1 : 0.75))
                .padding(Self.inset)
                .contentShape(Rectangle())
        }
        .buttonStyle(GlassPressStyle())
        .onHover { hovering = $0 }
        .accessibilityLabel(label)
        .help(label)
    }
}

/// A hero's graphic built from a symbol: the symbol large and bright,
/// tilted, in heroInk, alone (owner, 2026-10-12: no sparkles around it).
/// It is larger than the card is tall and set past its trailing and
/// bottom edges, so the card's border cuts it off (owner, 2026-10-12, as
/// the reference does).
public struct GlassHeroSymbol: View {
    private let systemImage: String

    public init(_ systemImage: String) {
        self.systemImage = systemImage
    }

    public var body: some View {
        let ink = GlassTokens.Color.heroInk.color
        ZStack {
            Image(systemName: systemImage)
                .glassGlyph(Self.symbolSize, weight: .bold)
                .foregroundStyle(LinearGradient(
                    colors: [ink.opacity(0.95), ink.opacity(0.35)], startPoint: .top, endPoint: .bottom))
                .rotationEffect(.degrees(-12))
                .shadow(color: .black.opacity(0.25), radius: 18, y: 10)
        }
        // Past the card's trailing and bottom edges.
        .offset(Self.bleed)
    }

    /// The main symbol's size, past the card's height, and how far the
    /// whole graphic is pushed past its trailing and bottom edges.
    static var symbolSize: CGFloat { 190 }
    static var bleed: CGSize { CGSize(width: 46, height: 34) }
}
