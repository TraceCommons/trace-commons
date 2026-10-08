import XCTest

@testable import TCDesign

/// A selected row sets its title and its sub-line on the selection fill, and
/// both are small text, so both must clear 4.5:1 in both appearances (owner
/// ruling, 2026-10-07: hold the WCAG floors). #1146's selection is its blue,
/// #3A7BD5, on which white is 4.22:1, and its sub-line is white at 80%,
/// about 3.3:1; the dark selection is that blue darkened the least, at the
/// same hue, that white clears the floor, and the sub-line is solid white.
@MainActor
final class SelectionContrastTests: XCTestCase {
    func test_aSelectedRowsInkClearsTextContrastOnTheSelection() {
        let c = GlassTokens.Color.selection
        for (name, fill) in [("dark", c.dark), ("light", c.light)] {
            let ink = name == "dark" ? GlassListRow.selectedInk.dark : GlassListRow.selectedInk.light
            let ratio = SwitchContrastTests.contrast(ink, fill)
            XCTAssertGreaterThanOrEqual(ratio, 4.5, "\(name): \(ratio)")
        }
    }

    func test_aSelectedRowsSubLineIsTheSameSolidInk() {
        XCTAssertEqual(GlassListRow.selectedInk, GlassTokens.Color.textOnAccent)
        XCTAssertEqual(GlassListRow.selectedInk.alpha, 1)
        XCTAssertEqual(GlassListRow.selectedSubInk, GlassListRow.selectedInk)
    }

    /// The dark selection keeps #1146's blue hue: only its lightness moved.
    func test_theDarkSelectionKeepsTheReferenceHue() {
        func hue(_ c: GlassRGBA) -> Double {
            let (r, g, b) = (c.red, c.green, c.blue)
            let (hi, lo) = (max(r, g, b), min(r, g, b))
            let d = hi - lo
            guard d > 0 else { return 0 }
            let h: Double = hi == r ? (g - b) / d : hi == g ? (b - r) / d + 2 : (r - g) / d + 4
            return (h * 60 + 360).truncatingRemainder(dividingBy: 360)
        }
        let selection = GlassTokens.Color.selection.dark
        let blue = GlassTokens.Color.blue.dark
        XCTAssertEqual(hue(selection), hue(blue), accuracy: 1)
        XCTAssertLessThan(SwitchContrastTests.contrast(GlassTokens.Color.textOnAccent, blue), 4.5,
                          "the reason for the nudge: white on #1146's blue")
        XCTAssertEqual(GlassTokens.Color.selection.light, GlassTokens.Color.blue.light)
    }
}
