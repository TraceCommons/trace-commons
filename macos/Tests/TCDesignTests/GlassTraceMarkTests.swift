import CoreGraphics
import SwiftUI
import XCTest

@testable import TCDesign

/// The Trace mark's live-trace variant (Claude Design 1c).
final class GlassTraceMarkTests: XCTestCase {
    /// Every join parses, and the mark stays inside its 200-unit square.
    @MainActor
    func testTheMarkParsesAndFitsItsSquare() throws {
        let joins = try XCTUnwrap(GlassTraceMark.joinPaths)
        XCTAssertEqual(joins.count, GlassTraceMark.joins.count)
        let square = CGRect(x: 0, y: 0, width: GlassTraceMark.side, height: GlassTraceMark.side)
        XCTAssertTrue(square.contains(GlassTraceMark.trace.boundingRect))
    }

    /// The 3 by 3 lattice: five visited nodes, the current step last, and
    /// four unvisited ones, never the same point twice.
    @MainActor
    func testTheLatticeHasNineNodes() {
        XCTAssertEqual(GlassTraceMark.visited.count, 5)
        XCTAssertEqual(GlassTraceMark.unvisited.count, 4)
        let all = (GlassTraceMark.visited + GlassTraceMark.unvisited).map { "\($0.x),\($0.y)" }
        XCTAssertEqual(Set(all).count, 9)
        XCTAssertEqual(GlassTraceMark.visited.last, CGPoint(x: 125.5, y: 125.5))
    }

    /// The trace's colours are tokens the JSON names.
    func testTheTraceColoursAreTokens() {
        for name in ["traceLiveStart", "traceLiveMid", "traceLiveEnd", "purple"] {
            XCTAssertNotNil(GlassTokens.Color.all[name], name)
        }
    }
}
