import XCTest

@testable import TCDesign

/// Every switch's knob is told apart from its track when on: the knob
/// clears the 3:1 glyph floor against the track (owner ruling, 2026-10-07:
/// hold the WCAG floors). No knob carries a ring (owner, 2026-10-08), so the
/// watch switch takes the Settings accent rather than the watch green, on
/// which white is about 1.8:1. Off, the knob is white; on, it is the
/// accent's ink, as the NEAR AI design system's switch draws it (owner,
/// 2026-10-20). The flat theme's tracks are measured in `NearAccentTests`.
final class SwitchContrastTests: XCTestCase {
    @MainActor
    func test_everyKnobClearsTheGlyphFloorWhenOn() {
        for kind in [GlassSwitchKind.standard, .settings, .watch] {
            let track = GlassToggleStyle.onColor(kind)
            let knob = GlassToggleStyle.knob(kind, isOn: true)
            let contrast = Self.contrast(knob, track)
            XCTAssertGreaterThanOrEqual(contrast, 3, "\(kind): \(contrast)")
        }
    }

    /// The knob is white while off and the accent's ink while on, on every
    /// track.
    @MainActor
    func test_theKnobIsWhiteOffAndTheAccentInkOn() {
        XCTAssertEqual(GlassTokens.Color.toggleKnobOff.rgb, 0xFFFFFF)
        for kind in [GlassSwitchKind.standard, .settings, .watch] {
            XCTAssertEqual(GlassToggleStyle.knob(kind, isOn: false), GlassTokens.Color.toggleKnobOff)
            XCTAssertEqual(GlassToggleStyle.knob(kind, isOn: true), GlassTokens.Color.textOnAccent)
        }
    }

    /// The flat theme's Dark draws the off knob the same navy as the on
    /// knob (owner, 2026-10-22), on an off track light enough that the knob
    /// still clears the 3:1 glyph floor on a pane or a card; Light keeps
    /// the white knob.
    func test_theFlatDarkOffKnobIsTheOnKnobsNavy() throws {
        typealias Flat = FlatThemeContrastTests
        let knob = try Flat.colour("toggleKnobOff", theme: "flatDark").rgb
        XCTAssertEqual(knob, try Flat.colour("textOnAccent", theme: "flatDark").rgb)
        let pane = try Flat.colour("paneOpaque", theme: "flatDark").rgb
        for ground in [pane, Flat.over(pane, try Flat.gradientMax("cardFill", theme: "flatDark"))] {
            let track = Flat.over(ground, try Flat.colour("toggleOff", theme: "flatDark"))
            XCTAssertGreaterThanOrEqual(Flat.contrast(knob, track), 3)
        }
        XCTAssertEqual(try Flat.colour("toggleKnobOff", theme: "flatLight").rgb, Flat.RGB(r: 1, g: 1, b: 1))
    }

    /// The watch switch matches the Settings switches; white on the watch
    /// green would not clear the floor without a ring.
    @MainActor
    func test_theWatchSwitchMatchesTheOthers() {
        XCTAssertEqual(GlassToggleStyle.onColor(.watch), GlassToggleStyle.onColor(.settings))
        XCTAssertLessThan(Self.contrast(GlassTokens.Color.textOnAccent, GlassTokens.Color.watchOn), 3)
    }

    /// #1146's timings: a toggle slides in `--tc-dur` (220ms), the watch
    /// switch in 150ms.
    @MainActor
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
