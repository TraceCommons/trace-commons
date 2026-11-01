import SwiftUI
import TCDesign
import TCShellCore

/// The daemon owns saved arguments; each field keeps its draft until Save succeeds.
struct ManagedCLIOptionsSection: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        GlassEyebrowCard(model.managedText("cli_options_title")) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                Text(model.managedText("cli_options_scope"))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                if let snapshot = model.managedSnapshot {
                    if let options = snapshot.cliOptions {
                        ForEach(ManagedSurface.supportedTools, id: \.self) { tool in
                            if options[tool] != nil { optionRow(tool: tool, options: options) }
                        }
                    } else {
                        Text(model.managedText("cli_options_unavailable"))
                            .glassType(GlassTokens.TypeScale.body)
                    }
                } else {
                    SettingsReadNotice(model.managedCLIOptionsRead, retry: model.refreshManagedSessions)
                }
            }
        }
    }

    private func optionRow(tool: String, options: [String: String]) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            Text(model.managedText(tool))
                .glassType(GlassTokens.TypeScale.body)
            GlassTextField(model.managedText("cli_options_label"), text: Binding(
                get: { model.managedCLIOptionsDrafts.value(tool: tool, saved: model.managedSnapshot?.cliOptions) },
                set: { model.managedCLIOptionsDrafts.edit(tool: tool, options: $0) }
            ), showsLabel: false)
            .accessibilityLabel(model.managedText(tool) + " " + model.managedText("cli_options_label"))
            .disabled(model.managedBusy)
            HStack {
                Spacer()
                Button(model.managedText("cli_options_save")) { model.saveManagedCLIOptions(tool: tool) }
                    .buttonStyle(GlassButtonStyle(.secondary))
                    .accessibilityLabel(model.managedText("cli_options_save_accessibility")
                        .replacingOccurrences(of: "{tool}", with: model.managedText(tool)))
                    .disabled(model.managedBusy || !model.managedCLIOptionsDrafts.isChanged(tool: tool, saved: options))
            }
            if model.managedErrorMethod == "managed_cli_options_set", model.managedErrorTool == tool,
               let key = model.managedErrorKey {
                Text(model.managedText(key))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}
