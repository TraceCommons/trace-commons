import SwiftUI
import TCDesign
import TCShellCore

/// When the Private AI tab offers Sign in (owner, 2026-10-24: "I would have
/// expected to see a signin button here that would take me to browser").
/// Pure, so the offer is held by tests rather than by a screen.
enum NearAiSignInOffer {
    /// The balance card: the balance answered that no sign-in is kept or that
    /// NEAR AI did not accept it, or the daemon's renewal found NEAR AI
    /// refusing the stored login. NEAR AI being unreachable is not a
    /// sign-in problem. Keep retry available after an unfinished sign-in.
    static func balanceOffers(state: String, renewalFailure: String?, failed: Bool = false) -> Bool {
        failed || state == "session_expired" || state == "no_session"
            || renewalFailure == "near_ai_sign_in_required"
    }

    /// The billing card: the funding read answered that there is no usable
    /// sign-in, or a previous attempt needs a retry.
    static func billingOffers(state: String?, failed: Bool = false) -> Bool {
        failed || state == "no_session" || state == "session_expired"
    }

    /// The button's label: the payout section's "Sign in again" (the same
    /// core constant, owner 2026-10-24), else the credential card's "Sign in".
    static func label(_ copy: PrivateInferenceCopy) -> String {
        if let line = HomeFormat.creditSignInLine, !line.isEmpty { return line }
        return copy.credentialObtain
    }

    /// The core's waiting line while the browser sign-in runs, the core's
    /// failure line after one that did not finish, else nil.
    static func progressLine(_ copy: PrivateInferenceCopy, signingIn: Bool, failed: Bool) -> String? {
        if signingIn { return copy.credentialObtaining }
        if failed { return copy.credentialFailed }
        return nil
    }
}

/// How a NEAR AI sign-in is offered (owner, 2026-11-01: "We are going back
/// to supporting all the near.ai cloud sign in mechanisms. Github, google,
/// or near wallet."): a menu with one item per login the core offers, in
/// the core's order and with the core's titles
/// (`CredentialSection.providerOptions`), or a plain button when the build
/// offers one (staging's Google login). A new login is a change to
/// `channel::NEAR_AI_SIGN_IN_PROVIDERS` and its title, not to this.
///
/// A menu starts the sign-in on the item chosen, so the wallet's notice
/// (`credentialWalletNotice`), which Settings draws beside its picker
/// before the button, has no place to be read first here.
enum NearAiSignInChoice: Equatable {
    /// One login, or none known (nil: the daemon's default).
    case button(provider: String?)
    /// Several logins, in the core's order.
    case menu([CredentialSection.ProviderOption])

    static func of(_ options: [CredentialSection.ProviderOption]) -> NearAiSignInChoice {
        options.count > 1 ? .menu(options) : .button(provider: options.first?.tag)
    }

    /// The logins this build offers, titled with `copy`; with no copy, the
    /// daemon's default alone.
    static func of(_ copy: PrivateInferenceCopy?) -> NearAiSignInChoice {
        of(copy.map { CredentialSection.providerOptions($0) } ?? [])
    }
}

/// A sign-in control for `NearAiSignInChoice`: the caller's label and
/// style either way, a native menu of the logins when there are several.
/// `start` gets the chosen login's tag.
struct NearAiSignInButton: View {
    let title: String
    /// VoiceOver's name for the control, when the label alone does not say
    /// what is signed in to.
    var accessibilityName: String?
    let choice: NearAiSignInChoice
    let style: TCDesign.GlassButtonStyle
    let start: (String?) -> Void

    var body: some View {
        switch choice {
        case .button(let provider):
            Button(title) { start(provider) }
                .buttonStyle(style)
                .accessibilityLabel(accessibilityName ?? title)
        case .menu(let options):
            Menu {
                ForEach(options, id: \.tag) { option in
                    Button(option.title) { start(option.tag) }
                }
            } label: {
                Text(title)
            }
            .menuStyle(.button)
            .buttonStyle(style)
            .accessibilityLabel(accessibilityName ?? title)
        }
    }
}

/// The balance card's sign-in: a primary button that runs the one flow
/// Settings' "Sign in again" runs (`AppModel.signInToNearAI`), with the
/// core's waiting and failure lines. Draws nothing while the NEAR AI login
/// is usable.
struct NearAiSignInControl: View {
    @EnvironmentObject private var model: AppModel
    let copy: PrivateInferenceCopy

    var body: some View {
        let offered = NearAiSignInOffer.balanceOffers(
            state: model.balanceStatus.state, renewalFailure: model.accountRenewalFailure,
            failed: model.nearAiSignInFailed)
        if offered || model.nearAiSigningIn {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                NearAiSignInButton(
                    title: NearAiSignInOffer.label(copy), choice: NearAiSignInChoice.of(copy),
                    style: GlassButtonStyle(.primary)
                ) { provider in
                    Task { await model.signInToNearAI(provider: provider) }
                }
                .disabled(model.credentialBusy || (model.nearAiSigningIn && !model.canRestartNearAiBrowserSignIn))
                if let line = NearAiSignInOffer.progressLine(
                    copy, signingIn: model.nearAiSigningIn, failed: model.nearAiSignInFailed)
                {
                    Text(line)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .task { if model.accountSession == .unread { model.refreshAccountSession() } }
        }
    }
}
