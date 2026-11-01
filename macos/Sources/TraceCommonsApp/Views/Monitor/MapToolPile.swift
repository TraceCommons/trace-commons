import SwiftUI
import TCDesign
import TCShellCore

/// The Traces map's tool group (owner, 2026-10-16): the tools as a pile of
/// coins on a card, the front coin first. At rest the coins sit close; under
/// the pointer the pile opens to its full column first, then the card
/// widens to show each tool's name and what waits from it, and leaving
/// reverses that. While a trace is moving through the map the card keeps
/// its size, so no line moves under it. Coins shuffle and grow in either
/// state. The moving trace's details are a card of their own, never under
/// the pointer (`TraceTransferCard`, owner, 2026-10-19).
///
/// Only the card itself is under the pointer for this (owner, 2026-10-19):
/// the map hands the pile where the pointer is, and the pile tests it
/// against its own card.
struct MapToolPile: View {
    let stack: FlowMapScene.ToolStack
    let nodes: [FlowMapScene.Node]
    let fit: FlowMapGeometry
    let run: MapTransferRun?
    /// Seconds into `run`.
    let t: Double
    /// The pointer on the map, in the map's own coordinates; nil off it.
    var pointer: CGPoint? = nil

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
        // Typed here, out of the view's expression, for the older type
        // checker (macOS 15 CI).
        let card = CGRect(x: left.x, y: left.y - height / 2, width: width, height: height)
        let overCard: Bool = pointer.map { card.contains($0) } ?? false
        // An open card is opaque: nothing under it shows through.
        let fill: Color = open && wide ? GlassTokens.Color.paneOpaque.color : GlassTokens.Color.wellFill.color
        ZStack(alignment: .topLeading) {
            RoundedRectangle(cornerRadius: 26 * s, style: .continuous)
                .fill(fill)
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
            // The light round a coin's rim: over the coins, under the
            // details.
            if let run, !orbits.isEmpty {
                Canvas { context, _ in
                    MapTransferDrawing.rimLights(in: &context, stack: stack, run: run, t: t, fit: fit)
                }
                .allowsHitTesting(false)
                .accessibilityHidden(true)
                .zIndex(500)
            }
        }
        .onChange(of: overCard) { _, inside in hover(inside) }
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
        let runGlow = MapTransferDrawing.glow(run?.action)
        let glow: Color = grown ? runGlow.opacity(0.45) : .clear
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
        }
        .frame(width: side, height: side)
        .opacity(node.dashed ? 0.55 : node.dim)
        .scaleEffect(grown ? 1.3 : 1)
        .shadow(color: glow, radius: 9 * s)
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
        let shown: Double = node.dashed ? 0.55 : 1
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
        .opacity(names ? shown : 0)
        .frame(width: 0, height: 0, alignment: .leading)
        .position(x: centre.x + 32 * s, y: centre.y)
        .allowsHitTesting(false)
        .accessibilityHidden(true)
        .animation(reduceMotion ? nil : .spring(response: 0.36, dampingFraction: 0.82), value: open)
    }

    // MARK: Hover

    private func hover(_ inside: Bool) {
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
}

/// What a run of transfers looks like at a moment (owner, 2026-10-16),
/// drawn on the map under the tool group, so its card and the details
/// cover it: the traces as glowing orbs on the lines, the pulse as each
/// vanishes at the pad, the rocket on the pad, its lift-off and flight to
/// the library's station, and the lines under the pad. The light round a
/// coin's rim is drawn with the pile, over its coins (`rimLights`).
///
/// On the standard map (owner, 2026-10-19) each contributed trace goes
/// into the laptop and rises from its screen to the library's orb, which
/// takes it in with a pulse; there is no rocket.
enum MapTransferDrawing {
    private typealias Stack = FlowMapScene.ToolStack

    /// The library orb's frame on the standard map, in design units, and
    /// the sphere's radius inside it.
    static let orbFrame: CGFloat = 104
    static var orbRadius: CGFloat { orbFrame / 2 * (1 - GlassLibraryOrb.inset) }

    /// A run's colour (owner, 2026-10-20): blue for traces being shared,
    /// orange for traces kept on this Mac, on the orbs, the coin's light,
    /// the pile's glow and the pad's charge. A dismissed trace is not
    /// shared, so it takes the kept colour, as it took the non-shared one
    /// before.
    static func glowToken(_ action: MapTransfer.Action?) -> GlassRGBA {
        action == .contribute ? GlassTokens.Color.mapOrbShared : GlassTokens.Color.mapOrbKept
    }

    static func glow(_ action: MapTransfer.Action?) -> Color { glowToken(action).color }

    static func draw(
        in context: inout GraphicsContext, scene: FlowMapScene, run: MapTransferRun, t: Double, fit: FlowMapGeometry
    ) {
        guard let stack = scene.stack else { return }
        let s = fit.scale
        let contributing = run.action == .contribute
        let glow = glow(run.action)
        let hub = scene.nodes.first { $0.id == "hub" }?.at ?? FlowMapScene.hubPoint
        let library = scene.nodes.first { $0.id == "library" }?.at ?? FlowMapScene.libraryPoint
        let outArc = scene.arcs.last { $0.from == stack.portRight }
        let rocketRest = CGPoint(x: hub.x, y: hub.y - 5 - rocketLength / 2)
        let screen = CGPoint(x: hub.x, y: hub.y - FlowMapView.laptopScreenTop)
        let orbBase = CGPoint(x: library.x, y: library.y + orbRadius)
        let pulseAt: CGPoint = run.rises ? library : (contributing ? rocketRest : hub)

        for (index, transfer) in run.transfers.enumerated() {
            switch run.leg(index, at: t) {
            case .waiting, .orbit:
                break
            case .inbound(let p):
                if let arc = folderArc(transfer.folder, in: scene) {
                    orb(at: fit.point(point(on: arc, at: ease(p))), glow: glow, s: s, in: &context)
                }
            case .outbound(let p):
                if let arc = outArc {
                    orb(at: fit.point(point(on: arc, at: ease(p))), glow: glow, s: s, in: &context)
                }
            case .ascent(let p):
                if p < 0.3 {
                    pulse(at: fit.point(screen), progress: p / 0.3, glow: glow, s: s * 0.6, in: &context)
                }
                rise(from: screen, to: orbBase, progress: p, glow: glow, fit: fit, in: &context)
            case .arrived(let since):
                if since < MapTransferRun.pulse {
                    pulse(at: fit.point(pulseAt), progress: since / MapTransferRun.pulse, glow: glow, s: s, in: &context)
                }
            }
        }

        if contributing, run.style == .launch {
            rocket(run: run, t: t, from: rocketRest, to: library, s: s, fit: fit, in: &context)
        }
        if run.rises {
            dock(at: fit.point(library), since: t - run.allArrived, s: s, run: run, in: &context)
        }
        lineUnderPad(run: run, t: t, hub: hub, fit: fit, in: &context)
    }

    /// The light going once round the rim of each coin a trace is in,
    /// counter-clockwise from its base: up its right side, over the top
    /// and down the left (owner, 2026-10-16).
    static func rimLights(
        in context: inout GraphicsContext, stack: FlowMapScene.ToolStack, run: MapTransferRun, t: Double,
        fit: FlowMapGeometry
    ) {
        let s = fit.scale
        let glow = glow(run.action)
        let order = run.order(stack.tools, at: t)
        for (id, p) in run.orbits(at: t) {
            guard let index = order.firstIndex(of: id) else { continue }
            let centre = fit.point(stack.coinCenter(index, open: false))
            let r: CGFloat = Stack.coin / 2 * 1.3 * s
            let angle: Double = 2 * Double.pi * p
            let light = CGPoint(x: centre.x + r * CGFloat(sin(angle)), y: centre.y + r * CGFloat(cos(angle)))
            orb(at: light, glow: glow, s: s * 0.75, in: &context)
        }
    }

    /// A trace rising from the laptop's screen to the orb's base: slowly
    /// away, then faster, with a short fading trail.
    static func rise(
        from start: CGPoint, to end: CGPoint, progress: Double, glow: Color, fit: FlowMapGeometry,
        in context: inout GraphicsContext
    ) {
        let s = fit.scale
        for step in (0 ... 3).reversed() {
            let lag: Double = Double(step) * 0.045
            let along: CGFloat = CGFloat(pow(max(0, progress - lag), 1.7))
            let at = CGPoint(x: start.x + (end.x - start.x) * along, y: start.y + (end.y - start.y) * along)
            var layer = context
            layer.opacity = 1 - Double(step) * 0.28
            let size: CGFloat = s * (1 - CGFloat(step) * 0.18)
            orb(at: fit.point(at), glow: glow, s: size, in: &layer)
        }
    }

    // MARK: Paths

    static func folderArc(_ folder: String, in scene: FlowMapScene) -> FlowMapScene.Arc? {
        guard let node = scene.nodes.first(where: { $0.id == "folder:\(folder)" }) else { return nil }
        let start = CGPoint(x: node.at.x + FlowMapScene.folderRadius + 1, y: node.at.y)
        return scene.arcs.first { arc in
            let dx: CGFloat = abs(arc.from.x - start.x)
            let dy: CGFloat = abs(arc.from.y - start.y)
            return dx < 0.5 && dy < 0.5
        }
    }

    static func point(on arc: FlowMapScene.Arc, at t: Double) -> CGPoint {
        // Small typed steps in one type: Swift 6.2 gives up on the sum of
        // four mixed Double and CGFloat products written in one line.
        let v = CGFloat(t)
        let u: CGFloat = 1 - v
        let a: CGFloat = u * u * u
        let b: CGFloat = 3 * u * u * v
        let c: CGFloat = 3 * u * v * v
        let d: CGFloat = v * v * v
        let near: CGPoint = CGPoint(x: a * arc.from.x + b * arc.control1.x, y: a * arc.from.y + b * arc.control1.y)
        let far: CGPoint = CGPoint(x: c * arc.control2.x + d * arc.to.x, y: c * arc.control2.y + d * arc.to.y)
        return CGPoint(x: near.x + far.x, y: near.y + far.y)
    }

    static func ease(_ t: Double) -> Double {
        if t < 0.5 { return 4 * t * t * t }
        let rest: Double = -2 * t + 2
        return 1 - pow(rest, 3) / 2
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
        let r: CGFloat = (6 + 38 * CGFloat(progress)) * s
        let alpha: Double = progress < 0.2 ? progress / 0.2 * 0.95 : 0.95 * (1 - (progress - 0.2) / 0.8)
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
        let glow: Color = flight != nil ? flameOuter : (charge > 0 ? GlassTokens.Color.mapOrbShared.color : flameOuter.opacity(0.5))
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
            let outline = GlassTokens.Color.statusOff.color
            let accent = GlassTokens.Color.purpleSoft.color
            layer.fill(nozzlePath, with: .color(outline))
            layer.stroke(nozzlePath, with: .color(outline), lineWidth: 0.9)
            layer.fill(finsPath, with: .color(accent))
            layer.stroke(finsPath, with: .color(outline), lineWidth: 0.9)
            layer.fill(bodyPath, with: .color(Color(white: 0.95)))
            layer.fill(nosePath, with: .color(accent))
            layer.stroke(bodyPath, with: .color(outline), lineWidth: 0.9)
            let window = Path(ellipseIn: porthole)
            layer.fill(window, with: .color(GlassTokens.Color.blue.color))
            layer.stroke(window, with: .color(outline), lineWidth: 0.9)
        }
    }

    static func smoke(at base: CGPoint, progress: Double, s: CGFloat, in context: inout GraphicsContext) {
        for (offset, delay) in [(CGFloat(0), 0.0), (CGFloat(-14), 0.15)] {
            let p: Double = max(0, progress - delay) / (1 - delay)
            guard p > 0 else { continue }
            let r: CGFloat = (6 + 30 * CGFloat(p)) * s
            let alpha: Double = 0.5 * (1 - p)
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
            let p: Double = since / 1.1
            let r: CGFloat = (28 + 26 * CGFloat(p)) * s
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
            line = run.style == .launch && run.transfers.count > 1 && arrived > 0 && t < run.liftOff
                ? words.collected(arrived, of: run.transfers.count) : nil
        case .keep:
            line = arrived == run.transfers.count ? words.keptHere(arrived) : nil
        case .dismiss:
            // A dismissal never plays on the map; only its card shows.
            line = nil
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

    // Drawn like the rocket emoji, side-on (Kristi, 2026-10-21: the old
    // lens read as a drop of water): a straight body from the tail at x 10,
    // an ogive nose to the point at x 29.5, a porthole, swept fins top and
    // bottom, and a short nozzle the flame (x 0 to 9) leaves from.

    /// The body and its nose, one outline.
    static let bodyPath: Path = {
        var p = Path()
        p.move(to: CGPoint(x: 10, y: 5.6))
        p.addLine(to: CGPoint(x: 20, y: 5.6))
        p.addCurve(to: CGPoint(x: 29.5, y: 9), control1: CGPoint(x: 24.5, y: 5.6), control2: CGPoint(x: 28, y: 7))
        p.addCurve(to: CGPoint(x: 20, y: 12.4), control1: CGPoint(x: 28, y: 11), control2: CGPoint(x: 24.5, y: 12.4))
        p.addLine(to: CGPoint(x: 10, y: 12.4))
        p.closeSubpath()
        return p
    }()

    /// The nose cone's colour, inside the body's outline.
    static let nosePath: Path = {
        var p = Path()
        p.move(to: CGPoint(x: 23.5, y: 6.1))
        p.addCurve(to: CGPoint(x: 29.5, y: 9), control1: CGPoint(x: 26, y: 6.6), control2: CGPoint(x: 28.3, y: 7.6))
        p.addCurve(to: CGPoint(x: 23.5, y: 11.9), control1: CGPoint(x: 28.3, y: 10.4), control2: CGPoint(x: 26, y: 11.4))
        p.addQuadCurve(to: CGPoint(x: 23.5, y: 6.1), control: CGPoint(x: 24.4, y: 9))
        p.closeSubpath()
        return p
    }()

    /// Two fins swept back from the body to tips by the tail.
    static let finsPath: Path = {
        var p = Path()
        p.move(to: CGPoint(x: 17, y: 5.6))
        p.addCurve(to: CGPoint(x: 10.5, y: 0.8), control1: CGPoint(x: 15, y: 3.2), control2: CGPoint(x: 12.5, y: 1.2))
        p.addCurve(to: CGPoint(x: 10, y: 5.6), control1: CGPoint(x: 9.4, y: 0.6), control2: CGPoint(x: 9.6, y: 3.5))
        p.closeSubpath()
        p.move(to: CGPoint(x: 17, y: 12.4))
        p.addCurve(to: CGPoint(x: 10.5, y: 17.2), control1: CGPoint(x: 15, y: 14.8), control2: CGPoint(x: 12.5, y: 16.8))
        p.addCurve(to: CGPoint(x: 10, y: 12.4), control1: CGPoint(x: 9.4, y: 17.4), control2: CGPoint(x: 9.6, y: 14.5))
        p.closeSubpath()
        return p
    }()

    /// The nozzle, flaring from the tail towards the flame.
    static let nozzlePath: Path = {
        var p = Path()
        p.move(to: CGPoint(x: 10.4, y: 7.2))
        p.addLine(to: CGPoint(x: 8, y: 6.4))
        p.addLine(to: CGPoint(x: 8, y: 11.6))
        p.addLine(to: CGPoint(x: 10.4, y: 10.8))
        p.closeSubpath()
        return p
    }()

    /// The round window, on the body behind the nose.
    static let porthole = CGRect(x: 16.6, y: 7, width: 4, height: 4)

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
