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

    private static func sources() throws -> [(String, String)] {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Sources/TCDesign")
        let files = try XCTUnwrap(FileManager.default.enumerator(at: root, includingPropertiesForKeys: nil))
            .compactMap { $0 as? URL }
            .filter { $0.pathExtension == "swift" }
        XCTAssertGreaterThanOrEqual(files.count, 8)
        return try files.map { ($0.lastPathComponent, try String(contentsOf: $0, encoding: .utf8)) }
            .sorted { $0.0 < $1.0 }
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
