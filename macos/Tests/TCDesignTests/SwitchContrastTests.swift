import XCTest

@testable import TCDesign

/// Every switch's knob is told apart from its track when on: the plain
/// white knob clears the 3:1 glyph floor against the track (owner ruling,
/// 2026-10-07: hold the WCAG floors). No knob carries a ring (owner,
/// 2026-10-08), so the watch switch takes the Settings purple rather than
/// the watch green, on which white is about 1.8:1.
final class SwitchContrastTests: XCTestCase {
    func test_everyKnobClearsTheGlyphFloorWhenOn() {
        for kind in [GlassSwitchKind.standard, .settings, .watch] {
            let track = GlassToggleStyle.onColor(kind)
            let knob = GlassToggleStyle.knob(kind, isOn: true)
            let contrast = Self.contrast(knob, track)
            XCTAssertGreaterThanOrEqual(contrast, 3, "\(kind): \(contrast)")
        }
    }

    /// The knob is white on every track.
    func test_theKnobIsWhiteOnEveryTrack() {
        for kind in [GlassSwitchKind.standard, .settings, .watch] {
            for isOn in [true, false] {
                XCTAssertEqual(GlassToggleStyle.knob(kind, isOn: isOn), GlassTokens.Color.textOnAccent)
            }
        }
    }

    /// The watch switch matches the Settings switches; white on the watch
    /// green would not clear the floor without a ring.
    func test_theWatchSwitchMatchesTheOthers() {
        XCTAssertEqual(GlassToggleStyle.onColor(.watch), GlassToggleStyle.onColor(.settings))
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
