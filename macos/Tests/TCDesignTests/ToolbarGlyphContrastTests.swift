import XCTest

@testable import TCDesign

/// A toolbar glyph takes #1146's inks (owner ruling, 2026-10-07: #1146
/// wins for theming): #E6E6EC, and #7C7C86 while its pane is hidden.
final class ToolbarGlyphContrastTests: XCTestCase {
    func test_toolbarGlyphsTakeTheReferenceInks() {
        XCTAssertEqual(GlassToolbarButton.glyph(pressed: true), GlassTokens.Color.toolbarGlyph)
        XCTAssertEqual(GlassToolbarButton.glyph(pressed: nil), GlassTokens.Color.toolbarGlyph)
        XCTAssertEqual(GlassToolbarButton.glyph(pressed: false), GlassTokens.Color.toolbarGlyphHidden)
        XCTAssertEqual(GlassTokens.Color.toolbarGlyph.rgb, 0xE6E6EC)
        XCTAssertEqual(GlassTokens.Color.toolbarGlyphHidden.rgb, 0x7C7C86)
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
