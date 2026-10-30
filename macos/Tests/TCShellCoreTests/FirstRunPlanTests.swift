import XCTest

@testable import TCShellCore

final class FirstRunPlanTests: XCTestCase {
    /// A state on Join that answered Tools, holds an invite and chose
    /// near.ai: what Sign in commits.
    private func answered() -> FirstRunState {
        var state = FirstRunState(step: .join)
        state.invite = "INVITE-1"
        state.account = .nearAI
        state.toolAnswers[.claudeCode] = .watch(path: "/Users/someone/.claude/projects")
        state.toolAnswers[.codex] = .off
        return state
    }

    /// `answered` after Sign in enrolled its invite: the enrollment Start's
    /// scopes, grant and marker belong to.
    private func signedIn() -> FirstRunState {
        var state = answered()
        state.daemonStarted = true
        state.enrolledInvite = "INVITE-1"
        state.signedIn = true
        return state
    }

    private func settings(_ call: FirstRunCall?) throws -> [String: Any] {
        guard case .startDaemon(let json)? = call else {
            XCTFail("expected startDaemon, got \(String(describing: call))")
            return [:]
        }
        let object = try JSONSerialization.jsonObject(with: Data(json.utf8))
        return try XCTUnwrap(object as? [String: Any])
    }

    private func isGrant(_ call: FirstRunCall) -> Bool {
        if case .grantAutomatic = call { return true }
        return false
    }

    /// Continue on Tools and Customize send the roots and nothing else:
    /// sign-in is Join's, the last step.
    func test_leavingToolsDeclaresTheRootsOnly() throws {
        for point in [CommitPoint.leaveTools, .declareRoots] {
            let calls = FirstRunPlan.calls(for: answered(), at: point)
            XCTAssertEqual(calls.count, 1, "\(point)")
            let decoded = try settings(calls.first)
            XCTAssertEqual(decoded["codex_source"] as? [String: String], ["mode": "off"])
        }
        var started = answered()
        started.daemonStarted = true
        started.startedSettingsJSON = started.sessionRoots.settingsJSON()
        XCTAssertEqual(FirstRunPlan.calls(for: started, at: .leaveTools), [], "nothing changed, nothing sent")
        started.answer(.codex, .watch(path: "/Users/someone/.codex/sessions"))
        XCTAssertEqual(try sourceSettings(FirstRunPlan.calls(for: started, at: .declareRoots).first).keys.sorted(),
            ["codex_source"])
    }

    func test_theDaemonStartsBeforeAnyJoinCall() throws {
        let calls = FirstRunPlan.calls(for: answered(), at: .signIn)
        XCTAssertEqual(calls.count, 4)
        let decoded = try settings(calls.first)
        XCTAssertEqual(decoded["claude_source"] as? [String: String],
            ["mode": "watch", "path": "/Users/someone/.claude/projects"])
        XCTAssertEqual(decoded["codex_source"] as? [String: String], ["mode": "off"])
        XCTAssertEqual(Array(calls.dropFirst()),
            [.lookupInvite("INVITE-1"), .enroll("INVITE-1"), .signInNearAI])
    }

    func test_unfinishedRootsPlanNothing() {
        var state = answered()
        state.toolAnswers[.codex] = nil
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .signIn), [],
            "no join call runs without a daemon to run it")
    }

    func test_anAddedTrajectoryFolderIsDeclared() throws {
        var state = answered()
        state.addedFolders = [AddedFolder(kind: .trajectory, path: "/Users/someone/exports")]
        let decoded = try settings(FirstRunPlan.calls(for: state, at: .signIn).first)
        XCTAssertEqual(decoded["trajectory_source"] as? [String: String],
            ["mode": "watch", "path": "/Users/someone/exports"])
    }

    /// An invite held on Join with "watch only" recorded (an earlier build's
    /// answer): the invite is kept, so only the account answer stops the
    /// join.
    func test_watchOnlyJoinsNothing() throws {
        var state = answered()
        state.account = .watchOnly
        XCTAssertEqual(state.invite, "INVITE-1")
        let json = try XCTUnwrap(state.sessionRoots.settingsJSON())
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .signIn),
            [.startDaemon(settingsJSON: json)])
    }

    /// A passkey an earlier build recorded as chosen opens its sheets at
    /// Sign in, in the near.ai sign-in's place: the two are one account, so
    /// the one chosen is the one planned. A created passkey, or none chosen,
    /// opens nothing.
    func test_aChosenPasskeyIsCreatedOnceTheDaemonStarts() throws {
        var state = answered()
        state.invite = ""
        state.account = .passkeyChosen
        let json = try XCTUnwrap(state.sessionRoots.settingsJSON())
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .signIn), [
            .startDaemon(settingsJSON: json), .openPasskeySheets,
        ])

        // Closed before a passkey existed and forward again: asked again,
        // behind no second start or sign-in.
        state.daemonStarted = true
        state.startedSettingsJSON = json
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .signIn), [.openPasskeySheets])

        for account in [AccountAnswer.none, .watchOnly, .nearAI, .passkey(name: "Mac"), .passkey(name: ""), .enrolled] {
            state.account = account
            XCTAssertFalse(FirstRunPlan.calls(for: state, at: .signIn).contains(.openPasskeySheets),
                "\(account)")
        }
    }

    /// Create passkey on Join opens the sheets over Join (#1030): with no
    /// daemon yet it starts one watching nothing -- Claude Code and Codex
    /// both declared `off`, so the start gate admits it and no folder is
    /// read -- then opens them. A running daemon only opens them.
    func test_createPasskeyOnJoinStartsTheDaemonWatchingNothing() throws {
        let state = FirstRunState()
        let calls = FirstRunPlan.calls(for: state, at: .passkeyOnJoin)
        XCTAssertEqual(calls.count, 2)
        XCTAssertEqual(calls.last, .openPasskeySheets)
        let declared = try settings(calls.first)
        XCTAssertEqual(declared.count, 2, "only the two roots the start gate reads")
        XCTAssertEqual(declared["claude_source"] as? [String: String], ["mode": "off"])
        XCTAssertEqual(declared["codex_source"] as? [String: String], ["mode": "off"])

        var running = state
        running.daemonStarted = true
        XCTAssertEqual(FirstRunPlan.calls(for: running, at: .passkeyOnJoin), [.openPasskeySheets])
    }

    /// Wherever Join offers no passkey, Create passkey plans nothing: an
    /// invite held or enrolled (the daemon refuses a passkey account over an
    /// enrollment), a signed-in near.ai, an enrollment, a held passkey.
    func test_createPasskeyOnJoinPlansNothingBesideAnotherAccount() {
        var withInvite = FirstRunState()
        withInvite.invite = "INVITE-1"
        var enrolledInvite = FirstRunState()
        enrolledInvite.enrolledInvite = "INVITE-1"
        var signedIn = FirstRunState(account: .nearAI)
        signedIn.signedIn = true
        for state in [
            withInvite, enrolledInvite, signedIn, FirstRunState(account: .enrolled),
            FirstRunState(account: .passkey(name: "Mac")), FirstRunState(account: .passkey(name: "")),
        ] {
            XCTAssertEqual(FirstRunPlan.calls(for: state, at: .passkeyOnJoin), [], "\(state.account)")
        }
    }

    /// A daemon a passkey started holds `off` for both roots. The next
    /// commit then sends only what differs from that, never a second start,
    /// and a row answered `off` again is not resent.
    func test_rootsAfterAPasskeyStartSendOnlyTheChangedRoots() throws {
        var state = answered()
        state.invite = ""
        state.account = .passkey(name: "Mac")
        state.daemonStarted = true
        state.startedSettingsJSON = FirstRunPlan.watchNothingSettingsJSON
        let calls = FirstRunPlan.calls(for: state, at: .signIn)
        guard case .setSourceSettings(let json)? = calls.first else {
            return XCTFail("expected setSourceSettings, got \(calls)")
        }
        XCTAssertEqual(calls.count, 1)
        let sent = try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: [String: String]])
        XCTAssertEqual(Set(sent.keys), ["claude_source"])
        XCTAssertEqual(sent["claude_source"]?["mode"], "watch")
    }

    /// A new passkey creates an account of its own, and the daemon refuses
    /// to create one over an enrollment (`account-already-enrolled`), so an
    /// invite and a chosen passkey never both reach the daemon. Join keeps
    /// them apart (`JoinScreenLayout.lookUp`); the plan refuses the pair anyway:
    /// the invite is joined and no sheet opens.
    func test_anInviteWithAChosenPasskeyOpensNoSheets() throws {
        var state = answered()
        state.account = .passkeyChosen
        let json = try XCTUnwrap(state.sessionRoots.settingsJSON())
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .signIn), [
            .startDaemon(settingsJSON: json), .lookupInvite("INVITE-1"), .enroll("INVITE-1"),
        ])

        // Already enrolled: still no sheet.
        state.daemonStarted = true
        state.startedSettingsJSON = json
        state.enrolledInvite = "INVITE-1"
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .signIn), [])
        state.invite = ""
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .signIn), [])
    }

    /// A plan-level stand-in only: the plan is pure, so this pins that a
    /// state the runner left untouched replans the whole join behind the
    /// start. The outcome mapping is `FirstRunRunnerTests`' (Task 3).
    func test_aFailedStartKeepsTheInviteAndJoinsNothing() throws {
        let state = answered()
        let json = try XCTUnwrap(state.sessionRoots.settingsJSON())
        let first = FirstRunPlan.calls(for: state, at: .signIn)
        XCTAssertEqual(first, [
            .startDaemon(settingsJSON: json),
            .lookupInvite("INVITE-1"),
            .enroll("INVITE-1"),
            .signInNearAI,
        ])

        // The start failed: the runner records nothing, so the retry is the
        // same plan, invite intact, and no join call runs ahead of the start.
        XCTAssertFalse(state.daemonStarted)
        XCTAssertNil(state.enrolledInvite)
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .signIn), first)
    }

    /// The state after every `signIn` call succeeded.
    private func joined(_ state: FirstRunState) throws -> FirstRunState {
        var joined = state
        joined.daemonStarted = true
        joined.startedSettingsJSON = try XCTUnwrap(state.sessionRoots.settingsJSON())
        joined.enrolledInvite = "INVITE-1"
        joined.signedIn = true
        return joined
    }

    func test_backAfterSignInDoesNotEnrollTwice() throws {
        var state = try joined(answered())
        state = FirstRunNavigation.back(state)
        state = FirstRunNavigation.back(state)
        XCTAssertEqual(state.step, .traces)
        XCTAssertEqual(state.invite, "INVITE-1")
        state = FirstRunNavigation.next(FirstRunNavigation.next(state))
        XCTAssertEqual(state.step, .join)

        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .signIn), [],
            "started, joined and signed in: nothing runs twice")
    }

    func test_anEnrolledInviteIsNotLookedUpAgain() throws {
        var state = try joined(answered())
        state.invite = "  INVITE-1\n"
        state.signedIn = false
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .signIn), [.signInNearAI],
            "a used-up invite would answer exhausted")
    }

    func test_aNewInviteAfterEnrollingIsLookedUpAndEnrolled() throws {
        var state = try joined(answered())
        state.invite = "INVITE-2"
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .signIn),
            [.lookupInvite("INVITE-2"), .enroll("INVITE-2")])
    }

    private func sourceSettings(_ call: FirstRunCall?) throws -> [String: [String: String]] {
        guard case .setSourceSettings(let json)? = call else {
            XCTFail("expected setSourceSettings, got \(String(describing: call))")
            return [:]
        }
        let object = try JSONSerialization.jsonObject(with: Data(json.utf8))
        return try XCTUnwrap(object as? [String: [String: String]])
    }

    func test_aFolderChangedAfterStartReachesTheDaemon() throws {
        var state = answered()
        state.toolAnswers[.codex] = .watch(path: "/Users/someone/.codex/sessions")
        state = try joined(state)
        state = FirstRunNavigation.back(to: .tools, from: state)
        XCTAssertEqual(state.step, .tools)
        state.answer(.codex, .off)

        let calls = FirstRunPlan.calls(for: state, at: .leaveTools)
        XCTAssertEqual(calls.count, 1)
        XCTAssertEqual(try sourceSettings(calls.first), ["codex_source": ["mode": "off"]],
            "only the changed answer is sent")
    }

    func test_aFolderWithdrawnAfterStartIsTurnedOff() throws {
        var state = answered()
        state.add(AddedFolder(kind: .trajectory, path: "/Users/someone/exports"))
        state = try joined(state)
        state.addedFolders = []

        let calls = FirstRunPlan.calls(for: state, at: .signIn)
        XCTAssertEqual(calls.count, 1)
        XCTAssertEqual(try sourceSettings(calls.first), ["trajectory_source": ["mode": "off"]],
            "an absent key leaves the daemon watching, so a withdrawn folder is sent as off")
    }

    func test_aStartedDaemonWithNoRecordedDeclarationIsSentTheWholeOne() throws {
        var state = try joined(answered())
        state.startedSettingsJSON = nil
        let calls = FirstRunPlan.calls(for: state, at: .signIn)
        XCTAssertEqual(calls.count, 1)
        XCTAssertEqual(try sourceSettings(calls.first), [
            "claude_source": ["mode": "watch", "path": "/Users/someone/.claude/projects"],
            "codex_source": ["mode": "off"],
        ])
    }

    func test_scopesAreSavedBeforeTheGrant() {
        var state = signedIn()
        state.scopes = ["research", "evaluation"]
        state.rules = ["p2": .ask, "p1": .autoUpload]
        state.pastSelections = ["p1": ["s2", "s1"]]
        state.privateAI = true
        state.sharing = .automatic
        state.witnessSigningAddress = "witness-1"
        state.grantReady = true

        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .start), [
            .setConsentScopes(["evaluation", "research"]),
            .setProjectMode(projectID: "p1", .autoUpload),
            .includePastSessions(projectID: "p1", ["s1", "s2"]),
            .setPrivateAI(true),
            .grantAutomatic(witness: "witness-1"),
            .setProjectMode(projectID: "p2", .ask),
            .markComplete,
        ])
    }

    /// Owner decision 2026-10-29 (#1747): an Ask picked in the same first
    /// run as "Automatic, including existing history" is kept. The daemon
    /// lets the grant replace an Ask set before it and keeps one set after
    /// it, so Start sends each Ask rule, and that folder's past sessions,
    /// after the grant. Never still goes before it, so a Never folder is
    /// never covered by the grant, even for a moment.
    func test_anAskPickedWithTheGrantIsSentAfterIt() {
        var state = signedIn()
        state.scopes = ["research"]
        state.rules = ["p1": .ask, "p2": .ignore, "p3": .autoUpload]
        state.pastSelections = ["p1": ["s1"], "p3": ["s3"]]
        state.sharing = .automatic
        state.witnessSigningAddress = "witness-1"
        state.grantReady = true

        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .start), [
            .setConsentScopes(["research"]),
            .setProjectMode(projectID: "p2", .ignore),
            .setProjectMode(projectID: "p3", .autoUpload),
            .includePastSessions(projectID: "p3", ["s3"]),
            .grantAutomatic(witness: "witness-1"),
            .setProjectMode(projectID: "p1", .ask),
            .includePastSessions(projectID: "p1", ["s1"]),
            .markComplete,
        ])
    }

    /// Automatic is not enough on its own: the grant is sent only once the
    /// core answered ready after both disclosures, which sets `grantReady`.
    func test_automaticWithoutTheCoresReadyAnswerNeverGrants() {
        var state = signedIn()
        state.scopes = ["research"]
        state.sharing = .automatic
        state.witnessSigningAddress = "witness-1"
        XCTAssertFalse(state.grantReady)
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .start),
            [.setConsentScopes(["research"]), .markComplete])

        state.grantReady = true
        XCTAssertTrue(FirstRunPlan.calls(for: state, at: .start).contains(.grantAutomatic(witness: "witness-1")))

        // Writing the sharing path again clears the marker, whatever it is
        // set to: a new choice needs a new answer.
        state.sharing = .automatic
        XCTAssertFalse(state.grantReady)
        XCTAssertFalse(FirstRunPlan.calls(for: state, at: .start).contains { isGrant($0) })
        state.grantReady = true
        state.sharing = .askMe
        XCTAssertFalse(state.grantReady)
    }

    func test_askMeNeverGrants() {
        var state = signedIn()
        state.scopes = ["research"]
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .start).count, 3,
                       "Ask must explicitly stop an earlier Automatic choice before saving scopes")
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .start),
            [.setAskEachTime, .setConsentScopes(["research"]), .markComplete])
    }

    /// Review Focus 5: Start finishes watching. Watch only holds no
    /// enrollment, so nothing goes to the enrollment's config (the daemon
    /// refuses consent scopes without one) and nothing is granted; Start
    /// ends in the watch-only marker, which needs no tenant. The local
    /// choices (folder rules, past sessions, Private AI) are still sent.
    func test_watchOnlyStartFinishesWithoutAnEnrolment() {
        var plain = answered()
        plain.account = .watchOnly
        plain.scopes = ["research"]
        plain.sharing = .automatic
        plain.grantReady = true
        XCTAssertEqual(FirstRunPlan.calls(for: plain, at: .start), [.markWatchOnlyComplete])

        var custom = answered()
        custom.account = .watchOnly
        custom.scopes = ["research"]
        custom.rules = ["p1": .ask]
        custom.pastSelections = ["p1": ["s1"]]
        custom.privateAI = true
        XCTAssertEqual(FirstRunPlan.calls(for: custom, at: .start), [
            .setProjectMode(projectID: "p1", .ask),
            .includePastSessions(projectID: "p1", ["s1"]),
            .setPrivateAI(true),
            .markWatchOnlyComplete,
        ])
    }

    /// The first-run consent bug, exactly: the daemon holds an enrollment
    /// (an invite enrolled before a near.ai sign-in failed, or one signed
    /// out of on Join), the person is on watch only, and Start carries an
    /// Automatic rule left on a folder and past-session picks. Nothing that
    /// would use the enrollment is sent -- no rule, no past session, no
    /// Private AI, no marker -- since every one of them would act under an
    /// enrollment whose scopes nobody chose, and the watch-only marker cannot
    /// finish while the daemon is logged in. Start is not offered for it
    /// either (`canContinue`).
    func test_watchOnlyUnderAHeldEnrolmentSendsNothing() {
        var afterFailedSignIn = answered()
        afterFailedSignIn.daemonStarted = true
        afterFailedSignIn.enrolledInvite = "INVITE-1"
        afterFailedSignIn.account = .watchOnly
        afterFailedSignIn.scopes = ["debugging_evaluation"]
        afterFailedSignIn.rules = ["p1": .autoUpload, "p2": .ask]
        afterFailedSignIn.pastSelections = ["p1": ["s1", "s2"], "p2": ["s3"]]
        afterFailedSignIn.privateAI = true
        XCTAssertTrue(afterFailedSignIn.daemonHoldsEnrolment)
        XCTAssertFalse(afterFailedSignIn.holdsEnrolment)
        XCTAssertEqual(FirstRunPlan.calls(for: afterFailedSignIn, at: .start), [])
        XCTAssertFalse(
            FirstRunNavigation.canContinue(
                afterFailedSignIn, candidates: [], requiredScope: "debugging_evaluation"))

        var signedOut = afterFailedSignIn
        signedOut.enrolledInvite = nil
        signedOut.signedOutOfEnrolment = true
        XCTAssertTrue(signedOut.daemonHoldsEnrolment)
        XCTAssertEqual(FirstRunPlan.calls(for: signedOut, at: .start), [])
        XCTAssertFalse(
            FirstRunNavigation.canContinue(signedOut, candidates: [], requiredScope: "debugging_evaluation"))

        // With no enrollment held, watch only still finishes as before.
        var watching = afterFailedSignIn
        watching.enrolledInvite = nil
        XCTAssertFalse(watching.daemonHoldsEnrolment)
        XCTAssertEqual(FirstRunPlan.calls(for: watching, at: .start).last, .markWatchOnlyComplete)
        XCTAssertTrue(FirstRunNavigation.canContinue(watching, candidates: [], requiredScope: "debugging_evaluation"))
    }

    /// Whether the daemon may hold an enrollment, whatever this first run
    /// treats as the account: one it enrolled, one an earlier run left, a
    /// bound passkey, near.ai's, and one signed out of on Join.
    func test_theDaemonHoldsAnEnrolmentWhateverTheAccountAnswer() {
        XCTAssertFalse(FirstRunState().daemonHoldsEnrolment)
        XCTAssertFalse(FirstRunState(account: .watchOnly).daemonHoldsEnrolment)
        XCTAssertFalse(FirstRunState(account: .nearAI).daemonHoldsEnrolment)
        XCTAssertFalse(FirstRunState(account: .passkeyChosen).daemonHoldsEnrolment)
        XCTAssertTrue(FirstRunState(account: .enrolled).daemonHoldsEnrolment)
        XCTAssertTrue(FirstRunState(account: .passkey(name: "")).daemonHoldsEnrolment)
        XCTAssertTrue(FirstRunState(account: .watchOnly, enrolledInvite: "I").daemonHoldsEnrolment)
        XCTAssertTrue(FirstRunState(account: .none, nearAIEnrolled: true).daemonHoldsEnrolment)
        XCTAssertTrue(FirstRunState(account: .watchOnly, signedOutOfEnrolment: true).daemonHoldsEnrolment)
    }

    /// Every first run passes Customize's rules (when it was opened) and the
    /// Private AI step, so Start sends what they hold; a first run that never
    /// opened Customize holds no rule and sends none, and a Private AI switch
    /// left off sends nothing.
    func test_startSendsCustomizesRulesAndThePrivateAISwitch() {
        var state = signedIn()
        state.scopes = ["research"]
        XCTAssertEqual(
            FirstRunPlan.calls(for: state, at: .start),
            [.setAskEachTime, .setConsentScopes(["research"]), .markComplete])
        state.rules = ["p1": .autoUpload]
        state.pastSelections = ["p1": ["s1"]]
        state.privateAI = true
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .start), [
            .setAskEachTime,
            .setConsentScopes(["research"]),
            .setProjectMode(projectID: "p1", .autoUpload),
            .includePastSessions(projectID: "p1", ["s1"]),
            .setPrivateAI(true),
            .markComplete,
        ])
    }

    /// Ask each time is the default for folders without a rule, not a
    /// global override (#1753 A2): a folder picked Automatic in Customize is
    /// armed and sends, as are its Ask and Never rules and its picked past
    /// sessions. Before, it was dropped, and the folder kept asking.
    func test_askEachTimeStillArmsAFolderPickedAutomatic() {
        var state = signedIn()
        state.scopes = ["research"]
        state.sharing = .askMe
        state.rules = ["p1": .autoUpload, "p2": .ask, "p3": .ignore]
        state.pastSelections = ["p1": ["s1"]]
        let calls = FirstRunPlan.calls(for: state, at: .start)
        XCTAssertEqual(calls, [
            .setAskEachTime,
            .setConsentScopes(["research"]),
            .setProjectMode(projectID: "p1", .autoUpload),
            .setProjectMode(projectID: "p2", .ask),
            .setProjectMode(projectID: "p3", .ignore),
            .includePastSessions(projectID: "p1", ["s1"]),
            .markComplete,
        ])
    }

    /// A folder's Automatic is picked and confirmed before sign-in (Ron,
    /// 2026-10-22) and armed at Start after the scopes, once an enrollment
    /// is held; watching only holds none, so its Automatic folder is not
    /// armed while its other rules are still sent.
    func test_aFoldersAutomaticIsArmedOnlyWithAnEnrolment() {
        var state = signedIn()
        state.scopes = ["research"]
        state.rules = ["p1": .autoUpload, "p2": .ask]
        state.sharing = .automatic
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .start), [
            .setConsentScopes(["research"]),
            .setProjectMode(projectID: "p1", .autoUpload),
            .setProjectMode(projectID: "p2", .ask),
            .markComplete,
        ])

        var watching = answered()
        watching.account = .watchOnly
        watching.rules = ["p1": .autoUpload, "p2": .ask]
        XCTAssertFalse(watching.holdsEnrolment)
        XCTAssertEqual(FirstRunPlan.calls(for: watching, at: .start), [
            .setProjectMode(projectID: "p2", .ask),
            .markWatchOnlyComplete,
        ])
    }

    func test_aNeverFolderIncludesNothing() {
        var state = signedIn()
        state.scopes = ["research"]
        state.rules = ["p1": .ignore]
        state.pastSelections = ["p1": ["s1"], "p2": []]
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .start), [
            .setAskEachTime,
            .setConsentScopes(["research"]),
            .setProjectMode(projectID: "p1", .ignore),
            .markComplete,
        ])
        XCTAssertFalse(FirstRunPlan.calls(for: state, at: .start).contains(where: isGrant))
    }

    /// Kristi's #1235 I1, as decided: a passkey chosen on Join but never
    /// created reaches Start without an account. Start reopens the sheets
    /// and sends nothing else -- no scopes, no grant, no marker -- since the
    /// sheets are not awaited and nothing after them may run.
    func test_aChosenPasskeyAtStartReopensTheSheetsAndNothingElse() {
        var state = answered()
        state.invite = ""
        state.account = .passkeyChosen
        state.daemonStarted = true
        state.scopes = ["research"]
        state.rules = ["p1": .ask]
        state.pastSelections = ["p1": ["s1"]]
        state.sharing = .automatic
        state.grantReady = true
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .start), [.openPasskeySheets])
    }

    /// Kristi's #1235 B1 floor: scopes, the grant and the enrollment's
    /// marker belong to an enrollment the daemon holds. An account answer
    /// without one sends none of them.
    func test_noEnrolmentNoScopesGrantOrMarker() {
        var state = answered()
        state.scopes = ["research"]
        state.sharing = .automatic
        state.grantReady = true
        state.witnessSigningAddress = "witness-1"
        XCTAssertNil(state.enrolledInvite)
        XCTAssertFalse(state.holdsEnrolment)
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .start), [])

        state.enrolledInvite = "INVITE-1"
        XCTAssertTrue(state.holdsEnrolment)
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .start), [
            .setConsentScopes(["research"]), .grantAutomatic(witness: "witness-1"), .markComplete,
        ])
    }

    /// The near.ai login the first run waits on: signed in once the daemon
    /// keeps a session; still waiting while the browser sign-in runs; over,
    /// unsigned, once the attempt ended any other way.
    func test_theNearAILoginWaitsOnlyWhileTheBrowserSignInRuns() {
        XCTAssertEqual(NearAILoginPoll.verdict(CredentialStatus(state: "x", sessionState: "present")), .signedIn)
        XCTAssertEqual(
            NearAILoginPoll.verdict(CredentialStatus(state: "x", attemptStatus: "waiting_for_browser")), .waiting)
        XCTAssertEqual(NearAILoginPoll.verdict(CredentialStatus(state: "x", attemptStatus: nil)), .waiting)
        for ended in ["cancelled", "failed", "complete", "something-new"] {
            XCTAssertEqual(NearAILoginPoll.verdict(CredentialStatus(state: "x", attemptStatus: ended)), .ended, ended)
        }
        XCTAssertEqual(NearAILoginPoll.verdict(nil), .waiting)
    }
}

/// Sign in again after NEAR AI refused the kept session: the kept session
/// says nothing about the new attempt.
final class NearAILoginFreshVerdictTests: XCTestCase {
    func test_onlyTheAttemptsOwnWordEndsAFreshSignIn() {
        func status(_ attempt: String?) -> CredentialStatus {
            CredentialStatus(state: "present", attemptID: "a", attemptStatus: attempt, sessionState: "present")
        }
        XCTAssertEqual(NearAILoginPoll.verdict(status("waiting_for_browser")), .signedIn,
                       "the ordinary poll trusts the kept session")
        XCTAssertEqual(NearAILoginPoll.freshVerdict(status("waiting_for_browser")), .waiting)
        XCTAssertEqual(NearAILoginPoll.freshVerdict(status(nil)), .waiting)
        XCTAssertEqual(NearAILoginPoll.freshVerdict(nil), .waiting)
        XCTAssertEqual(NearAILoginPoll.freshVerdict(status("complete")), .signedIn)
        XCTAssertEqual(NearAILoginPoll.freshVerdict(status("failed")), .ended)
        XCTAssertEqual(NearAILoginPoll.freshVerdict(status("cancelled")), .ended)
    }
}
