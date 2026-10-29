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
    /// The app needs a NEAR AI account (owner, 2026-10-12: sign-in
    /// required), so Start sharing waits for one: near.ai chosen and signed
    /// in (`canStart`); a passkey the
    /// daemon bound, which binding ties to this Mac's near.ai login
    /// (`account_bind`); or the enrollment near.ai made for this run, not
    /// signed out of. An invite alone, a passkey only chosen, or an earlier
    /// run's enrollment whose account is not known here is not one: near.ai
    /// is chosen beside it (`canToggleNearAI`), and an invite is then still
    /// joined first (`FirstRunPlan.leaveRoots`). There is no watching only.
    static func hasNearAIAccount(_ state: FirstRunState) -> Bool {
        switch state.account {
        case .nearAI, .passkey: return true
        case .none, .watchOnly, .passkeyChosen, .enrolled:
            return state.nearAIEnrolled && !state.signedOutOfEnrolment
        }
    }

    /// An authenticated last step may resolve a sole cloud organization.
    /// The org read itself decides whether selection can be skipped.
    static func canPrepareOrganization(
        _ state: FirstRunState, isCommitting: Bool, completed: Bool
    ) -> Bool {
        state.step == .join && canForward(state) && !isCommitting && !completed
    }

    /// Always "Start sharing": Join is the last step and has no Skip
    /// (owner, 2026-10-12: sign-in required). It stays off until a NEAR AI
    /// account is signed in (`canStart`).
    static func footerTitle(_ state: FirstRunState, copy: FirstRunCopy) -> String {
        copy.uses.start
    }

    /// Whether Join shows an account as signed into: near.ai or a passkey
    /// answered on this run, a known invite joined, or a held near.ai
    /// enrollment, and nothing signed out of. An enrollment the daemon holds
    /// with no invite or account to show (an earlier run's, a preset
    /// config's) draws no signed-in card.
    static func hasSignedInAccount(_ state: FirstRunState) -> Bool {
        if state.signedOutOfEnrolment { return false }
        switch state.account {
        case .nearAI, .passkeyChosen, .passkey: return true
        case .none, .watchOnly, .enrolled:
            return !(state.enrolledInvite ?? "").isEmpty || state.nearAIEnrolled
        }
    }

    /// No note: the body already says to sign in, and there is no Skip to
    /// explain (owner, 2026-10-12).
    static func footerNote(_ state: FirstRunState, copy: FirstRunCopy) -> String? {
        nil
    }

    /// Whether a NEAR AI account is signed in: one held, and near.ai's
    /// sign-in finished when that is the account. near.ai chosen and still
    /// signing in, or whose sign-in was cancelled, is not yet.
    static func canForward(_ state: FirstRunState) -> Bool {
        hasNearAIAccount(state) && (state.account != .nearAI || state.signedIn)
    }

    /// Start sharing: a NEAR AI account signed in (`canForward`), and what
    /// Start itself needs (`UsesScreenLayout.canStart`): the required use
    /// known, an enrollment to finish under, the sharing line present, and
    /// nothing committing.
    static func canStart(
        _ state: FirstRunState, uses: FirstRunCopy.Uses, requiredScope: ConsentScope?, grant: AutomaticGrantCopy?,
        isCommitting: Bool
    ) -> Bool {
        canForward(state)
            && UsesScreenLayout.canStart(
                readyToStart(state), uses: uses, requiredScope: requiredScope, grant: grant, isCommitting: isCommitting)
    }

    static func canAutomaticallyStart(
        _ state: FirstRunState, organizationReady: Bool, attempted: Bool, starting: Bool,
        uses: FirstRunCopy.Uses, requiredScope: ConsentScope?, grant: AutomaticGrantCopy?,
        isCommitting: Bool, completed: Bool
    ) -> Bool {
        organizationReady && !attempted && !starting
            && canPrepareOrganization(state, isCommitting: isCommitting, completed: completed)
            && canStart(state, uses: uses, requiredScope: requiredScope, grant: grant, isCommitting: isCommitting)
    }

    /// The state Start commits: a held near.ai enrollment with no account
    /// answered starts as that enrollment; with no NEAR AI account nothing
    /// changes (`canForward`).
    static func readyToStart(_ state: FirstRunState) -> FirstRunState {
        guard canForward(state) else { return state }
        var ready = state
        switch ready.account {
        case .nearAI, .passkey: break
        case .none, .watchOnly, .passkeyChosen, .enrolled: ready.account = .enrolled
        }
        return ready
    }

    /// Sign in with near.ai on Join, the last step: the daemon is running
    /// (Tools started it), so the button chooses near.ai and the sign-in runs
    /// at once (`CommitPoint.signIn`). A held account -- a signed-in near.ai
    /// or a passkey the daemon bound -- is not replaced; a passkey only
    /// chosen is.
    static func chooseNearAI(_ state: FirstRunState) -> FirstRunState {
        guard canToggleNearAI(state) else { return state }
        var chosen = state
        chosen.account = .nearAI
        return chosen
    }

    /// Signing in with near.ai needs no invite (owner, Ron's review of
    /// #1235): with one it signs in to the account the invite enrolls;
    /// without one it enrolls this Mac through the near.ai login
    /// (`FirstRunPlan`, `near_ai_account_enroll`). An earlier run's
    /// enrollment can have near.ai chosen beside it, which signs in to that
    /// enrollment's account (`signInNearAI`): the app needs a NEAR AI
    /// account (owner, 2026-10-12), so it must not be left with no way on.
    static func canToggleNearAI(_ state: FirstRunState) -> Bool {
        switch state.account {
        case .none, .watchOnly, .passkeyChosen, .enrolled: return true
        case .nearAI: return !state.signedIn
        case .passkey: return false
        }
    }

    /// near.ai chosen and not yet signed in: the choice is still undoable.
    static func nearAIChosen(_ state: FirstRunState) -> Bool {
        state.account == .nearAI && !state.signedIn
    }

    /// Nothing while a passkey is held: near.ai cannot be chosen over it.
    /// The sign-in runs as soon as near.ai is chosen, so there is no "once
    /// watching starts" to say (`near_ai_chosen` is unread).
    static func nearAILine(_ state: FirstRunState, copy: FirstRunCopy.Join) -> String? {
        if passkeyDone(state) { return nil }
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

    /// The line under Join's cards for the runner's failure: Start's, then
    /// the sign-in's -- the daemon not starting, a changed folder refused,
    /// the invite not checked or not joined. A near.ai sign-in or
    /// enrollment that cleared the choice is said under its card
    /// (`nearAINotice`), and a dead invite on the invite card, so neither
    /// is said twice.
    static func notice(
        for failure: FirstRunFailure?, in state: FirstRunState, copy: FirstRunCopy, onboarding: TCOnboardingCopy?,
        privateAI: PrivateInferenceCopy?
    ) -> String? {
        if let start = UsesScreenLayout.notice(for: failure, uses: copy.uses, privateAI: privateAI) { return start }
        if nearAINotice(state, failure: failure, copy: copy) != nil { return nil }
        if case .inviteDead? = failure { return nil }
        return FoldersScreenLayout.notice(for: failure, copy: copy, onboarding: onboarding)
    }

    /// Join offers near.ai alone (owner, 2026-10-12). The invite card is
    /// drawn only while an invite is held -- one opened from a link, or
    /// enrolled on an earlier run -- so what was joined is still shown.
    static func showsInviteCard(_ state: FirstRunState) -> Bool {
        holdsInvite(state)
    }

    /// The passkey card is drawn only while a passkey is held or chosen, so
    /// it can be named or undone; Join never offers to make one.
    static func showsPasskeyCard(_ state: FirstRunState) -> Bool {
        passkeyDone(state) || passkeyChosen(state)
    }

    /// An invite is held: pasted and found, or already enrolled.
    static func holdsInvite(_ state: FirstRunState) -> Bool {
        !state.invite.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || state.enrolledInvite != nil
    }

    static func nearAIAction(_ state: FirstRunState, copy: FirstRunCopy) -> String {
        copy.join.nearAiSignIn
    }

    /// "Signed in" is the daemon's fact, never the choice.
    static func showsSignedIn(_ state: FirstRunState) -> Bool {
        state.signedIn
    }

    /// "Look up": the host is read locally (`TCInvite.issuerHost`); the
    /// invite is looked up and joined by the daemon when the person signs in
    /// (`CommitPoint.signIn`). Something that is not an invite is refused and
    /// not kept, so it can never be enrolled.
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
    /// the near.ai card offers no action then. An earlier run's enrollment
    /// does offer it: it is how that person signs in (owner, 2026-10-12).
    static func showsNearAIAction(_ state: FirstRunState) -> Bool {
        !passkeyDone(state)
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
    /// the mark once it confirms, so any account can be chosen again. Every other
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

/// Join, the last step (Ron's 2026-10-21 canvas): the watched tools circling
/// the N, the title and body, the near.ai sign-in (and an invite or a
/// passkey already held), and Start sharing, which waits for a NEAR AI
/// account (owner, 2026-10-12: no Skip).
///
/// The daemon is running by now (Tools started it), so Sign in runs at once
/// (`CommitPoint.signIn`): the invite Join holds is looked up and joined,
/// then near.ai signs in to it, or without one enrolls this Mac through the
/// near.ai login. Start sharing goes through the sharing disclosures when
/// Automatic or Private AI asks for them, then commits `.start`. The passkey
/// sheets are presented by `firstRunPasskeySheets`, which the first-run host
/// mounts once for every step.
struct JoinScreen: View {
    @EnvironmentObject private var model: AppModel
    let copy: FirstRunCopy
    @ObservedObject var runner: FirstRunRunner
    let passkeyAccount: (any PasskeyAccount)?
    var issuerHost: (String) -> String? = JoinScreen.defaultIssuerHost

    /// The core's host reader, named so the test reaches the function the
    /// screen uses.
    nonisolated static func defaultIssuerHost(_ invite: String) -> String? { TCInvite.issuerHost(invite) }

    @State private var draft: String?
    @State private var refused = false
    @State private var disclosure: SharingDisclosureFlow?
    @State private var grant: AutomaticGrantCopy?
    @State private var grantRead = false
    /// A refusal decided before a Start that then failed, shown after the
    /// Start that succeeds.
    @State private var pendingRefusal: FirstRunFailure?
    @State private var starting = false
    @State private var signedInAt: Date?
    @State private var onboarding = TCOnboardingCopy.load()
    /// The referral words, for the optional code under NEAR AI's Sign in.
    @State private var referralWords = TCConsentCopy.referralCopyJSON().flatMap(ReferralCopy.decode(fromJSON:))
    @State private var checkedOrganization = false
    @State private var checkingOrganization = false
    @State private var automaticallyStarting = false
    @State private var singleOrganizationReady = false
    @State private var automaticStartAttempted = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var privateAI: PrivateInferenceCopy? { model.privateInferenceCopy }

    private var canPrepareOrganization: Bool {
        JoinScreenLayout.canPrepareOrganization(
            runner.state, isCommitting: runner.isCommitting, completed: runner.completed)
    }

    private var shouldAutomaticallyStart: Bool {
        JoinScreenLayout.canAutomaticallyStart(
            runner.state, organizationReady: singleOrganizationReady, attempted: automaticStartAttempted,
            starting: starting, uses: copy.uses,
            requiredScope: UsesScreenLayout.requiredScope(model.consentScopes),
            grant: grant, isCommitting: runner.isCommitting, completed: runner.completed)
    }

    var body: some View {
        Group {
            if checkingOrganization || automaticallyStarting
                || (canPrepareOrganization && !checkedOrganization)
            {
                GlassSpinner(standalone: true)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                joinFrame
            }
        }
        .onChange(of: runner.state.signedIn) { _, signedIn in
            if signedIn { signedInAt = Date() }
        }
        .task {
            // `enroll` re-reads the route, not status; the grant's
            // `connected` is read from status, so it is refreshed here.
            model.refreshStatus()
            model.refreshConsentOptions()
            model.refreshWitness()
            if !grantRead {
                grant = AutomaticGrantCopy.decode(
                    fromJSON: TCCoreCopy.automaticContributionCopyJSON(configDir: model.configDirectory))
                grantRead = true
            }
        }
        .glassModal(isPresented: Binding(get: { disclosure != nil }, set: { if !$0 { disclosure = nil } })) {
            if let grant {
                SharingDisclosureSheet(copy: copy, grant: grant, flow: $disclosure, onFinish: finish)
                    .environmentObject(model)
            }
        }
        .task(id: canPrepareOrganization && grantRead) {
            guard canPrepareOrganization, grantRead, !checkedOrganization else { return }
            checkedOrganization = true
            checkingOrganization = true
            let outcome = await model.prepareSingleOrganizationOnboarding()
            guard !Task.isCancelled, canPrepareOrganization else {
                checkingOrganization = false
                return
            }
            checkingOrganization = false
            if case .ready = outcome { singleOrganizationReady = true }
        }
        .task(id: shouldAutomaticallyStart) {
            guard shouldAutomaticallyStart else { return }
            // Metadata can arrive after the org read. Start once it is ready,
            // and leave retries or a cancelled disclosure to the person.
            automaticStartAttempted = true
            automaticallyStarting = true
            start()
        }
        .onChange(of: JoinScreenLayout.canForward(runner.state)) { _, ready in
            if !ready {
                checkedOrganization = false
                checkingOrganization = false
                automaticallyStarting = false
                singleOrganizationReady = false
                automaticStartAttempted = false
            }
        }
        .onChange(of: disclosure == nil) { _, closed in
            if closed && !starting && !runner.isCommitting {
                automaticallyStarting = false
            }
        }
    }

    private var joinFrame: some View {
        let required = UsesScreenLayout.requiredScope(model.consentScopes)
        return FirstRunFrame(
            copy: copy,
            state: $runner.state,
            isCommitting: runner.isCommitting,
            notice: JoinScreenLayout.notice(
                for: runner.failure, in: runner.state, copy: copy, onboarding: onboarding, privateAI: privateAI),
            footer: FirstRunFooter(
                title: JoinScreenLayout.footerTitle(runner.state, copy: copy),
                isEnabled: JoinScreenLayout.canStart(
                    runner.state, uses: copy.uses, requiredScope: required, grant: grant,
                    isCommitting: runner.isCommitting),
                // About taking the action, so directly above the action bar.
                note: copy.join.noSharing,
                busy: starting && runner.isCommitting,
                action: start)
        ) {
            orbit
        } title: {
            VStack(spacing: GlassTokens.Space.s3) {
                FirstRunTitle(light: copy.join.titleLight, bold: copy.join.titleBold)
                Text(copy.join.body)
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textSecondary)
                    .multilineTextAlignment(.center)
            }
        } content: {
            ScrollView {
                VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                    nearAICard
                    if JoinScreenLayout.showsNearAIAction(runner.state), let words = referralWords {
                        nearAIReferralField(words)
                    }
                    if runner.state.holdsEnrolment {
                        PayoutLinkView()
                    }
                    if JoinScreenLayout.showsInviteCard(runner.state) {
                        inviteCard
                    }
                    if JoinScreenLayout.showsPasskeyCard(runner.state) {
                        passkeyCard
                    }
                    if let notice = JoinScreenLayout.nearAINotice(runner.state, failure: runner.failure, copy: copy) {
                        GlassAlert(notice)
                    }
                    if let notice = runner.passkeyOutcome?.joinNotice(copy) {
                        GlassNotice(tone: .ask) { Text(notice) }
                    }
                }
                .frame(maxWidth: 460)
                .frame(maxWidth: .infinity)
            }
            .frame(maxWidth: 520)
            .scrollBounceBehavior(.basedOnSize)
        }
    }

    // MARK: - The graphic

    /// The watched tools circling the N on a slowly turning dashed ring; a
    /// sign-in sends a ring of light out from the N.
    private var orbit: some View {
        let watched = ToolsScreenLayout.watchedKinds(runner.state)
        return GlassAnimationTimeline(minimumInterval: 1.0 / 30, paused: reduceMotion) { date in
            let time = date.timeIntervalSinceReferenceDate
            let pulse = signedInAt.map { date.timeIntervalSince($0) / 1.6 }
            ZStack {
                Circle()
                    .strokeBorder(GlassTokens.Color.firstRunRule.color, style: StrokeStyle(lineWidth: 1, dash: [3, 4]))
                    .frame(width: 140, height: 140)
                    .rotationEffect(.degrees(reduceMotion ? 0 : (time / 90).truncatingRemainder(dividingBy: 1) * 360))
                if let pulse, pulse < 1, !reduceMotion {
                    Circle()
                        .fill(RadialGradient(
                            stops: [
                                .init(color: GlassTokens.Color.firstRunBeam.opacity(0).color, location: 0.26),
                                .init(color: GlassTokens.Color.firstRunBeam.opacity(0.6).color, location: 0.44),
                                .init(color: GlassTokens.Color.firstRunBeam.opacity(0).color, location: 0.64),
                            ],
                            center: .center, startRadius: 0, endRadius: 46))
                        .frame(width: 92, height: 92)
                        .blur(radius: 4)
                        .scaleEffect(0.3 + 1.6 * pulse)
                        .opacity(pulse < 0.18 ? pulse / 0.18 : (1 - pulse) / 0.82)
                }
                FirstRunHub()
                    .firstRunActor(FirstRunStageLayout.hubActor)
                    .frame(width: 56, height: 56)
                ForEach(Array(watched.enumerated()), id: \.element) { index, kind in
                    let point = FirstRunStageLayout.point(
                        CGPoint(x: 83, y: 83), radius: 70, degrees: FirstRunStageLayout.joinAngle(index, of: watched.count))
                    FirstRunToolFace(tool: kind.glassTool)
                        .firstRunActor(FirstRunStageLayout.tileActor(kind))
                        .frame(width: 26, height: 26)
                        .position(point)
                }
            }
            .frame(width: 166, height: 166)
        }
        .frame(height: 166)
        .frame(maxWidth: .infinity)
        .accessibilityHidden(true)
    }

    // MARK: - Cards

    private var inviteCard: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            if JoinScreenLayout.showsInviteField(runner.state) {
                // The field as it always reads; closed while an enrollment
                // the daemon holds came with no invite, since no second
                // invite is joined over it.
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
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .foregroundStyle(GlassColor.textPrimary)
            }
            inviteLine
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .firstRunCard(padding: EdgeInsets(top: 14, leading: 18, bottom: 14, trailing: 18))
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
            GlassAlert(line)
        case .note(let line):
            Text(line)
                .glassType(GlassTokens.TypeScale.label)
                .foregroundStyle(GlassColor.textSecondary)
        }
    }

    @ViewBuilder private var passkeyCard: some View {
        accountCard(
            title: copy.join.passkeyEyebrow,
            text: JoinScreenLayout.passkeyLine(runner.state, copy: copy.join),
            done: JoinScreenLayout.passkeyDone(runner.state) ? copy.join.passkeyDone : nil,
            chosen: JoinScreenLayout.passkeyDone(runner.state),
            showsAction: JoinScreenLayout.showsPasskeyAction(runner.state)
        ) {
            Button(action: passkeyAction) {
                // Only the undo of a choice an earlier build recorded carries
                // a glyph; starting the daemon for the sheets shows the
                // spinner.
                if JoinScreenLayout.passkeyChosen(runner.state) {
                    Label(JoinScreenLayout.passkeyAction(runner.state, copy: copy), systemImage: "arrow.uturn.backward")
                        .labelStyle(.titleAndIcon)
                } else {
                    HStack(spacing: GlassTokens.Space.s3) {
                        if runner.isCommitting, !starting { GlassSpinner() }
                        Text(JoinScreenLayout.passkeyAction(runner.state, copy: copy))
                    }
                }
            }
            .buttonStyle(GlassButtonStyle(.glass))
            .disabled(runner.isCommitting)
        }
        if let line = JoinScreenLayout.passkeyFailureLine(runner.failure, copy: copy.join) {
            GlassAlert(line)
        }
    }

    private var nearAICard: some View {
        accountCard(
            title: copy.join.nearAiEyebrow,
            text: JoinScreenLayout.nearAILine(runner.state, copy: copy.join),
            done: JoinScreenLayout.showsSignedIn(runner.state) ? copy.join.signedIn : nil,
            chosen: JoinScreenLayout.showsSignedIn(runner.state),
            showsAction: JoinScreenLayout.showsNearAIAction(runner.state)
        ) {
            HStack(spacing: GlassTokens.Space.s3) {
                if runner.signInWaiting {
                    Button(copy.passkey.cancel) { Task { await runner.cancelSignIn() } }
                        .buttonStyle(GlassButtonStyle(.secondary))
                }
                Button(action: signIn) {
                    HStack(spacing: GlassTokens.Space.s3) {
                        if runner.isCommitting, !starting, runner.state.account == .nearAI { GlassSpinner() }
                        Text(JoinScreenLayout.nearAIAction(runner.state, copy: copy))
                    }
                }
                .buttonStyle(GlassButtonStyle(.glass))
                .disabled(runner.isCommitting || !JoinScreenLayout.canToggleNearAI(runner.state))
            }
        }
    }

    /// The optional referral code under NEAR AI's Sign in (referral spec
    /// section 6): sent with the NEAR AI enrolment, never on its own. Its
    /// label and placeholder are the core's drafts (`ReferralCopy`, for owner
    /// review); nil words draw no field. A malformed code is said under the
    /// field, and one that does not apply never stops the sign-in.
    private func nearAIReferralField(_ words: ReferralCopy) -> some View {
        let problem = ReferralSurface(copy: words)
            .fieldProblem(runner.referralCode, display: TCConsentCopy.referralCodeDisplay)
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            GlassTextField(words.signUpLabel, text: $runner.referralCode,
                           prompt: words.fieldPlaceholder, invalid: problem != nil)
                .disabled(runner.isCommitting)
            if let problem {
                Text(problem)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassStatus.outside.textColor)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.horizontal, 18)
    }

    /// An account card: its title and line on the left, the action on the
    /// right until it is done, then its status; a held account is outlined
    /// in the accent. With the other account held there is no action to
    /// offer, so none is shown.
    private func accountCard<Action: View>(
        title: String, text: String?, done: String?, chosen: Bool, showsAction: Bool, @ViewBuilder action: () -> Action
    ) -> some View {
        HStack(spacing: GlassTokens.Space.s8) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                Text(title)
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .foregroundStyle(GlassColor.textPrimary)
                if let text {
                    Text(text)
                        .glassType(GlassTokens.TypeScale.label.weight(.regular))
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            // The text takes the row and the action keeps its own width, so
            // the text column can never squeeze it out of sight.
            .frame(maxWidth: .infinity, alignment: .leading)
            if let done {
                GlassStatusLabel(done, status: .on)
                    .fixedSize()
            } else if showsAction {
                action()
                    .fixedSize()
            }
        }
        .firstRunCard(padding: EdgeInsets(top: 14, leading: 18, bottom: 14, trailing: 18))
        .overlay(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .strokeBorder(chosen ? GlassTokens.Color.toggleOn.color : .clear, lineWidth: 1.5))
        .animation(GlassMotion.standard(reduceMotion), value: chosen)
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

    /// near.ai chosen and signed in at once: the daemon is running.
    private func signIn() {
        runner.state = JoinScreenLayout.chooseNearAI(runner.state)
        Task { await runner.commit(.signIn) }
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

    // MARK: - Start sharing

    /// Put the required use in the state, once it is known.
    private func includeRequired() {
        let included = UsesScreenLayout.includingRequired(
            runner.state, required: UsesScreenLayout.requiredScope(model.consentScopes))
        if included != runner.state { runner.state = included }
    }

    private func start() {
        runner.state = JoinScreenLayout.readyToStart(runner.state)
        // What Start sends matches what Uses showed: the required use
        // included.
        includeRequired()
        switch UsesScreenLayout.startRoute(runner.state) {
        case .disclose:
            disclosure = SharingDisclosureFlow()
        case .discloseWitness:
            disclosure = SharingDisclosureFlow(witnessOnly: true)
        case .commit:
            run { await UsesStart.plainStart(runner: runner, pending: $0) }
        }
    }

    /// Both disclosures seen: the core decides the grant, then Start runs.
    /// A not-ready answer finishes on Ask; its failure is shown after the
    /// commit, which clears failures when it begins.
    private func finish(_ flow: SharingDisclosureFlow) {
        // The Private AI path saw the witness disclosure and asks for no
        // grant: Start as on Ask.
        if flow.witnessOnly {
            run { await UsesStart.plainStart(runner: runner, pending: $0) }
            return
        }
        let request = SharingDisclosureFlow.grantRequest(
            flow.progress(connected: model.status.loggedIn, scopes: runner.state.scopes))
        run { await UsesStart.finish(runner: runner, request: request, pending: $0) }
    }

    private func run(_ commit: @escaping (FirstRunFailure?) async -> FirstRunFailure?) {
        starting = true
        Task {
            pendingRefusal = await commit(pendingRefusal)
            starting = false
            automaticallyStarting = false
        }
    }
}
