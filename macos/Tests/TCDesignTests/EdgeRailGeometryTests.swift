import CoreGraphics
import SwiftUI
@testable import TCDesign
import XCTest

/// Where the edge rail's panel sits, from its tokens.
final class EdgeRailGeometryTests: XCTestCase {
    private let screen = CGRect(x: 0, y: 0, width: 1512, height: 944)

    /// Closed, the rail is a thin zone on the right edge and nothing more.
    func testTheClosedRailCoversOnlyTheEdge() {
        let frame = EdgeRailGeometry.frame(open: false, visible: screen)
        XCTAssertEqual(frame.maxX, screen.maxX)
        XCTAssertEqual(frame.width, GlassTokens.Size.edgeRailZoneWidth)
        XCTAssertEqual(frame.midY, screen.midY)
    }

    /// Open, the rail and its peek fit against the edge, centred.
    func testTheOpenRailHoldsTheRailAndItsPeek() {
        let frame = EdgeRailGeometry.frame(open: true, visible: screen)
        XCTAssertEqual(frame.maxX, screen.maxX)
        XCTAssertGreaterThanOrEqual(
            frame.width,
            GlassTokens.Space.edgeRailInset + GlassTokens.Size.edgeRailWidth + GlassTokens.Space.edgeRailGap
                + GlassTokens.Size.edgeRailPeekWidth)
        XCTAssertEqual(frame.midY, screen.midY)
    }

    /// A screen shorter than the open rail clips the panel to the screen.
    func testTheRailNeverOutgrowsAShortScreen() {
        let short = CGRect(x: 0, y: 25, width: 1024, height: 400)
        let frame = EdgeRailGeometry.frame(open: true, visible: short)
        XCTAssertEqual(frame.height, short.height)
        XCTAssertEqual(frame.minY, short.minY)
    }

    /// A rail tile follows the control tiers' spec: the selection's label is
    /// `textOnSelected`, as on a segmented tab, and an idle tile's is primary.
    func testARailTileTakesTheSelectionInk() {
        XCTAssertEqual(GlassRailTile<EmptyView>.ink(selected: true), GlassTokens.Color.textOnSelected)
        XCTAssertEqual(GlassRailTile<EmptyView>.ink(selected: false), GlassTokens.Color.textPrimary)
    }

    /// Every edge rail measure is a token the JSON names.
    func testTheRailMeasuresAreTokens() {
        for name in ["edgeRailWidth", "edgeRailTile", "edgeRailRuleWidth", "edgeRailPeekWidth", "edgeRailOpenHeight",
                     "edgeRailZoneWidth", "edgeRailZoneHeight", "edgeRailHandleWidth", "edgeRailHandleHeight"] {
            XCTAssertNotNil(GlassTokens.Size.all[name], name)
        }
        XCTAssertNotNil(GlassTokens.Space.all["edgeRailInset"])
        XCTAssertNotNil(GlassTokens.Space.all["edgeRailGap"])
        XCTAssertNotNil(GlassTokens.Color.all["edgeRailHandle"])
    }
}
