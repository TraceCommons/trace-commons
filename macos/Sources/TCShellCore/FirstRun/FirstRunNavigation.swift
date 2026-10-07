import Foundation

/// Ron's step lists and the rules for moving between them (#1030
/// `ftux-model.ts`). Every function returns a new state and keeps every
/// answer; only `step` and `tier` move (and Back clears `grantReady`).
public enum FirstRunNavigation {
    public static func steps(for tier: FirstRunTier) -> [FirstRunStep] {
        switch tier {
        case .quick: return [.join, .folders, .uses]
        case .custom: return [.join, .tools, .rules, .uses]
        }
    }

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

    /// Ron's P-7, "Welcome back" (#1030 spec: "A returning user starts at
    /// P-7"; review of #1235 item 4): the first run opens at Welcome back
    /// when it is on Join with no account answered, nobody is signed in
    /// (`passkey_state` says `none`), and the daemon remembers a passkey used
    /// on this Mac. Anything the daemon cannot say (`unknown`, a null count,
    /// no answer) opens Join as before; so does a finished first run, a held
    /// enrolment, and any answer already given on Join.
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

    /// Switch tiers, keeping the person on the equivalent screen: Folders and
    /// Tools ask the same thing, and Quick has no Rules.
    public static func switchTier(_ state: FirstRunState, to tier: FirstRunTier) -> FirstRunState {
        var switched = state
        switched.tier = tier
        switch (tier, state.step) {
        case (.custom, .folders): switched.step = .tools
        case (.quick, .tools), (.quick, .rules): switched.step = .folders
        default: break
        }
        return switched
    }

    /// The invite was rejected when it was finally looked up: back to Join,
    /// every answer kept. The daemon is still running, so the next
    /// `leaveRoots` does not start it again, and an earlier enrolment the
    /// daemon holds is kept too.
    public static func returnToJoin(afterDeadInvite state: FirstRunState) -> FirstRunState {
        var returned = state
        returned.step = .join
        return returned
    }

    /// Whether the current step's Continue (or Start, on Uses) is enabled.
    ///
    /// - Join: an account answer, which may be "watch only".
    /// - Folders / Tools: every tool found on this Mac answered (a missing
    ///   one is not asked, spec rule 1), every added folder answered, no
    ///   tool watched in two rows, and a declaration the daemon would start
    ///   with.
    /// - Rules: always; every choice there is optional.
    /// - Uses: the required use ticked, and something Start can do: finish
    ///   watching only, reopen a chosen passkey's sheets, or finish an
    ///   enrolment the daemon holds. With no required use known, Start stays
    ///   disabled.
    public static func canContinue(
        _ state: FirstRunState,
        candidates: [SourceCandidate],
        requiredScope: String?
    ) -> Bool {
        switch state.step {
        case .join:
            return state.account != .none
        case .folders, .tools:
            let roots = state.sessionRoots
            let everyOfferedAnswered = candidates.filter(\.exists).allSatisfy { roots[$0.source].isAnswered }
            return everyOfferedAnswered && state.everyAddedFolderAnswered && state.watchedTwice.isEmpty
                && roots.settingsJSON() != nil
        case .rules:
            return true
        case .uses:
            guard let requiredScope else { return false }
            let startable = state.account == .watchOnly || state.account == .passkeyChosen || state.holdsEnrolment
            return startable && state.scopes.contains(requiredScope)
        }
    }

    /// Whether this first run can share automatically: only with an
    /// enrolment the daemon holds (`FirstRunState.holdsEnrolment`). Watching
    /// only, no answer, a passkey chosen but not created, and near.ai whose
    /// invite has not enrolled cannot.
    public static func canChooseAutomatic(_ state: FirstRunState) -> Bool {
        state.holdsEnrolment
    }

    /// The sharing paths the Uses picker offers, in Ron's #1030 order:
    /// Automatic first, then Ask me. The default answer stays Ask me.
    public static func sharingPaths(for state: FirstRunState) -> [SharingPath] {
        canChooseAutomatic(state) ? [.automatic, .askMe] : [.askMe]
    }

    private static func move(_ state: FirstRunState, by offset: Int) -> FirstRunState {
        let steps = steps(for: state.tier)
        guard let index = steps.firstIndex(of: state.step) else { return state }
        let target = index + offset
        guard steps.indices.contains(target) else { return state }
        var moved = state
        moved.step = steps[target]
        return moved
    }
}
