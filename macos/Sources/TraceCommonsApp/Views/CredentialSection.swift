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
    @State private var ownProvider = "github"
    /// Where the provider choice lives when the caller holds it. `nil` -- every
    /// production caller -- keeps it in this view's own state.
    ///
    /// A test seam, and the only one: on macOS 27 SwiftUI draws the picker
    /// itself, with no `NSPopUpButton` beneath it and no accessibility node in
    /// an offscreen host, so a test cannot reach the control to change it.
    /// Holding the same binding the picker writes lets it make the same change.
    var providerSelection: Binding<String>? = nil

    private var selection: Binding<String> { providerSelection ?? $ownProvider }
    private var provider: String { selection.wrappedValue }

    /// The sign-in providers the picker offers, in the order it offers them.
    /// The titles are `PrivateInferenceCopy`'s; the tags are the daemon's
    /// `provider` values.
    struct ProviderOption: Hashable {
        let tag: String
        let title: String
    }

    static func providerOptions(_ copy: PrivateInferenceCopy) -> [ProviderOption] {
        [
            ProviderOption(tag: "github", title: copy.credentialProviderGithub),
            ProviderOption(tag: "google", title: copy.credentialProviderGoogle),
            ProviderOption(tag: "near", title: copy.credentialProviderNear),
        ]
    }

    /// How often a ceremony in flight is re-read.
    ///
    /// It finishes in a browser this app does not own, so nothing here is
    /// told when it does; the poll is what closes that. It runs only while
    /// the shared table still offers a cancel, which is the one state that
    /// has an outcome pending.
    private static let pollInterval = Duration.seconds(2)

    var body: some View {
        let status = model.credentialStatus
        let tone = CredentialSurface.tone(status, calls: model.credentialCalls)
        let action = CredentialSurface.action(
            requiresSession ? CredentialStatus(state: status.sessionState) : status,
            calls: model.credentialCalls)
        let balanceAction = requiresSession ? CredentialAction.none : BalanceSurface.actionToDraw(
            balance: BalanceSurface.action(model.balanceStatus, calls: model.balanceCalls),
            credential: action)
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
            if let explains = CredentialSurface.actionExplains(action, copy: copy) {
                Text(explains)
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if action == .obtain || balanceAction == .obtain {
                // The glass pill shows only the chosen provider, so the
                // chooser's own word is drawn beside it, as the native
                // picker drew its label. VoiceOver hears it once, from the
                // pill, which carries both the label and the value.
                HStack(spacing: GlassTokens.Space.s4) {
                    Text(copy.credentialProviderLabel)
                        .glassType(GlassTokens.TypeScale.label)
                        .foregroundStyle(GlassColor.textSecondary)
                        .accessibilityHidden(true)
                    GlassPicker(
                        copy.credentialProviderLabel,
                        selection: Binding(
                            get: { provider },
                            set: { if let value = $0 { selection.wrappedValue = value } }),
                        options: Self.providerOptions(copy).map { GlassPickerOption($0.title, value: $0.tag) },
                        placeholder: copy.credentialProviderLabel)
                }
                .frame(minHeight: 44)
                .disabled(model.credentialBusy)
                if provider == "near" {
                    Text(copy.credentialWalletNotice)
                        .glassType(GlassTokens.TypeScale.body)
                        .foregroundStyle(GlassColor.textPrimary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            actionButton(action)
            // The balance and funding are their own panels after this card,
            // as #1146 draws them (`PrivateAIBalanceCard`, `FundingRow`). The
            // sign-in the balance needs stays here, beside the provider
            // chooser it uses, and never doubles this card's own button: the
            // decision is `BalanceSurface.actionToDraw`'s, not this view's.
            actionButton(balanceAction)
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
        case .migrate:
            model.migrateNearAiCredential()
        case .none:
            break
        }
    }
}
