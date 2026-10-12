import SwiftUI
import TCDesign
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// R8 of #1173: the flow map draws only what the core reports, against
/// C1's sample sets, and every node is something VoiceOver can inspect.
@MainActor
final class FlowMapSceneTests: XCTestCase {
    private func tree(_ set: SampleDaemonClient.SampleSet) async throws -> TracesTree {
        let client = SampleDaemonClient(set)
        return TracesTree.build(
            entries: try await client.listPending(projectId: nil),
            projects: try await client.listProjects().projects,
            settings: try? await client.settings(),
            scansWhenUnset: TracesTreeTests.scansWhenUnset)
    }

    /// The gate as the core reports it for `set`, with `edit` applied to
    /// the status's JSON object (the K2 recordings, edited, never a sample
    /// set of this file's own).
    private func gate(
        _ set: SampleDaemonClient.SampleSet, state: ScreenState = .ready,
        edit: (inout [String: Any]) -> Void = { _ in }
    ) async throws -> FlowMapScene.CommonsGate {
        let client = SampleDaemonClient(set)
        let read = try await client.status()
        let wire = try XCTUnwrap(TracesStore.wire(read))
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(wire.utf8)) as? [String: Any])
        edit(&object)
        let status = try DaemonDataDecoding.decoder().decode(
            DaemonData.Status.self, from: JSONSerialization.data(withJSONObject: object))
        return .init(state: state, status: status, destinations: try await client.toolDestinations())
    }

    private func flowing(_ scene: FlowMapScene) -> Int {
        scene.arcs.filter { $0.style == .flowing }.count
    }

    // MARK: Traces

    /// One node per tool this build knows and per drawn folder, plus this
    /// computer and the commons; every node inside the design space. The
    /// tools the tree leaves out are drawn too, dashed as not watched
    /// (#1146 draws every tool).
    func test_theTracesSceneDrawsEveryToolAndFolder() async throws {
        for set in SampleDaemonClient.SampleSet.allCases where set != .coreDown {
            let tree = try await tree(set)
            let scene = FlowMapScene.traces(tree, gate: try await gate(set))
            let folders = tree.tools.reduce(0) { $0 + min($1.folders.count, FlowMapScene.foldersShown) }
            XCTAssertEqual(scene.nodes.count, SourceKind.allCases.count + folders + 2, "\(set)")
            let tools = scene.nodes.filter { $0.id.hasPrefix("tool:") }
            XCTAssertEqual(tools.filter(\.dashed).count, SourceKind.allCases.count - tree.tools.count, "\(set)")
            for tool in tree.tools {
                XCTAssertEqual(tools.first { $0.id == "tool:\(tool.id)" }?.dashed, false, "\(set): \(tool.id)")
            }
            let space = CGRect(origin: .zero, size: FlowMapScene.size)
            for node in scene.nodes {
                XCTAssertTrue(space.contains(node.at), "\(set): \(node.id) at \(node.at)")
            }
        }
    }

    /// Every node has a name and a detail, so inspecting it never needs a
    /// pointer, and ids are unique so selection is unambiguous.
    func test_everyNodeIsNamedAndDescribed() async throws {
        let scene = FlowMapScene.traces(try await tree(.normalDay), gate: try await gate(.normalDay))
        XCTAssertEqual(Set(scene.nodes.map(\.id)).count, scene.nodes.count)
        for node in scene.nodes {
            XCTAssertFalse(node.label.isEmpty, node.id)
            XCTAssertFalse(node.detail.isEmpty, node.id)
        }
    }

    /// The arc to the commons moves for a tool with a folder set to
    /// contribute automatically, when the core says that tool's sessions go
    /// there and nothing stops them; it is the only thing moving.
    func test_onlyAutomaticFoldersReachTheCommons() async throws {
        let quiet = FlowMapScene.traces(try await tree(.normalDay), gate: try await gate(.normalDay))
        XCTAssertFalse(quiet.flows)
        XCTAssertEqual(quiet.nodes.first { $0.id == "library" }?.kind, .library(active: false))

        let armed = try await tree(.armedFolder)
        let open = try await gate(.armedFolder)
        XCTAssertTrue(open.open, "the recorded armedFolder day has nothing stopping a flow")
        let automatic = armed.tools.filter { $0.mode != .off && $0.folders.contains { $0.mode == .autoUpload } }
        XCTAssertFalse(automatic.isEmpty, "the recorded armedFolder day arms a folder under a tool")
        let scene = FlowMapScene.traces(armed, gate: open)
        XCTAssertEqual(flowing(scene), automatic.filter { open.sendsToCommons($0.kind) }.count)
        XCTAssertTrue(scene.flows)
        XCTAssertEqual(scene.nodes.first { $0.id == "library" }?.kind, .library(active: true))
        // #1146's library card: what was contributed, in the core's words;
        // a dash while History is unread, never none.
        let words = try XCTUnwrap(FlowMapScene.words)
        XCTAssertEqual(scene.nodes.first { $0.id == "library" }?.detail, words.library(contributed: nil))
        XCTAssertTrue(words.library(contributed: nil).hasPrefix("\u{2014} "))
    }

    /// The node cards say #1146's sentences: this computer and the library
    /// count what was contributed from History, a tool's title counts its
    /// folders, and a folder says its path, rule and counts.
    func test_theNodeCardsSayRonsSentences() async throws {
        let words = try XCTUnwrap(FlowMapScene.words)
        let tree = try await tree(.normalDay)
        let history = try await SampleDaemonClient(.normalDay).listHistory(limit: 100)
        let contributed = FlowMapScene.Contributions(history: history)
        let scene = FlowMapScene.traces(tree, gate: try await gate(.normalDay), contributed: contributed)
        let total = try XCTUnwrap(contributed.total)
        let waiting = tree.tools.reduce(0) { $0 + $1.waiting } + tree.unplaced.reduce(0) { $0 + $1.sessions.count }
        XCTAssertEqual(scene.nodes.first { $0.id == "hub" }?.detail, words.hub(waiting: waiting, contributed: total))
        XCTAssertEqual(scene.nodes.first { $0.id == "library" }?.detail, words.library(contributed: total))
        for tool in tree.tools {
            let node = try XCTUnwrap(scene.nodes.first { $0.id == "tool:\(tool.id)" })
            XCTAssertEqual(node.label, tool.kind.displayName)
            XCTAssertEqual(node.cardTitle, words.toolTitle(tool: tool.kind.displayName, folders: tool.folders.count))
            XCTAssertFalse(node.detail.contains("{"), node.detail)
        }
        for node in scene.nodes where node.id.hasPrefix("folder:") {
            XCTAssertTrue(node.detail.contains(" waiting, "), node.detail)
            XCTAssertFalse(node.detail.contains("{"), node.detail)
        }
        // Unset is never said as off; unknown is said as neither.
        XCTAssertEqual(words.tool(.unknown, waiting: 0), words.toolNothingWaiting)
        XCTAssertNotEqual(words.tool(.unset, waiting: 0), words.toolOff)
        XCTAssertEqual(words.folder(path: "/a", rule: "Ask me", waiting: 1, contributed: 2),
                       "/a. Rule: Ask me. 1 trace waiting, 2 contributed.")
        XCTAssertEqual(words.folder(path: nil, rule: nil, waiting: 3, contributed: nil),
                       "Rule: not set. 3 traces waiting, \u{2014} contributed.")
    }

    /// The cards' contributed counts are a whole count or a dash: a page
    /// the daemon capped (`HomeStore.historyLimit`) is not a total, and a
    /// page kept after a failed `list_history` is not current, as the
    /// Folder inspector's shared count (`SummaryFacts.wholeHistory`).
    func test_aCappedOrFailedHistoryCountsNoContributed() async throws {
        let words = try XCTUnwrap(FlowMapScene.words)
        let reply = try XCTUnwrap(SampleDaemonData.reply("list_history", in: .normalDay))
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(reply.utf8)) as? [String: Any])
        let first = try XCTUnwrap((object["history"] as? [[String: Any]])?.first)
        func rows(_ count: Int) throws -> [DaemonData.HistoryRow] {
            try (0..<count).map { index in
                var row = first
                row["status"] = "accepted"
                row["source"] = "claude-code"
                row["project_id"] = "p1"
                row["submission_id"] = "row-\(index)"
                row["session_hash"] = "sha256:row-\(index)"
                return try DaemonDataDecoding.decoder().decode(
                    DaemonData.HistoryRow.self, from: JSONSerialization.data(withJSONObject: row))
            }
        }
        let whole = FlowMapScene.Contributions(history: try rows(HomeStore.historyLimit - 1))
        XCTAssertEqual(whole.total, HomeStore.historyLimit - 1)
        XCTAssertEqual(whole.tool(.claudeCode), HomeStore.historyLimit - 1)
        XCTAssertEqual(whole.folder("p1"), HomeStore.historyLimit - 1)

        let capped = FlowMapScene.Contributions(history: try rows(HomeStore.historyLimit))
        XCTAssertNil(capped.total, "a capped page is counted as the whole")
        XCTAssertNil(capped.tool(.claudeCode))
        XCTAssertNil(capped.folder("p1"))
        let scene = FlowMapScene.traces(try await tree(.normalDay), gate: try await gate(.normalDay), contributed: capped)
        XCTAssertEqual(scene.nodes.first { $0.id == "library" }?.detail, words.library(contributed: nil))

        let stale = FlowMapScene.Contributions(history: try rows(3), failure: .unreachable)
        XCTAssertNil(stale.total, "the last good page is drawn as current after a failed read")
        XCTAssertEqual(FlowMapScene.Contributions(history: try rows(3), failure: nil).total, 3)

        // The window hands the map the failure with the page.
        let window = try TracesInspectorHostTests.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("historyFailure: home.failures[\"list_history\"]"))
    }

    /// Nothing moves to the commons while the core says something stops it:
    /// paused, held for review, a voided grant, a spent budget, signed out,
    /// no route for sessions, or any of those unread. Each still draws the
    /// armed tool's arc, dashed.
    func test_nothingFlowsWhileTheCoreSaysItIsStopped() async throws {
        let armed = try await tree(.armedFolder)
        let stops: [(String, (inout [String: Any]) -> Void)] = [
            ("paused", { $0["paused"] = true }),
            ("held", { $0["automatic_contribution_held"] = ["held_sessions": 2, "reasons": [], "projects": []] }),
            ("held unread", { $0["automatic_contribution_held"] = NSNull() }),
            ("grant voided", { $0["grant_voids"] = [["id": 1, "kind": "project", "project_id": "p"]] }),
            ("voids unread", { $0["grant_voids"] = NSNull() }),
            ("budget spent", { object in
                var budget = object["daily_budget"] as? [String: Any] ?? [:]
                budget["blocked"] = true
                object["daily_budget"] = budget
            }),
            ("signed out", { $0["logged_in"] = false }),
        ]
        for (name, edit) in stops {
            let stopped = try await gate(.armedFolder, edit: edit)
            let scene = FlowMapScene.traces(armed, gate: stopped)
            XCTAssertFalse(scene.flows, name)
            XCTAssertFalse(scene.arcs.filter { $0.style == .dashed }.isEmpty, "\(name): the armed arc is still drawn, dashed")
            XCTAssertEqual(scene.nodes.first { $0.id == "library" }?.kind, .library(active: false), name)
        }
    }

    /// A screen that is not current never draws a flow: core down, loading,
    /// paused, unknown, or a status or route the core did not answer.
    func test_nothingFlowsUnlessTheScreenIsReady() async throws {
        let armed = try await tree(.armedFolder)
        for state in [ScreenState.coreDown, .loading, .paused, .unknown] {
            let stale = try await gate(.armedFolder, state: state)
            XCTAssertFalse(FlowMapScene.traces(armed, gate: stale).flows, "\(state)")
        }
        var unread = try await gate(.armedFolder)
        unread.status = nil
        XCTAssertFalse(FlowMapScene.traces(armed, gate: unread).flows, "no status")
        unread = try await gate(.armedFolder)
        unread.destinations = nil
        XCTAssertFalse(FlowMapScene.traces(armed, gate: unread).flows, "no tool_destinations")
        let nothing = FlowMapScene.traces(armed, gate: .init(state: .coreDown, status: nil, destinations: nil))
        XCTAssertEqual(nothing.nodes.first { $0.id == "library" }?.detail,
                       FlowMapScene.words?.library(contributed: nil))
    }

    /// The menu's second-look row sets its count apart with a spaced dot,
    /// and an unread count is a dash, as `pair` draws it.
    func test_dotPairSpacesTheCountAndDashesAnUnreadOne() {
        XCTAssertEqual(FlowMapScene.dotPair("Worth a second look", 3), "Worth a second look · 3")
        XCTAssertEqual(FlowMapScene.dotPair("Worth a second look", nil), "Worth a second look · —")
    }

    /// An off tool never flows, even with an armed folder under it.
    func test_anOffToolNeverFlows() async throws {
        let open = try await gate(.armedFolder)
        let folder = TracesTree.FolderNode(id: "f", label: "f", mode: .autoUpload, sessions: [])
        let off = TracesTree(tools: [.init(kind: .claudeCode, mode: .off, folders: [folder])], unplaced: [])
        let scene = FlowMapScene.traces(off, gate: open)
        XCTAssertFalse(scene.flows)
        XCTAssertTrue(scene.arcs.allSatisfy { $0.style != .flowing && $0.style != .dashed })
    }

    /// An off tool is crossed out and dimmed; an unset one is never off.
    func test_offToolsAreCrossedOutAndUnsetIsNot() async throws {
        let tree = try await tree(.normalDay)
        let scene = FlowMapScene.traces(tree, gate: try await gate(.normalDay))
        for tool in tree.tools {
            let node = try XCTUnwrap(scene.nodes.first { $0.id == "tool:\(tool.id)" })
            guard case .tool(_, let off) = node.kind else { return XCTFail("\(node.id) is not a tool") }
            XCTAssertEqual(off, tool.mode == .off, tool.id)
        }
    }

    /// Folders past the first four are counted, not drawn.
    func test_extraFoldersAreCounted() {
        let folders = (0 ..< 6).map {
            TracesTree.FolderNode(id: "f\($0)", label: "f\($0)", mode: .ask, sessions: [])
        }
        let tree = TracesTree(tools: [.init(kind: .claudeCode, mode: .watch, folders: folders)], unplaced: [])
        let scene = FlowMapScene.traces(tree, gate: .init(state: .loading))
        XCTAssertEqual(scene.nodes.filter { if case .folder = $0.kind { true } else { false } }.count, FlowMapScene.foldersShown)
        XCTAssertEqual(scene.overflows.map(\.count), [2])
    }

    /// P13: a tool that has contributed is joined to the commons by a
    /// quiet arc, and the library is lit once anything was contributed,
    /// armed or not (#1146 `TracesMap`).
    func test_contributedToolsReachTheLibrary() {
        let folder = TracesTree.FolderNode(id: "f", label: "f", mode: .ask, sessions: [])
        let tree = TracesTree(tools: [.init(kind: .claudeCode, mode: .watch, folders: [folder])], unplaced: [])
        let none = FlowMapScene.traces(tree, gate: .init(state: .ready), contributed: .init(total: 0))
        XCTAssertEqual(none.nodes.first { $0.id == "library" }?.kind, .library(active: false))
        XCTAssertTrue(none.arcs.allSatisfy { $0.to != FlowMapScene.libraryPoint }, "nothing reaches the library")
        let sent = FlowMapScene.traces(
            tree, gate: .init(state: .ready), contributed: .init(total: 4, byTool: [.claudeCode: 4]))
        XCTAssertEqual(sent.nodes.first { $0.id == "library" }?.kind, .library(active: true))
        let toLibrary = sent.arcs.filter { $0.to == FlowMapScene.libraryPoint }
        XCTAssertEqual(toLibrary.map(\.style), [.quiet])
        XCTAssertFalse(sent.flows, "a past contribution never moves")
        // Unread history is never read as contributed.
        let unread = FlowMapScene.traces(tree, gate: .init(state: .ready), contributed: .unread)
        XCTAssertEqual(unread.nodes.first { $0.id == "library" }?.kind, .library(active: false))
    }

    /// A selection's tool is ringed, the others faded to 0.3 and their
    /// library arcs to 0.12 (#1146 `TracesMap`); nothing selected fades
    /// nothing.
    func test_theSelectionsToolIsRingedAndTheOthersFade() {
        let folder = TracesTree.FolderNode(id: "f", label: "f", mode: .ask, sessions: [])
        let tree = TracesTree(tools: [
            .init(kind: .claudeCode, mode: .watch, folders: [folder]),
            .init(kind: .codex, mode: .watch, folders: []),
        ], unplaced: [])
        let contributed = FlowMapScene.Contributions(total: 2, byTool: [.claudeCode: 1, .codex: 1])
        let plain = FlowMapScene.traces(tree, gate: .init(state: .ready), contributed: contributed)
        XCTAssertTrue(plain.nodes.allSatisfy { !$0.ringed })
        XCTAssertTrue(plain.nodes.filter { $0.id.hasPrefix("tool:") && !$0.dashed }.allSatisfy { $0.dim == 1 })
        let scene = FlowMapScene.traces(
            tree, gate: .init(state: .ready), contributed: contributed, selectedTool: SourceKind.codex.rawValue)
        let codex = try? XCTUnwrap(scene.nodes.first { $0.id == "tool:codex" })
        let claude = try? XCTUnwrap(scene.nodes.first { $0.id == "tool:claude-code" })
        XCTAssertEqual(codex?.ringed, true)
        XCTAssertEqual(codex?.dim, 1)
        XCTAssertEqual(claude?.ringed, false)
        XCTAssertEqual(claude?.dim, 0.3)
        let arcs = scene.arcs.filter { $0.to == FlowMapScene.libraryPoint }.map(\.dim).sorted()
        XCTAssertEqual(arcs, [0.12, 1])
        // Every tool stays drawn: the binoculars move the camera, they do
        // not filter the map.
        XCTAssertEqual(scene.focusPoint(tool: SourceKind.codex.rawValue), codex?.at)
        XCTAssertNil(scene.focusPoint(tool: nil))
    }

    /// The binoculars' camera (#1146 `cameraFor`): the tool's point lands
    /// at 1.6x on the same place #1146's transform puts it.
    func test_theFocusCameraScalesAboutTheTool() {
        let size = CGSize(width: 580, height: 760)
        let focus = CGPoint(x: 150, y: 170)
        let fit = FlowMapGeometry(size: size, zoom: 1, focus: focus)
        XCTAssertEqual(fit.scale, 1.6, accuracy: 0.0001)
        let drawn = fit.point(focus)
        XCTAssertEqual(drawn.x, 290 + 1.6 * 50, accuracy: 0.001)
        XCTAssertEqual(drawn.y, 380, accuracy: 0.001)
        XCTAssertEqual(FlowMapGeometry(size: size, zoom: 1).scale, 1, accuracy: 0.0001)
    }

    // MARK: Private AI

    /// Each tool's arc follows the core's state for it, never `connected`:
    /// every state has its own look, and only answering moves.
    func test_thePrivateAISceneFollowsTheCoresVerdict() async throws {
        let harnesses = try await SampleDaemonClient(.normalDay).harnessList()
        let states: [HarnessState] = [.answering, .connectedNoCalls, .activityShared, .notConnected, .unknown]
        let expected: [HarnessState: FlowMapScene.Arc.Style] = [
            .answering: .flowing, .connectedNoCalls: .quiet, .activityShared: .shared,
            .notConnected: .dashed, .unknown: .unknown,
        ]
        XCTAssertEqual(Set(expected.values).count, states.count, "every state draws differently")
        for state in states {
            let scene = FlowMapScene.privateAI(
                harnesses, destinationLabel: "D", privateAI: "running", sentence: { _ in nil }, state: { _ in state })
            XCTAssertEqual(scene.nodes.count, harnesses.harnesses.count + 1)
            // #1146: the node is the credential, with its tools under it.
            XCTAssertEqual(scene.nodes.first?.label, FlowMapScene.words?.credential ?? "D")
            XCTAssertEqual(scene.nodes.first?.sublabel,
                           FlowMapScene.words?.connected(tools: state == .notConnected || state == .unknown
                               ? 0 : harnesses.harnesses.count, sentence: false))
            XCTAssertTrue(scene.arcs.allSatisfy { $0.style == expected[state] }, "\(state)")
            XCTAssertEqual(scene.flows, state == .answering, "\(state)")
            XCTAssertEqual(scene.nodes.first?.kind, .destination(state == .answering ? .answering : .running), "\(state)")
        }
        // No sentence from the core: a dash, never words of the shell's own.
        let scene = FlowMapScene.privateAI(
            harnesses, destinationLabel: "D", privateAI: nil, sentence: { _ in nil }, state: { _ in .unknown })
        XCTAssertTrue(scene.nodes.dropFirst().allSatisfy { $0.detail == "—" })
    }

    /// The destination paints from the core's Private AI state, and unknown
    /// is kept apart from off.
    func test_theDestinationPaintsFromThePrivateAIState() {
        XCTAssertEqual(FlowMapScene.lamp("running", answering: true), .answering)
        XCTAssertEqual(FlowMapScene.lamp("running", answering: false), .running)
        XCTAssertEqual(FlowMapScene.lamp("off", answering: false), .off)
        XCTAssertEqual(FlowMapScene.lamp(nil, answering: false), .unknown)
        XCTAssertEqual(FlowMapScene.lamp("a-state-from-a-later-daemon", answering: false), .unknown)
        // Calls are never answered by a destination the core says is off.
        XCTAssertEqual(FlowMapScene.lamp("off", answering: true), .off)
    }

    /// Nothing answering: nothing moves and the destination is not lit.
    func test_nothingAnsweringIsDrawnStill() async throws {
        let harnesses = try await SampleDaemonClient(.empty).harnessList()
        let scene = FlowMapScene.privateAI(
            harnesses, destinationLabel: "D", privateAI: "off", sentence: { _ in nil }, state: { _ in .notConnected })
        XCTAssertFalse(scene.flows)
        XCTAssertEqual(scene.nodes.first?.kind, .destination(.off))
    }

    /// A tool that is not on this Mac gets the missing-tool sentence, never
    /// the not-connected one (`rowSentence`), and says nothing without the
    /// core's copy.
    func test_aMissingToolNeverSaysItIsNotConnected() throws {
        let row = try JSONDecoder().decode(HarnessRow.self, from: Data(
            #"{"id":"gemini","name":"Gemini CLI","installed":false,"connected":false,"connect_command":"c","state":"not_connected","can_connect":false,"can_disconnect":false}"#.utf8))
        let calls = HarnessCalls(
            stateCode: { _ in 31 }, planOutcomeCode: { _ in 40 }, actionAvailable: { _, _, _ in false },
            stateLine: { _ in "NOT CONNECTED" }, lastCallLine: { _ in "" }, outcomeLine: { _ in "" },
            spendLine: { _ in "" })
        XCTAssertNil(MonitorWindowView.rowSentence(row, copy: nil, calls: calls))
    }

    func test_harnessIdsMapToTheToolsWithArtwork() {
        XCTAssertEqual(FlowMapScene.glassTool(harness: "claude"), .claudeCode)
        XCTAssertEqual(FlowMapScene.glassTool(harness: "codex"), .codex)
        XCTAssertEqual(FlowMapScene.glassTool(harness: "opencode"), .openCode)
        XCTAssertNil(FlowMapScene.glassTool(harness: "something-new"))
    }

    // MARK: Geometry

    /// The design space fits the field and stays centred at any zoom.
    func test_theDesignSpaceFitsAndCentres() {
        let size = CGSize(width: 600, height: 700)
        let fit = FlowMapGeometry(size: size, zoom: 1)
        let topLeft = fit.point(.zero)
        let bottomRight = fit.point(CGPoint(x: FlowMapScene.size.width, y: FlowMapScene.size.height))
        XCTAssertGreaterThanOrEqual(topLeft.x, -0.001)
        XCTAssertGreaterThanOrEqual(topLeft.y, -0.001)
        XCTAssertLessThanOrEqual(bottomRight.x, size.width + 0.001)
        XCTAssertLessThanOrEqual(bottomRight.y, size.height + 0.001)
        let centre = CGPoint(x: FlowMapScene.size.width / 2, y: FlowMapScene.size.height / 2)
        for zoom in [0.6, 1, 2] as [CGFloat] {
            let p = FlowMapGeometry(size: size, zoom: zoom).point(centre)
            XCTAssertEqual(p.x, size.width / 2, accuracy: 0.001)
            XCTAssertEqual(p.y, size.height / 2, accuracy: 0.001)
        }
    }

    // MARK: Inference

    /// A partial sum would read as the whole: one unknown cost makes the
    /// total unknown. Priced is not billed, and zero is a number.
    func test_pricedTotalsAreUnknownWhenAnyCallIs() throws {
        let decode = { (json: String) in try JSONDecoder().decode(DaemonData.PricedCost.self, from: Data(json.utf8)) }
        let known = try decode(#"{"known":true,"priced_micros":8400}"#)
        let unknown = try decode(#"{"known":false,"priced_micros":null}"#)
        XCTAssertEqual(InferenceTabView.priced([known, unknown]), "—")
        XCTAssertEqual(InferenceTabView.priced([nil]), "—")
        XCTAssertNotEqual(InferenceTabView.priced([known]), "—")
        XCTAssertNotEqual(InferenceTabView.priced([]), "—")
    }

    /// Only `verified` is drawn as proof; `failed` is kept apart from the
    /// rest; every label has a word.
    func test_onlyVerifiedReadsAsProof() {
        XCTAssertEqual(InferenceTabView.tone(.verified), .on)
        XCTAssertEqual(InferenceTabView.tone(.failed), .failed)
        XCTAssertNotEqual(InferenceTabView.tone(.failed), InferenceTabView.tone(.outside))
        for label in DaemonData.ProofLabel.allCases where label != .verified {
            XCTAssertNotEqual(InferenceTabView.tone(label), .on, "\(label)")
        }
        for label in DaemonData.ProofLabel.allCases {
            XCTAssertFalse(InferenceWords.proof(label).isEmpty)
        }
        // A label from a newer daemon is a dash, never a verdict.
        XCTAssertEqual(InferenceWords.proof("a-label-from-a-later-daemon"), "—")
        XCTAssertEqual(InferenceTabView.tone("a-label-from-a-later-daemon"), .neutral)
    }

    func test_toolNamesComeFromTheAdapterOrStayRaw() {
        XCTAssertEqual(InferenceTabView.toolName("claude-code"), "Claude Code")
        XCTAssertEqual(InferenceTabView.toolName("unknown"), "—")
        XCTAssertEqual(InferenceTabView.toolName("new-tool"), "new-tool")
    }

    /// The store keeps each read apart: an unreadable ledger is a page that
    /// says so, not an empty one, and the provisional summary is not an error.
    func test_theStoreReadsEachMethodAlone() async {
        let store = InferenceStore(client: SampleDaemonClient(.empty))
        await store.load()
        XCTAssertEqual(store.calls?.readable, false)
        XCTAssertNotNil(store.harnesses)
        XCTAssertTrue(store.failures.isEmpty, "\(store.failures)")

        let busy = InferenceStore(client: SampleDaemonClient(.normalDay))
        await busy.load()
        XCTAssertEqual(busy.calls?.readable, true)
        XCTAssertFalse(busy.calls?.calls.isEmpty ?? true)
    }

    /// Totals cover the whole window: the core's summary when it answered,
    /// the page only when it is the whole ledger, and a dash for one page
    /// of a longer one.
    func test_totalsAreNeverOnePageOfMany() throws {
        let decoder = DaemonDataDecoding.decoder()
        let call = #"{"id":1,"at":"2026-09-30T09:00:00Z","tool":"claude-code","family":"anthropic","model":"m","route":"routed","cost":{"known":true,"priced_micros":1000},"proof":"verified"}"#
        let whole = try decoder.decode(DaemonData.InferenceCallPage.self, from: Data(
            #"{"readable":true,"window_hours":24,"calls":[\#(call)],"next_cursor":null}"#.utf8))
        let partial = try decoder.decode(DaemonData.InferenceCallPage.self, from: Data(
            #"{"readable":true,"window_hours":24,"calls":[\#(call)],"next_cursor":"c2"}"#.utf8))
        let one = InferenceTabView.totals(whole, summary: nil)
        XCTAssertEqual(one.calls, 1)
        XCTAssertEqual(one.verified, 1)
        let cut = InferenceTabView.totals(partial, summary: nil)
        XCTAssertNil(cut.calls)
        XCTAssertNil(cut.verified)
        XCTAssertEqual(cut.priced, "—")

        let summary = try decoder.decode(DaemonData.InferenceSummary.self, from: Data(
            #"{"readable":true,"window_hours":24,"models":[{"model":"a","calls":120,"priced_micros":5000,"proof_counts":{"verified":70,"failed":2}},{"model":"b","calls":30,"priced_micros":1000,"proof_counts":{"pending":30}}]}"#.utf8))
        let core = InferenceTabView.totals(partial, summary: summary)
        XCTAssertEqual(core.calls, 150)
        XCTAssertEqual(core.verified, 70)
        XCTAssertEqual(core.priced, InferenceTabView.money(6000))

        let unknownCalls = try decoder.decode(DaemonData.InferenceSummary.self, from: Data(
            #"{"readable":true,"window_hours":24,"models":[{"model":"a","calls":null,"priced_micros":null,"proof_counts":null}]}"#.utf8))
        let dashed = InferenceTabView.totals(partial, summary: unknownCalls)
        XCTAssertNil(dashed.calls)
        XCTAssertNil(dashed.verified)
        XCTAssertEqual(dashed.priced, "—")
    }

    /// The call count is the core's (K14): every tool's counts plus the
    /// unattributed calls, unknown when any part is, and it wins over one
    /// page of calls.
    func test_theCallCountIsTheCoresToolCounts() throws {
        let decoder = DaemonDataDecoding.decoder()
        let destinations = try decoder.decode(DaemonData.ToolDestinations.self, from: Data(
            #"{"private_ai":"off","sessions_route":"local","folders":null,"window_hours":24,"unattributed_calls":2,"tools":[{"tool":"claude-code","counts":{"sessions":4,"inference_calls":120}},{"tool":"codex","counts":{"sessions":1,"inference_calls":30}}]}"#.utf8))
        XCTAssertEqual(InferenceTabView.callCount(destinations), 152)
        let partial = try decoder.decode(DaemonData.InferenceCallPage.self, from: Data(
            #"{"readable":true,"window_hours":24,"calls":[],"next_cursor":"c2"}"#.utf8))
        XCTAssertEqual(InferenceTabView.totals(partial, summary: nil, destinations: destinations).calls, 152)
        let unread = try decoder.decode(DaemonData.ToolDestinations.self, from: Data(
            #"{"private_ai":"off","sessions_route":"local","folders":null,"window_hours":24,"unattributed_calls":null,"tools":[{"tool":"codex","counts":{"sessions":1,"inference_calls":30}}]}"#.utf8))
        XCTAssertNil(InferenceTabView.callCount(unread))
        XCTAssertNil(InferenceTabView.totals(partial, summary: nil, destinations: unread).calls)
    }

    /// The Inference tab's state on a core that is down: nothing read is
    /// drawn as a count, and the failure is recorded.
    func test_aCoreDownInferenceStoreDrawsNoCounts() async {
        let store = InferenceStore(client: SampleDaemonClient(.coreDown))
        await store.load()
        XCTAssertNil(store.calls)
        XCTAssertNotNil(store.failures["inference_calls"])
    }

    /// A node's halo scales with the map, as its disc does: zoomed in, the
    /// gap and the stroke grow; zoomed out, they shrink. At the design size
    /// they are the design's 4 and 3.
    func test_theRingScalesWithZoom() {
        let field = CGSize(width: FlowMapScene.size.width, height: FlowMapScene.size.height)
        let base = FlowMapView.ringMetrics(scale: FlowMapGeometry(size: field, zoom: 1).scale)
        XCTAssertEqual(base.offset, 4, accuracy: 0.0001)
        XCTAssertEqual(base.lineWidth, 3, accuracy: 0.0001)
        for zoom in [FlowMapView.zoomRange.lowerBound, 1.5, FlowMapView.zoomRange.upperBound] {
            let scale = FlowMapGeometry(size: field, zoom: zoom).scale
            let ring = FlowMapView.ringMetrics(scale: scale)
            XCTAssertEqual(ring.offset, 4 * zoom, accuracy: 0.0001, "offset at zoom \(zoom)")
            XCTAssertEqual(ring.lineWidth, 3 * zoom, accuracy: 0.0001, "width at zoom \(zoom)")
        }
    }

    /// The Inference tab says what window its counts cover, from the hours
    /// the core reported, in the core's words; a dash when none was
    /// reported, never a default window.
    func test_theInferenceTabSaysItsWindowFromTheData() throws {
        let decoder = DaemonDataDecoding.decoder()
        let words = try XCTUnwrap(MonitorWords.table)
        func page(_ hours: String) throws -> DaemonData.InferenceCallPage {
            try decoder.decode(DaemonData.InferenceCallPage.self, from: Data(
                #"{"readable":true,"window_hours":\#(hours),"calls":[],"next_cursor":null}"#.utf8))
        }
        func destinations(_ hours: String) throws -> DaemonData.ToolDestinations {
            try decoder.decode(DaemonData.ToolDestinations.self, from: Data(
                #"{"private_ai":"off","sessions_route":"local","folders":null,"window_hours":\#(hours),"unattributed_calls":0,"tools":[]}"#.utf8))
        }

        XCTAssertTrue(words.windowLastHours.contains("{hours}"))
        let day = InferenceTabView.windowLine(try page("24"), destinations: nil)
        XCTAssertEqual(day, words.windowLastHours.replacingOccurrences(of: "{hours}", with: "24"))
        XCTAssertTrue(day.contains("24"), day)
        // Another window is said as that window, not as 24.
        XCTAssertTrue(InferenceTabView.windowLine(try page("6"), destinations: nil).contains("6"))
        // The page's own window first; tool_destinations' when it has none.
        XCTAssertEqual(InferenceTabView.windowHours(try page("null"), destinations: try destinations("48")), 48)
        XCTAssertEqual(InferenceTabView.windowHours(try page("12"), destinations: try destinations("48")), 12)
        // Neither reported a window: a dash.
        XCTAssertNil(InferenceTabView.windowHours(try page("null"), destinations: try destinations("null")))
        XCTAssertEqual(InferenceTabView.windowLine(try page("null"), destinations: nil), "\u{2014}")
        XCTAssertEqual(words.windowLine(hours: nil), "\u{2014}")
    }

    /// A node's label and sublabel never meet: #1146's 11 and 10 point type
    /// 14 apart, scaled with the map, at every zoom, never below the floor.
    func test_nodeLabelsNeverTouch() {
        for scale: CGFloat in [0.3, 0.6, 0.85, 1, 1.4, 2] {
            let type = FlowMapView.labelType(scale: scale)
            XCTAssertLessThan(type.sublabelSize, type.labelSize, "the sublabel is the smaller caption")
            // Half of each line's height, at a generous 1.2 line height.
            let clearance = type.sublabelOffset - type.labelOffset
            XCTAssertGreaterThan(clearance, 0.6 * type.labelSize + 0.6 * type.sublabelSize, "at \(scale)")
            XCTAssertGreaterThanOrEqual(type.labelSize, 11 * FlowMapView.labelFloor)
        }
        XCTAssertEqual(FlowMapView.labelType(scale: 1).labelSize, 11)
        XCTAssertEqual(FlowMapView.labelType(scale: 1).sublabelSize, 10)
        // The credential's key is #1146's level outline, wider than tall.
        let key = FlowMapView.keyGlyph(centre: .zero, scale: 1).boundingRect
        XCTAssertGreaterThan(key.width, key.height)
    }
}
