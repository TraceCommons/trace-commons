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
        let origins = GlassMapMarks.origins(in: CGSize(width: 35, height: 35))
        XCTAssertEqual(origins.count, 2 * 2 * 2)
        let half = GlassMapMarks.size / 2
        XCTAssertTrue(origins.contains(CGPoint(x: 4.5 - half, y: 4.5 - half)))
        XCTAssertTrue(origins.contains(CGPoint(x: 13.5 - half, y: 13.5 - half)))
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

    /// The marks are faint at rest and stay translucent when lit.
    func test_theMarksStayTranslucent() {
        for ink in [GlassMapMarks.ink, GlassMapMarks.litInk, GlassMapMarks.bloom] {
            XCTAssertLessThan(ink.alpha, 0.5)
            XCTAssertLessThan(ink.light.alpha, 0.5)
        }
    }
}
