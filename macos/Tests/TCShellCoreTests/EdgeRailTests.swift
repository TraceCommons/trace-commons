import CoreGraphics
import Foundation
import TCShellCore
import XCTest

/// The edge rail's switch and where its panel sits.
final class EdgeRailTests: XCTestCase {
    private var defaults: UserDefaults!
    private let suite = "EdgeRailTests"

    override func setUp() {
        super.setUp()
        defaults = UserDefaults(suiteName: suite)
        defaults.removePersistentDomain(forName: suite)
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suite)
        super.tearDown()
    }

    /// Off until the contributor turns it on, and remembered after.
    func testTheRailIsOffUntilTurnedOn() {
        XCTAssertFalse(EdgeRailPreference.isEnabled(defaults))
        EdgeRailPreference.set(true, defaults)
        XCTAssertTrue(EdgeRailPreference.isEnabled(defaults))
        EdgeRailPreference.set(false, defaults)
        XCTAssertFalse(EdgeRailPreference.isEnabled(defaults))
    }

    private let screen = CGRect(x: 0, y: 0, width: 1512, height: 944)

    /// Closed, the rail is a thin zone on the right edge and nothing more.
    func testTheClosedRailCoversOnlyTheEdge() {
        let frame = EdgeRailGeometry.frame(open: false, visible: screen)
        XCTAssertEqual(frame.maxX, screen.maxX)
        XCTAssertEqual(frame.width, EdgeRailGeometry.zoneWidth)
        XCTAssertEqual(frame.midY, screen.midY)
    }

    /// Open, the rail and its peek fit against the edge, centred.
    func testTheOpenRailHoldsTheRailAndItsPeek() {
        let frame = EdgeRailGeometry.frame(open: true, visible: screen)
        XCTAssertEqual(frame.maxX, screen.maxX)
        XCTAssertGreaterThanOrEqual(
            frame.width,
            EdgeRailGeometry.edgeInset + EdgeRailGeometry.railWidth + EdgeRailGeometry.peekGap + EdgeRailGeometry.peekWidth)
        XCTAssertEqual(frame.midY, screen.midY)
    }

    /// A screen shorter than the open rail clips the panel to the screen.
    func testTheRailNeverOutgrowsAShortScreen() {
        let short = CGRect(x: 0, y: 25, width: 1024, height: 400)
        let frame = EdgeRailGeometry.frame(open: true, visible: short)
        XCTAssertEqual(frame.height, short.height)
        XCTAssertEqual(frame.minY, short.minY)
    }
}
