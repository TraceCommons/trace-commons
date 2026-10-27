#if DEBUG
import XCTest
@testable import TraceCommonsApp

/// Home's Kimi campaign and Insights cards each close with a small X, for
/// good on this Mac; a new campaign shows again (owner, 2026-10-27).
@MainActor
final class HomeClosedCardsTests: XCTestCase {
    func testAClosedCampaignStaysClosedAndANewOneShows() {
        XCTAssertTrue(HomeClosedCards.showsCampaign("kimi-k3", closed: ""))
        XCTAssertFalse(HomeClosedCards.showsCampaign("kimi-k3", closed: "kimi-k3"))
        XCTAssertTrue(HomeClosedCards.showsCampaign("kimi-k4", closed: "kimi-k3"))
    }

    func testBothCardsDrawTheXAndHideOnceClosed() throws {
        let home = try HistoryParityTests.text("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains("HomeClosedCards.showsCampaign(campaign.id, closed: closedCampaign)"))
        XCTAssertTrue(home.contains("closeLabel: MonitorWords.close, onClose: { closedCampaign = campaign.id }"))
        XCTAssertTrue(home.contains("if let insightsHeading, !insightsClosed {"))
        XCTAssertTrue(home.contains("closeLabel: MonitorWords.close, onClose: { insightsClosed = true }"))
    }

    /// Kept in this Mac's defaults, under keys of their own, so a restart
    /// keeps them closed.
    func testTheClosedCardsPersistInDefaults() throws {
        let home = try HistoryParityTests.text("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains("@AppStorage(HomeClosedCards.campaignKey) private var closedCampaign = \"\""))
        XCTAssertTrue(home.contains("@AppStorage(HomeClosedCards.insightsKey) private var insightsClosed = false"))
        XCTAssertNotEqual(HomeClosedCards.campaignKey, HomeClosedCards.insightsKey)

        let defaults = try XCTUnwrap(UserDefaults(suiteName: "HomeClosedCardsTests"))
        defer { defaults.removePersistentDomain(forName: "HomeClosedCardsTests") }
        defaults.set("kimi-k3", forKey: HomeClosedCards.campaignKey)
        let closed = defaults.string(forKey: HomeClosedCards.campaignKey) ?? ""
        XCTAssertFalse(HomeClosedCards.showsCampaign("kimi-k3", closed: closed))
    }

    /// The hero card draws an X only when given one: Missions has none.
    func testOnlyCardsGivenACloseDrawAnX() throws {
        let hero = try String(contentsOf: Self.packageRoot
            .appendingPathComponent("Sources/TCDesign/Components/HeroCard.swift"), encoding: .utf8)
        XCTAssertTrue(hero.contains("if let onClose {\n                GlassHeroClose(label: closeLabel ?? \"\", action: onClose)"))
        XCTAssertTrue(hero.contains(".foregroundStyle(GlassTokens.Color.heroInk.color"))
        let home = try HistoryParityTests.text("Views/Monitor/HomeViews.swift")
        XCTAssertEqual(home.components(separatedBy: "onClose:").count - 1, 2)
    }

    private static var packageRoot: URL {
        URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
    }
}
#endif
