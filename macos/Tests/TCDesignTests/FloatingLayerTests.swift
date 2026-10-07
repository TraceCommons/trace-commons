import SwiftUI
import XCTest

@testable import TCDesign

/// R4: Liquid Glass belongs to the floating layer only. Inside a pane
/// everything is painted, so glass is never stacked on glass.
final class FloatingLayerTests: XCTestCase {
    func test_theDefaultLayerIsContent() {
        XCTAssertEqual(EnvironmentValues().glassLayer, .content)
    }

    /// One place decides when a surface becomes Liquid Glass: the
    /// `glassSurface` modifier. A component calling `glassEffect` itself
    /// would skip the content-layer and Reduce Transparency checks.
    func test_onlyTheSurfaceModifierAppliesLiquidGlass() throws {
        var callers: [String] = []
        for (name, text) in try Self.sources() where text.contains(".glassEffect(") {
            callers.append(name)
        }
        XCTAssertEqual(callers, ["GlassStyle.swift"])
    }

    /// Controls go through `glassSurface`, so the same control is painted in
    /// a pane and glass over the map.
    func test_controlsFollowTheLayer() throws {
        for (name, text) in try Self.sources() where name != "GlassStyle.swift" {
            XCTAssertFalse(text.contains(".glassTier(.control"), "\(name) paints a control that should follow the layer")
        }
    }

    /// A floating surface is the painted tier over the HUD blur on every
    /// macOS, as #1146 draws its floating controls (owner ruling,
    /// 2026-10-07), whatever Reduce Transparency says, and a surface in a
    /// pane is the painted tier. Under Reduce Transparency the system makes
    /// the blur opaque by itself, so floating text never shows the map
    /// through (R14).
    func test_floatingSurfacesArePaintedOverTheBlur() {
        XCTAssertEqual(GlassSurfaceBacking.choose(floating: true), .blur)
        XCTAssertEqual(GlassSurfaceBacking.choose(floating: false), .painted)
    }

    private static func sources() throws -> [(String, String)] {
        try DesignSources.all()
    }
}

/// Before macOS 26 a floating surface is the painted tier over a real
/// within-window blur, not a flat translucent fill.
final class FloatingBlurTests: XCTestCase {
    @MainActor
    func test_theFallbackBlurBlendsWithinTheWindow() {
        let host = NSHostingView(rootView: GlassFloatingBlur(cornerRadius: 14).frame(width: 100, height: 60))
        host.layoutSubtreeIfNeeded()
        func all(_ view: NSView) -> [NSView] { view.subviews + view.subviews.flatMap(all) }
        let effect = all(host).compactMap { $0 as? NSVisualEffectView }.first
        XCTAssertEqual(effect?.blendingMode, .withinWindow)
        XCTAssertEqual(effect?.state, .active)
    }
}
