import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// Home's Insights hero, its words (owner, 2026-10-12): today's routed
/// tokens as a display figure, or, without a readable glance, the line
/// that says what opening it shows. In the hero's ink.
struct HomeInsightsVisual: View {
    let glance: DaemonData.InsightsGlance?
    let words: MonitorHomeHistoryCopy?

    static let copy = TCInsights.copy() ?? [:]

    var body: some View {
        let bars = HomeInsightsGraphic.bars(glance)
        if bars.isEmpty {
            if let teaser = words?.insightsTeaser {
                Text(teaser)
                    .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                    .foregroundStyle(GlassTokens.Color.heroInk.color.opacity(0.92))
                    .fixedSize(horizontal: false, vertical: true)
            }
        } else if let today = words?.insightsToday {
            let total = bars.reduce(UInt64(0)) { $0 + $1.tokens }
            Text(FirstRunCopy.fill(today, ["total": InsightsOverviewWords.figure(total, copy: Self.copy)]))
                .glassType(GlassTokens.TypeScale.heading.weight(.bold))
                .monospacedDigit()
                .foregroundStyle(GlassTokens.Color.heroInk.color)
        }
    }
}

/// Home's Insights hero, its graphic: today's tokens per tool as bars, the
/// longest first, in the hero's ink with each tool's name under its bar.
/// Without a readable glance, a chart symbol. Figures are the daemon's; a
/// tool whose tokens are unknown draws no bar.
struct HomeInsightsGraphic: View {
    let glance: DaemonData.InsightsGlance?

    /// The most tools drawn: the graphic stays a glance.
    static let toolLimit = 4
    /// Taller than the card, anchored below its bottom edge, so the card's
    /// border cuts the bars off (owner, 2026-10-12).
    static let barWidth: CGFloat = 30
    static let maxBarHeight: CGFloat = 190

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
            GlassHeroSymbol("chart.bar.xaxis", accent: "sparkle")
        } else {
            let ink = GlassTokens.Color.heroInk.color
            let most = Double(bars.first?.tokens ?? 1)
            HStack(alignment: .bottom, spacing: 14) {
                ForEach(bars, id: \.tool) { bar in
                    // The tool's name over its bar: the bar's foot is
                    // past the card's edge.
                    VStack(spacing: GlassTokens.Space.s2) {
                        Text(InferenceTabView.toolName(bar.tool))
                            .glassType(GlassTokens.TypeScale.micro)
                            .foregroundStyle(ink.opacity(0.85))
                            .lineLimit(1)
                            .frame(width: Self.barWidth + 22)
                        RoundedRectangle(cornerRadius: 6, style: .continuous)
                            .fill(LinearGradient(colors: [ink.opacity(0.95), ink.opacity(0.45)],
                                                 startPoint: .top, endPoint: .bottom))
                            .frame(width: Self.barWidth,
                                   height: max(8, Self.maxBarHeight * Double(bar.tokens) / most))
                    }
                }
            }
            .shadow(color: .black.opacity(0.2), radius: 12, y: 8)
            .frame(maxHeight: .infinity, alignment: .bottom)
            .offset(x: 30, y: 40)
        }
    }
}
