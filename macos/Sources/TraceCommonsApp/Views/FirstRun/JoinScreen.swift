import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// What the invite card shows under its field.
enum JoinInviteLine: Equatable {
    /// Nothing looked up yet.
    case hidden
    /// The invite's host, read locally before the daemon runs.
    case host(String)
    /// The core's joined line: the daemon enrolled the invite.
    case joined(String)
    /// The core's invite error.
    case error(String)
}

/// Join's decisions (#1030 `join-screen.tsx`), apart from the view so they
/// can be tested. Every string is the core's.
enum JoinLayout {
    /// Contributing needs an account; without one, setup is watching only.
    static func hasAccount(_ state: FirstRunState) -> Bool {
        switch state.account {
        case .none, .watchOnly: return false
        case .nearAI, .passkey: return true
        }
    }

    static func footerTitle(_ state: FirstRunState, copy: FirstRunCopy) -> String {
        hasAccount(state) ? copy.frame.continueButton : copy.join.skip
    }

    static func footerNote(_ state: FirstRunState, copy: FirstRunCopy) -> String? {
        hasAccount(state) ? nil : copy.join.skipNote
    }

    /// The footer's action. Without an account it is "Skip: watch only",
    /// which answers watch only; either way the person moves on.
    static func forward(_ state: FirstRunState) -> FirstRunState {
        var forwarded = state
        if !hasAccount(forwarded) { forwarded.account = .watchOnly }
        return FirstRunNavigation.next(forwarded)
    }

    /// near.ai is chosen here and signed in after the daemon starts
    /// (`FirstRunPlan`), so the button only chooses it, and pressed again
    /// undoes the choice until the sign-in happened. A held account -- a
    /// signed-in near.ai or a passkey the daemon bound -- is not replaced.
    static func toggleNearAI(_ state: FirstRunState) -> FirstRunState {
        guard canToggleNearAI(state) else { return state }
        var toggled = state
        toggled.account = nearAIChosen(state) ? .none : .nearAI
        return toggled
    }

    static func canToggleNearAI(_ state: FirstRunState) -> Bool {
        switch state.account {
        case .none, .watchOnly: return true
        case .nearAI: return !state.signedIn
        case .passkey: return false
        }
    }

    /// near.ai chosen and not yet signed in: the choice is still undoable.
    static func nearAIChosen(_ state: FirstRunState) -> Bool {
        state.account == .nearAI && !state.signedIn
    }

    static func nearAILine(_ state: FirstRunState, copy: FirstRunCopy.Join) -> String {
        nearAIChosen(state) ? copy.nearAiChosen : copy.nearAiText
    }

    static func nearAIAction(_ state: FirstRunState, copy: FirstRunCopy.Join) -> String {
        nearAIChosen(state) ? copy.nearAiUndo : copy.nearAiSignIn
    }

    /// "Signed in" is the daemon's fact, never the choice.
    static func showsSignedIn(_ state: FirstRunState) -> Bool {
        state.signedIn
    }

    /// "Look up": the host is read locally (`TCInvite.issuerHost`), since the
    /// daemon that would look the invite up does not run yet. Something that
    /// is not an invite is refused and not kept, so it can never be enrolled.
    /// A found invite replaces the one the daemon refused, so that refusal
    /// (`.inviteDead`) is cleared; any other failure is kept.
    static func lookUp(
        _ draft: String, in state: FirstRunState, failure: FirstRunFailure?, host: (String) -> String?
    ) -> (state: FirstRunState, found: Bool, failure: FirstRunFailure?) {
        let invite = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        var looked = state
        guard let issuerHost = host(invite) else {
            looked.invite = ""
            looked.issuerHost = nil
            return (looked, false, failure)
        }
        looked.invite = invite
        looked.issuerHost = issuerHost
        if case .inviteDead = failure { return (looked, true, nil) }
        return (looked, true, failure)
    }

    /// Once the daemon enrolled an invite the field is read-only, so an
    /// enrolled person cannot paste a second one.
    static func inviteIsEditable(_ state: FirstRunState) -> Bool {
        state.enrolledInvite == nil
    }

    static func inviteLine(
        _ state: FirstRunState,
        lookup: DaemonData.InviteLookup?,
        failure: FirstRunFailure?,
        copy: FirstRunCopy.Join,
        refused: Bool = false
    ) -> JoinInviteLine {
        if state.enrolledInvite != nil {
            return .joined(
                copy.inviteJoined
                    .replacingOccurrences(of: "{host}", with: state.issuerHost ?? dash)
                    .replacingOccurrences(of: "{pay_range}", with: payRange(lookup)))
        }
        if refused { return .error(copy.inviteError) }
        if case .inviteDead = failure { return .error(copy.inviteError) }
        // Watch only joins no invite (`FirstRunPlan`), so none is shown as
        // if it would be.
        if state.account == .watchOnly { return .hidden }
        if let host = state.issuerHost, !state.invite.isEmpty { return .host(host) }
        return .hidden
    }

    /// The invite's credit range, or a dash when the daemon gave none: an
    /// unknown range never reads as a figure.
    static func payRange(_ lookup: DaemonData.InviteLookup?) -> String {
        guard let range = lookup?.creditRange else { return dash }
        let span = range.min == range.max ? "\(range.min)" : "\(range.min)–\(range.max)"
        return "\(span) \(range.unit)"
    }

    /// The passkey ceremony completes with the daemon, which runs only once
    /// Folders or Tools commits, so before then creating one is unavailable.
    static func passkeyAvailable(_ state: FirstRunState, hasPasskeyAccount: Bool) -> Bool {
        hasPasskeyAccount && state.daemonStarted
    }

    static func passkeyDone(_ state: FirstRunState) -> Bool {
        if case .passkey = state.account { return true }
        return false
    }

    /// The passkey card's line: the invitation before a passkey exists, the
    /// ready line with its name after one was created, and nothing for a
    /// passkey known without a name (a sign-in, or an existing account).
    static func passkeyLine(_ state: FirstRunState, copy: FirstRunCopy.Join) -> String? {
        guard case .passkey(let name) = state.account else { return copy.passkeyText }
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : copy.passkeyReady.replacingOccurrences(of: "{name}", with: trimmed)
    }

    /// Record how the passkey sheets ended. A sign-in carries no name
    /// (`NativePasskeyCoordinator.perform(.login)` returns none), so its
    /// passkey is held with an empty one, which `passkeyLine` never shows.
    static func apply(
        _ outcome: PasskeySheetOutcome, to state: FirstRunState, copy: FirstRunCopy
    ) -> (state: FirstRunState, notice: String?) {
        var applied = state
        switch outcome {
        case .created(let name): applied.account = .passkey(name: name)
        case .signedIn, .existingAccount: applied.account = .passkey(name: "")
        case .closed: break
        case .signedOut:
            // `account_sign_out` clears the daemon's account session, the
            // one a near.ai sign-in holds too, so that fact goes with it and
            // a chosen near.ai is signed in again at the next commit.
            if passkeyDone(applied) { applied.account = .none }
            applied.signedIn = false
        }
        return (applied, outcome.joinNotice(copy))
    }

    private static let dash = "—"
}

/// Ron's Join (#1030 `join-screen.tsx`) in glass: the title, the invite card,
/// the passkey and near.ai cards, the quiet no-sharing card, and "Skip:
/// watch only" until an account exists.
///
/// Join holds no daemon client: on a first pass the daemon is not running.
/// The invite is looked up and enrolled, and a chosen near.ai signed in,
/// when Folders or Tools commits (`FirstRunPlan`). The one daemon path here
/// is `passkeyAccount`, whose ceremony completes with the daemon, so
/// creating a passkey is available only once the daemon started (back on
/// Join after Folders or Tools) and a `passkeyAccount` was given.
struct JoinScreen: View {
    let copy: FirstRunCopy
    @ObservedObject var runner: FirstRunRunner
    let passkeyAccount: (any PasskeyAccount)?
    var issuerHost: (String) -> String? = TCInvite.issuerHost

    @State private var draft: String?
    @State private var refused = false
    @State private var notice: String?
    @State private var passkeyModel: PasskeySheetModel?

    var body: some View {
        FirstRunFrame(
            copy: copy,
            state: $runner.state,
            onBack: nil,
            footer: FirstRunFooter(
                title: JoinLayout.footerTitle(runner.state, copy: copy),
                isEnabled: true,
                note: JoinLayout.footerNote(runner.state, copy: copy),
                action: { runner.state = JoinLayout.forward(runner.state) })
        ) {
            ScrollView {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
                    title
                    inviteCard
                    passkeyCard
                    nearAICard
                    if let notice {
                        GlassNotice(tone: .ask) { Text(notice) }
                    }
                    GlassCard(quiet: true) {
                        Text(copy.join.noSharing)
                            .glassType(GlassTokens.TypeScale.label)
                            .foregroundStyle(GlassColor.textSecondary)
                    }
                }
            }
        }
        .sheet(isPresented: passkeyPresented) {
            if let model = passkeyModel {
                PasskeySheets(copy: copy, model: model, returningName: nil) { outcome in
                    let applied = JoinLayout.apply(outcome, to: runner.state, copy: copy)
                    runner.state = applied.state
                    notice = applied.notice
                    passkeyModel = nil
                }
            }
        }
    }

    private var title: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            (Text(copy.join.titleLight) + Text(copy.join.titleBold).bold())
                .glassType(GlassTokens.TypeScale.display)
                .foregroundStyle(GlassColor.textPrimary)
            (Text(copy.join.body) + Text(" ") + Text(copy.join.bodyEmphasis).bold())
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textSecondary)
        }
    }

    private var inviteCard: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                if JoinLayout.inviteIsEditable(runner.state) {
                    HStack(alignment: .bottom, spacing: GlassTokens.Space.s3) {
                        GlassTextField(copy.join.inviteEyebrow, text: draftBinding, prompt: copy.join.invitePlaceholder)
                            .onSubmit(lookUp)
                        Button(copy.join.lookUp, action: lookUp)
                            .buttonStyle(GlassButtonStyle(.glass))
                            .disabled(currentDraft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                } else {
                    Text(copy.join.inviteEyebrow)
                        .glassType(GlassTokens.TypeScale.eyebrow)
                        .foregroundStyle(GlassColor.textTertiary)
                }
                inviteLine
            }
        }
    }

    @ViewBuilder
    private var inviteLine: some View {
        switch JoinLayout.inviteLine(
            runner.state, lookup: runner.lookup, failure: runner.failure, copy: copy.join, refused: refused)
        {
        case .hidden:
            EmptyView()
        case .host(let host):
            GlassStatusLabel(host, status: .ask)
        case .joined(let line):
            GlassStatusLabel(line, status: .on)
        case .error(let line):
            GlassNotice(tone: .outside) { Text(line) }
        }
    }

    private var passkeyCard: some View {
        accountCard(
            eyebrow: copy.join.passkeyEyebrow,
            text: JoinLayout.passkeyLine(runner.state, copy: copy.join),
            done: JoinLayout.passkeyDone(runner.state) ? copy.join.passkeyDone : nil
        ) {
            Button(copy.join.passkeyCreate, action: openPasskey)
                .buttonStyle(GlassButtonStyle(.glass))
                .disabled(!JoinLayout.passkeyAvailable(runner.state, hasPasskeyAccount: passkeyAccount != nil))
        }
    }

    private var nearAICard: some View {
        accountCard(
            eyebrow: copy.join.nearAiEyebrow,
            text: JoinLayout.nearAILine(runner.state, copy: copy.join),
            done: JoinLayout.showsSignedIn(runner.state) ? copy.join.signedIn : nil
        ) {
            Button {
                runner.state = JoinLayout.toggleNearAI(runner.state)
            } label: {
                Label(
                    JoinLayout.nearAIAction(runner.state, copy: copy.join),
                    systemImage: JoinLayout.nearAIChosen(runner.state)
                        ? "arrow.uturn.backward" : "arrow.up.right.square")
                .labelStyle(.titleAndIcon)
            }
            .buttonStyle(GlassButtonStyle(.glass))
            .disabled(!JoinLayout.canToggleNearAI(runner.state))
        }
    }

    /// Ron's `AccountCard`: eyebrow and line on the left, the action on the
    /// right until it is done, then its status.
    private func accountCard<Action: View>(
        eyebrow: String, text: String?, done: String?, @ViewBuilder action: () -> Action
    ) -> some View {
        GlassCard {
            HStack(spacing: GlassTokens.Space.s6) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                    Text(eyebrow)
                        .glassType(GlassTokens.TypeScale.eyebrow)
                        .foregroundStyle(GlassColor.textTertiary)
                    if let text {
                        Text(text)
                            .glassType(GlassTokens.TypeScale.label)
                            .foregroundStyle(GlassColor.textSecondary)
                    }
                }
                Spacer(minLength: 0)
                if let done {
                    GlassStatusLabel(done, status: .on)
                } else {
                    action()
                }
            }
        }
    }

    private var currentDraft: String {
        draft ?? runner.state.invite
    }

    private var draftBinding: Binding<String> {
        Binding(
            get: { currentDraft },
            set: {
                draft = $0
                refused = false
            })
    }

    private var passkeyPresented: Binding<Bool> {
        Binding(
            get: { passkeyModel != nil },
            set: { if !$0 { passkeyModel = nil } })
    }

    private func lookUp() {
        let looked = JoinLayout.lookUp(currentDraft, in: runner.state, failure: runner.failure, host: issuerHost)
        runner.state = looked.state
        runner.failure = looked.failure
        refused = !looked.found
        if looked.found { draft = nil }
    }

    /// A fresh model each time: a model's outcome is set once, so a reused
    /// one would leave the second presentation unable to finish.
    private func openPasskey() {
        guard let passkeyAccount else { return }
        notice = nil
        passkeyModel = PasskeySheetModel(copy: copy.passkey, account: passkeyAccount)
    }
}
