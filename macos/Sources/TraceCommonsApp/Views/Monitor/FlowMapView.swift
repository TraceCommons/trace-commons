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
    /// #1146 `right-3.5 bottom-3.5`: the zoom sits 14 in.
    static let overlayInset: CGFloat = 14

    /// The card's hint (#1146 `NodeCard`): shown while a card is peeked by
    /// hovering or focus, not once it is pinned.
    static func hint(pinned: Bool) -> String? {
        pinned ? nil : FlowMapScene.words?.hint
    }

    var body: some View {
        GeometryReader { proxy in
            let fit = FlowMapGeometry(size: proxy.size, zoom: zoom)
            ZStack(alignment: .topLeading) {
                TimelineView(.animation(minimumInterval: 1 / 30, paused: reduceMotion || !scene.flows || !state.isHealthy)) { timeline in
                    Canvas { context, _ in
                        draw(in: &context, fit: fit, phase: reduceMotion ? 0 : timeline.date.timeIntervalSinceReferenceDate * 18)
                    }
                }
                .accessibilityHidden(true)

                ForEach(scene.nodes) { node in
                    target(node, fit: fit)
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
                if zoomable { zoomControls.padding(Self.overlayInset) }
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

        if !legend.isEmpty { drawLegend(in: &context, fit: fit) }
    }

    private func drawNode(_ node: FlowMapScene.Node, in context: inout GraphicsContext, fit: FlowMapGeometry) {
        let centre = fit.point(node.at)
        let radius = node.radius * fit.scale
        let disc = Path(ellipseIn: CGRect(x: centre.x - radius, y: centre.y - radius, width: radius * 2, height: radius * 2))
        var layer = context
        layer.opacity = node.dim
        let selected = selection == node.id

        switch node.kind {
        case .hub:
            layer.fill(disc, with: .color(GlassTokens.Color.statusOff.color))
            ring(disc, radius: radius, centre: centre, in: &layer, colour: GlassColor.ink(0.25), scale: fit.scale)
        case .library(let active):
            layer.fill(disc, with: .color(active ? GlassTokens.Color.blue.color : GlassTokens.Color.mapNodeOff.color))
            if active { ring(disc, radius: radius, centre: centre, in: &layer, colour: GlassTokens.Color.blue.color.opacity(0.35), scale: fit.scale) }
        case .tool(let tool, let off):
            layer.fill(disc, with: .color(GlassColor.ink(0.12)))
            mark(tool, centre: centre, side: 16 * fit.scale, in: &layer)
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
            // protection is claimed for it (#1146).
            layer.draw(Text(Image(systemName: "key.fill")).font(.system(size: 14 * fit.scale)).foregroundStyle(GlassColor.textPrimary), at: centre)
        }

        if selected {
            layer.stroke(disc, with: .color(GlassColor.accentText), lineWidth: 2 * fit.scale)
        }

        let label = node.label.count > 18 ? node.label.prefix(17) + "…" : Substring(node.label)
        layer.draw(
            Text(label).font(GlassTokens.TypeScale.micro.font.weight(.semibold)).foregroundStyle(GlassColor.textSecondary),
            at: CGPoint(x: centre.x, y: centre.y + radius + 9 * fit.scale))
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

    private func drawLegend(in context: inout GraphicsContext, fit: FlowMapGeometry) {
        var x: CGFloat = 24
        for mode in legend {
            let rule = Self.rule(mode)
            let centre = fit.point(CGPoint(x: x, y: 730))
            let r = 5 * fit.scale
            let disc = Path(ellipseIn: CGRect(x: centre.x - r, y: centre.y - r, width: r * 2, height: r * 2))
            context.fill(disc, with: .color(rule.fill.color))
            context.stroke(disc, with: .color(rule.stroke.color), lineWidth: 1.5 * fit.scale)
            let text = context.resolve(
                Text(ProjectCopy.modeChoiceLabel(mode)).font(GlassTokens.TypeScale.micro.font.weight(.regular))
                    .foregroundStyle(GlassColor.textTertiary))
            let size = text.measure(in: CGSize(width: 400, height: 40))
            context.draw(text, at: CGPoint(x: centre.x + r + 4, y: centre.y), anchor: .leading)
            x += (r * 2 + 4 + size.width + 14) / fit.scale
        }
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

    private func target(_ node: FlowMapScene.Node, fit: FlowMapGeometry) -> some View {
        let side = max(24, (node.radius * 2 + 8) * fit.scale)
        return Button {
            pinned = pinned == node.id ? nil : node.id
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
        .accessibilityLabel(node.label)
        .accessibilityValue(node.detail)
        .accessibilityAddTraits(pinned == node.id ? .isSelected : [])
        .position(fit.point(node.at))
    }

    private func card(_ node: FlowMapScene.Node, fit: FlowMapGeometry, in size: CGSize) -> some View {
        let centre = fit.point(node.at)
        let width = GlassTokens.Size.nodeCardWidth
        // Beside the node, kept inside the field.
        let x = min(max(width / 2 + 8, centre.x + node.radius * fit.scale + 12 + width / 2), size.width - width / 2 - 8)
        let y = min(max(48, centre.y - 24), size.height - 48)
        let peeked = (hovered ?? focused) == node.id
        return GlassFloatingGroup {
            GlassNodeCard(node.cardTitle, detail: node.detail, hint: Self.hint(pinned: !peeked))
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
/// about the centre.
struct FlowMapGeometry {
    let scale: CGFloat
    let origin: CGPoint

    init(size: CGSize, zoom: CGFloat) {
        let design = FlowMapScene.size
        let fit = min(size.width / design.width, size.height / design.height)
        scale = max(0.01, fit * zoom)
        origin = CGPoint(x: (size.width - design.width * scale) / 2, y: (size.height - design.height * scale) / 2)
    }

    func point(_ p: CGPoint) -> CGPoint {
        CGPoint(x: origin.x + p.x * scale, y: origin.y + p.y * scale)
    }
}
