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

    /// Shown by default, as #1146's `useState(true)`; the View menu's
    /// choice hides them, and their sessions with them.
    func test_ignoredFoldersShowUnlessHidden() throws {
        let project = ProjectRow(projectId: "p1", projectLabel: "quiet", projectPath: "/x", mode: .ignore)
        let hidden = TracesTree.build(entries: [], projects: [project], settings: nil, scansWhenUnset: [])
        XCTAssertTrue(hidden.unplaced.isEmpty)
        let shown = TracesTree.build(entries: [], projects: [project], settings: nil, scansWhenUnset: [], showsIgnored: true)
        XCTAssertEqual(shown.unplaced.map(\.label), ["quiet"])
        XCTAssertEqual(shown.unplaced.first?.mode, .ignore)
        XCTAssertTrue(TracesStore(client: nil).showsIgnored, "ignored folders are shown by default (#1146)")
    }

    /// The store redraws from its last read when the choice changes,
    /// without asking the core again.
    func test_theStoreRedrawsWhenTheChoiceChanges() async throws {
        let store = TracesStore(client: SampleDaemonClient(.normalDay))
        store.showsIgnored = false
        await store.load()
        let before = store.tree
        store.showsIgnored = true
        let ignored = try await SampleDaemonClient(.normalDay).listProjects().projects.filter { $0.mode == .ignore }
        let drawn = Set((store.tree.tools.flatMap(\.folders) + store.tree.unplaced).map(\.id))
        for folder in ignored { XCTAssertTrue(drawn.contains(folder.projectId), folder.projectLabel) }
        store.showsIgnored = false
        XCTAssertEqual(store.tree, before)
    }

    /// Every folder starts closed, as #1146's tree does: only the folder
    /// rows are drawn until one is opened.
    func test_foldersStartClosed() async throws {
        let store = TracesStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        let tree = store.tree
        XCTAssertFalse(tree.folders.isEmpty)
        XCTAssertEqual(TracesTreeView.visibleRows(in: tree, expanded: []),
                       tree.folders.map { .folder(projectID: $0.id) })
        let first = try XCTUnwrap(tree.folders.first)
        XCTAssertTrue(TracesTreeView.visibleRows(in: tree, expanded: [first.id]).count
                      >= TracesTreeView.visibleRows(in: tree, expanded: []).count)
        let view = try Self.text("Views/Monitor/TracesViews.swift")
        XCTAssertTrue(view.contains("@State private var expanded: Set<String> = []"))
    }

    /// The main pane takes #1146's insets: the toolbar row and the tabs 12
    /// in, the Traces tree 8 in and to the bottom, the graph full bleed
    /// under a 0.5pt rule; the inspector 16 by 18; the map's field, edge
    /// and 14pt overlay inset; the View menu 250 wide.
    func test_theShellTakesTheReferenceLayout() throws {
        let window = try Self.text("Views/MonitorWindowView.swift")
        for needle in [
            "GlassPane(padding: 0) {",
            ".padding([.horizontal, .top], GlassTokens.Space.panePadding)",
            ".padding(Self.contentInsets(shown))",
            "EdgeInsets(top: 0, leading: GlassTokens.Space.treeInset, bottom: 0, trailing: GlassTokens.Space.treeInset)",
            "Rectangle().fill(GlassTokens.Color.rule.color).frame(height: 0.5)",
            ".transition(.move(edge: .bottom).combined(with: .opacity))",
            "GlassPane(insets: GlassPaneInsets.inspector) {",
            "GlassPane(padding: 0, isContent: true, edge: GlassTokens.Shadow.mapEdge) {",
            "GlassMapField()",
            ".padding(GlassTokens.Space.mapOverlayInset)",
            ".frame(width: Self.viewMenuWidth)",
            "static var viewMenuWidth: CGFloat { 250 }",
        ] {
            XCTAssertTrue(window.contains(needle), "MonitorWindowView.swift lacks \(needle)")
        }
        XCTAssertFalse(window.contains("RadialGradient("), "the map field is #1146's ellipse")
        let footer = try Self.text("Views/Monitor/MonitorShell.swift")
        XCTAssertFalse(footer.contains("GlassColor.hairline"), "the graph's rule is the shell's, at #1146's 0.12")
    }

    func test_theViewMenuIsInTheToolbar() throws {
        let window = try Self.text("Views/MonitorWindowView.swift")
        for needle in ["GlassToolbarButton(MonitorShellWords.view, icon: .glyph(.viewMenu), expanded: viewMenu)",
                       "GlassMenuItem(MonitorShellWords.showIgnoredFolders, checked: showsIgnored)",
                       "GlassToolbarButton(MonitorShellWords.graphToggle(shown: showsGraph),",
                       "GlassToolbarButton(MonitorShellWords.mapToggle(shown: showsMap),",
                       "GlassToolbarButton(MonitorShellWords.inspectorToggle(shown: showsInspector),",
                       "@SceneStorage(\"monitor.showsIgnored\") private var showsIgnored = true",
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

    /// #1146's three ranges: 24 hours in 2-hour buckets, 11 days, 42 days,
    /// each labelled as #1146 labels it, and zooming steps between them.
    func test_theGraphZoomsBetweenThreeRanges() throws {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = try XCTUnwrap(TimeZone(identifier: "UTC"))
        let now = Date(timeIntervalSince1970: 1_760_000_000) // 2025-10-09 08:53 UTC
        let hour = try XCTUnwrap(calendar.dateInterval(of: .hour, for: now)?.start)

        let hours = TracesGraphModel.buckets(shared: [now], kept: [], offset: 0, now: now, range: .hours, calendar: calendar)
        XCTAssertEqual(hours.count, 12)
        XCTAssertEqual(hours.last?.start, hour)
        XCTAssertEqual(hours.last?.shared, 1)
        XCTAssertEqual(hours.first?.label, "\(calendar.component(.hour, from: try XCTUnwrap(hours.first?.start))):00")
        XCTAssertEqual(hours[1].label, "", "every third hour bucket is labelled")
        XCTAssertEqual(hours.first.map { calendar.dateComponents([.hour], from: $0.start, to: hour).hour }, 22)

        let days = TracesGraphModel.buckets(shared: [], kept: [], offset: 0, now: now, calendar: calendar)
        XCTAssertEqual(days.count, 11)
        XCTAssertTrue(days.allSatisfy { !$0.label.isEmpty }, "every day has its weekday")
        XCTAssertEqual(days.last?.label, TracesGraphModel.weekday(now, calendar: calendar))

        let weeks = TracesGraphModel.buckets(shared: [], kept: [], offset: -1, now: now, range: .weeks, calendar: calendar)
        XCTAssertEqual(weeks.count, 42)
        XCTAssertEqual(weeks.filter { !$0.label.isEmpty }.count, 7, "every sixth day is labelled")
        XCTAssertEqual(weeks.last?.start, calendar.date(byAdding: .day, value: -42, to: calendar.startOfDay(for: now)))

        XCTAssertEqual(TracesGraphModel.Range.days.zoomedOut, .weeks)
        XCTAssertNil(TracesGraphModel.Range.weeks.zoomedOut)
        XCTAssertEqual(TracesGraphModel.Range.days.zoomedIn, .hours)
        XCTAssertNil(TracesGraphModel.Range.hours.zoomedIn)
        XCTAssertEqual(TracesGraphModel.Range.allCases.map(\.span), [24, 11, 42])
    }

    /// The range pill says the window in the core's words, or the hovered
    /// day in full; a bar says its day, then shared and kept.
    func test_theRangePillAndBarsUseTheCoresWords() throws {
        let words = try XCTUnwrap(MonitorWords.table?.tracesGraph)
        XCTAssertEqual(TracesGraphFooter.rangeLabel(range: .days, offset: 0, hovered: nil, words: words), "Last 11 days")
        XCTAssertEqual(TracesGraphFooter.rangeLabel(range: .hours, offset: 0, hovered: nil, words: words), "Last 24 hours")
        XCTAssertEqual(TracesGraphFooter.rangeLabel(range: .weeks, offset: -1, hovered: nil, words: words),
                       "42 days, 1 window back")
        XCTAssertEqual(TracesGraphFooter.rangeLabel(range: .days, offset: -3, hovered: nil, words: words),
                       "11 days, 3 windows back")
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = try XCTUnwrap(TimeZone(identifier: "UTC"))
        let date = Date(timeIntervalSince1970: 1_759_665_600)
        let day = TracesGraphModel.day(date, calendar: calendar)
        var style = Date.FormatStyle.dateTime.weekday(.wide).month(.wide).day()
        style.timeZone = calendar.timeZone
        XCTAssertEqual(day, date.formatted(style))
        XCTAssertEqual(words.bar(day: day, shared: 3, kept: 2), "\(day): 3 shared, 2 kept")
        XCTAssertEqual(TracesGraphFooter.figure(0, known: false), "—")
    }

    /// The binoculars' glyph: blue when focused, light when a tool can be
    /// focused, dim with nothing to focus on (#1146), never the purple CTA.
    func test_theFocusGlyphTakesItsInkFromItsState() throws {
        XCTAssertEqual(TracesGraphFooter.focusInk(focused: true, canFocus: true), GlassTokens.Color.graphFocusOn)
        XCTAssertEqual(TracesGraphFooter.focusInk(focused: false, canFocus: true), GlassTokens.Color.graphFocusIdle)
        XCTAssertEqual(TracesGraphFooter.focusInk(focused: true, canFocus: false), GlassTokens.Color.graphFocusOff)
        let shell = try Self.text("Views/Monitor/MonitorShell.swift")
        XCTAssertFalse(shell.contains("GlassButtonStyle(.glass, small: true, selected:"), "the focus pill is not a CTA")
        // Six pills around the range, in #1146's order.
        let order = ["MonitorShellWords.previous, glyph:", "systemImage: \"binoculars\"", "words?.zoomOut",
                     ".glassEdge(Self.rangeEdge", "words?.zoomIn", "MonitorShellWords.next, glyph:", "words?.jumpToNow"]
        var at = shell.startIndex
        for needle in order {
            let found = try XCTUnwrap(shell.range(of: needle, range: at..<shell.endIndex), needle)
            at = found.upperBound
        }
        XCTAssertEqual(TracesGraphFooter.height, 206)
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
        // The commons catalogue says so; #1146's Missions is the drafts.
        let missions = try XCTUnwrap(MonitorWindowView.breadcrumb(tab: .home, homePage: .missions, back: {}))
        XCTAssertEqual(missions.map(\.title),
                       [MonitorWindowView.Tab.home.title, try XCTUnwrap(MonitorWords.table).homeHistory.missionCatalogue])
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
                       // Ron's Missions card: his Drafts tag over the drafts.
                       "GlassEyebrowCard(MonitorWords.missions, action: openMissionDrafts)",
                       "GlassTag(words.draftsTag, tone: .ask)", "Text(words.noMissionDrafts)",
                       "Text(words.sources(draft.source_count))"] {
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
