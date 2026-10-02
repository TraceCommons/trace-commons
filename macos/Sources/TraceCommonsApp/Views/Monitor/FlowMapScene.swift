#if DEBUG
import CoreGraphics
import TCDesign
import TCShellCore

/// What the flow map draws (R8 of #1173), in #1146's 580×760 design space:
/// nodes, the arcs between them, and what each node says when inspected.
/// Pure data, so the layout and its words are tested without drawing.
///
/// Only what the core reports is drawn. A node's words are a name the core
/// reports, a single word, a number, or core copy (`ShellWordingTests`).
struct FlowMapScene: Equatable {
    struct Node: Identifiable, Equatable {
        enum Kind: Equatable {
            /// This computer: where sessions are recorded and scrubbed.
            case hub
            /// The commons library sessions are contributed to.
            case library(active: Bool)
            /// A watched tool. `off` crosses it out; unset draws it plain.
            case tool(GlassTool, off: Bool)
            /// A folder, ringed by its rule (nil: the core has not listed it).
            case folder(ProjectMode?)
            /// A tool that can send model calls here.
            case harness(GlassTool?, installed: Bool)
            /// The Private AI destination tools connect to.
            case destination(answering: Bool)
        }

        let id: String
        let kind: Kind
        let at: CGPoint
        let radius: CGFloat
        /// Drawn under the node; also its accessible name.
        let label: String
        /// The inspection card: its detail line, and the node's accessible value.
        let detail: String
        var dim: Double = 1
    }

    struct Arc: Equatable {
        enum Style: Equatable {
            /// A quiet link: tool to computer, folder to tool.
            case quiet
            /// Configured but not carrying anything: dashed.
            case dashed
            /// Carrying sessions or calls: moving dashes, in `statusOn`.
            case flowing
        }

        let from: CGPoint
        let control1: CGPoint
        let control2: CGPoint
        let to: CGPoint
        let style: Style
        var dim: Double = 1
    }

    /// A count of folders not drawn under a tool, as "+N".
    struct Overflow: Equatable {
        let at: CGPoint
        let count: Int
    }

    var nodes: [Node] = []
    var arcs: [Arc] = []
    var overflows: [Overflow] = []

    static let size = CGSize(width: 580, height: 760)

    /// Anything moving, so the timeline can stop when nothing is.
    var flows: Bool { arcs.contains { $0.style == .flowing } }

    // MARK: Traces

    static let hubPoint = CGPoint(x: 290, y: 400)
    static let libraryPoint = CGPoint(x: 470, y: 150)
    /// Folders drawn under one tool; the rest are counted.
    static let foldersShown = 4

    /// Tools and their folders, ringed by rule, joined to this computer;
    /// an arc to the commons for every tool with a folder set to contribute
    /// automatically.
    static func traces(_ tree: TracesTree) -> FlowMapScene {
        var scene = FlowMapScene()
        let tools = tree.tools
        let step = tools.count > 1 ? 460 / CGFloat(tools.count - 1) : 0
        var automatic = 0

        for (index, tool) in tools.enumerated() {
            let at = CGPoint(x: 150, y: tools.count > 1 ? 170 + CGFloat(index) * step : hubPoint.y)
            let off = tool.mode == .off
            let toolDim = off ? 0.4 : 1
            let sends = tool.folders.contains { $0.mode == .autoUpload }
            automatic += tool.folders.filter { $0.mode == .autoUpload }.count

            if sends {
                scene.arcs.append(curve(from: CGPoint(x: at.x + 12, y: at.y - 6), to: libraryPoint, style: .flowing, dim: off ? 0.3 : 1))
            }
            let midX = (hubPoint.x + at.x) / 2
            scene.arcs.append(Arc(
                from: at, control1: CGPoint(x: midX, y: at.y), control2: CGPoint(x: midX, y: hubPoint.y),
                to: CGPoint(x: hubPoint.x - 16, y: hubPoint.y), style: .quiet, dim: toolDim))

            let shown = Array(tool.folders.prefix(foldersShown))
            for (folderIndex, folder) in shown.enumerated() {
                let point = CGPoint(x: 58, y: at.y + (CGFloat(folderIndex) - CGFloat(shown.count - 1) / 2) * 46)
                scene.arcs.append(Arc(
                    from: CGPoint(x: at.x - 13, y: at.y), control1: CGPoint(x: at.x - 42, y: at.y),
                    control2: CGPoint(x: at.x - 42, y: point.y), to: CGPoint(x: point.x + 8, y: point.y),
                    style: .quiet, dim: toolDim))
                scene.nodes.append(Node(
                    id: "folder:\(folder.id)", kind: .folder(folder.mode), at: point, radius: 7,
                    label: folder.label, detail: folderDetail(folder), dim: toolDim))
            }
            if tool.folders.count > shown.count {
                scene.overflows.append(Overflow(at: CGPoint(x: 58, y: at.y + 118), count: tool.folders.count - shown.count))
            }
            scene.nodes.append(Node(
                id: "tool:\(tool.id)", kind: .tool(TracesTreeView.glassTool(tool.kind), off: off), at: at, radius: 13,
                label: tool.kind.displayName, detail: toolDetail(tool), dim: toolDim))
        }

        let waiting = tools.reduce(0) { $0 + $1.waiting } + tree.unplaced.reduce(0) { $0 + $1.sessions.count }
        scene.nodes.insert(Node(
            id: "hub", kind: .hub, at: hubPoint, radius: 16,
            label: MonitorWords.computer, detail: pair(MonitorWords.waiting, waiting)), at: 0)
        scene.nodes.append(Node(
            id: "library", kind: .library(active: automatic > 0), at: libraryPoint, radius: 11,
            label: MonitorWords.commons, detail: pair(ProjectCopy.modeChoiceLabel(.autoUpload), automatic)))
        return scene
    }

    static func toolDetail(_ tool: TracesTree.ToolNode) -> String {
        var parts: [String] = []
        switch tool.mode {
        case .watch: parts.append(MonitorWords.watched)
        case .off: parts.append(MonitorWords.off)
        // Unset and unknown say nothing: neither is ever drawn as off.
        case .unset, .unknown: break
        }
        parts.append(pair(MonitorWords.waiting, tool.waiting))
        parts.append(pair(MonitorWords.folders, tool.folders.count))
        return parts.joined(separator: " · ")
    }

    static func folderDetail(_ folder: TracesTree.FolderNode) -> String {
        [folder.mode.map(ProjectCopy.modeChoiceLabel) ?? "—", pair(MonitorWords.waiting, folder.sessions.count)]
            .joined(separator: " · ")
    }

    // MARK: Private AI

    static let destinationPoint = CGPoint(x: 290, y: 380)

    /// Each tool that can send model calls here, joined to the destination:
    /// solid when its config sends calls here, flowing only when the core
    /// says calls are answered, dashed when it is not connected.
    ///
    /// - `destinationLabel` is the core's name for the destination.
    /// - `sentence` is the core's state sentence for a row, if it has one.
    /// - `answering` is the core's verdict that a row's calls arrive.
    static func privateAI(
        _ harnesses: HarnessList,
        destinationLabel: String,
        sentence: (HarnessRow) -> String?,
        answering: (HarnessRow) -> Bool
    ) -> FlowMapScene {
        var scene = FlowMapScene()
        let rows = harnesses.harnesses
        let step = rows.count > 1 ? 420 / CGFloat(rows.count - 1) : 0
        var anyAnswering = false

        for (index, row) in rows.enumerated() {
            let y = rows.count > 1 ? 170 + CGFloat(index) * step : destinationPoint.y
            let answers = row.connected && answering(row)
            anyAnswering = anyAnswering || answers
            scene.arcs.append(Arc(
                from: CGPoint(x: 118, y: y), control1: CGPoint(x: 200, y: y),
                control2: CGPoint(x: 210, y: destinationPoint.y), to: destinationPoint,
                style: answers ? .flowing : row.connected ? .quiet : .dashed))
            scene.nodes.append(Node(
                id: "harness:\(row.id)", kind: .harness(glassTool(harness: row.id), installed: row.installed),
                at: CGPoint(x: 106, y: y), radius: 13, label: row.name, detail: sentence(row) ?? "—",
                dim: row.installed ? 1 : 0.5))
        }
        let connected = rows.filter(\.connected).count
        scene.nodes.insert(Node(
            id: "destination", kind: .destination(answering: anyAnswering), at: destinationPoint, radius: 26,
            label: destinationLabel, detail: pair(MonitorWords.connected, connected)), at: 0)
        return scene
    }

    /// IronWire's harness ids to the tools that have artwork.
    static func glassTool(harness id: String) -> GlassTool? {
        switch id {
        case "claude", "claude-code": .claudeCode
        case "codex": .codex
        case "gemini", "gemini-cli": .geminiCLI
        case "cline": .cline
        case "opencode": .openCode
        default: nil
        }
    }

    // MARK: Helpers

    /// A word and its count, in that order: "Waiting 3".
    static func pair(_ word: String, _ count: Int) -> String {
        "\(word) \(count)"
    }

    private static func curve(from: CGPoint, to: CGPoint, style: Arc.Style, dim: Double) -> Arc {
        let midX = (from.x + to.x) / 2
        return Arc(from: from, control1: CGPoint(x: midX, y: from.y), control2: CGPoint(x: midX, y: to.y), to: to, style: style, dim: dim)
    }
}
#endif
