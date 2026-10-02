import TCDesign
import TCShellCore
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

    // MARK: Traces

    /// One node per tool and per drawn folder, plus this computer and the
    /// commons; every node inside the design space.
    func test_theTracesSceneDrawsEveryToolAndFolder() async throws {
        for set in SampleDaemonClient.SampleSet.allCases where set != .coreDown {
            let tree = try await tree(set)
            let scene = FlowMapScene.traces(tree)
            let folders = tree.tools.reduce(0) { $0 + min($1.folders.count, FlowMapScene.foldersShown) }
            XCTAssertEqual(scene.nodes.count, tree.tools.count + folders + 2, "\(set)")
            let space = CGRect(origin: .zero, size: FlowMapScene.size)
            for node in scene.nodes {
                XCTAssertTrue(space.contains(node.at), "\(set): \(node.id) at \(node.at)")
            }
        }
    }

    /// Every node has a name and a detail, so inspecting it never needs a
    /// pointer, and ids are unique so selection is unambiguous.
    func test_everyNodeIsNamedAndDescribed() async throws {
        let scene = FlowMapScene.traces(try await tree(.normalDay))
        XCTAssertEqual(Set(scene.nodes.map(\.id)).count, scene.nodes.count)
        for node in scene.nodes {
            XCTAssertFalse(node.label.isEmpty, node.id)
            XCTAssertFalse(node.detail.isEmpty, node.id)
        }
    }

    /// The arc to the commons is drawn for a tool with a folder set to
    /// contribute automatically, and only then; it is the only thing moving.
    func test_onlyAutomaticFoldersReachTheCommons() async throws {
        let quiet = FlowMapScene.traces(try await tree(.normalDay))
        XCTAssertFalse(quiet.flows)
        XCTAssertEqual(quiet.nodes.first { $0.id == "library" }?.kind, .library(active: false))

        let armed = try await tree(.armedFolder)
        let automatic = armed.tools.filter { $0.folders.contains { $0.mode == .autoUpload } }
        let scene = FlowMapScene.traces(armed)
        XCTAssertEqual(scene.arcs.filter { $0.style == .flowing }.count, automatic.count)
        XCTAssertEqual(scene.flows, !automatic.isEmpty)
        if !automatic.isEmpty {
            XCTAssertEqual(scene.nodes.first { $0.id == "library" }?.kind, .library(active: true))
        }
    }

    /// An off tool is crossed out and dimmed; an unset one is never off.
    func test_offToolsAreCrossedOutAndUnsetIsNot() async throws {
        let tree = try await tree(.normalDay)
        let scene = FlowMapScene.traces(tree)
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
        let scene = FlowMapScene.traces(tree)
        XCTAssertEqual(scene.nodes.filter { if case .folder = $0.kind { true } else { false } }.count, FlowMapScene.foldersShown)
        XCTAssertEqual(scene.overflows.map(\.count), [2])
    }

    // MARK: Private AI

    /// Each tool joins the destination: flowing only when the core says its
    /// calls are answered, dashed when it is not connected.
    func test_thePrivateAISceneFollowsTheCoresVerdict() async throws {
        let harnesses = try await SampleDaemonClient(.normalDay).harnessList()
        let scene = FlowMapScene.privateAI(
            harnesses, destinationLabel: "D", sentence: { _ in nil }, answering: { $0.id == "claude" })
        XCTAssertEqual(scene.nodes.count, harnesses.harnesses.count + 1)
        XCTAssertEqual(scene.nodes.first?.label, "D")
        for (row, arc) in zip(harnesses.harnesses, scene.arcs) {
            let expected: FlowMapScene.Arc.Style =
                row.connected && row.id == "claude" ? .flowing : row.connected ? .quiet : .dashed
            XCTAssertEqual(arc.style, expected, row.id)
        }
        // No sentence from the core: a dash, never words of the shell's own.
        XCTAssertTrue(scene.nodes.dropFirst().allSatisfy { $0.detail == "—" })
    }

    /// Nothing answering: nothing moves and the destination is not lit.
    func test_nothingAnsweringIsDrawnStill() async throws {
        let harnesses = try await SampleDaemonClient(.empty).harnessList()
        let scene = FlowMapScene.privateAI(harnesses, destinationLabel: "D", sentence: { _ in nil }, answering: { _ in false })
        XCTAssertFalse(scene.flows)
        XCTAssertEqual(scene.nodes.first?.kind, .destination(answering: false))
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
        XCTAssertEqual(InferenceTabView.tone(.failed), .outside)
        for label in DaemonData.ProofLabel.allCases where label != .verified {
            XCTAssertNotEqual(InferenceTabView.tone(label), .on, "\(label)")
        }
        for label in DaemonData.ProofLabel.allCases {
            XCTAssertFalse(InferenceWords.proof(label).isEmpty)
        }
        XCTAssertEqual(InferenceWords.proof("a-label-from-a-later-daemon"), InferenceWords.proof(.unrecorded))
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
}
