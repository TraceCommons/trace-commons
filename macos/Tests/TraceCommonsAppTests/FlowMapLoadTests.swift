import CoreGraphics
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The flow maps under load (owner, 2026-10-14): every tool watching many
/// folders with long names, and many tools connected to Private AI, at a
/// tall, a wide and a small pane. Nothing runs off its band, no two names
/// in a column come closer than a line, and what is not drawn is counted.
@MainActor
final class FlowMapLoadTests: XCTestCase {
    static let panes = [CGSize(width: 440, height: 860), CGSize(width: 900, height: 560), CGSize(width: 360, height: 560)]

    static func heavyTree() -> TracesTree {
        let counts: [SourceKind: Int] = [.claudeCode: 9, .codex: 6, .geminiCli: 3, .cline: 1, .opencode: 5]
        var n = 0
        let tools = SourceKind.allCases.map { kind in
            TracesTree.ToolNode(kind: kind, mode: kind == .cline ? .off : .watch, folders: (0 ..< (counts[kind] ?? 0)).map { _ in
                defer { n += 1 }
                return TracesTree.FolderNode(id: "f\(n)", label: "a-long-worktree-name-\(n)", mode: .ask, sessions: [])
            })
        }
        return TracesTree(tools: tools, unplaced: [])
    }

    static func harnesses(_ count: Int) -> HarnessList {
        let rows = (0 ..< count).map { i in
            #"{"id":"h\#(i)","name":"Agent \#(i)","installed":true,"connected":true,"config_path":"/x","connect_command":"c","family":"f","answers_at":"A","state":"connected","last_call_at":null,"can_connect":false,"can_disconnect":true}"#
        }
        return HarnessSurface.list(fromJSON: #"{"catalog_present":true,"harnesses":["# + rows.joined(separator: ",")
            + #"],"activity":{"readable":true,"window_hours":24,"last_call_at":null,"families":[]},"spend":{"known":false},"destination_port":8463,"destination_credentialed":true}"#)
    }

    /// The least gap between neighbours in a column of nodes.
    static func tightest(_ nodes: [FlowMapScene.Node]) -> CGFloat {
        let ys = nodes.map(\.at.y).sorted()
        return zip(ys, ys.dropFirst()).map { $1 - $0 }.min() ?? .infinity
    }

    func test_aLoadedTracesMapFitsItsBandAndCountsWhatItDoesNotDraw() {
        let tree = Self.heavyTree()
        for pane in Self.panes {
            let space = FlowMapScene.space(for: pane)
            let band = FlowMapScene.band(space)
            let scene = FlowMapScene.traces(tree, gate: .init(state: .ready), space: space)
            for node in scene.nodes where node.id != "library" {
                XCTAssertTrue(band.contains(node.at.y), "\(node.id) off the band at \(pane)")
                XCTAssertTrue((0 ... space.width).contains(node.at.x), "\(node.id) off the space at \(pane)")
            }
            let folders = scene.nodes.filter { if case .folder = $0.kind { true } else { false } }
            XCTAssertGreaterThanOrEqual(Self.tightest(folders), FlowMapScene.folderStepRange.lowerBound - 0.5, "\(pane)")
            XCTAssertEqual(folders.count + scene.reveals.map(\.hidden).reduce(0, +),
                           tree.tools.map(\.folders.count).reduce(0, +), "every folder is drawn or counted at \(pane)")
            XCTAssertEqual(scene.nodes.filter { $0.id.hasPrefix("tool:") }.count, SourceKind.allCases.count)
        }
    }

    func test_aLoadedPrivateAIMapFitsItsBandAndCountsEveryTool() {
        for count in [6, 12, 20, 40] {
            let list = Self.harnesses(count)
            for pane in Self.panes {
                let space = FlowMapScene.space(for: pane)
                let band = FlowMapScene.band(space)
                let scene = FlowMapScene.privateAI(
                    list, destinationLabel: "Credential", privateAI: "running", sentence: { _ in nil },
                    state: { _ in .connectedNoCalls }, space: space)
                let tools = scene.nodes.filter { $0.id.hasPrefix("harness:") }
                for node in tools { XCTAssertTrue(band.contains(node.at.y), "\(node.id) off the band at \(pane)") }
                XCTAssertGreaterThanOrEqual(Self.tightest(tools), FlowMapScene.harnessStepRange.lowerBound - 0.5)
                XCTAssertEqual(tools.count + scene.overflows.map(\.count).reduce(0, +), count, "\(count) at \(pane)")
                // The credential counts every connected tool, drawn or not.
                let destination = scene.nodes.first { $0.id == "destination" }
                XCTAssertEqual(destination?.sublabel, FlowMapScene.words?.connected(tools: count, sentence: false))
            }
        }
    }

    /// A tool with no artwork carries two letters of its name, never a
    /// blank circle (owner, 2026-10-14).
    func test_aToolWithoutArtworkCarriesItsInitials() {
        XCTAssertEqual(FlowMapScene.initials("Cursor"), "Cu")
        XCTAssertEqual(FlowMapScene.initials("GitHub Copilot CLI"), "GC")
        XCTAssertEqual(FlowMapScene.initials("Agent 3"), "A3")
        let scene = FlowMapScene.privateAI(
            Self.harnesses(3), destinationLabel: "Credential", privateAI: "running", sentence: { _ in nil },
            state: { _ in .notConnected })
        for node in scene.nodes where node.id.hasPrefix("harness:") {
            guard case .harness(let tool, _) = node.kind else { return XCTFail(node.id) }
            XCTAssertNotNil(tool, "\(node.id) is a blank circle")
        }
    }
}
