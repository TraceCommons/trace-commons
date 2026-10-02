import SwiftUI

public extension EnvironmentValues {
    /// True while the button around this view is held down. A tier's fill
    /// reads it and darkens; nothing else in the button changes.
    @Entry var glassPressed: Bool = false
}

/// The pressed state (spec, "Components"): the fill 8% darker, over the
/// fast duration. Never a fade, which reads as disabled, and never the
/// label, which keeps its contrast.
public enum GlassPress {
    /// How much darker a pressed fill is.
    public static let darkening: Double = 0.08

    /// The brightness change for a fill: `-darkening` while pressed.
    public static func brightness(_ pressed: Bool) -> Double {
        pressed ? -darkening : 0
    }
}

/// A button with no look of its own whose content draws a tier
/// (`glassSurface` or `glassTier`): the round, pill and toolbar buttons,
/// a menu row, a segment. It publishes the press so that tier's fill
/// darkens; content with no tier darkens its own fill from `glassPressed`.
public struct GlassPressStyle: ButtonStyle {
    public init() {}

    public func makeBody(configuration: Configuration) -> some View {
        GlassPressBody(configuration: configuration)
    }
}

private struct GlassPressBody: View {
    let configuration: ButtonStyleConfiguration
    @Environment(\.isEnabled) private var isEnabled

    var body: some View {
        configuration.label
            .environment(\.glassPressed, configuration.isPressed && isEnabled)
    }
}

extension View {
    /// Darken this fill while the surrounding button is pressed.
    func glassPressedFill() -> some View {
        modifier(GlassPressedFill())
    }
}

private struct GlassPressedFill: ViewModifier {
    @Environment(\.glassPressed) private var pressed
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func body(content: Content) -> some View {
        content
            .brightness(GlassPress.brightness(pressed))
            .animation(GlassMotion.fast(reduceMotion), value: pressed)
    }
}
