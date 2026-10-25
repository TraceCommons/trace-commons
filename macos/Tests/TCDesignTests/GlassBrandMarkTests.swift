import CoreGraphics
import SwiftUI
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

    /// The wordmark's six paths parse, "near" and "AI" side by side, and
    /// together they fill the artwork's box; "AI" takes the kit's brand blue
    /// and "near" the kit's single inks.
    @MainActor
    func test_theWordmarkParsesAndFillsItsBox() throws {
        let near = try XCTUnwrap(GlassNearWordmark.nearPath).boundingRect
        let ai = try XCTUnwrap(GlassNearWordmark.aiPath).boundingRect
        XCTAssertEqual(GlassNearWordmark.near.count, 4)
        XCTAssertEqual(GlassNearWordmark.ai.count, 2)
        XCTAssertLessThan(near.maxX, ai.minX, "\"near\" comes before \"AI\"")
        let box = GlassNearWordmark.viewBox
        let bounds = near.union(ai)
        XCTAssertTrue(box.insetBy(dx: -0.5, dy: -0.5).contains(bounds), "the wordmark spills out of its box: \(bounds)")
        XCTAssertGreaterThan(bounds.width / box.width, 0.99)
        XCTAssertGreaterThan(bounds.height / box.height, 0.99)
        XCTAssertEqual(GlassNearWordmark.brandBlue, GlassRGBA(0x0091FD))
        XCTAssertEqual(GlassTokens.Color.menuItemMark, GlassRGBA(0xFFFFFF, light: GlassRGBA(0x000000)))
    }

    /// The edge rail's Traces icon (owner, 2026-10-22): four nodes in a T,
    /// three across the top joined by one line, and one under the middle
    /// joined to it, all inside the square.
    @MainActor
    func test_theTraceNodesMakeAT() {
        let nodes = GlassTraceNodesGlyph.nodes
        XCTAssertEqual(nodes.count, 4)
        XCTAssertEqual(Set(nodes[0...2].map(\.y)).count, 1, "the top three share a row")
        XCTAssertEqual(nodes[3].x, nodes[1].x, "the fourth is under the middle")
        XCTAssertGreaterThan(nodes[3].y, nodes[1].y)
        XCTAssertLessThan(nodes[0].x, nodes[1].x)
        XCTAssertLessThan(nodes[1].x, nodes[2].x)
        XCTAssertEqual(GlassTraceNodesGlyph.links.count, 2)
        XCTAssertTrue(GlassTraceNodesGlyph.links.contains { $0 == (0, 2) })
        XCTAssertTrue(GlassTraceNodesGlyph.links.contains { $0 == (1, 3) })
        let r = GlassTraceNodesGlyph.nodeRadius
        for node in nodes {
            XCTAssertTrue(CGRect(x: 0, y: 0, width: 1, height: 1).contains(
                CGRect(x: node.x - r, y: node.y - r, width: 2 * r, height: 2 * r)), "\(node) spills out")
        }
    }

    /// The rail's N mark is the brand mark in the kit's brand blue.
    func test_theNMarkIsTheBrandBlue() throws {
        let source = try XCTUnwrap(DesignSources.all().first { $0.0 == "GlassNearWordmark.swift" }?.1)
        XCTAssertTrue(source.contains("public struct GlassNearMark: View {"))
        XCTAssertTrue(source.contains("context.fill(mark, with: .color(GlassNearWordmark.brandBlue.color))"))
        XCTAssertNotNil(GlassBrandMark.path(at: .zero, size: 20))
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
    /// middle halfway, and runs left to right as a slanted band, on a usual
    /// map and on a very wide one.
    func test_theShimmerCrossesTheDiagonal() {
        for size in [CGSize(width: 400, height: 300), CGSize(width: 1600, height: 220)] {
            let corners = [CGPoint.zero, CGPoint(x: size.width, y: 0), CGPoint(x: 0, y: size.height),
                           CGPoint(x: size.width, y: size.height)]
            // Where a point falls along the band's axis: below 0 is before
            // the band, above 1 past it.
            func position(_ p: CGPoint, _ band: (start: CGPoint, end: CGPoint)) -> CGFloat {
                let axis = CGVector(dx: band.end.x - band.start.x, dy: band.end.y - band.start.y)
                let length = axis.dx * axis.dx + axis.dy * axis.dy
                return ((p.x - band.start.x) * axis.dx + (p.y - band.start.y) * axis.dy) / length
            }
            let before = GlassMapMarks.shimmerBand(sweep: 0, size: size)
            XCTAssertTrue(corners.allSatisfy { position($0, before) >= 0.999 }, "the band starts on a \(size) field")
            let after = GlassMapMarks.shimmerBand(sweep: 0.9999, size: size)
            XCTAssertTrue(corners.allSatisfy { position($0, after) <= 0.001 }, "the band ends on a \(size) field")
            let middle = GlassMapMarks.shimmerBand(sweep: 0.5, size: size)
            let centre = CGPoint(x: size.width / 2, y: size.height / 2)
            XCTAssertEqual(position(centre, middle), 0.5, accuracy: 0.001)
            // It travels rightward, square to a band that leans its top to
            // the right by the slant.
            XCTAssertGreaterThan(middle.end.x, middle.start.x)
            let axis = atan2(middle.end.y - middle.start.y, middle.end.x - middle.start.x) * 180 / .pi
            XCTAssertEqual(axis, GlassMapMarks.shimmerSlant, accuracy: 0.001)
            XCTAssertEqual(GlassMapMarks.shimmerBand(sweep: 3.5, size: size).start, middle.start)
        }
    }

    /// One dot to a cell, halfway between two marks of a row, every other
    /// gap, the rows taking turns.
    func test_aDotSitsBetweenEveryOtherPairOfMarks() {
        let pitch = GlassMapMarks.pitch
        let size = CGSize(width: pitch * 4 - 1, height: pitch * 2 - 1)
        let dots = GlassMapMarks.dots(in: size)
        XCTAssertEqual(dots.count, 4 * 2)
        let marks = GlassMapMarks.origins(in: size).map {
            CGPoint(x: $0.x + GlassMapMarks.size / 2, y: $0.y + GlassMapMarks.size / 2)
        }
        for dot in dots {
            // Two marks of its row, half a pitch either side of it.
            let row = marks.filter { abs($0.y - dot.y) < 0.001 }
            let left = row.contains { abs($0.x - (dot.x - pitch / 2)) < 0.001 }
            let right = row.contains { abs($0.x - (dot.x + pitch / 2)) < 0.001 } || dot.x + pitch / 2 > size.width
            XCTAssertTrue(left || dot.x - pitch / 2 < 0, "no mark left of \(dot)")
            XCTAssertTrue(right, "no mark right of \(dot)")
            XCTAssertFalse(marks.contains { hypot($0.x - dot.x, $0.y - dot.y) < pitch / 4 }, "a dot sits on a mark")
        }
        // Along a row, a dot every other gap: two pitches apart.
        let firstRow = dots.filter { abs($0.y - pitch / 4) < 0.001 }.map(\.x).sorted()
        XCTAssertEqual(firstRow.count, 2)
        XCTAssertEqual(firstRow[1] - firstRow[0], pitch * 2, accuracy: 0.001)
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

    /// The first run's marker (Ron's 2026-10-21 canvas): one stroke from
    /// inside the lower-left hook to the upper-right one, wide enough that,
    /// drawn whole, it covers every point of the mark, and the shine's band
    /// rests clear of the mark at both ends of its travel.
    func test_theMarkerCoversTheMarkAndTheShineRestsClearOfIt() throws {
        let line = GlassBrandMark.markerLine
        XCTAssertEqual(line.first, CGPoint(x: 167.5, y: 234.6))
        XCTAssertEqual(line.last, CGPoint(x: 226, y: 170))
        XCTAssertEqual(GlassBrandMark.markerLength, 983.4, accuracy: 1)
        XCTAssertEqual(GlassBrandMark.markerPoint(at: 0), line[0])
        XCTAssertEqual(GlassBrandMark.markerPoint(at: 10_000), line[line.count - 1])
        XCTAssertEqual(GlassBrandMark.markerPoint(at: -5), line[0])

        let mark = try XCTUnwrap(SVGPathData.parse(GlassBrandMark.data))
        var stroke = Path()
        stroke.addLines(line)
        let covered = stroke.strokedPath(
            StrokeStyle(lineWidth: GlassBrandMark.markerWidth, lineCap: .round, lineJoin: .round))
        let box = GlassBrandMark.viewBox
        var missed = 0
        for x in stride(from: box.minX, through: box.maxX, by: 4) {
            for y in stride(from: box.minY, through: box.maxY, by: 4) {
                let point = CGPoint(x: x, y: y)
                if mark.contains(point), !covered.contains(point) { missed += 1 }
            }
        }
        XCTAssertEqual(missed, 0, "the marker leaves part of the mark uninked")

        for end in [GlassBrandMark.shineTravel.lowerBound, GlassBrandMark.shineTravel.upperBound] {
            let band = GlassBrandMark.shineBand(at: end).band.boundingRect
            XCTAssertFalse(band.intersects(box), "the shine rests on the mark at \(end)")
        }
    }

    /// The light leans with its band (Ron, 2026-10-22): the gradient runs
    /// square to the band's edge, from its left edge to its right, so its
    /// brightest line is the band's forward-leaning centre, not upright.
    func test_theShineLeansForward() {
        let strip = GlassBrandMark.shineBand(at: 100)
        let lean = tan(18 * CGFloat.pi / 180)
        // The band's edge, going down: x falls as y grows.
        let edge = CGVector(dx: -lean, dy: 1)
        let across = CGVector(dx: strip.to.x - strip.from.x, dy: strip.to.y - strip.from.y)
        XCTAssertEqual(edge.dx * across.dx + edge.dy * across.dy, 0, accuracy: 0.0001, "square to the edge")
        XCTAssertGreaterThan(across.dy, 0, "not a level, upright gradient")
        // `to` lies on the right edge: x + 110 at its own height.
        XCTAssertEqual(strip.to.x, 100 + 110 - lean * strip.to.y, accuracy: 0.0001)
    }
}
