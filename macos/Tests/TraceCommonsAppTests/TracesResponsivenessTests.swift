import XCTest
import TCShellCore
@testable import TraceCommonsApp

/// The Traces tab under load: typing in its search, a few hundred sessions
/// waiting, and the burst of events a Submit all sends. These count work
/// done (narrowings, reloads, calls into the core), never time.
@MainActor
final class TracesResponsivenessTests: XCTestCase {
    private func loaded(_ set: SampleDaemonClient.SampleSet = .normalDay) async -> TracesStore {
        let store = TracesStore(client: SampleDaemonClient(set))
        await store.load()
        return store
    }

    // MARK: Search

    func test_typingAWordNarrowsTheTreeOnce() async {
        let store = await loaded()
        let before = store.narrowings
        for prefix in ["m", "mi", "min"] { store.searchText = prefix }
        XCTAssertEqual(store.narrowings, before, "no keystroke narrows the tree by itself")
        XCTAssertEqual(store.shownTree, store.tree)
        await store.searchSettled()
        XCTAssertEqual(store.narrowings, before + 1, "the word narrows once, not once per letter")
        XCTAssertEqual(store.appliedSearch, "min")
        XCTAssertEqual(store.shownTree, store.tree.matching("min"))
    }

    func test_clearingTheSearchShowsTheWholeTreeAtOnce() async {
        let store = await loaded()
        store.searchText = "zzz-no-such-folder"
        await store.searchSettled()
        XCTAssertNotEqual(store.shownTree, store.tree)
        store.searchText = ""
        XCTAssertEqual(store.appliedSearch, "")
        XCTAssertEqual(store.shownTree, store.tree, "clearing does not wait")
    }

    func test_aNewReadKeepsTheSearch() async {
        let store = await loaded()
        store.searchText = "min"
        await store.searchSettled()
        await store.load()
        XCTAssertEqual(store.shownTree, store.tree.matching("min"))
    }

    // MARK: Rows

    /// Each row's lines are the core's words, worked out once per read
    /// rather than on every redraw.
    func test_rowLinesAreTheCoresWordsFromTheRead() async throws {
        let store = await loaded()
        let entries = store.tree.folders.flatMap { $0.sessions }
        XCTAssertFalse(entries.isEmpty)
        for entry in entries {
            let lines = try XCTUnwrap(store.rowLines[entry.entryId], "no lines for \(entry.entryId)")
            XCTAssertEqual(lines.ineligible, store.ineligibleLine(entry))
            XCTAssertEqual(lines.attestation, store.attestationLine(entry))
            XCTAssertEqual(lines.tone, TracesStore.attestationTone(entry))
            XCTAssertEqual(store.lines(entry), lines)
        }
    }

    func test_aNewClientDropsTheOldRowsLines() async {
        let store = await loaded()
        XCTAssertFalse(store.rowLines.isEmpty)
        store.attach(nil)
        XCTAssertTrue(store.rowLines.isEmpty)
    }

    // MARK: Events

    /// A Submit all over a hundred traces sends hundreds of `queue_changed`.
    /// One reload each kept the main thread busy for as long as they came.
    func test_aBurstOfQueueChangesIsNotOneReloadPerEvent() async throws {
        let client = SampleDaemonClient(.normalDay)
        let store = TracesStore(client: client)
        store.reloadDelay = .milliseconds(20)
        let running = Task { await store.run() }
        defer { running.cancel() }
        while store.reloads == nil { await Task.yield() }
        let reloads = try XCTUnwrap(store.reloads)
        for _ in 0..<300 { client.emit(.queueChanged) }
        // Let `run` take every event off the stream, then let the reloads
        // they asked for finish.
        for _ in 0..<500 { await Task.yield() }
        await reloads.settled()
        XCTAssertGreaterThanOrEqual(reloads.runs, 1, "a change is still read")
        XCTAssertLessThan(reloads.runs, 10, "300 events are not 300 reloads")
        XCTAssertEqual(store.phase, .loaded)
    }

    // MARK: Folders a search opens

    func test_aSearchOpensWhatItMatched() {
        let opened = TracesTreeView.searchExpansion(
            query: "min", matched: ["a", "b"], expanded: ["b", "c"], searchOpened: [])
        XCTAssertEqual(opened.expanded, ["a", "b", "c"])
        XCTAssertEqual(opened.searchOpened, ["a"], "b was open before the search")
    }

    func test_clearingASearchClosesOnlyWhatItOpened() {
        let typed = TracesTreeView.searchExpansion(
            query: "m", matched: ["a", "b", "d"], expanded: ["b", "c"], searchOpened: [])
        let narrowed = TracesTreeView.searchExpansion(
            query: "mi", matched: ["a"], expanded: typed.expanded, searchOpened: typed.searchOpened)
        let cleared = TracesTreeView.searchExpansion(
            query: "", matched: [], expanded: narrowed.expanded, searchOpened: narrowed.searchOpened)
        XCTAssertEqual(cleared.expanded, ["b", "c"], "the folders open before the search stay open")
        XCTAssertTrue(cleared.searchOpened.isEmpty)
    }

    /// Clearing a search never hides the session the person selected
    /// inside a folder the search opened.
    func test_clearingASearchKeepsTheSelectedSessionsFolderOpen() {
        let typed = TracesTreeView.searchExpansion(
            query: "m", matched: ["a", "b"], expanded: [], searchOpened: [])
        let cleared = TracesTreeView.searchExpansion(
            query: "", matched: [], expanded: typed.expanded, searchOpened: typed.searchOpened, keep: "a")
        XCTAssertEqual(cleared.expanded, ["a"])
        XCTAssertTrue(cleared.searchOpened.isEmpty)
    }

    func test_aBlankSearchOpensNothing() {
        let blank = TracesTreeView.searchExpansion(
            query: "   ", matched: ["a"], expanded: ["c"], searchOpened: [])
        XCTAssertEqual(blank.expanded, ["c"])
        XCTAssertTrue(blank.searchOpened.isEmpty)
    }

    /// The tree stays an eager `VStack`: row menus float over later rows
    /// by `zIndex`, which a `LazyVStack` does not honour between its
    /// children, and `scrollTo` and VoiceOver focus reach rows a lazy stack
    /// has not built. Rows are cheap now that their lines come from the read.
    func test_theTreeIsAnEagerStack() throws {
        let source = try TracesParityTests.text("Views/Monitor/TracesViews.swift")
        let tree = try XCTUnwrap(source.range(of: "private var tree: some View {"))
        let folders = try XCTUnwrap(source.range(of: "ForEach(store.shownTree.folders)", range: tree.upperBound..<source.endIndex))
        let body = source[tree.upperBound..<folders.lowerBound]
        XCTAssertTrue(body.contains("VStack(spacing: 2)"))
        XCTAssertFalse(body.contains("LazyVStack("))
    }
}
