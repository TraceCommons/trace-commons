import SwiftUI
import TCBridge
import TCDesign
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The owner's 2026-10-22 menu-bar popover rulings: the NEAR AI wordmark in
/// the top row's left corner,
/// Private AI just under the top row with its name, On or Off and the tool
/// buttons, and the app's bar graph in place of the popover's own.
@MainActor
final class DesignRulings20261022Tests: XCTestCase {
    private static func panel() throws -> String {
        try GlassSurfaceRulesTests.text("Views/Monitor/MenuBarGlassPanel.swift")
    }

    /// The text of one of the panel's properties, up to the next `// MARK:`.
    private static func section(_ start: String, in panel: String) throws -> Substring {
        let open = try XCTUnwrap(panel.range(of: start), "the panel lacks \(start)")
        let end = panel.range(of: "    // MARK:", range: open.upperBound..<panel.endIndex)?.lowerBound ?? panel.endIndex
        return panel[open.upperBound..<end]
    }

    private static func snapshot(_ edit: (inout [String: Any]) -> Void = { _ in }) throws -> ManagedSnapshot {
        let repository = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
        let data = try Data(contentsOf: repository.appendingPathComponent("tests/fixtures/managed/snapshot.json"))
        var value = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        var capabilities = value["capabilities"] as? [String: Any] ?? [:]
        capabilities["near_ai_signin"] = true
        value["capabilities"] = capabilities
        edit(&value)
        return try JSONDecoder().decode(ManagedSnapshot.self, from: JSONSerialization.data(withJSONObject: value))
    }

    /// The top row, then Private AI, then the remaining sections in order.
    func test_privateAISitsUnderTheTopRow() throws {
        let panel = try Self.panel()
        let body = try XCTUnwrap(panel.range(of: "    var body: some View {"))
        let order = ["topRow", "privateAI", "nudgeRow", "settlementRow", "graph", "glance", "menuItems"]
        var last = body.upperBound
        for name in order {
            let line = try XCTUnwrap(
                panel.range(of: "            \(name)\n", range: last..<panel.endIndex), "\(name) is out of order")
            last = line.upperBound
        }
        XCTAssertFalse(panel.contains("    private var tools: some View {"), "the tools row is back on its own")
        XCTAssertFalse(panel.contains("            legend\n"), "the legend is drawn apart from the graph")
    }

    /// The wordmark leads the top row without a second-look category.
    func test_theWordmarkLeadsTheTopRow() throws {
        let panel = try Self.panel()
        let top = try Self.section("    private var topRow: some View {", in: panel)
        XCTAssertTrue(top.contains("GlassNearWordmark()\n                .frame(height: Self.wordmarkHeight)"))
        XCTAssertFalse(top.contains("MenuWords.flagged"), "the second look is back in the top row")
        XCTAssertFalse(panel.contains("flaggedRow"))
    }

    /// Private AI is drawn only while signed in to NEAR AI, under the core's
    /// name for it, with On or Off beside the name and the tool buttons
    /// under it.
    func test_privateAIIsDrawnOnlyWhileSignedIn() throws {
        let panel = try Self.panel()
        let section = try Self.section("    private var privateAI: some View {", in: panel)
        for needle in [
            "if let snapshot = model.managedSnapshot, snapshot.nearAiSignedIn,",
            "let name = model.privateInferenceCopy?.destination",
            "GlassEyebrowCard(name, accessory: {",
            "PrivateAIStatusLabel { open(.settings(.privateAI)) }",
            "if MenuPanelStatus.showsToolButtons(snapshot) {",
            "model.pendingManagedLaunch = tool",
            "open(.inference)",
        ] {
            XCTAssertTrue(section.contains(needle), "Private AI's section lacks \(needle)")
        }
        // On or Off, shared with the edge rail's Private AI peek.
        for needle in [
            "case .listener(let word, _): word",
            "case .off: MonitorWords.off",
            "on: model.daemonSettings?.privateInference,",
            "GlassStatusLabel(MenuPanelStatus.privateAIValue(state, model: model), status: MenuPanelStatus.privateAIDot(state))",
        ] {
            XCTAssertTrue(panel.contains(needle), "the popover's file lacks \(needle)")
        }
    }

    /// The dot is on only while Private AI is On; a switch on over a
    /// listener that is not working is outside, never on.
    func test_theDotIsOnOnlyWhilePrivateAIIsOn() {
        XCTAssertEqual(MenuPanelStatus.privateAIDot(.listener(word: "On", tone: .clear)), .on)
        XCTAssertEqual(MenuPanelStatus.privateAIDot(.listener(word: "Not working", tone: .refused)), .outside)
        XCTAssertEqual(MenuPanelStatus.privateAIDot(.off), .off)
        XCTAssertEqual(MenuPanelStatus.privateAIDot(.unknown), .off)
    }

    /// The tool buttons follow the Private AI tab's rule: drawn only while
    /// a tool can be started from a terminal.
    func test_theToolButtonsFollowThePrivateAITab() throws {
        let healthy = try Self.snapshot()
        XCTAssertTrue(healthy.nearAiSignedIn)
        XCTAssertFalse(healthy.signInStartTools.isEmpty)
        XCTAssertTrue(MenuPanelStatus.showsToolButtons(healthy))
        let noTerminal = try Self.snapshot { value in
            var capabilities = value["capabilities"] as? [String: Any] ?? [:]
            capabilities["terminal_launch"] = false
            value["capabilities"] = capabilities
        }
        XCTAssertEqual(ManagedSessionsSection.toolArea(noTerminal), .noTerminal)
        XCTAssertFalse(MenuPanelStatus.showsToolButtons(noTerminal))
    }

    /// The graph is the app's: its bar graph over the Traces graph's days,
    /// compact, at the edge rail peek's height, with the legend counting
    /// the graph's days or the day under the pointer.
    func test_theGraphIsTheAppsBarGraph() throws {
        let panel = try Self.panel()
        let graph = try Self.section("    private var graph: some View {", in: panel)
        XCTAssertTrue(graph.contains("GlassBarGraph(\n                MenuPanelData.bars(buckets, words: MonitorShellWords.graphWords),"))
        XCTAssertTrue(graph.contains("scaleFloor: 5, compact: true, hovered: $hovered)"))
        XCTAssertTrue(graph.contains(".frame(height: EdgeRailGraph.height)"))
        XCTAssertTrue(graph.contains("let shared = store.sharedCount.map { shown?.shared ?? $0 }"))
        XCTAssertFalse(panel.contains("GlassDayGraph("), "the popover's own graph is back")
        let store = try GlassSurfaceRulesTests.text("Views/Monitor/MenuPanelStore.swift")
        XCTAssertTrue(store.contains("TracesGraphModel.sharedTimes(history, tool: nil)"))
        XCTAssertTrue(store.contains("TracesGraphModel.keptTimes(pending, tool: nil)"))
        XCTAssertTrue(store.contains("offset: 0, now: clock(), range: .days)"))
    }

    // MARK: Insights, the Private AI tab and the Traces menus

    /// The Sessions figure's breakdown is always shown right under the
    /// Sessions card, with no chevron to open it below the harnesses; a
    /// week with no rows draws no empty card.
    func test_theSessionsBreakdownIsAlwaysShownUnderItsCard() throws {
        let overview = try GlassSurfaceRulesTests.text("Views/InsightsOverviewTab.swift")
        XCTAssertFalse(overview.contains("drillButton(\"sessions\")"), "the Sessions figure drills again")
        XCTAssertFalse(overview.contains("if model.drills(\"sessions\") { GlassChevron() }"))
        // Off Overview (product owner, 2026-10-28) except while a day is
        // picked on Tokens by day, which shows that day's sessions.
        XCTAssertTrue(overview.contains("header(overview)\n                    if pickedDay != nil { sessionsBreakdown(overview) }\n"))
        XCTAssertFalse(overview.contains("model.counterInputs"))
        let breakdown = try Self.section("    private func sessionsBreakdown(_ overview: InsightsWeekOverview) -> some View {",
                                         in: overview)
        XCTAssertTrue(breakdown.contains("if !sessions.rows.isEmpty {"))
        XCTAssertTrue(breakdown.contains("let inputs = model.sessionsInputs, !inputs.sessions.isEmpty"))
    }

    /// A Traces menu row lights as every menu row does: the menu's accent
    /// with its hover ink, never the control hover's dark well.
    func test_theTracesMenusHighlightAsMenusDo() throws {
        let traces = try GlassSurfaceRulesTests.text("Views/Monitor/TracesViews.swift")
        XCTAssertFalse(traces.contains(".glassHover(GlassTokens.Color.controlHover"))
        XCTAssertEqual(traces.components(separatedBy: ".glassHover(TracesMenuHighlight.fill, in: Capsule())").count - 1, 2)
        XCTAssertEqual(TracesMenuHighlight.fill, GlassTokens.Color.menuHover)
        XCTAssertEqual(TracesMenuHighlight.ink(lit: true), GlassTokens.Color.menuHoverText)
        XCTAssertEqual(TracesMenuHighlight.ink(lit: false), GlassTokens.Color.textPrimary)
    }

    // MARK: The edge rail

    private static func rail() throws -> String {
        try GlassSurfaceRulesTests.text("Views/EdgeRail/EdgeRailView.swift")
    }

    /// Home's peek is gone; Private AI comes first, with its shield, then
    /// Traces with four nodes in a T. The app's tile is the blue N mark.
    func test_theRailHasPrivateAIAndTraces() throws {
        XCTAssertEqual(EdgeRailPeek.allCases, [.privateAI, .traces])
        XCTAssertEqual(EdgeRailState().peek, .privateAI)
        XCTAssertEqual(EdgeRailPeek.privateAI.icon, .symbol("shield"))
        XCTAssertEqual(EdgeRailPeek.traces.icon, .traceNodes)
        let rail = try Self.rail()
        XCTAssertTrue(rail.contains("GlassNearMark()\n                    .frame(width: Self.markSide, height: Self.markSide)"))
        XCTAssertFalse(rail.contains("NSApp.applicationIconImage"), "the app icon is back on the rail")
        XCTAssertTrue(rail.contains("GlassTraceNodesGlyph(ink:"))
        XCTAssertFalse(rail.contains("HomeStatusLine("), "Home's peek is back")
    }

    /// Each peek's header: Private AI's On or Off right after its title,
    /// and the popover's buttons in the top right corner; Settings opens
    /// Private AI's section from its peek.
    func test_thePeeksHeaderHasThePopoversButtons() throws {
        let header = try Self.section("    private var header: some View {", in: try Self.rail())
        let title = try XCTUnwrap(header.range(of: "Text(peek.name(nav: nav, shell: shell))"))
        let status = try XCTUnwrap(header.range(of: "PrivateAIStatusLabel()"))
        let spacer = try XCTUnwrap(header.range(of: "Spacer(minLength: GlassTokens.Space.s4)"))
        let watching = try XCTUnwrap(header.range(of: "WatchingMenuButton()"))
        let settings = try XCTUnwrap(header.range(of: "icon: .glyph(.gear)"))
        let quit = try XCTUnwrap(header.range(of: "GlassRoundButton(MenuWords.quit, systemImage: \"power\") { NSApp.terminate(nil) }"))
        XCTAssertLessThan(title.lowerBound, status.lowerBound)
        XCTAssertLessThan(status.lowerBound, spacer.lowerBound)
        XCTAssertLessThan(spacer.lowerBound, watching.lowerBound)
        XCTAssertLessThan(watching.lowerBound, settings.lowerBound)
        XCTAssertLessThan(settings.lowerBound, quit.lowerBound)
        XCTAssertTrue(header.contains("if peek == .traces {\n                WatchingMenuButton()"))
        XCTAssertTrue(header.contains("open(.settings(EdgeRailSettings.section(peek)))"))
    }

    /// The Private AI peek: the tool buttons while signed in, Credit and
    /// Balance under them, and a small Private AI map at the foot, drawn by
    /// the window's own map code.
    func test_thePrivateAIPeekHasToolsAccountAndAMap() throws {
        let rail = try Self.rail()
        let peek = try Self.section("    private var privateAI: some View {", in: rail)
        let tools = try XCTUnwrap(peek.range(of: "MenuPanelStatus.showsToolButtons(snapshot)"))
        let account = try XCTUnwrap(peek.range(of: "HomeAccountTiles(store: data.home)"))
        let map = try XCTUnwrap(peek.range(of: "EdgeRailPrivateAIMap(data: data, label: label)"))
        XCTAssertLessThan(tools.lowerBound, account.lowerBound)
        XCTAssertLessThan(account.lowerBound, map.lowerBound)
        XCTAssertTrue(peek.contains("snapshot.nearAiSignedIn"))
        XCTAssertFalse(peek.contains("PrivateAIBalanceCard("), "the old balance card is back")
        XCTAssertTrue(rail.contains("PrivateAIFlowMap(\n                traces: data.traces, inference: data.inference"))
        let window = try GlassSurfaceRulesTests.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("PrivateAIFlowMap(\n                    traces: traces, inference: inference, label: privateAILabel"),
                      "the window's map and the rail's are one view")
        let home = try GlassSurfaceRulesTests.text("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains("HomeAccountTiles(store: store, openPrivateAI: openPrivateAI)"),
                      "Home's Account tiles and the rail's are one view")
    }

    /// The Traces peek: no card of what is where; every watched directory
    /// in a list that scrolls past seven rows, each with Submit and its
    /// "..." menu.
    func test_theTracesPeekListsEveryFolderWithItsActions() throws {
        let rail = try Self.rail()
        for gone in ["placeRow(", "rail?.onThisMac", "rail?.inTheLibrary", "foldersShown"] {
            XCTAssertFalse(rail.contains(gone), "the Traces peek draws \(gone) again")
        }
        XCTAssertTrue(rail.contains("EdgeRailFolderList(store: data.traces, open: open)"))
        XCTAssertTrue(rail.contains("GlassSplitButton(title, menuLabel: outcome?.submitAllAs ?? \"\""))
        XCTAssertTrue(rail.contains("TracesFolderMenu("))
        XCTAssertTrue(rail.contains("TracesSubmitMenu("))
        XCTAssertTrue(rail.contains("ScrollView {"))
        XCTAssertEqual(EdgeRailFolderListHeights.height(rows: 3), 3 * EdgeRailFolderListHeights.rowPitch)
        XCTAssertEqual(EdgeRailFolderListHeights.height(rows: 30), EdgeRailFolderListHeights.maxHeight)
        XCTAssertEqual(EdgeRailFolderListHeights.maxHeight, 7 * EdgeRailFolderListHeights.rowPitch)
        XCTAssertEqual(EdgeRailFolderListHeights.rows([], expanded: []), 0)
    }

    /// A folder's chevron opens its sessions under it (owner, 2026-10-22),
    /// as the tree's does; a session row opens it in the Traces tab, and
    /// the list grows by the open folders' sessions.
    func test_theTracesPeeksFoldersOpenToTheirSessions() throws {
        let rail = try Self.rail()
        for needle in [
            "expanded: folder.sessions.isEmpty ? nil : expanded.contains(folder.id),",
            "onToggleExpand: { toggle(folder.id) },",
            "ForEach(Self.drawnRows(folders, expanded: expanded))",
            "case .session(let entry, let off): sessionRow(entry, off: off, words: words)",
            "depth: .session, tile: TracesTreeView.sessionTile(entry), title: TracesTreeView.when(entry),",
            "onSelect: { open(.traces(entryId: entry.entryId)) },",
            "EdgeRailFolderListHeights.rows(folders, expanded: expanded)",
        ] {
            XCTAssertTrue(rail.contains(needle), "the rail lacks \(needle)")
        }
    }

    /// Opening a folder inserts every one of its sessions immediately
    /// after it. Closing removes only those rows; the remaining row ids
    /// stay stable for the lazy stack and its menu anchors.
    func test_theTracesPeekExpansionPreservesRowsAndTheirIdentity() async throws {
        let sessions = try await SampleDaemonClient(.normalDay).listPending(projectId: nil)
        let first = try XCTUnwrap(sessions.first)
        let second = try XCTUnwrap(sessions.dropFirst().first)
        let folder = TracesTree.FolderNode(id: "one", label: "one", mode: .ask, sessions: [first, second])
        let next = TracesTree.FolderNode(id: "two", label: "two", mode: .ask, sessions: [])
        let folders = [folder, next]
        let closed = EdgeRailFolderList.drawnRows(folders, expanded: [])
        XCTAssertEqual(closed, [.folder(folder), .folder(next)])

        let opened = EdgeRailFolderList.drawnRows(folders, expanded: [folder.id])
        XCTAssertEqual(opened, [.folder(folder), .session(first, off: false), .session(second, off: false), .folder(next)])
        XCTAssertEqual(opened.first?.id, closed.first?.id)
        XCTAssertEqual(opened.last?.id, closed.last?.id)
        XCTAssertEqual(Set(opened.map(\.id)).count, opened.count)
        XCTAssertEqual(EdgeRailFolderListHeights.rows(folders, expanded: [folder.id]), opened.count)
        XCTAssertEqual(EdgeRailFolderList.drawnRows(folders, expanded: ["no-longer-present"]), closed)
        XCTAssertEqual(EdgeRailFolderList.drawnRows(folders, expanded: []), closed)
        XCTAssertEqual(EdgeRailFolderList.drawnRows(folders, expanded: [folder.id]).map(\.id), opened.map(\.id))
    }

    /// No link or line under a peek (owner, 2026-10-22): the Open link and
    /// "Nothing is sent from the rail" are gone.
    func test_aPeekHasNoFooter() throws {
        let rail = try Self.rail()
        for gone in ["rail?.nothingSent", "rail?.open(section:", "private var link", "\n            link\n"] {
            XCTAssertFalse(rail.contains(gone), "the peek draws \(gone) again")
        }
    }

    // MARK: Submit

    /// Every Contribute button reads Submit (owner, 2026-10-22), and a
    /// trace's pill is Submit, with its own tip, in the tree and the rail.
    func test_aTracesPillIsSubmit() throws {
        let words = try XCTUnwrap(MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON()))
        XCTAssertEqual(words.contribute, "Submit")
        XCTAssertEqual(words.tree.submit, "Submit")
        XCTAssertFalse(words.tree.submitTraceTip.isEmpty)
        XCTAssertNotEqual(words.tree.submitTraceTip, words.tree.submitTip, "a trace's tip is not the folder's")
        let tree = try GlassSurfaceRulesTests.text("Views/Monitor/TracesViews.swift")
        XCTAssertTrue(tree.contains("pill: words?.tree.submit,"))
        XCTAssertTrue(tree.contains("help: words?.tree.submitTraceTip ?? \"\","))
        XCTAssertFalse(tree.contains("words?.tree.reviewing : words?.review"), "the Review pill is back")
        let rail = try Self.rail()
        XCTAssertTrue(rail.contains("submitTitle: words.tree.submit,"))
        XCTAssertTrue(rail.contains("onSubmit: store.maySubmitSession(entry, folderIgnored: off)"))
        XCTAssertFalse(rail.contains("submitTitle: words.review"))
    }

    /// A row's Submit sends only what the review card's Submit could, short
    /// of the preview: never without a client, never from an ignored
    /// folder, never a trace held as worth a second look.
    func test_aRowsSubmitIsGated() async throws {
        let empty = TracesStore(client: nil)
        let store = TracesStore(client: nil)
        store.attach(SampleDaemonClient(.normalDay))
        await store.load()
        let sessions = store.tree.allSessions
        XCTAssertFalse(sessions.isEmpty)
        for entry in sessions {
            XCTAssertFalse(empty.maySubmitSession(entry, folderIgnored: false), "no client")
            XCTAssertFalse(store.maySubmitSession(entry, folderIgnored: true), "an ignored folder")
            if entry.heldForReview {
                XCTAssertFalse(store.maySubmitSession(entry, folderIgnored: false), "held for a second look")
            }
            if store.status?.loggedIn != true {
                XCTAssertFalse(store.maySubmitSession(entry, folderIgnored: false), "no enrollment")
            }
        }
    }

    /// Settings from a peek: Private AI's own section, General from Traces.
    func test_aPeeksSettingsOpensItsSection() {
        XCTAssertEqual(EdgeRailSettings.section(.privateAI), .privateAI)
        XCTAssertEqual(EdgeRailSettings.section(.traces), .general)
    }
}
