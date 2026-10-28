import SwiftUI
import TCDesign
import TCShellCore

/// "Subscriptions" on the Insights Overview (PR #1608 design, 5.1): per
/// login, the plan, one meter per limit window, credits, why the usage is
/// not known, and when it was read. Drawn only while "Codex usage" is on.
/// Every word is the core's (`SubscriptionUsageWords`); Refresh, at the
/// title's trailing end as every refresh is, is the only thing that asks
/// the Codex app.
struct SubscriptionUsageCard: View {
    let model: SubscriptionUsageModel
    let copy: [String: String]

    private func text(_ key: String) -> String { SubscriptionUsageWords.text(key, copy) }

    var body: some View {
        VStack(alignment: .leading, spacing: InsightsLayout.groupGap) {
            InsightsSectionTitle(title: text(SubscriptionUsageWords.Key.cardTitle), info: []) {
                // Turns while the daemon probes, and takes no second press.
                GlassRefreshButton(text(SubscriptionUsageWords.Key.refreshA11y), isRefreshing: model.refreshing) {
                    model.refresh()
                }
            }
            VStack(alignment: .leading, spacing: InsightsLayout.groupGap) {
                // Read per draw, so "Resets in" and "Updated ago" move on.
                ForEach(model.usage.map { SubscriptionUsageWords.rows($0, copy: copy) } ?? []) { row in
                    login(row)
                }
            }
            .insightsCardBody()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
    }

    private func login(_ row: SubscriptionUsageWords.Row) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                Text(row.title).insightsHeading()
                if let plan = row.planLine { Text(plan).insightsCaption() }
            }
            ForEach(row.meters.filter { !$0.model }) { meter($0) }
            if row.meters.contains(where: \.model) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    ForEach(row.meters.filter(\.model)) { meter($0) }
                }
                .padding(.leading, GlassTokens.Space.s4)
            }
            ForEach(row.credits, id: \.self) { Text($0).insightsNote() }
            ForEach(row.status, id: \.self) { Text($0).insightsNote() }
            if let updated = row.updated { Text(updated).insightsCaption() }
        }
    }

    /// The window's name and "{n}% used" on one line, the bar, then "Resets
    /// in". An unknown window is its words only, never an empty bar.
    private func meter(_ meter: SubscriptionUsageWords.Meter) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s3) {
                // A model's own limit is drawn a step smaller.
                if meter.model {
                    Text(meter.name).insightsCaption()
                } else {
                    Text(meter.name).insightsEyebrow()
                }
                Spacer(minLength: GlassTokens.Space.s3)
                Text(meter.used).insightsNote().monospacedDigit()
            }
            if let fraction = meter.fraction {
                InsightsShareBar(fraction: fraction)
            }
            if let resets = meter.resets { Text(resets).insightsCaption() }
        }
        .accessibilityElement(children: .combine)
    }
}
