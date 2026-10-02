import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// R6 of #1173: the Traces tree is built only from what the core reports,
/// against C1's sample sets.
@MainActor
final class TracesTreeTests: XCTestCase {
    private func tree(_ set: SampleDaemonClient.SampleSet) async throws -> TracesTree {
        let client = SampleDaemonClient(set)
        return TracesTree.build(
            entries: try await client.listPending(projectId: nil),
            projects: try await client.listProjects().projects,
            settings: try? await client.settings())
    }

    /// Every waiting session appears in the tree exactly once.
    func test_everySessionIsInTheTreeOnce() async throws {
        for set in SampleDaemonClient.SampleSet.allCases where set != .coreDown {
            let client = SampleDaemonClient(set)
            let pending = try await client.listPending(projectId: nil).map(\.entryId)
            let drawn = try await tree(set).allSessions.map(\.entryId)
            XCTAssertEqual(drawn.sorted(), pending.sorted(), "\(set)")
        }
    }

    /// A folder sits under the tool most of its sessions came from.
    func test_aFolderSitsUnderItsMajorityTool() async throws {
        let tree = try await tree(.normalDay)
        let claude = try XCTUnwrap(tree.tools.first { $0.kind == .claudeCode })
        // api has two Claude Code sessions and one Codex session.
        XCTAssertTrue(claude.folders.contains { $0.label == "api" })
        XCTAssertFalse(tree.tools.first { $0.kind == .codex }?.folders.contains { $0.label == "api" } ?? false)
    }

    /// Unset is never drawn as off: an unset tool with nothing waiting is not
    /// drawn, and an unset tool is never `.off`.
    func test_unsetIsNeverOff() async throws {
        let tree = try await tree(.normalDay)
        XCTAssertNil(tree.tools.first { $0.kind == .opencode }, "opencode is unset with nothing waiting")
        XCTAssertNil(tree.tools.first { $0.kind == .cline }, "cline is unset with nothing waiting")
        XCTAssertEqual(tree.tools.first { $0.kind == .geminiCli }?.mode, .off)
        XCTAssertEqual(TracesTree.SourceMode(nil), .unset)
        XCTAssertEqual(TracesTree.SourceMode("unset"), .unset)
    }

    /// Unreadable settings leave every tool unset, which draws no switch.
    func test_unreadableSettingsDrawNoSwitches() async throws {
        let client = SampleDaemonClient(.normalDay)
        let tree = TracesTree.build(
            entries: try await client.listPending(projectId: nil),
            projects: try await client.listProjects().projects,
            settings: nil)
        XCTAssertTrue(tree.tools.allSatisfy { $0.mode == .unset })
    }

    /// Ignored folders are left out, and their sessions with them.
    func test_ignoredFoldersAreLeftOut() throws {
        let project = ProjectRow(projectId: "p1", projectLabel: "quiet", projectPath: "/x", mode: .ignore)
        let tree = TracesTree.build(entries: [], projects: [project], settings: nil)
        XCTAssertTrue(tree.tools.isEmpty)
        XCTAssertTrue(tree.unplaced.isEmpty)
    }

    /// A folder with nothing waiting cannot be placed until the core says
    /// which tool it belongs to (K11): it is listed on its own.
    func test_aFolderWithNothingWaitingIsUnplaced() throws {
        let project = ProjectRow(projectId: "p1", projectLabel: "docs", projectPath: "/x", mode: .ask)
        let tree = TracesTree.build(entries: [], projects: [project], settings: nil)
        XCTAssertEqual(tree.unplaced.map(\.label), ["docs"])
    }

    /// The core being down is a failure the tab says, not an empty tree.
    func test_coreDownIsAFailureNotAnEmptyTree() async {
        let store = TracesStore(client: SampleDaemonClient(.coreDown))
        await store.load()
        XCTAssertEqual(store.phase, .failed(.unreachable))
    }

    func test_theStoreLoadsTheSampleDay() async {
        let store = TracesStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        XCTAssertEqual(store.phase, .loaded)
        XCTAssertEqual(store.tree.allSessions.count, 3)
    }
}

/// R7: the badge and the review actions, against C1's sample sets.
@MainActor
final class TracesReviewTests: XCTestCase {
    /// The badge is `decisions_owed`; unknown stays unknown, never a count
    /// derived from the queue.
    func test_theBadgeIsDecisionsOwedOrUnknown() async {
        let normal = TracesStore(client: SampleDaemonClient(.normalDay))
        await normal.load()
        XCTAssertEqual(normal.decisionsOwed, normal.status?.decisionsOwed)
        XCTAssertNotNil(normal.decisionsOwed)

        let unknown = TracesStore(client: SampleDaemonClient(.unknownCounts))
        await unknown.load()
        XCTAssertNotNil(unknown.status, "status was read")
        XCTAssertNil(unknown.decisionsOwed, "decisions_owed absent is unknown")
        XCTAssertFalse(unknown.tree.allSessions.isEmpty, "and the queue is not where a count comes from")

        let down = TracesStore(client: SampleDaemonClient(.coreDown))
        await down.load()
        XCTAssertNil(down.decisionsOwed)
    }

    /// Keep offers its undo, and the undo withdraws it.
    func test_keepOffersAnUndo() async throws {
        let store = TracesStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        let id = try XCTUnwrap(store.tree.allSessions.first?.entryId)
        await store.perform(.keep, on: id)
        XCTAssertEqual(store.lastKept, id)
        await store.perform(.undoKeep, on: id)
        XCTAssertNil(store.lastKept)
        XCTAssertTrue(store.acting.isEmpty)
    }

    /// A refused action is kept against its session; nothing is applied.
    func test_aRefusedActionIsKept() async {
        let store = TracesStore(client: SampleDaemonClient(.coreDown))
        await store.perform(.contribute, on: "e1")
        XCTAssertEqual(store.actionError?.entryId, "e1")
        XCTAssertEqual(store.actionError?.error, .unreachable)
        XCTAssertNil(store.lastKept)
    }
}

/// Review of #1183: a folder keeps its three modes, and ignoring one says
/// what the core actually cleared.
@MainActor
final class TracesFolderModeTests: XCTestCase {
    func test_foldersCarryTheModesTheyCanBeSetTo() async throws {
        let store = TracesStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        let folders = store.tree.tools.flatMap(\.folders) + store.tree.unplaced
        XCTAssertFalse(folders.isEmpty)
        for folder in folders where folder.mode != nil {
            XCTAssertEqual(Set(folder.offerableModes), [.ask, .autoUpload, .ignore], folder.label)
        }
    }

    /// The core's `purged` is the authority. When it differs from the count
    /// the confirmation named, the difference is said in the core's words.
    func test_ignoringSaysWhenTheQueueChanged() async throws {
        let store = TracesStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        let folder = try XCTUnwrap(store.tree.tools.flatMap(\.folders).first { !$0.sessions.isEmpty })
        // The sample core answers purged 0; the confirmation promised more.
        await store.setFolderMode(folder, .ignore, promised: folder.sessions.count)
        XCTAssertEqual(
            store.folderNotice,
            ProjectIgnoreCopy.reconciliation(project: folder.label, promised: folder.sessions.count, purged: 0))
        XCTAssertNotNil(store.folderNotice)

        // When they agree, nothing is said.
        await store.setFolderMode(folder, .ignore, promised: 0)
        XCTAssertNil(store.folderNotice)
    }

    /// A refused write keeps its error and leaves the tree as the core has it.
    func test_aRefusedModeWriteKeepsItsError() async throws {
        let store = TracesStore(client: SampleDaemonClient(.coreDown))
        let folder = TracesTree.FolderNode(id: "p1", label: "docs", mode: .ask, offerableModes: [.ask, .ignore], sessions: [])
        await store.setFolderMode(folder, .ignore, promised: 0)
        XCTAssertEqual(store.phase, .failed(.unreachable))
        XCTAssertTrue(store.writing.isEmpty)
    }
}
