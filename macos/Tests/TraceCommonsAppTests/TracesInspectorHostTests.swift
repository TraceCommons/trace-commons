#if DEBUG
import XCTest
import TCBridge
import TCDesign
@testable import TCShellCore
@testable import TraceCommonsApp

/// Ron's inspector host (#1146, Task 3 of the #1241 port): the health
/// banners, then the prompts, then the selection's inspector, on Home,
/// Traces and History; Inference keeps its own. The inspector opens itself
/// when something it must show appears, and never closes itself.
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

    /// Review Focus 4: the health banners are drawn above whatever the
    /// selection shows, outside the switch on it, so selecting a session
    /// never hides a held queue or a core that is down.
    func test_healthBannersStayVisibleWithASessionSelected() throws {
        let body = try Self.hostBody()
        let banners = try XCTUnwrap(body.range(of: "TracesHealth.banners("))
        let prompts = try XCTUnwrap(body.range(of: "InspectorPrompts(store: traces)"))
        let selection = try XCTUnwrap(body.range(of: "switch shown {"))
        XCTAssertLessThan(banners.lowerBound, prompts.lowerBound, "the banners come first")
        XCTAssertLessThan(prompts.lowerBound, selection.lowerBound, "the prompts precede the selection's inspector")
        XCTAssertEqual(body.components(separatedBy: "TracesHealth.banners(").count - 1, 1)
        XCTAssertTrue(body.contains("GlassHealthBanner(banner: $0)"))
        XCTAssertTrue(body.contains("maxQueueEntries: model.daemonSettings?.maxQueueEntries"),
                      "the queue-full banner names the configured limit")
        XCTAssertTrue(body.contains("coreDown: TracesHealth.coreDownLine"))
        // History's detail is drawn in History's left pane (Task 8 of the
        // #1146 port): the host has no History arm and never draws it.
        let hostSource = try Self.text("Views/Monitor/TracesInspectorHost.swift")
        XCTAssertFalse(hostSource.contains("HistoryDetailInspector("), "the host draws History's detail")
        XCTAssertFalse(hostSource.contains("historyRow"), "the host is handed a History row")
        XCTAssertFalse(hostSource.contains("case history"))
        // The session arm is reached only through the resolved selection,
        // so a session that has gone is the Summary, not a stale card.
        let host = try Self.text("Views/Monitor/TracesInspectorHost.swift")
        XCTAssertTrue(host.contains("traces.selectedSession(selection)"))
        XCTAssertFalse(host.contains("case .session(let entryID)"), "never an entry picked by the raw selection")
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

    /// The window's inspector pane, from `} inspector: {` to the window
    /// modifiers, and its arms split at each `case`.
    static func inspectorArms() throws -> (pane: String, arms: [String]) {
        let window = try text("Views/MonitorWindowView.swift")
        let inspector = try XCTUnwrap(window.range(of: "} inspector: {"))
        let end = try XCTUnwrap(window.range(of: ".glassWindow()", range: inspector.upperBound..<window.endIndex))
        let pane = String(window[inspector.upperBound..<end.lowerBound])
        let switchStart = try XCTUnwrap(pane.range(of: "switch tab {"))
        let arms = pane[switchStart.upperBound...].components(separatedBy: "\n                case ").dropFirst()
        return (pane, Array(arms))
    }

    /// Inference keeps `PrivateAIInspectorView`, under the prompts, as
    /// Ron's shell mounts `WaitingPrompts` above `InferenceInspector`; Home,
    /// Traces and History get the host.
    func test_inferenceKeepsItsOwnInspector() throws {
        let (pane, arms) = try Self.inspectorArms()
        let inference = try XCTUnwrap(arms.first { $0.hasPrefix(".inference:") })
        let prompts = try XCTUnwrap(inference.range(of: "InspectorPrompts(store: traces)"))
        let own = try XCTUnwrap(inference.range(of: "PrivateAIInspectorView(store: inference"))
        XCTAssertLessThan(prompts.lowerBound, own.lowerBound, "the prompts sit above Inference's own inspector")
        XCTAssertFalse(inference.contains("TracesHealth.banners("), "the health banners are the Traces inspector's")
        XCTAssertFalse(inference.contains("TracesInspectorHost("))
        XCTAssertFalse(pane.contains("SessionReviewCard("), "the session card is the host's to draw")
        XCTAssertFalse(pane.contains("HomeSummaryInspector("), "Home's summary is the host's to draw")
        XCTAssertEqual(pane.components(separatedBy: "PrivateAIInspectorView(").count - 1, 1)
    }

    /// Every arm of the window's inspector switch draws the prompts, and
    /// every arm but Inference draws them through the host, under the
    /// health banners. No arm draws a detail that skips them: a History row
    /// is the host's to show.
    func test_everyInspectorArmDrawsThePrompts() throws {
        let (pane, arms) = try Self.inspectorArms()
        XCTAssertEqual(arms.count, 2, "one arm for Inference, one host for the rest")
        for arm in arms {
            if arm.hasPrefix(".inference:") {
                XCTAssertTrue(arm.contains("InspectorPrompts(store: traces)"))
            } else {
                XCTAssertTrue(arm.contains("TracesInspectorHost("), "an arm skips the host: \(arm)")
            }
        }
        XCTAssertFalse(pane.contains("HistoryDetailInspector("), "History's detail is drawn in History's left pane")
        let flat = pane.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        // History included: the inspector keeps the Traces selection's card
        // (Task 8 of the #1146 port).
        XCTAssertTrue(flat.contains("TracesInspectorHost(traces: traces, home: home, selection: selection)"))
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
    /// folder's Submit all in flight, the arming offer and the Private AI
    /// offer each open the inspector when they appear.
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
        XCTAssertTrue(arming.contains("offer:arming:p1"))
        XCTAssertTrue(InspectorDemand.opens(previous: none, current: arming))

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
        XCTAssertTrue(offered.contains("offer:private-ai"))
        XCTAssertTrue(InspectorDemand.opens(previous: before, current: offered))
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

    /// The offers bar above the tree is gone: the tree pane holds the tree
    /// and its loading and error lines, and the prompts live in the
    /// inspector, which a new one opens.
    func test_theOffersBarIsGone() throws {
        for rel in ["Views/Monitor/TracesViews.swift", "Views/Monitor/TracesOffers.swift",
                    "Views/MonitorWindowView.swift", "Views/Monitor/TracesInspectorHost.swift",
                    "Views/Monitor/InspectorPrompts.swift"] {
            XCTAssertFalse(try Self.text(rel).contains("TracesOffersBar"), "\(rel) still names the offers bar")
        }
        let tree = try Self.text("Views/Monitor/TracesViews.swift")
        let treeView = try XCTUnwrap(tree.range(of: "struct TracesTreeView"))
        let inspector = try XCTUnwrap(tree.range(of: "struct PreviewSlot"))
        let body = tree[treeView.lowerBound..<inspector.lowerBound]
        for gone in ["TracesHealth.banners(", "GlassHealthBanner(", "model.undo", "store.lastContributed",
                     "model.armingOffer", "CertificateSection("] {
            XCTAssertFalse(body.contains(gone), "the tree pane still draws \(gone)")
        }
        // A core that does not answer is still said beside the tree.
        XCTAssertTrue(body.contains("if case .failed(let error) = store.phase, let line = store.words?.line(for: error)"))

        let prompts = try Self.text("Views/Monitor/InspectorPrompts.swift")
        for needle in ["model.undo", "store.lastContributed", "store.lastContributedFolder", "store.lastKept",
                       "model.armingOffer", "model.showsPrivateInferenceOffer", "model.lastActionError",
                       "model.lastActionNotice", "FirstContributionGlassNote("] {
            XCTAssertTrue(prompts.contains(needle), "InspectorPrompts.swift lacks \(needle)")
        }
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
