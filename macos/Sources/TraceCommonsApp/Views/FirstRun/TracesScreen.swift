import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// What Traces shows, apart from the view so it can be tested: the watched
/// tools' traces counted, and what they could earn, an estimate in the core's
/// words. A per-trace range is known only from an invite's lookup, so before
/// joining the card says that points come once the person joins, and no
/// figure is made up.
enum TracesScreenLayout {
    /// A watched tool and its traces on this Mac.
    struct Row: Equatable {
        let kind: SourceKind
        let traces: UInt64
    }

    /// The watched tools found on this Mac, with their trace counts.
    static func rows(_ state: FirstRunState, discovered: [SourceCandidate]?) -> [Row] {
        ToolsScreenLayout.watchedKinds(state).map { kind in
            Row(kind: kind, traces: discovered?.first { $0.source == kind && $0.exists }?.sessionCount ?? 0)
        }
    }

    static func total(_ rows: [Row]) -> UInt64 {
        rows.reduce(0) { $0 + $1.traces }
    }

    /// The headline estimate, or nil without a range in the one unit the
    /// daemon accepts.
    static func points(total: UInt64, lookup: DaemonData.InviteLookup?, copy: FirstRunCopy.Traces) -> String? {
        guard let range = lookup?.creditRange, range.unit == JoinScreenLayout.pointsPerAcceptedTrace else { return nil }
        let low = (total * UInt64(max(0, range.min))).formatted()
        let high = (total * UInt64(max(0, range.max))).formatted()
        if range.min == range.max { return FirstRunCopy.fill(copy.pointsOne, ["min": low]) }
        return FirstRunCopy.fill(copy.points, ["min": low, "max": high])
    }

    /// The line under it: the range per trace, or that points come once the
    /// person joins.
    static func rate(lookup: DaemonData.InviteLookup?, copy: FirstRunCopy.Traces) -> String {
        guard let range = lookup?.creditRange, range.unit == JoinScreenLayout.pointsPerAcceptedTrace else {
            return copy.rateUnknown
        }
        if range.min == range.max { return FirstRunCopy.fill(copy.rateOne, ["min": "\(range.min)"]) }
        return FirstRunCopy.fill(copy.rate, ["min": "\(range.min)", "max": "\(range.max)"])
    }

    /// The count rises after the orbs start, over 1.9s.
    static let countDelay: Double = 0.9
    static let countDuration: Double = 1.9

    /// How far the count has risen, `elapsed` seconds after Traces opened.
    static func counted(after elapsed: Double, reduceMotion: Bool) -> Double {
        guard !reduceMotion else { return 1 }
        return FirstRunStageLayout.countEase((elapsed - countDelay) / countDuration)
    }

    /// A row's height in the column of tiles, and its centre.
    static let rowPitch: CGFloat = 62
    static func rowY(_ index: Int, of count: Int, center: CGFloat) -> CGFloat {
        center - CGFloat(count - 1) * rowPitch / 2 + CGFloat(index) * rowPitch
    }

    /// A stream from just past a row's label into the ring.
    static func stream(from start: CGPoint, to end: CGPoint) -> (CGPoint, CGPoint, CGPoint, CGPoint) {
        (start, CGPoint(x: start.x + 52, y: start.y), CGPoint(x: end.x - 36, y: end.y), end)
    }

    /// A point on a cubic Bezier.
    static func bezier(_ curve: (CGPoint, CGPoint, CGPoint, CGPoint), at t: CGFloat) -> CGPoint {
        let u = 1 - t
        let a = u * u * u, b = 3 * u * u * t, c = 3 * u * t * t, d = t * t * t
        return CGPoint(
            x: a * curve.0.x + b * curve.1.x + c * curve.2.x + d * curve.3.x,
            y: a * curve.0.y + b * curve.1.y + c * curve.2.y + d * curve.3.y)
    }
}

/// Traces: the watched tools in a column, soft orbs streaming from each into
/// a ring that fills as the count of their traces rises, the N inside it,
/// and what those traces could earn.
struct TracesScreen: View {
    let copy: FirstRunCopy
    @ObservedObject var runner: FirstRunRunner
    @ObservedObject var memory: FirstRunStageMemory
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var openedAt = Date()

    var body: some View {
        let rows = TracesScreenLayout.rows(runner.state, discovered: memory.discovered)
        let total = TracesScreenLayout.total(rows)
        FirstRunFrame(
            copy: copy,
            state: $runner.state,
            isCommitting: runner.isCommitting,
            footer: FirstRunFooter(
                title: copy.frame.continueButton, isEnabled: true,
                action: { runner.state = FirstRunNavigation.next(runner.state) })
        ) {
            graphic(rows: rows, total: total)
        } title: {
            FirstRunTitle(light: copy.traces.titleLight, bold: copy.traces.titleBold)
        } content: {
            estimate(total: total)
        }
        .onAppear {
            openedAt = memory.tracesCounted ? .distantPast : Date()
        }
    }

    private func graphic(rows: [TracesScreenLayout.Row], total: UInt64) -> some View {
        GlassAnimationTimeline(minimumInterval: 1.0 / 60, paused: reduceMotion) { date in
            let elapsed = date.timeIntervalSince(openedAt)
            let counted = TracesScreenLayout.counted(after: elapsed, reduceMotion: reduceMotion)
            GeometryReader { geometry in
                let left = (geometry.size.width - FirstRunStageLayout.canvas.width) / 2
                let center = CGPoint(x: left + 557, y: 114)
                ZStack {
                    ForEach(Array(rows.enumerated()), id: \.element.kind) { index, row in
                        let y = TracesScreenLayout.rowY(index, of: rows.count, center: center.y)
                        stream(row: row, index: index, y: y, left: left, center: center, elapsed: elapsed)
                        FirstRunToolFace(tool: row.kind.glassTool)
                            .firstRunActor(FirstRunStageLayout.tileActor(row.kind))
                            .frame(width: 36, height: 36)
                            .position(x: left + 207, y: y)
                        VStack(alignment: .leading, spacing: 0) {
                            Text(row.kind.displayName)
                                .glassType(GlassTokens.TypeScale.bodyStrong)
                                .foregroundStyle(GlassColor.textPrimary)
                            Text(FirstRunCopy.fill(copy.frame.sessionCount, ["count": row.traces.formatted()]))
                                .glassType(GlassTokens.TypeScale.caption)
                                .foregroundStyle(GlassColor.textSecondary)
                        }
                        .fixedSize()
                        .onGeometryChange(for: CGFloat.self, of: \.size.width) { width in
                            labelWidths[row.kind] = width
                        }
                        .position(x: left + 237 + (labelWidths[row.kind] ?? 110) / 2, y: y)
                    }
                    Circle()
                        .stroke(GlassTokens.Color.wellFill.color, lineWidth: 8)
                        .frame(width: 220, height: 220)
                        .position(center)
                    Circle()
                        .trim(from: 0, to: counted)
                        .stroke(GlassTokens.Color.firstRunGlow.color, style: StrokeStyle(lineWidth: 8, lineCap: .round))
                        .rotationEffect(.degrees(-90))
                        .shadow(color: GlassTokens.Color.firstRunGlow.opacity(0.35).color, radius: 6)
                        .frame(width: 220, height: 220)
                        .opacity(counted > 0 ? 1 : 0)
                        .position(center)
                    FirstRunHub()
                        .firstRunActor(FirstRunStageLayout.hubActor)
                        .frame(width: 36, height: 36)
                        .position(x: center.x, y: center.y - 52)
                    VStack(spacing: 0) {
                        Text((UInt64((Double(total) * counted).rounded())).formatted())
                            .glassType(GlassTokens.TypeScale.number)
                            .foregroundStyle(GlassColor.textPrimary)
                            .scaleEffect(1.5)
                            .frame(height: 48)
                        Text(copy.traces.onThisMac)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                    }
                    .position(x: center.x, y: center.y + 8)
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(Text("\(total.formatted()) \(copy.traces.onThisMac)"))
                }
            }
            .onChange(of: counted >= 1) { _, done in
                if done { memory.tracesCounted = true }
            }
        }
        .frame(height: 228)
        .frame(maxWidth: .infinity)
    }

    @State private var labelWidths: [SourceKind: CGFloat] = [:]

    private func stream(
        row: TracesScreenLayout.Row, index: Int, y: CGFloat, left: CGFloat, center: CGPoint, elapsed: Double
    ) -> some View {
        let start = CGPoint(x: left + 237 + (labelWidths[row.kind] ?? 110) + 12, y: y)
        let curve = TracesScreenLayout.stream(from: start, to: CGPoint(x: center.x - 114, y: center.y))
        let flowing = !reduceMotion && elapsed >= 0.5 && elapsed < TracesScreenLayout.countDelay + TracesScreenLayout.countDuration + 0.4
        return ZStack {
            Path { path in
                path.move(to: curve.0)
                path.addCurve(to: curve.3, control1: curve.1, control2: curve.2)
            }
            .stroke(GlassTokens.Color.rule.color, style: StrokeStyle(lineWidth: 1.5, dash: [2, 5]))
            if flowing {
                ForEach(0..<3, id: \.self) { orb in
                    let into = (elapsed - 0.5 - Double(orb) * 0.46 - Double(index) * 0.15) / 1.38
                    if into >= 0 {
                        let t = into.truncatingRemainder(dividingBy: 1)
                        let eased = GlassMotionCurve.value(t)
                        FirstRunOrb(diameter: 14)
                            .opacity(t < 0.12 ? t / 0.12 : (t > 0.88 ? (1 - t) / 0.12 : 1))
                            .position(TracesScreenLayout.bezier(curve, at: CGFloat(eased)))
                    }
                }
            }
        }
        .accessibilityHidden(true)
    }

    private func estimate(total: UInt64) -> some View {
        let done = memory.tracesCounted || reduceMotion
        return VStack(spacing: GlassTokens.Space.s1) {
            if let points = TracesScreenLayout.points(total: total, lookup: runner.lookup, copy: copy.traces) {
                Text(points)
                    .glassType(GlassTokens.TypeScale.heading)
                    .foregroundStyle(GlassColor.textPrimary)
            }
            Text(TracesScreenLayout.rate(lookup: runner.lookup, copy: copy.traces))
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
        }
        .multilineTextAlignment(.center)
        .firstRunCard(padding: EdgeInsets(top: 12, leading: 24, bottom: 12, trailing: 24))
        .fixedSize()
        .opacity(done ? 1 : 0)
        .offset(y: done ? 0 : 8)
        .animation(GlassMotion.spatial(reduceMotion, duration: 0.64), value: done)
    }
}
