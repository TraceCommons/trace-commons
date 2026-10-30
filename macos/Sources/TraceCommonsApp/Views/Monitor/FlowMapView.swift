import SwiftUI
import TCDesign
import TCShellCore

/// The flow map (R8 of #1173): a `FlowMapScene` drawn in a `Canvas`, with
/// moving dashes on the arcs that carry something (none under Reduce
/// Motion), and a node card on hover or keyboard focus.
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
    /// The traces the cards at the map's foot describe, oldest first
    /// (`MapTransferPlayer.cards`); none for no cards.
    var transferCards: [MapTransfer] = []
    /// A card's X, by the trace's id (`MapTransferPlayer.close(_:)`).
    var onCloseTransfer: ((String) -> Void)? = nil
    /// The Traces map's style: the rocket and station, or the laptop and
    /// the library orb (`TracesMapStyle`, Settings, General).
    var mapStyle: TracesMapStyle = .launch
    /// The + under the other devices (`FlowMapScene.addDevice`): starts
    /// linking a server. Nil draws no +.
    var onAddDevice: (() -> Void)? = nil
    /// The +'s accessible name and tooltip: the Devices section's own.
    var addDeviceLabel: String = ""

    @State private var zoom: CGFloat = 1
    @State private var hovered: String?
    /// Where the pointer is on the map, for the tool group's own hit tests
    /// (owner, 2026-10-19: the pile opens only over its card).
    @State private var pointer: CGPoint?
    /// The node keyboard focus is on (Full Keyboard Access). It selects the
    /// node as a hover does, so a keyboard user sees the same ring and card
    /// a pointer gets; the system focus ring stays on the button itself.
    @FocusState private var focused: String?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.colorSchemeContrast) private var contrast
    @Environment(\.colorScheme) private var colorScheme

    static let zoomRange: ClosedRange<CGFloat> = 0.6 ... 2
    static let zoomStep: CGFloat = 0.25

    /// The run the map draws: none under Reduce Motion, where the map
    /// stays still and only the card says what is happening.
    private var motion: MapTransferRun? { reduceMotion ? nil : transferRun }

    var body: some View {
        GeometryReader { proxy in
            let fit = FlowMapGeometry(size: proxy.size, zoom: zoom, focus: focus, space: scene.space)
            let library = libraryOnMap
            let resting = Self.restingMotion(
                reduceMotion: reduceMotion, flows: scene.flows, healthy: state.isHealthy, library: library != nil)
            ZStack(alignment: .topLeading) {
                // One timeline for everything on the map that moves at
                // rest: the arcs' dashes and the library. Only what moves
                // is redrawn on a tick: the arcs (a few strokes) and the
                // library; the nodes, their names and logos are drawn once
                // per change, on the canvas between them.
                GlassAnimationTimeline(minimumInterval: resting.interval, paused: resting.paused) { date in
                    ZStack(alignment: .topLeading) {
                        Canvas { context, _ in
                            drawArcs(in: &context, fit: fit, phase: reduceMotion ? 0 : date.timeIntervalSinceReferenceDate * 18)
                        }
                        .accessibilityHidden(true)

                        FlowMapStillLayer(key: stillKey(fit: fit)) { context in
                            drawStill(in: &context, fit: fit)
                        }
                        .equatable()
                        .accessibilityHidden(true)

                        // The Traces map's library: the station, or on the
                        // standard map the orb (owner, 2026-10-16 and
                        // 2026-10-19).
                        if let library, case .library(let active) = library.kind {
                            libraryArt(lit: active, style: mapStyle, scale: fit.scale, date: date)
                                .position(fit.point(library.at))
                                .allowsHitTesting(false)
                        }
                    }
                    .frame(width: proxy.size.width, height: proxy.size.height, alignment: .topLeading)
                }
                // What moves, under the tool group, so its card and the
                // trace's details cover it.
                if scene.stack != nil {
                    GlassAnimationTimeline(minimumInterval: 1 / 60, paused: motion == nil) { date in
                        let t = motion.map { date.timeIntervalSince($0.start) } ?? 0
                        Canvas { context, _ in
                            if let run = motion {
                                MapTransferDrawing.draw(in: &context, scene: scene, run: run, t: t, fit: fit)
                            }
                            // Between contributions the rocket stands on the pad.
                            if mapStyle == .launch, motion?.action != .contribute {
                                MapTransferDrawing.restingRocket(scene: scene, fit: fit, in: &context)
                            }
                        }
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                    }
                }

                ForEach(scene.nodes) { node in
                    target(node, fit: fit)
                }

                ForEach(scene.reveals, id: \.tool) { reveal in
                    revealPill(reveal, fit: fit)
                }

                if let at = scene.addDevice, let onAddDevice {
                    addDeviceButton(at: at, fit: fit, action: onAddDevice)
                }

                // The tool group over the targets: its open card keeps the
                // pointer even where it covers the pad.
                if let stack = scene.stack {
                    GlassAnimationTimeline(minimumInterval: 1 / 60, paused: motion == nil) { date in
                        let t = motion.map { date.timeIntervalSince($0.start) } ?? 0
                        MapToolPile(
                            stack: stack, nodes: scene.nodes.filter { $0.id.hasPrefix("tool:") }, fit: fit,
                            run: motion, t: t, pointer: pointer)
                            .frame(width: proxy.size.width, height: proxy.size.height, alignment: .topLeading)
                    }
                }

                if let id = selection, let node = scene.nodes.first(where: { $0.id == id }) {
                    card(node, fit: fit, in: proxy.size)
                }
            }
            .frame(width: proxy.size.width, height: proxy.size.height)
            .contentShape(Rectangle())
            .onContinuousHover { phase in
                switch phase {
                case .active(let location): pointer = location
                case .ended: pointer = nil
                }
            }
            // The map's foot, fixed whatever its zoom or focus (owner,
            // 2026-10-12): the trace's card above, then the legend and the
            // zoom on one row. The legend wraps onto a second line before it
            // would run under the zoom (owner, 2026-10-19: they overlapped
            // in a narrow map), and the card always sits above both.
            .overlay(alignment: .bottom) {
                VStack(spacing: GlassTokens.Space.s4) {
                    // The traces being contributed, kept or dismissed,
                    // stacked as they follow one another (owner,
                    // 2026-10-19), only while another tab than Traces is
                    // shown: Traces shows them over its own list.
                    if scene.stack != nil, !transferCards.isEmpty {
                        GlassPopoverStack(
                            TraceTransferCard.items(
                                transferCards,
                                folderLabel: { transfer in
                                    scene.nodes.first { $0.id == "folder:" + transfer.folder }?.label
                                },
                                onClose: onCloseTransfer),
                            anchor: .bottom)
                            .frame(maxWidth: Self.transferCardWidth)
                            .transition(.opacity)
                    }
                    if !legend.isEmpty || zoomable {
                        HStack(alignment: .bottom, spacing: GlassTokens.Space.s6) {
                            if !legend.isEmpty {
                                // Offered the row's width before the spacer,
                                // so it wraps only where it must.
                                legendView
                                    .layoutPriority(1)
                                    .accessibilityHidden(true)
                            }
                            Spacer(minLength: 0)
                            if zoomable { zoomControls.fixedSize() }
                        }
                    }
                }
                .padding(GlassTokens.Space.mapOverlayInset)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(accessibilityName)
        .onChange(of: scene) { _, scene in
            if let focused, !scene.nodes.contains(where: { $0.id == focused }) { self.focused = nil }
        }
    }

    /// The library on the Traces map, at the map's scale: the station on
    /// the rocket map, the orb on the standard one.
    /// It turns with the map's own timeline (`date`), not one of its own.
    @ViewBuilder
    private func libraryArt(lit: Bool, style: TracesMapStyle, scale: CGFloat, date: Date) -> some View {
        switch style {
        case .launch:
            GlassLibraryStation(lit: lit, date: date)
                .frame(width: Self.stationSize.width * scale, height: Self.stationSize.height * scale)
        case .standard:
            GlassLibraryOrb(lit: lit, date: date)
                .frame(width: MapTransferDrawing.orbFrame * scale, height: MapTransferDrawing.orbFrame * scale)
        }
    }

    /// The library node the Traces map draws as the station or the orb;
    /// none on the other maps, which draw it as a disc.
    private var libraryOnMap: FlowMapScene.Node? {
        guard scene.stack != nil else { return nil }
        return scene.nodes.first(where: { $0.id == "library" })
    }

    /// How the map's resting timeline runs: at 30 frames a second while
    /// arcs carry moving dashes (as before), at the library's own rate
    /// while only the library turns, and not at all when nothing moves or
    /// under Reduce Motion.
    static func restingMotion(reduceMotion: Bool, flows: Bool, healthy: Bool, library: Bool)
        -> (interval: Double, paused: Bool) {
        let dashes = !reduceMotion && flows && healthy
        let turns = !reduceMotion && library
        return (dashes ? 1 / 30 : GlassLibraryStation.frameInterval, !(dashes || turns))
    }

    /// What the still layer is drawn from: when none of it changes, the
    /// nodes, names and logos are not drawn again.
    private func stillKey(fit: FlowMapGeometry) -> FlowMapStillKey {
        FlowMapStillKey(scene: scene, fit: fit, selection: selection, mapStyle: mapStyle,
                        colorScheme: colorScheme, contrast: contrast)
    }

    /// The station's frame on the rocket map, in design units.
    static let stationSize = CGSize(width: 150, height: 130)

    /// The transfer card's widest.
    static let transferCardWidth: CGFloat = 460

    // MARK: Drawing

    /// The arcs, under everything else; the only part of the map drawn
    /// again on each tick of its timeline.
    private func drawArcs(in context: inout GraphicsContext, fit: FlowMapGeometry, phase: Double) {
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
    }

    /// The nodes and the overflow counts, over the arcs: drawn only when
    /// `stillKey` changes.
    private func drawStill(in context: inout GraphicsContext, fit: FlowMapGeometry) {
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
            switch mapStyle {
            case .launch: Self.drawPad(at: centre, scale: fit.scale, in: &layer)
            case .standard: Self.drawLaptop(at: centre, scale: fit.scale, in: &layer)
            }
        case .hub:
            layer.fill(disc, with: .color(GlassTokens.Color.statusOff.color))
            ring(disc, radius: radius, centre: centre, in: &layer, colour: GlassColor.ink(0.25), scale: fit.scale)
        case .library where traces:
            // No name under the station or the orb, even under the pointer
            // (owner, 2026-10-19): its hover card says it.
            return
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
        case .device:
            // Another device on the account: this computer's laptop, small,
            // on its own deck (its screen rising above the point, as this
            // computer's does). Not the launch pad: nothing launches from
            // here for it.
            Self.drawLaptop(at: CGPoint(x: centre.x, y: centre.y + 4 * fit.scale),
                            scale: fit.scale * Self.deviceScale, in: &layer)
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
    /// How far above this computer's point the laptop's screen rises: a
    /// contributed trace sets off upward from there on the standard map.
    nonisolated static let laptopScreenTop: CGFloat = 36

    /// Another device's laptop, against this computer's.
    nonisolated static let deviceScale: CGFloat = 0.45

    /// This computer as a laptop on the standard map (owner, 2026-10-19):
    /// its deck as wide as the pad, so the line out lands on its left edge,
    /// and its screen open above it.
    static func drawLaptop(at centre: CGPoint, scale s: CGFloat, in context: inout GraphicsContext) {
        let half: CGFloat = FlowMapScene.padHalfWidth * s
        let steel: Color = GlassTokens.Color.statusOff.color
        let deck = Path(roundedRect: CGRect(x: centre.x - half, y: centre.y - 3 * s, width: half * 2, height: 6 * s),
                        cornerRadius: 3 * s)
        context.fill(deck, with: .color(steel))
        let notch = Path(roundedRect: CGRect(x: centre.x - 7 * s, y: centre.y - 3 * s, width: 14 * s, height: 2 * s),
                         cornerRadius: 1 * s)
        context.fill(notch, with: .color(GlassTokens.Color.mapNodeOff.color))
        let top: CGFloat = centre.y - laptopScreenTop * s
        let lid = CGRect(x: centre.x - 25 * s, y: top, width: 50 * s, height: (laptopScreenTop - 4) * s)
        context.fill(Path(roundedRect: lid, cornerRadius: 3.5 * s), with: .color(steel.opacity(0.85)))
        let display: CGRect = lid.insetBy(dx: 2.5 * s, dy: 2.5 * s)
        let deep: Color = GlassTokens.Gradient.heroCampaign.stops.first?.color.color ?? GlassTokens.Color.blue.color
        let bright: Color = GlassTokens.Gradient.heroCampaign.stops.last?.color.color ?? GlassTokens.Color.blue.color
        context.fill(Path(roundedRect: display, cornerRadius: 2 * s), with: .linearGradient(
            Gradient(colors: [bright.opacity(0.9), deep]),
            startPoint: CGPoint(x: display.minX, y: display.minY), endPoint: CGPoint(x: display.maxX, y: display.maxY)))
    }

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
    nonisolated static func labelType(scale: CGFloat) -> (labelSize: CGFloat, sublabelSize: CGFloat,
                                              labelOffset: CGFloat, sublabelOffset: CGFloat) {
        let unit = max(scale, labelFloor)
        return (11 * unit, 10 * unit, 12 * unit, 26.5 * unit)
    }

    /// The smallest factor the map's labels are drawn at.
    nonisolated static let labelFloor: CGFloat = 0.85

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
    /// one with keyboard focus. A click keeps no card open (owner,
    /// 2026-10-26).
    private var selection: String? {
        hovered ?? focused
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
    /// a dashed ring. At the map's foot, each item whole, wrapping onto
    /// another line where the row would not fit beside the zoom.
    private var legendView: some View {
        HistoryFilterFlow(spacing: GlassTokens.Space.s5) {
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
    }

    private func legendItem(_ word: String, @ViewBuilder dot: () -> some View) -> some View {
        HStack(spacing: GlassTokens.Space.s2) {
            dot().frame(width: 10, height: 10)
            Text(word)
                .glassType(GlassTokens.TypeScale.micro.weight(.regular))
                .foregroundStyle(GlassColor.textTertiary)
                .lineLimit(1)
        }
        .fixedSize()
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

    /// The + that links another device: a glass circle in the column, as
    /// the folder pills are glass, scaled with the map.
    private func addDeviceButton(at point: CGPoint, fit: FlowMapGeometry, action: @escaping () -> Void) -> some View {
        let unit = max(fit.scale, Self.labelFloor)
        let side = 26 * unit
        return Button(action: action) {
            Image(systemName: "plus")
                .font(.system(size: 12 * unit, weight: .semibold))
                .foregroundStyle(GlassColor.textPrimary)
                .frame(width: side, height: side)
                .glassSurface(.control, hover: GlassTokens.Color.controlHover)
                .clipShape(Circle())
                .contentShape(Circle())
        }
        .buttonStyle(GlassPressStyle())
        .help(addDeviceLabel)
        .accessibilityLabel(addDeviceLabel)
        .position(fit.point(point))
    }

    private func card(_ node: FlowMapScene.Node, fit: FlowMapGeometry, in size: CGSize) -> some View {
        let centre = fit.point(node.at)
        let width = GlassTokens.Size.nodeCardWidth
        let drawn = Self.drawnHalfWidth(node, traces: scene.stack != nil, style: mapStyle) * fit.scale
        let x = Self.cardX(centre: centre.x, halfWidth: drawn, card: width, field: size.width)
        let y = min(max(48, centre.y - 24), size.height - 48)
        return GlassFloatingGroup {
            GlassNodeCard(node.cardTitle, detail: node.detail)
        }
        .position(x: x, y: y)
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }

    /// Half the width the map draws a node at, in design units: on the
    /// Traces map the station or the orb for the library and the pad or
    /// the laptop for this computer, far wider than their discs; elsewhere
    /// the disc.
    static func drawnHalfWidth(_ node: FlowMapScene.Node, traces: Bool, style: TracesMapStyle) -> CGFloat {
        guard traces else { return node.radius }
        switch node.kind {
        case .library: return style == .launch ? stationSize.width / 2 : MapTransferDrawing.orbFrame / 2
        case .hub: return FlowMapScene.padHalfWidth
        default: return node.radius
        }
    }

    /// The gap between a node's drawing and its card, and the card's least
    /// distance from the field's edge.
    static let cardGap: CGFloat = 12
    static let cardMargin: CGFloat = 8

    /// Where a node's card is centred across the field: beside the node's
    /// drawing, never over it (owner, 2026-10-26: the cards covered the
    /// station and the pad). On the right where the card fits whole, else
    /// on the left; where neither side holds it, the roomier side, kept
    /// inside the field. `centre` and `halfWidth` are in the field's points.
    static func cardX(centre: CGFloat, halfWidth: CGFloat, card: CGFloat, field: CGFloat) -> CGFloat {
        let right = centre + halfWidth + cardGap + card / 2
        if right + card / 2 + cardMargin <= field { return right }
        let left = centre - halfWidth - cardGap - card / 2
        if left - card / 2 >= cardMargin { return left }
        let x = field - centre > centre ? right : left
        return min(max(card / 2 + cardMargin, x), field - card / 2 - cardMargin)
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

/// Everything the map's still layer (nodes, names, logos, overflow counts)
/// is drawn from. Equal keys draw the same picture.
struct FlowMapStillKey: Equatable {
    let scene: FlowMapScene
    let fit: FlowMapGeometry
    let selection: String?
    let mapStyle: TracesMapStyle
    let colorScheme: ColorScheme
    let contrast: ColorSchemeContrast
}

/// A canvas drawn again only when its key changes. The map's timeline
/// re-evaluates the views inside it on every tick; compared by key, this
/// one keeps its last drawing instead of drawing every node again.
struct FlowMapStillLayer<Key: Equatable & Sendable>: View, Equatable {
    let key: Key
    let draw: (inout GraphicsContext) -> Void

    init(key: Key, draw: @escaping (inout GraphicsContext) -> Void) {
        self.key = key
        self.draw = draw
    }

    nonisolated static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.key == rhs.key
    }

    var body: some View {
        Canvas { context, _ in draw(&context) }
    }
}

/// The design space (580×760) fitted into the field, centred, and zoomed
/// about the centre; or, with a focus, scaled 1.6× about that point as
/// #1146's camera is (`cameraFor`).
struct FlowMapGeometry: Equatable {
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
