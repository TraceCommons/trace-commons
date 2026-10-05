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
    /// whole declaration is re-sent.
    static func initialState(startAt: Step, daemonRunning: Bool) -> FirstRunState {
        let tier: FirstRunTier = (startAt == .tools || startAt == .rules) ? .custom : .quick
        return FirstRunState(tier: tier, step: startAt, daemonStarted: daemonRunning)
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

    /// An invite link (`PendingInvite`) received while the first run is up:
    /// Join's Look up applied to it, and Join brought up, every other answer
    /// kept. It fills the field and goes no further; joining stays the
    /// person's decision, made when Folders or Tools commits.
    ///
    /// Nil while a commit runs (the runner moves the step on from wherever
    /// the state is when its calls end), once the daemon enrolled an invite
    /// (Join's field is then read-only), and for something that is not an
    /// invite. The host leaves the link parked while a commit runs and
    /// otherwise takes it, applied or not.
    static func receive(
        invite: String, in state: FirstRunState, failure: FirstRunFailure?, isCommitting: Bool,
        host: (String) -> String?
    ) -> (state: FirstRunState, failure: FirstRunFailure?)? {
        guard !isCommitting, JoinLayout.inviteIsEditable(state) else { return nil }
        let looked = JoinLayout.lookUp(invite, in: state, failure: failure, host: host)
        guard looked.outcome == .found else { return nil }
        var received = looked.state
        received.step = .join
        return (received, looked.failure)
    }
}
