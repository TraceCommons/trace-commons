import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// R6 of #1173: the Traces tree is built only from what the core reports,
/// against C1's sample sets.
@MainActor
final class TracesTreeTests: XCTestCase {
    /// The tools the core says are read from their usual folder while
    /// unset (`unset_scans_conventional`): Claude Code and Codex.
    static let scansWhenUnset: Set<SourceKind> = [.claudeCode, .codex]

    private func tree(_ set: SampleDaemonClient.SampleSet) async throws -> TracesTree {
        let client = SampleDaemonClient(set)
        return TracesTree.build(
            entries: try await client.listPending(projectId: nil),
            projects: try await client.listProjects().projects,
            settings: try? await client.settings(),
            scansWhenUnset: Self.scansWhenUnset)
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

    /// Unset is never drawn as off. A tool the core reads from its usual
    /// folder while unset (Claude Code, Codex) is always drawn, nothing
    /// waiting or not, because it IS being read; a tool that opens nothing
    /// while unset, with nothing waiting, is not drawn.
    func test_unsetIsNeverOff() async throws {
        let tree = try await tree(.normalDay)
        XCTAssertNil(tree.tools.first { $0.kind == .opencode }, "opencode is unset with nothing waiting")
        XCTAssertNil(tree.tools.first { $0.kind == .cline }, "cline is unset with nothing waiting")
        XCTAssertEqual(tree.tools.first { $0.kind == .geminiCli }?.mode, .off)
        XCTAssertEqual(TracesTree.SourceMode("unset"), .unset)
        XCTAssertEqual(TracesTree.SourceMode(nil), .unset)

        // Unset with nothing waiting: drawn for a tool scanned while unset.
        XCTAssertTrue(TracesTree.drawsTool(.claudeCode, mode: .unset, hasFolders: false, scansWhenUnset: Self.scansWhenUnset))
        XCTAssertTrue(TracesTree.drawsTool(.codex, mode: .unset, hasFolders: false, scansWhenUnset: Self.scansWhenUnset))
        XCTAssertFalse(TracesTree.drawsTool(.cline, mode: .unset, hasFolders: false, scansWhenUnset: Self.scansWhenUnset))
    }

    /// The set comes from the core's source copy, and is the two tools
    /// this file's fixture assumes.
    func test_theCoreSaysWhichToolsAreReadWhileUnset() {
        XCTAssertEqual(TracesStore.scansWhenUnset, Self.scansWhenUnset)
    }

    /// Unreadable settings are unknown, not unset: no switch, and the
    /// tools that may be being read are still drawn, so the tab can say the
    /// declaration could not be confirmed rather than drop them.
    func test_unreadableSettingsAreUnknownNotUnset() async throws {
        let tree = TracesTree.build(entries: [], projects: [], settings: nil, scansWhenUnset: Self.scansWhenUnset)
        XCTAssertEqual(Set(tree.tools.map(\.kind)), [.claudeCode, .codex])
        XCTAssertTrue(tree.tools.allSatisfy { $0.mode == .unknown })
    }

    /// Ignored folders are left out, and their sessions with them.
    func test_ignoredFoldersAreLeftOut() throws {
        let project = ProjectRow(projectId: "p1", projectLabel: "quiet", projectPath: "/x", mode: .ignore)
        let tree = TracesTree.build(entries: [], projects: [project], settings: nil, scansWhenUnset: [])
        XCTAssertTrue(tree.tools.isEmpty)
        XCTAssertTrue(tree.unplaced.isEmpty)
    }

    /// A folder with nothing waiting cannot be placed until the core says
    /// which tool it belongs to (K11): it is listed on its own.
    func test_aFolderWithNothingWaitingIsUnplaced() throws {
        let project = ProjectRow(projectId: "p1", projectLabel: "docs", projectPath: "/x", mode: .ask)
        let tree = TracesTree.build(entries: [], projects: [project], settings: nil, scansWhenUnset: [])
        XCTAssertEqual(tree.unplaced.map(\.label), ["docs"])
    }

    /// The tab's words are the core's: the store holds the decoded export,
    /// and the inspector's row labels are its fields, not Swift strings.
    func test_theTabsWordsComeFromTheCore() async throws {
        let store = TracesStore(client: SampleDaemonClient(.normalDay))
        let words = try XCTUnwrap(store.words)
        XCTAssertEqual(words, MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON()))
        let client = SampleDaemonClient(.normalDay)
        let pending = try await client.listPending(projectId: nil)
        let entry = try XCTUnwrap(pending.first)
        let keys = SessionInspectorView.rows(entry, nil, words: words).map(\.label)
        XCTAssertEqual(keys, [words.tool, words.folder, words.started, words.length, words.prompts,
                              words.size, words.sends, words.marks, words.unsure])
        // A failure is said in the core's line, not the error's label.
        XCTAssertEqual(words.line(for: .unreachable), words.coreUnreachable)
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
        // The notice is the core's, naming the folder and both counts.
        XCTAssertGreaterThan(folder.sessions.count, 0)
        await store.setFolderMode(folder, .ignore, promised: folder.sessions.count)
        let notice = try XCTUnwrap(store.folderNotice)
        XCTAssertTrue(notice.contains(folder.label), notice)
        XCTAssertTrue(notice.contains("0 "), notice)
        XCTAssertTrue(notice.contains(String(folder.sessions.count)), notice)

        // When they agree, nothing is said.
        await store.setFolderMode(folder, .ignore, promised: 0)
        XCTAssertNil(store.folderNotice)
    }

    /// The tool switch writes the source declaration through `setSource`,
    /// and the tree reloads from the core's answer.
    func test_theToolSwitchWritesTheSourceDeclaration() async throws {
        let store = TracesStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        await store.setSource(.codex, .off)
        XCTAssertEqual(store.phase, .loaded)
        XCTAssertTrue(store.writing.isEmpty)

        // A watch with no folder is not an answer; the refusal is kept.
        await store.setSource(.codex, .watch(path: ""))
        guard case .failed = store.phase else { return XCTFail("\(store.phase)") }
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
