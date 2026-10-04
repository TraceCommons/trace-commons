import XCTest

@testable import TCDesign

/// Every switch's knob clears the 3:1 glyph floor against its track when on.
/// The watch switch's white knob on #3DDC84 was about 1.8:1.
final class SwitchContrastTests: XCTestCase {
    func test_everyKnobClearsTheGlyphFloorWhenOn() {
        for kind in [GlassSwitchKind.standard, .settings, .watch] {
            let ratio = Self.contrast(GlassToggleStyle.knob(kind, isOn: true), GlassToggleStyle.onColor(kind))
            XCTAssertGreaterThanOrEqual(ratio, 3, "\(kind): \(ratio)")
        }
    }

    func test_whiteOnTheWatchGreenWouldNot() {
        XCTAssertLessThan(Self.contrast(GlassTokens.Color.textOnAccent, GlassTokens.Color.watchOn), 3)
    }

    static func contrast(_ a: GlassRGBA, _ b: GlassRGBA) -> Double {
        func lum(_ c: GlassRGBA) -> Double {
            func lin(_ v: Double) -> Double { v <= 0.04045 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4) }
            return 0.2126 * lin(c.red) + 0.7152 * lin(c.green) + 0.0722 * lin(c.blue)
        }
        let (x, y) = (lum(a), lum(b))
        return (max(x, y) + 0.05) / (min(x, y) + 0.05)
    }
}
