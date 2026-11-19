import SwiftUI
import TCDesign
import TCShellCore

/// Red Team's row in Settings' Private AI tools panel, under Claude
/// Desktop: whether its runtime is installed, and Install while it is not.
/// The daemon sends no `red_team` row while the flag is off, so the row
/// draws nothing then. Every word is the managed copy's.
struct RedTeamSettingsRow: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        if let setup = model.managedSnapshot?.redTeamSetup {
            ViewThatFits(in: .horizontal) {
                HStack(alignment: .top, spacing: 18) {
                    details(setup)
                    Spacer(minLength: 0)
                    action(setup)
                }
                VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                    details(setup)
                    action(setup)
                }
            }
            .padding(.vertical, 15)
            .frame(maxWidth: .infinity, alignment: .leading)
            .overlay(alignment: .bottom) { GlassHairline(GlassColor.hairline) }
        }
    }

    private var failed: Bool {
        model.managedErrorMethod == "managed_tool_setup" && model.managedErrorTool == "red_team"
    }

    private func details(_ setup: RedTeamSetup) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            Text(model.managedText("red_team"))
                .glassType(GlassTokens.TypeScale.bodyStrong)
                .foregroundStyle(GlassColor.textPrimary)
            Text(model.managedText(setup == .installed ? "red_team_installed" : "red_team_not_installed"))
                .glassType(GlassTokens.TypeScale.label.weight(.regular))
                .foregroundStyle(GlassColor.textSecondary)
            Text(model.managedText("red_team_settings_line"))
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
            if failed {
                GlassAlert(model.managedText("action_failed"))
            }
        }
        .fixedSize(horizontal: false, vertical: true)
    }

    @ViewBuilder
    private func action(_ setup: RedTeamSetup) -> some View {
        if setup == .notInstalled, model.managedSnapshot?.capabilities.toolSetup == true {
            // The daemon installs the pinned runtime inline; the button stays
            // busy until it answers, and the next snapshot says Installed.
            Button(model.managedText("install")) { model.setupManagedTool("red_team", action: "install") }
                .buttonStyle(GlassButtonStyle(.glass))
                .accessibilityLabel(model.managedText("install") + " " + model.managedText("red_team"))
                .disabled(model.managedBusy)
                .fixedSize()
        }
    }
}

/// The Private AI tab's dismissable Red Team setup card
/// (`RedTeamSetupCard`): while the runtime is not installed. Set up opens
/// Settings' Private AI section, where the Red Team row installs it; Not
/// now hides the card for good. Once installed, Start Red Team is on the
/// NEAR AI card beside the other tools.
struct RedTeamSetupPrompt: View {
    /// Opens the Private AI section of Settings.
    let onOpenSettings: () -> Void
    @EnvironmentObject private var model: AppModel
    @AppStorage(RedTeamSetupCard.dismissedKey) private var dismissed = false

    var body: some View {
        if RedTeamSetupCard.offered(setup: model.managedSnapshot?.redTeamSetup, dismissed: dismissed) {
            GlassNotice(tone: .ask, title: model.managedText("red_team_setup_title")) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    Text(model.managedText("red_team_setup_line"))
                        .glassType(GlassTokens.TypeScale.caption)
                        .fixedSize(horizontal: false, vertical: true)
                    HStack(spacing: GlassTokens.Space.s3) {
                        Button(model.managedText("red_team_setup_action"), action: onOpenSettings)
                            .buttonStyle(GlassButtonStyle(.glass))
                            .fixedSize()
                            .accessibilityLabel(model.managedText("red_team_setup_action") + " " + model.managedText("red_team"))
                        Button(model.managedText("red_team_setup_dismiss")) { dismissed = true }
                            .buttonStyle(GlassButtonStyle(.link))
                            .fixedSize()
                    }
                }
            }
        }
    }
}
