import SwiftUI
import TCDesign
import TCShellCore

/// The key this destination answers with.
///
/// **This file authors no wording at all**, and must never start: every
/// sentence is a field of `PrivateInferenceCopy` or comes back from a shared
/// table, and the three decisions on this card -- which sentence, which
/// tone, which button -- are all `CredentialSurface`'s. It holds no entry in
/// `ShellWordingTests`'s baseline and must not be given one.
///
/// Nothing here renders a key, a prefix, an id or an account name. The
/// payload has no hole for one and none is to be added: a key printed on a
/// screen is a key in a screenshot.
struct CredentialSection: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.openURL) private var openURL
    let copy: PrivateInferenceCopy
    var requiresSession = false
    var prominent = false
    /// Whether the section draws its own title. Inside #1146's connection
    /// panel it does not: the panel's title is the heading.
    var titled = true
    /// Where it is drawn (Settings IA, owner 2026-11-04). The account
    /// section holds the NEAR AI login whole; Private AI keeps only the way
    /// in (D8), and draws nothing while signed in.
    var placement: Placement = .account

    enum Placement: Equatable {
        /// The NEAR AI login in full, but never Forget: Sign out replaces it
        /// (D3, spec Q1 answered no).
        case account
        /// Only Sign in, or Cancel while one runs; nothing otherwise (D8).
        case signInShortcut
    }

    /// The buttons a placement may draw: Forget nowhere, and the shortcut
    /// only the way in.
    static func draws(_ action: CredentialAction, in placement: Placement) -> Bool {
        switch (action, placement) {
        case (.forget, _), (.none, _): false
        case (.obtain, _), (.cancel, _): true
        case (.migrate, .account): true
        case (.migrate, .signInShortcut): false
        }
    }

    var body: some View {
        let status = model.credentialStatus
        let tone = CredentialSurface.tone(status, calls: model.credentialCalls)
        let action = CredentialSurface.action(
            requiresSession ? CredentialStatus(state: status.sessionState) : status,
            calls: model.credentialCalls)
        let balanceAction = requiresSession ? CredentialAction.none : BalanceSurface.actionToDraw(
            balance: BalanceSurface.action(model.balanceStatus, calls: model.balanceCalls),
            credential: action)
        if Self.shows(action: action, balanceAction: balanceAction, in: placement) {
            card(status: status, tone: tone, action: action, balanceAction: balanceAction)
        }
    }

    /// Whether the card is drawn. The account section always draws it.
    /// Private AI's shortcut steps aside only while a NEAR AI login is held
    /// (Forget or the keychain move would be the card's button), unless the
    /// balance still asks for a sign-in: before a login, and while the state
    /// is unread, it says what signing in is for (D8).
    static func shows(action: CredentialAction, balanceAction: CredentialAction, in placement: Placement) -> Bool {
        switch placement {
        case .account: true
        case .signInShortcut:
            draws(balanceAction, in: placement) || (action != .forget && action != .migrate)
        }
    }

    @ViewBuilder
    private func card(
        status: CredentialStatus, tone: PrivateInferenceTone, action: CredentialAction, balanceAction: CredentialAction
    ) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            if !titled {
                // Drawn inside the connection card, which is its heading.
                EmptyView()
            } else if prominent {
                Text(copy.credentialTitle)
                    .glassType(GlassTokens.TypeScale.heading)
                    .foregroundStyle(GlassColor.textPrimary)
            } else {
                GlassSectionRule(copy.credentialTitle)
            }
            Text(copy.credentialWhat)
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            // What is true, painted from the shared tone -- never from
            // whether this shell happens to be holding an attempt. The dot
            // means nothing alone; the core's sentence beside it does.
            GlassStatusLabel(
                CredentialSurface.stateLine(status, copy: copy, calls: model.credentialCalls),
                status: PrivateInferenceIndicator.status(tone))
            .fixedSize(horizontal: false, vertical: true)
            // The sentence that must accompany the button comes from the
            // action, not from a branch here, and it is drawn ABOVE the
            // button: what pressing it costs is not a footnote to having
            // pressed it.
            if Self.draws(action, in: placement), let explains = CredentialSurface.actionExplains(action, copy: copy) {
                Text(explains)
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            actionButton(action)
            // The balance and funding are their own panels after this card,
            // as #1146 draws them (`PrivateAIBalanceCard`, `FundingRow`). The
            // sign-in the balance needs stays here, and never doubles this
            // card's own button: the decision is `BalanceSurface.actionToDraw`'s,
            // not this view's.
            actionButton(balanceAction)
        }
    }

    /// One button, or none at all.
    ///
    /// `.none` draws nothing rather than a disabled control: there is
    /// nothing to enable, and a greyed-out sign-in beside a state nobody
    /// could read still tells a contributor a sign-in is the thing to do.
    ///
    /// Cancel is drawn live even when this shell holds no attempt id. The
    /// daemon accepts an unnamed cancel and stops the sign-in it is holding,
    /// so an app restarted while the daemon kept running can still stop it.
    /// Only a write already in flight greys the button.
    @ViewBuilder
    private func actionButton(_ action: CredentialAction) -> some View {
        if Self.draws(action, in: placement), let label = CredentialSurface.actionLabel(action, copy: copy) {
            Button(label) { run(action) }
                .buttonStyle(GlassButtonStyle(prominent && action == .obtain ? .primary : .glass))
                .disabled(model.credentialBusy)
        }
    }

    private func run(_ action: CredentialAction) {
        switch action {
        case .obtain:
            Task {
                // The URL is served once, by start. Opening it is this
                // view's environment; the model has no window.
                // No provider; the daemon's browser page lists the build's
                // logins, and the wallet's notice with them (owner, 2026-11-02).
                guard let url = await model.startNearAiCredential() else { return }
                let accepted = await openURL.opened(url)
                // A browser that never opened leaves a ceremony nobody can
                // finish, so it is stopped rather than left to time out.
                if !accepted { model.cancelNearAiCredential() }
            }
        case .cancel:
            model.cancelNearAiCredential()
        case .forget:
            model.forgetNearAiCredential()
        case .migrate:
            model.migrateNearAiCredential()
        case .none:
            break
        }
    }
}
