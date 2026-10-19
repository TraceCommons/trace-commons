#if DEBUG
import XCTest
import TCBridge
import SwiftUI
import TCDesign
@testable import TCShellCore
@testable import TraceCommonsApp

/// Ron's inspector host (#1146, Task 3 of the #1241 port): the selection's
/// inspector on Home, Traces and History; Inference keeps its own. The
/// health banners and the prompts are popover notices over the tab they
/// belong to (owner, 2026-10-19, `MonitorNotices`). The inspector opens
/// itself when something it must show appears, and never closes itself.
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

    /// Review Focus 4, off the Traces tab: only Home's History page,
    /// restored from before History moved to Traces, still draws the
    /// health banners and the prompts in its inspector, above the
    /// selection's card and outside the choice of what the inspector shows.
    /// Traces (its tree and its History page) and Inference draw them as
    /// popover notices over the tab (owner, 2026-10-19), never here.
    func test_healthBannersStayVisibleWithASessionSelected() throws {
        XCTAssertFalse(MonitorWindowView.promptsInInspector(.traces), "Traces draws them over the tab")
        XCTAssertFalse(MonitorWindowView.promptsInInspector(.traces, tracesPage: .history),
                       "Traces' History draws them over the tab")
        XCTAssertTrue(MonitorWindowView.promptsInInspector(.home))
        XCTAssertFalse(MonitorWindowView.promptsInInspector(.inference), "Inference draws them over the tab")
        let host = try Self.inspectorColumn()
        let guarded = try XCTUnwrap(host.range(of: "if Self.promptsInInspector(shownTab, tracesPage: tracesPage) {"))
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

    /// Each tab draws its own notices (owner, 2026-10-19): Traces the
    /// undos, the core's health, the consent notices, the arming offer, the
    /// nudges and the first-contribution note; Private AI the offer to route
    /// AI requests through NEAR AI and the NEAR AI sign-in; every tab what
    /// is about the app or the account, and a failed action. Neither Home's
    /// page nor Inference's draws a prompt inline any more.
    func test_everyTabDrawsItsOwnNotices() throws {
        let host = try Self.inspectorColumn()
        XCTAssertTrue(host.contains("InspectorPromptsHeader(traces: traces)"), "Home's History skips the prompts")
        let inference = try Self.text("Views/Monitor/InferenceViews.swift")
        XCTAssertFalse(inference.contains("InspectorPrompts("), "Inference draws the prompts inline again")
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        XCTAssertFalse(home.contains("InspectorPromptsHeader("), "Home draws the prompts inline again")
        XCTAssertFalse(home.contains("NudgeGlassCard("), "History draws its verdict news inline again")

        let notices = try Self.text("Views/Monitor/MonitorNotices.swift")
        func member(_ opening: String) throws -> String {
            let start = try XCTUnwrap(notices.range(of: opening), opening)
            let end = notices.range(of: "\n    // MARK: ", range: start.upperBound..<notices.endIndex)?.lowerBound
                ?? notices.range(of: "\n    private ", range: start.upperBound..<notices.endIndex)?.lowerBound
                ?? notices.endIndex
            return String(notices[start.lowerBound..<end])
        }
        let traces = try member("    private var tracesOffers: [GlassPopoverItem] {")
            + member("    private var tracesNow: [GlassPopoverItem] {")
            + member("    private var consentItems: [GlassPopoverItem] {")
            + member("    private var undoItems: [GlassPopoverItem] {")
            + member("    private var approvalUndo: GlassPopoverItem? {")
        for needle in ["model.undo", "traces.lastContributed", "traces.lastContributedFolder", "traces.lastKept",
                       "model.armingOffer", "TracesHealth.banners(", "traces.nudgeCard", "offerItems(.traces)",
                       "model.status.grantVoids", "model.status.armingRewordings", "model.gateHeldNotice",
                       "model.witnessCopy?.onboarding", "traces.folderNotice"] {
            XCTAssertTrue(traces.contains(needle), "the Traces notices lack \(needle)")
        }
        let privateAI = try member("    private var privateAIItems: [GlassPopoverItem] {")
        for needle in ["model.showsPrivateInferenceOffer", "!snapshot.nearAiSignedIn", "model.managedText(\"sign_in_needed\")"] {
            XCTAssertTrue(privateAI.contains(needle), "the Private AI notices lack \(needle)")
        }
        XCTAssertFalse(traces.contains("model.showsPrivateInferenceOffer"), "Traces offers Private AI again")
        let everywhere = try member("    private var appItems: [GlassPopoverItem] {")
            + member("    private var actionItems: [GlassPopoverItem] {")
        for needle in ["model.isAttachedDaemon", "model.firstRunNotice", "model.legacyMigrationNotice",
                       "model.showsLegacyMigrationOfferCard", "model.lastActionNotice", "model.lastActionError",
                       "StartupRefusedBanner(sentence: sentence, popover: true)", "MonitorWords.signedOut"] {
            XCTAssertTrue(everywhere.contains(needle), "every tab's notices lack \(needle)")
        }
        let history = try member("    private var historyOffers: [GlassPopoverItem] {")
        XCTAssertTrue(history.contains("offerItems(.history)"))
        XCTAssertTrue(history.contains("home.verdictsCard"))
    }

    /// The notices float over the top of the tab, under the tab bar, and
    /// push nothing down (owner, 2026-10-19): the tree draws none of them,
    /// and starts at the tab's top. In the stack, what waits on an answer
    /// is behind what is happening now: the offers, then the consent
    /// notices and the core's health, then the undos, a failed action, and
    /// in front the traces moving now.
    func test_noticesFloatOverTheTabAndPushNothing() throws {
        let tree = try Self.text("Views/Monitor/TracesViews.swift")
        let treeView = try XCTUnwrap(tree.range(of: "struct TracesTreeView"))
        let slot = try XCTUnwrap(tree.range(of: "struct PreviewSlot"))
        let body = String(tree[treeView.lowerBound..<slot.lowerBound])
        for gone in ["TracesHealth.banners(", "InspectorPrompts(", "PromptsShelf", "NudgeGlassCard(",
                     "TraceTransferStack", "GlassPopoverStack"] {
            XCTAssertFalse(body.contains(gone), "the tree draws \(gone) again")
        }
        XCTAssertFalse(tree.contains("struct PromptsShelf"), "a shelf would push the tree down")

        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("""
                            content()
                                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                                .overlay(alignment: .top) { notices() }
                                .padding(.top, Self.contentTop)
        """), "the notices float over the tab's content, inside its insets")
        XCTAssertFalse(window.contains("ShellNotices()"), "the notices push the tabs down again")
        XCTAssertEqual(window.components(separatedBy: "InspectorPrompts(").count - 1, 0)
        XCTAssertEqual(window.components(separatedBy: "InspectorPromptsHeader(").count - 1, 1)

        let notices = try Self.text("Views/Monitor/MonitorNotices.swift")
        let items = try XCTUnwrap(notices.range(of: "    private var items: [GlassPopoverItem] {"))
        let itemsEnd = try XCTUnwrap(notices.range(of: "\n    }\n", range: items.upperBound..<notices.endIndex))
        let order = String(notices[items.upperBound..<itemsEnd.lowerBound])
        XCTAssertTrue(order.contains("items += tracesOffers + tracesNow"))
        XCTAssertTrue(order.contains("items += tracesOffers + historyOffers + tracesNow"))
        let actions = try XCTUnwrap(order.range(of: "items += actionItems"))
        let transfers = try XCTUnwrap(order.range(of: "TraceTransferCard.items("))
        XCTAssertLessThan(actions.lowerBound, transfers.lowerBound, "the traces moving now are in front")
        let now = try XCTUnwrap(notices.range(of: "    private var tracesNow: [GlassPopoverItem] {"))
        let nowBody = String(notices[now.upperBound...].prefix(2500))
        let health = try XCTUnwrap(nowBody.range(of: "TracesHealth.banners("))
        let undos = try XCTUnwrap(nowBody.range(of: "items += undoItems"))
        XCTAssertLessThan(health.lowerBound, undos.lowerBound, "an undo is behind the core's health")
        XCTAssertTrue(nowBody.contains("maxQueueEntries: model.daemonSettings?.maxQueueEntries"),
                      "the queue-full banner names the configured limit")
        // The core's health has no X: it goes when its cause does.
        let healthItem = String(nowBody[health.lowerBound...].prefix(700))
        XCTAssertTrue(healthItem.contains("GlassPopoverNotice(title: banner.title) {"))
    }

    /// A card closed with its X stays closed until the window opens again;
    /// every other card is drawn, in its order. Each tab's page picks its
    /// notices.
    func test_closedNoticesStayClosedAndEachPageHasItsPlace() {
        let items = ["a", "b", "c"].map { id in GlassPopoverItem(id: id) { EmptyView() } }
        XCTAssertEqual(MonitorNotices.shown(items, closed: ["b"]).map(\.id), ["a", "c"])
        XCTAssertEqual(MonitorNotices.shown(items, closed: []).map(\.id), ["a", "b", "c"])
        XCTAssertEqual(MonitorNotices.place(.traces, tracesPage: .tree), .traces)
        XCTAssertEqual(MonitorNotices.place(.traces, tracesPage: .history), .history)
        XCTAssertEqual(MonitorNotices.place(.inference, tracesPage: .tree), .privateAI)
        XCTAssertEqual(MonitorNotices.place(.home, tracesPage: .history), .home)
        XCTAssertEqual(MonitorNotices.place(.home, tracesPage: .tree, homePage: .history), .homeHistory)
        XCTAssertEqual(MonitorNotices.place(.insights, tracesPage: .tree), .insights)
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

    /// The inspector stays closed on Inference (owner, 2026-10-09): the
    /// Private AI switch heads the tab's main pane (owner, 2026-10-19), and
    /// the inspector column draws nothing of Inference's.
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
        XCTAssertEqual(views.components(separatedBy: "PrivateAIToggleRow(store: store)").count - 1, 1)
        XCTAssertFalse(views.contains("PrivateAIInspectorView(store: store)"), "the status row is drawn beside the switch")
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
            ".union(Self.promptsInInspector(shownTab, tracesPage: tracesPage) && inspectorAvailable\n"
                + "                ? InspectorDemand.offerKeys(model: model) : [])"))
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
