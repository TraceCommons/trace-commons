import SwiftUI
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
    @State private var provider = "github"

    /// How often a ceremony in flight is re-read.
    ///
    /// It finishes in a browser this app does not own, so nothing here is
    /// told when it does; the poll is what closes that. It runs only while
    /// the shared table still offers a cancel, which is the one state that
    /// has an outcome pending.
    private static let pollInterval = Duration.seconds(2)

    var body: some View {
        let status = model.credentialStatus
        let tone = PrivateInferenceIndicator.palette(
            CredentialSurface.tone(status, calls: model.credentialCalls))
        let action = CredentialSurface.action(
            requiresSession ? CredentialStatus(state: status.sessionState) : status,
            calls: model.credentialCalls)
        VStack(alignment: .leading, spacing: TC.Space.sm) {
            if prominent {
                Label(copy.credentialTitle, systemImage: "person.crop.circle")
                    .font(.title2.weight(.semibold))
            } else {
                TCSectionHeader(title: copy.credentialTitle)
            }
            Text(copy.credentialWhat)
                .font(TC.Font_.body)
                .fixedSize(horizontal: false, vertical: true)
            // What is true, painted from the shared tone -- never from
            // whether this shell happens to be holding an attempt.
            Label(
                CredentialSurface.stateLine(status, copy: copy, calls: model.credentialCalls),
                systemImage: tone.symbol
            )
            .font(TC.Font_.body)
            .foregroundStyle(tone.textColor)
            .fixedSize(horizontal: false, vertical: true)
            // The sentence that must accompany the button comes from the
            // action, not from a branch here, and it is drawn ABOVE the
            // button: what pressing it costs is not a footnote to having
            // pressed it.
            if let explains = CredentialSurface.actionExplains(action, copy: copy) {
                Text(explains)
                    .font(TC.Font_.body)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if action == .obtain {
                Picker(copy.credentialProviderLabel, selection: $provider) {
                    Text(copy.credentialProviderGithub).tag("github")
                    Text(copy.credentialProviderGoogle).tag("google")
                    Text(copy.credentialProviderNear).tag("near")
                }
                .frame(minHeight: 44)
                .disabled(model.credentialBusy)
                if provider == "near" {
                    Text(copy.credentialWalletNotice)
                        .font(TC.Font_.body)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            actionButton(action)
            // What the key is worth, on the same card as the key. A balance
            // is the one fact on this screen about an ACCOUNT rather than
            // this computer, and it is here because it is the thing the
            // sign-in above is for -- a contributor who has just signed in
            // should not have to go looking for what they signed in to see.
            //
            // `credentialAction` is passed so the two rows cannot draw the
            // same sign-in button twice; the decision is
            // `BalanceSurface.actionToDraw`'s, not this view's.
            if !requiresSession && action != .obtain {
                Divider().padding(.vertical, TC.Space.xs)
                BalanceRow(copy: copy, credentialAction: action, run: run)
                Divider().padding(.vertical, TC.Space.xs)
                FundingRow(copy: copy)
            }
        }
        .task(id: action) {
            // A state nobody could read polls nothing. There is no outcome
            // pending and no attempt to name.
            while action == .cancel, !Task.isCancelled {
                try? await Task.sleep(for: Self.pollInterval)
                guard !Task.isCancelled else { return }
                model.refreshNearAiCredential()
            }
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
        if let label = CredentialSurface.actionLabel(action, copy: copy) {
            if prominent && action == .obtain {
                Button(label) { run(action) }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                    .tint(TC.green)
                    .disabled(model.credentialBusy)
            } else {
                Button(label) { run(action) }
                    .buttonStyle(.bordered)
                    .disabled(model.credentialBusy)
            }
        }
    }

    private func run(_ action: CredentialAction) {
        switch action {
        case .obtain:
            Task {
                // The URL is served once, by start. Opening it is this
                // view's environment; the model has no window.
                guard let url = await model.startNearAiCredential(provider: provider) else { return }
                let accepted = await withCheckedContinuation { continuation in
                    openURL(url) { continuation.resume(returning: $0) }
                }
                // A browser that never opened leaves a ceremony nobody can
                // finish, so it is stopped rather than left to time out.
                if !accepted { model.cancelNearAiCredential() }
            }
        case .cancel:
            model.cancelNearAiCredential()
        case .forget:
            model.forgetNearAiCredential()
        case .none:
            break
        }
    }
}
