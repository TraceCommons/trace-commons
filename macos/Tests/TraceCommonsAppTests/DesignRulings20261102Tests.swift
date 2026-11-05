import XCTest

@testable import TraceCommonsApp

/// The owner's 2026-11-02 asks: the Automatic card over the Traces tree
/// becomes a toggle button with an eye in the toolbar, left of History;
/// the Private AI tab's guide and Home's Traces walkthrough take the
/// Insights empty state's card design, formatting and layout; Private AI
/// is first on Home again; and the balance card's head carries Sign in in
/// the re-read icon's place while a sign-in is needed.
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

    /// Private AI is the first section on Home, above Traces and Account.
    func test_privateAIIsFirstOnHome() throws {
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains(
            "return VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {\n"
                + "            noCredit\n            privateAI\n            HomeSection(words?.sectionTraces) {"))
        XCTAssertEqual(home.components(separatedBy: "            privateAI\n").count - 1, 1)
    }

    /// While a sign-in is needed, or one is running, the balance card's head
    /// carries Sign in where its re-read icon was, and the big button under
    /// the balance is gone; otherwise the re-read icon is back. With a state
    /// line to show, the card is drawn as the sessions card draws its empty
    /// line: the line as the subtitle, no dot, nothing under the head.
    func test_theBalanceCardSignsInFromItsHead() throws {
        XCTAssertTrue(NearAiSignInOffer.balanceHeadSignIn(state: "no_session", renewalFailure: nil, failed: false,
                                                          signingIn: false))
        XCTAssertTrue(NearAiSignInOffer.balanceHeadSignIn(state: "session_expired", renewalFailure: nil,
                                                          failed: false, signingIn: false))
        XCTAssertTrue(NearAiSignInOffer.balanceHeadSignIn(state: "settled", renewalFailure: nil, failed: false,
                                                          signingIn: true), "a running sign-in keeps it")
        XCTAssertFalse(NearAiSignInOffer.balanceHeadSignIn(state: "settled", renewalFailure: nil, failed: false,
                                                           signingIn: false))
        XCTAssertFalse(NearAiSignInOffer.balanceHeadSignIn(state: "unreachable", renewalFailure: nil, failed: false,
                                                           signingIn: false), "unreachable is not a sign-in")

        let account = try Self.text("Views/Monitor/InferenceAccount.swift")
        let start = try XCTUnwrap(account.range(of: "struct PrivateAIBalanceCard: View {"))
        let card = String(account[start.lowerBound...])
        XCTAssertTrue(card.contains(
            "if signIn, let line = BalanceSurface.stateLine(model.balanceStatus, copy: copy, calls: model.balanceCalls) {"))
        XCTAssertTrue(card.contains("subtitle: [line, NearAiSignInOffer.progressLine("))
        XCTAssertTrue(card.contains("trailing: { NearAiSignInHeadButton(copy: copy) })"))
        XCTAssertTrue(card.contains("refresh: signIn ? nil : GlassCardRefresh("))
        XCTAssertTrue(card.contains("if signIn { NearAiSignInHeadButton(copy: copy) }"))
        XCTAssertFalse(try Self.text("Views/BalanceRow.swift").contains("plainLine"))
        // The card it matches: the sessions card's empty line, a subtitle on
        // the head-only card.
        let sessions = try Self.text("Views/ManagedSessionsView.swift")
        XCTAssertTrue(sessions.contains("subtitle: [model.managedText(\"empty\")], trailing: { EmptyView() })"))

        let control = try Self.text("Views/NearAiSignInControl.swift")
        let head = try XCTUnwrap(control.range(of: "struct NearAiSignInHeadButton: View {"))
        let below = try XCTUnwrap(control.range(of: "struct NearAiSignInControl: View {"))
        let button = String(control[head.lowerBound..<below.lowerBound])
        XCTAssertTrue(button.contains("title: model.managedText(\"sign_in_near_ai\")"), "Home's \"Sign in\"")
        XCTAssertTrue(button.contains("style: GlassButtonStyle(.primary, small: true)"))
        XCTAssertFalse(String(control[below.lowerBound...]).contains("NearAiSignInButton("),
                       "the big button under the balance is back")
    }

    /// Every notice's call to action is a primary button (owner,
    /// 2026-11-02); its secondary actions stay links, and the nudge
    /// notice's Not now stays a link.
    func test_everyNoticeCallToActionIsPrimary() throws {
        let notices = try Self.text("Views/Monitor/MonitorNotices.swift")
        XCTAssertFalse(notices.contains("GlassButtonStyle(.glass)"), "a notice's call to action is glass again")
        // 13 since the Traces tab's Resume (owner, 2026-11-04).
        XCTAssertEqual(notices.components(separatedBy: "GlassButtonStyle(.primary)").count - 1, 13)
        XCTAssertTrue(notices.contains("GlassButtonStyle(.link)"), "secondary actions stay links")
        XCTAssertEqual(NudgeGlassCard.style(.seeHistory), .primary)
        XCTAssertEqual(NudgeGlassCard.style(.notNow(.idleSessions)), .link)
    }

    /// The pixel field is on every getting-started card (Ron, 2026-11-05; it
    /// was tried on the Private AI tab's first), out to the card's edges and
    /// corners, behind its words. Home's rolls as waves, to tell it apart.
    func test_thePixelFieldIsOnEveryGettingStartedCard() throws {
        XCTAssertTrue(try Self.text("Views/Monitor/InferenceViews.swift").contains("pixelField: true"))
        XCTAssertTrue(try Self.text("Views/Monitor/HomeViews.swift").contains("pixelField: true, fieldPattern: .waves)"))
        XCTAssertTrue(try Self.text("Views/InsightsGetStarted.swift").contains("pixelField: true"))
        let card = try Self.text("Views/GetStartedCard.swift")
        XCTAssertTrue(card.contains("var pixelField = false"))
        XCTAssertTrue(card.contains(".background { if pixelField { GetStartedCardField(pattern: fieldPattern) } }\n        .insightsSection()"))
        XCTAssertTrue(card.contains(".clipShape(RoundedRectangle(cornerRadius: GlassTokens.Radius.card, style: .continuous))"))
        XCTAssertTrue(card.contains(".padding(.horizontal, -GlassTokens.Space.cardPaddingHorizontal)"))
    }

    /// The Private AI tab's heading: its state in a pill, as Settings draws
    /// Watching, and the line under it regular and secondary, below the
    /// section title in weight.
    func test_thePrivateAIHeadingPillsItsStateAndQuietsItsLine() throws {
        let views = try Self.text("Views/Monitor/InferenceViews.swift")
        XCTAssertTrue(views.contains("GlassTabHeading(copy.inspectorStatus) {\n                GlassChip(status.word, status: status.dot)"))
        XCTAssertTrue(views.contains(
            "Text(Self.subtitle(copy, state: store.privateAI?.state))\n"
                + "                .glassType(GlassTokens.TypeScale.body)\n"
                + "                .foregroundStyle(GlassColor.textSecondary)"))
        XCTAssertTrue(try Self.text("Views/Monitor/SettingsModal.swift").contains("GlassChip(chip.word, status: chip.status)"),
                      "the pill it matches")
    }

    /// Every refresh control is a pill with the core's "Refresh" before its
    /// icon (owner, 2026-11-02), as tall as the round button it replaced;
    /// a control whose accessible name says more draws the word alone, and
    /// that name still begins with it.
    func test_everyRefreshIsAPillWithItsWord() throws {
        let containers = try String(
            contentsOf: GlassSurfaceRulesTests.root.deletingLastPathComponent()
                .appendingPathComponent("TCDesign/Components/Containers.swift"), encoding: .utf8)
        let start = try XCTUnwrap(containers.range(of: "struct GlassRefreshPill: View {"))
        let end = try XCTUnwrap(containers.range(of: "public struct GlassCardRefresh {"))
        let pill = String(containers[start.lowerBound..<end.lowerBound])
        let word = try XCTUnwrap(pill.range(of: "Text(title)"))
        let icon = try XCTUnwrap(pill.range(of: "Image(systemName: GlassRefreshButton.systemImage)"))
        XCTAssertLessThan(word.lowerBound, icon.lowerBound, "the word is left of the icon")
        XCTAssertTrue(pill.contains(".glassSurface(.control, radius: height / 2, hover: GlassTokens.Color.controlHover)"))
        XCTAssertFalse(containers.contains("GlassRoundButton(label, icon: .symbol(Self.systemImage)"), "the round refresh is back")

        let account = try Self.text("Views/Monitor/InferenceAccount.swift")
        XCTAssertTrue(account.contains("copy.panelBalanceRefreshAccessibility, title: copy.panelBalanceRefresh,"))
        XCTAssertTrue(account.contains("refresh: copy.panelToolsRefreshAccessibility, refreshWord: copy.panelRefresh,"))
        XCTAssertTrue(try Self.text("Views/PrivateInferenceView.swift")
            .contains("refresh: copy.panelConnectionRefreshAccessibility, refreshWord: copy.panelRefresh,"))
        XCTAssertTrue(try Self.text("Views/ManagedSessionsView.swift")
            .contains("refresh: .init(model.managedText(\"refresh\"), isDisabled: model.managedBusy) {"))
        XCTAssertTrue(try Self.text("Views/InsightsUsageCards.swift")
            .contains("GlassRefreshButton(accessibility, title: SettingsLegacyWords.refresh, small: true,"))
    }
}
