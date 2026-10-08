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
    /// Why the field is closed: a held passkey is an account of its own.
    case note(String)
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
enum JoinScreenLayout {
    /// The extra space above the invite card (under the body) and above the
    /// no-sharing card (under the account cards), on top of the frame's own
    /// gaps (owner, 2026-10-08): `s8` + this under the body, `s6` + this
    /// between the cards.
    static let extraGap = GlassTokens.Space.s4

    /// Contributing needs an account; without one, setup is watching only.
    static func hasAccount(_ state: FirstRunState) -> Bool {
        switch state.account {
        case .none, .watchOnly: return false
        case .nearAI, .passkeyChosen, .passkey, .enrolled: return true
        }
    }

    /// "Skip: watch only" is offered only while the daemon holds no
    /// enrollment (`FirstRunState.daemonHoldsEnrolment`). Watching only
    /// beside one would act under it, and could never finish: its marker is
    /// refused while the daemon is logged in.
    static func offersWatchOnly(_ state: FirstRunState) -> Bool {
        !state.daemonHoldsEnrolment
    }

    /// An enrollment the daemon holds for this run and nobody signed out of
    /// -- an invite enrolled before a near.ai sign-in failed, near.ai's own
    /// -- is the account when none is answered, as an earlier run's is
    /// (`OnboardingNavigation.recordEnrolment`).
    static func heldEnrolmentIsTheAccount(_ state: FirstRunState) -> Bool {
        !state.signedOutOfEnrolment && (state.enrolledInvite != nil || state.nearAIEnrolled)
    }

    static func footerTitle(_ state: FirstRunState, copy: FirstRunCopy) -> String {
        hasAccount(state) || !offersWatchOnly(state) ? copy.frame.continueButton : copy.join.skip
    }

    static func footerNote(_ state: FirstRunState, copy: FirstRunCopy) -> String? {
        hasAccount(state) || !offersWatchOnly(state) ? nil : copy.join.skipNote
    }

    /// Whether the footer can move on: with an account, as watch only, or
    /// as the enrollment this run holds. An enrollment signed out of waits for
    /// an account to be chosen.
    static func canForward(_ state: FirstRunState) -> Bool {
        hasAccount(state) || offersWatchOnly(state) || heldEnrolmentIsTheAccount(state)
    }

    /// The footer's action. Without an account it is "Skip: watch only",
    /// which answers watch only, or -- while the daemon holds this run's
    /// enrollment -- Continue as that enrollment; either way the person moves
    /// on. With nothing to go on as, it does nothing (`canForward`).
    static func forward(_ state: FirstRunState) -> FirstRunState {
        guard canForward(state) else { return state }
        var forwarded = state
        if !hasAccount(forwarded) {
            forwarded.account = offersWatchOnly(forwarded) ? .watchOnly : .enrolled
        }
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

    /// Signing in with near.ai needs no invite (owner, Ron's review of
    /// #1235): with one it signs in to the account the invite enrolls;
    /// without one it enrolls this Mac through the near.ai login
    /// (`FirstRunPlan`, `near_ai_account_enroll`).
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
        if nearAIChosen(state) { return copy.nearAiChosen }
        return copy.nearAiText
    }

    /// Why near.ai is no longer chosen: its login or enrollment did not
    /// succeed and the commit returned here (`returnToJoin(afterNearAIFailure:)`).
    /// The core's line the step would have said it in, shown only while no
    /// account is answered; choosing again takes it down.
    static func nearAINotice(_ state: FirstRunState, failure: FirstRunFailure?, copy: FirstRunCopy) -> String? {
        guard state.account == .none else { return nil }
        switch failure {
        case .signInFailed?, .nearAIEnrollFailed?:
            return FoldersScreenLayout.notice(for: failure, copy: copy, onboarding: nil)
        default:
            return nil
        }
    }

    /// An invite is held: pasted and found, or already enrolled.
    static func holdsInvite(_ state: FirstRunState) -> Bool {
        !state.invite.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || state.enrolledInvite != nil
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
    /// Looking an invite up asks to join, so it takes back watch only, and a
    /// passkey only chosen: a new passkey is an account of its own, never
    /// combined with an invite. Withdrawing the invite keeps near.ai, which
    /// needs none.
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
        if looked.account == .watchOnly || looked.account == .passkeyChosen { looked.account = .none }
        let same = invite == state.invite.trimmingCharacters(in: .whitespacesAndNewlines)
        return (looked, .found, same ? failure : deadCleared)
    }

    /// "Look up" acts on a non-empty field, or on an empty one while an
    /// invite is kept, which it then withdraws.
    static func canLookUp(_ draft: String, in state: FirstRunState) -> Bool {
        !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !state.invite.isEmpty
    }

    /// Once the daemon enrolled an invite the field is read-only, so an
    /// enrolled person cannot paste a second one; and while a passkey is
    /// held, which is an account of its own, no invite is joined beside it.
    static func inviteIsEditable(_ state: FirstRunState) -> Bool {
        state.enrolledInvite == nil && !passkeyDone(state)
    }

    /// The card shows its invite field unless an invite was joined or a
    /// passkey holds the account: an enrollment recorded with no invite
    /// shows the field, closed (`inviteIsEditable`), rather than a joined
    /// line naming an invite nobody entered.
    static func showsInviteField(_ state: FirstRunState) -> Bool {
        inviteIsEditable(state) || (!joinedInvite(state) && !passkeyDone(state))
    }

    /// An invite this first run (or an earlier one) is known to have joined.
    /// An enrollment recorded without one (`OnboardingNavigation.recordEnrolment`
    /// writes an empty `enrolledInvite`: an earlier run joined through
    /// near.ai or a passkey, or the daemon was already enrolled) joined no
    /// invite the shell knows of, so the card never says "Joined" for it.
    static func joinedInvite(_ state: FirstRunState) -> Bool {
        guard let enrolled = state.enrolledInvite else { return false }
        return !enrolled.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    static func inviteLine(
        _ state: FirstRunState,
        lookup: DaemonData.InviteLookup?,
        failure: FirstRunFailure?,
        copy: FirstRunCopy.Join,
        refused: Bool = false
    ) -> JoinInviteLine {
        if joinedInvite(state) {
            return .joined(
                FirstRunCopy.fill(
                    copy.inviteJoined,
                    ["host": state.issuerHost ?? copy.unknown, "pay_range": payRange(lookup, copy: copy)]))
        }
        if passkeyDone(state) { return .note(copy.inviteOrPasskey) }
        if refused { return .error(copy.inviteError) }
        if case .inviteDead = failure { return .error(copy.inviteDead) }
        // Watch only joins no invite (`FirstRunPlan`), so none is shown as
        // if it would be.
        if state.account == .watchOnly { return .hidden }
        if let host = state.issuerHost, !state.invite.isEmpty { return .host(host) }
        return .hidden
    }

    /// The invite's credit range in the core's words, or the core's
    /// `unknown` when the daemon gave none or gave a unit this build cannot
    /// word: an unknown range never reads as a figure, and the wire label
    /// never reaches the screen.
    static func payRange(_ lookup: DaemonData.InviteLookup?, copy: FirstRunCopy.Join) -> String {
        guard let range = lookup?.creditRange, range.unit == pointsPerAcceptedTrace else { return copy.unknown }
        if range.min == range.max {
            return FirstRunCopy.fill(copy.payRangePointsOne, ["min": "\(range.min)"])
        }
        return FirstRunCopy.fill(copy.payRangePoints, ["min": "\(range.min)", "max": "\(range.max)"])
    }

    /// The one credit-range unit the daemon accepts (`CREDIT_RANGE_UNIT_POINTS`).
    static let pointsPerAcceptedTrace = "points_per_accepted_trace"

    /// The passkey ceremony completes with the daemon. Once it runs, Create
    /// passkey opens the sheets now, over Join (#1030 `ftux-page.tsx`). A
    /// chosen passkey's button undoes instead (a choice an earlier build
    /// recorded for the Folders or Tools commit). A signed-in near.ai
    /// already holds the daemon's account session, so a passkey is not
    /// created over it, as near.ai is not chosen over a passkey
    /// (`canToggleNearAI`).
    static func passkeyOpensNow(_ state: FirstRunState, hasPasskeyAccount: Bool) -> Bool {
        hasPasskeyAccount && state.daemonStarted && offersPasskey(state)
    }

    /// Before the daemon runs, Create passkey starts it, watching nothing,
    /// and then opens the sheets over Join (`FirstRunRunner.openPasskeyOnJoin`),
    /// so the person never meets them on a later step.
    static func passkeyStartsDaemon(_ state: FirstRunState, hasPasskeyAccount: Bool) -> Bool {
        offersPasskey(state) && !passkeyOpensNow(state, hasPasskeyAccount: hasPasskeyAccount)
    }

    /// Create passkey is the button's action: offered, not chosen, not held.
    private static func offersPasskey(_ state: FirstRunState) -> Bool {
        showsPasskeyAction(state) && !passkeyChosen(state) && !passkeyDone(state)
    }

    /// The line under the passkey card when Create passkey could not start
    /// the daemon its sheets need.
    static func passkeyFailureLine(_ failure: FirstRunFailure?, copy: FirstRunCopy.Join) -> String? {
        failure == .passkeyUnavailable ? copy.passkeyUnavailable : nil
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
    /// over an enrollment an earlier first run left; nor beside a held
    /// invite, since the daemon refuses to create a passkey account over an
    /// enrollment (`account-already-enrolled`) and `passkeyLine` says so.
    static func showsPasskeyAction(_ state: FirstRunState) -> Bool {
        !state.signedIn && state.account != .enrolled && !holdsInvite(state)
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
            if showsPasskeyAction(state) { return copy.passkeyText }
            let heldBack = holdsInvite(state) && !state.signedIn && state.account != .enrolled
            return heldBack ? copy.inviteOrPasskey : nil
        }
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : FirstRunCopy.fill(copy.passkeyReady, ["name": trimmed])
    }

    /// Signing out clears every sign-in on Join (#1030 rule 6): the
    /// invite, near.ai and the passkey, held or only chosen, so no card
    /// claims an account that is not linked. `account_sign_out` has already
    /// ended the daemon's account session, the one near.ai holds too.
    ///
    /// An enrollment the daemon holds -- this run's invite, near.ai's
    /// invite-free enrollment, a passkey Verify bound, an earlier first
    /// run's -- outlives the sign-out until the daemon unenrolls. It is
    /// cleared here and marked (`signedOutOfEnrolment`), which fails
    /// closed: nothing that belongs to an enrollment is sent for it, and it
    /// is not recorded as the account again. The runner then asks the
    /// daemon to unenroll (`FirstRunRunner.unenrollAfterSignOut`) and clears
    /// the mark once it confirms, which offers watch only again. Every other
    /// answer (tools, rules, uses) is kept.
    static func signOut(_ state: FirstRunState) -> FirstRunState {
        var cleared = state
        if state.holdsEnrolment || state.enrolledInvite != nil || state.nearAIEnrolled || state.account == .enrolled {
            cleared.signedOutOfEnrolment = true
        }
        cleared.account = .none
        cleared.signedIn = false
        cleared.nearAIEnrolled = false
        cleared.invite = ""
        cleared.issuerHost = nil
        cleared.enrolledInvite = nil
        return cleared
    }

    /// Record how the passkey sheets ended. A sign-in ends them only once
    /// Verify bound its account, or joined this Mac to the account another
    /// Mac bound (`PasskeySheetOutcome.signedIn`), so every held passkey is
    /// an enrollment. The shell's login result carries no name
    /// (`NativePasskeyCoordinator.perform(.login)` returns none); the
    /// outcome carries the one the daemon remembers for the signed-in
    /// account's own record, when it has one: a name given here, or the
    /// label the server returned for that passkey at sign-in. Without it (and after a switch
    /// to an existing account) the passkey is held with an empty name, which
    /// `passkeyLine` never shows.
    static func apply(
        _ outcome: PasskeySheetOutcome, to state: FirstRunState, copy: FirstRunCopy
    ) -> (state: FirstRunState, notice: String?) {
        var applied = state
        switch outcome {
        case .created(let name):
            applied.account = .passkey(name: name)
            applied.signedOutOfEnrolment = false
        case .signedIn(let name):
            applied.account = .passkey(name: name ?? "")
            applied.signedOutOfEnrolment = false
        case .existingAccount:
            applied.account = .passkey(name: "")
            applied.signedOutOfEnrolment = false
        case .closed: break
        case .signedOut:
            applied = signOut(applied)
        }
        return (applied, outcome.joinNotice(copy))
    }
}

/// Ron's Join (#1030 `join-screen.tsx`) in glass: the title, the invite card,
/// the passkey and near.ai cards, the no-sharing card, and "Skip:
/// watch only" until an account exists.
///
/// Join holds no daemon client: on a first pass the daemon is not running.
/// The invite is looked up and enrolled, and a chosen near.ai signed in,
/// when Folders or Tools commits (`FirstRunPlan`). Create passkey is the
/// exception (#1030): its sheets open over Join, so when the daemon is not
/// running it starts it first, declaring both roots `off` so nothing is read
/// (`CommitPoint.passkeyOnJoin`), and the ceremony completes there through
/// `passkeyAccount`. The sheets are presented by `firstRunPasskeySheets`,
/// which the first-run host (`OnboardingCoordinatorView`) mounts once for
/// every step.
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
            footer: FirstRunFooter(
                title: JoinScreenLayout.footerTitle(runner.state, copy: copy),
                isEnabled: !runner.isCommitting && JoinScreenLayout.canForward(runner.state),
                note: JoinScreenLayout.footerNote(runner.state, copy: copy),
                action: { runner.state = JoinScreenLayout.forward(runner.state) })
        ) {
            title
        } content: {
            inviteCard
                .padding(.top, JoinScreenLayout.extraGap)
            passkeyCard
            nearAICard
            if let notice = JoinScreenLayout.nearAINotice(runner.state, failure: runner.failure, copy: copy) {
                GlassNotice(tone: .outside) { Text(notice) }
            }
            if let notice = runner.passkeyOutcome?.joinNotice(copy) {
                GlassNotice(tone: .ask) { Text(notice) }
            }
            // Plain text below the last sign-in card, in no card of its own
            // (owner, 2026-10-08), set apart from the cards by the extra gap.
            Text(copy.join.noSharing)
                .glassType(GlassTokens.TypeScale.label)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.top, JoinScreenLayout.extraGap)
        }
    }

    /// The title, then the body in two paragraphs: the bold sentence on a
    /// line of its own, in the primary ink (owner, 2026-10-08).
    private var title: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            FirstRunTitle(light: copy.join.titleLight, bold: copy.join.titleBold)
            Text(copy.join.body)
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            Text(copy.join.bodyEmphasis)
                .glassType(GlassTokens.TypeScale.body.weight(.bold))
                .foregroundStyle(GlassColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var inviteCard: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                if JoinScreenLayout.showsInviteField(runner.state) {
                    // The field as it always reads; closed while an
                    // enrollment the daemon holds came with no invite, since
                    // no second invite is joined over it.
                    let editable = JoinScreenLayout.inviteIsEditable(runner.state)
                    HStack(alignment: .bottom, spacing: GlassTokens.Space.s3) {
                        GlassTextField(copy.join.inviteEyebrow, text: draftBinding, prompt: copy.join.invitePlaceholder)
                            .onSubmit(lookUp)
                            .disabled(!editable)
                        Button(copy.join.lookUp, action: lookUp)
                            .buttonStyle(GlassButtonStyle(.glass))
                            .disabled(!editable || !JoinScreenLayout.canLookUp(currentDraft, in: runner.state))
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
        switch JoinScreenLayout.inviteLine(
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
        case .note(let line):
            Text(line)
                .glassType(GlassTokens.TypeScale.label)
                .foregroundStyle(GlassColor.textSecondary)
        }
    }

    @ViewBuilder private var passkeyCard: some View {
        accountCard(
            eyebrow: copy.join.passkeyEyebrow,
            text: JoinScreenLayout.passkeyLine(runner.state, copy: copy.join),
            done: JoinScreenLayout.passkeyDone(runner.state) ? copy.join.passkeyDone : nil,
            showsAction: JoinScreenLayout.showsPasskeyAction(runner.state)
        ) {
            Button(action: passkeyAction) {
                // Only the undo of a choice recorded for later carries a
                // glyph, as on the near.ai card; starting the daemon for the
                // sheets shows the spinner.
                if JoinScreenLayout.passkeyChosen(runner.state) {
                    Label(JoinScreenLayout.passkeyAction(runner.state, copy: copy), systemImage: "arrow.uturn.backward")
                        .labelStyle(.titleAndIcon)
                } else {
                    HStack(spacing: GlassTokens.Space.s3) {
                        if runner.isCommitting { GlassSpinner() }
                        Text(JoinScreenLayout.passkeyAction(runner.state, copy: copy))
                    }
                }
            }
            .buttonStyle(GlassButtonStyle(.glass))
            .disabled(runner.isCommitting)
        }
        if let line = JoinScreenLayout.passkeyFailureLine(runner.failure, copy: copy.join) {
            GlassNotice(tone: .outside) { Text(line) }
        }
    }

    private var nearAICard: some View {
        accountCard(
            eyebrow: copy.join.nearAiEyebrow,
            text: JoinScreenLayout.nearAILine(runner.state, copy: copy.join),
            done: JoinScreenLayout.showsSignedIn(runner.state) ? copy.join.signedIn : nil,
            showsAction: JoinScreenLayout.showsNearAIAction(runner.state)
        ) {
            Button {
                runner.state = JoinScreenLayout.toggleNearAI(runner.state)
            } label: {
                // Choosing near.ai opens nothing (the sign-in comes after the
                // daemon starts), so only the undo carries a glyph.
                if JoinScreenLayout.nearAIChosen(runner.state) {
                    Label(JoinScreenLayout.nearAIAction(runner.state, copy: copy), systemImage: "arrow.uturn.backward")
                        .labelStyle(.titleAndIcon)
                } else {
                    Text(JoinScreenLayout.nearAIAction(runner.state, copy: copy))
                }
            }
            .buttonStyle(GlassButtonStyle(.glass))
            .disabled(!JoinScreenLayout.canToggleNearAI(runner.state))
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
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                // #1030's `ftux-row--between`: the text takes the row and
                // the action keeps its own width. The action is fixed at its
                // ideal size so the text column can never squeeze it to zero
                // width and wrap its label out of sight.
                .frame(maxWidth: .infinity, alignment: .leading)
                if let done {
                    GlassStatusLabel(done, status: .on)
                        .fixedSize()
                } else if showsAction {
                    action()
                        .fixedSize()
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
        guard JoinScreenLayout.inviteIsEditable(runner.state) else { return }
        let looked = JoinScreenLayout.lookUp(currentDraft, in: runner.state, failure: runner.failure, host: issuerHost)
        runner.state = looked.state
        runner.failure = looked.failure
        refused = looked.outcome == .refused
        if looked.outcome != .refused { draft = nil }
    }

    /// Open the sheets over Join: at once while the daemon runs, after
    /// starting it otherwise. A choice recorded for later is undone.
    private func passkeyAction() {
        let hasAccount = passkeyAccount != nil
        if JoinScreenLayout.passkeyOpensNow(runner.state, hasPasskeyAccount: hasAccount) {
            runner.requestPasskey()
        } else if JoinScreenLayout.passkeyStartsDaemon(runner.state, hasPasskeyAccount: hasAccount) {
            Task { await runner.openPasskeyOnJoin() }
        } else {
            runner.state = JoinScreenLayout.togglePasskey(runner.state)
        }
    }
}
