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

    /// Each step's line says what happens in it, in a sentence or two (Ron,
    /// 2026-11-09: the earlier three-word lines were too short), and the
    /// title says what the three of them add up to.
    func test_theHeroSaysWhatEachStepDoes() throws {
        let words = try XCTUnwrap(MonitorWords.table).homeHistory
        XCTAssertEqual(words.tracesHeroTitle, "Your traces earn Private AI credit")
        XCTAssertEqual(words.tracesHeroAction, "Start earning")
        XCTAssertEqual(words.tracesHeroStepContribute, "Contribute")
        XCTAssertEqual(words.tracesHeroStepContributeDetail,
                       "Share traces from your coding tools. Keys, emails and file paths are removed before they're sent.")
        XCTAssertEqual(words.tracesHeroStepEarn, "Earn credit")
        XCTAssertEqual(words.tracesHeroStepEarnDetail,
                       "Each accepted trace earns credit. It becomes final once it settles.")
        XCTAssertEqual(words.tracesHeroStepUse, "Use Private AI")
        XCTAssertEqual(words.tracesHeroStepUseDetail,
                       "Spend your credit on Claude Code, Codex or OpenCode sessions through Private AI.")
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
        XCTAssertTrue(body.contains(".init(title: words.tracesHeroStepUse, detail: words.tracesHeroStepUseDetail),"))
        XCTAssertEqual(body.components(separatedBy: "link:").count - 1, 1, "one clear action")
    }

    /// The X closes the hero for good on this Mac, with the close word the
    /// campaign's and Insights' X use.
    func test_theXClosesTheHeroOnThisMac() throws {
        let home = try Self.home()
        XCTAssertTrue(home.contains(
            "@AppStorage(HomeClosedCards.tracesHeroKey) private var tracesHeroClosed = false"))
        let body = try Self.hero()
        XCTAssertTrue(body.contains("if let words, !tracesHeroClosed {"))
        XCTAssertTrue(body.contains("close: GetStartedCard.Close(label: MonitorWords.close) { tracesHeroClosed = true },"))
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

    /// In place of the Traces cards on the launch that finished first run,
    /// or once the tree has loaded with no folder watched: none, or every
    /// one set to Never. A tree still loading shows the cards.
    func test_theWalkthroughStandsInForTheCardsUntilAFolderIsWatched() {
        let rule = HomeFormat.showsTracesWalkthrough
        let ask = TracesTree.FolderNode(id: "a", label: "a", mode: .ask, sessions: [])
        let never = TracesTree.FolderNode(id: "n", label: "n", mode: .ignore, sessions: [])
        let unruled = TracesTree.FolderNode(id: "u", label: "u", mode: nil, sessions: [])
        XCTAssertTrue(rule([], true, false), "nothing set up")
        XCTAssertTrue(rule([never], true, false), "nothing watched")
        XCTAssertFalse(rule([never, ask], true, false))
        XCTAssertFalse(rule([unruled], true, false), "a folder the queue names is watched")
        XCTAssertFalse(rule([], false, false), "still loading")
        XCTAssertTrue(rule([ask], true, true), "the launch that finished first run")
        XCTAssertTrue(rule([], false, true))
    }

    /// The walkthrough takes the Traces cards' place under the Traces
    /// heading, under the no-credit line when that shows (Ron, 2026-11-09),
    /// and the launch that finished first run is the model's, set where
    /// first run is marked done.
    func test_theWalkthroughTakesTheCardsPlace() throws {
        let home = try Self.home()
        XCTAssertTrue(home.contains(
            "                noCredit\n                if showsTracesWalkthrough {\n"
                + "                    tracesHero\n                } else {\n                    Grid("))
        XCTAssertEqual(home.components(separatedBy: "                    tracesHero\n").count - 1, 1, "drawn once")
        XCTAssertTrue(home.contains("!tracesHeroClosed && words != nil && HomeFormat.showsTracesWalkthrough("))
        // Every folder, not the tree the idle filter narrowed (#1888 review).
        XCTAssertTrue(home.contains("folders: traces.everyFolder, loaded: traces.phase == .loaded,"))
        XCTAssertFalse(home.contains("folders: traces.tree.folders"))
        let model = try String(
            contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent("AppModel.swift"), encoding: .utf8)
        XCTAssertTrue(model.contains("    private(set) var finishedFirstRunThisLaunch = false"))
        XCTAssertTrue(model.contains(
            "UserDefaults.standard.set(true, forKey: Self.onboardingCompleteKey(tenantID))\n"
                + "        finishedFirstRunThisLaunch = true"))
    }
}
