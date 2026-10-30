import SwiftUI
import XCTest

@testable import TCDesign

/// The card shimmer (`GlassCardShimmer`, Home's Not contributed card, owner,
/// 2026-10-27): a soft band of light across the card, fast, each time its
/// tab is shown or its window comes back into focus, then every 18s; seen
/// as light on both themes, not a dull wash, and soft, not a reflection.
final class CardShimmerTests: XCTestCase {
    /// A pass every 18s, the first soon after the tab is shown, and each
    /// pass fast.
    @MainActor
    func test_aFastPassComesEvery18Seconds() {
        XCTAssertEqual(GlassCardShimmer.every, .seconds(18))
        XCTAssertLessThanOrEqual(GlassCardShimmer.first, .milliseconds(500), "the first pass comes as the tab opens")
        XCTAssertLessThan(GlassCardShimmer.duration, 1, "a pass is fast")
        XCTAssertGreaterThan(GlassCardShimmer.duration, 0.3, "still seen crossing")
    }

    /// A pass still under way when the window comes back into focus ends
    /// before the next starts; otherwise the first pass waits `first`.
    @MainActor
    func test_aPassUnderWayIsNotCutShort() {
        let now = ContinuousClock.now
        XCTAssertEqual(GlassCardShimmer.firstWait(now: now, passEnds: nil), GlassCardShimmer.first)
        XCTAssertEqual(GlassCardShimmer.firstWait(now: now, passEnds: now - .seconds(3)), GlassCardShimmer.first)
        XCTAssertEqual(GlassCardShimmer.firstWait(now: now, passEnds: now + .seconds(0.6)), .seconds(0.6))
    }

    /// At the start of a pass the band is wholly before the card, at the end
    /// wholly past it, whatever the card's shape, so nothing jumps on or
    /// off; halfway it is over the card's middle.
    @MainActor
    func test_theBandStartsAndEndsOffTheCard() {
        for size in [CGSize(width: 560, height: 70), CGSize(width: 240, height: 120), CGSize(width: 80, height: 300)] {
            let angle = GlassCardShine.slant * .pi / 180
            let unit = CGVector(dx: cos(angle), dy: sin(angle))
            func along(_ point: CGPoint) -> CGFloat { point.x * unit.dx + point.y * unit.dy }
            let corners = [CGPoint.zero, CGPoint(x: size.width, y: 0), CGPoint(x: 0, y: size.height),
                           CGPoint(x: size.width, y: size.height)].map(along)
            let before = GlassCardShine.band(progress: 0, size: size)
            XCTAssertLessThanOrEqual(along(before.end), corners.min()! + 0.001, "\(size): lit at the start")
            let past = GlassCardShine.band(progress: 1, size: size)
            XCTAssertGreaterThanOrEqual(along(past.start), corners.max()! - 0.001, "\(size): lit at the end")
            let middle = GlassCardShine.band(progress: 0.5, size: size)
            let centre = (along(middle.start) + along(middle.end)) / 2
            XCTAssertEqual(centre, along(CGPoint(x: size.width / 2, y: size.height / 2)), accuracy: 0.001)
        }
    }

    /// Two layers, each clear at both ends and falling off smoothly: a wide
    /// soft glow, and a brighter centre, on both themes. Soft (owner,
    /// 2026-10-27: the hard streak read as too reflective): no layer near
    /// full white on the dark theme, the centre a broad bell rather than a
    /// line, and one crest, with no second glint. Still bright enough to
    /// read as light; the light theme's centre is the stronger, since white
    /// on a pale card barely shows.
    @MainActor
    func test_theLightIsSoft() {
        for dark in [true, false] {
            for layer in [GlassCardShine.glow(dark: dark), GlassCardShine.core(dark: dark)] {
                let stops = layer.stops.map { (Double($0.location), $0.color.resolve(in: EnvironmentValues()).opacity) }
                XCTAssertEqual(stops.first?.1, 0, "lit at the band's edge")
                XCTAssertEqual(stops.last?.1, 0, "lit at the band's edge")
                XCTAssertEqual(stops.map(\.0), stops.map(\.0).sorted(), "the stops run in order")
            }
            let core = GlassCardShine.coreProfile(dark: dark)
            let glow = GlassCardShine.glowProfile(dark: dark)
            let crest = core.light(at: 0.5) + glow.light(at: 0.5)
            XCTAssertGreaterThanOrEqual(crest, 0.35, "still seen as light")
            XCTAssertGreaterThanOrEqual(core.spread, 0.06, "a broad centre, not a hard line")
            XCTAssertGreaterThan(glow.spread, core.spread)
            // One crest: from the middle out, each side only falls away, so
            // there is no bar and no second glint.
            for direction in [-1.0, 1.0] {
                var last = crest
                for step in 1...25 {
                    let x = 0.5 + direction * Double(step) * 0.02
                    let here = core.light(at: x) + glow.light(at: x)
                    XCTAssertLessThanOrEqual(here, last, "a second crest at \(x)")
                    last = here
                }
            }
        }
        XCTAssertLessThanOrEqual(GlassCardShine.coreProfile(dark: true).peak + GlassCardShine.glowProfile(dark: true).peak,
                                 0.5, "the dark theme's light adds; at full white it reads as a reflection")
        XCTAssertGreaterThan(GlassCardShine.coreProfile(dark: false).peak, GlassCardShine.coreProfile(dark: true).peak)
    }

    /// On the dark theme the light adds to what is under it; it lights the
    /// card's face and leaves only a faint blurred glow on its edge, with no
    /// crisp rim line; the tab shown or the window back in focus starts the
    /// passes over; none under Reduce Motion.
    @MainActor
    func test_theLightAddsWithNoCrispRim() throws {
        let source = try XCTUnwrap(DesignSources.all().first { $0.0 == "CardShimmer.swift" }?.1)
        XCTAssertTrue(source.contains(".blendMode(dark ? .plusLighter : .normal)"))
        XCTAssertTrue(source.contains("for layer in [Self.glow(dark: dark), Self.core(dark: dark)] {"))
        XCTAssertTrue(source.contains("context.fill(shape.path(in: card), with: light)"))
        XCTAssertFalse(source.contains("context.stroke("), "a crisp rim line is back")
        XCTAssertTrue(source.contains("glow.addFilter(.blur(radius: Self.rimGlowBlur))"))
        XCTAssertLessThanOrEqual(GlassCardShine.rimGlowShare, 0.5)
        XCTAssertTrue(source.contains(".onChange(of: activeState == .inactive)"))
        XCTAssertTrue(source.contains("if !inactive { focus += 1 }"))
        XCTAssertTrue(source.contains(".task(id: Run(reduceMotion: reduceMotion, focus: focus))"))
        XCTAssertTrue(source.contains("guard !reduceMotion else {"))
        XCTAssertTrue(source.contains("withAnimation(GlassMotion.curve(Self.duration)) { progress = 1 }"))
        XCTAssertTrue(source.contains("guard progress > 0, progress < 1 else { return }"), "light between passes")
    }
}
