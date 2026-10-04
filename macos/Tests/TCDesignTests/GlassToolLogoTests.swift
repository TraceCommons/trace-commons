import CoreGraphics
import XCTest

@testable import TCDesign

/// The tool logos parse, land inside their viewBox, and reach the tiles.
/// The parser cases cover the compact forms the minified artwork uses; a
/// misread there draws a wrong mark rather than failing loudly.
final class GlassToolLogoTests: XCTestCase {
    // MARK: Artwork

    func test_everyLogoParses() {
        for id in GlassToolLogoID.allCases {
            for (index, data) in id.artwork.paths.enumerated() {
                XCTAssertNotNil(SVGPathData.parse(data), "\(id) path \(index) does not parse")
            }
            XCTAssertNotNil(id.path, "\(id) has no combined path")
        }
    }

    func test_everyLogoFillsItsViewBox() throws {
        for id in GlassToolLogoID.allCases {
            let bounds = try XCTUnwrap(id.path).boundingRect
            let box = id.artwork.viewBox
            // Inside the box, within rounding...
            XCTAssertTrue(box.insetBy(dx: -0.01, dy: -0.01).contains(bounds), "\(id) spills out of its viewBox: \(bounds)")
            // ...and covering most of it, so a misparsed path that collapses
            // to a sliver fails here too.
            XCTAssertGreaterThan(max(bounds.width / box.width, bounds.height / box.height), 0.8, "\(id) is too small: \(bounds)")
        }
    }

    func test_theShapeScalesTheViewBoxIntoItsFrame() {
        let rect = CGRect(x: 10, y: 20, width: 15, height: 15)
        for id in GlassToolLogoID.allCases {
            let bounds = GlassToolLogoShape(id).path(in: rect).boundingRect
            XCTAssertTrue(rect.insetBy(dx: -0.01, dy: -0.01).contains(bounds), "\(id) drawn outside its frame: \(bounds)")
        }
    }

    func test_toolsWithArtworkUseIt() {
        XCTAssertEqual(GlassTool.claudeCode.logo, .claude)
        XCTAssertEqual(GlassTool.codex.logo, .codex)
        XCTAssertEqual(GlassTool.antigravity.logo, .antigravity)
        XCTAssertEqual(GlassTool.openCode.logo, .openCode)
        XCTAssertEqual(GlassTool.theia.logo, .theia)
        XCTAssertNil(GlassTool.geminiCLI.logo)
        XCTAssertNil(GlassTool.cline.logo)
        XCTAssertNil(GlassTool.other(initials: "Zz").logo)
    }

    // MARK: Parser

    func test_runTogetherNumbersSplitOnSignAndSecondPoint() throws {
        // `.08-.23` is two numbers; `1.5.5` is 1.5 then .5.
        let path = try XCTUnwrap(SVGPathData.parse("M0 0l.08-.23L1.5.5"))
        XCTAssertEqual(path.currentPoint.x, 1.5, accuracy: 1e-9)
        XCTAssertEqual(path.currentPoint.y, 0.5, accuracy: 1e-9)
        let points = Self.points(path)
        XCTAssertEqual(points[1].x, 0.08, accuracy: 1e-9)
        XCTAssertEqual(points[1].y, -0.23, accuracy: 1e-9)
    }

    func test_arcFlagsRunStraightIntoTheNextNumber() throws {
        // `0 00-.975 0`: rotation 0, flags 0 and 0, then the endpoint.
        let path = try XCTUnwrap(SVGPathData.parse("M1 1a.503.503 0 00-.975 0"))
        XCTAssertEqual(path.currentPoint.x, 0.025, accuracy: 1e-9)
        XCTAssertEqual(path.currentPoint.y, 1, accuracy: 1e-9)
    }

    func test_aHalfCircleArcBulgesOnTheSweepSide() throws {
        // From (0,0) to (2,0) with radius 1: sweep 1 goes through y = -1
        // in SVG's y-down space, sweep 0 through y = +1.
        let clockwise = try XCTUnwrap(SVGPathData.parse("M0 0A1 1 0 0 1 2 0")).boundingBoxOfPath
        XCTAssertEqual(clockwise.minY, -1, accuracy: 1e-3)
        XCTAssertEqual(clockwise.maxY, 0, accuracy: 1e-3)
        let counter = try XCTUnwrap(SVGPathData.parse("M0 0A1 1 0 0 0 2 0")).boundingBoxOfPath
        XCTAssertEqual(counter.maxY, 1, accuracy: 1e-3)
    }

    func test_pairsAfterAMovetoAreLinetos() throws {
        let path = try XCTUnwrap(SVGPathData.parse("m1 1 2 0 0 2z"))
        XCTAssertEqual(Self.points(path).count, 3)
        XCTAssertEqual(path.boundingBoxOfPath, CGRect(x: 1, y: 1, width: 2, height: 2))
    }

    func test_relativeMovetoAfterCloseStartsFromTheSubpathStart() throws {
        // The OpenCode mark: after `z` the pen is back at (16,6), so `m4 16`
        // lands on (20,22).
        let path = try XCTUnwrap(SVGPathData.parse("M16 6H8v12h8V6zm4 16H4V2h16v20z"))
        XCTAssertEqual(path.boundingBoxOfPath, CGRect(x: 4, y: 2, width: 16, height: 20))
    }

    func test_smoothCurveReflectsThePreviousControlPoint() throws {
        let path = try XCTUnwrap(SVGPathData.parse("M0 0C0 1 1 1 1 0s1-1 1 0"))
        var controls: [CGPoint] = []
        path.applyWithBlock { element in
            if element.pointee.type == .addCurveToPoint {
                controls.append(element.pointee.points[0])
            }
        }
        XCTAssertEqual(controls.count, 2)
        // Reflection of (1,1) about (1,0).
        XCTAssertEqual(controls[1].x, 1, accuracy: 1e-9)
        XCTAssertEqual(controls[1].y, -1, accuracy: 1e-9)
    }

    func test_exponentsAreNumbersNotCommands() throws {
        let path = try XCTUnwrap(SVGPathData.parse("M1e1 2E-1L0 0"))
        XCTAssertEqual(Self.points(path)[0], CGPoint(x: 10, y: 0.2))
    }

    func test_malformedDataIsRejectedNotHalfDrawn() {
        XCTAssertNil(SVGPathData.parse(""))
        XCTAssertNil(SVGPathData.parse("10 10"))
        XCTAssertNil(SVGPathData.parse("M0 0L1"))
        XCTAssertNil(SVGPathData.parse("M0 0a1 1 0 2 0 1 1"))
        XCTAssertNil(SVGPathData.parse("M0 0X1 1"))
    }

    /// The end point of every element, in order.
    private static func points(_ path: CGPath) -> [CGPoint] {
        var result: [CGPoint] = []
        path.applyWithBlock { element in
            let e = element.pointee
            switch e.type {
            case .moveToPoint, .addLineToPoint: result.append(e.points[0])
            case .addQuadCurveToPoint: result.append(e.points[1])
            case .addCurveToPoint: result.append(e.points[2])
            case .closeSubpath: break
            @unknown default: break
            }
        }
        return result
    }
}
