import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// Permission lives in Settings, alongside the information needed to grant
/// it. Opening this section reads daemon metadata only.
struct SubscriptionUsageSettingsSection: View {
    @EnvironmentObject private var app: AppModel
    private static let copy = TCInsights.copy() ?? [:]

    var body: some View {
        Group {
            if let usage = app.subscriptionUsageModel {
                SubscriptionUsageSettingsCard(model: usage, copy: Self.copy)
            }
        }
        .task(id: MonitorWindowView.Attachment(app)) {
            // Overview owns the shared observer. Settings only reads the
            // saved permission; opening it never starts a provider probe.
            await app.subscriptionUsageModel?.load()
        }
    }
}

struct SubscriptionUsageSettingsCard: View {
    let model: SubscriptionUsageModel
    let copy: [String: String]
    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        if model.canChange, let enabled = model.codexEnabled,
           !text("analytics_subscriptions_setting_codex").isEmpty {
            GlassEyebrowCard(text("analytics_subscriptions_title")) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    Text(text("analytics_subscriptions_codex_detail"))
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                    Toggle(text("analytics_subscriptions_setting_codex"), isOn: Binding(
                        get: { enabled },
                        set: { on in Task { await model.setEnabled("codex", on: on) } }
                    ))
                    .toggleStyle(GlassToggleStyle(.settings))
                    .disabled(!model.changingProviders.isEmpty)
                    if let failure = model.failureKey {
                        Text(text(failure)).insightsError()
                    }
                }
            }
        }
    }
}
