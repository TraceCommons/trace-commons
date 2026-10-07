import TCBridge
import TCDesign
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Task 5 of the #1146 port: Ron's Folder inspector. There is no Tool
/// inspector (owner, 2026-10-05): the tree has no tool level.
@MainActor
final class ToolFolderInspectorTests: XCTestCase {
    private func loaded() async throws -> (TracesStore, TracesTree.FolderNode) {
        let store = TracesStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        let folder = try XCTUnwrap(store.tree.folders.first { !$0.sessions.isEmpty && $0.mode != nil })
        return (store, folder)
    }

    /// Review Focus 3: a folder whose rule is Never shows that rule and no
    /// Submit all. Selecting it resolves to its inspector, whose Decisions
    /// section is drawn only behind the same gate.
    func test_aNeverFolderOffersNoSubmitAll() async throws {
        let (store, folder) = try await loaded()
        XCTAssertTrue(FolderInspector.offersSubmitAll(folder, store: store), "the sample folder offers Submit all")
        XCTAssertEqual(store.selectedFolder(.folder(projectID: folder.id))?.id, folder.id)
        XCTAssertNil(store.selectedFolder(.session(entryID: folder.id)))
        XCTAssertNil(store.selectedFolder(.folder(projectID: "no-such-folder")))

        let never = TracesTree.FolderNode(
            id: folder.id, label: folder.label, mode: .ignore, offerableModes: folder.offerableModes,
            sessions: folder.sessions, pendingCount: folder.pendingCount, contributableCount: folder.contributableCount)
        XCTAssertFalse(FolderInspector.offersSubmitAll(never, store: store))
        // The rule is shown, in the core's word for Never.
        let labels = try XCTUnwrap(TracesStore.disclosureCopy).folderModeLabels
        XCTAssertEqual(FolderInspector.ruleLabel(.ignore), labels["ignore"])
        XCTAssertEqual(FolderInspector.ruleLabel(.ask), labels["notify_only"])
        XCTAssertEqual(FolderInspector.ruleLabel(.autoUpload), labels["auto_upload"])

        // An ignored project from the core is still in the tree and resolves.
        let ignored = ProjectRow(projectId: "p1", projectLabel: "quiet", projectPath: "/x", mode: .ignore)
        let tree = TracesTree.build(
            entries: [], projects: [ignored], settings: nil, scansWhenUnset: [], showsIgnored: true)
        XCTAssertEqual(tree.folders.first?.mode, .ignore)
        // The bucket's empty path is a dash, never a blank value.
        let words = try XCTUnwrap(store.words)
        let bucket = TracesTree.FolderNode(id: "b", label: "b", mode: .ask, sessions: [], path: "")
        XCTAssertEqual(FolderInspector.rows(bucket, words: words).first?.value, "—")

        let source = try TracesParityTests.text("Views/Monitor/ToolFolderInspectors.swift")
        XCTAssertTrue(source.contains("let submits = Self.offersSubmitAll(folder, store: store)"))
        XCTAssertTrue(source.contains("if submits {"))
        XCTAssertTrue(source.contains("if submits, let outcome = store.disclosure?.outcome {"))
        XCTAssertTrue(source.contains("words.inspector.contributionRule"))
        XCTAssertTrue(source.contains("words.inspector.noRule"))
        XCTAssertFalse(source.contains("struct ToolInspector"), "no tool inspector: the tree has no tool level")
        let host = try TracesParityTests.text("Views/Monitor/TracesInspectorHost.swift")
        XCTAssertTrue(host.contains("traces.selectedFolder(selection)"))
        XCTAssertTrue(host.contains("FolderInspector(store: traces, folder: folder, history: history)"))
    }

    /// P15 and V3 of the #1146 delta: Ron's header, the shared/kept legend,
    /// three sections with bare content, and his Decisions card, whose
    /// Submit reads "Submit all eligible (n)" on one line.
    func test_theFolderInspectorIsRonsShape() async throws {
        let (store, folder) = try await loaded()
        let words = try XCTUnwrap(store.words)
        let offer = store.groupOffer(folder)
        let submit = FolderInspector.submitAllTitle(offer.count, words: words)
        XCTAssertEqual(submit, FirstRunCopy.fill(words.inspector.submitAllEligible, ["count": String(offer.count)]))
        XCTAssertTrue(submit.contains(String(offer.count)), submit)
        XCTAssertFalse(submit.contains("{"), submit)
        XCTAssertEqual(FolderInspector.waitingLine(1, words: words), words.inspector.waitingSessionsOne)
        XCTAssertEqual(FolderInspector.waitingLine(3, words: words),
                       FirstRunCopy.fill(words.inspector.waitingSessions, ["count": "3"]))

        // The eligible line is drawn only for a folder with an eligibility
        // question, and carries the core's withheld line after it.
        let asked = TracesTree.FolderNode(
            id: "p", label: "api", mode: .ask, sessions: folder.sessions, pendingCount: 3, contributableCount: 2)
        let line = try XCTUnwrap(FolderInspector.eligibleLine(
            asked, offer: GroupSubmitOffer(count: 2, offersContribute: true, withheldLine: "W"), words: words))
        XCTAssertEqual(line, FirstRunCopy.fill(words.tree.eligibleCount, ["count": "2"]) + " · W")
        XCTAssertEqual(
            FolderInspector.eligibleLine(asked, offer: GroupSubmitOffer(count: 2, offersContribute: true, withheldLine: nil),
                                         words: words),
            FirstRunCopy.fill(words.tree.eligibleCount, ["count": "2"]))
        let unasked = TracesTree.FolderNode(id: "p", label: "api", mode: .ask, sessions: folder.sessions)
        XCTAssertNil(FolderInspector.eligibleLine(
            unasked, offer: GroupSubmitOffer(count: 1, offersContribute: true, withheldLine: nil), words: words))

        // Shared counts this project's standing contributions, and is a
        // dash while history is unread: never zero for unknown.
        XCTAssertEqual(FolderInspector.shared(folder, history: nil), "—")
        XCTAssertEqual(FolderInspector.shared(folder, history: []), "0")

        let source = try TracesParityTests.text("Views/Monitor/ToolFolderInspectors.swift")
        for needle in ["InspectorHeader(tile: .folder, title: folder.label, sub: Self.headerSub(folder, words: words))",
                       "GlassLegendCell(table.shared, value: Self.shared(folder, history: history), status: .shared)",
                       "GlassLegendCell(table.kept, value: String(folder.sessions.count), status: .kept)",
                       "InspectorSection(words.inspector.project)", "InspectorSection(words.inspector.contributionRule)",
                       "InspectorSection(words.inspector.decisions)", "GlassCard(quiet: true)"] {
            XCTAssertTrue(source.contains(needle), "ToolFolderInspectors.swift lacks \(needle)")
        }
        XCTAssertFalse(source.contains("GlassEyebrowCard("), "the folder inspector still draws eyebrow cards")
        XCTAssertFalse(source.contains("TracesTreeView.submitTitle("), "the inspector's Submit is the tree pill's")
    }

    /// Automatic always goes through the arming confirmation, in the core's
    /// words; Never asks first when sessions are waiting; Ask me is a
    /// direct write. The inspector's picker takes the same route as the
    /// tree's switch and menu, and writes only through it.
    func test_automaticGoesThroughTheArmingConfirmation() throws {
        let folder = TracesTree.FolderNode(
            id: "p1", label: "api", mode: .ask, offerableModes: [.ask, .autoUpload, .ignore],
            sessions: [try recordedEntry()])
        guard case .confirm(let arming) = TracesTreeView.modeChange(folder, .autoUpload),
            case .arm(let copy) = arming.words
        else {
            return XCTFail("Automatic must ask first")
        }
        let core = try XCTUnwrap(ProjectArmingCopy.decode(fromJSON: TCCoreCopy.armingOfferCopyJSON(project: "api", count: 0)))
        XCTAssertEqual(copy.question, core.question)
        XCTAssertEqual(arming.mode, .autoUpload)

        guard case .confirm(let ignoring) = TracesTreeView.modeChange(folder, .ignore), case .ignore = ignoring.words else {
            return XCTFail("Never with sessions waiting asks first")
        }
        let empty = TracesTree.FolderNode(id: "p2", label: "docs", mode: .ask, offerableModes: [.ask, .ignore], sessions: [])
        guard case .apply = TracesTreeView.modeChange(empty, .ignore) else { return XCTFail("nothing to clear: direct") }
        let armed = TracesTree.FolderNode(id: "p3", label: "web", mode: .autoUpload, sessions: [])
        guard case .apply = TracesTreeView.modeChange(armed, .ask) else { return XCTFail("Ask me is direct") }
        guard case .noop = TracesTreeView.modeChange(folder, .ask) else { return XCTFail("the same mode is no change") }

        let source = try TracesParityTests.text("Views/Monitor/ToolFolderInspectors.swift")
        XCTAssertTrue(source.contains("TracesTreeView.modeChange(folder, wanted)"))
        XCTAssertTrue(source.contains("TracesTreeView.modeChange(folder, .ignore)"))
        XCTAssertEqual(source.components(separatedBy: "store.setFolderMode(").count - 1, 1, "one write, after the route")
        // Ron's whole-window confirmation: arming is the default action,
        // declining is the cancel.
        XCTAssertTrue(source.contains("GlassModalAction(copy.confirm, isDefault: true) { apply(pending.mode) }"))
        XCTAssertTrue(source.contains(".cancel(copy.decline, action: cancel)"))
    }

    /// Submit all as is a modal: each of its three outcomes sends that
    /// verdict for the folder, through `approveFolder`, and nothing else.
    func test_submitAllAsSendsTheChosenVerdict() async throws {
        let transport = RecordingTransport(.normalDay)
        let store = TracesStore(client: LiveDaemonClient(transport: transport))
        await store.load()
        XCTAssertEqual(store.phase, .loaded)
        let folder = try XCTUnwrap(store.tree.folders.first { store.mayContributeFolder($0) && !$0.sessions.isEmpty })
        for verdict in ContributorVerdict.allCases {
            transport.reset()
            await FolderInspector.submitAllAs(verdict, folder: folder, store: store)
            let approves = transport.calls.filter { $0.method == "approve" }
            XCTAssertEqual(approves.count, 1, "\(verdict)")
            let params = try XCTUnwrap(approves.first?.params)
            XCTAssertTrue(params.contains(#""outcome":"\#(verdict.rawValue)""#), params)
            XCTAssertTrue(params.contains(#""project_id":"\#(folder.id)""#), params)
            XCTAssertNotNil(store.lastContributedFolder)
        }

        let source = try TracesParityTests.text("Views/Monitor/ToolFolderInspectors.swift")
        let sheet = try XCTUnwrap(source.range(of: "title: outcome.submitAllAs, subtitle: outcome.submitAllAsTooltip"))
        let body = source[sheet.lowerBound...]
        for verdict in ["worked", "partly", "failed"] {
            XCTAssertTrue(body.contains("Self.submitAllAs(.\(verdict), folder: folder, store: store)"), verdict)
        }
        XCTAssertTrue(source.contains("await store.contributeFolder(folder, verdict: verdict)"))
        XCTAssertTrue(body.contains("outcome.worked"))
        XCTAssertTrue(body.contains("outcome.partly"))
        XCTAssertTrue(body.contains("outcome.failed"))
        XCTAssertTrue(body.contains("words.inspector.cancel"))
        // The count line is the core's, filled.
        let words = try XCTUnwrap(store.words)
        XCTAssertEqual(FolderInspector.applyLine(1, words: words), words.inspector.applyOutcomeOne)
        let many = FolderInspector.applyLine(3, words: words)
        XCTAssertTrue(many.contains("3"), many)
        XCTAssertFalse(many.contains("{"), many)
    }

    /// The disclosure bundle is decoded in one place, `TracesStore`: the
    /// folder inspector, the tree's folder rows and History read that one
    /// copy, so the three can never hold different words.
    func test_theDisclosureBundleIsDecodedOnce() async throws {
        let walker = try XCTUnwrap(FileManager.default.enumerator(
            at: ReleaseBuildGateTests.root, includingPropertiesForKeys: nil))
        var decoders: [String] = []
        for case let url as URL in walker where url.pathExtension == "swift" {
            let text = try String(contentsOf: url, encoding: .utf8)
            if text.contains("contributorDisclosureCopyJSON()") { decoders.append(url.lastPathComponent) }
        }
        XCTAssertEqual(decoders, ["TracesStore.swift"])
        let (store, _) = try await loaded()
        XCTAssertEqual(store.disclosure, TracesStore.disclosureCopy)
        let folders = try TracesParityTests.text("Views/Monitor/ToolFolderInspectors.swift")
        XCTAssertTrue(folders.contains("store.disclosure?.outcome"))
        let tree = try TracesParityTests.text("Views/Monitor/TracesViews.swift")
        XCTAssertTrue(tree.contains("modeLabels: store.disclosure?.folderModeLabels"))
        let home = try TracesParityTests.text("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains("HistoryList.labels(shell: MonitorWords.table?.shell)"))
    }
}

/// The sample day over the live client, recording every call. A folder
/// `approve` answers as the daemon does for a folder.
private final class RecordingTransport: DaemonTransport, @unchecked Sendable {
    let set: SampleDaemonClient.SampleSet
    private let lock = NSLock()
    private var recorded: [(method: String, params: String)] = []

    init(_ set: SampleDaemonClient.SampleSet) {
        self.set = set
    }

    var calls: [(method: String, params: String)] {
        lock.lock()
        defer { lock.unlock() }
        return recorded
    }

    func reset() {
        lock.lock()
        recorded = []
        lock.unlock()
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        lock.lock()
        recorded.append((method, paramsJSON))
        lock.unlock()
        if method == "approve", paramsJSON.contains("\"project_id\"") {
            return #"{"id":0,"result":{"approved":2,"hold_secs":30,"hold_until":null,"flagged":0,"redactions":{},"skipped":[],"excluded_held":0,"excluded_ineligible":0}}"#
        }
        return SampleDaemonData.reply(method, in: set).map { #"{"id":0,"result":\#($0)}"# }
            ?? #"{"id":0,"error":{"code":"bad_params","message":"unknown-method"}}"#
    }
}
