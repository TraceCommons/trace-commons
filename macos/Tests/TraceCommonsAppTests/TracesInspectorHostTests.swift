#if DEBUG
import XCTest
import TCBridge
import SwiftUI
import TCDesign
@testable import TCShellCore
@testable import TraceCommonsApp

/// Ron's inspector host (#1146, Task 3 of the #1241 port): the selection's
/// inspector on Home, Traces and History; Inference keeps its own. The
/// health banners and the prompts are drawn above the Traces tree (owner,
/// 2026-10-07). The inspector opens itself when something it must show
/// appears, and never closes itself.
@MainActor
final class TracesInspectorHostTests: XCTestCase {
    static func text(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    /// The host's body, from its `var body` to the end of the struct.
    static func hostBody() throws -> String {
        let host = try text("Views/Monitor/TracesInspectorHost.swift")
        let start = try XCTUnwrap(host.range(of: "struct TracesInspectorHost: View"))
        let body = try XCTUnwrap(host.range(of: "    var body: some View {", range: start.upperBound..<host.endIndex))
        let end = try XCTUnwrap(host.range(of: "\n}\n", range: body.upperBound..<host.endIndex))
        return String(host[body.lowerBound..<end.lowerBound])
    }

    /// Review Focus 4, with offers, undo and health above the tree (owner,
    /// 2026-10-07): the host draws only the selection's inspector, so
    /// selecting a session can never hide a held queue, a core that is down,
    /// an undo or an offer, which sit above the tree on Traces and above the
    /// host elsewhere (`InspectorPromptsHeader`).
    func test_theHostDrawsOnlyTheSelection() throws {
        let body = try Self.hostBody()
        for gone in ["TracesHealth.banners(", "GlassHealthBanner(", "InspectorPrompts("] {
            XCTAssertFalse(body.contains(gone), "the inspector host still draws \(gone)")
        }
        XCTAssertTrue(body.contains("switch shown {"))
        // An opened History row is drawn by the window's own inspector arm
        // (`HistoryInspectorPane`, V8): the host has no History arm.
        let hostSource = try Self.text("Views/Monitor/TracesInspectorHost.swift")
        XCTAssertFalse(hostSource.contains("HistoryDetailInspector("), "the host draws History's detail")
        XCTAssertFalse(hostSource.contains("historyRow"), "the host is handed a History row")
        XCTAssertFalse(hostSource.contains("case history"))
        // The session arm is reached only through the resolved selection,
        // so a session that has gone is the Summary, not a stale card.
        XCTAssertTrue(hostSource.contains("traces.selectedSession(selection)"))
        XCTAssertFalse(hostSource.contains("case .session(let entryID)"), "never an entry picked by the raw selection")
    }

    /// Review Focus 4, off the Traces tab: Home and History still offer a
    /// session's Contribute and Keep (the kept Traces selection's card), so
    /// the health banners and the prompts are drawn above it there, outside
    /// the choice of what the inspector shows. Selecting a session on
    /// Traces and switching to Home never hides an undo, an offer or a core
    /// that is down. On Traces they are above the tree, not repeated here.
    func test_healthBannersStayVisibleWithASessionSelected() throws {
        XCTAssertFalse(MonitorWindowView.promptsInInspector(.traces), "Traces draws them above the tree")
        XCTAssertTrue(MonitorWindowView.promptsInInspector(.home))
        XCTAssertFalse(MonitorWindowView.promptsInInspector(.inference), "Inference draws them atop its page")
        let host = try Self.inspectorColumn()
        let guarded = try XCTUnwrap(host.range(of: "if Self.promptsInInspector(shownTab) {"))
        let header = try XCTUnwrap(host.range(of: "InspectorPromptsHeader(traces: traces)"))
        let history = try XCTUnwrap(host.range(of: "HistoryInspectorPane(row: row)"))
        let selection = try XCTUnwrap(host.range(of: "TracesInspectorHost(traces: traces, home: home, selection: selection)"))
        XCTAssertLessThan(guarded.lowerBound, header.lowerBound, "the banners are drawn on Traces twice")
        XCTAssertLessThan(header.lowerBound, history.lowerBound, "an opened History row hides the prompts")
        XCTAssertLessThan(header.lowerBound, selection.lowerBound, "the selection's card hides the prompts")
        XCTAssertEqual(host.components(separatedBy: "InspectorPromptsHeader(").count - 1, 1)

        // The header: the banners, then the prompts.
        let source = try Self.text("Views/Monitor/TracesInspectorHost.swift")
        let start = try XCTUnwrap(source.range(of: "struct InspectorPromptsHeader: View"))
        let end = try XCTUnwrap(source.range(of: "\n}\n", range: start.upperBound..<source.endIndex))
        let body = String(source[start.upperBound..<end.lowerBound])
        let banners = try XCTUnwrap(body.range(of: "TracesHealth.banners("))
        let prompts = try XCTUnwrap(body.range(of: "InspectorPrompts(store: traces)"))
        XCTAssertLessThan(banners.lowerBound, prompts.lowerBound, "the banners come first")
        XCTAssertTrue(body.contains("GlassHealthBanner(banner: $0)"))
        XCTAssertTrue(body.contains("coreDown: TracesHealth.coreDownLine"))
        XCTAssertTrue(body.contains("maxQueueEntries: model.daemonSettings?.maxQueueEntries"),
                      "the queue-full banner names the configured limit")
    }

    /// Every arm of the window's inspector switch draws the prompts where
    /// it is not beside the Traces tree: Inference above its own inspector
    /// (Ron's shell mounts `WaitingPrompts` there), Home and History above
    /// the host. So any tab that offers Contribute also offers its Undo,
    /// and the Private AI offer is on Inference.
    func test_everyPageWithoutTheTreeDrawsThePrompts() throws {
        let host = try Self.inspectorColumn()
        XCTAssertTrue(host.contains("InspectorPromptsHeader(traces: traces)"), "History skips the prompts")
        // Home's page and Inference's page, where the inspector stays
        // closed, draw them at their top.
        let inference = try Self.text("Views/Monitor/InferenceViews.swift")
        XCTAssertEqual(inference.components(separatedBy: "InspectorPrompts(store: traces)").count - 1, 1,
                       "Inference skips the prompts")
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        XCTAssertEqual(home.components(separatedBy: "InspectorPromptsHeader(traces: traces)").count - 1, 1,
                       "Home skips the prompts")
    }

    /// V5 and V6 of the #1146 delta: the undos, the offers and the
    /// first-contribution note are drawn above the Traces tree, under the
    /// health banners, and in the inspector only off the Traces tab. R-LAYOUT-1: with a tree, they
    /// sit on a shelf above it that is capped at a share of the pane and
    /// scrolls inside itself, so however tall they are the tree keeps rows
    /// on screen and nothing is pushed out of the window; no branch draws
    /// them outside a scroll.
    func test_offersUndoAndHealthSitAboveTheTree() throws {
        let tree = try Self.text("Views/Monitor/TracesViews.swift")
        let treeView = try XCTUnwrap(tree.range(of: "struct TracesTreeView"))
        let slot = try XCTUnwrap(tree.range(of: "struct PreviewSlot"))
        let body = String(tree[treeView.lowerBound..<slot.lowerBound])
        let banners = try XCTUnwrap(body.range(of: "TracesHealth.banners("))
        let prompts = try XCTUnwrap(body.range(of: "InspectorPrompts(store: store)"))
        XCTAssertLessThan(banners.lowerBound, prompts.lowerBound, "the banners come first")
        XCTAssertEqual(body.components(separatedBy: "InspectorPrompts(").count - 1, 1)
        // With a tree, the prompts sit on the capped shelf, before the tree.
        let shelf = try XCTUnwrap(body.range(of: "PromptsShelf(cap: Self.promptsCap(paneHeight: pane.size.height)) {\n"))
        let afterShelf = try XCTUnwrap(body.range(of: "prompts\n", range: shelf.upperBound..<body.endIndex))
        let treeUse = try XCTUnwrap(body.range(of: "tree\n", range: afterShelf.upperBound..<body.endIndex))
        XCTAssertLessThan(afterShelf.lowerBound, treeUse.lowerBound, "the prompts precede the tree")
        let scroll = try XCTUnwrap(body.range(of: "ScrollViewReader { proxy in"))
        let folders = try XCTUnwrap(body.range(of: "ForEach(store.tree.folders)", range: scroll.upperBound..<body.endIndex))
        XCTAssertNil(body.range(of: "prompts\n", range: scroll.upperBound..<folders.lowerBound),
                     "the prompts are inside the tree's scroll again, where they can fill it")
        // Every use of the prompts in the view body is inside a ScrollView.
        let viewBody = try XCTUnwrap(body.range(of: "var body: some View {"))
        let bodyEnd = try XCTUnwrap(body.range(of: "/// A mode change's confirmation"))
        let drawn = String(body[viewBody.upperBound..<bodyEnd.lowerBound])
        let uses = drawn.components(separatedBy: "\n").filter { $0.trimmingCharacters(in: .whitespaces) == "prompts" }
        XCTAssertEqual(uses.count, 3, "the loading, empty and tree branches each draw the prompts")
        XCTAssertEqual(drawn.components(separatedBy: "ScrollView {").count - 1, 3,
                       "the loading and empty branches scroll the prompts, and the tree scrolls")
        let shelfType = try XCTUnwrap(tree.range(of: "struct PromptsShelf"))
        XCTAssertTrue(tree[shelfType.lowerBound...].contains("ScrollView {"), "the shelf scrolls its prompts")
        XCTAssertFalse(body.contains("            InspectorPrompts(store: store)\n            if store.phase"),
                       "the prompts are drawn above the tree's scroll again")
        XCTAssertTrue(body.contains("maxQueueEntries: model.daemonSettings?.maxQueueEntries"),
                      "the queue-full banner names the configured limit")
        // The inspector draws them only where the tree is not shown: once,
        // under the guard that limits them to Home (History).
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertEqual(window.components(separatedBy: "InspectorPrompts(").count - 1, 0)
        XCTAssertEqual(window.components(separatedBy: "InspectorPromptsHeader(").count - 1, 1)
    }

    /// R-LAYOUT-1, round 2: the prompts shelf never takes more than its
    /// share of the pane, so the tree always keeps rows on screen, and an
    /// empty shelf takes no room.
    func test_promptsShelfLeavesTheTreeRows() {
        let pane: CGFloat = 600
        let cap = TracesTreeView.promptsCap(paneHeight: pane)
        XCTAssertEqual(cap, pane * TracesTreeView.promptsShare, accuracy: 0.001)
        XCTAssertLessThanOrEqual(TracesTreeView.promptsShare, 0.5, "the tree keeps at least half the pane")
        // A tall offer plus the first-contribution note is clamped to the cap.
        XCTAssertEqual(PromptsShelf<EmptyView>.shelfHeight(content: 900, cap: cap), cap)
        XCTAssertGreaterThanOrEqual(pane - PromptsShelf<EmptyView>.shelfHeight(content: 900, cap: cap), 300)
        // A short notice is drawn at its own height, and nothing takes none.
        XCTAssertEqual(PromptsShelf<EmptyView>.shelfHeight(content: 40, cap: cap), 40)
        XCTAssertEqual(PromptsShelf<EmptyView>.shelfHeight(content: 0, cap: cap), 0)
        XCTAssertEqual(TracesTreeView.promptsCap(paneHeight: -10), 0)
    }

    /// An undo appearing is a new demand, and a new demand opens the
    /// inspector (Ron's `useInspectorDemand`).
    func test_aNewUndoOpensTheInspector() throws {
        let model = AppModel()
        let traces = TracesStore(client: nil)
        let before = InspectorDemand.keys(model: model, traces: traces, selection: nil)
        XCTAssertFalse(before.contains { $0.hasPrefix("undo:") })
        model.undo = AppModel.Undo(
            entryIDs: ["e1"], toastLine: "x", offerUndo: true, approvedAt: Date(timeIntervalSince1970: 100),
            heldSeconds: 0)
        let after = InspectorDemand.keys(model: model, traces: traces, selection: nil)
        XCTAssertTrue(after.contains { $0.hasPrefix("undo:") })
        XCTAssertTrue(InspectorDemand.opens(previous: before, current: after))

        // A second approval is a new undo even for the same session.
        let first = after
        model.undo = AppModel.Undo(
            entryIDs: ["e1"], toastLine: "x", offerUndo: true, approvedAt: Date(timeIntervalSince1970: 200),
            heldSeconds: 0)
        XCTAssertTrue(InspectorDemand.opens(
            previous: first, current: InspectorDemand.keys(model: model, traces: traces, selection: nil)))

        // The ticking count is not a new demand.
        let beforeTick = InspectorDemand.keys(model: model, traces: traces, selection: nil)
        model.undo?.heldSeconds = 7
        let ticked = InspectorDemand.keys(model: model, traces: traces, selection: nil)
        XCTAssertEqual(beforeTick, ticked)
        XCTAssertFalse(InspectorDemand.opens(previous: beforeTick, current: ticked))

        // The window watches the keys and opens on a new one.
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("InspectorDemand.keys(model: model, traces: traces, selection: selection)"))
        // Ron's shell keeps the last keys it saw (`lastDemand`).
        XCTAssertTrue(window.contains(
            "if InspectorDemand.opens(previous: lastDemand, current: current) { showsInspector = true }"))
    }

    /// A demand going away, or staying, never closes the inspector: only
    /// the person's toggle does.
    func test_theInspectorNeverClosesItself() throws {
        XCTAssertFalse(InspectorDemand.opens(previous: ["undo:x"], current: []))
        XCTAssertFalse(InspectorDemand.opens(previous: ["undo:x", "arming:p"], current: ["undo:x", "arming:p"]))
        XCTAssertFalse(InspectorDemand.opens(previous: ["undo:x", "arming:p"], current: ["arming:p"]))
        XCTAssertTrue(InspectorDemand.opens(previous: ["undo:x"], current: ["arming:p"]))

        let model = AppModel()
        let traces = TracesStore(client: nil)
        // Only a session selection is a demand; a folder is not.
        XCTAssertTrue(InspectorDemand.keys(model: model, traces: traces, selection: .folder(projectID: "p")).isEmpty)

        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertFalse(window.contains("showsInspector = false"), "nothing in the window closes the inspector")
        let host = try Self.text("Views/Monitor/TracesInspectorHost.swift")
        XCTAssertFalse(host.contains("showsInspector"), "the host never touches the pane")
    }

    /// The window's inspector column (`inspectorContent`, which the
    /// inspector pane scrolls).
    static func inspectorColumn() throws -> String {
        let window = try text("Views/MonitorWindowView.swift")
        let inspector = try XCTUnwrap(window.range(of: "private var inspectorContent: some View {"))
        let end = try XCTUnwrap(window.range(
            of: "/// Whether the inspector draws the prompts", range: inspector.upperBound..<window.endIndex))
        return String(window[inspector.upperBound..<end.lowerBound])
    }

    /// The inspector stays closed on Inference (owner, 2026-10-09): its
    /// Private AI summary heads the tab's main pane, and the inspector
    /// column draws nothing of Inference's.
    func test_inferenceHasNoInspector() throws {
        XCTAssertFalse(MonitorWindowView.inspectorAvailable(.inference, homePage: .overview))
        let pane = try Self.inspectorColumn()
        XCTAssertTrue(pane.contains("if !model.onboardingKnown || !inspectorAvailable {"))
        XCTAssertFalse(pane.contains("PrivateAIInspectorView("), "the inspector draws Inference's summary again")
        XCTAssertFalse(pane.contains("SessionReviewCard("), "the session card is the host's to draw")
        XCTAssertFalse(pane.contains("HomeSummaryInspector("), "Home's summary is the host's to draw")
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertFalse(window.contains("PrivateAIInspectorView("))
        let views = try Self.text("Views/Monitor/InferenceViews.swift")
        XCTAssertEqual(views.components(separatedBy: "PrivateAIInspectorView(store: store)").count - 1, 1)
    }

    /// The column draws the host; an opened History row shows in
    /// `HistoryInspectorPane` only on History, else the host.
    func test_theInspectorDrawsTheHost() throws {
        let pane = try Self.inspectorColumn()
        XCTAssertTrue(pane.contains("TracesInspectorHost("), "the column skips the host")
        XCTAssertFalse(pane.contains("HistoryDetailInspector("), "History's detail is drawn in History's left pane")
        let flat = pane.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        // History included: the inspector keeps the Traces selection's card
        // (Task 8 of the #1146 port).
        XCTAssertTrue(flat.contains("TracesInspectorHost(traces: traces, home: home, selection: selection)"))
        // Ron's inspector inset: 16 across and 18 down (L5), on the column
        // the pane scrolls.
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("inspectorContent\n"))
        XCTAssertTrue(window.contains(".padding(GlassPaneInsets.inspector)"))
    }

    /// A demand already there when the window opens opens the inspector,
    /// as Ron's effect does on mount (`lastDemand` starts empty): an undo
    /// made while the Monitor was closed is not left out of sight by a
    /// restored or a narrow-window closed inspector.
    func test_aDemandPresentAtMountOpensTheInspector() throws {
        XCTAssertFalse(InspectorDemand.opensOnAppear(keys: []))
        XCTAssertTrue(InspectorDemand.opensOnAppear(keys: ["undo:approval:e1@100.0"]))
        XCTAssertEqual(InspectorDemand.opensOnAppear(keys: ["arming:p"]),
                       InspectorDemand.opens(previous: [], current: ["arming:p"]))

        let window = try Self.text("Views/MonitorWindowView.swift")
        let flat = window.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        // A restored closed inspector: the window appears with a demand.
        XCTAssertTrue(flat.contains(
            ".onAppear { // `onChange` sees changes only:"))
        XCTAssertTrue(flat.contains(
            "if InspectorDemand.opensOnAppear(keys: demandKeys) { showsInspector = true } lastDemand = demandKeys"))
        // A narrow first layout seeds the inspector closed, unless a demand is there.
        XCTAssertTrue(flat.contains(
            "showsInspector = seed.showsInspector || InspectorDemand.opensOnAppear(keys: demandKeys)"))
    }

    /// Every one of Ron's demand keys is a demand: a selected session, a
    /// folder's Submit all in flight, and, wherever the inspector draws the
    /// prompts (every tab but Traces, where they are above the tree), the
    /// arming offer and the Private AI offer.
    func test_everyDemandKeyOpensTheInspector() async throws {
        let model = AppModel()
        // A resolved session selection.
        let traces = TracesStore(client: SampleDaemonClient(.normalDay))
        await traces.load()
        let session = try XCTUnwrap(traces.tree.allSessions.first)
        let none = InspectorDemand.keys(model: model, traces: traces, selection: nil)
        let reviewing = InspectorDemand.keys(model: model, traces: traces, selection: .session(entryID: session.entryId))
        XCTAssertTrue(reviewing.contains("review:\(session.entryId)"))
        XCTAssertTrue(InspectorDemand.opens(previous: none, current: reviewing))
        // A session that has gone is no demand.
        XCTAssertFalse(InspectorDemand.keys(model: model, traces: traces, selection: .session(entryID: "gone"))
            .contains { $0.hasPrefix("review:") })

        // The arming offer.
        model.setArmingOfferForTesting(ArmingOffer(projectId: "p1", projectLabel: "api", contributedCount: 3))
        let arming = InspectorDemand.keys(model: model, traces: traces, selection: nil)
        XCTAssertFalse(arming.contains { $0.hasPrefix("offer:") }, "an offer beside the tree opens the inspector")
        let armingOffer = InspectorDemand.offerKeys(model: model)
        XCTAssertTrue(armingOffer.contains("offer:arming:p1"))
        XCTAssertTrue(InspectorDemand.opens(previous: none, current: arming.union(armingOffer)))

        // The Private AI offer, while it is unanswered and off.
        let offerModel = AppModel()
        let before = InspectorDemand.keys(model: offerModel, traces: traces, selection: nil)
        let frame = #"{"id":1,"result":{"quiescence_secs":45,"digest_interval_secs":3600,"#
            + #""local_notifications":true,"queue_ttl_days":14,"max_queue_entries":500,"#
            + #""max_uploads_per_day":100,"private_inference":false,"private_inference_offer_seen":false,"#
            + #""near_ai_configured":false,"claude_root_configured":true,"codex_root_configured":true}}"#
        offerModel.setDaemonSettingsForTesting(try DaemonClient(daemon: FrameDaemon(frame)).settings())
        XCTAssertTrue(offerModel.showsPrivateInferenceOffer)
        let offered = InspectorDemand.keys(model: offerModel, traces: traces, selection: nil)
            .union(InspectorDemand.offerKeys(model: offerModel))
        XCTAssertTrue(offered.contains("offer:private-ai"))
        XCTAssertTrue(InspectorDemand.opens(previous: before, current: offered))

        // The window adds the offers only where the inspector draws them
        // and may open (not on Home's page, where they head the main pane).
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains(
            ".union(Self.promptsInInspector(shownTab) && inspectorAvailable ? InspectorDemand.offerKeys(model: model) : [])"))
        XCTAssertTrue(MonitorWindowView.inspectorAvailable(.home, homePage: .history))
        XCTAssertFalse(MonitorWindowView.inspectorAvailable(.home, homePage: .overview))
        XCTAssertTrue(MonitorWindowView.inspectorAvailable(.traces, homePage: .overview))
        XCTAssertFalse(MonitorWindowView.inspectorAvailable(.inference, homePage: .overview))
    }

    /// A folder's Submit all is a demand while it is in flight, and not
    /// before or after it.
    func test_aFolderSubmitInFlightOpensTheInspector() async throws {
        let entered = expectation(description: "the folder approve reached the core")
        let transport = ApproveGate(entered: entered)
        let traces = TracesStore(client: LiveDaemonClient(transport: transport))
        await traces.load()
        let folder = try XCTUnwrap(traces.tree.allFolders.first { traces.mayContributeFolder($0) && !$0.sessions.isEmpty })
        let model = AppModel()
        let idle = InspectorDemand.keys(model: model, traces: traces, selection: nil)
        XCTAssertFalse(idle.contains { $0.hasPrefix("folder-submit:") })
        let submit = Task { await traces.contributeFolder(folder, verdict: nil) }
        await fulfillment(of: [entered], timeout: 2)
        let busy = InspectorDemand.keys(model: model, traces: traces, selection: nil)
        XCTAssertTrue(busy.contains("folder-submit:\(folder.id)"))
        XCTAssertTrue(InspectorDemand.opens(previous: idle, current: busy))
        transport.open()
        await submit.value
        XCTAssertFalse(InspectorDemand.keys(model: model, traces: traces, selection: nil)
            .contains { $0.hasPrefix("folder-submit:") })
    }

    /// The contribution's and the folder's undo cards carry the core's
    /// Dismiss, as Ron's `UndoBar` does, and it clears them; "APPROVAL
    /// SAVED" heads every undo card.
    func test_storeUndosCanBeDismissed() async throws {
        let traces = TracesStore(client: SampleDaemonClient(.normalDay))
        await traces.load()
        let session = try XCTUnwrap(traces.tree.allSessions.first { !$0.heldForReview })
        await traces.perform(.contribute, on: session.entryId)
        XCTAssertNotNil(traces.lastContributed)
        traces.dismissContributed()
        XCTAssertNil(traces.lastContributed)

        let folder = try XCTUnwrap(traces.tree.allFolders.first { traces.mayContributeFolder($0) && !$0.sessions.isEmpty })
        await traces.contributeFolder(folder, verdict: nil)
        XCTAssertNotNil(traces.lastContributedFolder)
        traces.dismissContributedFolder()
        XCTAssertNil(traces.lastContributedFolder)

        let prompts = try Self.text("Views/Monitor/InspectorPrompts.swift")
        for needle in ["Button(dismissWord) { store.dismissContributed() }",
                       "Button(dismissWord) { store.dismissContributedFolder() }"] {
            XCTAssertTrue(prompts.contains(needle), "InspectorPrompts.swift lacks \(needle)")
        }
        XCTAssertFalse(prompts.contains("offerUndo ? store.words?.undo.approvalSaved : nil"))
        XCTAssertFalse(prompts.contains("offerUndo ? words.undo.approvalSaved : nil"))
        XCTAssertEqual(prompts.components(separatedBy: "eyebrow: words.undo.approvalSaved").count - 1, 2)
        XCTAssertTrue(prompts.contains("eyebrow: store.words?.undo.approvalSaved"))
    }

    /// The old offers bar's name is gone: the prompts above the tree are
    /// `InspectorPrompts`, one view, wherever they are drawn. Its pieces
    /// are all there.
    func test_thePromptsHoldEveryUndoAndOffer() throws {
        for rel in ["Views/Monitor/TracesViews.swift", "Views/Monitor/TracesOffers.swift",
                    "Views/MonitorWindowView.swift", "Views/Monitor/TracesInspectorHost.swift",
                    "Views/Monitor/InspectorPrompts.swift"] {
            XCTAssertFalse(try Self.text(rel).contains("TracesOffersBar"), "\(rel) still names the offers bar")
        }
        let tree = try Self.text("Views/Monitor/TracesViews.swift")
        let treeView = try XCTUnwrap(tree.range(of: "struct TracesTreeView"))
        let slot = try XCTUnwrap(tree.range(of: "struct PreviewSlot"))
        let body = tree[treeView.lowerBound..<slot.lowerBound]
        // The tree draws the prompts through the one view, not its pieces.
        for piece in ["model.undo", "store.lastContributed", "model.armingOffer", "CertificateSection("] {
            XCTAssertFalse(body.contains(piece), "the tree pane draws \(piece) itself")
        }

        let prompts = try Self.text("Views/Monitor/InspectorPrompts.swift")
        for needle in ["model.undo", "store.lastContributed", "store.lastContributedFolder", "store.lastKept",
                       "model.armingOffer", "model.showsPrivateInferenceOffer", "model.lastActionError",
                       "model.lastActionNotice", "FirstContributionGlassNote("] {
            XCTAssertTrue(prompts.contains(needle), "InspectorPrompts.swift lacks \(needle)")
        }
        // Ron's `UndoBar`: Undo a glass button, Dismiss a link, beside the
        // words rather than under them. The fourth link is a failed
        // action's Dismiss, after its unboxed line (Ron, 2026-10-09).
        XCTAssertEqual(prompts.components(separatedBy: ".buttonStyle(GlassButtonStyle(.link))").count - 1, 4)
        XCTAssertFalse(prompts.contains("GlassButtonStyle(.primary)"), "Undo is a glass button, as Ron's")
        XCTAssertTrue(prompts.contains("HStack(alignment: .center, spacing: GlassTokens.Space.s8) {"))
        for rel in ["Views/Monitor/TracesInspectorHost.swift", "Views/Monitor/InspectorPrompts.swift"] {
            try LegacySymbols.assertClean(rel)
            XCTAssertTrue(GlassSurfaceRulesTests.files.contains(rel), "\(rel) is not under the glass rules")
        }
    }
}

/// Answers every call with one frame.
private final class FrameDaemon: DaemonCalling {
    let frame: String
    init(_ frame: String) { self.frame = frame }
    func call(_ method: String, params paramsJSON: String) -> String { frame }
    func openPreview(entryID: String) throws -> TCPreview { throw DaemonDataError.unreachable }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
}

/// The sample day over the live client, with a folder's `approve` held (on
/// the client's work queue, not the main actor) until `open()`.
private final class ApproveGate: DaemonTransport, @unchecked Sendable {
    private let entered: XCTestExpectation
    private let gate = DispatchSemaphore(value: 0)

    init(entered: XCTestExpectation) { self.entered = entered }

    func open() { gate.signal() }

    func call(_ method: String, params paramsJSON: String) -> String {
        if method == "approve", paramsJSON.contains("project_id") {
            entered.fulfill()
            gate.wait()
        }
        return SampleDaemonData.reply(method, in: .normalDay).map { #"{"id":0,"result":\#($0)}"# }
            ?? #"{"id":0,"error":{"code":"bad_params","message":"unknown-method"}}"#
    }
}
#endif
