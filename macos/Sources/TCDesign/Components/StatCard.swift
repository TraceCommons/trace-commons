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
    /// A symbol on an accent tile at the card's top trailing corner (owner,
    /// 2026-10-12: icons make the tiles more engaging); nil draws none.
    private let systemImage: String?

    /// The icon tile's side and its glyph's size.
    static let iconSide: CGFloat = 26
    static let iconGlyph: CGFloat = 12

    public init(
        _ label: String, value: String, detail: String? = nil, detailInk: GlassRGBA? = nil,
        systemImage: String? = nil, interactive: Bool = false
    ) {
        self.systemImage = systemImage
        self.label = label
        self.value = value
        self.detail = detail
        self.detailInk = detailInk
        self.interactive = interactive
    }

    /// The value's type: #1146's 18px/700 tabular figure, on the nearest
    /// macOS text style (title2, 17pt), bold.
    static let valueType = GlassTokens.TypeScale.heading.weight(.bold)

    public var body: some View {
        HStack(alignment: .top, spacing: GlassTokens.Space.s3) {
            figures
            if let systemImage {
                Image(systemName: systemImage)
                    .glassGlyph(Self.iconGlyph, weight: .semibold)
                    .foregroundStyle(GlassTokens.Color.textOnSelected.color)
                    .frame(width: Self.iconSide, height: Self.iconSide)
                    .background(
                        RoundedRectangle(cornerRadius: 8, style: .continuous)
                            .fill(GlassTokens.Color.controlSelected.color))
                    .accessibilityHidden(true)
            }
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
            Text(value)
                .glassType(Self.valueType)
                .monospacedDigit()
                .foregroundStyle(GlassColor.textPrimary)
                .lineLimit(1)
                .truncationMode(.tail)
            if let detail {
                Text(detail)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(detailInk?.color ?? GlassColor.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, GlassTokens.Space.s1)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
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
