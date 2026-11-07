#if DEBUG
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// Home's Kimi campaign card closes with a small X (owner, 2026-10-27). A
/// closed campaign comes back when what the feed says changes, and Settings
/// can turn the campaign card off altogether. The Insights card and the
/// Traces walkthrough have no X (owner, 2026-11-04).
@MainActor
final class HomeClosedCardsTests: XCTestCase {
    private func campaign(id: String = "kimi-k3", title: String = "Kimi K3.1 Pro",
                          body: String = "Try Kimi with Private AI.", action: String = "Try Kimi",
                          url: String = "https://near.ai/models/kimi", symbol: String = "cpu",
                          image: String? = "https://assets.example/native/kimi.png") throws -> PrivateAICampaign {
        var json: [String: Any] = [
            "schema_version": 1, "id": id, "title": title, "body": body,
            "action_title": action, "action_url": url, "symbol": symbol,
        ]
        if let image { json["image_url"] = image }
        return try JSONDecoder().decode(PrivateAICampaign.self, from: JSONSerialization.data(withJSONObject: json))
    }

    /// Closed, it stays closed while the feed says the same thing; any
    /// change to what the card shows, under the same id or a new one,
    /// brings it back once.
    func testAClosedCampaignComesBackWhenItsContentChanges() throws {
        let shown = try campaign()
        let closed = HomeClosedCards.fingerprint(shown)
        XCTAssertTrue(HomeClosedCards.showsCampaign(HomeClosedCards.fingerprint(shown), closed: "", promotionsOn: true))
        XCTAssertFalse(HomeClosedCards.showsCampaign(HomeClosedCards.fingerprint(try campaign()), closed: closed,
                                                     promotionsOn: true), "unchanged: still closed")
        for changed in [
            try campaign(id: "kimi-k4"), try campaign(title: "Kimi K3.2"), try campaign(body: "Now faster."),
            try campaign(action: "Open Kimi"), try campaign(url: "https://near.ai/models/kimi-k3"),
            try campaign(symbol: "bolt"), try campaign(image: "https://assets.example/native/kimi-2.png"),
            try campaign(image: nil),
        ] {
            XCTAssertTrue(HomeClosedCards.showsCampaign(HomeClosedCards.fingerprint(changed), closed: closed,
                                                        promotionsOn: true), "\(changed) shows again")
        }
    }

    /// Settings' switch off hides the campaign card whatever the feed says,
    /// closed or not.
    func testPromotionsOffHidesTheCampaignCard() throws {
        let print = HomeClosedCards.fingerprint(try campaign())
        XCTAssertFalse(HomeClosedCards.showsCampaign(print, closed: "", promotionsOn: false))
        XCTAssertFalse(HomeClosedCards.showsCampaign(print, closed: "something-else", promotionsOn: false))
    }

    func testHomeAndSettingsReadTheSameKeys() throws {
        let home = try HistoryParityTests.text("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains("@AppStorage(HomeClosedCards.campaignKey) private var closedCampaign = \"\""))
        XCTAssertTrue(home.contains("@AppStorage(HomeClosedCards.promotionsKey) private var promotionsOn = true"))
        XCTAssertTrue(home.contains("onClose: { closedCampaign = HomeClosedCards.fingerprint(campaign) }"))
        XCTAssertFalse(home.contains("insightsClosedOn"))
        XCTAssertTrue(home.contains("@AppStorage(HomeClosedCards.tracesHeroKey) private var tracesHeroClosed = false"))
        XCTAssertTrue(home.contains("onClose: { tracesHeroClosed = true }"))
        let general = try HistoryParityTests.text("Views/Settings/GeneralSection.swift")
        XCTAssertTrue(general.contains("@AppStorage(HomeClosedCards.promotionsKey) private var promotionsOn = true"))
        XCTAssertTrue(general.contains("Toggle(words.promotionsToggle, isOn: $promotionsOn)"))
        XCTAssertNotEqual(HomeClosedCards.campaignKey, HomeClosedCards.promotionsKey)
    }

    /// The switch's words are the core's, and say what it does.
    func testThePromotionsSwitchSpeaksTheCoresWords() throws {
        let words = try XCTUnwrap(MonitorWords.table?.homeHistory)
        XCTAssertEqual(words.promotionsToggle, "Show promotions")
        XCTAssertFalse(words.promotionsTitle.isEmpty)
        XCTAssertFalse(words.promotionsCaption.isEmpty)
    }

    /// The hero card draws an X only when given one: the campaign has one,
    /// and the Traces hero has one that closes it for good (owner,
    /// 2026-11-07); Insights and Missions have none (owner, 2026-11-04).
    func testOnlyCardsGivenACloseDrawAnX() throws {
        let hero = try String(contentsOf: Self.packageRoot
            .appendingPathComponent("Sources/TCDesign/Components/HeroCard.swift"), encoding: .utf8)
        XCTAssertTrue(hero.contains("if let onClose {\n                GlassHeroClose(label: closeLabel ?? \"\", action: onClose)"))
        XCTAssertTrue(hero.contains(".foregroundStyle(GlassTokens.Color.heroInk.color"))
        let home = try HistoryParityTests.text("Views/Monitor/HomeViews.swift")
        XCTAssertEqual(home.components(separatedBy: "onClose:").count - 1, 2)
        XCTAssertEqual(home.components(separatedBy: "GetStartedCard.Close(").count - 1, 0)
    }

    private static var packageRoot: URL {
        URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
    }
}
#endif
