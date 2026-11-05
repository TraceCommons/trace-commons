import SwiftUI
import TCDesign
import TCShellCore

/// Home's reminder while Train coding models is off (product owner,
/// 2026-11-05): one slim amber line under the Traces hero, never red,
/// saying no credit is earned until it is on, with Open Settings at its
/// end. It has no X: it goes away by itself once the use is on. First run
/// draws none; the line under Train coding models says it there.
struct NoCreditNotice: View {
    let line: String
    let action: String
    let perform: () -> Void

    var body: some View {
        HStack(spacing: GlassTokens.Space.s3) {
            Image(systemName: "exclamationmark.circle.fill")
                .glassGlyph(14, weight: .semibold)
                .foregroundStyle(GlassStatus.ask.color)
                .accessibilityHidden(true)
            Text(line)
                .glassType(GlassTokens.TypeScale.label)
                .foregroundStyle(GlassColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: GlassTokens.Space.s3)
            Button(action, action: perform)
                .buttonStyle(GlassButtonStyle(.link))
        }
        .padding(.horizontal, GlassTokens.Space.s5)
        .padding(.vertical, GlassTokens.Space.s3)
        .background(
            RoundedRectangle(cornerRadius: GlassTokens.Radius.control, style: .continuous)
                .fill(GlassStatus.ask.color.opacity(0.12)))
        .accessibilityElement(children: .contain)
    }

    /// Whether the account's data uses leave out the one that earns credit.
    /// Only for a signed-in account: before sign-in nothing has been
    /// granted, and the reminder would be about no one.
    static func showsOnHome(loggedIn: Bool, scopes: [String]) -> Bool {
        loggedIn && !scopes.contains(SentTracesTrainingState.scope)
    }
}
