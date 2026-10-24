import SwiftUI
import TCDesign
import TCShellCore

/// The Private AI screen's prompt to choose a spending organization (owner
/// request, 2026-10-23, which is the approval for this notice existing; the
/// wording is draft).
///
/// Drawn only when the daemon says so: two or more active organizations, a
/// key spending from one, and no choice made. Nothing pauses meanwhile --
/// Private AI keeps spending from the organization the sign-in chose. One
/// organization, or a listing that failed, draws nothing. The action opens
/// Settings, so it says so ("Review settings", ui-copy-guide rule 2). "Not
/// now" keeps the current organization and hides the prompt (owner request,
/// 2026-10-24); the wording is draft.
struct SpendOrganizationPrompt: View {
    @EnvironmentObject private var model: AppModel
    /// Opens the Private AI section of Settings.
    let onOpenSettings: () -> Void

    var body: some View {
        let status = model.spendOrganizations
        if status.showsPrompt, let words = status.words {
            GlassNotice(tone: .ask, title: words.promptTitle) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    Text(words.promptLine)
                        .glassType(GlassTokens.TypeScale.caption)
                        .fixedSize(horizontal: false, vertical: true)
                    HStack(spacing: GlassTokens.Space.s3) {
                        Button(words.promptAction, action: onOpenSettings)
                            .buttonStyle(GlassButtonStyle(.glass))
                            .fixedSize()
                            .accessibilityLabel(words.promptActionAccessibility)
                        // Keeps the current organization and hides this
                        // prompt; only offered with a current organization
                        // (`showsPrompt` requires one).
                        Button(words.promptDismiss) {
                            Task { await model.dismissSpendOrganizationPrompt() }
                        }
                        .buttonStyle(GlassButtonStyle(.link))
                        .fixedSize()
                        .accessibilityLabel(words.promptDismissAccessibility)
                    }
                }
            }
        }
    }
}
