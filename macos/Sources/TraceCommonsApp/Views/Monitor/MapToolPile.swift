import SwiftUI
import TCDesign
import TCShellCore

/// The Traces map's tool group (owner, 2026-10-16): the tools as a pile of
/// coins on a card, the front coin first. At rest the coins sit close; under
/// the pointer the pile opens to its full column first, then the card
/// widens to show each tool's name and what waits from it, and leaving
/// reverses that. While a trace is moving through the map the card keeps
/// its size, so no line moves under it, and the pointer there shows that
/// trace's details instead. Coins shuffle and grow in either state.
struct MapToolPile: View {
    let stack: FlowMapScene.ToolStack
    let nodes: [FlowMapScene.Node]
    let fit: FlowMapGeometry
    let run: MapTransferRun?
    /// Seconds into `run`.
    let t: Double
    /// A folder's label, by its id, for the details card.
    let folderLabel: (String) -> String?
    /// Whether the transfer is a contribution, for its colours.
    var contributing: Bool { run?.action != .keep }

    @State private var over = false
    @State private var tall = false
    @State private var wide = false
    @State private var names = false
    @State private var staging: Task<Void, Never>?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private typealias Stack = FlowMapScene.ToolStack
    private static var words: MonitorFlowMapCopy? { FlowMapScene.words }

    var body: some View {
        let s = fit.scale
        let moving = run.map { $0.moving(at: t) } ?? false
        let open = tall && !moving
        let order = run?.order(stack.tools, at: t) ?? stack.tools
        let orbits = run?.orbits(at: t) ?? [:]
        let height = stack.height(open: open) * s
        let width = (wide && !moving ? Stack.openWidth : Stack.width) * s
        let left = fit.point(stack.portLeft)
        ZStack(alignment: .topLeading) {
            RoundedRectangle(cornerRadius: 26 * s, style: .continuous)
                .fill(open && wide ? GlassTokens.Color.paneOpaque.color.opacity(0.92) : GlassTokens.Color.wellFill.color)
                .frame(width: width, height: height)
                .position(x: left.x + width / 2, y: left.y)
                .accessibilityHidden(true)
            if let heading = Self.words?.toolsHeading {
                Text(heading.uppercased())
                    .font(.system(size: max(9, 10 * s), weight: .bold))
                    .tracking(0.8)
                    .foregroundStyle(GlassColor.textTertiary)
                    .fixedSize()
                    .position(x: left.x + 30 * s, y: left.y - height / 2 - 12 * s)
                    .accessibilityHidden(true)
            }
            ForEach(Array(order.enumerated()), id: \.element) { index, id in
                if let node = nodes.first(where: { $0.id == id }) {
                    coin(node, index: index, count: order.count, open: open, grown: orbits[id] != nil)
                    tag(node, index: index, open: open)
                }
            }
            // Where the pointer counts as over the group: the card, or the
            // details card too while it is open.
            Color.clear
                .contentShape(Rectangle())
                .frame(width: moving ? 252 : width, height: moving ? max(height, 214) : height)
                .position(x: moving ? left.x - 9 + 126 : left.x + width / 2, y: left.y)
                .onHover(perform: hover)
                .accessibilityHidden(true)
            if moving, over, let run, let transfer = run.current(at: t) {
                details(transfer, s: s)
                    .position(x: left.x - 9 + 126, y: left.y)
                    .transition(.scale(scale: 0.2, anchor: .leading).combined(with: .opacity))
            }
        }
    }

    // MARK: Coins

    private func coin(_ node: FlowMapScene.Node, index: Int, count: Int, open: Bool, grown: Bool) -> some View {
        let s = fit.scale
        let tool: GlassTool
        let off: Bool
        if case .tool(let kind, let isOff) = node.kind {
            tool = kind
            off = isOff
        } else {
            tool = .other(initials: "?")
            off = false
        }
        let tint = tool.tint.color
        let side = Stack.coin * s
        return ZStack {
            // The coin's edge below it, then an opaque face and its rim.
            Circle().fill(tint.opacity(0.45)).offset(y: 3 * s)
            Circle().fill(GlassTokens.Color.paneOpaque.color)
            Circle().fill(GlassColor.ink(0.12))
            Circle().strokeBorder(tint, style: StrokeStyle(lineWidth: 2 * s, dash: node.dashed ? [3 * s, 3 * s] : []))
            if let logo = tool.logo {
                GlassToolLogoShape(logo).fill(tint).frame(width: 20 * s, height: 20 * s)
            } else {
                Text(tool.initials)
                    .font(.system(size: 12 * s, weight: .bold, design: .monospaced))
                    .foregroundStyle(tint)
            }
            if off {
                Path { path in
                    path.move(to: CGPoint(x: side * 0.25, y: side * 0.25))
                    path.addLine(to: CGPoint(x: side * 0.75, y: side * 0.75))
                }
                .stroke(GlassColor.textPrimary, lineWidth: 2 * s)
            }
            if !node.dashed, open || index == 0 {
                // The lock: the tool's memory stays on this Mac. In the
                // closed pile only the front coin carries it, so the pips
                // do not stack.
                Image(systemName: "lock.fill")
                    .font(.system(size: 6.5 * s, weight: .semibold))
                    .foregroundStyle(GlassColor.textSecondary)
                    .frame(width: 15 * s, height: 15 * s)
                    .background(Circle().fill(GlassTokens.Color.paneOpaque.color))
                    .offset(x: side / 2 + 1 * s, y: 0)
            }
            if node.ringed {
                Circle().stroke(GlassColor.ink(0.35), lineWidth: 3 * s).padding(-4 * s)
            }
        }
        .frame(width: side, height: side)
        .opacity(node.dashed ? 0.55 : node.dim)
        .scaleEffect(grown ? 1.3 : 1)
        .shadow(color: grown ? (contributing ? GlassTokens.Color.statusOn.color : GlassTokens.Color.purpleSoft.color).opacity(0.45) : .clear,
                radius: 9 * s)
        .zIndex(grown ? 100 : Double(count - index))
        .position(fit.point(stack.coinCenter(index, open: open)))
        .animation(reduceMotion ? nil : .spring(response: 0.42, dampingFraction: 0.8), value: index)
        .animation(reduceMotion ? nil : .spring(response: 0.36, dampingFraction: 0.82), value: open)
        .animation(reduceMotion ? nil : .spring(response: 0.3, dampingFraction: 0.7), value: grown)
        .accessibilityElement()
        .accessibilityLabel(node.label)
        .accessibilityValue(node.detail)
    }

    /// A coin's name and what waits from it, beside it once the card is
    /// wide.
    private func tag(_ node: FlowMapScene.Node, index: Int, open: Bool) -> some View {
        let s = fit.scale
        let centre = fit.point(stack.coinCenter(index, open: open))
        let line = node.dashed ? Self.words?.legendNotWatched : node.waiting.flatMap { Self.words?.coinWaiting($0) }
        return VStack(alignment: .leading, spacing: 0) {
            Text(node.label)
                .font(.system(size: max(9.5, 11 * s), weight: .semibold))
                .foregroundStyle(GlassColor.textPrimary)
            if let line {
                Text(line)
                    .font(.system(size: max(9, 10 * s)))
                    .foregroundStyle(GlassColor.textTertiary)
            }
        }
        .lineLimit(1)
        .fixedSize()
        .opacity(names ? (node.dashed ? 0.55 : 1) : 0)
        .frame(width: 0, height: 0, alignment: .leading)
        .position(x: centre.x + 32 * s, y: centre.y)
        .allowsHitTesting(false)
        .accessibilityHidden(true)
        .animation(reduceMotion ? nil : .spring(response: 0.36, dampingFraction: 0.82), value: open)
    }

    // MARK: Hover

    private func hover(_ inside: Bool) {
        over = inside
        staging?.cancel()
        let motion = !reduceMotion
        if inside {
            withAnimation(motion ? .spring(response: 0.34, dampingFraction: 0.84) : nil) { tall = true }
            staging = Task { @MainActor in
                try? await Task.sleep(for: .milliseconds(motion ? 260 : 0))
                guard !Task.isCancelled else { return }
                withAnimation(motion ? .spring(response: 0.3, dampingFraction: 0.86) : nil) { wide = true }
                try? await Task.sleep(for: .milliseconds(motion ? 200 : 0))
                guard !Task.isCancelled else { return }
                withAnimation(motion ? .easeOut(duration: 0.18) : nil) { names = true }
            }
        } else {
            withAnimation(motion ? .easeOut(duration: 0.15) : nil) { names = false }
            staging = Task { @MainActor in
                try? await Task.sleep(for: .milliseconds(motion ? 100 : 0))
                guard !Task.isCancelled else { return }
                withAnimation(motion ? .spring(response: 0.28, dampingFraction: 0.9) : nil) { wide = false }
                try? await Task.sleep(for: .milliseconds(motion ? 220 : 0))
                guard !Task.isCancelled else { return }
                withAnimation(motion ? .spring(response: 0.34, dampingFraction: 0.86) : nil) { tall = false }
            }
        }
    }

    // MARK: Details

    /// The trace moving through the group, under the pointer (owner,
    /// 2026-10-16): the tool, whether it is contributed or kept, the trace,
    /// and the tool's memory, which does not move.
    @ViewBuilder
    private func details(_ transfer: MapTransfer, s: CGFloat) -> some View {
        if let words = Self.words {
            let tool = SourceKind(rawValue: transfer.tool)
            let name = tool?.displayName ?? transfer.tool
            let glass = tool.map(TracesTreeView.glassTool) ?? .other(initials: FlowMapScene.initials(transfer.tool))
            let contributing = transfer.action == .contribute
            VStack(alignment: .leading, spacing: 10) {
                HStack(spacing: 10) {
                    ZStack {
                        Circle().fill(GlassTokens.Color.paneOpaque.color)
                        Circle().strokeBorder(glass.tint.color, lineWidth: 2)
                        if let logo = glass.logo {
                            GlassToolLogoShape(logo).fill(glass.tint.color).frame(width: 14, height: 14)
                        } else {
                            Text(glass.initials).font(.system(size: 10, weight: .bold, design: .monospaced))
                                .foregroundStyle(glass.tint.color)
                        }
                    }
                    .frame(width: 30, height: 30)
                    Text(name).glassType(GlassTokens.TypeScale.bodyStrong).foregroundStyle(GlassColor.textPrimary)
                    Spacer(minLength: 0)
                    GlassTag(contributing ? words.transferContributing : words.transferKeeping,
                             tone: contributing ? .on : .neutral)
                }
                VStack(alignment: .leading, spacing: 2) {
                    Text(words.transferTrace.uppercased())
                        .glassType(GlassTokens.TypeScale.eyebrow)
                        .foregroundStyle(GlassColor.textTertiary)
                    Text(traceLine(transfer))
                        .glassType(GlassTokens.TypeScale.label.weight(.regular))
                        .foregroundStyle(GlassColor.textPrimary)
                }
                HStack(alignment: .top, spacing: 8) {
                    Image(systemName: "lock")
                        .font(.system(size: 11))
                        .foregroundStyle(GlassColor.textPrimary)
                    VStack(alignment: .leading, spacing: 2) {
                        if let files = words.memoryFiles(transfer.tool) {
                            Text(files).glassType(GlassTokens.TypeScale.mono).foregroundStyle(GlassColor.textPrimary)
                        }
                        Text(words.memoryStays)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                    }
                }
                .padding(.vertical, 8)
                .padding(.horizontal, 10)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: 11, style: .continuous).fill(GlassTokens.Color.wellFill.color))
                Text(words.transferNote(contributing: contributing, tool: name))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(14)
            .frame(width: 252, alignment: .leading)
            .glassSurface(.nodeCard, floating: true)
            .allowsHitTesting(false)
            .accessibilityElement(children: .combine)
        }
    }

    /// The trace's folder and, when recorded, its time (`MapTransfer.at`).
    private func traceLine(_ transfer: MapTransfer) -> String {
        let folder = folderLabel(transfer.folder) ?? transfer.folder
        guard let at = transfer.at else { return folder }
        return folder + " \u{00B7} " + at.formatted(date: .abbreviated, time: .shortened)
    }
}

/// What a run of transfers looks like at a moment (owner, 2026-10-16),
/// drawn over the map: the traces as glowing orbs on the lines, the light
/// round a coin's rim, the pulse as each vanishes at the pad, the rocket
/// on the pad, its lift-off and flight to the library's station, and the
/// lines under the pad.
enum MapTransferDrawing {
    private typealias Stack = FlowMapScene.ToolStack

    static func draw(
        in context: inout GraphicsContext, scene: FlowMapScene, run: MapTransferRun, t: Double, fit: FlowMapGeometry
    ) {
        guard let stack = scene.stack else { return }
        let s = fit.scale
        let contributing = run.action == .contribute
        let glow = contributing ? GlassTokens.Color.statusOn.color : GlassTokens.Color.purpleSoft.color
        let hub = scene.nodes.first { $0.id == "hub" }?.at ?? FlowMapScene.hubPoint
        let library = scene.nodes.first { $0.id == "library" }?.at ?? FlowMapScene.libraryPoint
        let outArc = scene.arcs.last { $0.from == stack.portRight }
        let order = run.order(stack.tools, at: t)
        let rocketRest = CGPoint(x: hub.x, y: hub.y - 5 - rocketLength / 2)
        let pulseAt = contributing ? rocketRest : hub

        for (index, transfer) in run.transfers.enumerated() {
            switch run.leg(index, at: t) {
            case .waiting:
                break
            case .inbound(let p):
                if let arc = folderArc(transfer.folder, in: scene) {
                    orb(at: fit.point(point(on: arc, at: ease(p))), glow: glow, s: s, in: &context)
                }
            case .orbit(let p):
                guard let index = order.firstIndex(of: "tool:" + transfer.tool) else { break }
                let centre = fit.point(stack.coinCenter(index, open: false))
                // From the coin's base, counter-clockwise: up its right side,
                // over the top and down the left (owner, 2026-10-16).
                let r = Stack.coin / 2 * 1.3 * s
                let angle = 2 * Double.pi * p
                let light = CGPoint(x: centre.x + r * CGFloat(sin(angle)), y: centre.y + r * CGFloat(cos(angle)))
                orb(at: light, glow: glow, s: s * 0.75, in: &context)
            case .outbound(let p):
                if let arc = outArc {
                    orb(at: fit.point(point(on: arc, at: ease(p))), glow: glow, s: s, in: &context)
                }
            case .arrived(let since):
                if since < MapTransferRun.pulse {
                    pulse(at: fit.point(pulseAt), progress: since / MapTransferRun.pulse, glow: glow, s: s, in: &context)
                }
            }
        }

        if contributing {
            rocket(run: run, t: t, from: rocketRest, to: library, s: s, fit: fit, in: &context)
        }
        lineUnderPad(run: run, t: t, hub: hub, fit: fit, in: &context)
    }

    // MARK: Paths

    static func folderArc(_ folder: String, in scene: FlowMapScene) -> FlowMapScene.Arc? {
        guard let node = scene.nodes.first(where: { $0.id == "folder:\(folder)" }) else { return nil }
        return scene.arcs.first { abs($0.from.y - node.at.y) < 0.5 && abs($0.from.x - node.at.x - FlowMapScene.folderRadius - 1) < 0.5 }
    }

    static func point(on arc: FlowMapScene.Arc, at t: Double) -> CGPoint {
        let u = 1 - t
        let a = u * u * u, b = 3 * u * u * t, c = 3 * u * t * t, d = t * t * t
        return CGPoint(x: a * arc.from.x + b * arc.control1.x + c * arc.control2.x + d * arc.to.x,
                       y: a * arc.from.y + b * arc.control1.y + c * arc.control2.y + d * arc.to.y)
    }

    static func ease(_ t: Double) -> Double {
        t < 0.5 ? 4 * t * t * t : 1 - pow(-2 * t + 2, 3) / 2
    }

    // MARK: Orbs and pulses

    static func orb(at centre: CGPoint, glow: Color, s: CGFloat, in context: inout GraphicsContext) {
        let r = 7 * s
        context.drawLayer { layer in
            layer.addFilter(.blur(radius: 6 * s))
            layer.fill(Path(ellipseIn: CGRect(x: centre.x - r * 2, y: centre.y - r * 2, width: r * 4, height: r * 4)),
                       with: .color(glow.opacity(0.45)))
        }
        context.fill(Path(ellipseIn: CGRect(x: centre.x - r, y: centre.y - r, width: r * 2, height: r * 2)),
                     with: .radialGradient(Gradient(colors: [.white, glow.opacity(0.9), glow.opacity(0)]),
                                           center: CGPoint(x: centre.x - r * 0.2, y: centre.y - r * 0.25),
                                           startRadius: 0, endRadius: r * 1.2))
    }

    static func pulse(at centre: CGPoint, progress: Double, glow: Color, s: CGFloat, in context: inout GraphicsContext) {
        let r = (6 + 38 * progress) * s
        let alpha = progress < 0.2 ? progress / 0.2 * 0.95 : 0.95 * (1 - (progress - 0.2) / 0.8)
        context.fill(Path(ellipseIn: CGRect(x: centre.x - r, y: centre.y - r, width: r * 2, height: r * 2)),
                     with: .radialGradient(Gradient(colors: [.white.opacity(alpha), glow.opacity(alpha * 0.75), glow.opacity(0)]),
                                           center: centre, startRadius: 0, endRadius: r))
    }

    // MARK: Rocket

    /// The rocket's length along its flight, in design units: the pad's
    /// own size, so it stands on it in proportion (owner, 2026-10-16).
    static let rocketLength: CGFloat = 44

    static func rocket(
        run: MapTransferRun, t: Double, from rest: CGPoint, to library: CGPoint, s: CGFloat,
        fit: FlowMapGeometry, in context: inout GraphicsContext
    ) {
        let flight = run.flight(at: t)
        if let flight, flight >= 1 {
            dock(at: fit.point(library), since: t - run.liftOff - MapTransferRun.flight, s: s, run: run, in: &context)
            return
        }
        // The flight path: up off the pad, then over to the station.
        let path = FlowMapScene.Arc(
            from: rest, control1: CGPoint(x: rest.x, y: rest.y - 118),
            control2: CGPoint(x: library.x, y: library.y + 110), to: library, style: .quiet)
        let progress = flight ?? 0
        let along = MapTransferRun.climb(progress)
        let at = point(on: path, at: along)
        let ahead = point(on: path, at: min(1, along + 0.01))
        var angle = atan2(Double(ahead.y - at.y), Double(ahead.x - at.x))
        if along >= 0.999 || (ahead == at) { angle = -Double.pi / 2 }
        var centre = fit.point(at)
        // Ignition: a shake on the pad before it moves.
        if flight != nil, progress < 0.26 {
            centre.x += CGFloat(sin(t * 90)) * 0.7 * s
        }
        let charge = min(run.arrived(at: t), 4)
        let glow: Color = flight != nil ? flameOuter : (charge > 0 ? GlassTokens.Color.statusOn.color : flameOuter.opacity(0.5))
        let glowRadius = flight != nil ? 8 * s : (5 + CGFloat(charge) * 4) * s

        if flight != nil, progress < 0.6 {
            smoke(at: fit.point(CGPoint(x: rest.x, y: rest.y + rocketLength / 2)), progress: progress / 0.6, s: s, in: &context)
        }
        drawRocket(at: centre, angle: angle, s: s, glow: glow, glowRadius: glowRadius,
                   burn: flight.map { 0.35 + 1.55 * CGFloat($0) }, in: &context)
    }

    /// The rocket standing on the pad between runs, unlit, ready for the
    /// next contribution.
    static func restingRocket(scene: FlowMapScene, fit: FlowMapGeometry, in context: inout GraphicsContext) {
        guard scene.stack != nil else { return }
        let hub = scene.nodes.first { $0.id == "hub" }?.at ?? FlowMapScene.hubPoint
        let rest = CGPoint(x: hub.x, y: hub.y - 5 - rocketLength / 2)
        drawRocket(at: fit.point(rest), angle: -Double.pi / 2, s: fit.scale, glow: .clear, glowRadius: 0, burn: nil,
                   in: &context)
    }

    /// The flame's colours, as the map's design canvas drew them (owner,
    /// 2026-10-16): a warm orange and a near-white core.
    static let flameOuter = Color(red: 1, green: 0.7, blue: 0.36)
    static let flameInner = Color(red: 1, green: 0.95, blue: 0.79)

    /// The rocket at `centre`, nose along `angle`; `burn` is the flame's
    /// size, none while it stands. Its body is outlined so it holds on a
    /// light map as well as a dark one.
    static func drawRocket(
        at centre: CGPoint, angle: Double, s: CGFloat, glow: Color, glowRadius: CGFloat, burn: CGFloat?,
        in context: inout GraphicsContext
    ) {
        context.drawLayer { layer in
            if glowRadius > 0 { layer.addFilter(.shadow(color: glow, radius: glowRadius)) }
            layer.translateBy(x: centre.x, y: centre.y)
            layer.rotate(by: .radians(angle))
            let k = rocketLength * s / 30
            layer.scaleBy(x: k, y: k)
            layer.translateBy(x: -15, y: -9)
            if let burn {
                // The flame grows as it climbs.
                layer.drawLayer { flame in
                    flame.translateBy(x: 9, y: 9)
                    flame.scaleBy(x: burn, y: burn)
                    flame.translateBy(x: -9, y: -9)
                    flame.fill(flamePath, with: .color(flameOuter))
                    flame.fill(flameCore, with: .color(flameInner))
                }
            }
            layer.fill(finsPath, with: .color(GlassTokens.Color.purpleSoft.color))
            layer.fill(bodyPath, with: .color(Color(white: 0.95)))
            layer.stroke(bodyPath, with: .color(GlassTokens.Color.statusOff.color), lineWidth: 0.9)
            layer.fill(Path(ellipseIn: CGRect(x: 18, y: 7, width: 4, height: 4)), with: .color(GlassTokens.Color.blue.color))
        }
    }

    static func smoke(at base: CGPoint, progress: Double, s: CGFloat, in context: inout GraphicsContext) {
        for (offset, delay) in [(CGFloat(0), 0.0), (CGFloat(-14), 0.15)] {
            let p = max(0, progress - delay) / (1 - delay)
            guard p > 0 else { continue }
            let r = (6 + 30 * p) * s
            let alpha = 0.5 * (1 - p)
            let centre = CGPoint(x: base.x + offset * s, y: base.y - 4 * s)
            context.fill(Path(ellipseIn: CGRect(x: centre.x - r, y: centre.y - r, width: r * 2, height: r * 2)),
                         with: .radialGradient(Gradient(colors: [Color(white: 0.86).opacity(alpha), Color(white: 0.66).opacity(0)]),
                                               center: centre, startRadius: 0, endRadius: r))
        }
    }

    /// The station lights as the rocket docks, and says how many arrived.
    static func dock(at centre: CGPoint, since: Double, s: CGFloat, run: MapTransferRun, in context: inout GraphicsContext) {
        guard since >= 0 else { return }
        if since < 1.1 {
            let p = since / 1.1
            let r = (28 + 26 * p) * s
            context.stroke(Path(ellipseIn: CGRect(x: centre.x - r, y: centre.y - r, width: r * 2, height: r * 2)),
                           with: .color(GlassTokens.Color.blue.color.opacity(0.75 * (1 - p))), lineWidth: 3 * s)
        }
        if since < MapTransferRun.linger {
            let text = Text("+\(run.transfers.count)")
                .font(.system(size: max(9, 10 * s), weight: .bold))
                .foregroundStyle(GlassTokens.Color.statusOnText.color)
            context.draw(text, at: CGPoint(x: centre.x + 40 * s, y: centre.y - 30 * s))
        }
    }

    // MARK: Under the pad

    static func lineUnderPad(run: MapTransferRun, t: Double, hub: CGPoint, fit: FlowMapGeometry, in context: inout GraphicsContext) {
        guard let words = FlowMapScene.words else { return }
        let arrived = run.arrived(at: t)
        let line: String?
        switch run.action {
        case .contribute:
            line = run.transfers.count > 1 && arrived > 0 && t < run.liftOff
                ? words.collected(arrived, of: run.transfers.count) : nil
        case .keep:
            line = arrived == run.transfers.count ? words.keptHere(arrived) : nil
        }
        guard let line else { return }
        let s = fit.scale
        let type = FlowMapView.labelType(scale: s)
        let centre = fit.point(hub)
        context.draw(
            Text(line).font(.system(size: type.sublabelSize)).foregroundStyle(GlassColor.textTertiary),
            at: CGPoint(x: centre.x, y: centre.y + 16 * s + type.sublabelOffset))
    }

    // MARK: The rocket's shape, in its own 30 by 18 units, nose to the right

    static let bodyPath: Path = {
        var p = Path()
        p.move(to: CGPoint(x: 8.5, y: 9))
        p.addCurve(to: CGPoint(x: 29, y: 9), control1: CGPoint(x: 11, y: 4), control2: CGPoint(x: 19, y: 3.5))
        p.addCurve(to: CGPoint(x: 8.5, y: 9), control1: CGPoint(x: 19, y: 14.5), control2: CGPoint(x: 11, y: 14))
        p.closeSubpath()
        return p
    }()

    static let finsPath: Path = {
        var p = Path()
        p.move(to: CGPoint(x: 11, y: 5.2))
        p.addLine(to: CGPoint(x: 9, y: 1.5))
        p.addLine(to: CGPoint(x: 15, y: 4.4))
        p.closeSubpath()
        p.move(to: CGPoint(x: 11, y: 12.8))
        p.addLine(to: CGPoint(x: 9, y: 16.5))
        p.addLine(to: CGPoint(x: 15, y: 13.6))
        p.closeSubpath()
        return p
    }()

    static let flamePath: Path = {
        var p = Path()
        p.move(to: CGPoint(x: 0, y: 9))
        p.addCurve(to: CGPoint(x: 9, y: 9), control1: CGPoint(x: 4, y: 5.5), control2: CGPoint(x: 7, y: 6.5))
        p.addCurve(to: CGPoint(x: 0, y: 9), control1: CGPoint(x: 7, y: 11.5), control2: CGPoint(x: 4, y: 12.5))
        p.closeSubpath()
        return p
    }()

    static let flameCore: Path = {
        var p = Path()
        p.move(to: CGPoint(x: 3, y: 9))
        p.addCurve(to: CGPoint(x: 9, y: 9), control1: CGPoint(x: 5, y: 7.5), control2: CGPoint(x: 7, y: 8))
        p.addCurve(to: CGPoint(x: 3, y: 9), control1: CGPoint(x: 7, y: 10), control2: CGPoint(x: 5, y: 10.5))
        p.closeSubpath()
        return p
    }()
}
