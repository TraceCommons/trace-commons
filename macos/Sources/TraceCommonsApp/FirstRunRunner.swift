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
    /// `first_run_estimate`: the commons' dollar estimate per accepted
    /// trace for Traces. Nil when it could not be asked.
    func firstRunEstimate() async -> DaemonData.FirstRunEstimate?
    func enrollInvite(_ invite: String) async -> Bool
    func signInNearAI() async -> Bool
    /// The near.ai login: true once the daemon keeps a near.ai session,
    /// after the browser sign-in when it kept none.
    func nearAILogin() async -> Bool
    /// End the browser sign-in `nearAILogin` is waiting on
    /// (`near_ai_credential_cancel`). True only once the daemon took it.
    func cancelNearAILogin() async -> Bool
    /// Enroll this Mac through that login, with no invite, carrying the
    /// referral code the person typed, if any (referral spec section 6).
    /// A code that does not apply never fails the enrolment.
    func enrollNearAI(referralCode: String?) async -> FirstRunNearAIEnrolment
    func setAskEachTime() async -> Bool
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
    /// `unenroll`: true only once the daemon confirmed it dropped this
    /// Mac's enrollment (or held none).
    func unenroll() async -> Bool
    /// The first run is finished: the marker was just written. `notice` is
    /// the core's sentence for what the person must still be told (a
    /// refused grant), or nil. Called synchronously after the marker, before
    /// the commit returns, because writing the marker ends onboarding and
    /// the first-run host, with its runner, leaves the screen at the next
    /// render; whoever is told must outlive it.
    func firstRunFinished(notice: String?)
}

extension FirstRunDaemon {
    /// A daemon that cannot say: Traces words its estimate in points.
    func firstRunEstimate() async -> DaemonData.FirstRunEstimate? { nil }
}

/// Why a commit stopped. The screens map each case to a core sentence; the
/// labels are the daemon's own, never a message body.
enum FirstRunFailure: Equatable {
    /// The daemon did not start.
    case startFailed
    /// The running daemon refused a changed declaration (`set_settings`) on
    /// a later commit. It keeps watching what it held before.
    case settingsFailed
    /// The invite was refused when it was looked up on Join.
    case inviteDead(label: String)
    /// The invite could not be looked up just now; Join stays and a retry
    /// looks it up again.
    case lookupUnavailable
    case enrollFailed
    case signInFailed
    /// The near.ai enrollment without an invite was refused; the daemon's
    /// label, for the core's line (`TCNearAiEnroll`).
    case nearAIEnrollFailed(label: String)
    case scopesFailed
    case sharingFailed
    /// A folder rule or a past-session include was not saved.
    case rulesFailed
    case privateAIFailed
    /// Automatic was refused; setup finished on Ask.
    case grantRefused(label: String)
    /// Every call succeeded but the completion marker was not written, so
    /// the first run is not over. Outranks `grantRefused`.
    case completeFailed
}

/// Executes `FirstRunPlan`'s calls in order and records what the daemon now
/// holds back into the state. The plan is recomputed from the state at every
/// commit. At `.leaveTools`, `.declareRoots` and `.signIn` a retry after a
/// failure repeats only what did not succeed; at `.start` it re-sends every
/// call, each of which is idempotent on the daemon, except that a refused
/// grant is not asked for again.
@MainActor
final class FirstRunRunner: ObservableObject {
    @Published var state: FirstRunState
    @Published var failure: FirstRunFailure?
    /// The last invite the daemon looked up and found valid.
    @Published private(set) var lookup: DaemonData.InviteLookup?
    /// The commons' dollar estimate for Traces, once the started daemon
    /// answered `first_run_estimate`; nil before, and Traces words points.
    @Published private(set) var dollarEstimate: DaemonData.FirstRunEstimate?
    private var dollarEstimateAsked = false
    @Published private(set) var isCommitting = false
    /// The near.ai login without an invite is running, which can wait on
    /// the browser for up to `NearAILoginPoll.limit`: Join offers Cancel
    /// beside the spinning Sign in only while this holds.
    @Published private(set) var signInWaiting = false
    /// The optional referral code on Join's NEAR AI sign-in, as typed. Sent
    /// with the NEAR AI enrolment only; the daemon checks it.
    @Published var referralCode = ""
    /// The login `signInWaiting` covers, so Cancel can end its wait.
    private var loginTask: Task<Bool, Never>?
    /// The daemon's answer to a Cancel, awaited before the commit ends so
    /// a Continue pressed straight after cannot start a sign-in the cancel
    /// then ends.
    private var loginCancel: Task<Bool, Never>?
    /// Start ran `markComplete`: the first run is finished.
    @Published private(set) var completed = false

    /// The daemon refused the grant during the last commit. `failure` says
    /// so too, unless `completeFailed` outranked it; this keeps the refusal
    /// for the retry that finishes on Ask.
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

    /// Run the calls for `point`. Leaving Tools moves on to the next step
    /// only when every call succeeded. On Join a dead invite, a near.ai
    /// sign-in that did not succeed (with an invite or without), or a
    /// near.ai enrollment without one that was refused clears the near.ai
    /// choice (`returnToJoin`); any other failure leaves the step where it
    /// is, and so does a cancelled browser wait. Start always ends in
    /// `markComplete` unless a call before the grant failed, and reports
    /// `completeFailed` when the marker was not written. `refusal` is one
    /// decided before Start (`UsesStart`); once the marker is written it is
    /// handed over with any the daemon gave (`FirstRunDaemon.firstRunFinished`).
    func commit(_ point: CommitPoint, carrying refusal: FirstRunFailure? = nil) async {
        guard !isCommitting else { return }
        isCommitting = true
        defer { isCommitting = false }
        failure = nil
        refusedGrant = nil
        carriedRefusal = refusal
        defer { carriedRefusal = nil }

        // Continue is disabled without a declaration; should it be pressed
        // anyway, nothing can start, and that is said rather than swallowed.
        if Self.declaresRoots(point), state.sessionRoots.settingsJSON() == nil {
            failure = .startFailed
            return
        }
        for call in FirstRunPlan.calls(for: state, at: point) {
            guard await run(call) else { return }
        }
        if point == .leaveTools {
            state = FirstRunNavigation.next(state)
        }
    }

    /// Ask the running daemon for the commons' dollar estimate, once per
    /// first run: Traces calls this as it opens. Nothing before the daemon
    /// starts; an unanswered ask is not repeated, and Traces stays on points.
    func loadDollarEstimate() async {
        guard state.daemonStarted, !dollarEstimateAsked else { return }
        dollarEstimateAsked = true
        dollarEstimate = await daemon.firstRunEstimate()
    }

    /// The commits that send the session roots, and so need a declaration.
    static func declaresRoots(_ point: CommitPoint) -> Bool {
        switch point {
        case .declareRoots, .leaveTools, .signIn: return true
        case .start: return false
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
            // choice cleared and the invite and its enrollment kept.
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
            switch await daemon.enrollNearAI(referralCode: Self.sentReferralCode(referralCode)) {
            case .enrolled:
                state.nearAIEnrolled = true
                state.signedIn = true
                state.signedOutOfEnrolment = false
            case .refused(let label):
                state = FirstRunNavigation.returnToJoin(afterNearAIFailure: state)
                return fail(.nearAIEnrollFailed(label: label))
            }
        case .setAskEachTime:
            guard await daemon.setAskEachTime() else { return fail(.sharingFailed) }
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
            // finishes on Ask, and says so.
            if case .refused(let label) = await daemon.grantAutomatic(witness: witness) {
                guard await daemon.setAskEachTime() else { return fail(.sharingFailed) }
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

    /// Cancel on Join while the near.ai browser sign-in runs:
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

    /// The referral code the enrolment carries: nil for an empty field, the
    /// typed text otherwise, which the daemon normalizes and checks.
    nonisolated static func sentReferralCode(_ typed: String) -> String? {
        let trimmed = typed.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
}
