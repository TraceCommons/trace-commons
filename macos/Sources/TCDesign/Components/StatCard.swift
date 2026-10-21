import SwiftUI

/// A stat tile (#1146 `StatCard`): an eyebrow label over a bold tabular
/// value, on a full card with 10/12 padding. The value truncates; the
/// label wraps at a space, never inside a word, as #1146's untruncated
/// `tc-eyebrow` does ("CREDIT / PENDING", never "CREDIT PEN…").
public struct GlassStatCard: View {
    private let label: String
    private let value: String
    /// A card that acts (it sits in a button): it lifts under the pointer,
    /// as an interactive `GlassCard` does.
    private let interactive: Bool
    /// A caption under the value, in `detailInk` when it carries a tone
    /// (a gain in green, a loss in red); nil draws none.
    private let detail: String?
    private let detailInk: GlassRGBA?
    /// A symbol on the card's leading side, in the accent, with no
    /// container (owner, 2026-10-12; moved to the leading side and enlarged,
    /// owner, 2026-10-15); nil draws none.
    private let systemImage: String?
    /// A figure beside the value, quieter than it (the size of what waits);
    /// nil draws none.
    private let valueAccessory: String?
    /// Where the symbol is drawn: beside the words, or as the card's
    /// watermark (`GlassCardWatermark`).
    private let iconPlacement: IconPlacement

    /// Where a stat card draws its symbol.
    public enum IconPlacement: Sendable {
        /// On the leading side, beside the label and value.
        case leading
        /// Large on the trailing side, tilted and cut off by the card's
        /// edges, behind the words (owner, 2026-10-20, Home's cards).
        case watermark
    }

    /// The icon's size: near the label and value's two lines together
    /// (14 + 22pt), so the symbol fills the tile's height.
    static let iconGlyph: CGFloat = 28
    /// The icon's column: one width for every symbol, the widest
    /// (`lock.laptopcomputer`) included, so the labels beside them start at
    /// the same place from tile to tile.
    static let iconColumn: CGFloat = 40

    public init(
        _ label: String, value: String, detail: String? = nil, detailInk: GlassRGBA? = nil,
        systemImage: String? = nil, valueAccessory: String? = nil, interactive: Bool = false,
        iconPlacement: IconPlacement = .leading
    ) {
        self.systemImage = systemImage
        self.iconPlacement = iconPlacement
        self.valueAccessory = valueAccessory
        self.label = label
        self.value = value
        self.detail = detail
        self.detailInk = detailInk
        self.interactive = interactive
    }

    /// The value's type: #1146's 18px/700 tabular figure, on the nearest
    /// macOS text style (title2, 17pt), bold.
    static let valueType = GlassTokens.TypeScale.heading.weight(.bold)

    /// The icon is centred on its own tile's text, not on the tile: a tile
    /// stretched to its row's taller neighbour keeps the icon beside its
    /// label and value.
    public var body: some View {
        HStack(alignment: .center, spacing: GlassTokens.Space.s3) {
            if let systemImage, iconPlacement == .leading {
                Image(systemName: systemImage)
                    .glassGlyph(Self.iconGlyph, weight: .regular)
                    .foregroundStyle(GlassTokens.Color.purpleSoft.color)
                    .frame(width: Self.iconColumn)
                    .accessibilityHidden(true)
            }
            figures
        }
        .padding(.vertical, 10)
        .padding(.horizontal, 12)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .glassCardWatermark(iconPlacement == .watermark ? systemImage : nil)
        .glassTier(.card, hover: GlassCard<EmptyView>.hoverFill(interactive: interactive))
        .accessibilityElement(children: .combine)
    }

    private var figures: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(label)
                .glassType(GlassTokens.TypeScale.eyebrow)
                .foregroundStyle(GlassColor.textTertiary)
                .lineLimit(2)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
            HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s3) {
                Text(value)
                    .glassType(Self.valueType)
                    .monospacedDigit()
                    .foregroundStyle(GlassColor.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                if let valueAccessory {
                    Text(valueAccessory)
                        .glassType(GlassTokens.TypeScale.caption)
                        .monospacedDigit()
                        .foregroundStyle(GlassColor.textTertiary)
                        .lineLimit(1)
                }
            }
            if let detail {
                Text(detail)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(detailInk?.color ?? GlassColor.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, GlassTokens.Space.s1)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// A card's symbol as a watermark (owner, 2026-10-20: "like in the larger
/// promo cards"): large on the trailing side, tilted as the promotions'
/// graphics are, running off the card's trailing and bottom edges, in the
/// accent a little faded. It sits behind the card's words and over its
/// fill: give it with `glassCardWatermark` before the card's tier.
public struct GlassCardWatermark: View {
    private let systemImage: String

    public init(_ systemImage: String) {
        self.systemImage = systemImage
    }

    /// The symbol's size, past a stat card's height.
    public static let size: CGFloat = 58
    /// The promotions' tilt (`GlassHeroSymbol`).
    public static let angle: Double = -12
    /// A little under full strength, so the words over it stay first.
    public static let opacity: Double = 0.55
    /// How far the symbol is pushed past the trailing and bottom edges.
    public static let bleed = CGSize(width: 12, height: 14)

    public var body: some View {
        Image(systemName: systemImage)
            .glassGlyph(Self.size, weight: .regular)
            .foregroundStyle(GlassTokens.Color.purpleSoft.color)
            .rotationEffect(.degrees(Self.angle))
            .opacity(Self.opacity)
            .fixedSize()
            .offset(Self.bleed)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomTrailing)
            .accessibilityHidden(true)
    }
}

public extension View {
    /// Draws `systemImage` as the card's watermark (`GlassCardWatermark`),
    /// clipped to the card's shape, behind this view. Apply it before the
    /// card's tier, so the tier's fill stays under it. Nil draws nothing.
    func glassCardWatermark(_ systemImage: String?, radius: CGFloat = GlassTier.card.radius) -> some View {
        background {
            if let systemImage {
                GlassCardWatermark(systemImage)
                    .clipShape(RoundedRectangle(cornerRadius: radius, style: .continuous))
            }
        }
    }
}

/// A failed request's line: plain caption text in the status colour, with
/// no frame, drawn under the buttons it is about. Ron, 2026-10-09: a boxed
/// line (#1146 `.tc-alert`'s quiet card) read as a disabled text field, so
/// every card says a failure this way, under its buttons.
public struct GlassAlert: View {
    private let text: String

    public init(_ text: String) {
        self.text = text
    }

    public var body: some View {
        Text(text)
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(GlassTokens.Color.statusOutsideText.color)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// A code or transcript block (#1146 `.tc-code`): a dark well in the
/// consent fill with the well's inner edge, mono 11.
public struct GlassCode: View {
    private let text: String

    public init(_ text: String) {
        self.text = text
    }

    public var body: some View {
        let shape = RoundedRectangle(cornerRadius: GlassTokens.Radius.cardQuiet, style: .continuous)
        Text(text)
            .glassType(GlassTokens.TypeScale.mono)
            .foregroundStyle(GlassColor.textPrimary)
            .textSelection(.enabled)
            .fixedSize(horizontal: false, vertical: true)
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(shape.fill(GlassTokens.Color.consentFill.color))
            .glassEdge(GlassTokens.Shadow.wellEdge, in: shape)
    }
}
