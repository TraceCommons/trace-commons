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
        XCTAssertEqual(copy.routingRowsSeen, "Receiving proxy rows")
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
}
