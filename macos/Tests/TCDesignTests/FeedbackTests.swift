import AppKit
import SwiftUI
import XCTest

@testable import TCDesign

/// The spinner, the skeleton and the warning glyph.
@MainActor
final class FeedbackTests: XCTestCase {
    func test_theSpinnerTurnsOncePerSpinAndStandsStillUnderReduceMotion() {
        let period = GlassTokens.Motion.spin
        XCTAssertEqual(GlassSpinner.angle(at: 0, reduceMotion: false), 0, accuracy: 0.001)
        XCTAssertEqual(GlassSpinner.angle(at: period / 4, reduceMotion: false), 90, accuracy: 0.001)
        XCTAssertEqual(GlassSpinner.angle(at: period * 10 + period / 2, reduceMotion: false), 180, accuracy: 0.01)
        XCTAssertEqual(GlassSpinner.angle(at: period / 4, reduceMotion: true), 0)
    }

    /// Hidden only when unlabelled beside its words; a standalone spinner
    /// stays a progress indicator to VoiceOver, as the stock one was.
    func test_onlyAnUnlabelledSpinnerBesideWordsIsHidden() {
        XCTAssertTrue(GlassSpinner.isHidden(label: "", standalone: false))
        XCTAssertFalse(GlassSpinner.isHidden(label: "", standalone: true))
        XCTAssertFalse(GlassSpinner.isHidden(label: "Loading", standalone: false))
    }

    func test_theSkeletonPulsesOnTheTokenCurveAndNotUnderReduceMotion() {
        XCTAssertNil(GlassSkeleton.pulse(true))
        XCTAssertNotNil(GlassSkeleton.pulse(false))
        XCTAssertEqual(GlassSkeleton.opacity(dim: true), GlassTokens.Opacity.skeletonDim)
        XCTAssertEqual(GlassSkeleton.opacity(dim: false), 1)
    }

    func test_sizes() {
        let spinner = NSHostingView(rootView: GlassSpinner()).fittingSize
        XCTAssertEqual(spinner.width, GlassTokens.Size.spinner, accuracy: 0.5)
        let skeleton = NSHostingView(rootView: GlassSkeleton(width: 80)).fittingSize
        XCTAssertEqual(skeleton.height, GlassTokens.Size.skeletonHeight, accuracy: 0.5)
        XCTAssertEqual(skeleton.width, 80, accuracy: 0.5)
        let glyph = NSHostingView(rootView: GlassWarningGlyph(size: 16)).fittingSize
        XCTAssertEqual(glyph.width, 16, accuracy: 0.5)
    }

    /// The glyph is #1146's 16-unit artwork, scaled to its frame.
    func test_theWarningGlyphIsTheSixteenUnitArtwork() {
        let box = CGRect(x: 0, y: 0, width: 16, height: 16)
        let triangle = GlassWarningShape.triangle.path(in: box).boundingRect
        XCTAssertEqual(triangle.minX, 1, accuracy: 0.001)
        XCTAssertEqual(triangle.maxX, 15, accuracy: 0.001)
        XCTAssertEqual(triangle.minY, 1.75, accuracy: 0.001)
        XCTAssertEqual(triangle.maxY, 14.25, accuracy: 0.001)
        let bar = GlassWarningShape.bar.path(in: box).boundingRect
        XCTAssertEqual(bar.minY, 6.25, accuracy: 0.001)
        XCTAssertEqual(bar.maxY, 9.75, accuracy: 0.001)
        let dot = GlassWarningShape.dot.path(in: CGRect(x: 0, y: 0, width: 32, height: 32)).boundingRect
        XCTAssertEqual(dot.midY, 23.8, accuracy: 0.001)
        XCTAssertEqual(dot.width, 3.4, accuracy: 0.001)
    }
}
