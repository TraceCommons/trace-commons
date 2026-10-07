import XCTest

@testable import TCDesign

/// The selection is #1146's blue, #3A7BD5, and a selected row sets its
/// title in white and its sub-line in white at 80%, as #1146 does (owner
/// ruling, 2026-10-07: #1146 wins for theming). White on it is 4.22:1, which
/// the ruling accepts over the earlier #2F6AC0 contrast fix.
@MainActor
final class SelectionContrastTests: XCTestCase {
    func test_theSelectionIsTheReferenceBlue() {
        XCTAssertEqual(GlassTokens.Color.selection.rgb, 0x3A7BD5)
        XCTAssertEqual(GlassTokens.Color.selection.rgb, GlassTokens.Color.blue.rgb)
    }

    func test_aSelectedRowsInkIsWhiteAndItsSubLineIsWhiteAt80() {
        XCTAssertEqual(GlassListRow.selectedInk, GlassTokens.Color.textOnAccent)
        XCTAssertEqual(GlassListRow.selectedSubInk.rgb, GlassListRow.selectedInk.rgb)
        XCTAssertEqual(GlassListRow.selectedSubInk.alpha, 0.8)
    }

    /// The title stays at the accepted 4.2:1 on the selection.
    func test_aSelectedRowsTitleKeepsTheAcceptedContrast() {
        let ratio = SwitchContrastTests.contrast(GlassListRow.selectedInk, GlassTokens.Color.selection)
        XCTAssertGreaterThanOrEqual(ratio, 4.2, "\(ratio)")
    }
}
