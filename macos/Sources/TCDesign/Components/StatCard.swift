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

    /// The icon's size: near the label and value's two lines together
    /// (14 + 22pt), so the symbol fills the tile's height.
    static let iconGlyph: CGFloat = 28
    /// The icon's column: one width for every symbol, the widest
    /// (`lock.laptopcomputer`) included, so the labels beside them start at
    /// the same place from tile to tile.
    static let iconColumn: CGFloat = 40

    public init(
        _ label: String, value: String, detail: String? = nil, detailInk: GlassRGBA? = nil,
        systemImage: String? = nil, valueAccessory: String? = nil, interactive: Bool = false
    ) {
        self.systemImage = systemImage
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
            if let systemImage {
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
