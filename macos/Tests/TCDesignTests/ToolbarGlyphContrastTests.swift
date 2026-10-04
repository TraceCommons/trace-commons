import XCTest

@testable import TCDesign

/// A toolbar glyph is a UI component, so even dimmed (its panel hidden) it
/// clears 3:1 on the control fill it sits on: white at 14% over the pane
/// base. The literal `Color(white: 0.49)` it replaced was about 2.8:1.
final class ToolbarGlyphContrastTests: XCTestCase {
    func test_aDimmedToolbarGlyphClearsTheGlyphFloor() {
        let base = GlassTokens.Color.paneBase
        let fill = GlassTokens.Gradient.controlFill.stops[0].color
        let ground = Self.over(fill, base)
        for pressed: Bool? in [false, true, nil] {
            let ratio = SwitchContrastTests.contrast(GlassToolbarButton.glyph(pressed: pressed), ground)
            XCTAssertGreaterThanOrEqual(ratio, 3, "pressed \(String(describing: pressed)): \(ratio)")
        }
    }

    /// `top` at its alpha over an opaque `bottom`.
    static func over(_ top: GlassRGBA, _ bottom: GlassRGBA) -> GlassRGBA {
        func channel(_ t: Double, _ b: Double) -> UInt32 { UInt32((t * top.alpha + b * (1 - top.alpha)) * 255 + 0.5) }
        let rgb = channel(top.red, bottom.red) << 16 | channel(top.green, bottom.green) << 8 | channel(top.blue, bottom.blue)
        return GlassRGBA(rgb)
    }
}
