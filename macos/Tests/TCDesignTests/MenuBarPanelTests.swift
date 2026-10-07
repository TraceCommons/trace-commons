import SwiftUI
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

    /// A core that is down or unread draws flat grey bars, no badge and the
    /// attention mark, never the last activity; trouble keeps the bars and
    /// adds the mark; live data draws as recorded.
    func test_anUnavailableCoreIsNeverDrawnAsActivity() {
        let columns = (0 ..< 7).map { GlassDayColumn(id: String($0), up: $0, down: 7 - $0) }
        let down = GlassMenuBarStrip.bars(columns, condition: .unavailable)
        XCTAssertTrue(down.allSatisfy { $0.up == 2 && $0.down == 2 })
        XCTAssertNil(GlassMenuBarStrip.shownBadge(4, condition: .unavailable))
        XCTAssertTrue(GlassMenuBarStrip.showsAttention(.unavailable))
        XCTAssertTrue(GlassMenuBarStrip.grey(.unavailable))

        let live = GlassMenuBarStrip.bars(columns, condition: .live)
        XCTAssertTrue(live.contains { $0.up > 2 })
        XCTAssertEqual(GlassMenuBarStrip.shownBadge(4, condition: .live), 4)
        XCTAssertFalse(GlassMenuBarStrip.showsAttention(.live))
        XCTAssertFalse(GlassMenuBarStrip.grey(.live))

        XCTAssertEqual(GlassMenuBarStrip.bars(columns, condition: .attention).map(\.up), live.map(\.up))
        XCTAssertTrue(GlassMenuBarStrip.showsAttention(.attention))
        XCTAssertTrue(GlassMenuBarStrip.grey(.paused))
    }

    /// A menu row's own hit rect clears the 28pt target: the room between
    /// two selections belongs to a row, not to a gap no button owns.
    @MainActor
    func test_aMenuRowsHitRectClearsTwentyEightPoints() {
        let row = Button("Quit…") {}.buttonStyle(GlassMenuRowStyle()).frame(width: 240)
        let height = NSHostingView(rootView: row.fixedSize(horizontal: false, vertical: true)).fittingSize.height
        XCTAssertGreaterThanOrEqual(height, 28)
    }
}
