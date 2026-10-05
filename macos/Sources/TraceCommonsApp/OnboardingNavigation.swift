import Foundation
import TCShellCore

/// The host-side rules for Ron's first run (#1030): where a host starts it,
/// when a window shows it, and what an invite link does to it. The rules
/// between its steps are `FirstRunNavigation`'s; the daemon calls are
/// `FirstRunPlan`'s, run by `FirstRunRunner`.
enum OnboardingNavigation {
    /// The first run's steps, under the name the hosts use.
    typealias Step = FirstRunStep

    /// The state a host starts the first run in: at `startAt`, in the tier
    /// that step belongs to (Tools and Rules are Custom's).
    ///
    /// A daemon already running (a returning person whose first run did not
    /// finish) is recorded as started, so it is not started again; what it
    /// holds is unknown here, so `startedSettingsJSON` stays nil and the
    /// whole declaration is re-sent. A daemon already enrolled is recorded
    /// too (`recordEnrolment`).
    static func initialState(startAt: Step, daemonRunning: Bool, enrolled: Bool) -> FirstRunState {
        let tier: FirstRunTier = (startAt == .tools || startAt == .rules) ? .custom : .quick
        let state = FirstRunState(tier: tier, step: startAt, daemonStarted: daemonRunning)
        return enrolled ? recordEnrolment(state) : state
    }

    /// The daemon holds an enrolment this first run did not make: an
    /// earlier first run joined and was quit before Start. Its invite (whose
    /// last use may be spent) is recorded as enrolled, so it is neither
    /// looked up nor joined again and Join's field is read-only, and with no
    /// account chosen here (or watch only) the account is the enrolment, so
    /// Join reads Continue and Automatic is offered. An account Join chose is
    /// kept, as is an enrolment this first run made.
    static func recordEnrolment(_ state: FirstRunState) -> FirstRunState {
        guard state.enrolledInvite == nil else { return state }
        var recorded = state
        recorded.enrolledInvite = ""
        if recorded.account == .none || recorded.account == .watchOnly {
            recorded.account = .enrolled
        }
        return recorded
    }

    /// Whether a window shows the first run rather than the core's startup
    /// notice or its own content.
    ///
    /// While the core is still starting, the notice. A refusal at launch,
    /// before the first run was shown, is the notice's too. A refusal from
    /// inside the first run (Folders or Tools started the daemon and it
    /// failed) keeps the first run on screen: that step shows the core's
    /// watcher line and keeps every answer, the invite included.
    static func hostsFirstRun(startup: AppModel.Startup, requiresOnboarding: Bool, entered: Bool) -> Bool {
        guard requiresOnboarding else { return false }
        switch startup {
        case .starting: return false
        case .needsRoots, .running: return true
        case .refused: return entered
        }
    }

    /// What a host does with an invite link (`PendingInvite`).
    enum InviteLinkAction: Equatable {
        /// Leave it parked, for a later moment or another host.
        case leaveParked
        /// Take it and apply nothing: it is not an invite, or the daemon
        /// enrolled one already.
        case discard
        /// Take it and put this state and failure on the runner.
        case apply(FirstRunState, FirstRunFailure?)
    }

    /// An invite link received while the first run is up: Join's Look up
    /// applied to it, and Join brought up, every other answer kept. It
    /// fills the field and goes no further; joining stays the person's
    /// decision, made when Folders or Tools commits.
    ///
    /// Left parked while a commit runs (the runner moves the step on from
    /// wherever the state is when its calls end), and by a host that does
    /// not take links at all (`hostTakesInvites` false: Private AI's
    /// Folders step, whose runner is thrown away when the person leaves
    /// that section, so a link it took would never reach the first run).
    /// Discarded once the daemon enrolled an invite (Join's field is then
    /// read-only), and when it is not an invite.
    static func receive(
        invite: String, in state: FirstRunState, failure: FirstRunFailure?, isCommitting: Bool,
        hostTakesInvites: Bool = true, host: (String) -> String?
    ) -> InviteLinkAction {
        guard hostTakesInvites, !isCommitting else { return .leaveParked }
        guard JoinLayout.inviteIsEditable(state) else { return .discard }
        let looked = JoinLayout.lookUp(invite, in: state, failure: failure, host: host)
        guard looked.outcome == .found else { return .discard }
        var received = looked.state
        received.step = .join
        return .apply(received, looked.failure)
    }
}
