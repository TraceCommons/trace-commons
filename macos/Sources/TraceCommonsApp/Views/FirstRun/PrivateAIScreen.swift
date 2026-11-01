import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// What the Private AI step shows, apart from the view so it can be tested.
enum PrivateAIScreenLayout {
    /// The tools with persistent connections in Settings. The diagram
    /// shows those found on this Mac; configuration stays in Settings.
    static let routable: [SourceKind] = [.claudeCode, .codex, .opencode]

    /// The found tools Private AI can connect, in ring order.
    static func connectable(_ discovered: [SourceCandidate]?) -> [SourceCandidate] {
        FirstRunStageLayout.ringRows(discovered ?? []).filter { $0.exists && routable.contains($0.source) }
    }

    /// How many tools were found on this Mac.
    static func found(_ discovered: [SourceCandidate]?) -> Int {
        (discovered ?? []).filter(\.exists).count
    }

    /// The line after the figure: "of {total} tools can connect".
    static func ofTotal(_ total: Int, copy: FirstRunCopy.PrivateAi) -> String {
        FirstRunCopy.fill(total == 1 ? copy.canConnectOne : copy.canConnect, ["total": total.formatted()])
    }

    /// The figure counts up one tool at a time, from 0.9s, 0.24s apart.
    static func counted(_ count: Int, after elapsed: Double, reduceMotion: Bool) -> Int {
        guard !reduceMotion else { return count }
        guard elapsed >= 0.9 else { return 0 }
        return min(count, 1 + Int((elapsed - 0.9) / 0.24))
    }

    /// A pill's width, in the canvas.
    static let pillWidth: CGFloat = 88
    /// NEAR AI's pill, at the end of the N's line.
    static let destinationX: CGFloat = 650

    /// Evenly space connected-tool candidates; one sits in the middle.
    static func rowY(_ index: Int, of count: Int) -> CGFloat {
        guard count > 1 else { return 101 }
        return 21 + 160 * CGFloat(index) / CGFloat(count - 1)
    }
}

/// Private AI: the tools that can connect on the left, each one's route
/// into the N, and the N's line on to NEAR AI -- what Private AI does
/// (owner, 2026-10-27). Dashed: no tool switches over until it is
/// connected (`private_ai.offer_lead`). The step has no switch (it changed
/// little); Settings turns Private AI on.
struct PrivateAIScreen: View {
    @EnvironmentObject private var model: AppModel
    let copy: FirstRunCopy
    @ObservedObject var runner: FirstRunRunner
    @ObservedObject var memory: FirstRunStageMemory
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var openedAt = Date()
    @State private var more = false

    private var privateAI: PrivateInferenceCopy? { model.privateInferenceCopy }

    var body: some View {
        let tools = PrivateAIScreenLayout.connectable(memory.discovered)
        FirstRunFrame(
            copy: copy,
            state: $runner.state,
            isCommitting: runner.isCommitting,
            footer: FirstRunFooter(
                title: copy.frame.continueButton, isEnabled: true,
                action: { runner.state = FirstRunNavigation.next(runner.state) })
        ) {
            diagram(tools)
        } title: {
            FirstRunTitle(light: copy.privateAi.titleLight, bold: copy.privateAi.titleBold)
        } content: {
            card(tools)
        }
        .onAppear {
            openedAt = memory.privateCounted ? .distantPast : Date()
            memory.privateCounted = true
        }
    }

    private func diagram(_ tools: [SourceCandidate]) -> some View {
        GlassAnimationTimeline(minimumInterval: 1.0 / 30, paused: reduceMotion) { date in
            let flow = reduceMotion ? 0 : CGFloat((date.timeIntervalSinceReferenceDate / 1.1).truncatingRemainder(dividingBy: 1))
            GeometryReader { geometry in
                let left = (geometry.size.width - FirstRunStageLayout.canvas.width) / 2
                let hub = CGPoint(x: left + 440, y: 101)
                let destination = left + PrivateAIScreenLayout.destinationX
                let half = PrivateAIScreenLayout.pillWidth / 2
                ZStack {
                    ForEach(Array(tools.enumerated()), id: \.element.source) { index, tool in
                        let y = PrivateAIScreenLayout.rowY(index, of: tools.count)
                        // Each tool's route into the N.
                        Path { path in
                            path.move(to: CGPoint(x: left + 254, y: y))
                            path.addCurve(
                                to: CGPoint(x: hub.x - 36, y: hub.y),
                                control1: CGPoint(x: left + 330, y: y), control2: CGPoint(x: left + 350, y: hub.y))
                        }
                        .stroke(GlassTokens.Color.firstRunGlow.color, style: StrokeStyle(lineWidth: 2, dash: [5, 6], dashPhase: -22 * flow))
                        FirstRunToolFace(tool: tool.source.glassTool)
                            .firstRunActor(FirstRunStageLayout.tileActor(tool.source))
                            .frame(width: 42, height: 42)
                            .position(x: left + 230, y: y)
                        Text(tool.source.displayName)
                            .glassType(GlassTokens.TypeScale.caption.weight(.semibold))
                            .foregroundStyle(GlassColor.textSecondary)
                            .position(x: left + 230, y: y + 34)
                    }
                    // The N on to NEAR AI.
                    Path { path in
                        path.move(to: CGPoint(x: hub.x + 36, y: hub.y))
                        path.addLine(to: CGPoint(x: destination - half, y: hub.y))
                    }
                    .stroke(GlassTokens.Color.firstRunGlow.color, style: StrokeStyle(lineWidth: 2, dash: [5, 6], dashPhase: -22 * flow))
                    FirstRunHub(glow: 1)
                        .firstRunActor(FirstRunStageLayout.hubActor)
                        .frame(width: 66, height: 66)
                        .position(hub)
                    if let near = privateAI?.statInferenceAccess {
                        pill(near, lit: true)
                            .position(x: destination, y: hub.y)
                    }
                }
            }
        }
        .frame(height: 221)
        .frame(maxWidth: .infinity)
        .accessibilityHidden(true)
    }

    private func pill(_ text: String, lit: Bool) -> some View {
        Text(text)
            .glassType(GlassTokens.TypeScale.caption.weight(.semibold))
            .foregroundStyle(lit ? GlassColor.accentText : GlassColor.textSecondary)
            .frame(width: PrivateAIScreenLayout.pillWidth, height: 24)
            .background(Capsule().fill(lit ? GlassTokens.Color.tintAccent.color : GlassTokens.Color.tintNeutral.color))
    }

    /// Worded by the Private AI copy, with Learn more for the rest of it.
    private func card(_ tools: [SourceCandidate]) -> some View {
        GlassAnimationTimeline(minimumInterval: 1.0 / 30, paused: reduceMotion) { date in
            let shown = PrivateAIScreenLayout.counted(
                tools.count, after: date.timeIntervalSince(openedAt), reduceMotion: reduceMotion)
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s3) {
                    Text(shown.formatted())
                        .glassType(GlassTokens.TypeScale.number)
                        .foregroundStyle(GlassColor.textPrimary)
                    Text(PrivateAIScreenLayout.ofTotal(PrivateAIScreenLayout.found(memory.discovered), copy: copy.privateAi))
                        .glassType(GlassTokens.TypeScale.body)
                        .foregroundStyle(GlassColor.textSecondary)
                }
                .accessibilityElement(children: .combine)
                if let privateAI {
                    Text(copy.privateAi.offerLead)
                        .glassType(GlassTokens.TypeScale.label.weight(.regular))
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                    if more {
                        ForEach([copy.privateAi.offerMore, privateAI.offerExposure, privateAI.offerNoRepoint], id: \.self) { line in
                            Text(line)
                                .glassType(GlassTokens.TypeScale.label.weight(.regular))
                                .foregroundStyle(GlassColor.textSecondary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                } else {
                    Text(copy.privateAi.unavailable)
                        .glassType(GlassTokens.TypeScale.label.weight(.regular))
                        .foregroundStyle(GlassColor.textSecondary)
                }
                // Learn more on the right; the step has no switch (owner,
                // 2026-10-27), so nothing here turns Private AI on.
                HStack(spacing: GlassTokens.Space.s5) {
                    Spacer(minLength: GlassTokens.Space.s6)
                    if privateAI != nil, !more {
                        Button { more = true } label: {
                            Text(copy.privateAi.learnMore)
                                .glassType(GlassTokens.TypeScale.label.weight(.regular))
                        }
                        .buttonStyle(GlassButtonStyle(.link))
                    }
                }
                .padding(.top, GlassTokens.Space.s4)
            }
            .frame(width: 460, alignment: .leading)
            .firstRunCard(padding: EdgeInsets(top: 14, leading: 20, bottom: 16, trailing: 20))
        }
    }
}
