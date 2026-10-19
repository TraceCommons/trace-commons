import CoreGraphics
import Foundation
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The Traces map's coin pile and what moves across it (owner,
/// 2026-10-16): each folder once with one line into the tool group, one
/// line out to the pad, and a run of transfers that is a function of time
/// alone, found only in what the core reported.
@MainActor
final class TracesMapPileTests: XCTestCase {
    private func folder(_ id: String, mode: ProjectMode? = .ask, sessions: [DaemonData.QueueEntry] = [])
        -> TracesTree.FolderNode
    {
        TracesTree.FolderNode(id: id, label: id, mode: mode, sessions: sessions)
    }

    private func entry(_ id: Int, _ project: String, source: String) throws -> DaemonData.QueueEntry {
        try recordedEntry([
            "entry_id": String(format: "00000000-0000-4000-8000-%012d", id),
            "project_id": project, "project_label": project, "source": source,
        ])
    }

    private func history(_ rows: [(id: String, project: String?, source: String?, sent: Date?)]) throws
        -> [DaemonData.HistoryRow]
    {
        let reply = try XCTUnwrap(SampleDaemonData.reply("list_history", in: .normalDay))
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(reply.utf8)) as? [String: Any])
        let first = try XCTUnwrap((object["history"] as? [[String: Any]])?.first)
        let format = ISO8601DateFormatter()
        return try rows.map { row in
            var fields = first
            fields["submission_id"] = row.id
            fields["session_hash"] = "sha256:" + row.id
            fields["project_id"] = row.project ?? NSNull()
            fields["source"] = row.source ?? NSNull()
            fields["submitted_at"] = row.sent.map { format.string(from: $0) } ?? NSNull()
            return try DaemonDataDecoding.decoder().decode(
                DaemonData.HistoryRow.self, from: JSONSerialization.data(withJSONObject: fields))
        }
    }

    // MARK: The pile

    /// The card's lines meet it at the middle of each side; closed, the
    /// coins sit nine apart, open 34, centred on the card either way.
    func test_thePileIsCentredAndOpensVertically() {
        let stack = FlowMapScene.ToolStack(center: CGPoint(x: 200, y: 300), tools: ["tool:a", "tool:b", "tool:c"])
        typealias Stack = FlowMapScene.ToolStack
        XCTAssertEqual(stack.portLeft, CGPoint(x: 200 - Stack.width / 2, y: 300))
        XCTAssertEqual(stack.portRight, CGPoint(x: 200 + Stack.width / 2, y: 300))
        XCTAssertEqual(stack.coinCenter(1, open: false), stack.center)
        XCTAssertEqual(stack.coinCenter(2, open: false).y - stack.coinCenter(1, open: false).y, Stack.step)
        XCTAssertEqual(stack.coinCenter(2, open: true).y - stack.coinCenter(1, open: true).y, Stack.openStep)
        XCTAssertEqual(stack.coinCenter(0, open: true).x, stack.center.x)
        XCTAssertGreaterThan(stack.height(open: true), stack.height(open: false))
        XCTAssertEqual(stack.height(open: false), 2 * Stack.step + Stack.coin + 2 * Stack.padding)
    }

    /// A folder two tools share is drawn once, with one line into the
    /// pile; every folder line ends at the card's left port, and one line
    /// leaves the right port for the pad's left edge.
    func test_eachFolderIsDrawnOnceWithOneLine() {
        let shared = folder("shared")
        let tree = TracesTree(tools: [
            .init(kind: .claudeCode, mode: .watch, folders: [shared, folder("a")]),
            .init(kind: .codex, mode: .watch, folders: [shared]),
        ], unplaced: [folder("loose")])
        let scene = FlowMapScene.traces(tree, gate: .init(state: .ready))
        let stack = try? XCTUnwrap(scene.stack)
        let folders = scene.nodes.filter { $0.id.hasPrefix("folder:") }.map(\.id)
        XCTAssertEqual(folders, ["folder:shared", "folder:a", "folder:loose"])
        XCTAssertEqual(scene.arcs.filter { $0.to == stack?.portLeft }.count, folders.count)
        let hub = try? XCTUnwrap(scene.nodes.first { $0.id == "hub" }?.at)
        let out = scene.arcs.filter { $0.from == stack?.portRight }
        XCTAssertEqual(out.count, 1)
        XCTAssertEqual(out.first?.to, hub.map { CGPoint(x: $0.x - FlowMapScene.padHalfWidth, y: $0.y) })
        XCTAssertEqual(scene.arcs.count, folders.count + 1, "nothing else is drawn as a line")
        XCTAssertEqual(FlowMapScene.carriers(of: shared, in: tree).map(\.id), ["claude-code", "codex"])
    }

    /// A folder's traces name their tool even where the tree places the
    /// folder under another, and the coins carry what waits for each.
    func test_aFoldersCarriersIncludeItsTracesSources() throws {
        let mixed = folder("mixed", sessions: [try entry(1, "mixed", source: "codex")])
        let tree = TracesTree(tools: [
            .init(kind: .claudeCode, mode: .watch, folders: [mixed]),
            .init(kind: .codex, mode: .watch, folders: []),
        ], unplaced: [])
        XCTAssertEqual(FlowMapScene.carriers(of: mixed, in: tree).map(\.id), ["claude-code", "codex"])
        let scene = FlowMapScene.traces(tree, gate: .init(state: .ready))
        XCTAssertEqual(scene.nodes.first { $0.id == "tool:claude-code" }?.waiting, 1)
        XCTAssertEqual(scene.nodes.first { $0.id == "tool:codex" }?.waiting, 0)
    }

    // MARK: A run

    private func run(_ count: Int, _ action: MapTransfer.Action = .contribute) -> MapTransferRun {
        let tools = ["claude-code", "codex", "opencode"]
        return MapTransferRun(transfers: (0 ..< count).map {
            MapTransfer(id: "t\($0)", folder: "f\($0)", tool: tools[$0 % tools.count], action: action)
        }, start: .now)
    }

    /// Each trace rides in, goes round its coin, rides out and arrives, a
    /// step after the one before.
    func test_aTracesLegsFollowOneAnother() {
        let run = run(2)
        XCTAssertEqual(run.leg(0, at: 0), .inbound(0))
        XCTAssertEqual(run.leg(1, at: 0), .waiting)
        guard case .orbit(let round) = run.leg(0, at: MapTransferRun.inbound + MapTransferRun.orbit / 2) else {
            return XCTFail("the trace goes round its coin")
        }
        XCTAssertEqual(round, 0.5, accuracy: 0.0001)
        guard case .outbound = run.leg(0, at: MapTransferRun.inbound + MapTransferRun.orbit + 0.1) else {
            return XCTFail("the trace leaves its coin")
        }
        guard case .inbound = run.leg(1, at: MapTransferRun.step + 0.1) else { return XCTFail("the second sets off") }
        XCTAssertEqual(run.arrived(at: run.allArrived), 2)
        XCTAssertEqual(run.arrived(at: run.allArrived - 0.01), 1)
        XCTAssertFalse(run.moving(at: run.allArrived))
    }

    /// The rocket waits on the pad until every trace has arrived, then
    /// sits through ignition and climbs slowly, then faster.
    func test_theRocketWaitsForEveryTraceThenClimbsFaster() {
        let run = run(3)
        XCTAssertNil(run.flight(at: run.allArrived))
        XCTAssertNil(run.flight(at: run.liftOff - 0.01))
        XCTAssertEqual(run.flight(at: run.liftOff), 0)
        XCTAssertEqual(run.flight(at: run.liftOff + MapTransferRun.flight * 2), 1)
        XCTAssertEqual(MapTransferRun.climb(0.2), 0, "still through ignition")
        XCTAssertEqual(MapTransferRun.climb(1), 1, accuracy: 0.0001)
        let steps = stride(from: 0.3, through: 1, by: 0.1).map(MapTransferRun.climb)
        let gains = zip(steps, steps.dropFirst()).map { $1 - $0 }
        XCTAssertTrue(gains.allSatisfy { $0 > 0 })
        XCTAssertTrue(zip(gains, gains.dropFirst()).allSatisfy { $1 > $0 }, "it speeds up")
        XCTAssertEqual(run.end, run.liftOff + MapTransferRun.flight + MapTransferRun.linger, accuracy: 0.0001)
        // A kept trace has no rocket.
        let kept = self.run(1, .keep)
        XCTAssertNil(kept.flight(at: kept.end))
        XCTAssertEqual(kept.end, kept.allArrived + MapTransferRun.keptLinger, accuracy: 0.0001)
    }

    /// A trace's tool moves to the front of the pile as it sets off; its
    /// coin is grown only while the rim light goes round.
    func test_aTracesCoinShufflesToTheFrontAndGrows() {
        let run = run(2)
        let base = ["tool:opencode", "tool:claude-code", "tool:codex"]
        XCTAssertEqual(run.order(base, at: -1), base)
        XCTAssertEqual(run.order(base, at: 0).first, "tool:claude-code")
        XCTAssertEqual(run.order(base, at: MapTransferRun.step).first, "tool:codex")
        XCTAssertEqual(run.orbits(at: 0), [:])
        XCTAssertEqual(run.orbits(at: MapTransferRun.inbound).keys.sorted(), ["tool:claude-code"])
        XCTAssertEqual(run.current(at: 0)?.id, "t0")
        XCTAssertEqual(run.current(at: MapTransferRun.step + 0.1)?.id, "t1")
        XCTAssertNil(run.current(at: run.allArrived))
    }

    /// On the standard map each contributed trace rises from the laptop to
    /// the orb on its own: no rocket, no gathering, and a kept trace still
    /// ends at this computer.
    func test_theStandardMapRaisesEachTraceToTheOrb() {
        var run = run(2)
        run.style = .standard
        XCTAssertTrue(run.rises)
        let rising = MapTransferRun.inbound + MapTransferRun.orbit + MapTransferRun.outbound + 0.1
        guard case .ascent = run.leg(0, at: rising) else { return XCTFail("the trace rises from the laptop") }
        XCTAssertEqual(run.allArrived, MapTransferRun.step + rising - 0.1 + MapTransferRun.ascent, accuracy: 0.0001)
        XCTAssertNil(run.flight(at: run.end), "the standard map has no rocket")
        XCTAssertEqual(run.end, run.allArrived + MapTransferRun.linger, accuracy: 0.0001)
        var kept = self.run(1, .keep)
        kept.style = .standard
        XCTAssertFalse(kept.rises)
        guard case .arrived = kept.leg(0, at: rising) else { return XCTFail("a kept trace ends at this computer") }
    }

    /// The library sits straight above this computer in every pane shape,
    /// clear of the pad's tower and the rocket on it.
    func test_theLibraryStaysAboveThisComputer() {
        for pane in FlowMapLoadTests.panes + [CGSize(width: 1200, height: 600)] {
            let space = FlowMapScene.space(for: pane)
            let hub = FlowMapScene.hub(in: space)
            let library = FlowMapScene.library(in: space)
            XCTAssertEqual(library.x, hub.x, "\(pane)")
            let stationFoot = library.y + FlowMapView.stationSize.height / 2
            XCTAssertLessThan(stationFoot, hub.y - 55, "the station meets the pad's tower at \(pane)")
            XCTAssertLessThan(library.y + MapTransferDrawing.orbRadius, hub.y - FlowMapView.laptopScreenTop, "\(pane)")
        }
    }

    /// The trace's details show over its folder's dot or its line while it
    /// moves, and nowhere else on the map.
    func test_theTracesFolderAndLineShowItsDetails() throws {
        let tree = TracesTree(tools: [.init(kind: .claudeCode, mode: .watch, folders: [folder("a"), folder("b")])],
                              unplaced: [])
        let scene = FlowMapScene.traces(tree, gate: .init(state: .ready))
        let fit = FlowMapGeometry(size: FlowMapScene.size, zoom: 1, focus: nil, space: scene.space)
        let run = MapTransferRun(transfers: [MapTransfer(id: "1", folder: "a", tool: "claude-code", action: .contribute)],
                                 start: .now)
        let t = 0.2
        let dot = try XCTUnwrap(scene.nodes.first { $0.id == "folder:a" }).at
        let arc = try XCTUnwrap(MapTransferDrawing.folderArc("a", in: scene))
        let onLine = fit.point(MapTransferDrawing.point(on: arc, at: 0.5))
        let other = fit.point(try XCTUnwrap(scene.nodes.first { $0.id == "folder:b" }).at)
        XCTAssertTrue(FlowMapView.pointer(fit.point(dot), onTraceOf: run, at: t, in: scene, fit: fit))
        XCTAssertTrue(FlowMapView.pointer(CGPoint(x: onLine.x, y: onLine.y + 3), onTraceOf: run, at: t, in: scene, fit: fit))
        XCTAssertFalse(FlowMapView.pointer(other, onTraceOf: run, at: t, in: scene, fit: fit), "another folder")
        XCTAssertFalse(FlowMapView.pointer(CGPoint(x: 5, y: 5), onTraceOf: run, at: t, in: scene, fit: fit))
        XCTAssertFalse(FlowMapView.pointer(fit.point(dot), onTraceOf: run, at: run.end, in: scene, fit: fit),
                       "nothing is moving")
        XCTAssertFalse(FlowMapView.pointer(nil, onTraceOf: run, at: t, in: scene, fit: fit))
        XCTAssertEqual(FlowMapView.distance(from: CGPoint(x: 5, y: 4), toSegment: .zero, CGPoint(x: 10, y: 0)), 4)
    }

    /// The details card covers the pile and runs right of it.
    func test_theDetailsCardSitsOverThePile() {
        let left = CGPoint(x: 100, y: 200)
        let area = MapToolPile.detailsArea(left: left, height: 80)
        XCTAssertEqual(area.minX, left.x - MapToolPile.detailsLead)
        XCTAssertEqual(area.width, MapToolPile.detailsWidth)
        XCTAssertEqual(area.midY, left.y)
        XCTAssertGreaterThanOrEqual(area.height, 214)
    }

    /// The rocket is the default; turning it off is kept as the standard
    /// map, and the window and Settings read the same key.
    func test_theMapStyleIsAShellPreference() throws {
        let suite = "TracesMapPileTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        XCTAssertEqual(TracesMapStyle.stored(defaults), .launch)
        TracesMapStyle.set(.standard, defaults)
        XCTAssertEqual(TracesMapStyle.stored(defaults), .standard)
        TracesMapStyle.set(.launch, defaults)
        XCTAssertEqual(TracesMapStyle.stored(defaults), .launch)
        let window = try TracesInspectorHostTests.text("Views/MonitorWindowView.swift")
        let settings = try TracesInspectorHostTests.text("Views/Settings/GeneralSection.swift")
        for source in [window, settings] {
            XCTAssertTrue(source.contains("@AppStorage(TracesMapStyle.key)"))
        }
        XCTAssertTrue(window.contains("mapStyle: mapStyle"))
        XCTAssertTrue(settings.contains("words.styleToggle"))
    }

    // MARK: Finding transfers

    /// A row History gained, sent after everything the last read held, is
    /// a contribution; an unread first page, a page further back, and a
    /// row with no folder or tool are not. They play in the order sent.
    func test_contributionsAreRowsHistoryGained() throws {
        let t0 = Date(timeIntervalSince1970: 1_800_000_000)
        let old = try history([("a", "p1", "claude-code", t0)])
        let new = try history([
            ("c", "p2", "codex", t0 + 20), ("b", "p1", "claude-code", t0 + 10), ("a", "p1", "claude-code", t0),
            ("older", "p1", "claude-code", t0 - 100), ("nofolder", nil, "codex", t0 + 30),
            ("notool", "p1", nil, t0 + 30), ("unsent", "p1", "codex", nil),
        ])
        XCTAssertEqual(MapTransferDetector.contributions(from: nil, to: new), [], "the first read finds none")
        XCTAssertEqual(MapTransferDetector.contributions(from: old, to: nil), [])
        let found = MapTransferDetector.contributions(from: old, to: new)
        XCTAssertEqual(found.map(\.id), ["b", "c"])
        XCTAssertEqual(found.map(\.folder), ["p1", "p2"])
        XCTAssertEqual(found.map(\.tool), ["claude-code", "codex"])
        XCTAssertTrue(found.allSatisfy { $0.action == .contribute })
        // Nothing contributed before: every row is new.
        XCTAssertEqual(MapTransferDetector.contributions(from: [], to: old).map(\.id), ["a"])
    }

    /// The window's last keep is found among the traces it was listed
    /// with; one that is not there plays nothing.
    func test_aKeepIsFoundAmongTheListedTraces() throws {
        let listed = [try entry(1, "p1", source: "codex"), try entry(2, "p2", source: "claude-code")]
        let kept = try XCTUnwrap(MapTransferDetector.kept(listed[1].entryId, in: listed))
        XCTAssertEqual(kept.folder, "p2")
        XCTAssertEqual(kept.tool, "claude-code")
        XCTAssertEqual(kept.action, .keep)
        XCTAssertNil(MapTransferDetector.kept("missing", in: listed))
        XCTAssertNil(MapTransferDetector.kept(nil, in: listed))
    }

    /// The player plays one action at a time, never the same trace twice,
    /// and nothing under Reduce Motion.
    func test_thePlayerBatchesByActionAndSkipsUnderReduceMotion() {
        let player = MapTransferPlayer()
        let one = MapTransfer(id: "1", folder: "f", tool: "codex", action: .contribute)
        let two = MapTransfer(id: "2", folder: "f", tool: "codex", action: .contribute)
        let keep = MapTransfer(id: "3", folder: "f", tool: "codex", action: .keep)
        player.enqueue([one, two, keep])
        XCTAssertEqual(player.run?.transfers.map(\.id), ["1", "2"])
        player.enqueue([one])
        XCTAssertEqual(player.run?.transfers.map(\.id), ["1", "2"], "a trace plays once")

        let still = MapTransferPlayer()
        still.reduceMotion = true
        still.enqueue([one])
        XCTAssertNil(still.run)
    }

    // MARK: Words

    /// The pile's and the details card's words come from the core and are
    /// filled here; a tool's memory files are named only for the tools the
    /// core names them for.
    func test_thePilesWordsAreTheCores() throws {
        let words = try XCTUnwrap(FlowMapScene.words)
        XCTAssertEqual(words.coinWaiting(3), "3 waiting")
        XCTAssertEqual(words.collected(2, of: 3), "Collected 2 of 3")
        XCTAssertEqual(words.keptHere(1), "Kept on this Mac")
        XCTAssertEqual(words.keptHere(2), "2 kept on this Mac")
        XCTAssertTrue(words.transferNote(contributing: true, tool: "Codex").contains("Codex"))
        XCTAssertFalse(words.transferNote(contributing: false, tool: "Codex").contains("{"))
        XCTAssertEqual(words.memoryFiles("claude-code"), "CLAUDE.md, memory/")
        XCTAssertEqual(words.memoryFiles("gemini-cli"), "GEMINI.md")
        XCTAssertNil(words.memoryFiles("something-new"))
        XCTAssertEqual(words.legendMemory, "Memory stays on this Mac")
        XCTAssertEqual(words.styleTitle, "Traces map")
        XCTAssertEqual(words.styleToggle, "Rocket and space station")
        XCTAssertFalse(words.styleCaption.isEmpty)
    }

    /// The window feeds the map what History gained and what it kept, and
    /// hands the run to the map.
    func test_theWindowFeedsTheMapItsTransfers() throws {
        let window = try TracesInspectorHostTests.text("Views/MonitorWindowView.swift")
        for needle in ["MapTransferDetector.contributions(from: old, to: new)",
                       "MapTransferDetector.kept(kept, in: traces.tree.allSessions + listedBefore)",
                       "transferRun: transfers.run"] {
            XCTAssertTrue(window.contains(needle), needle)
        }
    }
}
