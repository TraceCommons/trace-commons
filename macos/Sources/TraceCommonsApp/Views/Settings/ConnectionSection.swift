import SwiftUI
import TCBridge
import TCDesign

struct ConnectionSection: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            connection
            ContributionAccountCard()
        }
    }

    private var connection: some View {
        GlassEyebrowCard(SettingsWords.connection, title: connectionTitle) {
            if model.statusRead == .answered {
                GlassChip(glass: model.status.loggedIn ? SettingsLegacyWords.connectionReady
                                                       : SettingsLegacyWords.connectionLocalOnly,
                          muted: !model.status.loggedIn)
            }
        } content: {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                if model.statusRead != .answered {
                    SettingsReadNotice(model.statusRead, retry: model.refreshStatus)
                } else if !model.status.loggedIn {
                    Text(SettingsLegacyWords.queuedNothingSent)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textTertiary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if model.daemonSettings == nil {
                    // One indicator for the card while neither has answered.
                    if model.statusRead == .answered {
                        SettingsReadNotice(model.settingsRead, retry: model.refreshSettings)
                    }
                } else if let settings = model.daemonSettings {
                    sourceLine(TCSourceChecks.claude, settings.routingSourceModes.claude)
                    sourceLine(TCSourceChecks.codex, settings.routingSourceModes.codex)
                    sourceLine(TCSourceChecks.gemini, settings.routingSourceModes.gemini)
                    sourceLine(TCSourceChecks.cline, settings.routingSourceModes.cline)
                    // #1146 lists OpenCode too; its sentence is the core's.
                    sourceLine(TCSourceChecks.opencode, settings.opencodeSourceMode ?? "unset")
                    SettingsStateRow(title: SettingsLegacyWords.extraScanConfigured,
                                     isOn: settings.nearAIConfigured)
                }
            }
        }
    }

    /// #1146's head: CONNECTION over Connected or Not connected, with the
    /// Ready or Local only chip. Before the core's first answer there is no
    /// title and no chip: "Not connected" would be an answer nothing gave.
    private var connectionTitle: String? {
        guard model.statusRead == .answered else { return nil }
        return model.status.loggedIn ? SettingsLegacyWords.connected : SettingsLegacyWords.notConnected
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

/// Account contribution readiness and invite redemption. Every word is the
/// core's (`account_contribution` in the contributor crate, carried in the
/// Private AI copy); the card draws nothing until that copy has arrived.
struct ContributionAccountCard: View {
    @EnvironmentObject private var model: AppModel
    @State private var inviteCode = ""

    var body: some View {
        if let copy = model.privateInferenceCopy,
           let heading = copy.accountContributionHeading,
           let refresh = copy.accountContributionRefreshAction,
           let inviteLabel = copy.accountContributionInviteCode,
           let redeem = copy.accountContributionRedeemAction {
            GlassEyebrowCard(heading) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    Text(model.contributionLine)
                        .glassType(GlassTokens.TypeScale.caption)
                        .accessibilityAddTraits(.updatesFrequently)
                    Button(refresh) { Task { await model.updateContributionAccount() } }
                        .buttonStyle(GlassButtonStyle(.glass))
                    GlassTextField(inviteLabel, text: $inviteCode, secure: true)
                        .accessibilityLabel(inviteLabel)
                    Button(redeem) {
                        Task {
                            if await model.updateContributionAccount(inviteCode: inviteCode) {
                                inviteCode = ""
                            }
                        }
                    }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .disabled(inviteCode.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    if let pending = copy.accountContributionPendingCredit {
                        Text(pending)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                    }
                }
            }
            .disabled(model.contributionBusy)
        }
    }
}
