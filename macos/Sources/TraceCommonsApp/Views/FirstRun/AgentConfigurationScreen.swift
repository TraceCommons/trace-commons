import AppKit
import SwiftUI
import TCDesign
import TCShellCore

/// Guidance only: opening or copying the prompt does not change setup.
struct AgentConfigurationScreen: View {
    let copy: FirstRunCopy.AgentConfiguration
    let backLabel: String
    let onBack: () -> Void
    @State private var copied = false

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s7) {
            Text(copy.title)
                .glassType(GlassTokens.TypeScale.title)
                .accessibilityAddTraits(.isHeader)
            Text(copy.body)
                .glassType(GlassTokens.TypeScale.body)
            Text(copy.prerequisite)
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textSecondary)
            ScrollView {
                Text(copy.prompt)
                    .glassType(GlassTokens.TypeScale.body)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding()
            }
            .background(GlassColor.textSecondary.opacity(0.06), in: RoundedRectangle(cornerRadius: 8))
            HStack {
                Button(backLabel, action: onBack)
                    .keyboardShortcut(.cancelAction)
                    .buttonStyle(GlassButtonStyle(.secondary))
                Spacer()
                Button(copied ? copy.copied : copy.copy) {
                    NSPasteboard.general.clearContents()
                    copied = NSPasteboard.general.setString(copy.prompt, forType: .string)
                }
                .buttonStyle(GlassButtonStyle(.primary))
            }
        }
        .padding(.horizontal, FirstRunFrameLayout.barInset)
        .padding(.vertical, 32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
