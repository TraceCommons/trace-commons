import XCTest

@testable import TCShellCore

final class FirstRunPlanTests: XCTestCase {
    /// A Quick state that has answered Folders and holds an invite.
    private func answered(tier: FirstRunTier = .quick) -> FirstRunState {
        var state = FirstRunState(tier: tier, step: tier == .quick ? .folders : .tools)
        state.invite = "INVITE-1"
        state.account = .nearAI
        state.toolAnswers[.claudeCode] = .watch(path: "/Users/someone/.claude/projects")
        state.toolAnswers[.codex] = .off
        return state
    }

    /// `answered` on Uses, after leaving the roots enrolled its invite: the
    /// enrollment Start's scopes, grant and marker belong to.
    private func onUses(tier: FirstRunTier = .quick) -> FirstRunState {
        var state = answered(tier: tier)
        state.step = .uses
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

    func test_theDaemonStartsBeforeAnyJoinCall() throws {
        let calls = FirstRunPlan.calls(for: answered(), at: .leaveRoots)
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
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .leaveRoots), [],
            "no join call runs without a daemon to run it")
    }

    func test_anAddedTrajectoryFolderIsDeclared() throws {
        var state = answered(tier: .custom)
        state.addedFolders = [AddedFolder(kind: .trajectory, path: "/Users/someone/exports")]
        let decoded = try settings(FirstRunPlan.calls(for: state, at: .leaveRoots).first)
        XCTAssertEqual(decoded["trajectory_source"] as? [String: String],
            ["mode": "watch", "path": "/Users/someone/exports"])
    }

    /// An invite pasted on Join and then "watch only" chosen: the invite is
    /// kept, so only the account answer stops the join.
    func test_watchOnlyJoinsNothing() throws {
        var state = answered()
        state.account = .watchOnly
        XCTAssertEqual(state.invite, "INVITE-1")
        let json = try XCTUnwrap(state.sessionRoots.settingsJSON())
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .leaveRoots),
            [.startDaemon(settingsJSON: json)])
    }

    /// Create passkey is chosen on Join and its sheets open after the daemon
    /// starts, in the near.ai sign-in's place: the two are one account, so
    /// the one chosen is the one planned. A created passkey, or none chosen,
    /// opens nothing.
    func test_aChosenPasskeyIsCreatedOnceTheDaemonStarts() throws {
        var state = answered()
        state.invite = ""
        state.account = .passkeyChosen
        let json = try XCTUnwrap(state.sessionRoots.settingsJSON())
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .leaveRoots), [
            .startDaemon(settingsJSON: json), .openPasskeySheets,
        ])

        // Closed before a passkey existed and forward again: asked again,
        // behind no second start or sign-in.
        state.daemonStarted = true
        state.startedSettingsJSON = json
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .leaveRoots), [.openPasskeySheets])

        for account in [AccountAnswer.none, .watchOnly, .nearAI, .passkey(name: "Mac"), .passkey(name: ""), .enrolled] {
            state.account = account
            XCTAssertFalse(FirstRunPlan.calls(for: state, at: .leaveRoots).contains(.openPasskeySheets),
                "\(account)")
        }
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
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .leaveRoots), [
            .startDaemon(settingsJSON: json), .lookupInvite("INVITE-1"), .enroll("INVITE-1"),
        ])

        // Already enrolled: still no sheet.
        state.daemonStarted = true
        state.startedSettingsJSON = json
        state.enrolledInvite = "INVITE-1"
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .leaveRoots), [])
        state.invite = ""
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .leaveRoots), [])
    }

    /// A plan-level stand-in only: the plan is pure, so this pins that a
    /// state the runner left untouched replans the whole join behind the
    /// start. The outcome mapping is `FirstRunRunnerTests`' (Task 3).
    func test_aFailedStartKeepsTheInviteAndJoinsNothing() throws {
        let state = answered()
        let json = try XCTUnwrap(state.sessionRoots.settingsJSON())
        let first = FirstRunPlan.calls(for: state, at: .leaveRoots)
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
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .leaveRoots), first)
    }

    /// The state after every `leaveRoots` call succeeded.
    private func joined(_ state: FirstRunState) throws -> FirstRunState {
        var joined = state
        joined.daemonStarted = true
        joined.startedSettingsJSON = try XCTUnwrap(state.sessionRoots.settingsJSON())
        joined.enrolledInvite = "INVITE-1"
        joined.signedIn = true
        return joined
    }

    func test_backAfterStartDoesNotEnrollTwice() throws {
        var state = try joined(answered())
        state.step = .uses
        state = FirstRunNavigation.back(state)
        state = FirstRunNavigation.back(state)
        XCTAssertEqual(state.step, .join)
        XCTAssertEqual(state.invite, "INVITE-1")
        state = FirstRunNavigation.next(state)

        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .leaveRoots), [],
            "started, joined and signed in: nothing runs twice")
    }

    func test_anEnrolledInviteIsNotLookedUpAgain() throws {
        var state = try joined(answered())
        state.invite = "  INVITE-1\n"
        state.signedIn = false
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .leaveRoots), [.signInNearAI],
            "a used-up invite would answer exhausted")
    }

    func test_aNewInviteAfterEnrollingIsLookedUpAndEnrolled() throws {
        var state = try joined(answered())
        state.invite = "INVITE-2"
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .leaveRoots),
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
        state.step = .uses
        state = FirstRunNavigation.back(state)
        XCTAssertEqual(state.step, .folders)
        state.answer(.codex, .off)

        let calls = FirstRunPlan.calls(for: state, at: .leaveRoots)
        XCTAssertEqual(calls.count, 1)
        XCTAssertEqual(try sourceSettings(calls.first), ["codex_source": ["mode": "off"]],
            "only the changed answer is sent")
    }

    func test_aFolderWithdrawnAfterStartIsTurnedOff() throws {
        var state = answered(tier: .custom)
        state.add(AddedFolder(kind: .trajectory, path: "/Users/someone/exports"))
        state = try joined(state)
        state.addedFolders = []

        let calls = FirstRunPlan.calls(for: state, at: .leaveRoots)
        XCTAssertEqual(calls.count, 1)
        XCTAssertEqual(try sourceSettings(calls.first), ["trajectory_source": ["mode": "off"]],
            "an absent key leaves the daemon watching, so a withdrawn folder is sent as off")
    }

    func test_aStartedDaemonWithNoRecordedDeclarationIsSentTheWholeOne() throws {
        var state = try joined(answered())
        state.startedSettingsJSON = nil
        let calls = FirstRunPlan.calls(for: state, at: .leaveRoots)
        XCTAssertEqual(calls.count, 1)
        XCTAssertEqual(try sourceSettings(calls.first), [
            "claude_source": ["mode": "watch", "path": "/Users/someone/.claude/projects"],
            "codex_source": ["mode": "off"],
        ])
    }

    func test_scopesAreSavedBeforeTheGrant() {
        var state = onUses(tier: .custom)
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
            .setProjectMode(projectID: "p2", .ask),
            .includePastSessions(projectID: "p1", ["s1", "s2"]),
            .setPrivateAI(true),
            .grantAutomatic(witness: "witness-1"),
            .markComplete,
        ])
    }

    /// Automatic is not enough on its own: the grant is sent only once the
    /// core answered ready after both disclosures, which sets `grantReady`.
    func test_automaticWithoutTheCoresReadyAnswerNeverGrants() {
        var state = onUses()
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
        var state = onUses()
        state.scopes = ["research"]
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .start),
            [.setConsentScopes(["research"]), .markComplete])
    }

    /// Review Focus 5: Start finishes watching. Watch only holds no
    /// enrollment, so nothing goes to the enrollment's config (the daemon
    /// refuses consent scopes without one) and nothing is granted; Start
    /// ends in the watch-only marker, which needs no tenant. Custom's local
    /// choices (folder rules, past sessions, Private AI) are still sent.
    func test_watchOnlyStartFinishesWithoutAnEnrolment() {
        var quick = answered()
        quick.step = .uses
        quick.account = .watchOnly
        quick.scopes = ["research"]
        quick.sharing = .automatic
        quick.grantReady = true
        XCTAssertEqual(FirstRunPlan.calls(for: quick, at: .start), [.markWatchOnlyComplete])

        var custom = answered(tier: .custom)
        custom.step = .uses
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
        var afterFailedSignIn = answered(tier: .custom)
        afterFailedSignIn.step = .uses
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

    func test_quickNeverSetsARuleOrIncludesPastSessions() {
        var state = onUses()
        state.scopes = ["research"]
        state.rules = ["p1": .autoUpload]
        state.pastSelections = ["p1": ["s1"]]
        state.privateAI = true
        let calls = FirstRunPlan.calls(for: state, at: .start)
        XCTAssertEqual(calls, [.setConsentScopes(["research"]), .markComplete])
    }

    func test_aNeverFolderIncludesNothing() {
        var state = onUses(tier: .custom)
        state.scopes = ["research"]
        state.rules = ["p1": .ignore]
        state.pastSelections = ["p1": ["s1"], "p2": []]
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .start), [
            .setConsentScopes(["research"]),
            .setProjectMode(projectID: "p1", .ignore),
            .setPrivateAI(false),
            .markComplete,
        ])
        XCTAssertFalse(FirstRunPlan.calls(for: state, at: .start).contains(where: isGrant))
    }

    /// Kristi's #1235 I1, as decided: a passkey chosen on Join but never
    /// created reaches Start without an account. Start reopens the sheets
    /// and sends nothing else -- no scopes, no grant, no marker -- since the
    /// sheets are not awaited and nothing after them may run.
    func test_aChosenPasskeyAtStartReopensTheSheetsAndNothingElse() {
        var state = answered(tier: .custom)
        state.invite = ""
        state.step = .uses
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
        state.step = .uses
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
