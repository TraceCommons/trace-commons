import SwiftUI
import XCTest

@testable import TCDesign

/// The dithered pixel field (owner, 2026-11-02): lit densest in the bottom
/// trailing corner and gone before the far side, a real ordered dither, a
/// colour for every cell from the palette, and slow. On the main actor:
/// the field is a view, so its rules are main-actor isolated.
@MainActor
final class PixelFieldTests: XCTestCase {
    /// The Bayer matrix holds each of its 16 thresholds once, all inside (0, 1).
    func test_theDitherIsABayerMatrix() {
        var thresholds = Set<Double>()
        for row in 0..<4 {
            for column in 0..<4 { thresholds.insert(GlassPixelField.threshold(column: column, row: row)) }
        }
        XCTAssertEqual(thresholds.count, 16)
        XCTAssertTrue(thresholds.allSatisfy { $0 > 0 && $0 < 1 })
        XCTAssertEqual(GlassPixelField.threshold(column: 4, row: 4), GlassPixelField.threshold(column: 0, row: 0),
                       "the matrix tiles")
    }

    /// Bright in the corner, nothing at or past the field's edge, and dimmer
    /// further out whatever the ripple is doing.
    func test_theLightFallsOffFromTheCorner() {
        let size = CGSize(width: 600, height: 240)
        let reach = CGSize(width: 0.62, height: 1.05)
        XCTAssertEqual(GlassPixelField.distance(x: 600, y: 240, size: size, reach: reach), 0, accuracy: 1e-9)
        XCTAssertGreaterThan(GlassPixelField.distance(x: 0, y: 0, size: size, reach: reach), 1, "the top leading corner is past it")
        for time in [0.0, 1.3, 7.9] {
            XCTAssertGreaterThan(GlassPixelField.level(distance: 0, time: time), 0.7)
            XCTAssertEqual(GlassPixelField.level(distance: 1, time: time), 0)
            XCTAssertEqual(GlassPixelField.level(distance: 1.4, time: time), 0)
            XCTAssertGreaterThan(GlassPixelField.level(distance: 0.2, time: time),
                                 GlassPixelField.level(distance: 0.8, time: time))
        }
    }

    /// Every cell takes one of the palette's colours, and they move.
    func test_everyCellTakesAPaletteColour() {
        var seen = Set<Int>()
        for column in 0..<40 {
            for row in 0..<20 {
                let shade = GlassPixelField.shade(column: column, row: row, time: 3, count: 3)
                XCTAssertTrue((0..<3).contains(shade))
                seen.insert(shade)
            }
        }
        XCTAssertEqual(seen, [0, 1, 2], "all three blues are used")
        XCTAssertEqual(GlassPixelField.shade(column: 5, row: 5, time: 0, count: 1), 0)
        let moved = (0..<40).contains { column in
            GlassPixelField.shade(column: column, row: 3, time: 0, count: 3)
                != GlassPixelField.shade(column: column, row: 3, time: 4, count: 3)
        }
        XCTAssertTrue(moved, "the colours drift over time")
    }

    /// It runs slowly, on the shared timeline that stops while the window
    /// cannot be seen, and stands still under Reduce Motion; it never takes
    /// the pointer or VoiceOver's attention.
    func test_itIsSlowStillUnderReduceMotionAndInert() throws {
        XCTAssertLessThanOrEqual(GlassPixelField.framesPerSecond, 12)
        let source = try String(
            contentsOf: URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
                .deletingLastPathComponent().appendingPathComponent("Sources/TCDesign/Components/PixelField.swift"),
            encoding: .utf8)
        XCTAssertTrue(source.contains("GlassAnimationTimeline(minimumInterval: 1 / Self.framesPerSecond, paused: reduceMotion)"))
        XCTAssertTrue(source.contains("time: reduceMotion ? 0 :"))
        XCTAssertTrue(source.contains(".allowsHitTesting(false)"))
        XCTAssertTrue(source.contains(".accessibilityHidden(true)"))
    }
}

/// An icon card that is its title alone centres the title on its icon
/// (owner, 2026-11-02); one with a subtitle or anything under its head
/// keeps the icon's top on the title's cap height (owner, 2026-10-10).
final class IconCardTitleAlignmentTests: XCTestCase {
    @MainActor
    func test_aTitleAloneIsCentredOnItsIcon() {
        XCTAssertTrue(GlassIconCard<EmptyView, EmptyView>.centersTitle(subtitle: [], hasContent: false))
        XCTAssertFalse(GlassIconCard<EmptyView, EmptyView>.centersTitle(subtitle: ["A line"], hasContent: false))
        XCTAssertFalse(GlassIconCard<EmptyView, EmptyView>.centersTitle(subtitle: [], hasContent: true))
    }
}
