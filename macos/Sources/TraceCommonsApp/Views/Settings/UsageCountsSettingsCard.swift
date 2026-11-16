import SwiftUI
import TCDesign
import TCShellCore

/// Settings > General: the off switch for the anonymous weekly usage counts
/// (`share_usage_data`, #1920, approved by the owner 2026-11-16: on by
/// default, counted only from the US, which the daemon decides). Where the
/// spec puts it (`docs/superpowers/specs/2026-11-04-us-only-usage-analytics.md`,
/// follow-up 1). The switch reads the value the daemon confirmed and is
/// drawn only once the daemon reported one; a write the daemon refused says
/// the core's request-failed words under it. Every word is the core's
/// (`MonitorScreensCopy.usageData*`).
// COPY: pending owner approval. The title, label and caption are drafts
// (2026-11-16), marked DRAFT, NEEDS APPROVAL in the core's table
// (`preview_copy.rs`, `usage_data_*`); the owner approved the counts, not
// these words.
struct UsageCountsSettingsCard: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // The only part of General that reads the daemon: before it
            // answers, this card says so (the shared read state, no words of
            // its own), and the rest of General draws as before.
            if model.daemonSettings == nil {
                SettingsReadNotice(model.settingsRead, retry: model.refreshSettings)
            }
            // No card without a switch in it (an older daemon).
            if let on = model.daemonSettings?.shareUsageData, let words = MonitorWords.table {
                GlassEyebrowCard(words.usageDataTitle) {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                        Toggle(words.usageDataToggle, isOn: Binding(
                            get: { on },
                            set: { on in Task { await model.setShareUsageData(on) } }
                        ))
                        .toggleStyle(GlassToggleStyle(.settings))
                        .disabled(model.shareUsageDataBusy)
                        Text(words.usageDataCaption)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                        if let refusal = model.shareUsageDataRefusal {
                            GlassNotice(tone: .outside) { Text(refusal) }
                        }
                    }
                }
            }
        }
    }
}
