import AppKit
import SwiftUI
import TCDesign
import TCShellCore

/// The Private AI tab's notice offering Claude Desktop setup
/// (`ClaudeDesktopPrompt`): only with the flag on, Claude Desktop installed
/// on this Mac, the daemon saying it is not set up, and the notice not
/// dismissed. Set up opens Settings' Private AI section, where the Claude
/// Desktop row and its setup sheet are; Not now hides it for good.
struct ClaudeDesktopSetupPrompt: View {
    /// Opens the Private AI section of Settings.
    let onOpenSettings: () -> Void
    @EnvironmentObject private var model: AppModel
    @AppStorage(ClaudeDesktopPrompt.dismissedKey) private var dismissed = false
    @State private var setup: ClaudeDesktopSetupModel?
    @State private var desktopInstalled = false
    private var featureOn: Bool { model.featureIsEnabled(ClaudeDesktopSetupModel.featureID) }

    var body: some View {
        Group {
            if ClaudeDesktopPrompt.shows(featureOn: featureOn, desktopInstalled: desktopInstalled,
                                         dismissed: dismissed, status: setup?.status),
               let status = setup?.status {
                GlassNotice(tone: .ask, title: status.text("prompt_title")) {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                        Text(status.text("prompt_line"))
                            .glassType(GlassTokens.TypeScale.caption)
                            .fixedSize(horizontal: false, vertical: true)
                        HStack(spacing: GlassTokens.Space.s3) {
                            Button(status.text("setup"), action: onOpenSettings)
                                .buttonStyle(GlassButtonStyle(.glass))
                                .fixedSize()
                                .accessibilityLabel(status.text("setup") + " " + status.text("title"))
                            Button(status.text("prompt_dismiss")) { dismissed = true }
                                .buttonStyle(GlassButtonStyle(.link))
                                .fixedSize()
                        }
                    }
                }
            }
        }
        // Reads nothing while the flag is off or the notice is dismissed;
        // otherwise re-reads the status while the tab is in view, so the
        // notice goes once setup is saved in Settings.
        .task(id: featureOn && !dismissed) {
            guard featureOn, !dismissed else { setup = nil; return }
            desktopInstalled = NSWorkspace.shared.urlForApplication(
                withBundleIdentifier: ClaudeDesktopPrompt.desktopBundleID) != nil
            guard desktopInstalled else { return }
            while !Task.isCancelled {
                if setup == nil { setup = model.makeClaudeDesktopSetup() }
                await setup?.refresh()
                do { try await Task.sleep(for: .seconds(15)) } catch { break }
            }
        }
        .onDisappear { setup = nil }
    }
}
