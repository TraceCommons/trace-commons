import SwiftUI
import TCDesign
import TCShellCore

/// Settings > General: the off switch for the anonymous weekly usage counts
/// (`share_usage_data`, #1920, approved by the owner 2026-11-16: on by
/// default, counted only from the US, which the daemon decides). Where the
/// spec puts it (`docs/superpowers/specs/2026-11-04-us-only-usage-analytics.md`,
/// follow-up 1). The switch reads the value the daemon confirmed and is
/// drawn only once the daemon reported one; a write the daemon refused says
/// the core's request-failed words under it.
struct UsageCountsSettingsCard: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // General read nothing from the daemon before this card; before
            // the daemon answers it says so, never an empty card.
            if model.daemonSettings == nil {
                SettingsReadNotice(model.settingsRead, retry: model.refreshSettings)
            }
            // No card without a switch in it (an older daemon).
            if let on = model.daemonSettings?.shareUsageData {
                GlassEyebrowCard(UsageCountsWords.title) {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                        Toggle(UsageCountsWords.toggle, isOn: Binding(
                            get: { on },
                            set: { on in Task { await model.setShareUsageData(on) } }
                        ))
                        .toggleStyle(GlassToggleStyle(.settings))
                        .disabled(model.shareUsageDataBusy)
                        if let refusal = model.shareUsageDataRefusal {
                            GlassNotice(tone: .outside) { Text(refusal) }
                        }
                    }
                }
            }
        }
    }
}

/// The switch's words.
// COPY: pending owner approval. Drafts written 2026-11-16 for #1920; none
// of these strings has been approved, and they belong in the core's copy
// tables once they are (the spec's follow-up 1).
enum UsageCountsWords {
    static let title = "Usage data"
    static let toggle = "Share anonymous usage data"
}
