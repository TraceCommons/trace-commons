import TCBridge
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Home's Traces hero, the product owner's design (2026-10-28, 2026-10-29;
/// restored 2026-11-05): the first and largest card on Home, with three
/// illustrated steps, for everyone, every time, with no X. These tests pin
/// it; `.github/CODEOWNERS` makes the product owner the reviewer of this
/// file and of `HomeTracesHero.swift`.
@MainActor
final class HomeTracesHeroTests: XCTestCase {
    private static func text(_ path: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(path), encoding: .utf8)
    }

    /// The owner's words.
    func test_theHeroSpeaksTheOwnersWords() throws {
        let words = try XCTUnwrap(MonitorWords.table).homeHistory
        XCTAssertEqual(words.tracesHeroTitle, "Contribute traces, earn credit, use Private AI")
        XCTAssertEqual(
            words.tracesHeroBody,
            "Share the sessions from your AI coding tools. Each accepted trace earns you credit.")
        XCTAssertEqual(words.tracesHeroAction, "Start earning")
        XCTAssertEqual(words.tracesHeroStepContribute, "Contribute traces")
        XCTAssertEqual(words.tracesHeroStepContributeDetail, "Share the sessions from your AI coding tools")
        XCTAssertEqual(words.tracesHeroStepEarn, "Earn credit")
        XCTAssertEqual(words.tracesHeroStepEarnDetail, "Each accepted trace earns you credit")
        XCTAssertEqual(words.tracesHeroStepUse, "Use Private AI")
        XCTAssertEqual(words.tracesHeroStepUseDetail, "Spend your credit on Private AI")
    }

    /// The gradient hero with its ring, its line, Start earning on its pill
    /// and the three illustrated steps; Contribute opens Traces and Use
    /// Private AI opens Private AI.
    func test_theHeroIsTheOwnersDesign() throws {
        let hero = try Self.text("Views/Monitor/HomeTracesHero.swift")
        XCTAssertTrue(hero.contains("GlassHeroCard(\n            gradient: GlassTokens.Gradient.heroMissions,"))
        XCTAssertTrue(hero.contains("title: words.tracesHeroTitle, subtitle: words.tracesHeroBody,"))
        XCTAssertTrue(hero.contains("actionTitle: words.tracesHeroAction, action: openTraces,"))
        XCTAssertTrue(hero.contains("contentSpansGraphic: true, actionOnPill: true"))
        XCTAssertTrue(hero.contains("HomeHeroRing()"))
        XCTAssertTrue(hero.contains(
            ".init(number: 1, systemImage: \"apple.terminal\",\n"
                + "                      title: words.tracesHeroStepContribute, detail: words.tracesHeroStepContributeDetail,\n"
                + "                      action: openTraces),"))
        XCTAssertTrue(hero.contains(
            ".init(number: 2, systemImage: \"cylinder.split.1x2\",\n"
                + "                      title: words.tracesHeroStepEarn, detail: words.tracesHeroStepEarnDetail),"))
        XCTAssertTrue(hero.contains(
            ".init(number: 3, systemImage: \"sparkles\",\n"
                + "                      title: words.tracesHeroStepUse, detail: words.tracesHeroStepUseDetail,\n"
                + "                      action: openPrivateAI),"))
        for kept in ["struct HomeHeroSteps: View {", "struct HomeHeroStepPanel: View {", "struct HomeHeroRing: View {"] {
            XCTAssertTrue(hero.contains(kept), kept)
        }
    }

    /// For everyone, new and returning, until its X closes it for good on
    /// this Mac (owner, 2026-11-07): no other condition but the core's words
    /// having loaded, and nothing that hides it for returning people.
    func test_theHeroShowsUntilItsXClosesItForGood() throws {
        let hero = try Self.text("Views/Monitor/HomeTracesHero.swift")
        XCTAssertTrue(hero.contains("closeLabel: MonitorWords.close, onClose: onClose,"))
        XCTAssertFalse(hero.contains("@AppStorage"))
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains(
            "    private var tracesHero: some View {\n        if let words, !tracesHeroClosed {\n"
                + "            HomeTracesHero(words: words, openTraces: openTraces, openPrivateAI: openPrivateAI,\n"
                + "                           onClose: { tracesHeroClosed = true })\n"
                + "        }\n    }\n"))
        XCTAssertTrue(home.contains(
            "@AppStorage(HomeClosedCards.tracesHeroKey) private var tracesHeroClosed = false"))
        XCTAssertEqual(HomeClosedCards.tracesHeroKey, "home.tracesHeroStepsClosed")
        for absent in ["showsTracesWalkthrough", "finishedFirstRunThisLaunch"] {
            XCTAssertFalse(home.contains(absent), absent)
        }
    }

    /// It leads Home: the first card, over every section.
    func test_theHeroLeadsHome() throws {
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains(
            "        return VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {\n"
                + "            tracesHero\n            noCredit\n            privateAI\n            HomeSection(words?.sectionTraces) {\n"))
        XCTAssertEqual(home.components(separatedBy: "            tracesHero\n").count - 1, 1, "drawn once")
    }

    /// The owner is the reviewer of the hero and of these tests.
    func test_theOwnerReviewsTheHero() throws {
        let owners = try String(
            contentsOf: GlassSurfaceRulesTests.root.deletingLastPathComponent().deletingLastPathComponent()
                .deletingLastPathComponent().appendingPathComponent(".github/CODEOWNERS"), encoding: .utf8)
        XCTAssertTrue(owners.contains("/macos/Sources/TraceCommonsApp/Views/Monitor/HomeTracesHero.swift @poldsam"))
        XCTAssertTrue(owners.contains("/macos/Tests/TraceCommonsAppTests/HomeTracesHeroTests.swift @poldsam"))
    }
}
