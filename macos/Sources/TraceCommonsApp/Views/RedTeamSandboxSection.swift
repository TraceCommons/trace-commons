import AppKit
import SwiftUI
import TCDesign
import TCShellCore

/// Input folder, Output folder, and Firewall settings shown for Red Team
/// sandbox launches in `ManagedFolderExplainerSheet` and `ManagedLaunchSheet`.
struct RedTeamSandboxSection: View {
    @EnvironmentObject private var model: AppModel
    @Binding var draft: RedTeamSandboxDraft
    @State private var hostInput = ""

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                GlassSectionRule(model.managedText("red_team_input_folder"))
                if draft.inputs.isEmpty {
                    HStack(spacing: GlassTokens.Space.s3) {
                        GlassToolTile(.folder)
                        Text(model.managedText("red_team_input_empty"))
                            .glassType(GlassTokens.TypeScale.label.weight(.regular))
                            .foregroundStyle(GlassColor.textTertiary)
                        Spacer(minLength: 0)
                        Button(model.managedText("red_team_pick_input")) {
                            pickInputs()
                        }
                        .buttonStyle(GlassButtonStyle(.glass, small: true))
                        .accessibilityLabel(model.managedText("red_team_pick_input_accessibility"))
                    }
                } else {
                    ForEach(draft.inputs, id: \.self) { path in
                        HStack(spacing: GlassTokens.Space.s2) {
                            GlassToolTile(.folder)
                            Text(path)
                                .glassType(GlassTokens.TypeScale.mono)
                                .foregroundStyle(GlassColor.textPrimary)
                                .lineLimit(1)
                                .truncationMode(.middle)
                                .textSelection(.enabled)
                                .help(path)
                            Spacer(minLength: 0)
                            Button {
                                draft.inputs.removeAll { $0 == path }
                            } label: {
                                Image(systemName: "xmark")
                            }
                            .buttonStyle(GlassButtonStyle(.glass, small: true))
                            .accessibilityLabel(model.managedText("red_team_remove_file_accessibility"))
                        }
                    }
                    Button(model.managedText("red_team_pick_input")) {
                        pickInputs()
                    }
                    .buttonStyle(GlassButtonStyle(.glass, small: true))
                    .accessibilityLabel(model.managedText("red_team_pick_input_accessibility"))
                }
            }

            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                GlassSectionRule(model.managedText("red_team_output_folder"))
                HStack(spacing: GlassTokens.Space.s3) {
                    GlassToolTile(.folder)
                    if let outputDir = draft.outputDir {
                        Text(outputDir)
                            .glassType(GlassTokens.TypeScale.mono)
                            .foregroundStyle(GlassColor.textPrimary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .textSelection(.enabled)
                            .help(outputDir)
                    } else {
                        Text(model.managedText("red_team_output_default"))
                            .glassType(GlassTokens.TypeScale.label.weight(.regular))
                            .foregroundStyle(GlassColor.textTertiary)
                            .lineLimit(1)
                    }
                    Spacer(minLength: GlassTokens.Space.s3)
                    if draft.outputDir != nil {
                        Button(model.managedText("red_team_clear_output")) {
                            draft.outputDir = nil
                        }
                        .buttonStyle(GlassButtonStyle(.glass, small: true))
                        .fixedSize()
                        .accessibilityLabel(model.managedText("red_team_clear_output_accessibility"))
                    }
                    Button(model.managedText("red_team_pick_output")) {
                        if let chosen = FolderPanel.choose() {
                            draft.outputDir = chosen
                        }
                    }
                    .buttonStyle(GlassButtonStyle(.glass, small: true))
                    .fixedSize()
                    .accessibilityLabel(model.managedText("red_team_pick_output_accessibility"))
                }
            }

            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                GlassSectionRule(model.managedText("red_team_firewall"))
                Toggle(model.managedText("red_team_network_allow"), isOn: $draft.networkEnabled)
                    .toggleStyle(GlassToggleStyle(.settings))
                Text(model.managedText(draft.networkEnabled ? "red_team_firewall_allowed" : "red_team_firewall_blocked"))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                if draft.networkEnabled {
                    HStack(alignment: .bottom, spacing: GlassTokens.Space.s2) {
                        GlassTextField(
                            model.managedText("red_team_network"),
                            text: $hostInput,
                            prompt: model.managedText("red_team_host_placeholder")
                        )
                        Button(model.managedText("red_team_add_host")) {
                            addHost()
                        }
                        .buttonStyle(GlassButtonStyle(.glass, small: true))
                        .disabled(hostInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                        .accessibilityLabel(model.managedText("red_team_add_host_accessibility"))
                    }
                    ForEach(draft.allowHosts, id: \.self) { host in
                        HStack(spacing: GlassTokens.Space.s2) {
                            Text(host)
                                .glassType(GlassTokens.TypeScale.mono)
                                .foregroundStyle(GlassColor.textPrimary)
                                .lineLimit(1)
                            Spacer(minLength: 0)
                            Button {
                                draft.allowHosts.removeAll { $0 == host }
                            } label: {
                                Image(systemName: "xmark")
                            }
                            .buttonStyle(GlassButtonStyle(.glass, small: true))
                            .accessibilityLabel(model.managedText("red_team_remove_host_accessibility"))
                        }
                    }
                    Toggle(model.managedText("red_team_packages"), isOn: $draft.packages)
                        .toggleStyle(GlassToggleStyle(.settings))
                }
            }
        }
    }

    private func pickInputs() {
        for path in FolderPanel.chooseFilesAndFolders() {
            if !draft.inputs.contains(path) {
                draft.inputs.append(path)
            }
        }
    }

    private func addHost() {
        let trimmed = hostInput.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        if !draft.allowHosts.contains(trimmed) {
            draft.allowHosts.append(trimmed)
        }
        hostInput = ""
    }
}
