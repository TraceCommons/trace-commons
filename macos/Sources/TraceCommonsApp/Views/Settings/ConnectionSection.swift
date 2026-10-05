import SwiftUI
import TCBridge
import TCDesign

struct ConnectionSection: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        GlassEyebrowCard(SettingsWords.connection) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                // Before the first answer the status is a placeholder, and
                // "Not connected" would be an answer nothing gave.
                if !model.status.answered {
                    SettingsAwaiting()
                } else if model.status.loggedIn {
                    GlassStatusLabel(SettingsLegacyWords.connected, status: .on)
                } else {
                    GlassStatusLabel(SettingsLegacyWords.notConnected, status: .ask)
                    Text(SettingsLegacyWords.queuedNothingSent)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                }
                if model.daemonSettings == nil {
                    // One indicator for the card while neither has answered.
                    if model.status.answered { SettingsAwaiting() }
                } else if let settings = model.daemonSettings {
                    sourceLine(TCSourceChecks.claude, settings.routingSourceModes.claude)
                    sourceLine(TCSourceChecks.codex, settings.routingSourceModes.codex)
                    sourceLine(TCSourceChecks.gemini, settings.routingSourceModes.gemini)
                    sourceLine(TCSourceChecks.cline, settings.routingSourceModes.cline)
                    SettingsStateRow(title: SettingsLegacyWords.extraScanConfigured,
                                     isOn: settings.nearAIConfigured)
                }
            }
        }
    }

    /// The core's sentence for one source's MODE (it words watch and off
    /// differently, so the line carries the state); nothing when the ABI
    /// refused, as the legacy row does.
    @ViewBuilder
    private func sourceLine(_ tool: String, _ mode: String) -> some View {
        if let line = TCSourceChecks.checkLine(tool: tool, sourceMode: mode) {
            GlassStatusLabel(
                line, status: Self.sourceStatus(tool: tool, mode: mode, copy: TCSourceChecks.settingsCopy()))
        }
    }

    /// The dot for one source's mode. Only a declined source is off. An
    /// unset source is on when the core says it is read from the usual
    /// place (`unset_scans_conventional`, Claude Code and Codex), and is
    /// otherwise not decided. With no core copy, or a mode the shell does
    /// not know, nothing says whether it is read, so it is neither on nor
    /// off.
    static func sourceStatus(tool: String, mode: String, copy: SourceSettingsCopy?) -> GlassStatus {
        switch mode {
        case "watch": return .on
        case "off": return .off
        case "unset":
            let scans = copy?.tools.values.first { $0.key == tool }?.unsetScansConventional
            return scans == true ? .on : .ask
        default: return .ask
        }
    }
}
