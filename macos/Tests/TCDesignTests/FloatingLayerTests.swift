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
