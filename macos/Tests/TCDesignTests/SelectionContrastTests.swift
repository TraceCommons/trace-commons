import XCTest

@testable import TCDesign

/// A selected row sets its title and its sub-line on the selection fill, and
/// both are small text, so both must clear 4.5:1. White on #3A7BD5 was 4.22:1,
/// and the sub-line at 0.8 opacity about 3.3:1.
final class SelectionContrastTests: XCTestCase {
    func test_aSelectedRowsInkClearsTextContrastOnTheSelection() {
        let ratio = SwitchContrastTests.contrast(GlassListRow.selectedInk, GlassTokens.Color.selection)
        XCTAssertGreaterThanOrEqual(ratio, 4.5, "\(ratio)")
    }

    func test_aSelectedRowsSubLineIsTheSameSolidInk() {
        XCTAssertEqual(GlassListRow.selectedInk.alpha, 1)
        XCTAssertEqual(GlassListRow.selectedSubInk, GlassListRow.selectedInk)
    }
}
