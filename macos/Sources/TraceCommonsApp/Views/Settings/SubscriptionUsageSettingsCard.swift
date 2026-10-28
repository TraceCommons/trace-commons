import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// "Subscription usage" in Settings (PR #1608 design, 3.4 and 4.1): the
/// "Codex usage" switch (`subscription_usage.codex`, off by default) and
/// what it does. Drawn only from the daemon's own value, so a daemon built
/// without the `subscription-usage` feature, which does not report the key,
/// draws no card. Moved only by a write the daemon confirmed; turning it
/// off clears the daemon's held reading. Every word is the core's
/// (`subscription_usage_*` in the Insights copy table).
struct SubscriptionUsageSettingsCard: View {
    @EnvironmentObject private var model: AppModel

    private static let copy = TCInsights.copy() ?? [:]
    private func text(_ key: String) -> String { SubscriptionUsageWords.text(key, Self.copy) }

    /// The switch's value: the daemon's, or on for the Insights design
    /// sample (`TRACE_COMMONS_INSIGHTS_SAMPLE=1`, debug builds), which
    /// draws the Subscriptions card from a sample.
    private var codex: Bool? {
        #if DEBUG
        if InsightsSampleWeek.enabled { return true }
        #endif
        return model.daemonSettings?.subscriptionUsage?.codex
    }

    var body: some View {
        if let on = codex {
            GlassEyebrowCard(text(SubscriptionUsageWords.Key.sectionTitle)) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    Toggle(text(SubscriptionUsageWords.Key.codexSwitch), isOn: Binding(
                        get: { on },
                        set: { on in Task { await set(on) } }
                    ))
                    .toggleStyle(GlassToggleStyle(.settings))
                    // It takes no second write while one is in flight.
                    .disabled(model.subscriptionUsageBusy)
                    Text(text(SubscriptionUsageWords.Key.codexDetail))
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                    if let refusal = model.subscriptionUsageRefusal {
                        GlassNotice(tone: .outside) { Text(refusal) }
                    }
                }
            }
        }
    }

    /// The sample's switch is drawn on and writes nothing.
    private func set(_ on: Bool) async {
        #if DEBUG
        if InsightsSampleWeek.enabled { return }
        #endif
        await model.setSubscriptionUsageCodex(on)
    }
}
