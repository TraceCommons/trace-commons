#if DEBUG
import TCDesign
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The monitor shell after #1146's `monitor-shell.tsx` (Phase B1 on #1241):
/// the View menu's ignored folders, the Traces graph footer, the inspector
/// opening on demand, the shell's breadcrumb, and Home's three counts.
@MainActor
final class MonitorShellTests: XCTestCase {
    // MARK: Show ignored folders

    /// Hidden by default; the View menu's choice draws them, and their
    /// sessions with them.
    func test_ignoredFoldersShowOnlyWhenAskedFor() throws {
        let project = ProjectRow(projectId: "p1", projectLabel: "quiet", projectPath: "/x", mode: .ignore)
        let hidden = TracesTree.build(entries: [], projects: [project], settings: nil, scansWhenUnset: [])
        XCTAssertTrue(hidden.unplaced.isEmpty)
        let shown = TracesTree.build(entries: [], projects: [project], settings: nil, scansWhenUnset: [], showsIgnored: true)
        XCTAssertEqual(shown.unplaced.map(\.label), ["quiet"])
        XCTAssertEqual(shown.unplaced.first?.mode, .ignore)
        XCTAssertFalse(TracesStore(client: nil).showsIgnored, "ignored folders are hidden by default")
    }

    /// The store redraws from its last read when the choice changes,
    /// without asking the core again.
    func test_theStoreRedrawsWhenTheChoiceChanges() async throws {
        let store = TracesStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        let before = store.tree
        store.showsIgnored = true
        let ignored = try await SampleDaemonClient(.normalDay).listProjects().projects.filter { $0.mode == .ignore }
        let drawn = Set((store.tree.tools.flatMap(\.folders) + store.tree.unplaced).map(\.id))
        for folder in ignored { XCTAssertTrue(drawn.contains(folder.projectId), folder.projectLabel) }
        store.showsIgnored = false
        XCTAssertEqual(store.tree, before)
    }

    func test_theViewMenuIsInTheToolbar() throws {
        let window = try Self.text("Views/MonitorWindowView.swift")
        for needle in ["GlassToolbarButton(MonitorShellWords.view, systemImage: \"line.3.horizontal\", expanded: viewMenu)",
                       "GlassMenuItem(MonitorShellWords.showIgnoredFolders, checked: showsIgnored)",
                       "GlassToolbarButton(MonitorShellWords.graphToggle(shown: showsGraph),",
                       "GlassToolbarButton(MonitorShellWords.mapToggle(shown: showsMap),",
                       "GlassToolbarButton(MonitorShellWords.inspectorToggle(shown: showsInspector),",
                       "@SceneStorage(\"monitor.showsIgnored\") private var showsIgnored = false",
                       ".onChange(of: showsIgnored) { _, shows in traces.showsIgnored = shows }"] {
            XCTAssertTrue(window.contains(needle), "MonitorWindowView.swift lacks \(needle)")
        }
    }

    // MARK: Map focus

    func test_focusingTheMapKeepsOnlyThatTool() async throws {
        let client = SampleDaemonClient(.normalDay)
        let tree = TracesTree.build(
            entries: try await client.listPending(projectId: nil), projects: try await client.listProjects().projects,
            settings: try? await client.settings(), scansWhenUnset: [.claudeCode, .codex])
        let tool = try XCTUnwrap(tree.tools.first)
        let focused = tree.focused(on: tool.id)
        XCTAssertEqual(focused.tools.map(\.id), [tool.id])
        XCTAssertTrue(focused.unplaced.isEmpty)
        XCTAssertEqual(tree.focused(on: nil), tree)
        XCTAssertEqual(tree.focused(on: "not-a-tool"), tree)
    }

    // MARK: Graph

    func test_theGraphBucketsCountPerDay() {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        let now = Date(timeIntervalSince1970: 1_760_000_000) // a fixed instant
        let today = calendar.startOfDay(for: now)
        let day: TimeInterval = 86_400
        let shared = [today.addingTimeInterval(60), today.addingTimeInterval(-day + 60), today.addingTimeInterval(-20 * day)]
        let kept = [today.addingTimeInterval(120), today.addingTimeInterval(120)]
        let buckets = TracesGraphModel.buckets(shared: shared, kept: kept, offset: 0, now: now, calendar: calendar)
        XCTAssertEqual(buckets.count, TracesGraphModel.days)
        XCTAssertEqual(buckets.last?.start, today)
        XCTAssertEqual(buckets.last?.shared, 1)
        XCTAssertEqual(buckets.last?.kept, 2)
        XCTAssertEqual(buckets[buckets.count - 2].shared, 1)
        XCTAssertEqual(buckets.map(\.shared).reduce(0, +), 2, "twenty days back is outside the window")
        let earlier = TracesGraphModel.buckets(shared: shared, kept: kept, offset: -1, now: now, calendar: calendar)
        XCTAssertEqual(earlier.last?.start, calendar.date(byAdding: .day, value: -11, to: today))
        XCTAssertEqual(earlier.map(\.shared).reduce(0, +), 1)
    }

    /// Shared is what left and still stands: withdrawn is not counted.
    func test_sharedCountsContributionsThatStand() {
        XCTAssertEqual(TracesGraphModel.contributedStatuses, ["accepted", "submitted", "quarantined"])
        XCTAssertEqual(TracesGraphFooter.figure(3, known: true), "3")
        XCTAssertEqual(TracesGraphFooter.figure(0, known: false), "—", "an unread History is not nothing shared")
    }

    // MARK: Inspector demand

    func test_somethingNewOpensTheInspector() {
        let none = InspectorDemand.keys(approvalUndo: nil, contributed: nil, kept: nil, contributedFolder: nil,
                                        submittingFolder: nil, privateAIOffer: false, armingOffer: nil)
        XCTAssertTrue(none.isEmpty)
        let submitting = InspectorDemand.keys(approvalUndo: nil, contributed: nil, kept: nil, contributedFolder: nil,
                                              submittingFolder: "p1", privateAIOffer: false, armingOffer: nil)
        XCTAssertTrue(InspectorDemand.opens(previous: none, current: submitting))
        let undo = InspectorDemand.keys(approvalUndo: nil, contributed: nil, kept: nil, contributedFolder: "p1",
                                        submittingFolder: nil, privateAIOffer: false, armingOffer: nil)
        XCTAssertTrue(InspectorDemand.opens(previous: submitting, current: undo))
        let offers = InspectorDemand.keys(approvalUndo: ["e1"], contributed: "e1", kept: "e2", contributedFolder: nil,
                                          submittingFolder: nil, privateAIOffer: true, armingOffer: "p2")
        XCTAssertEqual(offers.count, 5)
        XCTAssertTrue(InspectorDemand.opens(previous: none, current: offers))
        // Going away, or still asking, leaves the person's choice alone.
        XCTAssertFalse(InspectorDemand.opens(previous: offers, current: none))
        XCTAssertFalse(InspectorDemand.opens(previous: offers, current: offers))
    }

    func test_theWindowOpensTheInspectorOnDemand() throws {
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains(
            "if InspectorDemand.opens(previous: lastDemand, current: current) { showsInspector = true }"))
        // The window's keys are the port's `keys(model:traces:selection:)`
        // (#1241), which reads the shell's inputs and adds the selected
        // session and the approval's time.
        XCTAssertTrue(window.contains("InspectorDemand.keys(model: model, traces: traces, selection: selection)"))
        let demand = try Self.text("Views/Monitor/TracesInspectorHost.swift")
        for input in ["model.undo?.entryIDs", "traces.lastContributed?.entryId", "traces.lastKept",
                      "traces.lastContributedFolder?.projectId", "traces.submittingFolder",
                      "model.showsPrivateInferenceOffer", "model.armingOffer?.projectId"] {
            XCTAssertTrue(demand.contains(input), "the demand does not read \(input)")
        }
    }

    // MARK: Breadcrumb

    /// The shell draws Home's trail under the tabs; the pages do not.
    func test_theBreadcrumbIsTheShells() throws {
        XCTAssertNil(MonitorWindowView.breadcrumb(tab: .home, homePage: .overview, back: {}))
        XCTAssertNil(MonitorWindowView.breadcrumb(tab: .traces, homePage: .history, back: {}))
        var backs = 0
        let history = try XCTUnwrap(MonitorWindowView.breadcrumb(tab: .home, homePage: .history, back: { backs += 1 }))
        XCTAssertEqual(history.map(\.title), [MonitorWindowView.Tab.home.title, MonitorWords.history])
        history.first?.action?()
        XCTAssertEqual(backs, 1)
        XCTAssertNil(history.last?.action, "the current page is not a link")
        let missions = try XCTUnwrap(MonitorWindowView.breadcrumb(tab: .home, homePage: .missions, back: {}))
        XCTAssertEqual(missions.map(\.title), [MonitorWindowView.Tab.home.title, MonitorWords.missions])
        for page in ["Views/Monitor/HomeViews.swift", "Views/Monitor/MissionsViews.swift"] {
            XCTAssertFalse(try Self.text(page).contains("GlassBreadcrumb("), "\(page) draws its own breadcrumb")
        }
    }

    // MARK: Home

    /// Pending credit is a figure only with the commons' statement of its
    /// condition (D6); a dash otherwise.
    func test_pendingCreditNeedsItsCondition() {
        XCTAssertEqual(HomeFormat.pendingFigure(12.5, condition: nil), "—")
        XCTAssertEqual(HomeFormat.pendingFigure(12.5, condition: "Settles monthly"), HomeFormat.points(12.5))
        XCTAssertEqual(HomeFormat.pendingFigure(nil, condition: "Settles monthly"), "—")
    }

    func test_homeLinksToTracesAndCountsThree() throws {
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        for needle in ["Button(action: openTraces)", "HomeStatTile(label: MonitorWords.waiting,",
                       "HomeStatTile(label: MonitorWords.contributed,", "HomeStatTile(label: HomeFormat.creditPendingWord,",
                       "Text(MissionFormat.count(store.missions))"] {
            XCTAssertTrue(home.contains(needle), "HomeViews.swift lacks \(needle)")
        }
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("openTraces: { tab = .traces },"))
    }

    // MARK: Inference

    func test_theInferenceInspectorSummarises() {
        XCTAssertEqual(PrivateAIInspectorView.connectedCount(nil), "—")
        XCTAssertEqual(PrivateAIInspectorView.names([]), "—")
    }

    /// #1146's "{n} of {m} tools connected", from the core; nothing when
    /// the list was not read.
    func test_theInferenceInspectorSaysHowManyToolsAreConnected() {
        XCTAssertNil(PrivateAIInspectorView.connectedLine(nil))
        XCTAssertEqual(PrivateAIInspectorView.connectedLine([]), "0 of 0 tools connected")
    }

    private static func text(_ path: String) throws -> String {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp").appendingPathComponent(path)
        return try String(contentsOf: url, encoding: .utf8)
    }
}
#endif
