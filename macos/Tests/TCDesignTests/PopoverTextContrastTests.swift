import XCTest

@testable import TCDesign

/// An info bubble's text reaches 4.5:1 on the popover's fill (WCAG 1.4.3) in
/// both appearances, whatever the system material under the fill is: the
/// fill is blended over black and over white, the two extremes. The first-run
/// info bubbles drew dark grey text on a dark grey ground.
final class PopoverTextContrastTests: XCTestCase {
    typealias RGB = LightContrastGroundsTests.RGB

    private let black: RGB = (0, 0, 0)
    private let white: RGB = (1, 1, 1)

    func test_popoverTextReachesFourPointFiveOnThePopoverFillInBothAppearances() {
        let c = GlassTokens.Color.self
        for (appearance, fill, inks) in [
            ("dark", c.popoverFill.dark,
             [("textPrimary", c.textPrimary.dark), ("textSecondary", c.textSecondary.dark),
              ("textSecondaryHighContrast", GlassTokens.Color.textSecondaryHighContrast.dark)]),
            ("light", c.popoverFill.light,
             [("textPrimary", c.textPrimary.light), ("textSecondary", c.textSecondary.light),
              ("textSecondaryHighContrast", GlassTokens.Color.textSecondaryHighContrast.light)]),
        ] {
            for (under, material) in [("black", black), ("white", white)] {
                let ground = LightContrastGroundsTests.over(fill, material)
                for (name, ink) in inks {
                    XCTAssertGreaterThanOrEqual(
                        LightContrastGroundsTests.contrast(LightContrastGroundsTests.solid(ink), ground), 4.5,
                        "\(name) on the popover fill over \(under) (\(appearance))")
                }
            }
        }
    }

    /// Every popover of text draws it through `glassPopoverText`, which owns
    /// the ink and the fill, so none can come back unpaired.
    func test_everyTextPopoverUsesTheSharedTextAndFill() throws {
        let components = try DesignSources.components()
        let info = try XCTUnwrap(components.first { $0.0 == "InfoButton.swift" }?.1)
        XCTAssertTrue(info.contains("func glassPopoverText()"))
        XCTAssertTrue(info.contains(".background(GlassTokens.Color.popoverFill.color)"))
        XCTAssertTrue(info.contains("Text(text).glassPopoverText()"))
        let containers = try XCTUnwrap(components.first { $0.0 == "Containers.swift" }?.1)
        XCTAssertTrue(containers.contains("Text(info).glassPopoverText()"))
    }
}
