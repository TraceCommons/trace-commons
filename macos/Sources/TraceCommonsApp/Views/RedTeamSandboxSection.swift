import AppKit
import SwiftUI
import TCDesign
import TCShellCore

/// Files and Network settings shown inside `ManagedLaunchSheet` only for Red Team.
struct RedTeamSandboxSection: View {
    @EnvironmentObject private var model: AppModel
    @Binding var draft: RedTeamSandboxDraft
    @State private var hostInput = ""

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            GlassSectionRule(model.managedText("red_team_files"))
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                if draft.inputs.isEmpty {
                    Text(model.managedText("red_team_files_empty"))
                        .glassType(GlassTokens.TypeScale.label.weight(.regular))
                        .foregroundStyle(GlassColor.textTertiary)
                } else {
                    ForEach(draft.inputs, id: \.self) { path in
                        HStack(spacing: GlassTokens.Space.s2) {
                            Text(path)
                                .glassType(GlassTokens.TypeScale.mono)
                                .foregroundStyle(GlassColor.textPrimary)
                                .lineLimit(1)
                                .truncationMode(.middle)
                                .textSelection(.enabled)
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
                }
                Button(model.managedText("red_team_add_files")) {
                    pickInputs()
                }
                .buttonStyle(GlassButtonStyle(.glass, small: true))
                .accessibilityLabel(model.managedText("red_team_add_files_accessibility"))
            }

            GlassSectionRule(model.managedText("red_team_network"))
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                Toggle(model.managedText("red_team_network_allow"), isOn: $draft.networkEnabled)
                    .toggleStyle(.switch)
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
                        .toggleStyle(.switch)
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
