import AppKit
import SwiftUI
import TCDesign
import TCShellCore

/// The Private AI tab's Claude Desktop card (`ClaudeDesktopPrompt.mode`).
/// Offer: a dismissable notice while Claude Desktop is installed on this Mac
/// and not set up; Set up opens Settings' Private AI section, where the
/// Claude Desktop row and its setup sheet are, and Not now hides the offer
/// for good. Connected: while it is set up, its status with Disconnect (no
/// confirmation, like the Settings row) and Set up to change models; the
/// offer's dismissal and an absent Claude.app do not hide it.
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
            if let setup, let status = setup.status {
                switch ClaudeDesktopPrompt.mode(featureOn: featureOn, desktopInstalled: desktopInstalled,
                                                dismissed: dismissed, status: status) {
                case .offer: offer(status)
                case .connected: connected(setup, status)
                case .hidden: EmptyView()
                }
            }
        }
        // Reads nothing while the flag is off; otherwise re-reads the status
        // while the tab is in view, so the card follows setup and disconnect
        // done in Settings.
        .task(id: featureOn) {
            guard featureOn else { setup = nil; return }
            desktopInstalled = NSWorkspace.shared.urlForApplication(
                withBundleIdentifier: ClaudeDesktopPrompt.desktopBundleID) != nil
            while !Task.isCancelled {
                if setup == nil { setup = model.makeClaudeDesktopSetup() }
                await setup?.refresh()
                do { try await Task.sleep(for: .seconds(15)) } catch { break }
            }
        }
        .onDisappear { setup = nil }
    }

    private func offer(_ status: ClaudeDesktopStatus) -> some View {
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

    private func connected(_ setup: ClaudeDesktopSetupModel, _ status: ClaudeDesktopStatus) -> some View {
        GlassCard(quiet: true) {
            ViewThatFits(in: .horizontal) {
                HStack(alignment: .top, spacing: 18) {
                    details(setup, status)
                    Spacer(minLength: 0)
                    actions(setup, status)
                }
                VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                    details(setup, status)
                    actions(setup, status)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func details(_ setup: ClaudeDesktopSetupModel, _ status: ClaudeDesktopStatus) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            Text(status.text("title"))
                .glassType(GlassTokens.TypeScale.bodyStrong)
                .foregroundStyle(GlassColor.textPrimary)
            Text(status.statusLine)
                .glassType(GlassTokens.TypeScale.label.weight(.regular))
                .foregroundStyle(GlassColor.textSecondary)
            if let id = status.model {
                Text(ManagedSurface.rowModel(id, names: setup.modelNames))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .help(id)
            }
            if setup.failed {
                Text(status.text("error"))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
            }
        }
        .fixedSize(horizontal: false, vertical: true)
    }

    private func actions(_ setup: ClaudeDesktopSetupModel, _ status: ClaudeDesktopStatus) -> some View {
        HStack(spacing: GlassTokens.Space.s3) {
            Button(status.text("setup"), action: onOpenSettings)
                .buttonStyle(GlassButtonStyle(.glass))
                .accessibilityLabel(status.text("setup") + " " + status.text("title"))
            Button(status.text("disconnect")) { Task { await setup.disconnect() } }
                .buttonStyle(GlassButtonStyle(.destructive))
                .disabled(setup.busy)
        }
        .fixedSize()
    }
}
