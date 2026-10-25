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
                Button(NearAiSignInOffer.label(copy)) {
                    Task { await model.signInToNearAI() }
                }
                .buttonStyle(GlassButtonStyle(.primary))
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
