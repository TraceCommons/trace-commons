import SwiftUI
import TCBridge
import TCDesign

struct ConnectionSection: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        GlassEyebrowCard(SettingsWords.connection) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                if model.status.loggedIn {
                    GlassStatusLabel(SettingsLegacyWords.connected, status: .on)
                } else {
                    GlassStatusLabel(SettingsLegacyWords.notConnected, status: .ask)
                    Text(SettingsLegacyWords.queuedNothingSent)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                }
                if let settings = model.daemonSettings {
                    sourceLine(TCSourceChecks.claude, settings.routingSourceModes.claude)
                    sourceLine(TCSourceChecks.codex, settings.routingSourceModes.codex)
                    sourceLine(TCSourceChecks.gemini, settings.routingSourceModes.gemini)
                    sourceLine(TCSourceChecks.cline, settings.routingSourceModes.cline)
                    GlassStatusLabel(SettingsLegacyWords.extraScanConfigured,
                                     status: settings.nearAIConfigured ? .on : .off)
                }
            }
        }
    }

    /// The core's sentence for one source's MODE; nothing when the ABI
    /// refused, as the legacy row does.
    @ViewBuilder
    private func sourceLine(_ tool: String, _ mode: String) -> some View {
        if let line = TCSourceChecks.checkLine(tool: tool, sourceMode: mode) {
            GlassStatusLabel(line, status: mode == "watch" ? .on : .off)
                .accessibilityElement(children: .combine)
        }
    }
}
