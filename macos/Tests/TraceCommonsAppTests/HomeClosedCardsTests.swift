#if DEBUG
import XCTest
@testable import TraceCommonsApp

/// Home's campaign and Insights cards have no X: the campaign lost its X on
/// 2026-11-05 and Insights on 2026-11-09 (Ron). A close saved by an earlier
/// build no longer hides either card, since with no X it would have hidden
/// it for good. Settings can still turn the campaign card off altogether,
/// and the Traces walkthrough keeps its own X.
@MainActor
final class HomeClosedCardsTests: XCTestCase {
    /// The campaign card shows while promotions are on, whatever an earlier
    /// build saved about closing it; the Insights card always shows.
    func testOnlySettingsSwitchHidesThePromotions() throws {
        let home = try HistoryParityTests.text("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains("if let campaign = campaignStore.campaign, promotionsOn {"))
        XCTAssertTrue(home.contains("if let insightsHeading {\n                    GlassHeroCard("))
        // The keys an earlier build saved its closes under are read no more.
        for gone in ["closedCampaign", "insightsClosedOn", "home.closedCampaignFingerprint",
                     "home.insightsCardClosedOn", "showsCampaign(", "showsInsights("] {
            XCTAssertFalse(home.contains(gone), gone)
        }
    }

    func testHomeAndSettingsReadTheSameKeys() throws {
        let home = try HistoryParityTests.text("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains("@AppStorage(HomeClosedCards.promotionsKey) private var promotionsOn = true"))
        XCTAssertTrue(home.contains("@AppStorage(HomeClosedCards.tracesHeroKey) private var tracesHeroClosed = false"))
        XCTAssertTrue(home.contains("close: GetStartedCard.Close(label: MonitorWords.close) { tracesHeroClosed = true },"))
        let general = try HistoryParityTests.text("Views/Settings/GeneralSection.swift")
        XCTAssertTrue(general.contains("@AppStorage(HomeClosedCards.promotionsKey) private var promotionsOn = true"))
        XCTAssertTrue(general.contains("Toggle(words.promotionsToggle, isOn: $promotionsOn)"))
        XCTAssertNotEqual(HomeClosedCards.promotionsKey, HomeClosedCards.tracesHeroKey)
    }

    /// The switch's words are the core's, and say what it does.
    func testThePromotionsSwitchSpeaksTheCoresWords() throws {
        let words = try XCTUnwrap(MonitorWords.table?.homeHistory)
        XCTAssertEqual(words.promotionsToggle, "Show promotions")
        XCTAssertFalse(words.promotionsTitle.isEmpty)
        XCTAssertFalse(words.promotionsCaption.isEmpty)
    }

    /// The hero card draws an X only when given one, and no Home hero is
    /// given one: not the campaign (Ron, 2026-11-05), not Insights (Ron,
    /// 2026-11-09), not Missions. The Traces walkthrough is the
    /// getting-started card since 2026-11-02 (owner), with its own X.
    func testOnlyCardsGivenACloseDrawAnX() throws {
        let hero = try String(contentsOf: Self.packageRoot
            .appendingPathComponent("Sources/TCDesign/Components/HeroCard.swift"), encoding: .utf8)
        XCTAssertTrue(hero.contains("if let onClose {\n                GlassHeroClose(label: closeLabel ?? \"\", action: onClose)"))
        XCTAssertTrue(hero.contains(".foregroundStyle(GlassTokens.Color.heroInk.color"))
        let home = try HistoryParityTests.text("Views/Monitor/HomeViews.swift")
        XCTAssertEqual(home.components(separatedBy: "onClose:").count - 1, 0)
        XCTAssertEqual(home.components(separatedBy: "closeLabel:").count - 1, 0)
        XCTAssertEqual(home.components(separatedBy: "GetStartedCard.Close(").count - 1, 1)
    }

    private static var packageRoot: URL {
        URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
    }
}
#endif
