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
    func saveConsentScopes(_ scopes: [String]) async -> Bool
    func setProjectMode(projectID: String, mode: ProjectMode) async -> Bool
    func includePastSessions(projectID: String, sessionIDs: [String]) async -> Bool
    func setPrivateAI(_ on: Bool) async -> Bool
    func grantAutomatic(witness: String?) async -> FirstRunGrantAnswer
    /// True only once the completion marker is actually written. The marker
    /// is keyed by tenant, so with no tenant known this answers false.
    func markComplete() async -> Bool
}

/// Why a commit stopped. The screens map each case to a core sentence; the
/// labels are the daemon's own, never a message body.
enum FirstRunFailure: Equatable {
    /// The daemon did not start, or did not take a changed declaration.
    case startFailed
    /// The invite was refused when it was looked up; the person is back on
    /// Join.
    case inviteDead(label: String)
    /// The invite could not be looked up just now; the step stays and a
    /// retry looks it up again.
    case lookupUnavailable
    case enrollFailed
    case signInFailed
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

    private let daemon: FirstRunDaemon

    init(state: FirstRunState, daemon: FirstRunDaemon) {
        self.state = state
        self.daemon = daemon
    }

    /// Run the calls for `point`. Leaving the roots moves on to the next step
    /// only when every call succeeded; a dead invite goes back to Join; any
    /// other failure leaves the step where it is. Start always ends in
    /// `markComplete` unless a call before the grant failed, and reports
    /// `completeFailed` when the marker was not written.
    func commit(_ point: CommitPoint) async {
        guard !isCommitting else { return }
        isCommitting = true
        defer { isCommitting = false }
        failure = nil

        // Continue is disabled without a declaration; should it be pressed
        // anyway, nothing can start, and that is said rather than swallowed.
        if point == .leaveRoots, state.sessionRoots.settingsJSON() == nil {
            failure = .startFailed
            return
        }
        for call in FirstRunPlan.calls(for: state, at: point) {
            guard await run(call) else { return }
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
            guard await daemon.setSourceSettings(settingsJSON: changed) else { return fail(.startFailed) }
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
        case .signInNearAI:
            guard await daemon.signInNearAI() else { return fail(.signInFailed) }
            state.signedIn = true
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
            }
        case .markComplete:
            guard await daemon.markComplete() else { return fail(.completeFailed) }
        }
        return true
    }

    private func fail(_ reason: FirstRunFailure) -> Bool {
        failure = reason
        return false
    }
}
