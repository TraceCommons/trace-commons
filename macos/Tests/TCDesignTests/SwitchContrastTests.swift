import XCTest

@testable import TCDesign

/// The switch knob is plain white on every track, the watch switch's
/// too (owner ruling, 2026-10-07: white watch knob, as #1146 draws it).
/// The standard and Settings knobs clear the 3:1 glyph floor against their
/// track; the watch knob on the bright dark-appearance green is about
/// 1.8:1, which the ruling accepts, and the earlier ring is gone.
final class SwitchContrastTests: XCTestCase {
    func test_theToggleKnobsClearTheGlyphFloorWhenOn() {
        for kind in [GlassSwitchKind.standard, .settings] {
            let ratio = Self.contrast(GlassToggleStyle.knob(kind, isOn: true), GlassToggleStyle.onColor(kind))
            XCTAssertGreaterThanOrEqual(ratio, 3, "\(kind): \(ratio)")
        }
    }

    /// The knob is white on every track, with nothing drawn around it.
    func test_theKnobIsWhiteOnEveryTrack() {
        for kind in [GlassSwitchKind.standard, .settings, .watch] {
            for isOn in [true, false] {
                XCTAssertEqual(GlassToggleStyle.knob(kind, isOn: isOn), GlassTokens.Color.textOnAccent)
            }
        }
        XCTAssertNil(GlassTokens.Color.all["switchKnobEdge"])
    }

    /// The accepted cost of the ruling, recorded so a token change that
    /// moves it is seen: white on the dark watch green is under 3:1.
    func test_whiteOnTheDarkWatchGreenIsTheAcceptedException() {
        XCTAssertLessThan(Self.contrast(GlassTokens.Color.textOnAccent, GlassTokens.Color.watchOn), 3)
    }

    /// #1146's timings: a toggle slides in `--tc-dur` (220ms), the watch
    /// switch in 150ms.
    func test_switchDurationsFollowTheReference() {
        XCTAssertEqual(GlassToggleStyle.duration(.standard), GlassTokens.Motion.standard)
        XCTAssertEqual(GlassToggleStyle.duration(.settings), GlassTokens.Motion.standard)
        XCTAssertEqual(GlassToggleStyle.duration(.watch), GlassTokens.Motion.fast)
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
