import XCTest

@testable import TCDesign

/// A toolbar glyph takes #1146's inks (owner ruling, 2026-10-07: #1146
/// wins for theming): #E6E6EC, and a dimmer grey while its pane is hidden.
/// A glyph is a UI component, so even dimmed it clears 3:1 on the control
/// fill it sits on, every stop of it over the pane base, in both
/// appearances (owner ruling, 2026-10-07: hold the WCAG floors). #1146's
/// dimmed #7C7C86 was 2.76:1 in dark; the token is that grey lightened the
/// least that clears the floor.
final class ToolbarGlyphContrastTests: XCTestCase {
    func test_toolbarGlyphsTakeTheReferenceInks() {
        XCTAssertEqual(GlassToolbarButton.glyph(pressed: true), GlassTokens.Color.toolbarGlyph)
        XCTAssertEqual(GlassToolbarButton.glyph(pressed: nil), GlassTokens.Color.toolbarGlyph)
        XCTAssertEqual(GlassToolbarButton.glyph(pressed: false), GlassTokens.Color.toolbarGlyphHidden)
        XCTAssertEqual(GlassTokens.Color.toolbarGlyph.rgb, 0xE6E6EC)
    }

    func test_aDimmedToolbarGlyphClearsTheGlyphFloor() {
        for dark in [true, false] {
            let base = dark ? GlassTokens.Color.paneBase.dark : GlassTokens.Color.paneBase.light
            for stop in GlassTokens.Gradient.controlFill.stops {
                let ground = Self.over(dark ? stop.color.dark : stop.color.light, GlassRGBA(base.rgb))
                for pressed: Bool? in [false, true, nil] {
                    let glyph = GlassToolbarButton.glyph(pressed: pressed)
                    let ratio = SwitchContrastTests.contrast(dark ? glyph.dark : glyph.light, ground)
                    XCTAssertGreaterThanOrEqual(
                        ratio, 3, "\(dark ? "dark" : "light") pressed \(String(describing: pressed)) at \(stop.location): \(ratio)")
                }
            }
        }
    }

    /// #1146's own dimmed grey would not, in dark: the reason for the nudge.
    func test_theReferenceDimmedGreyWouldNot() {
        let ground = Self.over(GlassTokens.Gradient.controlFill.stops[0].color, GlassRGBA(GlassTokens.Color.paneBase.rgb))
        XCTAssertLessThan(SwitchContrastTests.contrast(GlassRGBA(0x7C7C86), ground), 3)
    }

    /// Lit, the glyph clears text contrast on the control fill.
    func test_aLitToolbarGlyphClearsTextContrast() {
        let ground = Self.over(GlassTokens.Gradient.controlFill.stops[0].color, GlassTokens.Color.paneBase)
        let ratio = SwitchContrastTests.contrast(GlassToolbarButton.glyph(pressed: true), ground)
        XCTAssertGreaterThanOrEqual(ratio, 4.5, "\(ratio)")
    }

    /// `top` at its alpha over an opaque `bottom`.
    static func over(_ top: GlassRGBA, _ bottom: GlassRGBA) -> GlassRGBA {
        func channel(_ t: Double, _ b: Double) -> UInt32 { UInt32((t * top.alpha + b * (1 - top.alpha)) * 255 + 0.5) }
        let rgb = channel(top.red, bottom.red) << 16 | channel(top.green, bottom.green) << 8 | channel(top.blue, bottom.blue)
        return GlassRGBA(rgb)
    }
}
