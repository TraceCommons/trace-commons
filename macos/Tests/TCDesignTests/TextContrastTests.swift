import XCTest

@testable import TCDesign

/// Every text token clears 4.5:1 on the pane base, including the tertiary
/// sub-line of an "off" row, which fades the whole row by `rowOff` while it
/// stays interactive. At the earlier 0.6 the faded sub-line was about 3.9:1.
final class TextContrastTests: XCTestCase {
    private let base = GlassRGBA(GlassTokens.Color.paneBase.rgb)

    func test_everyTextTokenClearsTextContrastOnThePane() {
        let text: [(String, GlassRGBA)] = [
            ("textPrimary", GlassTokens.Color.textPrimary),
            ("textSecondary", GlassTokens.Color.textSecondary),
            ("textTertiary", GlassTokens.Color.textTertiary),
            ("purpleText", GlassTokens.Color.purpleText),
        ]
        for (name, token) in text {
            let ratio = SwitchContrastTests.contrast(token, base)
            XCTAssertGreaterThanOrEqual(ratio, 4.5, "\(name): \(ratio)")
        }
    }

    func test_anOffRowsSubLineStillClearsTextContrast() {
        let faded = ToolbarGlyphContrastTests.over(
            GlassRGBA(GlassTokens.Color.textTertiary.rgb, alpha: GlassTokens.Opacity.rowOff),
            base
        )
        let ratio = SwitchContrastTests.contrast(faded, base)
        XCTAssertGreaterThanOrEqual(ratio, 4.5, "\(ratio)")
    }
}
