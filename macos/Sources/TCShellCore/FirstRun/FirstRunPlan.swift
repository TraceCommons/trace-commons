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
    /// With an invite: sign in to the account the invite enrolled
    /// (`account_sign_in`).
    case signInNearAI
    /// Without an invite: the near.ai login (the browser sign-in, skipped
    /// when the daemon already keeps one), then the enrolment through it
    /// (`near_ai_account_enroll`). Signing in with near.ai needs no invite
    /// (owner, Ron's review of #1235).
    case nearAILogin
    case enrollNearAI
    /// Open the passkey sheets for a passkey chosen on Join. Not a daemon
    /// call: the person goes through the sheets, whose ceremony completes
    /// with the daemon this commit started.
    case openPasskeySheets
    case setConsentScopes([String])
    case setProjectMode(projectID: String, ProjectMode)
    case includePastSessions(projectID: String, [String])
    case setPrivateAI(Bool)
    case grantAutomatic(witness: String?)
    /// The completion marker for an enrolment, keyed by its tenant.
    case markComplete
    /// The completion marker for watching only, which has no tenant to key
    /// a marker by.
    case markWatchOnlyComplete
}

/// How the first run's near.ai login stands, read from the daemon's
/// credential status while the browser sign-in runs.
public enum NearAILoginPoll: Equatable, Sendable {
    case signedIn
    case waiting
    case ended

    /// The ceremony's waiting word (`nearai_credential::ceremony`).
    public static let waitingForBrowser = "waiting_for_browser"
    /// How long the first run waits on the browser before it gives up and
    /// says the sign-in did not finish.
    public static let limit: TimeInterval = 600

    /// A kept session is signed in. An attempt still waiting on the
    /// browser, or a poll that could not be read, waits. Any other attempt
    /// word -- cancelled, failed, finished without a session, or one this
    /// build does not know -- is over.
    public static func verdict(_ status: CredentialStatus?) -> NearAILoginPoll {
        guard let status else { return .waiting }
        if status.sessionState == CredentialSurface.statePresent { return .signedIn }
        guard let attempt = status.attemptStatus else { return .waiting }
        return attempt == waitingForBrowser ? .waiting : .ended
    }
}

/// Where the first run commits answers to the daemon.
public enum CommitPoint: Equatable, Sendable {
    /// Create passkey on Join (#1030 `ftux-page.tsx`): the sheets open over
    /// Join, so the daemon their ceremony completes with starts here when it
    /// is not running yet. The step does not move.
    case passkeyOnJoin
    /// Leaving Folders (Quick) or Tools (Custom): the daemon starts, then
    /// the invite deferred from Join is looked up and joined, and the
    /// account chosen there is signed in or created.
    case leaveRoots
    /// Start on Uses.
    case start
}

/// State to ordered daemon calls. Pure, and recomputed at every commit, so a
/// retry after a failure is the same plan minus what already succeeded.
public enum FirstRunPlan {
    public static func calls(for state: FirstRunState, at commit: CommitPoint) -> [FirstRunCall] {
        switch commit {
        case .passkeyOnJoin: return passkeyOnJoin(state)
        case .leaveRoots: return leaveRoots(state)
        case .start: return start(state)
        }
    }

    /// The declaration a daemon started on Join holds until Folders or Tools
    /// answers: Claude Code and Codex both `off`, so the start gate
    /// (`daemon::settings::roots_declared`) admits it and nothing is read.
    /// An undeclared root would read the tool's conventional folder unasked,
    /// so it is never started without one. Binding an account
    /// (`nearai_onboarding::bind`) reads no source settings, so the passkey
    /// ceremony completes against this daemon. The first run never reads
    /// these `off`s back as answers: the next commit sends every row that
    /// differs (`changedDeclarations`), and a resumed first run sends the
    /// whole declaration again (`OnboardingNavigation.initialState`).
    public static let watchNothingSettingsJSON: String? = SessionRoots(claude: .off, codex: .off).settingsJSON()

    /// Create passkey on Join: start the daemon watching nothing when it is
    /// not running, then open the sheets. Nothing when Join offers no
    /// passkey (a held invite, a signed-in near.ai, an enrolment or a held
    /// passkey), as `JoinScreenLayout.showsPasskeyAction` decides; the daemon
    /// refuses a passkey account over an enrolment (`account-already-enrolled`).
    private static func passkeyOnJoin(_ state: FirstRunState) -> [FirstRunCall] {
        let invite = state.invite.trimmingCharacters(in: .whitespacesAndNewlines)
        let held: Bool = {
            if case .passkey = state.account { return true }
            return state.account == .enrolled
        }()
        guard !state.signedIn, !held, invite.isEmpty, state.enrolledInvite == nil else { return [] }
        if state.daemonStarted { return [.openPasskeySheets] }
        guard let json = watchNothingSettingsJSON else { return [] }
        return [.startDaemon(settingsJSON: json), .openPasskeySheets]
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
        let joinsInvite = state.account != .watchOnly && !invite.isEmpty
        if joinsInvite, state.enrolledInvite != invite {
            calls.append(.lookupInvite(invite))
            calls.append(.enroll(invite))
        }
        // One account: the near.ai sign-in or the passkey sheets, whichever
        // Join chose, in the same place. near.ai with an invite signs in to
        // the account the invite enrolled; without one it enrolls this Mac
        // through the near.ai login, and needs no invite.
        if state.account == .nearAI {
            if joinsInvite || state.enrolledInvite != nil {
                if !state.signedIn { calls.append(.signInNearAI) }
            } else if !state.nearAIEnrolled {
                calls.append(contentsOf: [.nearAILogin, .enrollNearAI])
            }
        }
        // A new passkey creates an account of its own, and the daemon
        // refuses to create one over an enrolment
        // (`account-already-enrolled`), so no sheet opens beside an invite.
        // Join keeps the two apart; this refuses the pair anyway.
        if state.account == .passkeyChosen, !joinsInvite, state.enrolledInvite == nil {
            calls.append(.openPasskeySheets)
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

    /// Watch only holds no enrolment, so Start sends nothing that belongs
    /// to one: no consent scopes (the daemon keeps them in the enrolment's
    /// config and refuses them without it), no grant, and the watch-only
    /// marker instead of the tenant's. Custom's folder rules, past sessions
    /// and Private AI are local to the daemon and are sent either way: for
    /// watching only, picked past sessions are queued on this Mac as pending
    /// offers and none is sent, which the Rules card says
    /// (`rules.past_sessions_watch_only`).
    ///
    /// A passkey chosen on Join and not yet created holds no account, so
    /// Start reopens its sheets and sends nothing else: the sheets are not
    /// awaited, and nothing may run behind them. Any other account answer
    /// without an enrolment the daemon holds sends nothing at all, since
    /// scopes, the grant and the marker all belong to one (`canContinue`
    /// keeps Start off for it).
    ///
    /// Watching only while the daemon still holds an enrolment
    /// (`FirstRunState.daemonHoldsEnrolment`: one signed out of on Join, or
    /// an invite enrolled before a near.ai sign-in failed) sends nothing at
    /// all. Its rules and past sessions would act under that enrolment,
    /// whose scopes nobody chose here, and its marker is refused while the
    /// daemon is logged in. Join does not offer watch only then, and
    /// `canContinue` keeps Start off for it.
    private static func start(_ state: FirstRunState) -> [FirstRunCall] {
        if state.account == .passkeyChosen { return [.openPasskeySheets] }
        let watchOnly = state.account == .watchOnly
        if watchOnly, state.daemonHoldsEnrolment { return [] }
        guard watchOnly || state.holdsEnrolment else { return [] }
        var calls: [FirstRunCall] = watchOnly ? [] : [.setConsentScopes(state.scopes.sorted())]
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
        if state.sharing == .automatic, state.grantReady, FirstRunNavigation.canChooseAutomatic(state) {
            calls.append(.grantAutomatic(witness: state.witnessSigningAddress))
        }
        calls.append(watchOnly ? .markWatchOnlyComplete : .markComplete)
        return calls
    }
}
