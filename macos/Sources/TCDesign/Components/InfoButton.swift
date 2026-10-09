import SwiftUI

/// An "i" placed after a row's title that holds the row's longer text
/// (owner, 2026-10-08: the Uses screen's scope descriptions and the
/// Sharing answers' detail).
///
/// Hovering shows the text in a popover; clicking pins it open until the
/// person clicks elsewhere (the system popover closes on an outside click
/// and on Escape, and puts focus back on the button). It is a button, so it
/// is a keyboard stop under Full Keyboard Access, and Space pins it.
/// VoiceOver reads `label` and then the text itself as the value, so the
/// text is reachable without the popover. There is no animation of its
/// own: the system popover's follows Reduce Motion.
///
/// Every word is the caller's: `label` (e.g. the core's "More about
/// {title}", filled) and `text` (e.g. a consent scope's description).
public struct GlassInfoButton: View {
    private let label: String
    private let text: String
    @State private var hovering = false
    @State private var pinned = false

    public init(_ label: String, text: String) {
        self.label = label
        self.text = text
    }

    /// The popover's width: the text wraps inside it.
    static let popoverWidth: CGFloat = 260

    /// Shown while hovered or pinned.
    static func isShown(hovering: Bool, pinned: Bool) -> Bool {
        hovering || pinned
    }

    public var body: some View {
        Button {
            pinned.toggle()
        } label: {
            Image(systemName: pinned ? "info.circle.fill" : "info.circle")
                .glassGlyph(12)
                .foregroundStyle(GlassColor.textTertiary)
                .contentShape(Rectangle())
                .accessibilityHidden(true)
        }
        .buttonStyle(GlassPressStyle())
        .onHover { hovering = $0 }
        .popover(isPresented: shown, arrowEdge: .bottom) {
            Text(text)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
                .frame(width: Self.popoverWidth, alignment: .leading)
                .padding(GlassTokens.Space.s5)
        }
        .accessibilityLabel(label)
        .accessibilityValue(text)
    }

    private var shown: Binding<Bool> {
        Binding(
            get: { Self.isShown(hovering: hovering, pinned: pinned) },
            set: { isPresented in
                // The popover closing (an outside click, Escape) unpins it.
                guard !isPresented else { return }
                hovering = false
                pinned = false
            })
    }
}
