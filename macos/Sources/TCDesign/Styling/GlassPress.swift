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

/// A control drawn as a `GlassPressStyle` button but made of a tap, for
/// the small controls repeated on every row of a long list (a row's
/// disclosure chevron and its "..." menu). SwiftUI walks the window's whole
/// responder tree for accessibility focus each time a `Button` appears or
/// its accessibility changes, so three buttons on each of a few dozen rows
/// put about a hundred such walks behind each tab switch. A tap is not a
/// focus responder, so it adds none. It looks and presses the same: the
/// press darkens the fill through `glassPressed`, and a disabled control
/// fades and takes no tap. The caller gives it its accessibility: a label,
/// the button trait and the action, or a representation.
struct GlassTapControl: ViewModifier {
    let action: () -> Void
    @GestureState private var pressed = false
    @Environment(\.isEnabled) private var isEnabled

    func body(content: Content) -> some View {
        content
            .environment(\.glassPressed, pressed && isEnabled)
            .opacity(isEnabled ? 1 : GlassTokens.Opacity.disabled)
            .onTapGesture { if isEnabled { action() } }
            .simultaneousGesture(
                DragGesture(minimumDistance: 0).updating($pressed) { _, state, _ in state = true },
                including: isEnabled ? .all : .none)
    }
}

extension View {
    /// A press-styled tap control (`GlassTapControl`), in place of a
    /// `Button` with `GlassPressStyle`, for a control on every row of a list.
    func glassTapControl(action: @escaping () -> Void) -> some View {
        modifier(GlassTapControl(action: action))
    }

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
