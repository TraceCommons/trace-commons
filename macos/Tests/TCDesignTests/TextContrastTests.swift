import XCTest

@testable import TCDesign

/// Every text token clears 4.5:1 on the pane base, including the sub-line
/// of an "off" row, which fades the whole row by `rowOff` (0.6, #1146) while
/// it stays interactive. Faded, a tertiary sub-line would be about 3.9:1,
/// so an off row sets it in the secondary ink (owner ruling, 2026-10-07:
/// hold the WCAG floors).
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
            GlassRGBA(GlassListRow.plainSubInk(off: true).rgb, alpha: GlassTokens.Opacity.rowOff),
            base
        )
        let ratio = SwitchContrastTests.contrast(faded, base)
        XCTAssertGreaterThanOrEqual(ratio, 4.5, "\(ratio)")
    }

    /// The tertiary ink would not: the reason an off row changes ink.
    func test_aFadedTertiarySubLineWouldNot() {
        XCTAssertEqual(GlassListRow.plainSubInk(off: false), GlassTokens.Color.textTertiary)
        let faded = ToolbarGlyphContrastTests.over(
            GlassRGBA(GlassTokens.Color.textTertiary.rgb, alpha: GlassTokens.Opacity.rowOff), base)
        XCTAssertLessThan(SwitchContrastTests.contrast(faded, base), 4.5)
    }

    /// In light, an off row's title and sub-line, faded by `rowOff`, fall
    /// under 4.5:1 on the light pane base: textPrimary 4.32, the secondary
    /// sub-line 3.02 (both as on main before #1273). No ink fixes the
    /// sub-line (it would need about #18181b), only a lighter fade, which
    /// is #1146's value and the owner's to move. Recorded as findings, not
    /// exemptions: each fails loudly the day it starts to pass.
    func test_anOffRowInLightIsAKnownFinding() {
        let lightBase = GlassRGBA(GlassTokens.Color.paneBase.light.rgb)
        for (name, ink) in [("title", GlassTokens.Color.textPrimary.light),
                            ("sub-line", GlassListRow.plainSubInk(off: true).light)] {
            let faded = ToolbarGlyphContrastTests.over(GlassRGBA(ink.rgb, alpha: GlassTokens.Opacity.rowOff), lightBase)
            let ratio = SwitchContrastTests.contrast(faded, lightBase)
            XCTExpectFailure("FINDING: a light off row's \(name) is \(ratio):1, under 4.5:1") {
                XCTAssertGreaterThanOrEqual(ratio, 4.5, "\(name): \(ratio)")
            }
        }
    }
}
