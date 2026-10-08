import XCTest

@testable import TCDesign

/// Every switch's knob is told apart from its track when on: the knob, or
/// the ring drawn inside it, clears the 3:1 glyph floor against the track
/// (owner ruling, 2026-10-07: hold the WCAG floors). The knob is white on
/// every track, as #1146 draws it; on the bright watch green white is about
/// 1.8:1, so the watch knob carries a `switchKnobEdge` ring, which over the
/// white knob clears it.
final class SwitchContrastTests: XCTestCase {
    func test_everyKnobOrItsRingClearsTheGlyphFloorWhenOn() {
        for kind in [GlassSwitchKind.standard, .settings, .watch] {
            let track = GlassToggleStyle.onColor(kind)
            let knob = GlassToggleStyle.knob(kind, isOn: true)
            var best = Self.contrast(knob, track)
            if let edge = GlassToggleStyle.knobEdge(kind, isOn: true) {
                best = max(best, Self.contrast(ToolbarGlyphContrastTests.over(edge, knob), track))
            }
            XCTAssertGreaterThanOrEqual(best, 3, "\(kind): \(best)")
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

    /// Only the watch switch, on, needs the ring.
    func test_onlyTheWatchKnobOnCarriesTheRing() {
        XCTAssertEqual(GlassToggleStyle.knobEdge(.watch, isOn: true), GlassTokens.Color.switchKnobEdge)
        XCTAssertNil(GlassToggleStyle.knobEdge(.watch, isOn: false))
        XCTAssertNil(GlassToggleStyle.knobEdge(.standard, isOn: true))
        XCTAssertNil(GlassToggleStyle.knobEdge(.settings, isOn: true))
    }

    /// The reason for the ring: white alone on the watch green would not
    /// clear the floor.
    func test_whiteOnTheWatchGreenWouldNot() {
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
