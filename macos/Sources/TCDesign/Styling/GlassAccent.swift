import SwiftUI

/// The accent as a fill (owner, 2026-10-12): the flat theme draws selected
/// states, toggles and step progress with the accent gradient
/// (`accentFill`: teal to green in Light, purple to magenta in Dark);
/// classic keeps each control's own flat colour.
public enum GlassAccent {
    /// Whether accent fills are the gradient.
    public static var isGradient: Bool { GlassTheme.current == .flat }

    /// The accent gradient in the flat theme; `solid` in classic.
    public static func fill(_ solid: GlassRGBA) -> AnyShapeStyle {
        isGradient ? AnyShapeStyle(GlassTokens.Gradient.accentFill.linear) : AnyShapeStyle(solid.color)
    }
}
