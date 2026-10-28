import SwiftUI
import TCDesign
import XCTest

@testable import TraceCommonsApp

/// The owner's 2026-10-27 asks for Home's Not contributed card and NEAR AI
/// Balance tile.
@MainActor
final class DesignRulings20261027Tests: XCTestCase {
    private static func text(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    private static func notContributedCard() throws -> Substring {
        let home = try text("Views/Monitor/HomeViews.swift")
        let card = try XCTUnwrap(home.range(of: "private struct HomeNotContributedCard: View {"))
        let end = try XCTUnwrap(home.range(of: "\n}\n", range: card.upperBound..<home.endIndex))
        return home[card.upperBound..<end.lowerBound]
    }

    /// The call to share sits in a translucent card of its own, to the right
    /// of the label and the count: a card on the card, so it takes the
    /// nested card's fill, which the watermark shows through.
    func test_theCallToShareSitsInItsOwnCardOnTheRight() throws {
        let body = try Self.notContributedCard()
        let count = try XCTUnwrap(body.range(of: "Text(String(count))"))
        let column = try XCTUnwrap(body.range(of: ".fixedSize()\n", range: count.upperBound..<body.endIndex))
        let detail = try XCTUnwrap(body.range(of: "Text(detail)"))
        let inner = try XCTUnwrap(body.range(of: ".glassTier(.cardQuiet)"))
        let outer = try XCTUnwrap(body.range(of: ".glassTier(.card,"))
        XCTAssertLessThan(column.lowerBound, detail.lowerBound, "the call to share is right of the count")
        XCTAssertLessThan(detail.lowerBound, inner.lowerBound)
        XCTAssertLessThan(inner.lowerBound, outer.lowerBound, "the inner card sits on the card")
        XCTAssertTrue(body.contains("HStack(alignment: .center, spacing: GlassTokens.Space.s6) {"))
        // The nested card's fill on a card is translucent both ways.
        XCTAssertLessThan(GlassTokens.Color.cardNested.alpha, 0.2)
        XCTAssertLessThan(GlassTokens.Color.cardNested.light.alpha, 0.2)
    }

    /// A soft band of light crosses the card: brighter and faster than the
    /// band before, but soft, not a reflection, every 18s and each time
    /// Home is opened.
    func test_lightGlancesAcrossTheCardEvery18Seconds() throws {
        let body = try Self.notContributedCard()
        XCTAssertTrue(body.contains(".glassCardShimmer()"))
        XCTAssertFalse(body.contains("glassCardBeacon"))
        XCTAssertEqual(GlassCardShimmer.every, .seconds(18))
        XCTAssertLessThan(GlassCardShimmer.duration, 1)
        // Home is built afresh each time its tab is opened, which starts
        // the passes.
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("case .home:\n                    HomeTabView("))
    }

    /// Credit arriving bursts gold coins out of the Balance tile, and the
    /// tile no longer repeats what was received under the 24-hour change.
    func test_creditBurstsCoinsAndTheReceivedLineIsGone() throws {
        let flourish = try Self.text("Views/Monitor/BalanceArrivalFlourish.swift")
        XCTAssertTrue(flourish.contains(".glassCoinBurst(start: burst, still: reduceMotion)"))
        XCTAssertFalse(flourish.contains("strokeBorder"))
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        XCTAssertFalse(home.contains("balanceReceived"), "the received line is back")
        XCTAssertFalse(home.contains("supplementaryDetail"), "the received line is back")
    }
}
