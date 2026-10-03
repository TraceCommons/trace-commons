import AppKit
import SwiftUI

/// Which material sits under a pane (D4).
public enum GlassMaterial: Sendable, Equatable {
    /// Liquid Glass, `NSGlassEffectView` (macOS 26 and later).
    case liquidGlass
    /// The HUD vibrancy material, blended behind the window (macOS 14–25).
    case vibrancy
    /// No material: the opaque pane base, for a pane in the content layer
    /// (the map), which Apple keeps out of Liquid Glass.
    case opaque

    /// The material this Mac draws for a pane in the navigation layer.
    ///
    /// Reduce Transparency, Increase Contrast and Reduce Motion are not
    /// handled here. Liquid Glass and the system materials adapt to them by
    /// themselves (Liquid Glass turns frostier under Reduce Transparency and
    /// takes a contrasting border under Increase Contrast), and Apple's
    /// guidance is to let them rather than swap in a fill of our own (R14).
    public static func current(content: Bool = false) -> GlassMaterial {
        if content { return .opaque }
        if #available(macOS 26.0, *) { return .liquidGlass }
        return .vibrancy
    }
}

public extension EnvironmentValues {
    /// True inside a pane in the content layer (the map): it is drawn on the
    /// opaque base, never Liquid Glass, so the controls floating on it are
    /// the only glass there (no glass on glass).
    @Entry var glassPaneIsContent: Bool = false
}

/// The native material under a glass pane, clipped to the pane's rounded
/// rect. The same choice #1146's `tc_glass_view_make` makes for the Tauri
/// window: Liquid Glass where it exists, the HUD material before it.
///
/// It blends with what is behind the window, so it only shows through in a
/// window that draws no background of its own; see `glassWindow()`.
struct GlassBackdrop: NSViewRepresentable {
    let material: GlassMaterial
    let cornerRadius: CGFloat

    func makeNSView(context: Context) -> NSView {
        Self.makeView(material, cornerRadius: cornerRadius)
    }

    /// The native view for a material. `.opaque` is no material at all, so
    /// it is a solid layer in the opaque pane base, never a vibrancy view.
    static func makeView(_ material: GlassMaterial, cornerRadius: CGFloat) -> NSView {
        switch material {
        case .liquidGlass:
            if #available(macOS 26.0, *) {
                let glass = NSGlassEffectView()
                glass.style = .regular
                glass.cornerRadius = cornerRadius
                return glass
            }
            return vibrancy(cornerRadius)
        case .vibrancy:
            return vibrancy(cornerRadius)
        case .opaque:
            return opaque(cornerRadius)
        }
    }

    func updateNSView(_ view: NSView, context: Context) {
        if #available(macOS 26.0, *), let glass = view as? NSGlassEffectView {
            glass.cornerRadius = cornerRadius
        } else {
            view.layer?.cornerRadius = cornerRadius
        }
    }

    private static func opaque(_ cornerRadius: CGFloat) -> NSView {
        let view = OpaquePaneView()
        view.layer?.cornerRadius = cornerRadius
        view.layer?.cornerCurve = .continuous
        view.layer?.masksToBounds = true
        return view
    }

    private static func vibrancy(_ cornerRadius: CGFloat) -> NSView {
        let effect = NSVisualEffectView()
        effect.material = .hudWindow
        effect.blendingMode = .behindWindow
        effect.state = .active
        effect.wantsLayer = true
        effect.layer?.cornerRadius = cornerRadius
        effect.layer?.cornerCurve = .continuous
        effect.layer?.masksToBounds = true
        return effect
    }
}

/// A pane's whole fill.
///
/// - On macOS 26, Liquid Glass alone. No veil and no sheen: the regular
///   variant keeps its own contents legible and draws its own highlights,
///   and Apple reserves a dimming layer for the clear variant and tinting
///   for primary actions.
/// - Before 26, the HUD material with the veil and sheen over it: a painted
///   approximation of the glass, as the spec gives it.
/// - In the content layer, the opaque base.
struct GlassPaneFill: View {
    let radius: CGFloat
    @Environment(\.glassPaneIsContent) private var content

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: radius, style: .continuous)
        switch GlassMaterial.current(content: content) {
        case .liquidGlass:
            GlassBackdrop(material: .liquidGlass, cornerRadius: radius)
        case .vibrancy:
            ZStack {
                GlassBackdrop(material: .vibrancy, cornerRadius: radius)
                shape.fill(GlassTokens.Color.glassVeil.color)
                shape.fill(GlassTokens.Gradient.paneFill.linear)
            }
        case .opaque:
            ZStack {
                shape.fill(GlassTokens.Color.paneOpaque.color)
                shape.fill(GlassTokens.Gradient.paneFill.linear)
            }
        }
    }
}

/// The blur under a floating surface before macOS 26: the HUD material
/// blended within the window, so it blurs the map or pane the surface floats
/// on. The spec's popover tier asks for blur 24 at 170% saturation; the
/// system material is the nearest native equivalent, and it adapts with the
/// system, Reduce Transparency included.
struct GlassFloatingBlur: NSViewRepresentable {
    let cornerRadius: CGFloat

    func makeNSView(context: Context) -> NSVisualEffectView {
        let effect = NSVisualEffectView()
        effect.material = .hudWindow
        effect.blendingMode = .withinWindow
        effect.state = .active
        effect.wantsLayer = true
        effect.layer?.cornerRadius = cornerRadius
        effect.layer?.cornerCurve = .continuous
        effect.layer?.masksToBounds = true
        return effect
    }

    func updateNSView(_ view: NSVisualEffectView, context: Context) {
        view.layer?.cornerRadius = cornerRadius
    }
}

// MARK: - Window

/// Makes the hosting window a floating glass window: no background of its
/// own and a transparent full-size title bar, in the system appearance. Panes then float
/// on the desktop with the gap between them showing through.
private struct GlassWindowConfigurator: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView {
        let view = WindowProbe()
        view.onWindow = Self.configure
        return view
    }

    func updateNSView(_ view: NSView, context: Context) {}

    @MainActor
    static func configure(_ window: NSWindow) {
        window.isOpaque = false
        window.backgroundColor = .clear
        window.hasShadow = false
        window.titlebarAppearsTransparent = true
        window.titleVisibility = .hidden
        window.styleMask.insert(.fullSizeContentView)
        window.isMovableByWindowBackground = true
        // No appearance of our own: the window follows the person's system
        // appearance, light or dark, and every token resolves for it.
        // An empty unified toolbar makes the title bar taller and brings the
        // real traffic lights in from the window's corner, so with the 10pt
        // window padding they sit inside the main pane, not on its rim.
        if window.toolbar == nil {
            window.toolbar = NSToolbar(identifier: "glass-window")
        }
        window.toolbarStyle = .unified
    }

    private final class WindowProbe: NSView {
        var onWindow: (@MainActor (NSWindow) -> Void)?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if let window { onWindow?(window) }
        }
    }
}

public extension View {
    /// Host this view in a floating glass window (see `GlassThreePane`).
    func glassWindow() -> some View {
        background(GlassWindowConfigurator().frame(width: 0, height: 0))
    }
}

/// The opaque pane base as a layer-backed view that follows the
/// appearance. A layer's `CGColor` does not resolve per appearance, so the
/// fill is set again whenever the view's effective appearance changes;
/// baking in one value drew the dark base under the light appearance.
final class OpaquePaneView: NSView {
    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        applyFill()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        wantsLayer = true
        applyFill()
    }

    override var wantsUpdateLayer: Bool { true }

    override func updateLayer() {
        applyFill()
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        applyFill()
    }

    private func applyFill() {
        var fill: CGColor?
        effectiveAppearance.performAsCurrentDrawingAppearance {
            fill = GlassTokens.Color.paneOpaque.dynamicNSColor.usingColorSpace(.sRGB)?.cgColor
        }
        layer?.backgroundColor = fill
    }
}
