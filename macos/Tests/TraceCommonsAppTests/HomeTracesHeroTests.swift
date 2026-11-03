import TCBridge
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Home's Traces hero (owner, 2026-10-28): the macOS MVP is "contribute
/// traces, earn credit", so Home has one card that says so and opens
/// Traces. Condensed on 2026-11-02 (owner): shorter words, no line under
/// the title, an X that closes it, steps drawn as first run's screens, and
/// in place of the Traces cards only while no folder is watched or on the
/// launch that finished first run.
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

    func test_theHeroOpensTracesAndItsStepsOpenTheirTabs() throws {
        let body = try Self.hero()
        XCTAssertTrue(body.contains("GlassHeroCard("))
        XCTAssertTrue(body.contains("title: words.tracesHeroTitle,\n"), "no subtitle under the title")
        XCTAssertFalse(body.contains("subtitle:"))
        XCTAssertTrue(body.contains("actionTitle: words.tracesHeroAction, action: openTraces"))
        // The steps and the title both run the card's width; only the X
        // keeps the title off the trailing edge.
        XCTAssertTrue(body.contains("contentSpansGraphic: true, titleSpansGraphic: true, actionOnPill: true"))
        XCTAssertTrue(body.contains("HomeHeroSteps(steps: ["))
        // Contribute opens Traces and Use Private AI opens Private AI, each
        // its own button, so the hero is a panel and only its pill is the
        // hero's button; Earn credit opens nothing (owner, 2026-10-29).
        XCTAssertTrue(body.contains("detail: words.tracesHeroStepContributeDetail,\n                          action: openTraces),"))
        XCTAssertTrue(body.contains("detail: words.tracesHeroStepEarnDetail),"))
        XCTAssertTrue(body.contains("detail: words.tracesHeroStepUseDetail,\n                          action: openPrivateAI),"))
    }

    /// The X closes the hero for good on this Mac, with the close word the
    /// campaign's and Insights' X use.
    func test_theXClosesTheHeroOnThisMac() throws {
        let home = try Self.home()
        XCTAssertTrue(home.contains(
            "@AppStorage(HomeClosedCards.tracesHeroKey) private var tracesHeroClosed = false"))
        let body = try Self.hero()
        XCTAssertTrue(body.contains("if let words, !tracesHeroClosed {"))
        XCTAssertTrue(body.contains("closeLabel: MonitorWords.close, onClose: { tracesHeroClosed = true },"))
    }

    /// Each step shows its first-run screen in small (owner, 2026-11-02:
    /// "match the applicable FTUX dialogue visuals and screens"), joined by a
    /// lit stretch of first run's step thread; no numbered disc, no arrow,
    /// and no card behind it (owner, 2026-11-02: "remove the step card
    /// backgrounds"), so its words are the hero's ink. The pictures are
    /// still: no timeline.
    func test_theStepsShowFirstRunScreensWithNoCard() throws {
        let home = try Self.home()
        let start = try XCTUnwrap(home.range(of: "struct HomeHeroSteps: View {"))
        let end = try XCTUnwrap(home.range(of: "struct HomeHeroRing: View {", range: start.upperBound..<home.endIndex))
        let steps = home[start.upperBound..<end.lowerBound]
        XCTAssertFalse(steps.contains("Text(\"\\(step.number)\")"))
        XCTAssertFalse(steps.contains("arrow.right"))
        XCTAssertTrue(steps.contains("if step.number > 1 { HomeHeroThread() }"))
        XCTAssertTrue(steps.contains("HomeHeroStepGraphic(graphic: step.graphic)"))
        let panelStart = try XCTUnwrap(steps.range(of: "struct HomeHeroStepPanel: View {"))
        let panel = steps[panelStart.upperBound...]
        XCTAssertFalse(panel.contains("GlassTokens.Color.firstRunCard"), "a card is back behind the step")
        XCTAssertFalse(panel.contains("strokeBorder"), "a card's edge is back round the step")
        XCTAssertFalse(panel.contains(".background("), "a background is back behind the step")
        XCTAssertTrue(panel.contains("let ink = GlassTokens.Color.heroInk.color"), "the hero's ink")
        XCTAssertFalse(panel.contains("GlassColor.textPrimary"), "a card's ink on the hero's ground")
        for piece in ["FirstRunToolFace(tool: .claudeCode)", "FirstRunToolFace(tool: .codex)", "FirstRunOrb(",
                      "GlassBrandMarkDrawing(fill: glow.color)", "GlassTokens.Color.firstRunBeamCore"] {
            XCTAssertTrue(steps.contains(piece), piece)
        }
        XCTAssertFalse(steps.contains("GlassAnimationTimeline"), "a step's picture moves")
        XCTAssertFalse(steps.contains("FirstRunHub("), "first run's hub breathes")
        let hero = try Self.hero()
        XCTAssertTrue(hero.contains(".init(number: 1, graphic: .tools,"))
        XCTAssertTrue(hero.contains(".init(number: 2, graphic: .traces,"))
        XCTAssertTrue(hero.contains(".init(number: 3, graphic: .privateAI,"))
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
    /// heading, and the launch that finished first run is the model's,
    /// set where first run is marked done.
    func test_theWalkthroughTakesTheCardsPlace() throws {
        let home = try Self.home()
        XCTAssertTrue(home.contains(
            "HomeSection(words?.sectionTraces) {\n                if showsTracesWalkthrough {\n"
                + "                    tracesHero\n                } else {\n                    Grid("))
        XCTAssertEqual(home.components(separatedBy: "                    tracesHero\n").count - 1, 1, "drawn once")
        XCTAssertTrue(home.contains("!tracesHeroClosed && words != nil && HomeFormat.showsTracesWalkthrough("))
        let model = try String(
            contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent("AppModel.swift"), encoding: .utf8)
        XCTAssertTrue(model.contains("    private(set) var finishedFirstRunThisLaunch = false"))
        XCTAssertTrue(model.contains(
            "UserDefaults.standard.set(true, forKey: Self.onboardingCompleteKey(tenantID))\n"
                + "        finishedFirstRunThisLaunch = true"))
    }
}
