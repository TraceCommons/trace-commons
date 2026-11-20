import CoreGraphics
import simd
import SwiftUI
import XCTest

@testable import TCDesign

/// The Traces map's library station (owner, 2026-10-16): a mesh built
/// round the NEAR AI mark, turned as a solid, never a flat picture.
final class LibraryStationTests: XCTestCase {
    /// The mark's outline is the brand path, flattened, at the asked
    /// height, centred on the station's axis.
    @MainActor
    func test_theMarksOutlineIsTheBrandPathAtItsHeight() {
        let outline = GlassLibraryStation.markOutline(height: 30)
        XCTAssertGreaterThan(outline.count, 20)
        let ys = outline.map(\.y), xs = outline.map(\.x)
        XCTAssertEqual((ys.max() ?? 0) - (ys.min() ?? 0), 30, accuracy: 0.6)
        XCTAssertEqual(((xs.max() ?? 0) + (xs.min() ?? 0)) / 2, 0, accuracy: 0.6)
        XCTAssertEqual(((ys.max() ?? 0) + (ys.min() ?? 0)) / 2, 0, accuracy: 0.6)
    }

    /// The mesh has depth: the mark's two faces and its sides, a ring, and
    /// parts on every side of the axis.
    @MainActor
    func test_theMeshIsASolidRoundTheMark() {
        let mesh = GlassLibraryStation.mesh
        XCTAssertEqual(mesh.filter(\.markCap).count, 2)
        XCTAssertGreaterThan(mesh.filter { $0.mark && !$0.markCap }.count, 20, "the mark has no sides")
        let points = mesh.flatMap(\.points)
        let ys = points.map(\.y)
        XCTAssertGreaterThan((ys.max() ?? 0) - (ys.min() ?? 0), 6, "the mesh is flat")
        XCTAssertLessThan(points.map(\.x).min() ?? 0, -36)
        XCTAssertGreaterThan(points.map(\.x).max() ?? 0, 36)
        XCTAssertTrue(mesh.allSatisfy { $0.points.count >= 3 })
    }

    /// The standard map's orb: its meridians turn, each between edge-on
    /// and full width, half of them on the near side, and its sphere
    /// leaves room for the glow.
    @MainActor
    func test_theOrbsMeridiansTurn() {
        let rest = GlassLibraryOrb.meridianWidths(angle: 0)
        let turned = GlassLibraryOrb.meridianWidths(angle: 0.5)
        XCTAssertEqual(rest.count, 6)
        XCTAssertTrue(rest.allSatisfy { (0 ... 1).contains($0.width) })
        XCTAssertNotEqual(rest.map(\.width), turned.map(\.width))
        XCTAssertEqual(GlassLibraryOrb.radius(in: CGSize(width: 104, height: 104)), 52 * (1 - GlassLibraryOrb.inset),
                       accuracy: 0.0001)
        XCTAssertLessThan(GlassLibraryOrb.radius(in: CGSize(width: 104, height: 80)), 40)
    }

    /// Turning is a rotation: it keeps every point's distance from the
    /// centre, and a quarter turn moves the parts.
    @MainActor
    func test_turningKeepsTheStationsShape() {
        for p in [SIMD3<Double>(36, 0, 0), SIMD3(0, 3.2, -15), SIMD3(-20, 1, 12)] {
            for angle in [0.0, 0.6, 2.1] {
                XCTAssertEqual(simd_length(GlassLibraryStation.turned(p, angle: angle)), simd_length(p), accuracy: 1e-9)
            }
        }
        let p = SIMD3<Double>(36, 0, 0)
        XCTAssertGreaterThan(
            simd_distance(GlassLibraryStation.turned(p, angle: 0), GlassLibraryStation.turned(p, angle: .pi / 2)), 30)
    }

    /// Before anything is contributed the station is drawn as bright as
    /// after (owner, 2026-10-19: dimmed, it read too dark beside the launch
    /// pad); only the glow is softer.
    @MainActor
    func test_theRestingStationIsDrawnAtFullBrightness() throws {
        let source = try String(
            contentsOf: URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
                .deletingLastPathComponent().appendingPathComponent("Sources/TCDesign/Components/LibraryStation.swift"),
            encoding: .utf8)
        XCTAssertFalse(source.contains("let dim ="), "the station is dimmed at rest again")
        XCTAssertGreaterThan(GlassLibraryStation.glow(lit: false), 0)
        XCTAssertLessThan(GlassLibraryStation.glow(lit: false), GlassLibraryStation.glow(lit: true))
    }

    /// The ring lies in the plane the station turns in, so every turn of
    /// it projects to the same outline: what lets its glow be drawn once.
    @MainActor
    func test_theRingsOutlineIsTheSameAtEveryAngle() {
        let size = CGSize(width: 150, height: 130)
        let radius = GlassLibraryStation.ringRadius
        for a in stride(from: 0.0, to: 2 * Double.pi, by: 0.37) {
            let p = SIMD3<Double>(radius * cos(a), 0, radius * sin(a))
            let turned = GlassLibraryStation.turned(p, angle: 1.3)
            let projected = GlassLibraryStation.project(turned, size: size)
            let onRing = (0 ..< 720).contains { step in
                let b = Double(step) / 720 * 2 * .pi
                let q = GlassLibraryStation.project(
                    GlassLibraryStation.turned(SIMD3(radius * cos(b), 0, radius * sin(b)), angle: 0), size: size)
                return hypot(q.x - projected.x, q.y - projected.y) < 0.5
            }
            XCTAssertTrue(onRing, "a turned point of the ring left the ring's outline")
        }
    }

    /// The glow is an image with room for its blur, lit round the ring
    /// and clear at the centre, and drawn once per size.
    @MainActor
    func test_theGlowIsDrawnOnceRoundTheRing() throws {
        let size = CGSize(width: 150, height: 130)
        let image = try XCTUnwrap(GlassLibraryStationGlow.image(size: size, scale: 2, lit: true))
        let margin = GlassLibraryStationGlow.margin
        XCTAssertEqual(image.width, Int((size.width + margin * 2) * 2))
        XCTAssertEqual(image.height, Int((size.height + margin * 2) * 2))
        XCTAssertTrue(GlassLibraryStationGlow.image(size: size, scale: 2, lit: true) === image, "drawn again for the same size")

        let data = try XCTUnwrap(image.dataProvider?.data as Data?)
        func alpha(atPoint point: CGPoint) -> UInt8 {
            let x = Int((point.x + margin) * 2), y = Int((point.y + margin) * 2)
            return data[y * image.bytesPerRow + x * 4 + 3]
        }
        let outline = GlassLibraryStation.ringOutline(size: size).boundingRect
        XCTAssertEqual(alpha(atPoint: CGPoint(x: size.width / 2, y: size.height / 2)), 0, "the glow covers the mark")
        XCTAssertGreaterThan(alpha(atPoint: CGPoint(x: outline.maxX - 1, y: outline.midY)), 0, "no glow on the ring")
        XCTAssertEqual(alpha(atPoint: CGPoint(x: -margin + 1, y: -margin + 1)), 0, "the glow runs off the image")
        XCTAssertNil(GlassLibraryStationGlow.image(size: .zero, scale: 2, lit: true))
    }

    /// The indexed mesh a frame draws from is the mesh itself: every face
    /// gets back its own corners in its own order, and the corners the
    /// ring's faces share are stored once.
    @MainActor
    func test_theIndexedMeshIsTheMesh() {
        let mesh = GlassLibraryStation.mesh
        let indexed = GlassLibraryStation.indexed
        XCTAssertEqual(indexed.starts.count, mesh.count + 1)
        for (f, face) in mesh.enumerated() {
            let corners = indexed.corners[indexed.starts[f] ..< indexed.starts[f + 1]].map { indexed.vertices[$0] }
            XCTAssertEqual(corners, face.points, "face \(f) changed")
        }
        XCTAssertLessThan(indexed.vertices.count, mesh.reduce(0) { $0 + $1.points.count } / 2)
    }
}
