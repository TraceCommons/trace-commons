import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The Insights switch in Settings (Ron's #1287 review, adopted by the
/// owner): the weekly summary card. Counting tokens in the watched folders
/// is always on (product owner, 2026-10-29), so it has no switch here, and
/// the higher-than-usual card's switch is gone (product owner, 2026-10-29,
/// #1753: it controlled nothing once that card left Overview). Drawn in
/// Settings > Insights (`InsightsSection`), which waits for first run (R-43). The
/// switch reads the value the daemon confirmed and is drawn only once the
/// daemon reported one; every word is the core's analytics copy.
/// Settings > Insights (Settings IA D13): the weekly-summary switch and the
/// subscription-usage permission, out of Tools, where they sat only to wait
/// for first run.
struct InsightsSection: View {
    var body: some View {
        InsightsSettingsCard()
        SubscriptionUsageSettingsSection()
    }
}

struct InsightsSettingsCard: View {
    @EnvironmentObject private var model: AppModel
    private static let copy = TCInsights.copy() ?? [:]
    private func text(_ key: String) -> String { InsightsOverviewWords.text(key, Self.copy) }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // Its own section since the Settings IA review (D13), so before
            // the daemon answers it says so, never an empty section.
            if model.daemonSettings == nil {
                SettingsReadNotice(model.settingsRead, retry: model.refreshSettings)
            }
            // No headed card without a switch in it.
            if model.daemonSettings?.insightsRecapCardEnabled != nil {
                GlassEyebrowCard(text("title")) {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                        if let settings = model.daemonSettings {
                            // Counting is always on (product owner, 2026-10-29),
                            // so it has no switch.
                            if let on = settings.insightsRecapCardEnabled {
                                Toggle(text("analytics_setting_recap_card"), isOn: Binding(
                                    get: { on },
                                    set: { on in Task { await model.setInsightsSwitch(.recapCard, on) } }
                                ))
                                .toggleStyle(GlassToggleStyle(.settings))
                                .disabled(model.insightsSwitchBusy.contains(.recapCard))
                                refusal(model.insightsSwitchRefusal[.recapCard])
                            }
                        }
                    }
                }
            }
        }
        // "Hide" on the summary card writes the daemon directly, so the
        // setting is read again whenever this appears.
        .task {
            model.refreshSettings()
        }
    }

    @ViewBuilder
    private func refusal(_ words: String?) -> some View {
        if let words { GlassNotice(tone: .outside) { Text(words) } }
    }
}
