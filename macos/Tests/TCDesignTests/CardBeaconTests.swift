import SwiftUI
import XCTest

@testable import TCDesign

/// The card beacon (`GlassCardBeacon`, Home's Not contributed card, owner,
/// 2026-10-22): a soft light that fades in on the card's edge, turns once
/// round it like a lighthouse's beam and fades away, when the card's tab is
/// shown or its window comes back into focus, then every 15s.
final class CardBeaconTests: XCTestCase {
    /// A pass every 15s, the first soon after focus; the light fades in
    /// and out inside one turn, and the turn ends well before the next.
    func test_aPassComesEvery15SecondsAndRestsBetween() {
        XCTAssertEqual(GlassCardBeacon.every, .seconds(15))
        XCTAssertLessThanOrEqual(GlassCardBeacon.first, .seconds(1), "the first pass comes soon after focus")
        XCTAssertLessThan(GlassCardBeacon.fade * 2, GlassCardBeacon.duration)
        XCTAssertLessThan(.seconds(GlassCardBeacon.duration), GlassCardBeacon.every / 2)
    }

    /// The light is soft (owner, 2026-10-22: dimmed a bit): never at full
    /// strength, and its glow softer than the edge it lights.
    func test_theLightIsSoft() throws {
        XCTAssertLessThan(GlassCardBeacon.peak, 1)
        XCTAssertGreaterThan(GlassCardBeacon.peak, 0.3, "still seen")
        XCTAssertLessThan(GlassCardBeacon.glowShare, 1)
        let source = try XCTUnwrap(DesignSources.all().first { $0.0 == "CardBeacon.swift" }?.1)
        XCTAssertTrue(source.contains(".opacity(light * Self.peak)"))
        XCTAssertTrue(source.contains(".opacity(Self.glowShare)"))
    }

    /// A turn still under way when the window comes back into focus ends
    /// before the next starts; otherwise the first pass waits `first`.
    func test_aTurnUnderWayIsNotCutShort() {
        let now = ContinuousClock.now
        XCTAssertEqual(GlassCardBeacon.firstWait(now: now, passEnds: nil), GlassCardBeacon.first)
        XCTAssertEqual(GlassCardBeacon.firstWait(now: now, passEnds: now - .seconds(3)), GlassCardBeacon.first)
        XCTAssertEqual(GlassCardBeacon.firstWait(now: now, passEnds: now + .seconds(2.5)), .seconds(2.5))
    }

    /// Two narrow lit arcs half a turn apart, top and bottom at the start
    /// (owner, 2026-10-22: two light sources), clear everywhere else.
    func test_twoBeamsHalfATurnApart() throws {
        let stops = GlassCardBeacon.beam(.white).stops
        XCTAssertEqual(stops.first?.color, .clear)
        XCTAssertEqual(stops.last?.color, .clear)
        let lit = stops.filter { $0.color != .clear }.map(\.location)
        XCTAssertEqual(lit.map { Double($0) }, GlassCardBeacon.crests)
        XCTAssertEqual(lit.count, 2)
        XCTAssertEqual(lit[1] - lit[0], 0.5, accuracy: 0.0001, "the two lights face each other")
        // From the right, clockwise: a quarter turn is the bottom, three
        // quarters the top.
        XCTAssertEqual(lit, [0.25, 0.75])
        let source = try XCTUnwrap(DesignSources.all().first { $0.0 == "CardBeacon.swift" }?.1)
        XCTAssertTrue(source.contains("center: .center,\n                               angle: .degrees(0))"))
        XCTAssertLessThanOrEqual(GlassCardBeacon.beamShare, 0.25, "each beam is a spot, not a ring")
        XCTAssertEqual(stops.map(\.location), stops.map(\.location).sorted(), "the stops run in order")
    }

    /// The window back in focus starts the passes over, and the light goes
    /// out before a pass started over mid-turn begins again. The turn is
    /// linear (a lighthouse turns at one speed); the fades take the one
    /// motion curve.
    func test_focusStartsThePassesOver() throws {
        let source = try XCTUnwrap(DesignSources.all().first { $0.0 == "CardBeacon.swift" }?.1)
        XCTAssertTrue(source.contains(".onChange(of: activeState == .inactive)"))
        XCTAssertTrue(source.contains("if !inactive { focus += 1 }"))
        XCTAssertTrue(source.contains(".task(id: Run(reduceMotion: reduceMotion, focus: focus))"))
        XCTAssertTrue(source.contains("if light > 0 {\n                withAnimation(GlassMotion.curve(Self.fade)) { light = 0 }"))
        XCTAssertTrue(source.contains("withAnimation(.linear(duration: Self.duration)) { turn = 1 }"))
        XCTAssertTrue(source.contains("withAnimation(GlassMotion.curve(Self.fade)) { light = 1 }"))
        XCTAssertTrue(source.contains("guard !reduceMotion else {"))
    }
}
