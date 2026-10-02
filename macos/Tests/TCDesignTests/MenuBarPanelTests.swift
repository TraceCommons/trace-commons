import XCTest

@testable import TCDesign

/// R13: the menu-bar popover's graph and strip draw counts honestly.
final class MenuBarPanelTests: XCTestCase {
    /// A day with nothing draws no bar; any count draws at least 6pt, and
    /// more draws taller.
    func test_aZeroDrawsNothingAndAnyCountIsVisible() {
        XCTAssertEqual(GlassDayGraph.height(0, max: 5), 0)
        XCTAssertGreaterThanOrEqual(GlassDayGraph.height(1, max: 100), 6)
        XCTAssertGreaterThan(GlassDayGraph.height(5, max: 5), GlassDayGraph.height(1, max: 5))
        XCTAssertLessThanOrEqual(GlassDayGraph.height(5, max: 5), GlassDayGraph.plotHeight / 2)
    }

    /// The strip's bars stay between 2pt and 9pt, as the handoff sizes them.
    func test_theStripStaysInItsTwentyTwoPoints() {
        XCTAssertEqual(GlassMenuBarStrip.height(0, max: 0), 2)
        XCTAssertEqual(GlassMenuBarStrip.height(0, max: 4), 2)
        XCTAssertEqual(GlassMenuBarStrip.height(4, max: 4), 9)
    }
}
