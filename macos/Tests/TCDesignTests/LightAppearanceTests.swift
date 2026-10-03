import AppKit
import XCTest

@testable import TCDesign

/// The glass system follows the person's system appearance, light or dark
/// (Apple's Human Interface Guidelines: respect the system appearance). Every
/// colour resolves per appearance, and text and status colours keep their
/// contrast floors in both.
final class LightAppearanceTests: XCTestCase {
    /// A colour with a light value resolves to it under the light appearance
    /// and to its dark value under the dark one.
    @MainActor
    func test_coloursResolvePerAppearance() throws {
        let light = try XCTUnwrap(NSAppearance(named: .aqua))
        let dark = try XCTUnwrap(NSAppearance(named: .darkAqua))
        let token = GlassTokens.Color.textPrimary
        XCTAssertNotNil(token.lightRGB)
        XCTAssertEqual(Self.resolve(token.dynamicNSColor, in: light), Self.components(token.light))
        XCTAssertEqual(Self.resolve(token.dynamicNSColor, in: dark), Self.components(token.dark))
        XCTAssertTrue(GlassRGBA.isLight(light))
        XCTAssertFalse(GlassRGBA.isLight(dark))
    }

    /// Increase Contrast and the appearance combine: each of the four
    /// resolves to its own value.
    @MainActor
    func test_increaseContrastAppliesInBothAppearances() throws {
        let light = try XCTUnwrap(NSAppearance(named: .aqua))
        let base = GlassTokens.Color.textSecondary
        let high = GlassTokens.Color.textSecondaryHighContrast
        let increased = base.adaptiveNSColor(highContrast: high, increases: { _ in true })
        let usual = base.adaptiveNSColor(highContrast: high, increases: { _ in false })
        XCTAssertEqual(Self.resolve(increased, in: light), Self.components(high.light))
        XCTAssertEqual(Self.resolve(usual, in: light), Self.components(base.light))
    }

    /// Body text reaches 4.5:1 on the pane base and the scene in both
    /// appearances; accent text too.
    func test_textReachesFourPointFiveToOneInBothAppearances() {
        let texts = [GlassTokens.Color.textPrimary, GlassTokens.Color.textSecondary, GlassTokens.Color.textTertiary, GlassTokens.Color.purpleText,
                     GlassTokens.Color.textSecondaryHighContrast, GlassTokens.Color.textTertiaryHighContrast]
        let grounds = [GlassTokens.Color.paneOpaque, GlassTokens.Color.sceneWarm]
        for text in texts {
            for ground in grounds {
                XCTAssertGreaterThanOrEqual(Self.contrast(text.light, ground.light), 4.5, "light \(text) on \(ground)")
                XCTAssertGreaterThanOrEqual(Self.contrast(text.dark, ground.dark), 4.5, "dark \(text) on \(ground)")
            }
        }
    }

    /// Status dots and glyphs reach the 3:1 non-text floor in both
    /// appearances, and white on the brand purple stays readable.
    func test_statusAndAccentKeepTheirFloors() {
        for status in [GlassTokens.Color.statusOn, GlassTokens.Color.statusAsk, GlassTokens.Color.statusOutside, GlassTokens.Color.dataShared, GlassTokens.Color.dataKept] {
            XCTAssertGreaterThanOrEqual(Self.contrast(status.light, GlassTokens.Color.paneOpaque.light), 3, "light \(status)")
            XCTAssertGreaterThanOrEqual(Self.contrast(status.dark, GlassTokens.Color.paneOpaque.dark), 3, "dark \(status)")
        }
        XCTAssertGreaterThanOrEqual(Self.contrast(GlassTokens.Color.textOnAccent, GlassTokens.Color.purple), 4.5)
    }

    /// Every switch knob clears 3:1 against its on track in the light
    /// appearance too.
    func test_switchKnobsKeepTheirFloorInLight() {
        for kind in [GlassSwitchKind.standard, .settings, .watch] {
            let track = GlassToggleStyle.onColor(kind).light
            let knob = GlassToggleStyle.knob(kind, isOn: true).light
            XCTAssertGreaterThanOrEqual(Self.contrast(knob, track), 3, "\(kind)")
        }
    }

    /// No white overlay is left to vanish on a light surface: every
    /// translucent white colour and gradient stop has a light value. White
    /// inset highlights in the edges are deliberate in both appearances.
    func test_noDarkOnlyWhiteOverlaySurvives() {
        for (name, token) in GlassTokens.Color.all where token.rgb == 0xFFFFFF && token.alpha < 1 {
            XCTAssertNotNil(token.lightRGB, "color.\(name)")
        }
        for (name, gradient) in GlassTokens.Gradient.all {
            for stop in gradient.stops where stop.color.rgb == 0xFFFFFF && stop.color.alpha < 1 {
                XCTAssertNotNil(stop.color.lightRGB, "gradient.\(name)")
            }
        }
    }

    // MARK: Helpers

    @MainActor
    private static func resolve(_ color: NSColor, in appearance: NSAppearance) -> [CGFloat] {
        var out: [CGFloat] = []
        appearance.performAsCurrentDrawingAppearance {
            out = color.usingColorSpace(.sRGB)?.cgColor.components ?? []
        }
        return out
    }

    private static func components(_ rgba: GlassRGBA) -> [CGFloat] {
        rgba.nsColor.usingColorSpace(.sRGB)?.cgColor.components ?? []
    }

    private static func luminance(_ c: GlassRGBA) -> Double {
        func channel(_ v: Double) -> Double { v <= 0.03928 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4) }
        return 0.2126 * channel(c.red) + 0.7152 * channel(c.green) + 0.0722 * channel(c.blue)
    }

    private static func contrast(_ a: GlassRGBA, _ b: GlassRGBA) -> Double {
        let (x, y) = (luminance(a), luminance(b))
        return (max(x, y) + 0.05) / (min(x, y) + 0.05)
    }
}
