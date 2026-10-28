import Foundation

/// The animated first run's steps and the rules for moving between them
/// (Ron's 2026-10-21 canvas; #1030 `ftux-model.ts` before it). Every
/// function returns a new state and keeps every answer; only `step` moves
/// (and Back clears `grantReady`).
public enum FirstRunNavigation {
    /// One order for everyone: there is no Quick or Custom setup any more.
    /// Customize on Tools opens the per-repo rules in place, and signing in
    /// is the last step.
    public static let steps: [FirstRunStep] = [.welcome, .tools, .uses, .traces, .privateAI, .join]

    /// The steps the step thread names: every one after Welcome.
    public static var threadSteps: [FirstRunStep] { Array(steps.dropFirst()) }

    /// The following step, or the same state on the last one.
    public static func next(_ state: FirstRunState) -> FirstRunState {
        move(state, by: 1)
    }

    /// The previous step, or the same state on the first one. Answers and the
    /// daemon's progress are kept, so going forward again neither starts nor
    /// enrolls twice. The core's ready answer for a grant is not: it belongs
    /// to the Start it was asked for.
    public static func back(_ state: FirstRunState) -> FirstRunState {
        var moved = move(state, by: -1)
        moved.grantReady = false
        return moved
    }

    /// Back to a step already passed, picked on the step thread: as Back,
    /// every answer kept and the grant's ready answer cleared. A step not
    /// yet reached is not a way forward; the state is returned unchanged.
    public static func back(to step: FirstRunStep, from state: FirstRunState) -> FirstRunState {
        guard let target = steps.firstIndex(of: step), let current = steps.firstIndex(of: state.step), target < current
        else { return state }
        var moved = state
        moved.step = step
        moved.grantReady = false
        return moved
    }

    /// Ron's P-7, "Welcome back" (#1030 spec: "A returning user starts at
    /// P-7"; review of #1235 item 4): the passkey sheets open at Welcome
    /// back once the first run reaches Join, the last step, with no account
    /// answered, nobody is signed in
    /// (`passkey_state` says `none`), and the daemon remembers a passkey used
    /// on this Mac. Anything the daemon cannot say (`unknown`, a null count,
    /// no answer) opens Join as before; so does a finished first run, a held
    /// enrollment, and any answer already given on Join.
    public static func opensWelcomeBack(
        _ state: FirstRunState, passkeys: NativePasskeyState?, completed: Bool
    ) -> Bool {
        guard mayOfferWelcomeBack(state, completed: completed), let passkeys else { return false }
        return passkeys.state == "none" && (passkeys.passkeyCount ?? 0) > 0
    }

    /// The part of `opensWelcomeBack` the first run decides alone, before
    /// the daemon is asked: on Join, unfinished, no account answered.
    public static func mayOfferWelcomeBack(_ state: FirstRunState, completed: Bool) -> Bool {
        !completed && state.step == .join && state.account == .none
    }

    /// The invite was rejected when it was finally looked up: back to Join,
    /// every answer kept. The daemon is still running, so the next
    /// `signIn` does not start it again, and an earlier enrollment the
    /// daemon holds is kept too.
    public static func returnToJoin(afterDeadInvite state: FirstRunState) -> FirstRunState {
        var returned = state
        returned.step = .join
        return returned
    }

    /// The near.ai login did not finish, or the enrollment through it was
    /// refused (Kristi's review of #1261), or, with an invite, the sign-in to
    /// the account it enrolled did not go through (owner, 2026-10-07); the
    /// invite and its enrollment are kept. The person is on Join, where the
    /// sign-in runs, with the near.ai choice cleared, to choose again. Every other answer is kept,
    /// as after a dead invite, and the daemon is not started again. Clearing
    /// the choice also means a near.ai session the daemon still keeps is
    /// only reused once near.ai is chosen again.
    public static func returnToJoin(afterNearAIFailure state: FirstRunState) -> FirstRunState {
        var returned = returnToJoin(afterDeadInvite: state)
        if returned.account == .nearAI { returned.account = .none }
        return returned
    }

    /// Whether the current step's Continue (or Start sharing, on Join) is
    /// enabled.
    ///
    /// - Welcome, Traces, Private AI: always; nothing there is required.
    /// - Tools: something watched (Ron, 2026-10-22: every check starts
    ///   unticked, and an unticked tool is Not used, so there is nothing
    ///   left unanswered to wait on), no tool watched in two rows, and a
    ///   declaration the daemon would start with. A tool missing from this
    ///   Mac is not asked (spec rule 1).
    /// - Uses: the required use known, so nobody passes Uses without
    ///   seeing it. It is not waited on: it is always included
    ///   (`FirstRunState.includingRequiredScope`, owner 2026-10-08).
    /// - Join: the required use known, and something Start can do: finish
    ///   watching only (not while the daemon holds an enrollment, which
    ///   watching only cannot finish under), reopen a chosen passkey's
    ///   sheets, or finish an enrollment the daemon holds. Join itself also
    ///   waits for a NEAR AI account (`JoinScreenLayout`).
    public static func canContinue(
        _ state: FirstRunState,
        candidates: [SourceCandidate],
        requiredScope: String?
    ) -> Bool {
        switch state.step {
        case .welcome, .traces, .privateAI:
            return true
        case .tools:
            return state.watchesAnything && state.watchedTwice.isEmpty && state.sessionRoots.settingsJSON() != nil
        case .uses:
            // A Sharing choice is the person's to pick; none is preselected.
            return requiredScope != nil && state.sharingChosen
        case .join:
            guard requiredScope != nil else { return false }
            let watchesOnly = state.account == .watchOnly && !state.daemonHoldsEnrolment
            // Watching only shares nothing, so it needs no Sharing choice.
            guard watchesOnly || state.sharingChosen else { return false }
            return watchesOnly || state.account == .passkeyChosen || state.holdsEnrolment
        }
    }

    /// Whether this first run can share automatically: only with an
    /// enrollment the daemon holds (`FirstRunState.holdsEnrolment`). Watching
    /// only, no answer, a passkey chosen but not created, and near.ai whose
    /// invite has not enrolled cannot.
    public static func canChooseAutomatic(_ state: FirstRunState) -> Bool {
        state.holdsEnrolment
    }

    /// The sharing paths Uses offers: Ask, then Automatic, with no tag;
    /// neither is chosen for the person (owner, 2026-10-27).
    /// Uses comes before sign-in, and sign-in is required (owner,
    /// 2026-10-12), so Automatic is offered there and said to turn on once
    /// the person signs in (Ron's 2026-10-21 canvas). Start on Join grants
    /// it only with an enrollment the daemon holds (`canChooseAutomatic`),
    /// after both disclosures.
    public static func sharingPaths(for state: FirstRunState) -> [SharingPath] {
        [.askMe, .automatic]
    }

    private static func move(_ state: FirstRunState, by offset: Int) -> FirstRunState {
        guard let index = steps.firstIndex(of: state.step) else { return state }
        let target = index + offset
        guard steps.indices.contains(target) else { return state }
        var moved = state
        moved.step = steps[target]
        return moved
    }
}
