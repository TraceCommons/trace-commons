import Foundation
import TCShellCore

/// What `invite_lookup` answered: the issuer and pay range for Join's joined
/// line, the daemon's refusal label for the core's invite error, or that the
/// lookup never reached an answer (no daemon, an unreadable reply, the
/// issuer unreachable), which says nothing about the invite.
enum FirstRunLookup: Equatable {
    case found(DaemonData.InviteLookup)
    case refused(label: String)
    case unavailable
}

/// What `grant_automatic` answered.
enum FirstRunGrantAnswer: Equatable {
    case granted
    /// The daemon's refusal label, for example
    /// `automatic-grant-witness-changed` or `arming-terms-unavailable`.
    case refused(label: String)
}

/// What `near_ai_account_enroll` answered.
enum FirstRunNearAIEnrolment: Equatable {
    case enrolled
    /// The daemon's own label, which `TCNearAiEnroll.line(label:)` words.
    case refused(label: String)
}

/// The daemon calls a first run makes, one per `FirstRunCall`. Each answers
/// whether the daemon confirmed it; nothing here carries a sentence.
/// `AppModel` is the live one; tests record.
@MainActor
protocol FirstRunDaemon: AnyObject {
    func startDaemon(settingsJSON: String) async -> Bool
    func setSourceSettings(settingsJSON: String) async -> Bool
    func lookupInvite(_ invite: String) async -> FirstRunLookup
    func enrollInvite(_ invite: String) async -> Bool
    func signInNearAI() async -> Bool
    /// The near.ai login: true once the daemon keeps a near.ai session,
    /// after the browser sign-in when it kept none.
    func nearAILogin() async -> Bool
    /// End the browser sign-in `nearAILogin` is waiting on
    /// (`near_ai_credential_cancel`). True only once the daemon took it.
    func cancelNearAILogin() async -> Bool
    /// Enroll this Mac through that login, with no invite.
    func enrollNearAI() async -> FirstRunNearAIEnrolment
    func saveConsentScopes(_ scopes: [String]) async -> Bool
    func setProjectMode(projectID: String, mode: ProjectMode) async -> Bool
    func includePastSessions(projectID: String, sessionIDs: [String]) async -> Bool
    func setPrivateAI(_ on: Bool) async -> Bool
    func grantAutomatic(witness: String?) async -> FirstRunGrantAnswer
    /// True only once the completion marker is actually written. The marker
    /// is keyed by tenant, so with no tenant known this answers false.
    func markComplete() async -> Bool
    /// True only once the watch-only marker is actually written. It is keyed
    /// by the daemon's config directory, so without one this answers false.
    func markWatchOnlyComplete() async -> Bool
    /// The first run is finished: the marker was just written. `notice` is
    /// the core's sentence for what the person must still be told (a
    /// refused grant), or nil. Called synchronously after the marker, before
    /// the commit returns, because writing the marker ends onboarding and
    /// the first-run host, with its runner, leaves the screen at the next
    /// render; whoever is told must outlive it.
    func firstRunFinished(notice: String?)
}

/// Why a commit stopped. The screens map each case to a core sentence; the
/// labels are the daemon's own, never a message body.
enum FirstRunFailure: Equatable {
    /// The daemon did not start.
    case startFailed
    /// Create passkey on Join could not start the daemon its sheets need,
    /// so none opened and no account was made. Join says so.
    case passkeyUnavailable
    /// The running daemon refused a changed declaration (`set_settings`) on
    /// a later Continue. It keeps watching what it held before.
    case settingsFailed
    /// The invite was refused when it was looked up; the person is back on
    /// Join.
    case inviteDead(label: String)
    /// The invite could not be looked up just now; the step stays and a
    /// retry looks it up again.
    case lookupUnavailable
    case enrollFailed
    case signInFailed
    /// The near.ai enrolment without an invite was refused; the daemon's
    /// label, for the core's line (`TCNearAiEnroll`).
    case nearAIEnrollFailed(label: String)
    case scopesFailed
    /// A folder rule or a past-session include was not saved.
    case rulesFailed
    case privateAIFailed
    /// Automatic was refused; setup finished on Ask me.
    case grantRefused(label: String)
    /// Every call succeeded but the completion marker was not written, so
    /// the first run is not over. Outranks `grantRefused`.
    case completeFailed
}

/// Executes `FirstRunPlan`'s calls in order and records what the daemon now
/// holds back into the state. The plan is recomputed from the state at every
/// commit. At `.leaveRoots` a retry after a failure repeats only what did not
/// succeed; at `.start` it re-sends every call, each of which is idempotent
/// on the daemon, except that a refused grant is not asked for again.
@MainActor
final class FirstRunRunner: ObservableObject {
    @Published var state: FirstRunState
    @Published var failure: FirstRunFailure?
    /// The last invite the daemon looked up and found valid.
    @Published private(set) var lookup: DaemonData.InviteLookup?
    @Published private(set) var isCommitting = false
    /// The near.ai login without an invite is running, which can wait on
    /// the browser for up to `NearAILoginPoll.limit`: Folders and Tools
    /// offer Cancel beside the spinning Continue only while this holds.
    @Published private(set) var signInWaiting = false
    /// The login `signInWaiting` covers, so Cancel can end its wait.
    private var loginTask: Task<Bool, Never>?
    /// The daemon's answer to a Cancel, awaited before the commit ends so
    /// a Continue pressed straight after cannot start a sign-in the cancel
    /// then ends.
    private var loginCancel: Task<Bool, Never>?
    /// The passkey sheets are asked for: by Create passkey on Join, after
    /// starting the daemon when it was not running (`openPasskeyOnJoin`),
    /// or by the commit that started the daemon for a passkey an earlier
    /// build recorded as chosen. `firstRunPasskeySheets`, mounted on the
    /// first-run host, presents them on whichever step is on screen.
    @Published private(set) var passkeyDue = false
    /// How the passkey sheets last ended, for Join's notice
    /// (`PasskeySheetOutcome.joinNotice`). Cleared when they are asked for
    /// again.
    @Published private(set) var passkeyOutcome: PasskeySheetOutcome?
    /// The step the requested sheets open at: P-1 when Join asked, P-7 when
    /// the first run opened for a returning person (`offerWelcomeBack`).
    @Published private(set) var passkeyStart: PasskeySheetStep = .choose
    /// The remembered passkey's name P-7 shows, when the daemon has one.
    @Published private(set) var returningName: String?
    /// Welcome back is offered once per first run: dismissed, it does not
    /// come back when the daemon restarts or Join is shown again.
    private var welcomeBackOffered = false

    /// Start ran `markComplete`: the first run is finished, and a passkey
    /// sheet that ends afterwards does not reopen Join.
    @Published private(set) var completed = false

    /// The daemon refused the grant during the last commit. `failure` says
    /// so too, unless `completeFailed` outranked it; this keeps the refusal
    /// for the retry that finishes on Ask me.
    @Published private(set) var refusedGrant: FirstRunFailure?

    private let daemon: FirstRunDaemon
    /// The core's sentence for the refusal a finished first run carries
    /// (`UsesScreenLayout.finishedNotice`), from the host's copy.
    private let finishedNotice: (FirstRunFailure?) -> String?
    /// A refusal decided before this Start (`UsesStart`), handed over with
    /// the daemon's own if the marker is written.
    private var carriedRefusal: FirstRunFailure?

    init(
        state: FirstRunState, daemon: FirstRunDaemon,
        finishedNotice: @escaping (FirstRunFailure?) -> String? = { _ in nil }
    ) {
        self.state = state
        self.daemon = daemon
        self.finishedNotice = finishedNotice
    }

    /// Run the calls for `point`. Leaving the roots moves on to the next step
    /// only when every call succeeded; a dead invite, a near.ai sign-in that
    /// did not succeed (with an invite or without), or a near.ai enrolment
    /// without one that was refused, goes back to Join; any other failure
    /// leaves the step where it is, and so does a cancelled browser wait. Start always ends in
    /// `markComplete` unless a call before the grant failed, and reports
    /// `completeFailed` when the marker was not written. `refusal` is one
    /// decided before Start (`UsesStart`); once the marker is written it is
    /// handed over with any the daemon gave (`FirstRunDaemon.firstRunFinished`).
    func commit(_ point: CommitPoint, carrying refusal: FirstRunFailure? = nil) async {
        guard !isCommitting else { return }
        isCommitting = true
        defer { isCommitting = false }
        // Create passkey on Join takes back only its own failure: a refused
        // invite's line stays on Join beside it.
        if point != .passkeyOnJoin || failure == .passkeyUnavailable { failure = nil }
        refusedGrant = nil
        carriedRefusal = refusal
        defer { carriedRefusal = nil }

        // Continue is disabled without a declaration; should it be pressed
        // anyway, nothing can start, and that is said rather than swallowed.
        if point == .leaveRoots, state.sessionRoots.settingsJSON() == nil {
            failure = .startFailed
            return
        }
        for call in FirstRunPlan.calls(for: state, at: point) {
            guard await run(call) else {
                // On Join a failed start is the passkey's, not the
                // watcher's: no folder has been answered yet.
                if point == .passkeyOnJoin, failure == .startFailed { failure = .passkeyUnavailable }
                return
            }
        }
        if point == .leaveRoots {
            state = FirstRunNavigation.next(state)
        }
    }

    /// One call, its outcome recorded. False stops the commit.
    private func run(_ call: FirstRunCall) async -> Bool {
        switch call {
        case .startDaemon(let json):
            guard await daemon.startDaemon(settingsJSON: json) else { return fail(.startFailed) }
            state.daemonStarted = true
            state.startedSettingsJSON = json
        case .setSourceSettings(let changed):
            guard await daemon.setSourceSettings(settingsJSON: changed) else { return fail(.settingsFailed) }
            // The daemon merged the change, so it now holds the whole
            // current declaration, not just the part that was sent.
            state.startedSettingsJSON = state.sessionRoots.settingsJSON()
        case .lookupInvite(let invite):
            switch await daemon.lookupInvite(invite) {
            case .found(let found):
                lookup = found
            case .refused(let label):
                lookup = nil
                state = FirstRunNavigation.returnToJoin(afterDeadInvite: state)
                return fail(.inviteDead(label: label))
            case .unavailable:
                lookup = nil
                return fail(.lookupUnavailable)
            }
        case .enroll(let invite):
            guard await daemon.enrollInvite(invite) else { return fail(.enrollFailed) }
            state.enrolledInvite = invite
            state.signedOutOfEnrolment = false
        case .signInNearAI:
            // With an invite too (owner, 2026-10-07): back to Join with the
            // choice cleared and the invite and its enrolment kept.
            guard await daemon.signInNearAI() else {
                state = FirstRunNavigation.returnToJoin(afterNearAIFailure: state)
                return fail(.signInFailed)
            }
            state.signedIn = true
        case .nearAILogin:
            let daemon = self.daemon
            let login = Task { await daemon.nearAILogin() }
            loginTask = login
            signInWaiting = true
            let signedIn = await login.value
            signInWaiting = false
            loginTask = nil
            // The person cancelled: the step and the near.ai choice stay,
            // nothing is enrolled even if the sign-in finished meanwhile,
            // and the next Continue signs in afresh. A cancel the daemon
            // did not take still ended the wait here, and reads as the
            // sign-in that did not finish.
            if let cancel = loginCancel {
                loginCancel = nil
                if !(await cancel.value) { failure = .signInFailed }
                return false
            }
            // No Back to leave a sign-in that keeps failing: back to Join,
            // the choice cleared, which says why (`nearAINotice`).
            guard signedIn else {
                state = FirstRunNavigation.returnToJoin(afterNearAIFailure: state)
                return fail(.signInFailed)
            }
        case .enrollNearAI:
            switch await daemon.enrollNearAI() {
            case .enrolled:
                state.nearAIEnrolled = true
                state.signedIn = true
                state.signedOutOfEnrolment = false
            case .refused(let label):
                state = FirstRunNavigation.returnToJoin(afterNearAIFailure: state)
                return fail(.nearAIEnrollFailed(label: label))
            }
        case .openPasskeySheets:
            // The person's ceremony, not awaited: the commit moves on and
            // the sheets record their outcome when they end.
            requestPasskey()
        case .setConsentScopes(let scopes):
            guard await daemon.saveConsentScopes(scopes) else { return fail(.scopesFailed) }
        case .setProjectMode(let projectID, let mode):
            guard await daemon.setProjectMode(projectID: projectID, mode: mode) else { return fail(.rulesFailed) }
        case .includePastSessions(let projectID, let sessions):
            guard await daemon.includePastSessions(projectID: projectID, sessionIDs: sessions)
            else { return fail(.rulesFailed) }
        case .setPrivateAI(let on):
            guard await daemon.setPrivateAI(on) else { return fail(.privateAIFailed) }
        case .grantAutomatic(let witness):
            // The one call that does not stop the run: a refused grant
            // finishes on Ask me, and says so.
            if case .refused(let label) = await daemon.grantAutomatic(witness: witness) {
                state.sharing = .askMe
                failure = .grantRefused(label: label)
                refusedGrant = failure
            }
        case .markComplete:
            guard await daemon.markComplete() else { return fail(.completeFailed) }
            finish()
        case .markWatchOnlyComplete:
            guard await daemon.markWatchOnlyComplete() else { return fail(.completeFailed) }
            finish()
        }
        return true
    }

    /// Cancel on Folders or Tools while the near.ai browser sign-in runs:
    /// the daemon is asked to end its attempt, and the wait ends here
    /// whatever it answers (fail closed); the commit stops once the daemon
    /// has answered. Nothing to cancel, nothing done.
    func cancelSignIn() async {
        guard signInWaiting, loginCancel == nil, let login = loginTask else { return }
        signInWaiting = false
        let daemon = self.daemon
        let cancel = Task { await daemon.cancelNearAILogin() }
        loginCancel = cancel
        login.cancel()
        _ = await cancel.value
    }

    /// Create passkey on Join: the sheets open over Join, as in #1030, with
    /// the daemon started first (watching nothing) when it is not running
    /// (`CommitPoint.passkeyOnJoin`). A start that fails opens nothing and
    /// reads `passkeyUnavailable`; Join stays.
    func openPasskeyOnJoin() async {
        await commit(.passkeyOnJoin)
    }

    /// Ask for the passkey sheets, at P-1.
    func requestPasskey() {
        passkeyStart = .choose
        returningName = nil
        passkeyOutcome = nil
        passkeyDue = true
    }

    /// Ron's P-7 for a returning person (`FirstRunNavigation.opensWelcomeBack`):
    /// asks the daemon which passkeys this Mac remembers and, if the rule
    /// holds, opens the sheets at Welcome back with the remembered name. The
    /// rule is checked again once the daemon answers, since Join (or an
    /// enrolment the first status reported) can change meanwhile. Offered at
    /// most once; its Sign in is the ordinary sign-in, and "Other sign-in
    /// options" closes it and leaves Join as it was.
    func offerWelcomeBack(from account: any PasskeyAccount) async {
        guard !welcomeBackOffered, !passkeyDue,
            FirstRunNavigation.mayOfferWelcomeBack(state, completed: completed)
        else { return }
        welcomeBackOffered = true
        let passkeys = await account.passkeyState()
        guard !passkeyDue, FirstRunNavigation.opensWelcomeBack(state, passkeys: passkeys, completed: completed)
        else { return }
        passkeyStart = .welcomeBack
        returningName = passkeys?.rememberedName
        passkeyOutcome = nil
        passkeyDue = true
    }

    /// The passkey sheets ended: record their outcome (`JoinScreenLayout.apply`)
    /// and lower the request, whatever the outcome, so a closed sheet is
    /// asked again only by the next commit or the button.
    ///
    /// The sheets open after Folders or Tools, so a sign-out (Verify
    /// cancelled) can end them on a later step. It clears every sign-in, the
    /// invite included, so the person goes back to Join, which says why;
    /// every other answer is kept.
    /// After Start the first run is finished and stays where it is.
    func finishPasskey(_ outcome: PasskeySheetOutcome, copy: FirstRunCopy) {
        state = JoinScreenLayout.apply(outcome, to: state, copy: copy).state
        // A sign-out clears the invite (`JoinScreenLayout.signOut`), so its
        // lookup goes with it: no joined line outlives it.
        if outcome == .signedOut { lookup = nil }
        if outcome == .signedOut, !completed { state.step = .join }
        passkeyOutcome = outcome
        passkeyDue = false
    }

    /// The marker is written. No suspension point separates this from the
    /// write, so the notice is handed over before the host can go.
    private func finish() {
        completed = true
        daemon.firstRunFinished(notice: finishedNotice(refusedGrant ?? carriedRefusal))
    }

    private func fail(_ reason: FirstRunFailure) -> Bool {
        failure = reason
        return false
    }
}
