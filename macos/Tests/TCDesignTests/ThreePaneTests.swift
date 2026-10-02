import SwiftUI
import XCTest

@testable import TCDesign

/// R5: the window is #1146's floating three-pane layout (D9 as decided on
/// #1173), at the design's measures.
final class ThreePaneTests: XCTestCase {
    func test_theDefaultWidthIsThePanesAndTheirGaps() {
        XCTAssertEqual(GlassThreePane<EmptyView, EmptyView, EmptyView>.defaultWidth, 400 + 10 + 600 + 10 + 300)
    }

    func test_onlyTheLeadingPaneTakesTheWindowControls() {
        XCTAssertEqual(EnvironmentValues().glassWindowControlsInset, 0)
        XCTAssertGreaterThan(GlassTokens.Space.windowControlsInset, GlassTokens.Size.control - 1)
        // Wide enough for the three window buttons at their default size.
        XCTAssertGreaterThanOrEqual(GlassTokens.Space.windowControlsWidth, 70)
    }
}
