import SwiftUI

/// A row of a modal's section list (#1146 `settings-modal.tsx`): 12pt
/// secondary text, full width, and on hover a faint ink fill with the text
/// lifted to primary. Never the blue menu selection: the row scrolls a body
/// to its section, it selects nothing. The button's own label is drawn; this
/// style authors no words.
public struct GlassSectionNavRowStyle: ButtonStyle {
    public init() {}

    /// The hover fill (#1146 `hover:bg-white/8`).
    public static let hoverInk: Double = 0.08
    /// The row's corner (#1146 `rounded-lg`).
    public static let radius: CGFloat = GlassTokens.Radius.control

    public func makeBody(configuration: Configuration) -> some View {
        GlassSectionNavRowBody(configuration: configuration)
    }
}

private struct GlassSectionNavRowBody: View {
    let configuration: ButtonStyleConfiguration
    @State private var hovering = false
    @Environment(\.isEnabled) private var isEnabled

    var body: some View {
        let lit = hovering && isEnabled
        configuration.label
            .glassType(GlassTokens.TypeScale.label.weight(.regular))
            .foregroundStyle(lit ? GlassColor.textPrimary : (isEnabled ? GlassColor.textSecondary : GlassColor.textTertiary))
            .lineLimit(1)
            .truncationMode(.tail)
            .frame(maxWidth: .infinity, alignment: .leading)
            // #1146 `px-2.5 py-1.5`.
            .padding(.horizontal, GlassTokens.Space.s5)
            .padding(.vertical, GlassTokens.Space.s3)
            .background(
                RoundedRectangle(cornerRadius: GlassSectionNavRowStyle.radius, style: .continuous)
                    .fill(lit ? GlassColor.ink(GlassSectionNavRowStyle.hoverInk) : .clear)
                    .glassPressedFill()
            )
            .environment(\.glassPressed, configuration.isPressed && isEnabled)
            .contentShape(Rectangle())
            .onHover { hovering = $0 }
    }
}
