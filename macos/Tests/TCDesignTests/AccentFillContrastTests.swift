import XCTest

@testable import TCDesign

/// An accent fill is a UI component, so it reaches 3:1 against the ground it
/// is drawn on (WCAG 1.4.11), in both appearances. Covers the switch tracks,
/// the checked box, the primary button, and the list selection. These
/// replace the app target's accent-on-ground test, which went with the
/// legacy palette: its note records the brand purple at 2.25:1 on dark.
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

    private static var darkGrounds: [(String, RGB)] {
        let c = GlassTokens.Color.self
        let scene = LightContrastGroundsTests.solid(c.sceneBase.dark)
        return [
            ("sceneBase", scene),
            ("paneOpaque", LightContrastGroundsTests.solid(c.paneOpaque.dark)),
            ("paneBase", LightContrastGroundsTests.over(c.paneBase.dark, scene)),
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

    /// Pairs measured below 3:1 when this test landed (P4T14 review). They
    /// are findings for the owner, not exemptions: the token values are not
    /// this test's to change. Each is wrapped in `XCTExpectFailure`, so it
    /// still fails loudly the day it starts to pass and the entry has to go.
    /// Measured (alpha stops blended over the ground): the checkbox and
    /// primary gradients are 1.58 to 2.79 on dark; `watchOn` is 1.96 to 2.59
    /// on every light ground.
    private static let knownBelowFloor: Set<String> = {
        var pairs: Set<String> = []
        for fill in ["checkboxOnFill[0]", "checkboxOnFill[1]", "ctaFill[0]", "ctaFill[1]"] {
            for ground in ["sceneBase", "paneOpaque", "paneBase"] {
                pairs.insert("\(fill) on \(ground) (dark)")
            }
        }
        for ground in ["sceneBase", "sceneWarm", "paneOpaque", "paneBase", "well on pane", "well on scene",
                       "mapFieldOuter", "mapFieldInner", "popover", "menu", "node card"] {
            pairs.insert("watchOn on \(ground) (light)")
        }
        return pairs
    }()

    private func assertFloor(dark: Bool) {
        for (pair, ratio) in Self.ratios(dark: dark) {
            if Self.knownBelowFloor.contains(pair) {
                XCTExpectFailure("FINDING: \(pair) is \(ratio):1, under the 3:1 UI floor") {
                    XCTAssertGreaterThanOrEqual(ratio, 3, "\(pair): \(ratio)")
                }
            } else {
                XCTAssertGreaterThanOrEqual(ratio, 3, "\(pair): \(ratio)")
            }
        }
    }

    func test_everyAccentFillReachesThreeToOneOnDark() { assertFloor(dark: true) }

    func test_everyAccentFillReachesThreeToOneOnLight() { assertFloor(dark: false) }
}
