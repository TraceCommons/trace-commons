import SwiftUI

/// The accent gradient (`accentFill`), for decoration only (owner,
/// 2026-10-12): the selected tab's glow and the promotion cards' tint.
/// Controls, buttons, toggles and checkboxes keep the solid accent. The
/// flat theme runs teal to green in Light and purple to magenta in Dark;
/// classic keeps its own purples for both.
public enum GlassAccent {
    /// Whether the decoration draws the gradient (the flat theme).
    public static var isGradient: Bool { GlassTheme.current == .flat }

    /// The gradient's darker stop: the foot of the tab glow and its line.
    public static var deep: GlassRGBA { GlassTokens.Gradient.accentFill.stops[0].color }
    /// The lighter stop: the top of the tab glow.
    public static var bright: GlassRGBA { GlassTokens.Gradient.accentFill.stops[1].color }

    /// A promotion's tint runs from `promoStart` at its top leading corner
    /// to `promoEnd` at its bottom trailing one: the gradient's stops in the
    /// flat theme, classic's accent and far stop otherwise.
    public static var promoStart: GlassRGBA { isGradient ? deep : GlassTokens.Color.purpleSoft }
    public static var promoEnd: GlassRGBA { isGradient ? bright : GlassTokens.Color.promoTintEnd }
}
