import CoreGraphics
import XCTest

@testable import TCDesign

/// The NEAR AI mark parses and fills its box, and the map's mark tile
/// covers the field and lights only around the pointer.
final class GlassBrandMarkTests: XCTestCase {
    func test_theMarkParsesAndFillsItsBox() throws {
        let parsed = try XCTUnwrap(SVGPathData.parse(GlassBrandMark.data))
        let box = GlassBrandMark.viewBox
        let bounds = parsed.boundingBoxOfPath
        XCTAssertTrue(box.insetBy(dx: -0.1, dy: -0.1).contains(bounds), "the mark spills out of its box: \(bounds)")
        XCTAssertGreaterThan(bounds.width / box.width, 0.99)
        XCTAssertGreaterThan(bounds.height / box.height, 0.99)
    }

    func test_theMarkIsDrawnAtItsSizeAndPlace() throws {
        let mark = try XCTUnwrap(GlassBrandMark.path(at: CGPoint(x: 10, y: 20), size: 4.2)).boundingRect
        XCTAssertEqual(mark.minX, 10, accuracy: 0.01)
        XCTAssertEqual(mark.minY, 20, accuracy: 0.01)
        XCTAssertEqual(max(mark.width, mark.height), 4.2, accuracy: 0.01)
    }

    /// Two marks to a cell, on its diagonal, every cell of the field drawn.
    func test_theTileCoversTheField() {
        let pitch = GlassMapMarks.pitch
        let origins = GlassMapMarks.origins(in: CGSize(width: pitch * 2 - 1, height: pitch * 2 - 1))
        XCTAssertEqual(origins.count, 2 * 2 * 2)
        let half = GlassMapMarks.size / 2
        XCTAssertTrue(origins.contains(CGPoint(x: pitch / 4 - half, y: pitch / 4 - half)))
        XCTAssertTrue(origins.contains(CGPoint(x: pitch * 3 / 4 - half, y: pitch * 3 / 4 - half)))
        XCTAssertEqual(GlassMapMarks.origins(in: .zero), [])
    }

    /// The light only visits the cells around the pointer.
    func test_theLightVisitsOnlyTheCellsNearThePointer() {
        let field = CGSize(width: 1000, height: 800)
        let near = GlassMapMarks.origins(in: field, near: CGRect(x: 100, y: 100, width: 36, height: 36))
        XCTAssertLessThan(near.count, GlassMapMarks.origins(in: field).count / 50)
        XCTAssertEqual(GlassMapMarks.origins(in: field, near: CGRect(x: 2000, y: 2000, width: 10, height: 10)), [])
    }

    func test_theLightFallsOffToNothingAtItsReach() {
        XCTAssertEqual(GlassMapMarks.light(distance: 0), 1, accuracy: 0.0001)
        XCTAssertEqual(GlassMapMarks.light(distance: GlassMapMarks.reach), 0)
        XCTAssertEqual(GlassMapMarks.light(distance: GlassMapMarks.reach * 2), 0)
        let steps = stride(from: 0, to: GlassMapMarks.reach, by: 10).map { GlassMapMarks.light(distance: $0) }
        XCTAssertEqual(steps, steps.sorted(by: >))
    }

    /// A pass starts and ends with the band off the field, crosses its
    /// middle halfway, runs top-left to bottom-right, and lies turned
    /// counterclockwise from square to the diagonal.
    func test_theShimmerCrossesTheDiagonal() {
        let size = CGSize(width: 400, height: 300)
        let corners = [CGPoint.zero, CGPoint(x: 400, y: 0), CGPoint(x: 0, y: 300), CGPoint(x: 400, y: 300)]
        // Where a point falls along the band's axis: below 0 is before the
        // band, above 1 past it.
        func position(_ p: CGPoint, _ band: (start: CGPoint, end: CGPoint)) -> CGFloat {
            let axis = CGVector(dx: band.end.x - band.start.x, dy: band.end.y - band.start.y)
            let length = axis.dx * axis.dx + axis.dy * axis.dy
            return ((p.x - band.start.x) * axis.dx + (p.y - band.start.y) * axis.dy) / length
        }
        let before = GlassMapMarks.shimmerBand(sweep: 0, size: size)
        XCTAssertTrue(corners.allSatisfy { position($0, before) >= 1 }, "the band starts on the field")
        let after = GlassMapMarks.shimmerBand(sweep: 0.9999, size: size)
        XCTAssertTrue(corners.allSatisfy { position($0, after) <= 0 }, "the band ends on the field")
        let middle = GlassMapMarks.shimmerBand(sweep: 0.5, size: size)
        XCTAssertEqual((middle.start.x + middle.end.x) / 2, 200, accuracy: 0.001)
        XCTAssertEqual((middle.start.y + middle.end.y) / 2, 150, accuracy: 0.001)
        // The axis is the diagonal turned 5 degrees counterclockwise: up on
        // screen, which is a smaller angle below the horizontal.
        let axis = atan2(middle.end.y - middle.start.y, middle.end.x - middle.start.x) * 180 / .pi
        let diagonal = atan2(300.0, 400.0) * 180 / .pi
        XCTAssertEqual(diagonal - axis, GlassMapMarks.shimmerTilt, accuracy: 0.001)
        // A later pass is the same pass.
        XCTAssertEqual(GlassMapMarks.shimmerBand(sweep: 3.5, size: size).start, middle.start)
    }

    /// The first pass comes soon after launch and a pass is quick.
    func test_theShimmerIsSeenOnLaunchAndIsQuick() {
        XCTAssertLessThanOrEqual(GlassMapMarks.shimmerFirst, .seconds(2))
        XCTAssertLessThanOrEqual(GlassMapMarks.shimmerDuration, 1.5)
        XCTAssertGreaterThan(GlassMapMarks.shimmerEvery, GlassMapMarks.shimmerFirst)
    }

    /// The marks are faint at rest and stay translucent when lit.
    func test_theMarksStayTranslucent() {
        for ink in [GlassMapMarks.ink, GlassMapMarks.litInk, GlassMapMarks.bloom, GlassMapMarks.shimmerInk] {
            XCTAssertLessThan(ink.alpha, 0.5)
            XCTAssertLessThan(ink.light.alpha, 0.5)
        }
    }
}
