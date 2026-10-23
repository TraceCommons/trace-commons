import SwiftUI
import TCDesign
import TCShellCore

/// The animated first run's stage (Ron's 2026-10-21 canvas): one scene
/// behind every step, the step thread, and the pieces the steps share --
/// the NEAR mark, the tool tiles, the soft orbs and the marker check.
///
/// The decisions are `FirstRunStageLayout`'s, apart from the views so they
/// can be tested. Every word is the core's; nothing here writes one.
enum FirstRunStageLayout {
    /// The design's own canvas, the first-run window's default size. The
    /// stage lays out in it and scales down to fit a smaller host.
    static let canvas = CGSize(width: 860, height: 760)

    /// The step thread's position of a step: nil on Welcome, which shows
    /// no thread.
    static func threadIndex(of step: FirstRunStep) -> Int? {
        FirstRunNavigation.threadSteps.firstIndex(of: step)
    }

    /// How a stretch of the thread reads: the one before step `index` is
    /// travelled once that step is reached, and the newest one leads.
    enum Segment: Equatable {
        case ahead
        case travelled
        case leading
    }

    static func segment(before index: Int, current: Int) -> Segment {
        if index == current { return .leading }
        return index < current ? .travelled : .ahead
    }

    /// Where a tile sits on a ring of `count`, in degrees clockwise from the
    /// top: evenly spaced from the top.
    static func ringAngle(_ index: Int, of count: Int) -> Double {
        guard count > 0 else { return -90 }
        return -90 + 360 / Double(count) * Double(index)
    }

    /// Where a watched tile circles the N on Join: one on top, two either
    /// side, three as a triangle, four on the diagonals, more evenly.
    static func joinAngle(_ index: Int, of count: Int) -> Double {
        switch count {
        case 1: return -90
        case 2: return [180, 0][index]
        case 3: return [-90, 30, 150][index]
        case 4: return [-135, -45, 45, 135][index]
        default: return ringAngle(index, of: count)
        }
    }

    /// The point at `radius` and `degrees` round `center`.
    static func point(_ center: CGPoint, radius: CGFloat, degrees: Double) -> CGPoint {
        let radians = degrees * .pi / 180
        return CGPoint(x: center.x + radius * CGFloat(cos(radians)), y: center.y + radius * CGFloat(sin(radians)))
    }

    /// The scan on Tools: one turn of the sweep, after a lead-in, from 36
    /// degrees before the top to the top again.
    static let scanLead: Double = 1
    static let scanTurn: Double = 2
    static let sweepStart: Double = -36

    /// When the sweep's edge reaches tile `index` of `count`, in seconds
    /// from the scan's start: it settles there, found or not.
    static func scanMoment(_ index: Int, of count: Int) -> Double {
        guard count > 0 else { return scanLead }
        let angle = 360 / Double(count) * Double(index)
        return scanLead + scanTurn * (angle - sweepStart) / (360 - sweepStart)
    }

    /// The scan is over: every tile has settled.
    static func scanDone(after elapsed: Double) -> Bool {
        elapsed >= scanLead + scanTurn + 0.25
    }

    /// The sweep's rotation, in degrees clockwise from the top, `elapsed`
    /// seconds into the scan; nil before and after its turn.
    static func sweepRotation(after elapsed: Double) -> Double? {
        let into = elapsed - scanLead
        guard into >= 0, into <= scanTurn else { return nil }
        return sweepStart + (360 - sweepStart) * into / scanTurn
    }

    /// The ease-out cubic the count on Traces rises along.
    static func countEase(_ progress: Double) -> Double {
        let p = min(1, max(0, progress))
        return 1 - pow(1 - p, 3)
    }

    /// A spring-like settle for a value moving from 0 to 1 over `duration`,
    /// overshooting a little: the tiles bursting out of the N.
    static func burst(_ elapsed: Double, duration: Double) -> Double {
        guard elapsed > 0 else { return 0 }
        guard elapsed < duration else { return 1 }
        let t = elapsed / duration
        return 1 - exp(-6 * t) * cos(7.5 * t)
    }
}

// MARK: - The scene

/// The navy base every step shares, with the brand-blue glow breathing from
/// the top-left corner (Ron, 2026-10-21: "a soft brand accent blue gradient
/// glow emanating from the upper left corner"). Light follows the system
/// appearance through the tokens.
struct FirstRunBackdrop: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        GeometryReader { geometry in
            let size = geometry.size
            let strong = scheme == .dark ? 0.5 : 0.3
            let soft = scheme == .dark ? 0.14 : 0.08
            let glow = GlassTokens.Color.firstRunGlow
            ZStack(alignment: .topLeading) {
                RadialGradient(
                    colors: [GlassTokens.Color.sceneWarm.color, GlassTokens.Color.sceneBase.color],
                    center: .top, startRadius: 0, endRadius: max(size.width, size.height) * 0.9)
                EllipticalGradient(
                    stops: [
                        .init(color: glow.opacity(strong).color, location: 0),
                        .init(color: glow.opacity(soft).color, location: 0.42),
                        .init(color: glow.opacity(0).color, location: 0.75),
                    ],
                    center: .center)
                    .frame(width: size.width * 2.4, height: size.height * 1.9)
                    .offset(x: -size.width * 1.2, y: -size.height * 0.95)
                GlassAnimationTimeline(minimumInterval: 1.0 / 30, paused: reduceMotion) { date in
                    // Seven seconds each way, eased at both ends.
                    let phase = reduceMotion ? 1 : 0.5 - 0.5 * cos(.pi * date.timeIntervalSinceReferenceDate / 7)
                    Ellipse()
                        .fill(RadialGradient(
                            colors: [glow.opacity(strong).color, glow.opacity(0).color],
                            center: .center, startRadius: 0, endRadius: 300))
                        .frame(width: 640, height: 580)
                        .blur(radius: 30)
                        .scaleEffect(0.9 + 0.25 * phase, anchor: .topLeading)
                        .opacity(0.6 + 0.4 * phase)
                        .offset(x: -140, y: -140)
                }
            }
            .frame(width: size.width, height: size.height, alignment: .topLeading)
            .clipped()
        }
        .ignoresSafeArea()
        .accessibilityHidden(true)
    }
}

// MARK: - The step thread

/// The step titles hung on one thread (Ron, 2026-10-22): the stretch
/// already travelled glows and runs on to the new step, which lights up as
/// it arrives; the stretch still to go is dotted. A step already passed is
/// a button back to it.
struct FirstRunStepThread: View {
    let labels: [String]
    let current: Int
    var enabled = true
    let onSelect: (Int) -> Void
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    static let segmentWidth: CGFloat = 38

    var body: some View {
        HStack(spacing: 0) {
            ForEach(labels.indices, id: \.self) { index in
                if index > 0 { segment(before: index) }
                title(index)
            }
        }
        .frame(height: 28)
        .accessibilityElement(children: .contain)
    }

    private func segment(before index: Int) -> some View {
        let state = FirstRunStageLayout.segment(before: index, current: current)
        let glow = GlassTokens.Color.firstRunGlow
        return ZStack(alignment: .leading) {
            Path { path in
                path.move(to: CGPoint(x: 0, y: 1.5))
                path.addLine(to: CGPoint(x: Self.segmentWidth, y: 1.5))
            }
            .stroke(GlassTokens.Color.firstRunRule.color, style: StrokeStyle(lineWidth: 2, lineCap: .butt, dash: [2, 4]))
            Capsule()
                .fill(
                    LinearGradient(
                        colors: state == .leading
                            ? [glow.color, GlassTokens.Color.firstRunBeamCore.color] : [glow.color, glow.color],
                        startPoint: .leading, endPoint: .trailing)
                )
                .frame(width: state == .ahead ? 0 : Self.segmentWidth, height: 3)
                .shadow(color: glow.opacity(0.35).color, radius: state == .leading ? 5 : 4)
                .animation(GlassMotion.timed(reduceMotion, duration: 0.52), value: current)
        }
        .frame(width: Self.segmentWidth, height: 3)
        .accessibilityHidden(true)
    }

    @ViewBuilder private func title(_ index: Int) -> some View {
        let isCurrent = index == current
        let text = Text(labels[index])
            .glassType(GlassTokens.TypeScale.label.weight(isCurrent ? .semibold : .medium))
            .foregroundStyle(
                isCurrent ? GlassColor.textPrimary : index < current ? GlassColor.textSecondary : GlassColor.textTertiary)
            .shadow(color: isCurrent ? GlassTokens.Color.firstRunGlow.opacity(0.35).color : .clear, radius: 7)
            .animation(GlassMotion.timed(reduceMotion, duration: 0.3)?.delay(0.42), value: current)
            .padding(.horizontal, 10)
            .frame(height: 28)
        if index < current {
            Button { onSelect(index) } label: { text.contentShape(Rectangle()) }
                .buttonStyle(.plain)
                .disabled(!enabled)
        } else {
            text.accessibilityAddTraits(isCurrent ? .isSelected : [])
        }
    }
}

// MARK: - The NEAR mark

/// The NEAR mark at the centre of each step, with the glow behind it that
/// breathes (4s). `charged` brightens it while a tool is watched on Tools;
/// `glow` scales the glow, which Private AI dims until its switch is on.
/// Welcome passes `drawn` and `shine` for the marker and the light band.
///
/// It fills the size it is given, so the same mark can move and grow from
/// one step's place to the next (`firstRunActor`); the caller frames it.
struct FirstRunHub: View {
    var drawn: Double = 1
    var shine: Double? = nil
    var showsTip = false
    var glow: Double = 1
    var charged = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        GeometryReader { geometry in
            let size = min(geometry.size.width, geometry.size.height)
            let disc = size * 220 / 132
            ZStack {
                GlassAnimationTimeline(minimumInterval: 1.0 / 30, paused: reduceMotion) { date in
                    let breath = breathing(date.timeIntervalSinceReferenceDate)
                    Circle()
                        .fill(RadialGradient(
                            colors: [
                                GlassTokens.Color.firstRunGlow.opacity(scheme == .dark ? 0.35 : 0.2).color,
                                GlassTokens.Color.firstRunGlow.opacity(0).color,
                            ],
                            center: .center, startRadius: 0, endRadius: disc * 0.31))
                        .frame(width: disc, height: disc)
                        .scaleEffect(breath.scale)
                        .opacity(breath.opacity * glow)
                }
                .frame(width: disc, height: disc)
                GlassBrandMarkDrawing(
                    fill: GlassTokens.Color.firstRunGlow.color, drawn: drawn, shine: shine,
                    tip: showsTip ? GlassTokens.Color.heroInk.color : nil,
                    tipGlow: GlassTokens.Color.firstRunGlow.color)
                    .frame(width: size, height: size)
            }
            .frame(width: geometry.size.width, height: geometry.size.height)
        }
        .accessibilityHidden(true)
    }

    private func breathing(_ time: TimeInterval) -> (scale: Double, opacity: Double) {
        guard !reduceMotion else { return (1, 1) }
        if charged {
            let p = 0.5 - 0.5 * cos(2 * .pi * time / 1.8)
            return (1.02 + 0.12 * p, 0.9 + 0.1 * p)
        }
        let p = 0.5 - 0.5 * cos(2 * .pi * time / 4)
        return (1 + 0.08 * p, 1 - 0.25 * p)
    }
}

// MARK: - Tools, orbs and the marker check

/// A tool's tile on the stage: its logo in its tint on a raised face, or its
/// initials. `lit` glows it while the tool is watched. It fills the size it
/// is given, so a tile can move and shrink between steps; the caller frames
/// it.
struct FirstRunToolFace: View {
    let tool: GlassTool
    var lit = false

    var body: some View {
        GeometryReader { geometry in
            let side = min(geometry.size.width, geometry.size.height)
            RoundedRectangle(cornerRadius: side * 14 / 52, style: .continuous)
                .fill(GlassTokens.Color.firstRunTile.color)
                .shadow(color: GlassTokens.Color.firstRunTileShadow.color, radius: side * 10 / 52, y: side * 8 / 52)
                .overlay {
                    if let logo = tool.logo {
                        GlassToolLogoShape(logo)
                            .fill(tool.tint.color)
                            .frame(width: side / 2, height: side / 2)
                    } else {
                        Text(tool.initials)
                            .glassGlyph(max(6, side * 16 / 52), weight: .bold)
                            .foregroundStyle(tool.tint.color)
                    }
                }
                .shadow(color: lit ? GlassTokens.Color.firstRunGlow.opacity(0.35).color : .clear, radius: side * 11 / 52)
                .frame(width: side, height: side)
                .frame(width: geometry.size.width, height: geometry.size.height)
        }
        .accessibilityHidden(true)
    }
}

/// A trace in transit, or one arrived: a soft glowing orb, white at its
/// heart fading through the beam's blue to nothing.
struct FirstRunOrb: View {
    var diameter: CGFloat = 18

    var body: some View {
        Circle()
            .fill(RadialGradient(
                stops: [
                    .init(color: GlassTokens.Color.heroInk.color, location: 0),
                    .init(color: GlassTokens.Color.firstRunBeamCore.color, location: 0.22),
                    .init(color: GlassTokens.Color.firstRunBeam.opacity(0.65).color, location: 0.46),
                    .init(color: GlassTokens.Color.firstRunGlow.opacity(0).color, location: 0.72),
                ],
                center: .center, startRadius: 0, endRadius: diameter / 2))
            .frame(width: diameter, height: diameter)
            .accessibilityHidden(true)
    }
}

/// The oversized check drawn like a marker stroke that runs past its box
/// (Ron, 2026-10-22): ticked is Watch, unticked is Not used. It is a
/// checkbox to assistive tech, named by the caller's label.
struct FirstRunMarkerCheckStyle: ToggleStyle {
    func makeBody(configuration: Configuration) -> some View {
        FirstRunMarkerCheck(configuration: configuration)
    }
}

private struct FirstRunMarkerCheck: View {
    let configuration: ToggleStyleConfiguration
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.isEnabled) private var isEnabled
    @State private var hovered = false

    var body: some View {
        let on = configuration.isOn
        Button { configuration.isOn.toggle() } label: {
            ZStack(alignment: .topLeading) {
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(on ? GlassTokens.Color.tintAccent.color : .clear)
                    .overlay(
                        RoundedRectangle(cornerRadius: 6, style: .continuous)
                            .strokeBorder(
                                on ? GlassTokens.Color.firstRunGlow.color
                                    : (hovered ? GlassColor.textPrimary : GlassColor.textTertiary),
                                lineWidth: 1.5)
                    )
                    .frame(width: 22, height: 22)
                    .offset(x: 2, y: 2)
                FirstRunMarkerCheckShape()
                    .trim(from: 0, to: on ? 1 : 0)
                    .stroke(GlassTokens.Color.toggleOn.color, style: StrokeStyle(lineWidth: 3.6, lineCap: .round, lineJoin: .round))
                    .frame(width: 44, height: 44)
                    .offset(x: -6, y: -14)
                    .animation(GlassMotion.timed(reduceMotion, duration: on ? 0.34 : 0.16), value: on)
                    .allowsHitTesting(false)
            }
            .frame(width: 26, height: 26, alignment: .topLeading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .opacity(isEnabled ? 1 : 0.45)
        .onHover { hovered = $0 }
        .accessibilityRepresentation {
            // A native toggle, so the system states checked or unchecked in
            // the person's language, as `GlassCheckboxStyle` does.
            Toggle(isOn: configuration.$isOn) { configuration.label }
        }
    }
}

/// The check's stroke in its own 44 by 44 box: a short down-stroke, then a
/// flick up and a little out past the box's corner (shortened by a quarter,
/// Ron 2026-10-22: the tail ran too far out).
struct FirstRunMarkerCheckShape: Shape {
    func path(in rect: CGRect) -> Path {
        let sx = rect.width / 44
        let sy = rect.height / 44
        func p(_ x: CGFloat, _ y: CGFloat) -> CGPoint { CGPoint(x: rect.minX + x * sx, y: rect.minY + y * sy) }
        var path = Path()
        path.move(to: p(11, 27))
        path.addCurve(to: p(18, 35), control1: p(13, 29), control2: p(15.5, 32))
        path.addCurve(to: p(35, 12), control1: p(22, 27.5), control2: p(28, 18.5))
        return path
    }
}

/// The canvas's small segmented control (`.seg.sm`, Ron 2026-10-22): a
/// well holding one pill per option, the chosen one in the accent, with no
/// status dot. The caller's label names the group to assistive tech; each
/// option is a button that says whether it is chosen.
struct FirstRunSegmented<Value: Hashable>: View {
    struct Option {
        let title: String
        let value: Value
    }

    let label: String
    @Binding var selection: Value
    let options: [Option]
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var hovered: Value?

    var body: some View {
        HStack(spacing: 2) {
            ForEach(options, id: \.value) { option in
                let chosen = option.value == selection
                Button {
                    selection = option.value
                } label: {
                    Text(option.title)
                        .glassType(GlassTokens.TypeScale.caption.weight(.medium))
                        .foregroundStyle(chosen ? GlassTokens.Color.textOnAccent.color : GlassColor.textPrimary)
                        .padding(.horizontal, 10)
                        .frame(height: 22)
                        .background(
                            Capsule().fill(
                                chosen ? GlassTokens.Color.toggleOn.color
                                    : hovered == option.value ? GlassTokens.Color.firstRunControlHover.color : .clear))
                        .contentShape(Capsule())
                }
                .buttonStyle(.plain)
                .onHover { hovered = $0 ? option.value : (hovered == option.value ? nil : hovered) }
                .accessibilityAddTraits(chosen ? .isSelected : [])
                .animation(GlassMotion.timed(reduceMotion, duration: 0.15), value: chosen)
            }
        }
        .padding(2)
        .background(Capsule().fill(GlassTokens.Color.wellFill.color))
        .fixedSize()
        .accessibilityElement(children: .contain)
        .accessibilityLabel(label)
    }
}

/// The canvas's small switch (`.switch.sm`, 30 by 18) with its label after
/// it, as Customize's past traces read (Ron, 2026-10-22: toggles on the
/// left). It is a switch to assistive tech, named by the label.
struct FirstRunSwitchStyle: ToggleStyle {
    func makeBody(configuration: Configuration) -> some View {
        FirstRunSwitch(configuration: configuration)
    }
}

private struct FirstRunSwitch: View {
    let configuration: ToggleStyleConfiguration
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.isEnabled) private var isEnabled

    var body: some View {
        let on = configuration.isOn
        HStack(spacing: 10) {
            Button {
                withAnimation(GlassMotion.timed(reduceMotion, duration: 0.2)) { configuration.isOn.toggle() }
            } label: {
                ZStack(alignment: on ? .trailing : .leading) {
                    Capsule()
                        .fill(on ? GlassTokens.Color.toggleOn.color : GlassTokens.Color.toggleOff.color)
                    Circle()
                        .fill((on ? GlassTokens.Color.textOnAccent : GlassTokens.Color.toggleKnobOff).color)
                        .frame(width: 14, height: 14)
                        .padding(2)
                }
                .frame(width: 30, height: 18)
            }
            .buttonStyle(.plain)
            configuration.label
        }
        .opacity(isEnabled ? 1 : GlassTokens.Opacity.disabled)
        .accessibilityRepresentation {
            Toggle(isOn: configuration.$isOn) { configuration.label }
        }
    }
}

// MARK: - Actors

extension EnvironmentValues {
    /// The namespace the first run's actors -- the N and the tool tiles --
    /// share across steps, so each step moves them instead of cutting to a
    /// new page. Nil outside the first-run host.
    @Entry var firstRunActors: Namespace.ID? = nil
}

extension View {
    /// Mark this view as the first run's actor `id`: the same object on
    /// every step, moved to where the next step draws it.
    func firstRunActor(_ id: String) -> some View {
        modifier(FirstRunActorModifier(id: id))
    }
}

private struct FirstRunActorModifier: ViewModifier {
    let id: String
    @Environment(\.firstRunActors) private var namespace

    func body(content: Content) -> some View {
        if let namespace {
            content.matchedGeometryEffect(id: id, in: namespace, properties: .frame)
        } else {
            content
        }
    }
}

extension FirstRunStageLayout {
    /// The N's actor id.
    static let hubActor = "hub"

    /// A tool tile's actor id.
    static func tileActor(_ kind: SourceKind) -> String { "tile-" + kind.rawValue }

    /// The order the Tools ring holds every tool the shell knows, as the
    /// canvas lays them out: discovery's rows in this order.
    static let ringOrder: [SourceKind] = [.claudeCode, .geminiCli, .codex, .cline, .opencode]

    /// Discovery's rows in ring order; a kind discovery did not report is
    /// left out.
    static func ringRows(_ rows: [SourceCandidate]) -> [SourceCandidate] {
        ringOrder.compactMap { kind in rows.first { $0.source == kind } }
    }
}

// MARK: - What the stage remembers

/// What has already played in this first run, so going Back to a step does
/// not replay it: the Tools scan, the Traces count, the Private AI count.
/// Held by the host for the whole first run.
@MainActor
final class FirstRunStageMemory: ObservableObject {
    /// When the Tools scan started, while it runs and settles; nil before
    /// and after.
    @Published private(set) var scanStartedAt: Date?
    /// The scan is over: the tiles have settled and the list is up.
    @Published private(set) var toolsScanned = false
    @Published var tracesCounted = false
    @Published var privateCounted = false
    /// Discovery's rows as Tools last read them, for Traces' counts and
    /// Private AI's tools.
    @Published var discovered: [SourceCandidate]?

    /// Start the Tools scan, once per first run. Under Reduce Motion it is
    /// over at once.
    func startScan(reduceMotion: Bool) {
        guard scanStartedAt == nil, !toolsScanned else { return }
        guard !reduceMotion else {
            toolsScanned = true
            return
        }
        scanStartedAt = Date()
        Task { @MainActor [weak self] in
            let turn = FirstRunStageLayout.scanLead + FirstRunStageLayout.scanTurn + 0.25
            try? await Task.sleep(for: .seconds(turn))
            self?.toolsScanned = true
            // The ring re-spaces for 0.9s after the turn; then it is settled.
            try? await Task.sleep(for: .seconds(1))
            self?.scanStartedAt = nil
        }
    }
}

extension View {
    /// The first run's card over the scene: its padding, and the card fill.
    func firstRunCard(
        padding: EdgeInsets = EdgeInsets(top: 4, leading: 16, bottom: 8, trailing: 16)
    ) -> some View {
        self.padding(padding)
            .background(
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .fill(GlassTokens.Color.firstRunCard.color))
    }
}
