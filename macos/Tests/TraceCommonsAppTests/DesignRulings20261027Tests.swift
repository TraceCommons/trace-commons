import SwiftUI
import TCDesign
import TCShellCore
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
        XCTAssertTrue(flourish.contains(".glassCoinBurst(start: burst, still: reduceMotion, trailingRoom: Self.edgeRoom)"))
        XCTAssertFalse(flourish.contains("strokeBorder"))
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        XCTAssertFalse(home.contains("balanceReceived"), "the received line is back")
        XCTAssertFalse(home.contains("supplementaryDetail"), "the received line is back")
        // Nothing on macOS reads the core's received-in-24-hours figures.
        let fields = Mirror(reflecting: BalanceStatus.unreported).children.compactMap(\.label)
        XCTAssertTrue(fields.contains("receivedTotalNanos"))
        XCTAssertFalse(fields.contains { $0.hasPrefix("received24h") }, "the received line's fields are back")
    }

    /// The coins fall over the cards below the Balance tile, on Home and in
    /// the edge rail, rather than under them.
    func test_theCoinsFallOverTheCardsBelow() throws {
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        let account = try XCTUnwrap(home.range(of: "HomeSection(words?.sectionAccount) {"))
        let raised = try XCTUnwrap(home.range(of: ".zIndex(1)", range: account.upperBound..<home.endIndex))
        let privateAI = try XCTUnwrap(home.range(of: "privateAI\n", range: account.upperBound..<home.endIndex))
        XCTAssertLessThan(raised.lowerBound, privateAI.lowerBound, "the Account section is under Private AI")
        XCTAssertTrue(home.contains("stats\n                    .zIndex(1)"), "the tiles are under the cards below")
        let rail = try Self.text("Views/EdgeRail/EdgeRailView.swift")
        let tiles = try XCTUnwrap(rail.range(of: "HomeAccountTiles(store: data.home) { open(.inference) }\n"))
        let railRaised = try XCTUnwrap(rail.range(of: ".zIndex(1)", range: tiles.upperBound..<rail.endIndex))
        let map = try XCTUnwrap(rail.range(of: "EdgeRailPrivateAIMap(", range: tiles.upperBound..<rail.endIndex))
        XCTAssertLessThan(railRaised.lowerBound, map.lowerBound, "the rail's tiles are under its map")
    }

    /// A balance refresh mid-burst does not cut the coins off: they clear on
    /// a `.task(id: burst)`, which only a newer burst or the tile going away
    /// restarts or ends.
    func test_aRefreshMidBurstLetsTheCoinsLand() throws {
        let flourish = try Self.text("Views/Monitor/BalanceArrivalFlourish.swift")
        XCTAssertTrue(flourish.contains(".task(id: burst) {"))
        XCTAssertTrue(flourish.contains("guard !Task.isCancelled else { return }\n                burst = nil"))
        XCTAssertEqual(flourish.components(separatedBy: "burst = nil").count - 1, 1, "cleared in one place")
    }
}
