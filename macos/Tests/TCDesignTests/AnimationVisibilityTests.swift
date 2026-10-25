import AppKit
import simd
import SwiftUI
import XCTest

@testable import TCDesign

/// The app's continuous animations hold still while they cannot be seen
/// (`GlassAnimationTimeline`), and what they redraw on every frame is
/// worked out once where it does not depend on the time.
final class AnimationVisibilityTests: XCTestCase {
    // MARK: The gate

    /// An animation runs only when its caller would run it and it can be
    /// seen.
    func test_pausedWhenCallerPausesOrCannotBeSeen() {
        XCTAssertFalse(GlassAnimationGate.paused(false, visible: true))
        XCTAssertTrue(GlassAnimationGate.paused(false, visible: false))
        XCTAssertTrue(GlassAnimationGate.paused(true, visible: true))
        XCTAssertTrue(GlassAnimationGate.paused(true, visible: false))
    }

    /// A view can be seen only in a window AppKit reports as visible, not
    /// minimised, in an app that is not hidden.
    func test_visibleOnlyInAnUncoveredWindowOfAShownApp() {
        XCTAssertTrue(GlassAnimationGate.isVisible(inWindow: true, occlusionVisible: true, miniaturized: false, appHidden: false))
        XCTAssertFalse(GlassAnimationGate.isVisible(inWindow: false, occlusionVisible: true, miniaturized: false, appHidden: false))
        XCTAssertFalse(GlassAnimationGate.isVisible(inWindow: true, occlusionVisible: false, miniaturized: false, appHidden: false))
        XCTAssertFalse(GlassAnimationGate.isVisible(inWindow: true, occlusionVisible: true, miniaturized: true, appHidden: false))
        XCTAssertFalse(GlassAnimationGate.isVisible(inWindow: true, occlusionVisible: true, miniaturized: false, appHidden: true))
    }

    /// No window, or a window never ordered on screen, cannot be seen.
    @MainActor
    func test_noWindowOrAWindowOffScreenIsNotVisible() {
        XCTAssertFalse(GlassAnimationGate.isVisible(nil))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 40, height: 40),
                              styleMask: [.borderless], backing: .buffered, defer: true)
        window.isReleasedWhenClosed = false
        XCTAssertFalse(GlassAnimationGate.isVisible(window))
    }

    /// With no root keeping it, the environment says visible, so a
    /// preview, a test or an `ImageRenderer` image animates as before.
    func test_visibleByDefault() {
        XCTAssertTrue(EnvironmentValues().glassAnimationsVisible)
    }

    /// The probe reports its window's state once it is in one, and again
    /// (as not visible) once it leaves it; an unchanged state is not
    /// reported twice.
    @MainActor
    func test_probeReportsItsWindowAndLeavingIt() {
        var reports: [Bool] = []
        let probe = GlassWindowVisibilityProbe.ProbeView()
        probe.onChange = { reports.append($0) }
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 40, height: 40),
                              styleMask: [.borderless], backing: .buffered, defer: true)
        window.isReleasedWhenClosed = false
        window.contentView?.addSubview(probe)
        XCTAssertEqual(probe.reported, false, "a window never ordered in cannot be seen")
        probe.refresh()
        probe.removeFromSuperview()
        XCTAssertEqual(probe.reported, false)

        let delivered = expectation(description: "reports delivered")
        DispatchQueue.main.async { delivered.fulfill() }
        wait(for: [delivered], timeout: 2)
        XCTAssertEqual(reports, [false], "one report: the state never changed")
    }

    // MARK: Colours

    /// A token colour is one `NSColor` per value, made once, not a new
    /// closure-backed colour on every read.
    func test_dynamicColourIsMadeOncePerValue() {
        let token = GlassTokens.Color.textPrimary
        XCTAssertNotNil(token.lightRGB)
        XCTAssertTrue(token.dynamicNSColor === token.dynamicNSColor)
        XCTAssertTrue(token.opacity(0.4).dynamicNSColor === token.opacity(0.4).dynamicNSColor)
        XCTAssertFalse(token.opacity(0.4).dynamicNSColor === token.opacity(0.5).dynamicNSColor)
    }

    /// The cache is bounded: past its limit it lets the colours go and
    /// makes them again, still one per value.
    func test_colourCacheIsBounded() {
        let cache = GlassColorCache()
        var made = 0
        func make(_ value: GlassRGBA) -> NSColor {
            made += 1
            return value.makeDynamicNSColor()
        }
        let first = GlassRGBA(0x101010, alpha: 1, light: GlassRGBA(0xEFEFEF))
        let colour = cache.dynamic(first, make: make)
        XCTAssertTrue(cache.dynamic(first, make: make) === colour)
        XCTAssertEqual(made, 1)
        for index in 0 ..< GlassColorCache.limit + 10 {
            _ = cache.dynamic(GlassRGBA(0x202020, alpha: Double(index) / 10_000, light: GlassRGBA(0xDFDFDF)), make: make)
        }
        XCTAssertLessThanOrEqual(cache.colourCount, GlassColorCache.limit)
    }

    /// Light or dark is worked out once per appearance name, and the kept
    /// answer is the one `bestMatch` gives.
    @MainActor
    func test_appearanceAnswersAreKeptPerName() throws {
        let cache = GlassColorCache()
        var asked = 0
        let names: [NSAppearance.Name] = [.aqua, .darkAqua, .vibrantLight, .vibrantDark]
        for name in names {
            let appearance = try XCTUnwrap(NSAppearance(named: name))
            let expected = GlassRGBA.resolveIsLight(appearance)
            for _ in 0 ..< 3 {
                XCTAssertEqual(cache.isLight(name) { _ in asked += 1; return expected }, expected)
            }
            XCTAssertEqual(GlassRGBA.isLight(appearance), expected)
        }
        XCTAssertEqual(asked, names.count)
        XCTAssertTrue(GlassRGBA.isLight(try XCTUnwrap(NSAppearance(named: .aqua))))
        XCTAssertFalse(GlassRGBA.isLight(try XCTUnwrap(NSAppearance(named: .darkAqua))))
    }

    // MARK: The station

    /// The station's per-frame turn with its constant cosines and sines
    /// worked out once gives exactly the figures the per-corner version
    /// gave, so the picture is unchanged.
    @MainActor
    func test_stationTurnIsUnchangedByHoistedTrig() {
        func original(_ p: SIMD3<Double>, angle: Double) -> SIMD3<Double> {
            let cy = cos(angle), sy = sin(angle)
            let x = p.x * cy + p.z * sy, z = -p.x * sy + p.z * cy, y = p.y
            let cx = cos(GlassLibraryStation.tilt), sx = sin(GlassLibraryStation.tilt)
            let y2 = y * cx - z * sx, z2 = y * sx + z * cx
            let cz = cos(GlassLibraryStation.lean), sz = sin(GlassLibraryStation.lean)
            return SIMD3(x * cz - y2 * sz, x * sz + y2 * cz, z2)
        }
        for angle in stride(from: 0.0, through: 2 * .pi, by: 0.37) {
            for face in GlassLibraryStation.mesh.prefix(60) {
                for p in face.points {
                    XCTAssertEqual(GlassLibraryStation.turned(p, angle: angle), original(p, angle: angle))
                }
            }
        }
    }

    /// The station's angle runs from its rest angle through one turn per
    /// `turnSeconds`, and holds at rest under Reduce Motion.
    @MainActor
    func test_stationAngle() {
        let rest = GlassLibraryStation.restAngle
        XCTAssertEqual(GlassLibraryStation.angle(at: 0, reduceMotion: false), rest, accuracy: 1e-12)
        XCTAssertEqual(GlassLibraryStation.angle(at: GlassLibraryStation.turnSeconds / 2, reduceMotion: false), rest + .pi, accuracy: 1e-9)
        XCTAssertEqual(GlassLibraryStation.angle(at: 1234.5, reduceMotion: true), rest)
    }
}
