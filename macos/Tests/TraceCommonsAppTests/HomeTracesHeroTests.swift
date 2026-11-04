import TCBridge
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Home's Traces hero (owner, 2026-10-28): the macOS MVP is "contribute
/// traces, earn credit", so Home has one card that says so and opens
/// Traces. Condensed on 2026-11-02 (owner): shorter words, no line under
/// the title, an X that closes it, and in place of the Traces cards only
/// while no folder is watched or on the launch that finished first run.
/// Later that day it took the Insights empty state's card (owner,
/// `GetStartedCard`): a plain card, numbered steps, no pictures.
@MainActor
final class HomeTracesHeroTests: XCTestCase {
    private static func home() throws -> String {
        try String(
            contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent("Views/Monitor/HomeViews.swift"), encoding: .utf8)
    }

    private static func hero() throws -> Substring {
        let home = try home()
        let start = try XCTUnwrap(home.range(of: "private var tracesHero: some View {"))
        let end = try XCTUnwrap(home.range(of: "/// Home's Private AI section", range: start.upperBound..<home.endIndex))
        return home[start.upperBound..<end.lowerBound]
    }

    /// Each step's title and line read as one short sentence, and the
    /// title says what the three of them add up to.
    func test_theHeroSaysItInFewWords() throws {
        let words = try XCTUnwrap(MonitorWords.table).homeHistory
        XCTAssertEqual(words.tracesHeroTitle, "Your traces earn Private AI credit")
        XCTAssertEqual(words.tracesHeroAction, "Start earning")
        XCTAssertEqual(words.tracesHeroStepContribute, "Contribute")
        XCTAssertEqual(words.tracesHeroStepContributeDetail, "Traces from tools")
        XCTAssertEqual(words.tracesHeroStepEarn, "Earn credit")
        XCTAssertEqual(words.tracesHeroStepEarnDetail, "For each trace")
        XCTAssertEqual(words.tracesHeroStepUse, "Use Private AI")
        XCTAssertEqual(words.tracesHeroStepUseDetail, "With your credit")
        // The line under the title is gone from the core's table and the
        // shell's copy alike.
        let copy = try String(
            contentsOf: GlassSurfaceRulesTests.root.deletingLastPathComponent()
                .appendingPathComponent("TCShellCore/MonitorTracesCopy.swift"), encoding: .utf8)
        XCTAssertFalse(copy.contains("tracesHeroBody"))
        XCTAssertFalse(copy.contains("traces_hero_body"))
    }

    /// The walkthrough is the shared getting-started card: its title with
    /// no line under it, the three steps in order, and Start earning as the
    /// one action, a text link under Contribute that opens Traces.
    func test_theHeroIsTheGetStartedCardAndStartEarningOpensTraces() throws {
        let body = try Self.hero()
        XCTAssertTrue(body.contains("GetStartedCard(\n                title: words.tracesHeroTitle,\n                steps: ["))
        XCTAssertFalse(body.contains("lead:"), "no line under the title")
        XCTAssertTrue(body.contains(
            "detail: words.tracesHeroStepContributeDetail,\n"
                + "                          link: GetStartedCard.Link(title: words.tracesHeroAction, action: openTraces)),"))
        XCTAssertTrue(body.contains(".init(title: words.tracesHeroStepEarn, detail: words.tracesHeroStepEarnDetail),"))
        XCTAssertTrue(body.contains(
            ".init(title: words.tracesHeroStepUse, detail: words.tracesHeroStepUseDetail,\n"
                + "                          link: GetStartedCard.Link(title: words.tracesHeroUseAction, action: openPrivateAI)),"))
        XCTAssertEqual(body.components(separatedBy: "link:").count - 1, 2, "a link to Traces and one to Private AI")
    }

    /// The walkthrough shows to everyone, every time, with no X (owner,
    /// 2026-11-04): its links to the tabs are why it stays on Home.
    func test_theHeroHasNoX() throws {
        let home = try Self.home()
        XCTAssertFalse(home.contains("tracesHeroClosed"))
        XCTAssertFalse(home.contains("tracesHeroKey"))
        let body = try Self.hero()
        XCTAssertTrue(body.contains("if let words {"))
        XCTAssertFalse(body.contains("close:"))
    }

    /// No gradient, ring or first-run pictures any more: the card is plain
    /// (owner, 2026-11-02), and the views that drew them are gone.
    func test_theHeroDrawsNoGradientOrPictures() throws {
        let body = try Self.hero()
        XCTAssertFalse(body.contains("GlassHeroCard("))
        XCTAssertFalse(body.contains("heroMissions"))
        let home = try Self.home()
        for gone in ["struct HomeHeroSteps", "struct HomeHeroThread", "struct HomeHeroStepGraphic",
                     "struct HomeHeroStepPanel", "struct HomeHeroRing"] {
            XCTAssertFalse(home.contains(gone), gone)
        }
    }

    /// The walkthrough leads the Traces section, over the Traces cards,
    /// whatever folders are watched (owner, 2026-11-04).
    func test_theWalkthroughLeadsTheTracesCards() throws {
        let home = try Self.home()
        XCTAssertTrue(home.contains(
            "HomeSection(words?.sectionTraces) {\n"
                + "                VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {\n"
                + "                    tracesHero\n                    Grid("))
        XCTAssertEqual(home.components(separatedBy: "                    tracesHero\n").count - 1, 1, "drawn once")
        XCTAssertFalse(home.contains("showsTracesWalkthrough"))
    }
}
