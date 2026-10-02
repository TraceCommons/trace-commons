import AppKit
import SwiftUI

/// Which material sits under a pane (D4).
public enum GlassMaterial: Sendable, Equatable {
    /// Liquid Glass, `NSGlassEffectView` (macOS 26 and later).
    case liquidGlass
    /// The HUD vibrancy material, blended behind the window (macOS 14–25).
    case vibrancy
    /// No material: the opaque pane base. Reduce Transparency, and any
    /// surface that must not show what is behind the window.
    case opaque

    /// The material this Mac draws for a pane.
    public static func current(reduceTransparency: Bool) -> GlassMaterial {
        if reduceTransparency { return .opaque }
        if #available(macOS 26.0, *) { return .liquidGlass }
        return .vibrancy
    }
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
        switch material {
        case .liquidGlass:
            if #available(macOS 26.0, *) {
                let glass = NSGlassEffectView()
                glass.style = .regular
                glass.cornerRadius = cornerRadius
                return glass
            }
            return Self.vibrancy(cornerRadius)
        case .vibrancy, .opaque:
            return Self.vibrancy(cornerRadius)
        }
    }

    func updateNSView(_ view: NSView, context: Context) {
        if #available(macOS 26.0, *), let glass = view as? NSGlassEffectView {
            glass.cornerRadius = cornerRadius
        } else {
            view.layer?.cornerRadius = cornerRadius
        }
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

/// A pane's whole fill: the native material, then the dark veil that keeps
/// text at contrast on it, then the specular sheen. With Reduce
/// Transparency the material and veil give way to the opaque base.
struct GlassPaneFill: View {
    let radius: CGFloat
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: radius, style: .continuous)
        let material = GlassMaterial.current(reduceTransparency: reduceTransparency)
        ZStack {
            if material == .opaque {
                shape.fill(GlassTokens.Color.paneBase.color.opacity(1))
            } else {
                GlassBackdrop(material: material, cornerRadius: radius)
                shape.fill(GlassTokens.Color.glassVeil.color)
            }
            shape.fill(GlassTokens.Gradient.paneFill.linear)
        }
    }
}

// MARK: - Window

/// Makes the hosting window a floating glass window: no background of its
/// own, a transparent full-size title bar, and dark (D2). Panes then float
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
        window.appearance = NSAppearance(named: .darkAqua)
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
