import SwiftUI

/// A stat tile (#1146 `StatCard`): an eyebrow label over a bold tabular
/// value, on a full card with 10/12 padding. The value truncates; the
/// label wraps at a space, never inside a word, as #1146's untruncated
/// `tc-eyebrow` does ("CREDIT / PENDING", never "CREDIT PEN…").
public struct GlassStatCard: View {
    private let label: String
    private let value: String

    public init(_ label: String, value: String) {
        self.label = label
        self.value = value
    }

    /// The value's type: #1146's 18px/700 tabular figure, on the nearest
    /// macOS text style (title2, 17pt), bold.
    static let valueType = GlassTokens.TypeScale.heading.weight(.bold)

    public var body: some View {
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
        }
        .padding(.vertical, 10)
        .padding(.horizontal, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .glassTier(.card)
        .accessibilityElement(children: .combine)
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
