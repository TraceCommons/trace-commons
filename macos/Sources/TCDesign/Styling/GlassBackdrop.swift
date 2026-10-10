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
        if content && !GlassTheme.contentIsGlass { return .opaque }
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
            if GlassTheme.current == .flat {
                if let view = flatView(cornerRadius) { return view }
            }
            if #available(macOS 26.0, *) {
                let glass = NSGlassEffectView()
                // The regular, frosted style in every theme: the clear
                // style showed the desktop unblurred behind a focused window
                // (owner feedback, 2026-10-09).
                glass.style = .regular
                glass.appearance = GlassTheme.materialAppearance
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

    /// The flat theme's material, where it is not the default frosted glass.
    private static func flatView(_ cornerRadius: CGFloat) -> NSView? {
        switch GlassTheme.flatMaterial {
        case .regular:
            return nil
        case .regularTint, .clearTint:
            guard #available(macOS 26.0, *) else { return nil }
            let glass = NSGlassEffectView()
            glass.style = GlassTheme.flatMaterial == .clearTint ? .clear : .regular
            // A faint share of the veil, for the glass's own colour; the
            // veil itself is painted over it (`GlassPaneFill`).
            glass.tintColor = GlassTheme.glassTint.dynamicNSColor
            glass.appearance = GlassTheme.materialAppearance
            glass.cornerRadius = cornerRadius
            return glass
        case .sidebar, .hud:
            let effect = NSVisualEffectView()
            effect.material = GlassTheme.flatMaterial == .sidebar ? .sidebar : .hudWindow
            effect.blendingMode = .behindWindow
            effect.state = .active
            effect.appearance = GlassTheme.materialAppearance
            effect.wantsLayer = true
            effect.layer?.cornerRadius = cornerRadius
            effect.layer?.cornerCurve = .continuous
            effect.layer?.masksToBounds = true
            return effect
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
        effect.appearance = GlassTheme.materialAppearance
        effect.wantsLayer = true
        effect.layer?.cornerRadius = cornerRadius
        effect.layer?.cornerCurve = .continuous
        effect.layer?.masksToBounds = true
        return effect
    }
}

/// A pane's whole fill: the native material with #1146's veil and pane
/// gradient over it, as #1146's `html.tc-native-glass .tc-pane` draws it
/// (owner ruling, 2026-10-07: the pane gradient, veil and edge from #1146,
/// on macOS 26 too).
///
/// - On macOS 26, Liquid Glass under the veil and the gradient.
/// - Before 26, the HUD material under the same two.
/// - In the content layer, the opaque base.
struct GlassPaneFill: View {
    let radius: CGFloat
    @Environment(\.glassPaneIsContent) private var content
    @Environment(\.controlActiveState) private var activeState
    @Environment(\.colorSchemeContrast) private var contrast

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: radius, style: .continuous)
        // Reduce Transparency is the system's to apply: it frosts Liquid
        // Glass and turns the materials opaque (R14).
        let material = GlassMaterial.current(content: content)
        switch material {
        case .liquidGlass, .vibrancy:
            ZStack {
                GlassBackdrop(material: material, cornerRadius: radius)
                shape.fill(GlassTokens.Color.glassVeil.color)
                shape.fill(GlassTokens.Gradient.paneFill.linear)
                // Out of focus a window reads darker (inactiveDim, nothing in
                // classic); under Increase Contrast the flat veil is deepened.
                if activeState != .key {
                    shape.fill(GlassTokens.Color.inactiveDim.color)
                }
                if GlassTheme.current == .flat && contrast == .increased {
                    shape.fill(GlassTokens.Color.glassVeil.color.opacity(0.5))
                }
            }
        case .opaque:
            ZStack {
                shape.fill(GlassTokens.Color.paneOpaque.color)
                shape.fill(GlassTokens.Gradient.paneFill.linear)
            }
        }
    }
}

/// The blur under a floating surface: the HUD material blended within the
/// window, so it blurs the map or pane the surface floats on (#1146's
/// `backdrop-filter` on its floating controls, popovers and node cards). The spec's popover tier asks for blur 24 at 170% saturation; the
/// system material is the nearest native equivalent, and it adapts with the
/// system, Reduce Transparency included.
struct GlassFloatingBlur: NSViewRepresentable {
    let cornerRadius: CGFloat

    func makeNSView(context: Context) -> NSVisualEffectView {
        let effect = NSVisualEffectView()
        effect.material = .hudWindow
        effect.blendingMode = .withinWindow
        effect.state = .active
        effect.appearance = GlassTheme.materialAppearance
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
        // Not quite clear: the window server passes a click on a fully
        // transparent pixel to the window below, so the pane gaps and the
        // panes' rounded corners took no clicks, and the edges and corners
        // there would not start a resize. At this alpha nothing shows, and
        // the whole frame catches the resize cursor (and, with
        // `isMovableByWindowBackground`, a drag in a gap moves the window).
        window.backgroundColor = NSColor(white: 0, alpha: 0.004)
        window.hasShadow = false
        window.titlebarAppearsTransparent = true
        window.titleVisibility = .hidden
        window.styleMask.insert(.fullSizeContentView)
        window.isMovableByWindowBackground = true
        // No appearance of our own: the window follows the person's system
        // appearance, light or dark, and every token resolves for it.
        // An empty unified toolbar makes the title bar taller and brings the
        // real traffic lights in from the window's corner, so they sit
        // inside the main pane (which runs to the window's edge), not on
        // its rim.
        if window.toolbar == nil {
            window.toolbar = NSToolbar(identifier: "glass-window")
        }
        window.toolbarStyle = .unified
        // No title-bar separator: the panes run under the title bar, so its
        // hairline was drawn across the window's top and showed as a line in
        // each gap between the panes (owner, 2026-10-08).
        window.titlebarSeparatorStyle = .none
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
