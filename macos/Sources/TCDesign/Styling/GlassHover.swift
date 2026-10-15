import SwiftUI

/// The hover state (#1146 glass.css, the `:hover` rules): a fill under the
/// pointer, over the control's own fill and behind its label, faded in on
/// the fast duration and shown only while the control can be used.
public enum GlassHover {
    /// Whether a hover fill shows: under the pointer, and enabled. A
    /// disabled control does not answer the pointer.
    public static func shows(hovering: Bool, enabled: Bool) -> Bool {
        hovering && enabled
    }
}

public extension View {
    /// A hover fill in `shape`, behind this view's content: `fill` while the
    /// pointer is over it and it is enabled, nothing otherwise. Apply it
    /// inside the control's tier (before `glassSurface`), so it draws over
    /// the tier's fill and under the label.
    func glassHover<S: Shape>(_ fill: GlassRGBA, in shape: S) -> some View {
        modifier(GlassHoverFill(fill: fill, shape: shape))
    }
}

private struct GlassHoverFill<S: Shape>: ViewModifier {
    let fill: GlassRGBA
    let shape: S
    @State private var hovering = false
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func body(content: Content) -> some View {
        let shows = GlassHover.shows(hovering: hovering, enabled: isEnabled)
        content
            .background {
                shape
                    .fill(fill.color)
                    .opacity(shows ? 1 : 0)
                    .animation(GlassMotion.fast(reduceMotion), value: shows)
            }
            .onHover { hovering = $0 }
    }
}
