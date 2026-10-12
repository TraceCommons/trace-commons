import XCTest
@testable import TCDesign

/// The Insights charts' lighter accent series -- the Overview and Sessions
/// cache-write series and the Patterns weekly bars -- are `accentText`
/// (`purpleText`) at `opacity.chartSeriesReduced`. Ron's #1287 review: they
/// must reach at least 3:1 against the pane under the flat theme, Light and
/// Dark (WCAG 1.4.11, graphical objects).
///
/// Composited the way `FlatThemeContrastTests` composites text, from the
/// token JSON: the worst desktop for the appearance, the veil, the pane's
/// fill, then a card's or a quiet card's fill, each also with the
/// out-of-focus `inactiveDim` painted over the veil.
final class ChartSeriesContrastTests: XCTestCase {
    static let floor = 3.0

    func test_theReducedAccentSeriesClearsThreeToOneInFlatLight() throws {
        try assertClearsTheFloor(in: "flatLight", opacity: GlassTokens.Opacity.chartSeriesReduced)
    }

    func test_theReducedAccentSeriesClearsThreeToOneInFlatDark() throws {
        try assertClearsTheFloor(in: "flatDark", opacity: GlassTokens.Opacity.chartSeriesReduced)
    }

    /// The series it replaced, accentText at 0.55, did not: the floor bites.
    func test_theFormerOpacityFailsTheFloor() throws {
        for theme in ["flatLight", "flatDark"] {
            XCTAssertLessThan(try Self.worstRatio(theme, opacity: 0.55), Self.floor, theme)
        }
    }

    /// Still a lighter step: the reduced series stays apart from the
    /// full-strength accent series stacked beside it.
    func test_theReducedSeriesStaysApartFromTheFullAccent() throws {
        XCTAssertLessThan(GlassTokens.Opacity.chartSeriesReduced, 1)
        for theme in ["flatLight", "flatDark"] {
            let accent = try FlatThemeContrastTests.colour("purpleText", theme: theme)
            var reduced = accent
            reduced.alpha *= GlassTokens.Opacity.chartSeriesReduced
            for (name, ground) in try Self.grounds(theme) {
                let ratio = FlatThemeContrastTests.contrast(
                    FlatThemeContrastTests.over(ground, reduced), FlatThemeContrastTests.over(ground, accent))
                XCTAssertGreaterThanOrEqual(ratio, 1.25, "\(theme): reduced against full accent on \(name)")
            }
        }
    }

    private func assertClearsTheFloor(in theme: String, opacity: Double) throws {
        let accent = try FlatThemeContrastTests.colour("purpleText", theme: theme)
        var series = accent
        series.alpha *= opacity
        for (name, ground) in try Self.grounds(theme) {
            let ratio = FlatThemeContrastTests.contrast(FlatThemeContrastTests.over(ground, series), ground)
            XCTAssertGreaterThanOrEqual(
                ratio, Self.floor,
                "\(theme): accentText at \(opacity) on \(name) is \(String(format: "%.2f", ratio)):1")
        }
    }

    static func worstRatio(_ theme: String, opacity: Double) throws -> Double {
        var series = try FlatThemeContrastTests.colour("purpleText", theme: theme)
        series.alpha *= opacity
        return try grounds(theme).map { _, ground in
            FlatThemeContrastTests.contrast(FlatThemeContrastTests.over(ground, series), ground)
        }.min() ?? 0
    }

    /// The pane, a card and a quiet card over the worst desktop, in focus
    /// and out of it.
    static func grounds(_ theme: String) throws -> [(String, FlatThemeContrastTests.RGB)] {
        let desktop = theme == "flatLight"
            ? FlatThemeContrastTests.RGB(r: 0.2, g: 0.2, b: 0.2)
            : FlatThemeContrastTests.RGB(r: 1, g: 1, b: 1)
        let veiled = FlatThemeContrastTests.over(desktop, try FlatThemeContrastTests.colour("glassVeil", theme: theme))
        let dim = try FlatThemeContrastTests.colour("inactiveDim", theme: theme)
        var grounds: [(String, FlatThemeContrastTests.RGB)] = []
        for (focus, base) in [("in focus", veiled), ("out of focus", FlatThemeContrastTests.over(veiled, dim))] {
            let pane = FlatThemeContrastTests.over(base, try FlatThemeContrastTests.gradientMax("paneFill", theme: theme))
            grounds.append(("pane, \(focus)", pane))
            grounds.append(("card, \(focus)",
                            FlatThemeContrastTests.over(pane, try FlatThemeContrastTests.gradientMax("cardFill", theme: theme))))
            grounds.append(("quiet card, \(focus)",
                            FlatThemeContrastTests.over(pane, try FlatThemeContrastTests.gradientMax("cardFillQuiet", theme: theme))))
        }
        return grounds
    }
}
