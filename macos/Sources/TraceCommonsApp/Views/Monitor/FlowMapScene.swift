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
            /// The Private AI destination tools connect to, drawn from the
            /// core's Private AI state.
            case destination(Lamp)
        }

        /// The Private AI destination's state, from `tool_destinations
        /// .private_ai` (or the status's `private_inference_state`). Unknown
        /// is drawn apart from off: a missing signal is never "off".
        enum Lamp: Equatable {
            /// Running, and the core says some tool's calls are answered.
            case answering
            /// Running, with nothing answered.
            case running
            case off
            case unknown
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
            /// Calls arrived in this tool's protocol family, but more than
            /// one connected tool speaks it, so the core cannot say they
            /// were this tool's: lit, never moving.
            case shared
            /// The core could not say: dotted, never solid or moving.
            case unknown
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

    /// What decides whether sessions move to the commons, as the core
    /// reports it. Anything unread is closed: a missing signal never draws
    /// a flow (#1190).
    struct CommonsGate: Equatable {
        var state: ScreenState
        var status: DaemonData.Status?
        var destinations: DaemonData.ToolDestinations?

        /// Whether anything at all can move to the commons now: the screen
        /// is current, nothing is paused, held, voided or over budget, the
        /// contributor is signed in, and the core names a route for
        /// sessions.
        var open: Bool {
            guard state == .ready, let status, let destinations else { return false }
            guard status.paused == false, status.loggedIn == true else { return false }
            guard let held = status.automaticContributionHeld?.heldSessions, held == 0 else { return false }
            guard let voids = status.grantVoids, voids.isEmpty else { return false }
            guard status.dailyBudget?.blocked != true else { return false }
            guard let route = destinations.sessionsRoute, Self.routes.contains(route) else { return false }
            return true
        }

        /// The `sessions_route` values with somewhere to send sessions;
        /// `witness_refusing`, `not_enrolled` and `settings_unreadable` have
        /// none.
        static let routes: Set<String> = ["witness", "local"]

        /// Whether the core says this tool's sessions are watched and go to
        /// the commons.
        func sendsToCommons(_ kind: SourceKind) -> Bool {
            guard let route = destinations?.tools.first(where: { $0.tool == kind.rawValue })?.sessions else { return false }
            return route.watch == "watched" && route.to.contains("commons")
        }
    }

    /// Tools and their folders, ringed by rule, joined to this computer.
    /// A tool with a folder set to contribute automatically is joined to
    /// the commons: moving only when the gate is open and the core says
    /// that tool's sessions go there, dashed otherwise, and never for a
    /// tool that is off.
    static func traces(_ tree: TracesTree, gate: CommonsGate) -> FlowMapScene {
        var scene = FlowMapScene()
        let tools = tree.tools
        let step = tools.count > 1 ? 460 / CGFloat(tools.count - 1) : 0
        var flowing = false

        for (index, tool) in tools.enumerated() {
            let at = CGPoint(x: 150, y: tools.count > 1 ? 170 + CGFloat(index) * step : hubPoint.y)
            let off = tool.mode == .off
            let toolDim = off ? 0.4 : 1
            let armed = tool.folders.contains { $0.mode == .autoUpload }

            if armed && !off {
                let moves = gate.open && gate.sendsToCommons(tool.kind)
                flowing = flowing || moves
                scene.arcs.append(curve(from: CGPoint(x: at.x + 12, y: at.y - 6), to: libraryPoint, style: moves ? .flowing : .dashed, dim: 1))
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
        // The armed count is the core's (`tool_destinations.folders.armed`),
        // a dash when it did not say.
        scene.nodes.append(Node(
            id: "library", kind: .library(active: flowing), at: libraryPoint, radius: 11,
            label: MonitorWords.commons,
            detail: pair(ProjectCopy.modeChoiceLabel(.autoUpload), gate.destinations?.folders?.armed)))
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

    /// Each tool that can send model calls here, joined to the destination,
    /// drawn from the core's state for it (`HarnessState`), never from
    /// `connected`: flowing only when its calls are answered, solid when
    /// connected with nothing answered, lit but still when its family's
    /// calls cannot be attributed, dashed when not connected, dotted when
    /// the core could not say.
    ///
    /// - `destinationLabel` is the core's name for the destination.
    /// - `privateAI` is the core's Private AI state label (`running`,
    ///   `off`, ...), nil when it did not say.
    /// - `sentence` is the core's sentence for a row (`rowSentence`).
    /// - `state` is the core's verdict for a row.
    static func privateAI(
        _ harnesses: HarnessList,
        destinationLabel: String,
        privateAI: String?,
        sentence: (HarnessRow) -> String?,
        state: (HarnessRow) -> HarnessState
    ) -> FlowMapScene {
        var scene = FlowMapScene()
        let rows = harnesses.harnesses
        let step = rows.count > 1 ? 420 / CGFloat(rows.count - 1) : 0
        var anyAnswering = false
        var connected = 0

        for (index, row) in rows.enumerated() {
            let y = rows.count > 1 ? 170 + CGFloat(index) * step : destinationPoint.y
            let style: Arc.Style
            switch state(row) {
            case .answering:
                style = .flowing
                anyAnswering = true
                connected += 1
            case .connectedNoCalls:
                style = .quiet
                connected += 1
            case .activityShared:
                style = .shared
                connected += 1
            case .notConnected: style = .dashed
            case .unknown: style = .unknown
            }
            scene.arcs.append(Arc(
                from: CGPoint(x: 118, y: y), control1: CGPoint(x: 200, y: y),
                control2: CGPoint(x: 210, y: destinationPoint.y), to: destinationPoint,
                style: style))
            scene.nodes.append(Node(
                id: "harness:\(row.id)", kind: .harness(glassTool(harness: row.id), installed: row.installed),
                at: CGPoint(x: 106, y: y), radius: 13, label: row.name, detail: sentence(row) ?? "—",
                dim: row.installed ? 1 : 0.5))
        }
        let lamp = lamp(privateAI, answering: anyAnswering)
        var detail = pair(MonitorWords.connected, connected)
        if lamp == .unknown { detail += " · " + MonitorWords.unknown }
        scene.nodes.insert(Node(
            id: "destination", kind: .destination(lamp), at: destinationPoint, radius: 26,
            label: destinationLabel, detail: detail), at: 0)
        return scene
    }

    /// The destination's lamp from the core's Private AI state label. A
    /// label this shell does not know is unknown, never off.
    static func lamp(_ privateAI: String?, answering: Bool) -> Node.Lamp {
        switch privateAI {
        case "running": answering ? .answering : .running
        case "off": .off
        default: .unknown
        }
    }

    /// IronWire's harness ids to the tools that have artwork. The table is
    /// `HarnessToolArt`, beside the tools list the release window draws.
    static func glassTool(harness id: String) -> GlassTool? {
        HarnessToolArt.tool(harness: id)
    }

    // MARK: Helpers

    /// A word and its count, in that order: "Waiting 3".
    static func pair(_ word: String, _ count: Int) -> String {
        "\(word) \(count)"
    }

    /// A word and a count the core may not have reported: a dash then.
    static func pair(_ word: String, _ count: Int?) -> String {
        "\(word) \(count.map(String.init) ?? "—")"
    }

    private static func curve(from: CGPoint, to: CGPoint, style: Arc.Style, dim: Double) -> Arc {
        let midX = (from.x + to.x) / 2
        return Arc(from: from, control1: CGPoint(x: midX, y: from.y), control2: CGPoint(x: midX, y: to.y), to: to, style: style, dim: dim)
    }
}
