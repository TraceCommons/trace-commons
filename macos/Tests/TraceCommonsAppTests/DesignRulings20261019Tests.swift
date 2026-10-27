import SwiftUI
import TCBridge
import TCDesign
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The owner's 2026-10-19 review of the inspector, the Summary, the menu
/// bar's popover and the popover notices, pinned as the parity tests pin
/// structure.
@MainActor
final class DesignRulings20261019Tests: XCTestCase {
    private static func text(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    /// Top projects and top tools carry their tile on the left, as the
    /// Traces tree's rows do: the folder glyph, and the tool's logo.
    func test_theSummaryListsCarryTheirTiles() throws {
        let words = try XCTUnwrap(MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON()))
        let entry = try recordedEntry([
            "entry_id": "00000000-0000-4000-8000-000000000001", "project_id": "p1", "project_label": "payments",
            "source": "claude-code", "declared_source": "claude-code",
        ])
        let folder = TracesTree.FolderNode(id: "p1", label: "payments", mode: .ask, sessions: [entry])
        let projects = SummaryFacts.topProjects(folders: [folder], history: [], counts: words.counts)
        XCTAssertEqual(projects.first?.tile, GlassToolTile.Kind.folder)
        let tools = SummaryFacts.topTools(sessions: [entry], history: [], counts: words.counts)
        XCTAssertEqual(tools.first?.tile, GlassToolTile.Kind.tool(SourceKind.claudeCode.glassTool))
        let source = try Self.text("Views/Monitor/SummaryInspector.swift")
        XCTAssertTrue(source.contains("HStack(alignment: .center, spacing: GlassTokens.Space.s5) {\n"
            + "                        if let tile = line.tile { GlassToolTile(tile) }"))
    }

    /// An inspector section's chevron works: every section opens and
    /// closes unless a caller says otherwise.
    func test_inspectorSectionsCollapse() throws {
        let host = try Self.text("Views/Monitor/TracesInspectorHost.swift")
        XCTAssertTrue(host.contains("init(_ title: String, collapsible: Bool = true, @ViewBuilder content: () -> Content)"))
    }

    /// Text in the review card runs further in than the boxes in it.
    func test_theReviewCardsTextIsInset() throws {
        let card = try Self.text("Views/Monitor/SessionReviewCard.swift")
        XCTAssertEqual(SessionReviewCard.textInset, GlassTokens.Space.s4)
        XCTAssertGreaterThanOrEqual(card.components(separatedBy: ".padding(.horizontal, Self.textInset)").count - 1, 2)
    }

    /// The popover's top row: Watching, Settings and Quit as icon buttons
    /// on the right; no pills. On the left were the traces worth a second
    /// look, until the NEAR AI wordmark took their place (owner,
    /// 2026-10-22, `DesignRulings20261022Tests`).
    func test_thePopoversTopRowHoldsTheIconButtons() throws {
        let panel = try Self.text("Views/Monitor/MenuBarGlassPanel.swift")
        let row = try XCTUnwrap(panel.range(of: "    private var topRow: some View {"))
        let body = String(panel[row.upperBound...].prefix(1200))
        let wordmark = try XCTUnwrap(body.range(of: "GlassNearWordmark()"))
        // Watching is the shared round dropdown (owner, 2026-10-22: the
        // edge rail's Traces peek has it too).
        let watching = try XCTUnwrap(body.range(of: "WatchingMenuButton()"))
        XCTAssertTrue(panel.contains("GlassRoundMenu(MonitorWords.watching, systemImage: paused ? \"eye.slash\" : \"eye\")"))
        let settings = try XCTUnwrap(body.range(of: "icon: .glyph(.gear)"))
        let quit = try XCTUnwrap(body.range(of: "GlassRoundButton(MenuWords.quit, systemImage: \"power\")"))
        XCTAssertLessThan(wordmark.lowerBound, watching.lowerBound)
        XCTAssertLessThan(watching.lowerBound, settings.lowerBound)
        XCTAssertLessThan(settings.lowerBound, quit.lowerBound)
        // Watching is a traditional dropdown (owner, 2026-10-19), not a
        // list that slides open in the panel.
        XCTAssertTrue(panel.contains("Section(MonitorWords.watching) {"))
        XCTAssertTrue(panel.contains("Button(MenuBarWords.resume) { model.resume() }"))
        XCTAssertTrue(panel.contains("Button(label) { model.pause(until: MenuBarWords.pauseUntil(index)) }"))
        for gone in ["GlassStatePill(", "private var pills", "privateAIOptions", "Button(MenuWords.settings)",
                     "Button(MenuWords.quit)", "GlassSubListHeader(", "GlassOptionRow(", "watchOpen"] {
            XCTAssertFalse(panel.contains(gone), "the popover draws \(gone) again")
        }
    }

    /// A new popover notice slides in over the last one and nothing else
    /// moves; a card revealed by a close fades in place.
    func test_aNewNoticeSlidesOverTheLast() throws {
        let source = try String(
            contentsOf: GlassSurfaceRulesTests.root.deletingLastPathComponent()
                .appendingPathComponent("TCDesign/Components/PopoverNotice.swift"),
            encoding: .utf8)
        XCTAssertTrue(source.contains("return .asymmetric(insertion: fresh ? slide : .opacity, removal: .opacity)"))
        XCTAssertTrue(source.contains(".transition(Self.arrival(anchor, fresh: !seen.contains(front.id)))"))
        // The plates are the stack's, outside the keyed front card.
        let keyed = try XCTUnwrap(source.range(of: ".id(front.id)"))
        let plates = try XCTUnwrap(source.range(of: "plate(level: CGFloat(index + 1))"))
        XCTAssertLessThan(keyed.lowerBound, plates.lowerBound)
    }

    /// The promo cards' large graphics are drawn at half strength, so they
    /// sit behind the words.
    func test_thePromoGraphicsAreFaded() throws {
        let source = try String(
            contentsOf: GlassSurfaceRulesTests.root.deletingLastPathComponent()
                .appendingPathComponent("TCDesign/Components/HeroCard.swift"),
            encoding: .utf8)
        XCTAssertTrue(source.contains("static var graphicOpacity: Double { 0.5 }"))
        XCTAssertTrue(source.contains(".opacity(Self.graphicOpacity)"))
    }

    /// Signed out, the Balance tile's "Sign in required" sits beside its
    /// dash; an expired sign-in has no extra notice beside recovery controls.
    func test_theSignedOutBalanceSitsBesideTheDash() throws {
        let model = AppModel()
        let copy = try XCTUnwrap(model.privateInferenceCopy)
        let signedOut = HomeFormat.balance(
            BalanceStatus.parse(fromJSON: #"{"state":"no_session"}"#), copy: copy, calls: model.balanceCalls)
        XCTAssertEqual(signedOut.value, "—")
        XCTAssertEqual(signedOut.beside, "Sign in required")
        XCTAssertNil(signedOut.detail)
        let expired = HomeFormat.balance(
            BalanceStatus.parse(fromJSON: #"{"state":"session_expired"}"#), copy: copy, calls: model.balanceCalls)
        XCTAssertEqual(expired.value, "—")
        XCTAssertNil(expired.beside)
        XCTAssertNil(expired.detail)
    }

    /// The tabs' own headings share one style (owner, 2026-10-19): the
    /// Traces list's title and the Private AI status both use
    /// `GlassTabHeading`, in the card-title face, at the content's edge.
    func test_theTabHeadingsShareOneStyle() throws {
        let traces = try Self.text("Views/Monitor/NudgeViews.swift")
        let privateAI = try Self.text("Views/Monitor/InferenceViews.swift")
        XCTAssertTrue(traces.contains("GlassTabHeading(store.words?.tree.treeTitle ?? \"\") {"))
        XCTAssertTrue(privateAI.contains("GlassTabHeading(copy.inspectorStatus) {"))
        XCTAssertFalse(privateAI.contains("InspectorFactRow"), "the status has its own row style again")
        XCTAssertEqual(GlassTabHeading<EmptyView>.titleType, GlassTokens.TypeScale.bodyStrong)
    }

    /// The estimate's info popover is in plain words.
    func test_theEstimateExplainerIsPlain() throws {
        let copy = try XCTUnwrap(NudgeCopy.decode(fromJSON: TCCoreCopy.nudgeCopyJSON()))
        let explainer = try XCTUnwrap(copy[.settlementEstimateExplainer])
        XCTAssertTrue(explainer.hasPrefix("This is a guess worked out on your Mac"))
        XCTAssertFalse(explainer.contains("makeup"))
    }
}
