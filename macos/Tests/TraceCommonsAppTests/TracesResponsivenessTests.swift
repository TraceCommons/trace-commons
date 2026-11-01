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

    /// Each folder's Submit all offer is the shared table's answer, worked
    /// out with the read, so drawing a folder row calls nothing in the core.
    func test_folderOffersAreWorkedOutWithTheRead() async throws {
        for set: SampleDaemonClient.SampleSet in [.normalDay, .busyQueue] {
            let store = await loaded(set)
            XCTAssertFalse(store.tree.folders.isEmpty)
            for folder in store.tree.folders {
                let input = TracesStore.offerInput(folder, idleOnly: store.idleOnly)
                let held = try XCTUnwrap(store.offers[input], "no offer for \(folder.label)")
                XCTAssertEqual(held, TracesStore.offer(input))
                XCTAssertEqual(store.groupOffer(folder), held)
            }
        }
        // Under the idle filter the offer counts what is drawn instead.
        let store = await loaded(.busyQueue)
        await store.showIdleOnly(true)
        for folder in store.tree.folders {
            let input = TracesStore.offerInput(folder, idleOnly: true)
            XCTAssertNil(input.contributable)
            XCTAssertEqual(store.offers[input], TracesStore.offer(input))
        }
        store.attach(nil)
        XCTAssertTrue(store.offers.isEmpty, "a new client keeps the old offers")
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

    /// The tree is a `LazyVStack`. Rows were not cheap: at 231 folders,
    /// building every one took a fifth of a second each time the tab opened
    /// (`TracesScalePerfTests`, `TracesTreeScaleTests`). What the eager
    /// stack was kept for, each with its own mechanism now:
    /// - row menus float over later rows from one overlay on the stack, at
    ///   the bounds the open row publishes, not by `zIndex` between lazy
    ///   children;
    /// - `scrollTo` reaches an unbuilt row by the flat `ForEach`'s ids, and
    ///   waits for a folder it just opened to be drawn;
    /// - VoiceOver focus moves after the scroll has built the row.
    func test_theTreeIsALazyStackWithItsMenusOverIt() throws {
        let source = try TracesParityTests.text("Views/Monitor/TracesViews.swift")
        let tree = try XCTUnwrap(source.range(of: "private var tree: some View {"))
        let rows = try XCTUnwrap(source.range(
            of: "ForEach(Self.drawnRows(in: store.shownTree, expanded: expanded))", range: tree.upperBound..<source.endIndex))
        let head = source[tree.upperBound..<rows.lowerBound]
        XCTAssertTrue(head.contains("LazyVStack(spacing: 2)"))
        let body = String(source[tree.upperBound...].prefix(4000))
        for needle in [
            ".overlayPreferenceValue(TracesMenuAnchors.self, alignment: .topLeading)",
            "revealed.scrollTarget = id", ".onChange(of: expanded)", "Task { @MainActor in spoken = id }",
            // An open menu is read after the rows, so VoiceOver is moved to it.
            ".onChange(of: rowMenu)", "Task { @MainActor in menuSpoken = open }",
        ] {
            XCTAssertTrue(body.contains(needle) || source.contains(needle), "the tree lacks \(needle)")
        }
        // No row floats its own menu or orders itself over the others.
        let rowsStart = try XCTUnwrap(source.range(of: "struct TracesFolderRow"))
        let rowsEnd = try XCTUnwrap(source.range(of: "struct PreviewSlot"))
        let rowViews = source[rowsStart.lowerBound..<rowsEnd.lowerBound]
        XCTAssertFalse(rowViews.contains(".zIndex("))
        XCTAssertFalse(rowViews.contains("TracesRowMenuPill("))
        XCTAssertEqual(rowViews.components(separatedBy: ".anchorPreference(key: TracesMenuAnchors.self").count - 1, 2)
        XCTAssertEqual(source.components(separatedBy: ".accessibilityFocused($menuSpoken, equals: ").count - 1, 2)
        // Each row is rebuilt only when what it draws changes.
        XCTAssertEqual(source.components(separatedBy: ".equatable()\n        .id(Self.rowID(").count - 1, 2)
    }

    // MARK: VoiceOver (follows #1505's review)

    /// One row menu is open at a time, and every open or switch is a new
    /// value, so the tree's `onChange` fires and VoiceOver moves to the
    /// menu. Under #1505 the key was `folderMenu ?? submitMenu ??
    /// sessionMenu`: a folder's "..." menu then its Submit chevron kept the
    /// key, and a session's menu opened over an open folder menu kept it too.
    func test_openingAnyMenuClosesTheOthersAndChangesTheKey() {
        typealias Menu = TracesTreeView.RowMenu
        let all: [Menu] = [.folder("X"), .submit("X"), .session("S"), .folder("Y"), .submit("Y"), .session("T")]
        for current in all {
            for opened in all where opened != current {
                let next = TracesTreeView.toggled(opened, current: current)
                XCTAssertEqual(next, opened, "opening \(opened) over \(current)")
                XCTAssertNotEqual(next, current, "opening \(opened) over \(current) left the key unchanged")
            }
            // The same control again closes its menu.
            XCTAssertNil(TracesTreeView.toggled(current, current: current))
            XCTAssertEqual(TracesTreeView.toggled(current, current: nil), current)
        }
    }

    /// The menu state is the one tagged value, and VoiceOver's menu focus
    /// is keyed on it, not a Bool that is already true on a switch.
    func test_menuFocusFollowsTheOneOpenMenu() throws {
        let source = try TracesParityTests.text("Views/Monitor/TracesViews.swift")
        for gone in ["@State private var folderMenu", "@State private var submitMenu", "@State private var sessionMenu",
                     "@AccessibilityFocusState private var menuSpoken: Bool", "folderMenu ?? submitMenu"] {
            XCTAssertFalse(source.contains(gone), "TracesViews.swift still has \(gone)")
        }
        for needle in ["@State private var rowMenu: RowMenu?", "@AccessibilityFocusState private var menuSpoken: RowMenu?",
                       ".onChange(of: rowMenu)", "Task { @MainActor in menuSpoken = open }"] {
            XCTAssertTrue(source.contains(needle), "TracesViews.swift lacks \(needle)")
        }
        XCTAssertEqual(source.components(separatedBy: ".accessibilityFocused($menuSpoken, equals: ").count - 1, 2)
        // Every control that opens a menu goes through the one toggle.
        XCTAssertEqual(source.components(separatedBy: "rowMenu = Self.toggled(").count - 1, 3)
    }

    /// VoiceOver focus is on the element it lands on: the `GlassListRow`,
    /// an accessibility container. #1505 moved it to the row wrappers,
    /// which are plain views. The `.id` stays on the wrapper, the
    /// `LazyVStack`'s child that `scrollTo` reaches.
    func test_rowFocusIsOnTheListRowContainer() throws {
        let source = try TracesParityTests.text("Views/Monitor/TracesViews.swift")
        let rowsStart = try XCTUnwrap(source.range(of: "struct TracesFolderRow"))
        let rowsEnd = try XCTUnwrap(source.range(of: "struct PreviewSlot"))
        let rowViews = String(source[rowsStart.lowerBound..<rowsEnd.lowerBound])
        let tree = String(source[..<rowsStart.lowerBound])
        XCTAssertEqual(tree.components(separatedBy: ".accessibilityFocused($spoken").count - 1, 0,
                       "the tree focuses the row wrapper, not its container")
        XCTAssertEqual(rowViews.components(separatedBy: "let spoken: AccessibilityFocusState<String?>.Binding").count - 1, 2)
        for row in ["struct TracesFolderRow", "struct TracesSessionRow"] {
            let start = try XCTUnwrap(rowViews.range(of: row))
            let body = try XCTUnwrap(rowViews.range(of: "var body: some View {", range: start.upperBound..<rowViews.endIndex))
            let listRow = try XCTUnwrap(rowViews.range(of: "GlassListRow(", range: body.upperBound..<rowViews.endIndex))
            let call = try XCTUnwrap(rowViews.range(of: "\n            )\n", range: listRow.upperBound..<rowViews.endIndex))
            let after = rowViews[call.upperBound...].prefix(120)
            XCTAssertTrue(after.hasPrefix("            .accessibilityFocused(spoken, equals: TracesTreeView.rowID("),
                          "\(row): the focus is not the GlassListRow's first modifier")
        }
        XCTAssertEqual(source.components(separatedBy: ".equatable()\n        .id(Self.rowID(").count - 1, 2)
    }
}
