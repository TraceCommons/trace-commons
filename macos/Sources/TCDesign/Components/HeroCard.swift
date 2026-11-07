import AppKit
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
    /// With `contentSpansGraphic`, the title and line run the card's width
    /// as well, stopping only short of the close button (owner, 2026-11-02:
    /// Home's Traces hero, condensed, has no graphic worth keeping the
    /// title off).
    private let titleSpansGraphic: Bool
    /// The action is the pill alone, a button of its own, and the card is
    /// a plain panel, so the caller's content can hold buttons of its own
    /// (owner, 2026-10-29: Home's Traces hero's steps each open a tab).
    private let actionOnPill: Bool
    /// A narrow card's form (product owner, 2026-11-07): the title in the
    /// smaller title type and no subtitle, so the card takes less of a
    /// narrow window. The caller decides when, from the width it has.
    private let compact: Bool
    private let graphic: Graphic
    private let content: Content
    @State private var hovering = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    public init(
        gradient: GlassGradient, title: String, subtitle: String? = nil,
        actionTitle: String? = nil, action: (() -> Void)? = nil,
        closeLabel: String? = nil, onClose: (() -> Void)? = nil,
        contentSpansGraphic: Bool = false, titleSpansGraphic: Bool = false, actionOnPill: Bool = false,
        compact: Bool = false,
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
        self.titleSpansGraphic = titleSpansGraphic
        self.actionOnPill = actionOnPill
        self.compact = compact
        self.graphic = graphic()
        self.content = content()
    }

    /// The card's least height, and how much of its width the words take:
    /// the rest is the graphic's. The words set the card's height, so a
    /// longer title or a wrapped line never pushes the action off the card.
    static var minHeight: CGFloat { 156 }
    static var graphicReserve: CGFloat { 150 }
    /// How much of the large graphic shows through: all of it. A pixel graphic
    /// carries its own transparency in its cells (Ron, 2026-11-05).
    static var graphicOpacity: Double { 1 }

    /// How far a title that runs the card's width stops short of its
    /// trailing edge for the close button: the X's whole target, less the
    /// card's own padding, so a click on the title never closes the card.
    static var closeReserve: CGFloat {
        max(0, GlassHeroClose.inset * 2 + GlassHeroClose.glyph - GlassTokens.Space.cardPaddingHorizontal)
    }

    /// The title and line's trailing room inside the words: none when the
    /// whole of the words stops short of the graphic, the graphic's when
    /// only the caller's content runs under it, and the close button's
    /// when the title runs the card's width too.
    static func titleTrailing(contentSpansGraphic: Bool, titleSpansGraphic: Bool, closes: Bool) -> CGFloat {
        guard contentSpansGraphic else { return 0 }
        guard titleSpansGraphic else { return graphicReserve }
        return closes ? closeReserve : 0
    }

    /// The title's face: larger and heavier than a card title, as a hero's
    /// is (the references' display type, on the system face).
    static var titleType: GlassTypeStyle { GlassTokens.TypeScale.display.weight(.heavy) }
    /// The compact card's title (`compact`).
    static var compactTitleType: GlassTypeStyle { GlassTokens.TypeScale.title.weight(.heavy) }

    public var body: some View {
        Group {
            if actionOnPill {
                card.accessibilityElement(children: .contain)
            } else if let action {
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
            .frame(maxWidth: .infinity, minHeight: compact ? nil : Self.minHeight, alignment: .leading)
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
                    .glassType(compact ? Self.compactTitleType : Self.titleType)
                    .foregroundStyle(GlassTokens.Color.heroInk.color)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityAddTraits(.isHeader)
                if let subtitle, !compact {
                    Text(subtitle)
                        .glassType(GlassTokens.TypeScale.label.weight(.regular))
                        .foregroundStyle(GlassTokens.Color.heroInk.color.opacity(0.88))
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .padding(.trailing, Self.titleTrailing(
                contentSpansGraphic: contentSpansGraphic, titleSpansGraphic: titleSpansGraphic, closes: onClose != nil))
            content
            if let actionTitle {
                if actionOnPill, let action {
                    Button(action: action) { pill(actionTitle) }
                        .buttonStyle(GlassPressStyle())
                } else {
                    pill(actionTitle)
                }
            }
        }
        .padding(GlassTokens.Space.cardPaddingHorizontal)
    }

    private func pill(_ actionTitle: String) -> some View {
        Text(actionTitle)
            .glassType(GlassTokens.TypeScale.label.weight(.semibold))
            .foregroundStyle(GlassTokens.Color.heroButtonInk.color)
            .padding(.horizontal, 14)
            .padding(.vertical, 7)
            .background(Capsule().fill(GlassTokens.Color.heroButtonFill.color))
            .padding(.top, GlassTokens.Space.s1)
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

/// A hero's graphic built from a symbol or image: large and bright, tilted,
/// in heroInk, alone (owner, 2026-10-12: no sparkles around it), on the pixel
/// grid (`GlassPixelGlyph`; Ron, 2026-11-05).
/// It is larger than the card is tall and set past its trailing and
/// bottom edges, so the card's border cuts it off (owner, 2026-10-12, as
/// the reference does).
public struct GlassHeroSymbol: View {
    private let source: GlassPixelGlyph.Source

    public init(_ systemImage: String) {
        source = .symbol(systemImage, weight: .bold)
    }

    public init(image: NSImage) {
        source = .image(image)
    }

    public var body: some View {
        GlassPixelGlyph(
            source, size: Self.symbolSize,
            // The pixel field's own blues, strong enough that the whole shape
            // reads at the contrast the field's corner has: a glyph is spread
            // over the card where the field is massed in one corner.
            palette: Self.tints.map { $0.opacity(Self.brightCells) },
            dimmed: Self.tints.map { $0.opacity(Self.dimCells) },
            cell: Self.cell)
        // The whole grid tilts: tilting the source and sampling it instead
        // breaks straight edges into stairs.
        .rotationEffect(.degrees(Self.angle))
        // Past the card's trailing and bottom edges.
        .offset(Self.bleed)
    }

    /// The main symbol's size, past the card's height, and how far the
    /// whole graphic is pushed past its trailing and bottom edges.
    static var symbolSize: CGFloat { 190 }
    static var bleed: CGSize { CGSize(width: 46, height: 34) }
    /// The promotions' tilt.
    static var angle: Double { -12 }
    /// The pixel field's pitch, its three blues (`GetStartedCardField`), and
    /// the cells' strengths.
    static var cell: CGFloat { 6 }
    static var tints: [GlassRGBA] {
        [GlassTokens.Color.dataKept, GlassTokens.Color.chartSeries1, GlassTokens.Color.firstRunBeamCore]
    }
    static var brightCells: Double { 0.46 }
    static var dimCells: Double { 0.32 }
}
