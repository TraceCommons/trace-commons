import XCTest

@testable import TCDesign

/// Every text token clears 4.5:1 on the pane base, in both appearances,
/// and so does every word on an "off" row. An off row used to fade as a
/// whole by `rowOff` (0.6, #1146), which put a light off row's title at
/// 4.32:1 and its sub-line at 3.02:1; it now dims by ink, and only its
/// decorative tool tile keeps the fade (owner ruling, 2026-10-07: the
/// floors win everywhere).
final class TextContrastTests: XCTestCase {
    private let base = GlassRGBA(GlassTokens.Color.paneBase.rgb)
    private let lightBase = GlassRGBA(GlassTokens.Color.paneBase.light.rgb)

    func test_everyTextTokenClearsTextContrastOnThePane() {
        let text: [(String, GlassRGBA)] = [
            ("textPrimary", GlassTokens.Color.textPrimary),
            ("textSecondary", GlassTokens.Color.textSecondary),
            ("textTertiary", GlassTokens.Color.textTertiary),
            ("purpleText", GlassTokens.Color.purpleText),
        ]
        for (name, token) in text {
            let dark = SwitchContrastTests.contrast(GlassRGBA(token.rgb), base)
            XCTAssertGreaterThanOrEqual(dark, 4.5, "\(name) dark: \(dark)")
            let light = SwitchContrastTests.contrast(GlassRGBA(token.light.rgb), lightBase)
            XCTAssertGreaterThanOrEqual(light, 4.5, "\(name) light: \(light)")
        }
    }

    func test_anOffRowsWordsClearTextContrastInBothAppearances() {
        let title = GlassListRow.titleInk(selected: false, off: true)
        let sub = GlassListRow.plainSubInk(off: true)
        for (name, ink) in [("title", title), ("sub-line", sub)] {
            let dark = SwitchContrastTests.contrast(GlassRGBA(ink.rgb), base)
            XCTAssertGreaterThanOrEqual(dark, 4.5, "off \(name) dark: \(dark)")
            let light = SwitchContrastTests.contrast(GlassRGBA(ink.light.rgb), lightBase)
            XCTAssertGreaterThanOrEqual(light, 4.5, "off \(name) light: \(light)")
        }
    }

    /// An off row still reads as off: its title steps down from the on
    /// row's, and the row itself is no longer faded.
    func test_anOffRowIsDimmedByInkNotOpacity() throws {
        XCTAssertNotEqual(GlassListRow.titleInk(selected: false, off: true),
                          GlassListRow.titleInk(selected: false, off: false))
        let row = try XCTUnwrap(Dictionary(uniqueKeysWithValues: try DesignSources.components())["ListRow.swift"])
        XCTAssertFalse(row.contains(".opacity(off ? GlassTokens.Opacity.rowOff : 1)\n        .contentShape"))
    }
}
