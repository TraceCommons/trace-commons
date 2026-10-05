import Foundation

/// One daemon call the first run makes. The runner executes these in order
/// and stops at the first failure.
public enum FirstRunCall: Equatable, Sendable {
    case startDaemon(settingsJSON: String)
    /// A `set_settings` object (sorted keys) carrying only the source
    /// declarations that changed since the daemon started.
    case setSourceSettings(settingsJSON: String)
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
        // No join call runs without a declaration the daemon would take.
        guard let json = state.sessionRoots.settingsJSON() else { return [] }
        var calls: [FirstRunCall] = []
        if !state.daemonStarted {
            calls.append(.startDaemon(settingsJSON: json))
        } else if let changed = changedDeclarations(from: state.startedSettingsJSON, to: json) {
            calls.append(.setSourceSettings(settingsJSON: changed))
        }
        let invite = state.invite.trimmingCharacters(in: .whitespacesAndNewlines)
        if state.account != .watchOnly, !invite.isEmpty, state.enrolledInvite != invite {
            calls.append(.lookupInvite(invite))
            calls.append(.enroll(invite))
        }
        if state.account == .nearAI, !state.signedIn {
            calls.append(.signInNearAI)
        }
        return calls
    }

    /// The declarations in `current` that differ from `started`, as a sorted
    /// `set_settings` object, or nil when nothing changed.
    ///
    /// `set_settings` merges the keys it is given, so a key withdrawn since
    /// the start is sent as `off`; leaving it out would leave the daemon
    /// watching it. An unknown or unreadable `started` sends everything.
    private static func changedDeclarations(from started: String?, to current: String) -> String? {
        // Unreadable here means unknown, and unknown sends everything.
        guard let now = declarations(current) else { return current }
        let before = started.flatMap(declarations) ?? [:]
        var changed: [String: [String: String]] = [:]
        for (key, declaration) in now where before[key] != declaration {
            changed[key] = declaration
        }
        for key in before.keys where now[key] == nil {
            changed[key] = ["mode": "off"]
        }
        guard !changed.isEmpty,
            let data = try? JSONSerialization.data(withJSONObject: changed, options: [.sortedKeys]),
            let json = String(data: data, encoding: .utf8)
        else { return nil }
        return json
    }

    private static func declarations(_ json: String) -> [String: [String: String]]? {
        guard let object = try? JSONSerialization.jsonObject(with: Data(json.utf8)) else { return nil }
        return object as? [String: [String: String]]
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
        // Automatic alone is not enough: only the core's ready answer after
        // both disclosures (`grantReady`) sends the grant.
        if state.sharing == .automatic, state.grantReady, FirstRunNavigation.canChooseAutomatic(state.account) {
            calls.append(.grantAutomatic(witness: state.witnessSigningAddress))
        }
        calls.append(.markComplete)
        return calls
    }
}
