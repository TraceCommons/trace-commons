import TCBridge
import TCDesign
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The nudge on Traces: the idle and backlog card, what its buttons send,
/// the idle filter, the suggested order and a row's tags.
@MainActor
final class NudgeTracesTests: XCTestCase {
    private func entries(_ json: String) throws -> [DaemonData.QueueEntry] {
        try DaemonDataDecoding.decoder().decode(DaemonData.PendingList.self, from: Data(json.utf8)).pending
    }

    /// Three sessions the daemon listed in suggested order: the oldest
    /// first, across two folders.
    private var suggested: [DaemonData.QueueEntry] {
        get throws {
            try entries(#"""
                {"pending":[
                 {"entry_id":"old","source":"codex","project_id":"b","project_label":"B","state":"pending",
                  "started_at":"2026-10-01T09:00:00Z","mission_fit":1},
                 {"entry_id":"new","source":"codex","project_id":"a","project_label":"A","state":"pending",
                  "started_at":"2026-10-07T09:00:00Z"},
                 {"entry_id":"mid","source":"codex","project_id":"b","project_label":"B","state":"pending",
                  "started_at":"2026-10-04T09:00:00Z"}]}
                """#)
        }
    }

    // MARK: - The order

    /// Queue order draws newest first, as before. Suggested order keeps the
    /// daemon's order: folders by their first suggested session, sessions
    /// as listed.
    func test_suggestedOrderIsTheDaemonsOrderNotRecency() throws {
        let byRecency = TracesTree.build(entries: try suggested, projects: [], settings: nil, scansWhenUnset: [])
        XCTAssertEqual(byRecency.folders.map(\.id), ["a", "b"])
        XCTAssertEqual(byRecency.allSessions.map(\.entryId), ["new", "mid", "old"])

        let kept = TracesTree.build(
            entries: try suggested, projects: [], settings: nil, scansWhenUnset: [], keepsOrder: true)
        XCTAssertEqual(kept.folders.map(\.id), ["b", "a"])
        XCTAssertEqual(kept.allSessions.map(\.entryId), ["old", "mid", "new"])
    }

    /// Narrowed to the idle sessions, a folder with none of them is not
    /// drawn: the list shows the set the card named.
    func test_theIdleFilterDrawsOnlyFoldersWithIdleSessions() async throws {
        let client = SampleDaemonClient(.normalDay)
        let projects = try await client.listProjects().projects
        let idle = try await client.listPending(projectId: nil, filter: .idleSessions, order: nil)
        let tree = TracesTree.build(
            entries: idle, projects: projects, settings: nil, scansWhenUnset: [], onlyWithSessions: true)
        XCTAssertFalse(tree.folders.isEmpty)
        XCTAssertTrue(tree.folders.allSatisfy { !$0.sessions.isEmpty })
        XCTAssertEqual(tree.allSessions.count, idle.count)
    }

    // MARK: - The card

    func test_theIdleCardIsTheDaemonsWords() async {
        let store = TracesStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        let card = store.nudgeCard
        XCTAssertEqual(card?.kind, .idleSessions)
        XCTAssertEqual(card?.title, "2 sessions from Claude Code have been idle for 3 days or more")
        XCTAssertEqual(card?.actions.map(\.label), ["Review", "Not now"])
        // Nothing to say, nothing drawn.
        let quiet = TracesStore(client: SampleDaemonClient(.empty))
        await quiet.load()
        XCTAssertNil(quiet.nudgeCard)
        let older = TracesStore(client: SampleDaemonClient(.unknownCounts))
        await older.load()
        XCTAssertNil(older.nudgeCard)
    }

    /// Review records that the suggestion was opened and narrows the list
    /// to the idle sessions; Show all leaves the filter.
    func test_reviewOpensTheIdleSessionsAndShowAllLeaves() async throws {
        let client = SampleDaemonClient(.normalDay)
        let store = TracesStore(client: client)
        await store.load()
        let everything = store.tree.allSessions.count
        await store.perform(.review(.idleSessions))
        XCTAssertEqual(client.nudgeCalls, ["nudge_opened idle_sessions"])
        XCTAssertTrue(store.idleOnly)
        XCTAssertEqual(store.tree.allSessions.count, 2)
        XCTAssertLessThanOrEqual(store.tree.allSessions.count, everything)
        await store.showIdleOnly(false)
        XCTAssertFalse(store.idleOnly)
        XCTAssertEqual(store.tree.allSessions.count, everything)
    }

    func test_notNowDeclinesTheKindAndGoesNowhere() async {
        let client = SampleDaemonClient(.normalDay)
        let store = TracesStore(client: client)
        await store.load()
        await store.perform(.notNow(.idleSessions))
        XCTAssertEqual(client.nudgeCalls, ["nudge_decline idle_sessions"])
        XCTAssertFalse(store.idleOnly)
        XCTAssertNil(store.nudgeError)
    }

    /// A refused request is said, never drawn as done.
    func test_aRefusedNudgeRequestIsKept() async {
        let store = TracesStore(client: SampleDaemonClient(.coreDown))
        await store.perform(.notNow(.idleSessions))
        XCTAssertEqual(store.nudgeError, .unreachable)
    }

    /// Until an order is chosen the tree is drawn as it always was, newest
    /// first, and neither choice is shown as selected. A chosen order is
    /// the daemon's: suggested, or the queue's own, oldest first.
    func test_theOrderControlAsksTheDaemonForItsOrder() async {
        let store = TracesStore(client: SampleDaemonClient(.busyQueue))
        await store.load()
        XCTAssertNil(store.order)
        XCTAssertFalse(store.keepsOrder)
        await store.setOrder(.suggested)
        XCTAssertEqual(store.order, .suggested)
        XCTAssertTrue(store.keepsOrder)
        XCTAssertEqual(store.phase, .loaded)
        await store.setOrder(.queue)
        XCTAssertTrue(store.keepsOrder)
        let listed = try? await SampleDaemonClient(.busyQueue).listPending(projectId: nil)
        // Oldest first is the queue as the daemon listed it, folder by folder.
        var firstSeen: [String] = []
        for entry in listed ?? [] where !firstSeen.contains(entry.projectId) { firstSeen.append(entry.projectId) }
        XCTAssertFalse(firstSeen.isEmpty)
        XCTAssertEqual(store.tree.folders.filter { !$0.sessions.isEmpty }.map(\.id), firstSeen)
        let grouped = firstSeen.flatMap { id in (listed ?? []).filter { $0.projectId == id }.map(\.entryId) }
        XCTAssertEqual(store.tree.allSessions.map(\.entryId), grouped)
    }

    /// A new daemon starts unfiltered, and nothing from the old one stays.
    func test_aNewClientClearsTheFilter() async {
        let store = TracesStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        await store.perform(.review(.idleSessions))
        store.attach(SampleDaemonClient(.normalDay))
        XCTAssertFalse(store.idleOnly)
        XCTAssertNil(store.nudgeError)
        XCTAssertEqual(store.rowTags, [:])
    }

    /// A notification's Review, or the panel row, can open a fresh Monitor
    /// window whose list asks for the idle filter before the window has
    /// attached its client. The request survives that first attach and
    /// narrows the list it loads, in either order.
    func test_anIdleRequestBeforeTheFirstAttachNarrowsTheList() async throws {
        let client = SampleDaemonClient(.normalDay)
        let fresh = TracesStore(client: nil)
        await fresh.requestIdleOnly()
        // The daemon still starting: no client yet, the request waits.
        fresh.attach(nil, awaiting: true)
        fresh.attach(client)
        await fresh.load()
        XCTAssertTrue(fresh.idleOnly, "the first attach cleared the requested filter")
        XCTAssertEqual(fresh.tree.allSessions.count, 2)

        let attachedFirst = TracesStore(client: nil)
        attachedFirst.attach(client)
        await attachedFirst.requestIdleOnly()
        XCTAssertTrue(attachedFirst.idleOnly)
        XCTAssertEqual(attachedFirst.tree.allSessions.count, 2)

        // Applied once: a later new client still starts unfiltered.
        fresh.attach(SampleDaemonClient(.normalDay))
        XCTAssertFalse(fresh.idleOnly)
    }

    /// From the panel row or a notification, Review opens Traces; the
    /// window then narrows the list (`consumePending`).
    func test_theIdleDestinationOpensTraces() {
        var tab = MonitorWindowView.Tab.home
        var page = HomeTabView.Page.overview
        var selection: MonitorSelection?
        var inspector = false
        MonitorWindowView.land(.idleSessions, tab: &tab, homePage: &page, selection: &selection, showsInspector: &inspector)
        XCTAssertEqual(tab, .traces)
        XCTAssertNil(selection)
    }

    /// The filtered list opens the folders it lists.
    func test_theIdleFilterOpensItsFolders() async throws {
        let client = SampleDaemonClient(.normalDay)
        let idle = try await client.listPending(projectId: nil, filter: .idleSessions, order: nil)
        let tree = TracesTree.build(
            entries: idle, projects: try await client.listProjects().projects, settings: nil,
            scansWhenUnset: [], onlyWithSessions: true)
        XCTAssertEqual(TracesTreeView.idleFolders(tree), Set(idle.map(\.projectId)))
    }

    // MARK: - Row tags

    /// The tags are the core's words for the row's own fields, kept by
    /// entry id; a row with nothing drawable has none.
    func test_rowTagsAreTheCoresWordsForEachRow() throws {
        let rows = try entries(#"""
            {"pending":[
             {"entry_id":"fit","source":"codex","project_id":"p","project_label":"P","state":"pending","mission_fit":2,
              "credit_estimate":{"low":2.0,"high":4.5,"tier":"higher","calibration":"c","basis":"published","drawn":true}},
             {"entry_id":"hidden","source":"codex","project_id":"p","project_label":"P","state":"pending","mission_fit":0,
              "credit_estimate":{"low":1.0,"high":3.0,"calibration":"c","basis":"built_in","drawn":false}},
             {"entry_id":"plain","source":"codex","project_id":"p","project_label":"P","state":"pending"}]}
            """#)
        let tags = TracesStore.rowTags(rows)
        XCTAssertEqual(Set(tags.keys), ["fit"])
        XCTAssertEqual(tags["fit"]?.missionFit, "Fits a mission")
        XCTAssertEqual(tags["fit"]?.estimateBand, "Estimate: about 2 to 4.5 credit")
        XCTAssertEqual(tags["fit"]?.estimateTier, "Higher estimate")
    }

    /// The order control's and filter's words are the core's.
    func test_theListControlsAreTheCoresWords() throws {
        let copy = try XCTUnwrap(TracesStore(client: nil).nudgeCopy)
        XCTAssertEqual(copy[.listOrderSuggested], "Suggested first")
        XCTAssertEqual(copy[.listOrderQueue], "Oldest first")
        XCTAssertNotNil(copy[.listFilterIdle])
        XCTAssertNotNil(copy[.listFilterClear])
    }
}
