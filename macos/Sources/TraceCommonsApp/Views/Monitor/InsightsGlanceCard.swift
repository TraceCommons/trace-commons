import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The menu-bar glance: today's routed calls per tool, from the proxy
/// ledger (`insights_glance`), laid out like the popover's recent
/// activity. One row per tool in the daemon's order; tools are never
/// added together. Every word is the core's (`tc_insights_copy_json`).
///
/// The panel draws it only for fresh data with rows
/// (`MenuPanelData.glanceToDraw`). The context tip shows only when lit,
/// and has no mute: no daemon setting stands behind one yet.
struct InsightsGlanceCard: View {
    let glance: DaemonData.InsightsGlance
    /// Opens the Inference tab, where each call's tokens are.
    let open: () -> Void

    private static let copy = TCInsights.copy() ?? [:]

    var body: some View {
        let copy = Self.copy
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: GlassTokens.Space.s2) {
                Text(InsightsOverviewWords.text("analytics_today", copy))
                    .glassType(GlassTokens.TypeScale.label.weight(.regular))
                    .foregroundStyle(GlassColor.textTertiary)
                if let partial = InsightsGlanceWords.dayPartial(glance, copy: copy) {
                    GlassTag(partial)
                }
                Spacer(minLength: GlassTokens.Space.s2)
                Text(InsightsOverviewWords.text("analytics_glance_routed_only", copy))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
            }
            .padding(.horizontal, GlassTokens.Space.s3)
            .padding(.top, GlassTokens.Space.s1)
            ForEach(Array((glance.tools ?? []).enumerated()), id: \.offset) { _, tool in
                GlassActivityRow(
                    tool: MenuPanelData.tool(tool.tool),
                    text: InsightsGlanceWords.rowText(tool, copy: copy),
                    trailing: InsightsGlanceWords.partial(tool, copy: copy),
                    action: open)
                    .accessibilityValue(InsightsGlanceWords.accessibilityValue(tool) ?? "")
            }
            if let tip = InsightsGlanceWords.tip(glance, copy: copy) {
                Text(tip)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .padding(.horizontal, GlassTokens.Space.s3)
                    .padding(.top, GlassTokens.Space.s1)
                    .accessibilityHint(InsightsOverviewWords.text("analytics_mark_a11y_tip", copy))
            }
        }
    }
}

/// The glance's words: the core's templates with the daemon's figures in
/// their holes. A figure the daemon does not know is the dash, never zero.
enum InsightsGlanceWords {
    /// "{tokens} tokens · {p}% of input from cache"; the tokens alone when
    /// the known calls read no input; the dash when no call is known.
    static func figureLine(_ tool: DaemonData.InsightsGlanceTool, copy: [String: String]) -> String {
        guard let tokens = tool.tokens else { return InsightsOverviewWords.text("analytics_unavailable", copy) }
        let figure = InsightsOverviewWords.figure(tokens, copy: copy)
        guard let share = tool.cacheShare else {
            return InsightsOverviewWords.fill(
                InsightsOverviewWords.text("analytics_glance_tokens_only", copy), ["tokens": figure])
        }
        return InsightsOverviewWords.fill(
            InsightsOverviewWords.text("analytics_glance_line", copy),
            ["tokens": figure, "p": InsightsOverviewWords.percent(share.permille)])
    }

    /// The tool's name and its figures, with the recent rows' joiner.
    static func rowText(_ tool: DaemonData.InsightsGlanceTool, copy: [String: String]) -> String {
        "\(InferenceTabView.toolName(tool.tool)) · \(figureLine(tool, copy: copy))"
    }

    /// Partial when some, but not all, of the tool's calls are known: its
    /// tokens cover only those.
    static func partial(_ tool: DaemonData.InsightsGlanceTool, copy: [String: String]) -> String? {
        guard tool.knownCalls > 0, tool.knownCalls < tool.calls else { return nil }
        return InsightsOverviewWords.text("analytics_state_partial", copy)
    }

    /// Partial for the whole day when ledger rows could not be read.
    static func dayPartial(_ glance: DaemonData.InsightsGlance, copy: [String: String]) -> String? {
        guard let unreadable = glance.coverage?.unreadableRows, unreadable > 0 else { return nil }
        return InsightsOverviewWords.text("analytics_state_partial", copy)
    }

    /// An unknown figure is read out as unknown rather than as the dash.
    static func accessibilityValue(_ tool: DaemonData.InsightsGlanceTool) -> String? {
        tool.tokens == nil ? MonitorWords.unknown : nil
    }

    /// The context tip's two sentences, only for a lit tip.
    static func tip(_ glance: DaemonData.InsightsGlance, copy: [String: String]) -> String? {
        guard let lit = glance.contextTip?.lit else { return nil }
        let context = InsightsOverviewWords.fill(
            InsightsOverviewWords.text("analytics_tip_context", copy),
            ["ctx": InsightsOverviewWords.figure(lit.context, copy: copy)])
        let threshold = InsightsOverviewWords.fill(
            InsightsOverviewWords.text("analytics_tip_threshold", copy),
            ["threshold": InsightsOverviewWords.figure(UInt64(max(lit.threshold, 0)), copy: copy)])
        return context + " " + threshold
    }
}
