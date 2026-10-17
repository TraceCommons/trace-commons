import SwiftUI
import TCDesign
import TCShellCore

/// The flow map (R8 of #1173): a `FlowMapScene` drawn in a `Canvas`, with
/// moving dashes on the arcs that carry something (none under Reduce
/// Motion), and a node card on hover or click.
///
/// The drawing is a picture; the nodes are not. Each node is also an
/// accessible button with its name and its card's detail as the value, so
/// VoiceOver and Full Keyboard Access can inspect what a pointer can.
struct FlowMapView: View {
    let scene: FlowMapScene
    /// The rule legend under the map (Traces only).
    let legend: [ProjectMode]
    let zoomable: Bool
    let accessibilityName: String
    /// The stack-wide state (ScreenState). Only `ready` draws anything as
    /// moving: paused, unknown and core-down arcs are drawn still, so a
    /// missing or stale signal never reads as live.
    var state: ScreenState = .ready
    /// The binoculars' point in the design space (#1146 `cameraFor`): the
    /// map is scaled up about it, and every node stays drawn.
    var focus: CGPoint? = nil
    /// A folder node was clicked: its project id, so the window selects the
    /// folder in the tree and shows it in the inspector (owner, 2026-10-09).
    var onSelectFolder: ((String) -> Void)? = nil
    /// A tool's folder pill was clicked (`FlowMapScene.revealing(_:in:)`).
    var onReveal: ((FlowMapScene.FolderReveal) -> Void)? = nil
    /// The traces moving across the Traces map now (`MapTransferPlayer`).
    var transferRun: MapTransferRun? = nil

    @State private var zoom: CGFloat = 1
    @State private var hovered: String?
    @State private var pinned: String?
    /// The node keyboard focus is on (Full Keyboard Access). It selects the
    /// node as a hover does, so a keyboard user sees the same ring and card
    /// a pointer gets; the system focus ring stays on the button itself.
    @FocusState private var focused: String?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.colorSchemeContrast) private var contrast

    static let zoomRange: ClosedRange<CGFloat> = 0.6 ... 2
    static let zoomStep: CGFloat = 0.25

    var body: some View {
        GeometryReader { proxy in
            let fit = FlowMapGeometry(size: proxy.size, zoom: zoom, focus: focus, space: scene.space)
            ZStack(alignment: .topLeading) {
                TimelineView(.animation(minimumInterval: 1 / 30, paused: reduceMotion || !scene.flows || !state.isHealthy)) { timeline in
                    Canvas { context, _ in
                        draw(in: &context, fit: fit, phase: reduceMotion ? 0 : timeline.date.timeIntervalSinceReferenceDate * 18)
                    }
                }
                .accessibilityHidden(true)

                // The Traces map's library is a station, its tools a pile of
                // coins, and what moves is drawn over both (owner,
                // 2026-10-16).
                if scene.stack != nil, let library = scene.nodes.first(where: { $0.id == "library" }),
                   case .library(let active) = library.kind {
                    GlassLibraryStation(lit: active)
                        .frame(width: 150 * fit.scale, height: 130 * fit.scale)
                        .position(fit.point(library.at))
                        .allowsHitTesting(false)
                }
                if let stack = scene.stack {
                    TimelineView(.animation(minimumInterval: 1 / 60, paused: transferRun == nil)) { timeline in
                        let t = transferRun.map { timeline.date.timeIntervalSince($0.start) } ?? 0
                        ZStack(alignment: .topLeading) {
                            MapToolPile(
                                stack: stack, nodes: scene.nodes.filter { $0.id.hasPrefix("tool:") }, fit: fit,
                                run: transferRun, t: t,
                                folderLabel: { id in scene.nodes.first { $0.id == "folder:\(id)" }?.label })
                            Canvas { context, _ in
                                if let run = transferRun {
                                    MapTransferDrawing.draw(in: &context, scene: scene, run: run, t: t, fit: fit)
                                }
                                // Between contributions the rocket stands on the pad.
                                if transferRun?.action != .contribute {
                                    MapTransferDrawing.restingRocket(scene: scene, fit: fit, in: &context)
                                }
                            }
                            .allowsHitTesting(false)
                            .accessibilityHidden(true)
                        }
                        .frame(width: proxy.size.width, height: proxy.size.height, alignment: .topLeading)
                    }
                }

                ForEach(scene.nodes) { node in
                    target(node, fit: fit)
                }

                ForEach(scene.reveals, id: \.tool) { reveal in
                    revealPill(reveal, fit: fit)
                }

                if let id = selection, let node = scene.nodes.first(where: { $0.id == id }) {
                    card(node, fit: fit, in: proxy.size)
                }
            }
            .frame(width: proxy.size.width, height: proxy.size.height)
            .contentShape(Rectangle())
            // A click on the field, not a node, lets go of the pinned card.
            .onTapGesture { pinned = nil }
            .overlay(alignment: .bottomTrailing) {
                if zoomable { zoomControls.padding(GlassTokens.Space.mapOverlayInset) }
            }
            // The legend stays at the window's foot, whatever the map's
            // zoom or focus (owner, 2026-10-12): it is not drawn in the
            // map's own space, which scales and moves.
            .overlay(alignment: .bottomLeading) {
                if !legend.isEmpty {
                    legendView
                        .padding(GlassTokens.Space.mapOverlayInset)
                        .accessibilityHidden(true)
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(accessibilityName)
        .onChange(of: scene) { _, scene in
            if let pinned, !scene.nodes.contains(where: { $0.id == pinned }) { self.pinned = nil }
            if let focused, !scene.nodes.contains(where: { $0.id == focused }) { self.focused = nil }
        }
    }

    // MARK: Drawing

    private func draw(in context: inout GraphicsContext, fit: FlowMapGeometry, phase: Double) {
        for arc in scene.arcs {
            var path = Path()
            path.move(to: fit.point(arc.from))
            path.addCurve(to: fit.point(arc.to), control1: fit.point(arc.control1), control2: fit.point(arc.control2))
            let width = 1.4 * fit.scale
            // Increase Contrast: the quiet links are drawn plainly enough to
            // follow (R14).
            let strong = contrast == .increased
            switch arc.style {
            case .quiet:
                context.stroke(path, with: .color(GlassColor.ink((strong ? 0.5 : 0.2) * arc.dim)), lineWidth: width)
            case .dashed:
                context.stroke(path, with: .color(GlassColor.ink((strong ? 0.45 : 0.18) * arc.dim)),
                               style: StrokeStyle(lineWidth: width, dash: [5 * fit.scale, 5 * fit.scale]))
            case .shared:
                // Lit but still: calls in this family arrived, not
                // attributable to this tool.
                context.stroke(path, with: .color(GlassTokens.Color.statusOn.color.opacity(0.35 * arc.dim)), lineWidth: width)
            case .unknown:
                // The core could not say: dotted, never solid.
                context.stroke(path, with: .color(.white.opacity(0.18 * arc.dim)),
                               style: StrokeStyle(lineWidth: width, lineCap: .round, dash: [0.5 * fit.scale, 4 * fit.scale]))
            case .flowing where !state.isHealthy:
                // Paused, unknown or stale: still, and dashed, not live.
                context.stroke(path, with: .color(GlassColor.ink((strong ? 0.45 : 0.25) * arc.dim)),
                               style: StrokeStyle(lineWidth: width, dash: [5 * fit.scale, 5 * fit.scale]))
            case .flowing:
                context.stroke(path, with: .color(GlassTokens.Color.statusOn.color.opacity(0.35 * arc.dim)), lineWidth: width)
                context.stroke(path, with: .color(GlassTokens.Color.statusOn.color.opacity(arc.dim)),
                               style: StrokeStyle(lineWidth: width * 1.3, lineCap: .round,
                                                  dash: [4 * fit.scale, 8 * fit.scale], dashPhase: -phase * fit.scale))
            }
        }

        for node in scene.nodes {
            drawNode(node, in: &context, fit: fit)
        }

        for overflow in scene.overflows {
            context.draw(
                Text("+\(overflow.count)").font(GlassTokens.TypeScale.micro.font).foregroundStyle(GlassColor.textTertiary),
                at: fit.point(overflow.at))
        }

    }

    private func drawNode(_ node: FlowMapScene.Node, in context: inout GraphicsContext, fit: FlowMapGeometry) {
        // On the Traces map (owner, 2026-10-16) the tools are the pile's
        // coins (`MapToolPile`) and the library is the station
        // (`GlassLibraryStation`), both drawn over this canvas; this
        // computer is a launch pad.
        let traces = scene.stack != nil
        if traces, case .tool = node.kind { return }
        let centre = fit.point(node.at)
        let radius = node.radius * fit.scale
        let disc = Path(ellipseIn: CGRect(x: centre.x - radius, y: centre.y - radius, width: radius * 2, height: radius * 2))
        var layer = context
        layer.opacity = node.dim
        let selected = selection == node.id
        let drawnElsewhere = traces && (node.id == "hub" || node.id == "library")

        switch node.kind {
        case .hub where traces:
            Self.drawPad(at: centre, scale: fit.scale, in: &layer)
        case .hub:
            layer.fill(disc, with: .color(GlassTokens.Color.statusOff.color))
            ring(disc, radius: radius, centre: centre, in: &layer, colour: GlassColor.ink(0.25), scale: fit.scale)
        case .library where traces:
            // The station's name and count show only under the pointer
            // (owner, 2026-10-16); the card says them too.
            guard selected else { return }
        case .library(let active):
            layer.fill(disc, with: .color(active ? GlassTokens.Color.blue.color : GlassTokens.Color.mapNodeOff.color))
            if active { ring(disc, radius: radius, centre: centre, in: &layer, colour: GlassTokens.Color.blue.color.opacity(0.35), scale: fit.scale) }
        case .tool(let tool, let off):
            layer.fill(disc, with: .color(GlassColor.ink(0.12)))
            mark(tool, centre: centre, side: 16 * fit.scale, in: &layer)
            if node.dashed {
                // The legend's "Not watched" (#1146's "Tool not watched"): a dashed rim.
                layer.stroke(disc, with: .color(GlassTokens.Color.statusOff.color),
                             style: StrokeStyle(lineWidth: 1.5 * fit.scale, dash: [2 * fit.scale, 2 * fit.scale]))
            }
            if node.ringed {
                ring(disc, radius: radius, centre: centre, in: &layer, colour: GlassColor.ink(0.35), scale: fit.scale)
            }
            if off {
                var slash = Path()
                slash.move(to: CGPoint(x: centre.x - 9 * fit.scale, y: centre.y - 9 * fit.scale))
                slash.addLine(to: CGPoint(x: centre.x + 9 * fit.scale, y: centre.y + 9 * fit.scale))
                layer.stroke(slash, with: .color(GlassColor.textPrimary), lineWidth: 2 * fit.scale)
            }
        case .folder(let mode):
            let rule = Self.rule(mode)
            layer.fill(disc, with: .color(rule.fill.color))
            layer.stroke(disc, with: .color(rule.stroke.color), lineWidth: 2 * fit.scale)
        case .harness(let tool, _):
            layer.fill(disc, with: .color(GlassColor.ink(0.12)))
            if let tool { mark(tool, centre: centre, side: 16 * fit.scale, in: &layer) }
        case .destination(let lamp):
            let lit = lamp == .answering || lamp == .running
            layer.fill(disc, with: .color(lit ? GlassTokens.Color.mapCredentialOn.color : GlassTokens.Color.mapNodeOff.color))
            if lamp == .answering { ring(disc, radius: radius, centre: centre, in: &layer, colour: GlassTokens.Color.statusOn.color.opacity(0.35), scale: fit.scale) }
            if lamp == .unknown {
                // Unknown is not off: a dotted rim says the core did not say.
                layer.stroke(disc, with: .color(.white.opacity(0.5)),
                             style: StrokeStyle(lineWidth: 1.5 * fit.scale, lineCap: .round, dash: [0.5 * fit.scale, 4 * fit.scale]))
            }
            // A key, not a shield: the node is the credential, and no
            // protection is claimed for it. #1146's outlined, level key.
            layer.stroke(Self.keyGlyph(centre: centre, scale: fit.scale), with: .color(GlassColor.textPrimary),
                         style: StrokeStyle(lineWidth: 2 * fit.scale, lineCap: .round, lineJoin: .round))
        }

        if selected, !drawnElsewhere {
            layer.stroke(disc, with: .color(GlassColor.accentText), lineWidth: 2 * fit.scale)
        }

        // #1146's `.tc-map__label` and `.tc-map__sublabel`: 11 semibold in
        // the primary ink, then 10 regular in the off tone, 14 below. Type
        // and gaps scale together, so the two lines never meet.
        let type = Self.labelType(scale: fit.scale)
        // Folder names are cut (they are the person's, and run long); the
        // map's own words and the tools' names are drawn whole.
        if node.labelBeside {
            // A folder's name sits to the left of its dot, on its line, so a
            // folder takes one line of height and a long list stays legible
            // (owner, 2026-10-14: the map under load).
            let text = if case .folder = node.kind { Self.shortLabel(node.label) } else { node.label }
            layer.draw(
                Text(text).font(.system(size: type.labelSize, weight: .semibold))
                    .foregroundStyle(GlassColor.textPrimary),
                at: CGPoint(x: centre.x - radius - 6 * max(fit.scale, Self.labelFloor), y: centre.y), anchor: .trailing)
        } else {
            layer.draw(
                Text(node.label).font(.system(size: type.labelSize, weight: .semibold)).foregroundStyle(GlassColor.textPrimary),
                at: CGPoint(x: centre.x, y: centre.y + radius + type.labelOffset))
        }
        if let sublabel = node.sublabel {
            layer.draw(
                Text(sublabel).font(.system(size: type.sublabelSize))
                    .foregroundStyle(GlassTokens.Color.statusOff.color),
                at: CGPoint(x: centre.x, y: centre.y + radius + type.sublabelOffset))
        }
    }

    /// This computer as a launch pad (owner, 2026-10-16): a slab on a base,
    /// with a service tower beside it; the line out lands on its left edge
    /// (`FlowMapScene.padHalfWidth`) and the rocket stands on it.
    static func drawPad(at centre: CGPoint, scale s: CGFloat, in context: inout GraphicsContext) {
        let half = FlowMapScene.padHalfWidth * s
        let slab = Path(roundedRect: CGRect(x: centre.x - half, y: centre.y - 5 * s, width: half * 2, height: 10 * s),
                        cornerRadius: 4 * s)
        context.fill(slab, with: .linearGradient(
            Gradient(colors: [GlassTokens.Color.statusOff.color.opacity(0.75), GlassTokens.Color.statusOff.color]),
            startPoint: CGPoint(x: centre.x, y: centre.y - 5 * s), endPoint: CGPoint(x: centre.x, y: centre.y + 5 * s)))
        context.fill(Path(roundedRect: CGRect(x: centre.x - 24 * s, y: centre.y + 5 * s, width: 48 * s, height: 6 * s),
                          cornerRadius: 3 * s), with: .color(GlassTokens.Color.mapNodeOff.color))
        let steel = GlassTokens.Color.statusOff.color.opacity(0.7)
        context.fill(Path(roundedRect: CGRect(x: centre.x + 28 * s, y: centre.y - 55 * s, width: 4 * s, height: 50 * s),
                          cornerRadius: 2 * s), with: .color(steel))
        context.fill(Path(roundedRect: CGRect(x: centre.x + 14 * s, y: centre.y - 43 * s, width: 16 * s, height: 3 * s),
                          cornerRadius: 1.5 * s), with: .color(steel))
        context.fill(Path(roundedRect: CGRect(x: centre.x + 18 * s, y: centre.y - 27 * s, width: 12 * s, height: 3 * s),
                          cornerRadius: 1.5 * s), with: .color(steel))
    }

    /// A node's label and sublabel: their sizes, and how far below the disc
    /// each line's centre sits. In #1146's SVG, type scales with the map;
    /// here it does too, down to a floor that keeps it legible, and the
    /// gaps use the same factor as the type.
    static func labelType(scale: CGFloat) -> (labelSize: CGFloat, sublabelSize: CGFloat,
                                              labelOffset: CGFloat, sublabelOffset: CGFloat) {
        let unit = max(scale, labelFloor)
        return (11 * unit, 10 * unit, 12 * unit, 26.5 * unit)
    }

    /// The smallest factor the map's labels are drawn at.
    static let labelFloor: CGFloat = 0.85

    /// #1146's credential key: an open bow on the left, a level shaft and
    /// two bits, in the map's units around the node's centre.
    static func keyGlyph(centre: CGPoint, scale: CGFloat) -> Path {
        var path = Path()
        let bow = CGPoint(x: centre.x - 9 * scale, y: centre.y)
        path.addEllipse(in: CGRect(x: bow.x - 6 * scale, y: bow.y - 6 * scale, width: 12 * scale, height: 12 * scale))
        path.move(to: CGPoint(x: centre.x - 3 * scale, y: centre.y))
        path.addLine(to: CGPoint(x: centre.x + 11 * scale, y: centre.y))
        path.move(to: CGPoint(x: centre.x + 7 * scale, y: centre.y))
        path.addLine(to: CGPoint(x: centre.x + 7 * scale, y: centre.y + 5 * scale))
        path.move(to: CGPoint(x: centre.x + 11 * scale, y: centre.y))
        path.addLine(to: CGPoint(x: centre.x + 11 * scale, y: centre.y + 4 * scale))
        return path
    }

    private func ring(_ disc: Path, radius: CGFloat, centre: CGPoint, in context: inout GraphicsContext, colour: Color, scale: CGFloat) {
        let metrics = Self.ringMetrics(scale: scale)
        let outer = radius + metrics.offset
        context.stroke(
            Path(ellipseIn: CGRect(x: centre.x - outer, y: centre.y - outer, width: outer * 2, height: outer * 2)),
            with: .color(colour), lineWidth: metrics.lineWidth)
    }

    /// A node's halo, in design units scaled with the map: its gap from the
    /// disc and its width grow and shrink with zoom, as the disc does.
    static func ringMetrics(scale: CGFloat) -> (offset: CGFloat, lineWidth: CGFloat) {
        (4 * scale, 3 * scale)
    }

    /// The node whose ring and card are drawn: the hovered one, then the
    /// one with keyboard focus, then the pinned one.
    private var selection: String? {
        hovered ?? focused ?? pinned
    }

    /// The tool's logo in its tint, or its initials when it has none.
    private func mark(_ tool: GlassTool, centre: CGPoint, side: CGFloat, in context: inout GraphicsContext) {
        let rect = CGRect(x: centre.x - side / 2, y: centre.y - side / 2, width: side, height: side)
        if let logo = tool.logo {
            context.fill(GlassToolLogoShape(logo).path(in: rect), with: .color(tool.tint.color))
        } else {
            context.draw(Text(tool.initials).font(GlassTokens.TypeScale.micro.font).foregroundStyle(tool.tint.color), at: centre)
        }
    }

    /// The rule legend, then #1146's last item: a tool that is not watched,
    /// a dashed ring. Fixed in size, at the map's foot.
    private var legendView: some View {
        HStack(spacing: GlassTokens.Space.s5) {
            ForEach(legend, id: \.self) { mode in
                let rule = Self.rule(mode)
                legendItem(ProjectCopy.modeChoiceLabel(mode)) {
                    Circle().fill(rule.fill.color).overlay(Circle().stroke(rule.stroke.color, lineWidth: 1.5))
                }
            }
            if let word = FlowMapScene.words?.legendNotWatched {
                legendItem(word) {
                    Circle().stroke(GlassTokens.Color.statusOff.color, style: StrokeStyle(lineWidth: 1.5, dash: [2, 2]))
                }
            }
            // The lock on each coin: a tool's memory stays on this Mac.
            if scene.stack != nil, let word = FlowMapScene.words?.legendMemory {
                legendItem(word) {
                    Image(systemName: "lock").font(.system(size: 8, weight: .semibold))
                        .foregroundStyle(GlassColor.textTertiary)
                }
            }
        }
        .fixedSize()
    }

    private func legendItem(_ word: String, @ViewBuilder dot: () -> some View) -> some View {
        HStack(spacing: GlassTokens.Space.s2) {
            dot().frame(width: 10, height: 10)
            Text(word)
                .glassType(GlassTokens.TypeScale.micro.weight(.regular))
                .foregroundStyle(GlassColor.textTertiary)
        }
    }

    /// A folder's name on the map: at most `labelLimit` characters, the
    /// last an ellipsis when it is cut (owner, 2026-10-12). The full name is
    /// the node's tooltip and accessible name.
    static let labelLimit = 14
    static func shortLabel(_ label: String) -> String {
        label.count > labelLimit ? String(label.prefix(labelLimit - 1)) + "\u{2026}" : label
    }

    /// A folder's dot by rule: the fill and the ring.
    static func rule(_ mode: ProjectMode?) -> (fill: GlassRGBA, stroke: GlassRGBA) {
        switch mode {
        case .autoUpload: (GlassTokens.Color.mapRuleAutoFill, GlassTokens.Color.statusOn)
        case .ask: (GlassTokens.Color.mapRuleAskFill, GlassTokens.Color.statusAsk)
        case .ignore: (GlassTokens.Color.mapRuleIgnoreFill, GlassTokens.Color.statusOutside)
        case nil: (GlassTokens.Color.mapRuleUnsetFill, GlassTokens.Color.statusOff)
        }
    }

    // MARK: Nodes as controls

    /// A folder node's project id (`FlowMapScene` names them `folder:<id>`).
    static func folderID(_ nodeID: String) -> String? {
        nodeID.hasPrefix("folder:") ? String(nodeID.dropFirst("folder:".count)) : nil
    }

    private func target(_ node: FlowMapScene.Node, fit: FlowMapGeometry) -> some View {
        let side = max(24, (node.radius * 2 + 8) * fit.scale)
        return Button {
            pinned = pinned == node.id ? nil : node.id
            if let onSelectFolder, let projectID = Self.folderID(node.id) { onSelectFolder(projectID) }
        } label: {
            Circle().fill(Color.clear).frame(width: side, height: side).contentShape(Circle())
        }
        .buttonStyle(.plain)
        // Focusable under Full Keyboard Access, with the system focus ring
        // (#1206 chose it over a drawn outline); focus also selects the
        // node, so its ring and card show.
        .focused($focused, equals: node.id)
        .onHover { inside in
            if inside { hovered = node.id } else if hovered == node.id { hovered = nil }
        }
        .help(node.label)
        .allowsHitTesting(scene.stack == nil || !node.id.hasPrefix("tool:"))
        .accessibilityLabel(node.label)
        .accessibilityValue(node.detail)
        .accessibilityAddTraits(pinned == node.id ? .isSelected : [])
        .position(fit.point(node.at))
    }

    /// The folder pill (owner, 2026-10-15; one column of folders since
    /// 2026-10-16): "+N" draws the next few folders, and once they are all
    /// drawn, the core's fold-back word folds them away again. Its trailing
    /// edge sits where the folder names end, on the line under the last one. It is a glass control pill in
    /// the names' own type, so it scales with the map and fits one folder
    /// line however tightly the lines are packed. Without the core's words
    /// only the count, a figure, is drawn.
    @ViewBuilder
    private func revealPill(_ reveal: FlowMapScene.FolderReveal, fit: FlowMapGeometry) -> some View {
        let words = FlowMapScene.words
        let folds = reveal.hidden == 0
        if let text = folds ? words?.foldFolders : "+\(reveal.hidden)" {
            let name = folds ? words?.foldFoldersLabel : words?.revealFolders(reveal.next)
            let unit = max(fit.scale, Self.labelFloor)
            let size = Self.labelType(scale: fit.scale).labelSize
            let column = fit.point(reveal.at)
            let end = column.x - FlowMapScene.folderRadius * fit.scale - 6 * unit
            Button { onReveal?(reveal) } label: {
                Text(text)
                    .font(.system(size: size, weight: .semibold))
                    .monospacedDigit()
                    .foregroundStyle(GlassColor.textPrimary)
                    .padding(.horizontal, 8 * unit)
                    .frame(height: size + 8 * unit)
                    .glassSurface(.control, hover: GlassTokens.Color.controlHover)
                    .contentShape(Capsule())
            }
            .buttonStyle(GlassPressStyle())
            .fixedSize()
            .opacity(reveal.dim)
            .help(name ?? text)
            .accessibilityLabel(name ?? text)
            .frame(width: 0, height: 0, alignment: .trailing)
            .position(x: end, y: column.y)
        }
    }

    private func card(_ node: FlowMapScene.Node, fit: FlowMapGeometry, in size: CGSize) -> some View {
        let centre = fit.point(node.at)
        let width = GlassTokens.Size.nodeCardWidth
        // Beside the node, kept inside the field.
        let x = min(max(width / 2 + 8, centre.x + node.radius * fit.scale + 12 + width / 2), size.width - width / 2 - 8)
        let y = min(max(48, centre.y - 24), size.height - 48)
        return GlassFloatingGroup {
            GlassNodeCard(node.cardTitle, detail: node.detail)
        }
        .position(x: x, y: y)
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }

    // MARK: Zoom

    private var zoomControls: some View {
        GlassFloatingGroup {
            GlassToolbarGroup {
                GlassToolbarButton(MonitorWords.reduce, systemImage: "minus") {
                    zoom = max(Self.zoomRange.lowerBound, zoom - Self.zoomStep)
                }
                .disabled(zoom <= Self.zoomRange.lowerBound)
                GlassToolbarButton(MonitorWords.enlarge, systemImage: "plus") {
                    zoom = min(Self.zoomRange.upperBound, zoom + Self.zoomStep)
                }
                .disabled(zoom >= Self.zoomRange.upperBound)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(FlowMapScene.words?.zoomLabel ?? "")
    }
}

/// The design space (580×760) fitted into the field, centred, and zoomed
/// about the centre; or, with a focus, scaled 1.6× about that point as
/// #1146's camera is (`cameraFor`).
struct FlowMapGeometry {
    let scale: CGFloat
    let origin: CGPoint

    static let focusScale: CGFloat = 1.6

    /// `space` is the scene's design space (`FlowMapScene.space`), #1146's
    /// 580×760 by default.
    init(size: CGSize, zoom: CGFloat, focus: CGPoint? = nil, space: CGSize = FlowMapScene.size) {
        let design = space
        let fit = min(size.width / design.width, size.height / design.height)
        let centred = CGPoint(x: (size.width - design.width * fit) / 2, y: (size.height - design.height * fit) / 2)
        if let focus {
            // #1146: translate(290 - 1.6(x - 50), 380 - 1.6y) scale(1.6), in
            // design units, then fitted; 290 and 380 are the space's
            // middle, 50 its nudge right of the focused node.
            let k = Self.focusScale
            scale = max(0.01, fit * k)
            origin = CGPoint(
                x: centred.x + fit * (design.width / 2 - k * (focus.x - 50)),
                y: centred.y + fit * (design.height / 2 - k * focus.y))
        } else {
            scale = max(0.01, fit * zoom)
            origin = CGPoint(x: (size.width - design.width * scale) / 2, y: (size.height - design.height * scale) / 2)
        }
    }

    func point(_ p: CGPoint) -> CGPoint {
        CGPoint(x: origin.x + p.x * scale, y: origin.y + p.y * scale)
    }
}
