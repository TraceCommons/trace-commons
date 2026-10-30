import SwiftUI
import XCTest

@testable import TCDesign

/// The coin burst (`GlassCoinBurst`, NEAR AI credit arriving on Home's
/// Balance tile, owner, 2026-10-27): very small gold coins exploding out of
/// the card's centre like a firework.
final class CoinBurstTests: XCTestCase {
    /// The same coins every time, and every one very small.
    @MainActor
    func test_theCoinsAreVerySmallAndRepeatable() {
        XCTAssertEqual(GlassCoinBurst.coins.count, GlassCoinBurst.count)
        XCTAssertGreaterThanOrEqual(GlassCoinBurst.count, 30, "a burst, not a sprinkle")
        XCTAssertEqual(GlassCoinBurst.coins, (0..<GlassCoinBurst.count).map { GlassCoinBurst.coin(at: $0) })
        for coin in GlassCoinBurst.coins {
            XCTAssertGreaterThanOrEqual(coin.diameter, 3)
            XCTAssertLessThanOrEqual(coin.diameter, 6.5, "very small coins")
            XCTAssertGreaterThan(coin.spin, 0, "it tumbles")
            XCTAssertLessThan(coin.delay, 0.15, "they leave together")
        }
    }

    /// Like a firework: every coin leaves the centre, every way round the
    /// circle.
    @MainActor
    func test_theCoinsLeaveTheCentreEveryWay() {
        for coin in GlassCoinBurst.coins {
            let start = GlassCoinBurst.offset(coin, at: 0)
            XCTAssertEqual(start.dx, 0, accuracy: 0.0001)
            XCTAssertEqual(start.dy, 0, accuracy: 0.0001)
        }
        let sectors = Set(GlassCoinBurst.coins.map { coin -> Int in
            let turn = (coin.angle / (2 * .pi)).truncatingRemainder(dividingBy: 1)
            return Int(((turn < 0 ? turn + 1 : turn) * 8).rounded(.down))
        })
        XCTAssertEqual(sectors, Set(0..<8), "a coin in every eighth of the circle")
        for coin in GlassCoinBurst.coins {
            let spread = GlassCoinBurst.offset(coin, at: GlassCoinBurst.stillAt)
            XCTAssertGreaterThan(hypot(spread.dx, spread.dy), 20, "spread out when drawn still")
        }
    }

    /// Fast out of the centre, slowing as it spreads, then drooping.
    @MainActor
    func test_theCoinsSlowAndDroop() throws {
        let sideways = try XCTUnwrap(GlassCoinBurst.coins.min { abs(cos($0.angle)) > abs(cos($1.angle)) })
        let x = { (t: Double) in abs(GlassCoinBurst.offset(sideways, at: t).dx) }
        XCTAssertGreaterThan(x(0.2) - x(0.1), x(0.7) - x(0.6), "it slows as it spreads")
        let y = { (t: Double) in GlassCoinBurst.offset(sideways, at: t).dy }
        XCTAssertGreaterThan(y(GlassCoinBurst.duration) - y(0.5), 20, "it droops")
    }

    /// Whole while they fly, then fading, gone by the end.
    @MainActor
    func test_theCoinsFadeAway() {
        XCTAssertEqual(GlassCoinBurst.opacity(at: 0), 1)
        XCTAssertEqual(GlassCoinBurst.opacity(at: GlassCoinBurst.duration * GlassCoinBurst.fadeFrom), 1)
        XCTAssertLessThan(GlassCoinBurst.opacity(at: GlassCoinBurst.duration * 0.85), 0.5)
        XCTAssertEqual(GlassCoinBurst.opacity(at: GlassCoinBurst.duration), 0)
        XCTAssertLessThan(GlassCoinBurst.duration, 2.5, "a burst, not a show")
    }

    /// Every coin still clearly seen stays inside the field the burst draws
    /// on, the Balance tile and `spill` past it, so none is cut off by its
    /// edge.
    @MainActor
    func test_aSeenCoinIsNeverCutOff() {
        let tile = CGSize(width: 240, height: 80)
        let reach = CGSize(width: tile.width / 2 + GlassCoinBurst.spill, height: tile.height / 2 + GlassCoinBurst.spill)
        var t = 0.0
        while t < GlassCoinBurst.duration {
            if GlassCoinBurst.opacity(at: t) >= 0.2 {
                for coin in GlassCoinBurst.coins {
                    let at = GlassCoinBurst.offset(coin, at: t)
                    XCTAssertLessThan(abs(at.dx) + coin.diameter, reach.width, "cut off at \(t)s")
                    XCTAssertLessThan(abs(at.dy) + coin.diameter, reach.height, "cut off at \(t)s")
                }
            }
            t += 0.05
        }
    }

    /// No coin passes the card's sides, however narrow, where a scroll view
    /// or a panel holding the card would cut it off; a wide card lets the
    /// coins fly their full reach.
    @MainActor
    func test_noCoinPassesTheCardsSides() {
        for width in [100.0, 160, 240, 400] {
            let half = CGFloat(width / 2)
            let sideways = GlassCoinBurst.sideways(halfWidth: half)
            var t = 0.0
            while t < GlassCoinBurst.duration {
                for coin in GlassCoinBurst.coins {
                    let at = GlassCoinBurst.offset(coin, at: t, sideways: sideways)
                    XCTAssertLessThanOrEqual(abs(at.dx) + coin.diameter / 2, half, "\(width)pt card: past its side at \(t)s")
                }
                t += 0.05
            }
        }
        XCTAssertEqual(GlassCoinBurst.sideways(halfWidth: 400), 1, "a wide card holds the whole burst")
        XCTAssertGreaterThan(GlassCoinBurst.sideways(halfWidth: 120), 0.9, "Home's Balance tile: nearly all of it")
    }

    /// The burst adds a view only while there is one; almost always there
    /// is none.
    func test_noBurstAddsNoView() throws {
        let source = try XCTUnwrap(DesignSources.all().first { $0.0 == "CoinBurst.swift" }?.1)
        let modifier = try XCTUnwrap(source.range(of: "func glassCoinBurst("))
        let check = try XCTUnwrap(source.range(of: "if start != nil {", range: modifier.upperBound..<source.endIndex))
        let burst = try XCTUnwrap(source.range(of: "GlassCoinBurst(start: start, still: still)",
                                               range: modifier.upperBound..<source.endIndex))
        XCTAssertLessThan(check.lowerBound, burst.lowerBound)
    }

    /// Nothing moves under Reduce Motion: the timeline is paused while the
    /// coins are still, and the drawing reads no clock.
    func test_reduceMotionDrawsTheCoinsStill() throws {
        let source = try XCTUnwrap(DesignSources.all().first { $0.0 == "CoinBurst.swift" }?.1)
        XCTAssertTrue(source.contains("GlassAnimationTimeline(paused: start == nil || still)"))
        XCTAssertTrue(source.contains("let t = still ? Self.stillAt : date.timeIntervalSince(start)"))
        XCTAssertTrue(source.contains(".allowsHitTesting(false)"))
        XCTAssertTrue(source.contains(".accessibilityHidden(true)"))
    }
}
