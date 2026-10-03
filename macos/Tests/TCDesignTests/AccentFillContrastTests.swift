import XCTest

@testable import TCDesign

/// An accent fill is a UI component, so it reaches 3:1 against the ground it
/// is drawn on (WCAG 1.4.11), in both appearances. Covers the switch tracks,
/// the checked box, the primary button, and the list selection, every stop
/// of a gradient blended over the ground. These replace the app target's
/// accent-on-ground test, which went with the legacy palette.
///
/// No pair is exempt. R-42 measured 23 below the floor when this test
/// landed: the checkbox and primary gradients' translucent dark stops (1.58
/// to 2.79) and the light watch green (1.96 to 2.59). The dark stops are now
/// purpleSoft, solid (3.33 or more), and the light watch green is statusOn's
/// light value (3.01 or more).
final class AccentFillContrastTests: XCTestCase {
    typealias RGB = LightContrastGroundsTests.RGB

    private struct Fill {
        let name: String
        let dark: GlassRGBA
        let light: GlassRGBA
    }

    private static var fills: [Fill] {
        let c = GlassTokens.Color.self
        var out: [Fill] = [
            Fill(name: "toggleOn", dark: c.toggleOn.dark, light: c.toggleOn.light),
            Fill(name: "toggleOnSettings", dark: c.toggleOnSettings.dark, light: c.toggleOnSettings.light),
            Fill(name: "watchOn", dark: c.watchOn.dark, light: c.watchOn.light),
            Fill(name: "selection", dark: c.selection.dark, light: c.selection.light),
        ]
        for (name, gradient) in [("checkboxOnFill", GlassTokens.Gradient.checkboxOnFill),
                                 ("ctaFill", GlassTokens.Gradient.ctaFill)] {
            for (index, stop) in gradient.stops.enumerated() {
                out.append(Fill(name: "\(name)[\(index)]", dark: stop.color.dark, light: stop.color.light))
            }
        }
        return out
    }

    /// Every dark ground an accent fill is drawn on, each blended over what
    /// is under it: the scene, panes, a well on a pane and on the scene,
    /// popovers, menus and node cards. Two of the light grounds are left
    /// out because no accent fill sits straight on them: sceneWarm paints
    /// only behind the first-run pane (and a debug preview), and the map
    /// field carries its controls on glass groups and node cards.
    private static var darkGrounds: [(String, RGB)] {
        let c = GlassTokens.Color.self
        let scene = LightContrastGroundsTests.solid(c.sceneBase.dark)
        let pane = LightContrastGroundsTests.solid(c.paneOpaque.dark)
        let outer = LightContrastGroundsTests.solid(c.mapFieldOuter.dark)
        return [
            ("sceneBase", scene),
            ("paneOpaque", pane),
            ("paneBase", LightContrastGroundsTests.over(c.paneBase.dark, scene)),
            ("well on pane", LightContrastGroundsTests.over(c.wellFill.dark, pane)),
            ("well on scene", LightContrastGroundsTests.over(c.wellFill.dark, scene)),
            ("popover", LightContrastGroundsTests.over(c.popoverFill.dark, scene)),
            ("menu", LightContrastGroundsTests.over(c.menuFill.dark, scene)),
            ("node card", LightContrastGroundsTests.over(c.nodeCardFill.dark, outer)),
        ]
    }

    private static func ratios(dark: Bool) -> [(String, Double)] {
        var out: [(String, Double)] = []
        for fill in fills {
            let grounds = dark ? darkGrounds : LightContrastGroundsTests.lightGrounds
            for (ground, rgb) in grounds {
                let painted = LightContrastGroundsTests.over(dark ? fill.dark : fill.light, rgb)
                out.append(("\(fill.name) on \(ground) (\(dark ? "dark" : "light"))",
                            LightContrastGroundsTests.contrast(painted, rgb)))
            }
        }
        return out
    }

    private func assertFloor(dark: Bool) {
        let measured = Self.ratios(dark: dark)
        // Every fill against every ground: a shrunken list cannot pass by
        // measuring less.
        let grounds = dark ? Self.darkGrounds.count : LightContrastGroundsTests.lightGrounds.count
        XCTAssertEqual(measured.count, Self.fills.count * grounds)
        XCTAssertGreaterThanOrEqual(Self.fills.count, 8)
        for (pair, ratio) in measured {
            XCTAssertGreaterThanOrEqual(ratio, 3, "\(pair): \(ratio)")
        }
    }

    func test_everyAccentFillReachesThreeToOneOnDark() { assertFloor(dark: true) }

    func test_everyAccentFillReachesThreeToOneOnLight() { assertFloor(dark: false) }
}
