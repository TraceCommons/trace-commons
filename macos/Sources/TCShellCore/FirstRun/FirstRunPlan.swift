import Foundation

/// One daemon call the first run makes. The runner executes these in order
/// and stops at the first failure.
public enum FirstRunCall: Equatable, Sendable {
    case startDaemon(settingsJSON: String)
    case lookupInvite(String)
    case enroll(String)
    case signInNearAI
    case setConsentScopes([String])
    case setProjectMode(projectID: String, ProjectMode)
    case includePastSessions(projectID: String, [String])
    case setPrivateAI(Bool)
    case grantAutomatic(witness: String?)
    case markComplete
}

/// Where the first run commits answers to the daemon.
public enum CommitPoint: Equatable, Sendable {
    /// Leaving Folders (Quick) or Tools (Custom): the daemon starts, then
    /// the invite deferred from Join is looked up and joined.
    case leaveRoots
    /// Start on Uses.
    case start
}

/// State to ordered daemon calls. Pure, and recomputed at every commit, so a
/// retry after a failure is the same plan minus what already succeeded.
public enum FirstRunPlan {
    public static func calls(for state: FirstRunState, at commit: CommitPoint) -> [FirstRunCall] {
        switch commit {
        case .leaveRoots: return leaveRoots(state)
        case .start: return start(state)
        }
    }

    private static func leaveRoots(_ state: FirstRunState) -> [FirstRunCall] {
        var calls: [FirstRunCall] = []
        if !state.daemonStarted {
            // No join call runs without a daemon to run it.
            guard let json = state.sessionRoots.settingsJSON() else { return [] }
            calls.append(.startDaemon(settingsJSON: json))
        }
        let invite = state.invite.trimmingCharacters(in: .whitespacesAndNewlines)
        if state.account != .watchOnly, !invite.isEmpty {
            calls.append(.lookupInvite(invite))
            if !state.enrolled {
                calls.append(.enroll(invite))
            }
        }
        if state.account == .nearAI {
            calls.append(.signInNearAI)
        }
        return calls
    }

    private static func start(_ state: FirstRunState) -> [FirstRunCall] {
        var calls: [FirstRunCall] = [.setConsentScopes(state.scopes.sorted())]
        if state.tier == .custom {
            for projectID in state.rules.keys.sorted() {
                if let mode = state.rules[projectID] {
                    calls.append(.setProjectMode(projectID: projectID, mode))
                }
            }
            for projectID in state.pastSelections.keys.sorted() {
                // A Never folder contributes nothing, its past sessions
                // included; the daemon would refuse the whole include.
                guard state.rules[projectID] != .ignore,
                    let sessions = state.pastSelections[projectID], !sessions.isEmpty
                else { continue }
                calls.append(.includePastSessions(projectID: projectID, sessions.sorted()))
            }
            calls.append(.setPrivateAI(state.privateAI))
        }
        if state.sharing == .automatic, FirstRunNavigation.canChooseAutomatic(state.account) {
            calls.append(.grantAutomatic(witness: state.witnessSigningAddress))
        }
        calls.append(.markComplete)
        return calls
    }
}
