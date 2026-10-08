import TCBridge
import TCDesign
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Fix round 1 of the #1146 glass parity (#1268): the behaviour the round
/// changed on the Traces surface, pinned against the core's words.
@MainActor
final class GlassParityRoundOneTests: XCTestCase {
    private func loaded() async throws -> (TracesStore, TracesTree.FolderNode) {
        let store = TracesStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        let folder = try XCTUnwrap(store.tree.folders.first { !$0.sessions.isEmpty && $0.mode != nil })
        return (store, folder)
    }

    /// P11: a folder selection focuses the map on the tool it is drawn
    /// under, as a session selection does on its own tool.
    func test_aFolderSelectionHasATool() async throws {
        let (store, folder) = try await loaded()
        let tool = MonitorWindowView.selectedTool(store.tree, session: nil, folder: folder)
        XCTAssertNotNil(tool, "the binoculars stay dim for a folder")
        XCTAssertEqual(tool, store.tree.tools.first { $0.folders.contains { $0.id == folder.id } }?.kind)
        let session = try XCTUnwrap(folder.sessions.first)
        XCTAssertEqual(
            MonitorWindowView.selectedTool(store.tree, session: session, folder: nil),
            SourceKind(rawValue: session.declaredSource ?? session.source) ?? SourceKind(rawValue: session.source))
        XCTAssertNil(MonitorWindowView.selectedTool(store.tree, session: nil, folder: nil))
    }

    /// The store's undo cards say #1146's "{label} approved" for the
    /// folder, and fall back to the core's toast when nothing was approved
    /// or no folder is named.
    func test_theUndoTitleNamesTheFolder() throws {
        let words = try XCTUnwrap(MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON()))
        let toast = SubmitToast(line: "Approved.", offerUndo: true)
        XCTAssertEqual(
            InspectorPrompts.title("api", toast: toast, words: words),
            words.undo.approved.replacingOccurrences(of: "{label}", with: "api"))
        XCTAssertEqual(InspectorPrompts.title(nil, toast: toast, words: words), "Approved.")
        XCTAssertEqual(InspectorPrompts.title("", toast: toast, words: words), "Approved.")
        let nothing = SubmitToast(line: "Nothing approved.", offerUndo: false)
        XCTAssertEqual(InspectorPrompts.title("api", toast: nothing, words: words), "Nothing approved.")
        XCTAssertFalse(words.undo.mayHaveStarted.isEmpty)
    }

    /// A Contribute keeps the session's folder name for its undo card.
    func test_aContributionKeepsItsFolderName() async throws {
        let (store, folder) = try await loaded()
        let session = try XCTUnwrap(folder.sessions.first)
        await store.perform(.contribute, on: session.entryId)
        guard let contributed = store.lastContributed else { return }
        XCTAssertEqual(contributed.label, session.projectLabel)
    }

    /// The safeguards' routing cell says #1146's short label for the
    /// state, from the core; an unknown state is Unknown.
    func test_theRoutingCellIsTheShortLabel() throws {
        let copy = try XCTUnwrap(MonitorWords.table?.safeguards)
        XCTAssertEqual(copy.routingLabel("rows_seen"), copy.routingRowsSeen)
        XCTAssertEqual(copy.routingLabel("awaiting_rows"), copy.routingAwaitingRows)
        XCTAssertEqual(copy.routingLabel("not_declared"), copy.routingNotDeclared)
        XCTAssertEqual(copy.routingLabel("token_unreadable"), copy.routingTokenUnreadable)
        XCTAssertEqual(copy.routingLabel("something_new"), copy.routingUnknown)
        XCTAssertEqual(copy.routingLabel(nil), copy.routingUnknown)
        XCTAssertEqual(copy.routingRowsSeen, "Receiving Private AI records")
    }

    /// The Traces legend and the credential node take the core's words.
    func test_theMapWordsAreTheCores() throws {
        let words = try XCTUnwrap(FlowMapScene.words)
        XCTAssertEqual(words.legendNotWatched, "Tool not watched")
        XCTAssertEqual(words.credential, "NEAR AI credential")
        XCTAssertEqual(words.noneFound, "No configured tools found.")
        let empty = FlowMapScene.privateAI(
            HarnessList.none, destinationLabel: "D", privateAI: "off",
            sentence: { _ in nil }, state: { _ in .unknown })
        XCTAssertEqual(empty.nodes.first?.sublabel, words.noneFound)
    }

    /// INF-MAP-1: the Inference tab shows the map's Private AI view, the
    /// Traces tab its Traces view, and Home keeps the choice (#1146).
    func test_aTabPicksItsMapView() {
        XCTAssertEqual(MonitorWindowView.mapTab(for: .inference), .privateAI)
        XCTAssertEqual(MonitorWindowView.mapTab(for: .traces), .traces)
        XCTAssertNil(MonitorWindowView.mapTab(for: .home))
    }

    /// The Inference inspector reads the same tool list as the Local tools
    /// card and the map; a list not read is nil, never no tools.
    func test_theInferenceInspectorReadsTheOneToolList() throws {
        XCTAssertNil(PrivateAIInspectorView.rows(.none))
        // The Inference store's read is consulted only for whether the list
        // is current (`liveHarnesses`), never for its rows.
        let source = try TracesParityTests.text("Views/Monitor/InferenceViews.swift")
        XCTAssertEqual(source.components(separatedBy: "store.harnesses").count - 1,
                       source.components(separatedBy: "read: store.harnesses,").count - 1,
                       "the inspector reads a second tool list")
        let window = try TracesParityTests.text("Views/MonitorWindowView.swift")
        XCTAssertEqual(window.components(separatedBy: "inference.harnesses").count - 1,
                       window.components(separatedBy: "read: inference.harnesses,").count - 1,
                       "the map reads a second tool list")
    }

    /// The one tool list is only drawn as current while this window's
    /// client has read it and its last read did not fail: a new client, or
    /// a core that stopped answering, reads as unknown (a dash, no map),
    /// never as the last list (`InferenceStore.attach` forgets it).
    func test_theToolListIsUnknownWhenTheCoreIsDownOrTheClientChanged() async throws {
        let list = try await SampleDaemonClient(.normalDay).harnessList()
        XCTAssertNotEqual(list, .none)
        XCTAssertEqual(PrivateAIInspectorView.liveHarnesses(list, read: list, failure: nil), list)
        XCTAssertEqual(PrivateAIInspectorView.liveHarnesses(list, read: nil, failure: nil), .none,
                       "a list this client never read is drawn as current")
        XCTAssertEqual(PrivateAIInspectorView.liveHarnesses(list, read: list, failure: .unreachable), .none,
                       "a list the core stopped answering for is drawn as current")
        XCTAssertNil(PrivateAIInspectorView.rows(PrivateAIInspectorView.liveHarnesses(list, read: nil, failure: nil)))

        // A store on a live client has read it; a new client forgets it.
        let store = InferenceStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        XCTAssertEqual(PrivateAIInspectorView.liveHarnesses(
            list, read: store.harnesses, failure: store.failures["harness_list"]), list)
        store.attach(SampleDaemonClient(.coreDown))
        XCTAssertEqual(PrivateAIInspectorView.liveHarnesses(
            list, read: store.harnesses, failure: store.failures["harness_list"]), .none)
        await store.load()
        XCTAssertEqual(PrivateAIInspectorView.liveHarnesses(
            list, read: store.harnesses, failure: store.failures["harness_list"]), .none)

        // The inspector's rows and the map both read through it.
        let source = try TracesParityTests.text("Views/Monitor/InferenceViews.swift")
        XCTAssertFalse(source.contains("Self.rows(model.harnesses)"), "the inspector reads the list unguarded")
        let window = try TracesParityTests.text("Views/MonitorWindowView.swift")
        XCTAssertFalse(window.contains("let harnesses = model.harnesses\n"), "the map reads the list unguarded")
        XCTAssertTrue(window.contains("PrivateAIInspectorView.liveHarnesses("))
    }

    /// HH-4: a History row's tile is the folder's first letter (#1146).
    func test_aHistoryTileIsTheFoldersInitial() {
        XCTAssertEqual(GlassToolTile.initial("api", mark: "m"), "A")
        XCTAssertEqual(GlassToolTile.initial("ébène", mark: "m"), "É")
        XCTAssertEqual(GlassToolTile.initial("", mark: "m"), "m")
        // The mark is the core's word, set at both window roots.
        XCTAssertEqual(TracesTreeWords.table?.folderMark, "dir")
        for rel in ["Views/MonitorWindowView.swift", "Views/Monitor/FirstRunViews.swift"] {
            XCTAssertTrue(try TracesParityTests.text(rel).contains(
                ".environment(\\.glassFolderMark, TracesTreeWords.table?.folderMark ?? \"\")"), rel)
        }
    }

    /// HH-1: the stat card's eyebrow wraps to a second line instead of
    /// truncating ("CREDIT / PENDING", never "CREDIT PEN...").
    func test_theStatEyebrowWraps() throws {
        let source = try TracesParityTests.text("../TCDesign/Components/StatCard.swift")
        let label = try XCTUnwrap(source.range(of: "Text(label)"))
        let value = try XCTUnwrap(source.range(of: "Text(value)"))
        let eyebrow = String(source[label.upperBound..<value.lowerBound])
        XCTAssertTrue(eyebrow.contains(".lineLimit(2)"), eyebrow)
        XCTAssertFalse(eyebrow.contains(".lineLimit(1)"), eyebrow)
    }

    /// HH-2: the withdrawal confirmation's footer is Keep it then Confirm
    /// withdrawal (destructive, never the default), both disarmed while it
    /// is in flight; without the core's words only Keep it is offered.
    func test_theWithdrawalModalActions() {
        let confirmation = WithdrawalCopy.Confirmation(
            question: "Q", description: "D", ambiguity: nil, bodies: ["b"], gravest: 0, credit: nil,
            confirmLabel: "Confirm", busyLabel: "Busy")
        let idle = WithdrawalConfirmationModal.actions(
            confirmation, keepLabel: "Keep", inFlight: false, keep: {}, confirm: {})
        XCTAssertEqual(idle.map(\.title), ["Keep", "Confirm"])
        XCTAssertEqual(idle.map(\.role), [.cancel, .destructive])
        XCTAssertTrue(idle.allSatisfy(\.isEnabled))
        XCTAssertFalse(idle.contains(where: \.isDefault))
        let busy = WithdrawalConfirmationModal.actions(
            confirmation, keepLabel: "Keep", inFlight: true, keep: {}, confirm: {})
        XCTAssertEqual(busy.map(\.title), ["Keep", "Busy"])
        XCTAssertFalse(busy.contains(where: \.isEnabled))
        let unworded = WithdrawalConfirmationModal.actions(nil, keepLabel: "Keep", inFlight: false, keep: {}, confirm: {})
        XCTAssertEqual(unworded.map(\.title), ["Keep"])
    }

    /// The first run's tool row shows a home-relative path, and only a
    /// path under the home folder is shortened.
    func test_aToolPathIsHomeRelative() {
        XCTAssertEqual(ToolAnswerRowLayout.homeRelative("/Users/a/.claude/projects", home: "/Users/a"), "~/.claude/projects")
        XCTAssertEqual(ToolAnswerRowLayout.homeRelative("/Users/a", home: "/Users/a"), "~")
        XCTAssertEqual(ToolAnswerRowLayout.homeRelative("/Users/ab/x", home: "/Users/a"), "/Users/ab/x")
        XCTAssertEqual(ToolAnswerRowLayout.homeRelative("/tmp/x", home: "/Users/a"), "/tmp/x")
        XCTAssertEqual(UsesScreenLayout.checkCaptionIndent, 15 + GlassTokens.Space.s4)
    }

    // MARK: #1273 review, macOS minors

    /// A wording-defect notice is never titled with an empty string: the
    /// table's own words, else the core's unavailable word.
    func test_aWordingDefectTitleIsNeverEmpty() throws {
        let unavailable = try XCTUnwrap(ShellWords.unavailableTitle)
        XCTAssertEqual(unavailable, "Status unavailable")
        XCTAssertEqual(ShellWords.defectTitle(nil), unavailable)
        XCTAssertEqual(ShellWords.defectTitle(""), unavailable)
        XCTAssertEqual(ShellWords.defectTitle("Do not trust this"), "Do not trust this")
        XCTAssertNotNil(WithdrawalCopy.wordingDefect)
        XCTAssertNotNil(PublicProfileCopy.wordingDefect)
        for rel in ["Views/WithdrawalCopy.swift", "Views/PublicProfileCopy.swift"] {
            let source = try TracesParityTests.text(rel)
            XCTAssertFalse(source.contains("?? \"\"\n    }\n}"), "\(rel) falls back to an empty title")
            XCTAssertTrue(source.contains("static var wordingDefect: String? { ShellWords.defectTitle(words?.wordingDefect) }"), rel)
        }
    }

    /// The modals that wait on work in flight say so, so the scrim, Escape
    /// and the close button cannot abandon it.
    func test_theBusyModalsIgnoreTheScrimWhileBusy() throws {
        for (rel, needle) in [("Views/HarnessListView.swift", "busy: model.harnessBusy"),
                              ("Views/Settings/PublicProfileSection.swift", "busy: model.profileBusy,"),
                              ("Views/ManagedSessionsView.swift", "busy: model.managedBusy,")] {
            XCTAssertTrue(try TracesParityTests.text(rel).contains(needle), "\(rel) lacks \(needle)")
        }
        XCTAssertEqual(
            try TracesParityTests.text("Views/HarnessListView.swift").components(separatedBy: "busy: model.harnessBusy").count - 1,
            2, "the preview and the exposure question both wait on the write")
    }

    /// The Runtime tile reads the core's `runtime_word`; no Swift switch
    /// re-implements it.
    func test_theRuntimeWordIsTheCores() throws {
        let account = try TracesParityTests.text("Views/Monitor/InferenceAccount.swift")
        XCTAssertTrue(account.contains("TCPrivateInference.runtimeWord(state: state?.state ?? \"\") ?? copy.runtimeUnknown"))
        XCTAssertFalse(account.contains("case \"running_elsewhere\""))
        let copy = try XCTUnwrap(PrivateInferenceCopy.decode(fromJSON: TCPrivateInference.copyJSON() ?? ""))
        XCTAssertEqual(TCPrivateInference.runtimeWord(state: "off"), copy.runtimeOff)
        XCTAssertEqual(TCPrivateInference.runtimeWord(state: "running"), copy.runtimeOn)
        XCTAssertEqual(TCPrivateInference.runtimeWord(state: ""), copy.runtimeUnknown)
        XCTAssertEqual(TCPrivateInference.runtimeWord(state: "a_later_state"), copy.runtimeUnknown)
    }

    /// zmanian's follow-up on #1273: the Traces graph footer counts History
    /// by the map's rule, so a capped or stale page is a dash, never a total.
    func test_theGraphFooterCountsOnlyAWholeCurrentPage() throws {
        XCTAssertNil(TracesGraphFooter.countable(nil, failure: nil))
        XCTAssertEqual(TracesGraphFooter.countable([], failure: nil), [])
        XCTAssertNil(TracesGraphFooter.countable([], failure: .unreachable), "a stale page is counted as current")
        let window = try TracesParityTests.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains(
            "history: TracesGraphFooter.countable(home.history, failure: home.failures[\"list_history\"]),"))
        XCTAssertFalse(window.contains("history: home.history, sessions:"))
    }
}
