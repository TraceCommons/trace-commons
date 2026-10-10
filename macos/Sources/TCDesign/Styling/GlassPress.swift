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

    /// For a control with no fill of its own (a glyph, an unselected tab):
    /// the opacity of a black wash under its label, so what is behind the
    /// label goes `darkening` darker while the label keeps its contrast.
    public static func washOpacity(_ pressed: Bool) -> Double {
        pressed ? darkening : 0
    }
}

/// A button with no look of its own whose content draws a tier
/// (`glassSurface` or `glassTier`): the round, pill and toolbar buttons,
/// a menu row, a segment. It publishes the press so that tier's fill
/// darkens; content with no tier darkens its own fill from `glassPressed`.
public struct GlassPressStyle: ButtonStyle {
    private let disabledOpacity: Double

    /// `disabledOpacity` is how far a disabled control fades: the shared
    /// `Opacity.disabled`, or `Opacity.disabledCheck` for a checkbox or a
    /// radio (#1146 `.tc-checkbox:disabled`, `.tc-radio:disabled`).
    public init(disabledOpacity: Double = GlassTokens.Opacity.disabled) {
        self.disabledOpacity = disabledOpacity
    }

    public func makeBody(configuration: Configuration) -> some View {
        GlassPressBody(configuration: configuration, disabledOpacity: disabledOpacity)
    }
}

private struct GlassPressBody: View {
    let configuration: ButtonStyleConfiguration
    let disabledOpacity: Double
    @Environment(\.isEnabled) private var isEnabled

    var body: some View {
        configuration.label
            .environment(\.glassPressed, configuration.isPressed && isEnabled)
            // A control that cannot be used says so, as the glass buttons do.
            .opacity(isEnabled ? 1 : disabledOpacity)
    }
}

extension View {
    /// Darken this fill while the surrounding button is pressed. Apply it to
    /// a fill, never to a label.
    func glassPressedFill() -> some View {
        modifier(GlassPressedFill())
    }

    /// For a control with no fill: a wash in `shape`, behind this label,
    /// that darkens what is under it while the surrounding button is
    /// pressed. The label itself is untouched.
    func glassPressedWash<S: Shape>(_ shape: S) -> some View {
        background { GlassPressedWash(shape: shape) }
    }
}

private struct GlassPressedWash<S: Shape>: View {
    let shape: S
    @Environment(\.glassPressed) private var pressed
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        shape
            .fill(Color.black.opacity(GlassPress.washOpacity(pressed)))
            .animation(GlassMotion.fast(reduceMotion), value: pressed)
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
