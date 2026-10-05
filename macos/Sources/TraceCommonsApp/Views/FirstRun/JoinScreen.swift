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

/// How "Look up" ended.
enum JoinLookUpOutcome: Equatable {
    /// The invite's host was read; the invite is kept.
    case found
    /// Not an invite: refused, and not kept.
    case refused
    /// The field was empty: the kept invite is dropped, with no error.
    case withdrawn
}

/// Join's decisions (#1030 `join-screen.tsx`), apart from the view so they
/// can be tested. Every string is the core's.
enum JoinLayout {
    /// Contributing needs an account; without one, setup is watching only.
    static func hasAccount(_ state: FirstRunState) -> Bool {
        switch state.account {
        case .none, .watchOnly: return false
        case .nearAI, .passkeyChosen, .passkey, .enrolled: return true
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
    /// signed-in near.ai or a passkey the daemon bound -- is not replaced; a
    /// passkey only chosen is.
    static func toggleNearAI(_ state: FirstRunState) -> FirstRunState {
        guard canToggleNearAI(state) else { return state }
        var toggled = state
        toggled.account = nearAIChosen(state) ? .none : .nearAI
        return toggled
    }

    static func canToggleNearAI(_ state: FirstRunState) -> Bool {
        switch state.account {
        case .none, .watchOnly, .passkeyChosen: return true
        case .nearAI: return !state.signedIn
        case .passkey, .enrolled: return false
        }
    }

    /// near.ai chosen and not yet signed in: the choice is still undoable.
    static func nearAIChosen(_ state: FirstRunState) -> Bool {
        state.account == .nearAI && !state.signedIn
    }

    /// Nothing while a passkey is held: near.ai cannot be chosen over it.
    static func nearAILine(_ state: FirstRunState, copy: FirstRunCopy.Join) -> String? {
        if passkeyDone(state) || state.account == .enrolled { return nil }
        return nearAIChosen(state) ? copy.nearAiChosen : copy.nearAiText
    }

    static func nearAIAction(_ state: FirstRunState, copy: FirstRunCopy) -> String {
        nearAIChosen(state) ? copy.frame.undo : copy.join.nearAiSignIn
    }

    /// "Signed in" is the daemon's fact, never the choice.
    static func showsSignedIn(_ state: FirstRunState) -> Bool {
        state.signedIn
    }

    /// "Look up": the host is read locally (`TCInvite.issuerHost`), since the
    /// daemon that would look the invite up does not run yet. Something that
    /// is not an invite is refused and not kept, so it can never be enrolled.
    ///
    /// An empty field withdraws the kept invite. That is how a person whose
    /// invite the daemon refused goes on without one: the refusal
    /// (`.inviteDead`) goes with it, and nothing is joined. A found invite
    /// other than the refused one clears that refusal too; the same invite
    /// looked up again keeps it. Any other failure is kept.
    ///
    /// While that refusal stands, a refused paste leaves the refused invite
    /// held, so the refusal stays tied to it: looked up again it is still
    /// the same invite, and an emptied field can still withdraw it.
    ///
    /// Looking an invite up asks to join, so it takes back watch only.
    static func lookUp(
        _ draft: String, in state: FirstRunState, failure: FirstRunFailure?, host: (String) -> String?
    ) -> (state: FirstRunState, outcome: JoinLookUpOutcome, failure: FirstRunFailure?) {
        let invite = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        var looked = state
        let deadCleared: FirstRunFailure? = {
            if case .inviteDead = failure { return nil }
            return failure
        }()
        if invite.isEmpty {
            looked.invite = ""
            looked.issuerHost = nil
            return (looked, .withdrawn, deadCleared)
        }
        guard let issuerHost = host(invite) else {
            if case .inviteDead = failure { return (looked, .refused, failure) }
            looked.invite = ""
            looked.issuerHost = nil
            return (looked, .refused, failure)
        }
        looked.invite = invite
        looked.issuerHost = issuerHost
        if looked.account == .watchOnly { looked.account = .none }
        let same = invite == state.invite.trimmingCharacters(in: .whitespacesAndNewlines)
        return (looked, .found, same ? failure : deadCleared)
    }

    /// "Look up" acts on a non-empty field, or on an empty one while an
    /// invite is kept, which it then withdraws.
    static func canLookUp(_ draft: String, in state: FirstRunState) -> Bool {
        !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !state.invite.isEmpty
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
    /// Folders or Tools commits. Before then Create passkey records the
    /// choice (`togglePasskey`) and the commit opens the sheets
    /// (`FirstRunCall.openPasskeySheets`); once the daemon runs, it opens
    /// them now. A chosen passkey's button undoes instead. A signed-in
    /// near.ai already holds the daemon's account session, so a passkey is
    /// not created over it, as near.ai is not chosen over a passkey
    /// (`canToggleNearAI`).
    static func passkeyOpensNow(_ state: FirstRunState, hasPasskeyAccount: Bool) -> Bool {
        hasPasskeyAccount && state.daemonStarted && !state.signedIn && !passkeyChosen(state)
            && !passkeyDone(state) && state.account != .enrolled
    }

    /// Create passkey chosen and not yet created: the choice is undoable.
    static func passkeyChosen(_ state: FirstRunState) -> Bool {
        state.account == .passkeyChosen
    }

    /// Choose Create passkey, or undo that choice. A chosen near.ai is
    /// replaced, as a chosen passkey is by near.ai; a held account is not.
    static func togglePasskey(_ state: FirstRunState) -> FirstRunState {
        guard showsPasskeyAction(state), !passkeyDone(state) else { return state }
        var toggled = state
        toggled.account = passkeyChosen(state) ? .none : .passkeyChosen
        return toggled
    }

    static func passkeyAction(_ state: FirstRunState, copy: FirstRunCopy) -> String {
        passkeyChosen(state) ? copy.frame.undo : copy.join.passkeyCreate
    }

    /// A signed-in near.ai is held, and a passkey is never created over it,
    /// so the passkey card offers no action then (`passkeyOpensNow`); nor
    /// over an enrolment an earlier first run left.
    static func showsPasskeyAction(_ state: FirstRunState) -> Bool {
        !state.signedIn && state.account != .enrolled
    }

    /// A held passkey is never replaced by near.ai (`canToggleNearAI`), so
    /// the near.ai card offers no action then.
    static func showsNearAIAction(_ state: FirstRunState) -> Bool {
        !passkeyDone(state) && state.account != .enrolled
    }

    static func passkeyDone(_ state: FirstRunState) -> Bool {
        if case .passkey = state.account { return true }
        return false
    }

    /// The passkey card's line: the invitation before a passkey exists, when
    /// it happens once chosen, the ready line with its name after one was
    /// created, and nothing for a
    /// passkey known without a name (a sign-in, or an existing account) or
    /// while a signed-in near.ai holds the account.
    static func passkeyLine(_ state: FirstRunState, copy: FirstRunCopy.Join) -> String? {
        if passkeyChosen(state) { return copy.passkeyChosen }
        guard case .passkey(let name) = state.account else {
            return showsPasskeyAction(state) ? copy.passkeyText : nil
        }
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
            // a chosen near.ai is signed in again at the next commit. The
            // passkey, held or only chosen, is not asked for again.
            if passkeyDone(applied) || passkeyChosen(applied) { applied.account = .none }
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
/// The invite is looked up and enrolled, a chosen near.ai signed in and a
/// chosen passkey's sheets opened, when Folders or Tools commits
/// (`FirstRunPlan`). The one daemon path here is `passkeyAccount`, whose
/// ceremony completes with the daemon: back on Join after the daemon
/// started, Create passkey opens the sheets at once. They are presented by
/// `firstRunPasskeySheets`, which the first-run host
/// (`OnboardingCoordinatorView`) mounts once for every step.
struct JoinScreen: View {
    let copy: FirstRunCopy
    @ObservedObject var runner: FirstRunRunner
    let passkeyAccount: (any PasskeyAccount)?
    var issuerHost: (String) -> String? = JoinScreen.defaultIssuerHost

    /// The core's host reader, named so the test reaches the function the
    /// screen uses.
    static func defaultIssuerHost(_ invite: String) -> String? { TCInvite.issuerHost(invite) }

    @State private var draft: String?
    @State private var refused = false

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
                    if let notice = runner.passkeyOutcome?.joinNotice(copy) {
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
                            .disabled(!JoinLayout.canLookUp(currentDraft, in: runner.state))
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
            done: JoinLayout.passkeyDone(runner.state) ? copy.join.passkeyDone : nil,
            showsAction: JoinLayout.showsPasskeyAction(runner.state)
        ) {
            Button(action: passkeyAction) {
                // Choosing opens nothing until the daemon runs, so only the
                // undo carries a glyph, as on the near.ai card.
                if JoinLayout.passkeyChosen(runner.state) {
                    Label(JoinLayout.passkeyAction(runner.state, copy: copy), systemImage: "arrow.uturn.backward")
                        .labelStyle(.titleAndIcon)
                } else {
                    Text(JoinLayout.passkeyAction(runner.state, copy: copy))
                }
            }
            .buttonStyle(GlassButtonStyle(.glass))
        }
    }

    private var nearAICard: some View {
        accountCard(
            eyebrow: copy.join.nearAiEyebrow,
            text: JoinLayout.nearAILine(runner.state, copy: copy.join),
            done: JoinLayout.showsSignedIn(runner.state) ? copy.join.signedIn : nil,
            showsAction: JoinLayout.showsNearAIAction(runner.state)
        ) {
            Button {
                runner.state = JoinLayout.toggleNearAI(runner.state)
            } label: {
                // Choosing near.ai opens nothing (the sign-in comes after the
                // daemon starts), so only the undo carries a glyph.
                if JoinLayout.nearAIChosen(runner.state) {
                    Label(JoinLayout.nearAIAction(runner.state, copy: copy), systemImage: "arrow.uturn.backward")
                        .labelStyle(.titleAndIcon)
                } else {
                    Text(JoinLayout.nearAIAction(runner.state, copy: copy))
                }
            }
            .buttonStyle(GlassButtonStyle(.glass))
            .disabled(!JoinLayout.canToggleNearAI(runner.state))
        }
    }

    /// Ron's `AccountCard`: eyebrow and line on the left, the action on the
    /// right until it is done, then its status. With the other account held
    /// there is no action to offer, so none is shown.
    private func accountCard<Action: View>(
        eyebrow: String, text: String?, done: String?, showsAction: Bool, @ViewBuilder action: () -> Action
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
                } else if showsAction {
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

    private func lookUp() {
        let looked = JoinLayout.lookUp(currentDraft, in: runner.state, failure: runner.failure, host: issuerHost)
        runner.state = looked.state
        runner.failure = looked.failure
        refused = looked.outcome == .refused
        if looked.outcome != .refused { draft = nil }
    }

    /// Before the daemon runs, record or undo the choice; once it runs,
    /// open the sheets.
    private func passkeyAction() {
        if JoinLayout.passkeyOpensNow(runner.state, hasPasskeyAccount: passkeyAccount != nil) {
            runner.requestPasskey()
        } else {
            runner.state = JoinLayout.togglePasskey(runner.state)
        }
    }
}
