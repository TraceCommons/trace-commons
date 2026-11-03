import XCTest

@testable import TraceCommonsApp

/// The owner's 2026-11-02 asks: the Automatic card over the Traces tree
/// becomes a toggle button with an eye in the toolbar, left of History;
/// and the Private AI tab's guide and Home's Traces walkthrough take the
/// Insights empty state's card design, formatting and layout.
@MainActor
final class DesignRulings20261102Tests: XCTestCase {
    private static func text(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    /// The eye sits between search and History in the toolbar row, and the
    /// card that held the switch over the tree is gone. The toolbar is the
    /// same wherever it is drawn, a filtered empty list included.
    func test_theAutomaticEyeSitsLeftOfHistory() throws {
        let controls = try Self.text("Views/Monitor/NudgeViews.swift")
        let search = try XCTUnwrap(controls.range(of: "                search(tree)\n"))
        let eye = try XCTUnwrap(controls.range(of: "if let automatic { SharingChoiceCard(toolbar: automatic) }"))
        let history = try XCTUnwrap(controls.range(of: "GlassRoundButton(tree.openHistory, systemImage: Self.historyIcon"))
        XCTAssertLessThan(search.upperBound, eye.lowerBound)
        XCTAssertLessThan(eye.upperBound, history.lowerBound)

        let traces = try Self.text("Views/Monitor/TracesViews.swift")
        XCTAssertEqual(traces.components(separatedBy: "SharingChoiceCard(").count - 1, 0, "no card over the tree")
        XCTAssertEqual(
            traces.components(separatedBy: "TracesListControls(store: store, openHistory: openHistory, automatic: automaticToggle)")
                .count - 1, 2)
    }

    /// The eye is a toggle on the round button, on while Automatic is, and
    /// pressing it takes the same steps as Settings' radios: on goes
    /// through the keep-Ask question and setup's disclosures, off is Ask
    /// each time. Its count is in the tooltip; it is hidden, not dimmed,
    /// until the grant and its words are known.
    func test_theEyeTogglesAutomaticThroughTheSameSteps() throws {
        XCTAssertEqual(SharingChoice.toolbarIcon, "eye")
        XCTAssertEqual(SharingChoice.toolbarHelp(name: "Automatic for every folder", line: "0 of 3 folders automatic."),
                       "Automatic for every folder\n0 of 3 folders automatic.")
        XCTAssertEqual(SharingChoice.toolbarHelp(name: "Automatic", line: ""), "Automatic")
        XCTAssertTrue(SharingChoice.drawsToolbarButton(grantKnown: true, wordsKnown: true))
        XCTAssertFalse(SharingChoice.drawsToolbarButton(grantKnown: false, wordsKnown: true))
        XCTAssertFalse(SharingChoice.drawsToolbarButton(grantKnown: true, wordsKnown: false))

        let card = try Self.text("Views/Settings/SharingChoiceCard.swift")
        XCTAssertTrue(card.contains("GlassRoundButton(words.accessibleName, systemImage: SharingChoice.toolbarIcon, isOn: on,"))
        XCTAssertTrue(card.contains("choose(on ? .askMe : .automatic)"))
        XCTAssertTrue(card.contains("help: SharingChoice.toolbarHelp(name: words.accessibleName, line: words.line)"))
        XCTAssertFalse(card.contains("GlassCard {"), "the card over the tree is gone")
        XCTAssertFalse(card.contains("GlassToggleStyle("), "a toolbar toggle, not a switch")
        // The steps behind it are Settings' own, unchanged.
        XCTAssertTrue(card.contains("case .keepAsks(let count): keepAsksCount = count"))
        XCTAssertTrue(card.contains("disclosure = SharingDisclosureFlow()"))
    }

    /// One card for all three: Insights' empty state, the Private AI guide
    /// and Home's Traces walkthrough draw the same view, which has no
    /// gradient, graphic or pill, and an X only when given one.
    func test_theThreeGettingStartedCardsAreOneDesign() throws {
        XCTAssertTrue(try Self.text("Views/InsightsGetStarted.swift").contains("GetStartedCard("))
        XCTAssertTrue(try Self.text("Views/Monitor/InferenceViews.swift").contains("GetStartedCard("))
        XCTAssertTrue(try Self.text("Views/Monitor/HomeViews.swift").contains("GetStartedCard("))
        let card = try Self.text("Views/GetStartedCard.swift")
        for absent in ["Gradient", "GlassHeroCard(", "GlassHeroSymbol(", "heroInk", "actionOnPill"] {
            XCTAssertFalse(card.contains(absent), absent)
        }
        XCTAssertTrue(card.contains("if let close { GetStartedCardClose(close: close) }"))
        XCTAssertTrue(card.contains(".insightsSection()"), "the Insights card's own section")
        XCTAssertTrue(card.contains("Image(systemName: \"arrow.right\")"), "a step's action is a text link")
    }
}
