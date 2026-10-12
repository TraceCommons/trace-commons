import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// Home's Insights promotion body (owner, 2026-10-12): today's routed
/// tokens as the headline, then a bar per tool, longest first, so the card
/// shows the analytics before it is opened. Without a readable glance it
/// says what opening it shows. Figures are the daemon's; a tool whose
/// tokens are unknown draws no bar.
struct HomeInsightsVisual: View {
    let glance: DaemonData.InsightsGlance?
    let words: MonitorHomeHistoryCopy?

    private static let copy = TCInsights.copy() ?? [:]
    /// The most tools drawn: the card stays a glance.
    static let toolLimit = 3

    static func bars(_ glance: DaemonData.InsightsGlance?) -> [(tool: String, tokens: UInt64)] {
        guard let glance, glance.enabled, glance.readable != false else { return [] }
        return (glance.tools ?? [])
            .compactMap { tool in tool.tokens.map { (tool: tool.tool, tokens: $0) } }
            .filter { $0.tokens > 0 }
            .sorted { $0.tokens > $1.tokens }
            .prefix(toolLimit)
            .map { $0 }
    }

    var body: some View {
        let bars = Self.bars(glance)
        if bars.isEmpty {
            if let teaser = words?.insightsTeaser {
                Text(teaser)
                    .glassType(GlassTokens.TypeScale.label.weight(.regular))
                    .foregroundStyle(GlassColor.textSecondary)
            }
        } else {
            let total = bars.reduce(UInt64(0)) { $0 + $1.tokens }
            let most = Double(bars.first?.tokens ?? 1)
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                if let today = words?.insightsToday {
                    Text(FirstRunCopy.fill(today, ["total": InsightsOverviewWords.figure(total, copy: Self.copy)]))
                        .glassType(GlassTokens.TypeScale.heading.weight(.bold))
                        .monospacedDigit()
                        .foregroundStyle(GlassColor.textPrimary)
                }
                ForEach(bars, id: \.tool) { bar in
                    HStack(spacing: GlassTokens.Space.s3) {
                        Text(InferenceTabView.toolName(bar.tool))
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                            .frame(width: 84, alignment: .leading)
                            .lineLimit(1)
                        GeometryReader { proxy in
                            Capsule()
                                .fill(GlassTokens.Color.purpleSoft.color)
                                .frame(width: max(4, proxy.size.width * Double(bar.tokens) / most))
                        }
                        .frame(height: 6)
                        Text(InsightsOverviewWords.figure(bar.tokens, copy: Self.copy))
                            .glassType(GlassTokens.TypeScale.caption)
                            .monospacedDigit()
                            .foregroundStyle(GlassColor.textTertiary)
                            .fixedSize()
                    }
                    .accessibilityElement(children: .combine)
                }
            }
        }
    }
}
