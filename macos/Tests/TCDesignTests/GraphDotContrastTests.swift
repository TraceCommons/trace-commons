import XCTest

@testable import TCDesign

/// The bar graph's lit dots are its marks, so each clears the 3:1 non-text
/// floor against the pane and stands apart from an unlit dot (2:1, resting
/// or under the pointer), in every appearance. Composited over paneOpaque:
/// classic dark and light, and the flat theme's dark pane.
final class GraphDotContrastTests: XCTestCase {
    typealias RGB = LightContrastGroundsTests.RGB

    /// Every level `dotAlphas` gives a lit dot, across counts that reach
    /// the partial-dot floor and the faintest row of a full column.
    static var litLevels: [Double] {
        let values = stride(from: 0.25, through: 40, by: 0.25)
        return values.flatMap { GlassBarGraph.dotAlphas($0, of: 40) }.filter { $0 > 0 }
    }

    func test_everyLitDotClearsThePaneAndAnUnlitDot() throws {
        let c = GlassTokens.Color.self
        let flatPane = try FlatThemeContrastTests.colour("paneOpaque", theme: "flatDark").rgb
        let appearances: [(String, RGB, Bool)] = [
            ("dark", LightContrastGroundsTests.solid(c.paneOpaque.dark), true),
            ("light", LightContrastGroundsTests.solid(c.paneOpaque.light), false),
            ("flat dark", (flatPane.r, flatPane.g, flatPane.b), true),
        ]
        let levels = Self.litLevels
        XCTAssertEqual(levels.min() ?? 1, 0.35, accuracy: 0.0001, "the sweep misses the faintest lit dot")
        for (name, pane, dark) in appearances {
            let ink = dark ? c.ink.dark : c.ink.light
            let unlit = [0.09, 0.2].map { LightContrastGroundsTests.over(ink.opacity($0), pane) }
            for data in [c.dataShared, c.dataKept] {
                let colour = dark ? data.dark : data.light
                for level in levels {
                    let (alpha, lift) = GlassBarGraph.dotInk(level, dark: dark)
                    let dot = LightContrastGroundsTests.over(
                        ink.opacity(lift), LightContrastGroundsTests.over(colour.opacity(alpha), pane))
                    let ratio = LightContrastGroundsTests.contrast(dot, pane)
                    XCTAssertGreaterThanOrEqual(ratio, 3, "\(name): a lit dot at \(level) is \(String(format: "%.2f", ratio)):1 on the pane")
                    for ground in unlit {
                        let apart = LightContrastGroundsTests.contrast(dot, ground)
                        XCTAssertGreaterThanOrEqual(apart, 2, "\(name): a lit dot at \(level) is \(String(format: "%.2f", apart)):1 on an unlit dot")
                    }
                }
            }
        }
    }

    /// The taper still brightens toward the tip: a higher level is never
    /// drawn fainter (light) or less lifted (dark).
    func test_theTaperStillBrightensTowardTheTip() {
        let levels = Self.litLevels.sorted()
        for (lower, higher) in zip(levels, levels.dropFirst()) {
            XCTAssertLessThanOrEqual(GlassBarGraph.dotInk(lower, dark: false).alpha, GlassBarGraph.dotInk(higher, dark: false).alpha)
            XCTAssertLessThanOrEqual(GlassBarGraph.dotInk(lower, dark: true).lift, GlassBarGraph.dotInk(higher, dark: true).lift)
        }
        XCTAssertLessThan(GlassBarGraph.dotInk(0.35, dark: false).alpha, GlassBarGraph.dotInk(1, dark: false).alpha)
        XCTAssertLessThan(GlassBarGraph.dotInk(0.35, dark: true).lift, GlassBarGraph.dotInk(1, dark: true).lift)
    }
}
