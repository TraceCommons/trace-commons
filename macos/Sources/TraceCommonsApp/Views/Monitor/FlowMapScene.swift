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
            /// This computer: where sessions are collected.
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
        /// The card's title when it is not the label (#1146: a tool's card
        /// adds its folder count).
        var title: String? = nil
        /// A tool that is not watched: drawn with a dashed rim (#1146's
        /// "Tool not watched").
        var dashed = false
        /// Ringed as the selection's tool (#1146 `ToolGroup` ring).
        var ringed = false
        /// A second line under the label (#1146: the credential's tools).
        var sublabel: String? = nil

        /// Whether the label sits to the left of the node, on its line,
        /// rather than under it: folders and the Private AI tools, the
        /// nodes that come in long lists (owner, 2026-10-14).
        var labelBeside: Bool {
            switch kind {
            case .folder, .harness: true
            default: false
            }
        }

        var cardTitle: String { title ?? label }
    }

    /// What has been contributed from this machine, from History's rows
    /// that left and still stand (#1146 `ToolNode.contributed`). Nil
    /// counts while History has not been read: an unread record is never
    /// said as nothing contributed. A page the daemon capped, or one kept
    /// after a failed `list_history`, is not a whole current count either,
    /// so it is nil too, as the Folder inspector's shared count.
    struct Contributions: Equatable {
        var total: Int?
        var byTool: [SourceKind: Int] = [:]
        var byFolder: [String: Int] = [:]

        static let unread = Contributions(total: nil)

        init(total: Int?, byTool: [SourceKind: Int] = [:], byFolder: [String: Int] = [:]) {
            self.total = total
            self.byTool = byTool
            self.byFolder = byFolder
        }

        @MainActor
        init(history: [DaemonData.HistoryRow]?, failure: DaemonDataError? = nil) {
            guard let history = SummaryFacts.wholeHistory(SummaryFacts.fresh(history, unless: failure)) else {
                self = .unread
                return
            }
            let stand = history.filter { TracesGraphModel.contributedStatuses.contains($0.status ?? "") }
            total = stand.count
            for row in stand {
                if let tool = row.source.flatMap(SourceKind.init(rawValue:)) { byTool[tool, default: 0] += 1 }
                if let folder = row.projectId { byFolder[folder, default: 0] += 1 }
            }
        }

        func tool(_ kind: SourceKind) -> Int? { total == nil ? nil : byTool[kind] ?? 0 }
        func folder(_ id: String) -> Int? { total == nil ? nil : byFolder[id] ?? 0 }
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

    /// A count of tools not drawn in the Private AI map, as "+N".
    struct Overflow: Equatable {
        let at: CGPoint
        let count: Int
    }

    /// The pill under a tool's folders (owner, 2026-10-15): "+N" while some
    /// are not drawn, and a click draws the next `foldersRevealed` of them;
    /// once a reveal has drawn them all, a pill that folds them back.
    struct FolderReveal: Equatable {
        /// The tool's id (`ToolNode.id`) and its name.
        let tool: String
        let toolName: String
        /// The folder column on the pill's line: the pill ends where the
        /// names do, left of the dots.
        let at: CGPoint
        /// The folders not drawn; 0 for the pill that folds them back.
        let hidden: Int
        /// Faded with its tool.
        var dim: Double = 1

        /// How many folders a click draws.
        var next: Int { min(hidden, FlowMapScene.foldersRevealed) }
    }

    var nodes: [Node] = []
    var arcs: [Arc] = []
    var overflows: [Overflow] = []
    var reveals: [FolderReveal] = []
    /// The design space this scene is laid out in: #1146's 580×760 by
    /// default, or one shaped like the pane (`space(for:)`).
    var space: CGSize = FlowMapScene.size

    static let size = CGSize(width: 580, height: 760)

    /// A design space with the pane's proportions and #1146's area, so the
    /// map fills the pane rather than letterboxing in it, at much the same
    /// scale (owner, 2026-10-14: reflow for the space there is). Very tall
    /// or very wide panes are held to a sensible shape.
    static func space(for pane: CGSize) -> CGSize {
        guard pane.width > 0, pane.height > 0 else { return size }
        let aspect = min(max(pane.width / pane.height, 0.42), 1.7)
        let area = size.width * size.height
        return CGSize(width: (area * aspect).squareRoot(), height: (area / aspect).squareRoot())
    }

    /// The band nodes are laid out in: clear of the view switch at the top
    /// and the legend and zoom controls at the bottom.
    static func band(_ space: CGSize) -> ClosedRange<CGFloat> {
        (space.height * 0.13) ... (space.height * 0.87)
    }

    /// Positions for `weights` stacked down the band, each given room in
    /// proportion to its weight, no more than `maxStep` per unit and at
    /// least `minStep`, the group centred on the band. Returns each
    /// item's centre and the room it was given.
    static func stack(
        _ weights: [CGFloat], in band: ClosedRange<CGFloat>, maxStep: CGFloat, minStep: CGFloat = 0
    ) -> [(y: CGFloat, room: CGFloat)] {
        guard !weights.isEmpty else { return [] }
        let units = weights.reduce(0, +)
        let length = band.upperBound - band.lowerBound
        let step = max(minStep, min(maxStep, length / max(units, 1)))
        let total = step * units
        var y = (band.lowerBound + band.upperBound) / 2 - total / 2
        return weights.map { weight in
            let room = step * weight
            defer { y += room }
            return (y + room / 2, room)
        }
    }

    /// Anything moving, so the timeline can stop when nothing is.
    var flows: Bool { arcs.contains { $0.style == .flowing } }

    // MARK: Traces

    static let hubPoint = hub(in: size)
    static let libraryPoint = library(in: size)

    /// The columns, left to right: folders (their names to the left of
    /// their dots, so the column sits far enough in for a 14-character
    /// name), tools, this computer, and the commons above it.
    static func folderX(_ space: CGSize) -> CGFloat { max(124, space.width * 0.3) }
    static func toolX(_ space: CGSize) -> CGFloat { space.width * 0.5 }
    static func hub(in space: CGSize) -> CGPoint {
        let band = band(space)
        return CGPoint(x: space.width * 0.72, y: (band.lowerBound + band.upperBound) / 2)
    }
    static func library(in space: CGSize) -> CGPoint {
        CGPoint(x: space.width * 0.84, y: band(space).lowerBound + space.height * 0.04)
    }

    /// A folder line's least and most height, the room left between tools
    /// and under a "+N" (a whole line, for its pill), and the least a tool
    /// takes, in lines: its node and the name under it, so a tool with one
    /// folder or none never meets the next at the least line height.
    static let folderStepRange: ClosedRange<CGFloat> = 22 ... 34
    static let toolGap: CGFloat = 0.9
    static let overflowLine: CGFloat = 1
    static let toolLines: CGFloat = 1.3

    /// How many folders each tool shows: the most, up to `foldersShown`,
    /// that let every tool's folders fit the band at the least line
    /// height; the rest are counted as "+N" (owner, 2026-10-14: a loaded
    /// map must not run off the pane).
    static func folderCap(_ tools: [TracesTree.ToolNode], band: ClosedRange<CGFloat>) -> Int {
        let length = band.upperBound - band.lowerBound
        for cap in stride(from: foldersShown, to: 1, by: -1)
        where units(tools, cap: cap) * folderStepRange.lowerBound <= length {
            return cap
        }
        return 1
    }

    /// The lines a tool takes at `cap` folders, `revealed` more drawn by its
    /// pill: its folders (`toolLines` at least), the pill when it has more
    /// than `cap`, and the gap after it.
    static func units(_ tool: TracesTree.ToolNode, cap: Int, revealed: Int = 0) -> CGFloat {
        max(toolLines, CGFloat(shown(tool, cap: cap, revealed: revealed)))
            + (tool.folders.count > cap ? overflowLine : 0) + toolGap
    }

    static func units(_ tools: [TracesTree.ToolNode], cap: Int, revealed: [String: Int] = [:]) -> CGFloat {
        tools.reduce(0) { $0 + units($1, cap: cap, revealed: revealed[$1.id] ?? 0) }
    }

    /// How many of a tool's folders are drawn: `cap`, and `revealed` more.
    static func shown(_ tool: TracesTree.ToolNode, cap: Int, revealed: Int) -> Int {
        min(tool.folders.count, cap + max(0, revealed))
    }

    /// The most folders drawn beside one tool; the rest are counted.
    static let foldersShown = 6
    /// A folder dot's radius.
    static let folderRadius: CGFloat = 7
    /// How many more folders one click of a tool's "+N" pill draws (owner,
    /// 2026-10-15).
    static let foldersRevealed = 6

    /// The folders each tool's pill has drawn after a click on `reveal`:
    /// the next few, or, from the pill that folds them back, none.
    static func revealing(_ reveal: FolderReveal, in revealed: [String: Int]) -> [String: Int] {
        var revealed = revealed
        if reveal.hidden > 0 {
            revealed[reveal.tool, default: 0] += reveal.next
        } else {
            revealed[reveal.tool] = nil
        }
        return revealed
    }

    /// The design space grown, with its shape kept, until the band holds
    /// `units` lines at the least line height: folders a pill has drawn
    /// past what fits zoom the map out rather than run off it.
    static func space(_ space: CGSize, holding units: CGFloat) -> CGSize {
        let band = band(space)
        let length = band.upperBound - band.lowerBound
        let needed = units * folderStepRange.lowerBound
        guard length > 0, needed > length else { return space }
        let factor = needed / length
        return CGSize(width: space.width * factor, height: space.height * factor)
    }

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

    /// Every tool this build knows, the tree's first, then the ones the
    /// tree leaves out as not watched (#1146 draws every tool, the
    /// unwatched ones dashed).
    static func mapTools(_ tree: TracesTree) -> [(tool: TracesTree.ToolNode, watched: Bool)] {
        let drawn = tree.tools.map { (tool: $0, watched: true) }
        let rest = SourceKind.allCases
            .filter { kind in !tree.tools.contains { $0.kind == kind } }
            .map { (tool: TracesTree.ToolNode(kind: $0, mode: .unset, folders: []), watched: false) }
        return drawn + rest
    }

    /// Tools and their folders, ringed by rule, joined to this computer.
    /// A tool with a folder set to contribute automatically is joined to
    /// the commons: moving only when the gate is open and the core says
    /// that tool's sessions go there, dashed otherwise, and never for a
    /// tool that is off. A tool that has contributed is joined to it by a
    /// quiet arc (#1146), and the library is lit once anything has been.
    ///
    /// `selectedTool` is the selection's tool: the others are faded, their
    /// library arcs most of all, and it is ringed (#1146 `TracesMap`).
    /// `revealed` is how many folders past the cap each tool's pill has
    /// drawn, by tool id (`revealing(_:in:)`).
    static func traces(
        _ tree: TracesTree, gate: CommonsGate, contributed: Contributions = .unread, selectedTool: String? = nil,
        revealed: [String: Int] = [:], space: CGSize = FlowMapScene.size
    ) -> FlowMapScene {
        var scene = FlowMapScene()
        let tools = mapTools(tree)
        // Each tool's room in the band is in proportion to the lines it
        // takes, so a tool with many folders never crowds its neighbour,
        // and the folders shown are as many as fit (`folderCap`), and as
        // many more as a pill has drawn, the map zoomed out to hold them.
        let cap = folderCap(tools.map(\.tool), band: band(space))
        let space = Self.space(space, holding: units(tools.map(\.tool), cap: cap, revealed: revealed))
        scene.space = space
        let hubPoint = hub(in: space)
        let libraryPoint = library(in: space)
        let folderX = folderX(space)
        let band = band(space)
        let toolUnits = tools.map { units($0.tool, cap: cap, revealed: revealed[$0.tool.id] ?? 0) }
        let slots = stack(
            toolUnits, in: band, maxStep: folderStepRange.upperBound, minStep: folderStepRange.lowerBound)
        let lineStep = slots.first.map { $0.room / toolUnits[0] } ?? folderStepRange.upperBound
        var flowing = false

        for (index, (tool, watched)) in tools.enumerated() {
            // The tool on its folders' middle line, the gap after them.
            let drawn = shown(tool, cap: cap, revealed: revealed[tool.id] ?? 0)
            let shownCount = CGFloat(max(1, drawn))
            let top = slots[index].y - slots[index].room / 2 + lineStep / 2
            let at = CGPoint(x: toolX(space), y: top + (shownCount - 1) * lineStep / 2)
            let folderGap = lineStep
            let off = tool.mode == .off
            let faded = selectedTool != nil && selectedTool != tool.id
            let toolDim = (off ? 0.4 : 1) * (faded ? 0.3 : 1)
            let armed = tool.folders.contains { $0.mode == .autoUpload }
            let sent = (contributed.tool(tool.kind) ?? 0) > 0
            let arcDim = (off ? 0.3 : 1) * (faded ? 0.12 : 1)

            if armed && !off {
                let moves = gate.open && gate.sendsToCommons(tool.kind)
                flowing = flowing || moves
                scene.arcs.append(curve(
                    from: CGPoint(x: at.x + 12, y: at.y - 6), to: libraryPoint, style: moves ? .flowing : .dashed, dim: arcDim))
            } else if sent {
                scene.arcs.append(curve(from: CGPoint(x: at.x + 12, y: at.y - 6), to: libraryPoint, style: .quiet, dim: arcDim))
            }
            let midX = (hubPoint.x + at.x) / 2
            scene.arcs.append(Arc(
                from: at, control1: CGPoint(x: midX, y: at.y), control2: CGPoint(x: midX, y: hubPoint.y),
                to: CGPoint(x: hubPoint.x - 16, y: hubPoint.y), style: .quiet, dim: toolDim))

            let shown = Array(tool.folders.prefix(drawn))
            for (folderIndex, folder) in shown.enumerated() {
                let point = CGPoint(x: folderX, y: at.y + (CGFloat(folderIndex) - CGFloat(shown.count - 1) / 2) * folderGap)
                let bend = (at.x + point.x) / 2
                scene.arcs.append(Arc(
                    from: CGPoint(x: at.x - 13, y: at.y), control1: CGPoint(x: bend, y: at.y),
                    control2: CGPoint(x: bend, y: point.y), to: CGPoint(x: point.x + 8, y: point.y),
                    style: .quiet, dim: toolDim))
                scene.nodes.append(Node(
                    id: "folder:\(folder.id)", kind: .folder(folder.mode), at: point, radius: folderRadius,
                    label: folder.label, detail: folderDetail(folder, contributed: contributed.folder(folder.id)),
                    dim: toolDim))
            }
            if tool.folders.count > cap {
                // A line under the last name, in the names' column.
                scene.reveals.append(FolderReveal(
                    tool: tool.id, toolName: tool.kind.displayName,
                    at: CGPoint(x: folderX, y: at.y + (CGFloat(shown.count - 1) / 2 + overflowLine) * folderGap),
                    hidden: tool.folders.count - shown.count, dim: toolDim))
            }
            scene.nodes.append(Node(
                id: "tool:\(tool.id)", kind: .tool(TracesTreeView.glassTool(tool.kind), off: off), at: at, radius: 13,
                label: tool.kind.displayName, detail: toolDetail(tool), dim: toolDim, title: toolTitle(tool),
                dashed: !watched, ringed: selectedTool == tool.id))
        }

        let toolsWaiting: Int = tree.tools.reduce(0) { $0 + $1.waiting }
        let unplacedWaiting: Int = tree.unplaced.reduce(0) { $0 + $1.sessions.count }
        let waiting = toolsWaiting + unplacedWaiting
        scene.nodes.insert(Node(
            id: "hub", kind: .hub, at: hubPoint, radius: 16,
            label: MonitorWords.computer,
            detail: words?.hub(waiting: waiting, contributed: contributed.total) ?? pair(MonitorWords.waiting, waiting)),
            at: 0)
        scene.nodes.append(Node(
            id: "library", kind: .library(active: flowing || (contributed.total ?? 0) > 0), at: libraryPoint, radius: 11,
            label: MonitorWords.commons,
            detail: words?.library(contributed: contributed.total) ?? pair(MonitorWords.table?.contributed ?? "", contributed.total)))
        return scene
    }

    /// The camera's centre for the binoculars (#1146 `cameraFor`): the
    /// tool's node, nil when it is not drawn.
    func focusPoint(tool id: String?) -> CGPoint? {
        guard let id else { return nil }
        return nodes.first { $0.id == "tool:\(id)" }?.at
    }

    /// The flow map's words (#1146 `FlowMap`), from the core.
    static var words: MonitorFlowMapCopy? { MonitorWords.table?.flowMap }

    /// A tool's card title: its name and how many folders (#1146).
    static func toolTitle(_ tool: TracesTree.ToolNode) -> String {
        words?.toolTitle(tool: tool.kind.displayName, folders: tool.folders.count) ?? tool.kind.displayName
    }

    /// A tool's card (#1146): what watching means, then what waits. Unset
    /// is never said as off, and unknown is said as neither.
    static func toolDetail(_ tool: TracesTree.ToolNode) -> String {
        let state: MonitorFlowMapCopy.ToolState = switch tool.mode {
        case .watch: .watched
        case .off: .off
        case .unset: .unset
        case .unknown: .unknown
        }
        return words?.tool(state, waiting: tool.waiting) ?? pair(MonitorWords.waiting, tool.waiting)
    }

    /// A folder's card (#1146): its path, its rule in the core's mode
    /// names, and its counts.
    static func folderDetail(_ folder: TracesTree.FolderNode, contributed: Int? = nil) -> String {
        words?.folder(
            path: folder.path, rule: folder.mode.map(ProjectCopy.modeChoiceLabel),
            waiting: folder.sessions.count, contributed: contributed)
            ?? pair(MonitorWords.waiting, folder.sessions.count)
    }

    // MARK: Private AI

    static let destinationPoint = destination(in: size)

    /// The destination, right of centre on the band's middle; the tools in
    /// a column to its left, their names to the left of their icons.
    static func destination(in space: CGSize) -> CGPoint {
        let band = band(space)
        return CGPoint(x: space.width * 0.68, y: (band.lowerBound + band.upperBound) / 2)
    }
    static func harnessX(_ space: CGSize) -> CGFloat { max(150, space.width * 0.36) }

    /// A tool line's least and most height in the Private AI map.
    static let harnessStepRange: ClosedRange<CGFloat> = 30 ... 72

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
        state: (HarnessRow) -> HarnessState,
        space: CGSize = FlowMapScene.size
    ) -> FlowMapScene {
        var scene = FlowMapScene()
        scene.space = space
        let rows = harnesses.harnesses
        let destinationPoint = destination(in: space)
        let x = harnessX(space)
        // Evenly down the band, no further apart than reads as one group,
        // centred on the destination; as many as fit at the least line
        // height, the rest counted as "+N" under them.
        let band = band(space)
        let fit = max(1, Int((band.upperBound - band.lowerBound) / harnessStepRange.lowerBound))
        let shown = rows.count > fit ? Array(rows.prefix(fit - 1)) : rows
        let weights = Array(repeating: CGFloat(1), count: shown.count) + (shown.count < rows.count ? [0.7] : [])
        let slots = stack(weights, in: band, maxStep: harnessStepRange.upperBound, minStep: harnessStepRange.lowerBound)
        if shown.count < rows.count, let last = slots.last {
            scene.overflows.append(Overflow(at: CGPoint(x: x - 24, y: last.y), count: rows.count - shown.count))
        }
        var anyAnswering = false
        var connected = 0

        for (index, row) in shown.enumerated() {
            let y = slots[index].y
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
            let bend = (x + destinationPoint.x) / 2
            scene.arcs.append(Arc(
                from: CGPoint(x: x + 12, y: y), control1: CGPoint(x: bend, y: y),
                control2: CGPoint(x: bend, y: destinationPoint.y), to: destinationPoint,
                style: style))
            scene.nodes.append(Node(
                id: "harness:\(row.id)",
                kind: .harness(glassTool(harness: row.id) ?? .other(initials: initials(row.name)), installed: row.installed),
                at: CGPoint(x: x, y: y), radius: 13, label: row.name, detail: sentence(row) ?? "—",
                dim: row.installed ? 1 : 0.5))
        }
        // The tools counted as "+N" still count: the credential says how
        // many are connected and whether any is answered, drawn or not.
        for row in rows.dropFirst(shown.count) {
            switch state(row) {
            case .answering:
                anyAnswering = true
                connected += 1
            case .connectedNoCalls, .activityShared:
                connected += 1
            case .notConnected, .unknown:
                break
            }
        }
        let lamp = lamp(privateAI, answering: anyAnswering)
        // #1146's credential card: how many tools are connected, in a
        // sentence; unknown is said, never left to read as off.
        var detail = words?.connected(tools: connected, sentence: true) ?? pair(MonitorWords.connected, connected)
        if lamp == .unknown { detail += " " + MonitorWords.unknown }
        // #1146: the credential, with how many tools it answers for, and
        // the destination's own state after it.
        scene.nodes.insert(Node(
            id: "destination", kind: .destination(lamp), at: destinationPoint, radius: 26,
            label: words?.credential ?? destinationLabel, detail: detail,
            sublabel: rows.isEmpty ? words?.noneFound : words?.connected(tools: connected, sentence: false)), at: 0)
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

    /// A tool with no artwork is drawn as two letters of its name (owner,
    /// 2026-10-14: a long list of blank circles reads as one): a word's
    /// first two ("Cursor" is "Cu"), or the first letters of its first two
    /// words ("GitHub Copilot CLI" is "GC").
    static func initials(_ name: String) -> String {
        let words = name.split(whereSeparator: { !$0.isLetter && !$0.isNumber })
        guard let first = words.first else { return "?" }
        if words.count > 1, let second = words.dropFirst().first?.first {
            return String(first.prefix(1)).uppercased() + String(second).uppercased()
        }
        return String(first.prefix(1)).uppercased() + String(first.dropFirst().prefix(1)).lowercased()
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

    /// A heading and a count the core may not have reported, set apart by
    /// a spaced dot: "Worth a second look · 3", a dash in place of the count.
    static func dotPair(_ word: String, _ count: Int?) -> String {
        "\(word) · \(count.map(String.init) ?? "—")"
    }

    private static func curve(from: CGPoint, to: CGPoint, style: Arc.Style, dim: Double) -> Arc {
        let midX = (from.x + to.x) / 2
        return Arc(from: from, control1: CGPoint(x: midX, y: from.y), control2: CGPoint(x: midX, y: to.y), to: to, style: style, dim: dim)
    }
}
