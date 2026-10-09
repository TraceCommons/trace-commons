import AppKit
import SwiftUI
import XCTest

@testable import TCDesign

/// R14: accessibility follows Apple's Liquid Glass guidance. Native glass
/// adapts to Reduce Transparency, Increase Contrast and Reduce Motion by
/// itself, and the shell does not override it; painted surfaces and custom
/// colours carry their own Increase Contrast values.
final class AccessibilityTests: XCTestCase {
    // MARK: Increase Contrast

    /// Secondary and tertiary text and the hairline resolve to their
    /// Increase Contrast values under the system's high-contrast appearance,
    /// and to their usual values otherwise.
    @MainActor
    func test_customColoursFollowIncreaseContrast() throws {
        let pairs: [(GlassRGBA, GlassRGBA)] = [
            (GlassTokens.Color.textSecondary, GlassTokens.Color.textSecondaryHighContrast),
            (GlassTokens.Color.textTertiary, GlassTokens.Color.textTertiaryHighContrast),
            (GlassTokens.Color.hairline, GlassTokens.Color.hairlineHighContrast),
        ]
        let dark = try XCTUnwrap(NSAppearance(named: .darkAqua))
        for (base, high) in pairs {
            let usual = base.adaptiveNSColor(highContrast: high, increases: { _ in false })
            let increased = base.adaptiveNSColor(highContrast: high, increases: { _ in true })
            XCTAssertEqual(Self.resolve(usual, in: dark), base.nsColor.cgColor.components ?? [])
            XCTAssertEqual(Self.resolve(increased, in: dark), high.nsColor.cgColor.components ?? [])
        }
        // The plain dark appearance is not a high-contrast one; the
        // system's own setting is read alongside it.
        XCTAssertFalse(GlassRGBA.isHighContrast(dark))
        XCTAssertEqual(
            GlassRGBA.increasesContrast(dark), NSWorkspace.shared.accessibilityDisplayShouldIncreaseContrast)
    }

    /// The Increase Contrast values are more contrasting on the dark panes:
    /// lighter text, a more opaque hairline.
    func test_increasedValuesAreMoreContrasting() {
        XCTAssertGreaterThan(Self.luminance(GlassTokens.Color.textSecondaryHighContrast), Self.luminance(GlassTokens.Color.textSecondary))
        XCTAssertGreaterThan(Self.luminance(GlassTokens.Color.textTertiaryHighContrast), Self.luminance(GlassTokens.Color.textTertiary))
        XCTAssertGreaterThan(GlassTokens.Color.hairlineHighContrast.alpha, GlassTokens.Color.hairline.alpha)
        XCTAssertEqual(GlassTokens.Color.edgeHighContrast.alpha, 0.4, accuracy: 0.0001)
    }

    // MARK: Apple's Liquid Glass rules

    /// Native glass is not swapped for a fill under Reduce Transparency: the
    /// system frosts Liquid Glass and turns its materials opaque. Nothing in
    /// the styling reads the setting to override it.
    func test_nativeGlassIsLeftToTheSystemUnderReduceTransparency() throws {
        for name in ["GlassStyle.swift", "GlassBackdrop.swift"] {
            let source = try Self.source(name)
            XCTAssertFalse(source.contains("accessibilityReduceTransparency"), "\(name) overrides Reduce Transparency")
        }
    }

    /// Floating surfaces are #1146's painted tiers over a blur, never a
    /// tinted or clear glass of their own (owner ruling, 2026-10-07).
    func test_floatingSurfacesArePaintedOverABlur() throws {
        let style = try Self.source("GlassStyle.swift")
        XCTAssertFalse(style.contains(".regular.tint("), "floating glass is tinted")
        XCTAssertFalse(style.contains(".glassEffect("), "a floating surface draws Liquid Glass, not #1146's tier")
    }

    // MARK: Helpers

    @MainActor
    private static func resolve(_ color: NSColor, in appearance: NSAppearance) -> [CGFloat] {
        var components: [CGFloat] = []
        appearance.performAsCurrentDrawingAppearance {
            components = color.usingColorSpace(.sRGB)?.cgColor.components ?? []
        }
        return components
    }

    private static func luminance(_ rgba: GlassRGBA) -> Double {
        0.2126 * rgba.red + 0.7152 * rgba.green + 0.0722 * rgba.blue
    }

    private static func source(_ name: String) throws -> String {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TCDesign/Styling").appendingPathComponent(name)
        return try String(contentsOf: url, encoding: .utf8)
    }
}
