import XCTest

@testable import TCShellCore

final class FirstRunNavigationTests: XCTestCase {
    private func candidate(_ kind: SourceKind, exists: Bool = true) -> SourceCandidate {
        SourceCandidate(
            source: kind,
            path: "/Users/someone/\(kind.rawValue)",
            exists: exists,
            sessionCount: exists ? 3 : 0,
            mostRecent: nil,
            relocatedByEnv: false
        )
    }

    /// The animated first run (Ron's 2026-10-21 canvas): one order for
    /// everyone, signing in last. The step thread names every step after
    /// Welcome.
    func test_theStepsRunFromWelcomeToJoin() {
        XCTAssertEqual(FirstRunNavigation.steps, [.welcome, .tools, .uses, .traces, .privateAI, .join])
        XCTAssertEqual(FirstRunNavigation.threadSteps, [.tools, .uses, .traces, .privateAI, .join])
        XCTAssertEqual(FirstRunState().step, .welcome, "a first run opens on Welcome")
        var state = FirstRunState()
        var visited: [FirstRunStep] = [state.step]
        for _ in 0..<6 {
            state = FirstRunNavigation.next(state)
            visited.append(state.step)
        }
        XCTAssertEqual(visited, [.welcome, .tools, .uses, .traces, .privateAI, .join, .join])
    }

    /// A step already passed, picked on the thread, is Back to it; one not
    /// yet reached is no way forward.
    func test_theThreadGoesBackButNeverForward() {
        var state = FirstRunState(step: .traces, scopes: ["research"])
        state.grantReady = true
        let back = FirstRunNavigation.back(to: .tools, from: state)
        XCTAssertEqual(back.step, .tools)
        XCTAssertFalse(back.grantReady)
        XCTAssertEqual(back.scopes, ["research"], "every answer is kept")
        XCTAssertEqual(FirstRunNavigation.back(to: .join, from: state), state)
        XCTAssertEqual(FirstRunNavigation.back(to: .traces, from: state), state)
    }

    /// Welcome, Traces and Private AI ask nothing that is required.
    func test_welcomeTracesAndPrivateAIAlwaysGoOn() {
        for step: FirstRunStep in [.welcome, .traces, .privateAI] {
            XCTAssertTrue(
                FirstRunNavigation.canContinue(FirstRunState(step: step), candidates: [], requiredScope: nil), "\(step)")
        }
    }

    /// Uses goes on once the required use is known, so nobody passes it
    /// without seeing that use, and once the person picked a Sharing
    /// choice: neither is preselected (owner, 2026-10-27).
    func test_usesWaitsForTheRequiredUseToBeKnown() {
        var state = FirstRunState(step: .uses)
        XCTAssertFalse(state.sharingChosen)
        XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: [], requiredScope: "research"), "no pick yet")
        state.sharingChosen = true
        XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: [], requiredScope: nil))
        XCTAssertTrue(FirstRunNavigation.canContinue(state, candidates: [], requiredScope: "research"))
    }

    /// Spec rule 1, the owner's reversal in Ron's design review of #1235:
    /// a tool not found on this Mac is not asked. Every found tool's check
    /// starts unticked, Not used (Ron, 2026-10-22), and Continue waits for
    /// one to be ticked.
    func test_aMissingToolIsNotAskedAndFoundOnesStartUnticked() {
        let candidates = [candidate(.claudeCode), candidate(.codex, exists: false), candidate(.cline, exists: false)]
        var state = FirstRunState(step: .tools)
        state.recordDiscovery(candidates)
        XCTAssertEqual(state.toolAnswers[.claudeCode], .off, "found, and unticked")
        XCTAssertNil(state.toolAnswers[.codex], "nothing is written for the missing tool")
        XCTAssertNil(state.toolAnswers[.cline])
        XCTAssertFalse(state.watchesAnything)
        XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))

        state.toolAnswers[.claudeCode] = .watch(path: "/Users/someone/claude-code")
        XCTAssertTrue(state.watchesAnything)
        XCTAssertTrue(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))
    }

    /// Discovery never overwrites an answer: a ticked tool stays ticked when
    /// the rows are read again.
    func test_discoveryKeepsAnAnswerGiven() {
        let candidates = [candidate(.claudeCode), candidate(.codex)]
        var state = FirstRunState(step: .tools)
        state.recordDiscovery(candidates)
        state.toolAnswers[.codex] = .watch(path: "/Users/someone/codex")
        state.recordDiscovery(candidates)
        XCTAssertEqual(state.toolAnswers[.codex], .watch(path: "/Users/someone/codex"))
        XCTAssertEqual(state.toolAnswers[.claudeCode], .off)
    }

    /// The rule a missing tool is declared by. Claude Code and Codex must be
    /// declared for the daemon to start, and an absent declaration of
    /// either reads its conventional folder, so a tool not on this Mac
    /// would be read unasked the day it is installed. A missing Claude Code
    /// or Codex is declared `off`: watch nothing. A missing optional tool
    /// is left undeclared, which constructs no adapter. An answer the
    /// person did give always wins.
    func test_aMissingClaudeOrCodexIsDeclaredOffAndAnOptionalOneNothing() throws {
        let candidates = [candidate(.claudeCode, exists: false), candidate(.codex), candidate(.cline, exists: false)]
        var state = FirstRunState(step: .tools)
        state.recordDiscovery(candidates)
        state.toolAnswers[.codex] = .watch(path: "/Users/someone/codex")
        XCTAssertTrue(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))

        let json = try XCTUnwrap(state.sessionRoots.settingsJSON())
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: [String: String]])
        XCTAssertEqual(object["claude_source"], ["mode": "off"])
        XCTAssertEqual(object["codex_source"]?["mode"], "watch")
        XCTAssertNil(object["cline_source"])
        XCTAssertEqual(
            FirstRunPlan.calls(for: state, at: .leaveTools), [.startDaemon(settingsJSON: json)])

        // Found again later (installed while the app was away): it is
        // offered, unticked, and declared off until it is ticked.
        state.recordDiscovery([candidate(.claudeCode), candidate(.codex), candidate(.cline, exists: false)])
        XCTAssertEqual(state.toolAnswers[.claudeCode], .off)
        XCTAssertTrue(
            FirstRunNavigation.canContinue(
                state, candidates: [candidate(.claudeCode), candidate(.codex)], requiredScope: nil))
    }

    /// An optional tool found here gets a check like any other, and an
    /// unticked one is declared off: it is answered, Not used.
    func test_aFoundOptionalToolIsDeclaredOffUntilTicked() throws {
        let candidates = [candidate(.claudeCode), candidate(.codex), candidate(.cline)]
        var state = FirstRunState(step: .tools)
        state.recordDiscovery(candidates)
        XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))
        state.toolAnswers[.cline] = .watch(path: "/Users/someone/cline")
        XCTAssertTrue(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))
        let json = try XCTUnwrap(state.sessionRoots.settingsJSON())
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: [String: String]])
        XCTAssertEqual(object["claude_source"], ["mode": "off"])
        XCTAssertEqual(object["codex_source"], ["mode": "off"])
        XCTAssertEqual(object["cline_source"]?["mode"], "watch")
    }

    /// Ron's review of #1235, item 3: a folder added with "Not seeing your
    /// tool above?" starts unanswered, in a row of its own: its check is
    /// unticked, like every tool's. Recognising its layout stays; answering
    /// for the person does not. Nothing is written to the matched tool's
    /// row.
    func test_anAddedFolderStartsUnanswered() {
        let candidates = [candidate(.claudeCode), candidate(.codex)]
        var state = FirstRunState(step: .tools)
        state.toolAnswers[.claudeCode] = .off
        state.toolAnswers[.codex] = .off
        state.add(AddedFolder(kind: .source(.codex), path: "/Volumes/moved/codex"))
        XCTAssertNil(state.addedFolders.first?.watched)
        XCTAssertEqual(state.toolAnswers[.codex], .off, "the tool's own row keeps its answer")
        XCTAssertEqual(state.sessionRoots.codex, .off, "an unanswered folder declares nothing")
        XCTAssertFalse(
            FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil),
            "an unticked folder watches nothing")

        state.answerAdded(path: "/Volumes/moved/codex", watched: true)
        XCTAssertTrue(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))
        XCTAssertEqual(state.sessionRoots.codex, .watch(path: "/Volumes/moved/codex"))

        // Unticking the added row declares nothing for the tool: its own
        // row's answer stands, and nothing is watched again.
        state.answerAdded(path: "/Volumes/moved/codex", watched: false)
        XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))
        XCTAssertEqual(state.sessionRoots.codex, .off)
        XCTAssertEqual(state.addedFolders.count, 1, "the row stays, answered")
    }

    /// The rule for one tool watched in two rows (Ron's item 3 left it
    /// open): the daemon watches one folder per tool, so Continue is held
    /// while a tool's own row and a folder added for it both read Watch,
    /// and the added row says why. Nothing is un-answered for the person.
    func test_oneToolWatchedInTwoRowsHoldsContinue() {
        let candidates = [candidate(.claudeCode), candidate(.codex)]
        var state = FirstRunState(step: .tools)
        state.toolAnswers[.claudeCode] = .off
        state.toolAnswers[.codex] = .watch(path: "/Users/someone/codex")
        state.add(AddedFolder(kind: .source(.codex), path: "/Volumes/moved/codex"))
        state.answerAdded(path: "/Volumes/moved/codex", watched: true)
        XCTAssertEqual(state.watchedTwice, [.codex])
        XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))
        XCTAssertEqual(state.toolAnswers[.codex], .watch(path: "/Users/someone/codex"))

        state.toolAnswers[.codex] = .off
        XCTAssertEqual(state.watchedTwice, [])
        XCTAssertTrue(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))
    }

    /// Something watched is not enough on its own: the declaration the
    /// daemon starts with keeps Continue closed until Claude Code and Codex
    /// are both declared.
    func test_somethingWatchedStillNeedsADeclaration() {
        var state = FirstRunState(step: .tools)
        XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: [], requiredScope: nil))

        state.toolAnswers[.claudeCode] = .watch(path: "/Users/someone/claude")
        XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: [], requiredScope: nil))

        state.toolAnswers[.codex] = .off
        XCTAssertTrue(FirstRunNavigation.canContinue(state, candidates: [], requiredScope: nil))
    }

    /// The required use is always included (owner, 2026-10-08), so Start
    /// sharing waits for it to be known, never for a tick.
    func test_startWaitsForTheRequiredUseToBeKnownNotTicked() {
        var state = FirstRunState(step: .join)
        state.account = .enrolled
        state.scopes = ["evaluation"]
        state.sharingChosen = true
        XCTAssertTrue(FirstRunNavigation.canContinue(state, candidates: [], requiredScope: "research"))
        XCTAssertFalse(
            FirstRunNavigation.canContinue(state, candidates: [], requiredScope: nil),
            "with no required use known, Start stays disabled"
        )

        // Including it adds only the required use, once, and keeps the rest.
        let included = state.includingRequiredScope("research")
        XCTAssertEqual(included.scopes, ["evaluation", "research"])
        XCTAssertEqual(included.includingRequiredScope("research"), included)
        XCTAssertEqual(state.includingRequiredScope(nil), state)
        var others = included
        others.scopes = state.scopes
        XCTAssertEqual(others, state, "nothing but the scopes changes")

        // Start sends what the state holds, the required use included.
        XCTAssertEqual(
            Array(FirstRunPlan.calls(for: included, at: .start).prefix(2)),
            [.setAskEachTime, .setConsentScopes(["evaluation", "research"]) ])
    }

    func test_joinWaitsForSomethingStartCanFinish() {
        var state = FirstRunState(step: .join, sharingChosen: true)
        XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: [], requiredScope: "research"))
        state.account = .enrolled
        XCTAssertTrue(FirstRunNavigation.canContinue(state, candidates: [], requiredScope: "research"))
    }

    /// Start, on Join, waits for a Sharing choice the person picked, even
    /// with an enrollment held. Watching only shares nothing and needs none.
    func test_joinWaitsForASharingChoice() {
        var state = FirstRunState(step: .join, account: .enrolled)
        XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: [], requiredScope: "research"))
        state.sharing = .askMe
        XCTAssertFalse(
            FirstRunNavigation.canContinue(state, candidates: [], requiredScope: "research"),
            "the field's starting value is not a pick")
        state.sharingChosen = true
        XCTAssertTrue(FirstRunNavigation.canContinue(state, candidates: [], requiredScope: "research"))
        let watching = FirstRunState(step: .join, account: .watchOnly)
        XCTAssertTrue(FirstRunNavigation.canContinue(watching, candidates: [], requiredScope: "research"))
    }

    /// Uses comes before sign-in, which is required (owner, 2026-10-12),
    /// so it offers Automatic to everyone (Ron's 2026-10-21 canvas); only
    /// an enrollment the daemon holds can be granted it, at Start.
    func test_usesOffersAutomaticButOnlyAnEnrolmentIsGrantedIt() {
        for account: AccountAnswer in [.none, .watchOnly, .nearAI, .passkey(name: "Laptop"), .enrolled] {
            XCTAssertEqual(FirstRunNavigation.sharingPaths(for: FirstRunState(account: account)), [.askMe, .automatic])
        }
        XCTAssertFalse(FirstRunNavigation.canChooseAutomatic(FirstRunState(account: .watchOnly)))
        XCTAssertTrue(FirstRunNavigation.canChooseAutomatic(FirstRunState(account: .nearAI, enrolledInvite: "INVITE-1")))

        var state = FirstRunState(step: .join)
        state.account = .watchOnly
        state.sharing = .automatic
        state.scopes = ["research"]
        let calls = FirstRunPlan.calls(for: state, at: .start)
        XCTAssertFalse(calls.contains { if case .grantAutomatic = $0 { return true } else { return false } })
        // Watching has no tenant to key the enrollment's marker by, so Start
        // finishes on the watch-only one (`AppModel.markWatchOnlyComplete`).
        XCTAssertEqual(calls.last, .markWatchOnlyComplete, "Start finishes watching")
    }

    /// A passkey chosen on Join is not an account yet: its sheets open once
    /// the daemon runs and may close or sign out, so Automatic waits for the
    /// passkey the daemon holds. Start with Automatic grants nothing for it.
    func test_aPasskeyNotYetCreatedCannotChooseAutomatic() {
        XCTAssertFalse(FirstRunNavigation.canChooseAutomatic(FirstRunState(account: .passkeyChosen)))
        XCTAssertFalse(FirstRunNavigation.canChooseAutomatic(FirstRunState(account: .none)))
        XCTAssertTrue(FirstRunNavigation.canChooseAutomatic(FirstRunState(account: .passkey(name: ""))))

        var state = FirstRunState(step: .join)
        state.account = .passkeyChosen
        state.sharing = .automatic
        state.scopes = ["research"]
        let calls = FirstRunPlan.calls(for: state, at: .start)
        XCTAssertFalse(calls.contains { if case .grantAutomatic = $0 { return true } else { return false } })
        // Its Start reopens the sheets and finishes nothing (Kristi's #1235 I1).
        XCTAssertEqual(calls, [.openPasskeySheets])
    }

    func test_backKeepsEveryAnswer() {
        var state = FirstRunState(step: .uses)
        state.invite = "INVITE-1"
        state.toolAnswers[.codex] = .off
        state.scopes = ["research"]
        let back = FirstRunNavigation.back(state)
        XCTAssertEqual(back.step, .tools)
        var expected = state
        expected.step = .tools
        XCTAssertEqual(back, expected)
        XCTAssertEqual(FirstRunNavigation.back(FirstRunNavigation.back(back)).step, .welcome)
        XCTAssertEqual(FirstRunNavigation.next(back).step, .uses)
    }

    /// The core's ready answer belongs to the Start it was asked for. Back
    /// clears it, so coming back to Uses goes through the disclosures again.
    func test_backClearsTheGrantMarker() {
        var state = FirstRunState(step: .join, account: .nearAI, scopes: ["research"])
        state.sharing = .automatic
        state.grantReady = true
        let back = FirstRunNavigation.back(state)
        XCTAssertFalse(back.grantReady)
        XCTAssertEqual(back.sharing, .automatic, "the answer itself is kept")
        XCTAssertFalse(FirstRunNavigation.next(back).grantReady)
    }

    func test_aDeadInviteReturnsToJoinWithAnswersKept() {
        var state = FirstRunState(step: .join)
        state.invite = "INVITE-DEAD"
        state.account = .nearAI
        state.toolAnswers[.claudeCode] = .watch(path: "/Users/someone/.claude/projects")
        state.toolAnswers[.codex] = .off
        state.daemonStarted = true
        state.startedSettingsJSON = state.sessionRoots.settingsJSON()

        let returned = FirstRunNavigation.returnToJoin(afterDeadInvite: state)
        XCTAssertEqual(returned.step, .join)
        XCTAssertNil(returned.enrolledInvite)
        XCTAssertTrue(returned.daemonStarted)
        XCTAssertEqual(returned, state, "folder answers and the invite are kept")

        var fixed = returned
        fixed.invite = "INVITE-NEW"
        XCTAssertEqual(
            FirstRunPlan.calls(for: fixed, at: .signIn),
            [.lookupInvite("INVITE-NEW"), .enroll("INVITE-NEW"), .signInNearAI],
            "the daemon is already running, so it is not started again"
        )
    }

    /// Kristi's review of #1261: a near.ai sign-in or enrollment that keeps
    /// failing clears the near.ai choice on Join, so the person chooses
    /// again; every other answer is kept, and the daemon is not started
    /// twice.
    func test_aFailedNearAISignInReturnsToJoinWithTheChoiceCleared() {
        var state = FirstRunState(step: .join)
        state.account = .nearAI
        state.toolAnswers[.claudeCode] = .watch(path: "/Users/someone/.claude/projects")
        state.toolAnswers[.codex] = .off
        state.rules["repo-1"] = .ask
        state.scopes = ["traces"]
        state.daemonStarted = true
        state.startedSettingsJSON = state.sessionRoots.settingsJSON()

        let returned = FirstRunNavigation.returnToJoin(afterNearAIFailure: state)
        XCTAssertEqual(returned.step, .join)
        XCTAssertEqual(returned.account, .none, "near.ai is not kept as the answer")
        var expected = state
        expected.account = .none
        XCTAssertEqual(returned, expected, "every other answer is kept")

        // Until near.ai is chosen again, no sign-in signs in or enrolls, so
        // a session the daemon still keeps is never reused unasked.
        XCTAssertEqual(FirstRunPlan.calls(for: returned, at: .signIn), [])
        var watching = returned
        watching.account = .watchOnly
        XCTAssertEqual(FirstRunPlan.calls(for: watching, at: .signIn), [])
        var passkey = returned
        passkey.account = .passkeyChosen
        XCTAssertEqual(FirstRunPlan.calls(for: passkey, at: .signIn), [.openPasskeySheets])
        var again = returned
        again.account = .nearAI
        XCTAssertEqual(FirstRunPlan.calls(for: again, at: .signIn), [.nearAILogin, .enrollNearAI])
    }

    /// Only a near.ai answer is cleared: the function never drops a held
    /// account.
    func test_aNearAIFailureKeepsAnyOtherAccount() {
        var state = FirstRunState(step: .join)
        state.account = .passkey(name: "Mac")
        XCTAssertEqual(FirstRunNavigation.returnToJoin(afterNearAIFailure: state).account, .passkey(name: "Mac"))
    }

    func test_aDeadInviteKeepsAnEarlierEnrolment() {
        var state = FirstRunState(step: .join)
        state.invite = "INVITE-2"
        state.enrolledInvite = "INVITE-1"
        state.daemonStarted = true
        let returned = FirstRunNavigation.returnToJoin(afterDeadInvite: state)
        XCTAssertEqual(returned.enrolledInvite, "INVITE-1", "the daemon still holds that enrolment")
    }

    /// The added row and the tool's own row are answered apart: answering
    /// one never removes or rewrites the other.
    func test_anAddedFolderAndItsToolsRowAreAnsweredApart() {
        var state = FirstRunState(step: .tools)
        state.add(AddedFolder(kind: .source(.codex), path: "/Volumes/moved/codex"))
        state.answer(.codex, .off)
        XCTAssertEqual(state.addedFolders, [AddedFolder(kind: .source(.codex), path: "/Volumes/moved/codex")])
        state.answerAdded(path: "/Volumes/moved/codex", watched: true)
        XCTAssertEqual(state.toolAnswers[.codex], .off)
        XCTAssertEqual(state.sessionRoots.codex, .watch(path: "/Volumes/moved/codex"))
    }

    /// One folder is one thing: adding it again as another kind replaces the
    /// first answer instead of declaring the folder under both.
    func test_aFolderAddedAgainAsAnotherKindReplacesTheFirst() {
        let path = "/Users/someone/exports"
        var state = FirstRunState(step: .tools)
        state.add(AddedFolder(kind: .trajectory, path: path))
        state.add(AddedFolder(kind: .source(.opencode), path: path))
        XCTAssertEqual(state.addedFolders, [AddedFolder(kind: .source(.opencode), path: path)])
        XCTAssertEqual(state.sessionRoots.trajectory, .undecided)
        XCTAssertEqual(state.sessionRoots.opencode, .undecided, "unanswered until the person answers")
        state.answerAdded(path: path, watched: true)
        XCTAssertEqual(state.sessionRoots.opencode, .watch(path: path))

        state.add(AddedFolder(kind: .trajectory, path: path))
        XCTAssertEqual(state.addedFolders, [AddedFolder(kind: .trajectory, path: path)])
        XCTAssertEqual(state.sessionRoots.opencode, .undecided)
    }

    /// The trajectory folder has no tool row, so it is withdrawn on its own.
    /// That a key withdrawn after a start is sent `off` is
    /// `FirstRunPlanTests.test_aFolderWithdrawnAfterStartIsTurnedOff`.
    func test_theTrajectoryFolderCanBeWithdrawn() {
        var state = FirstRunState(step: .tools)
        state.add(AddedFolder(kind: .source(.opencode), path: "/o"))
        state.add(AddedFolder(kind: .trajectory, path: "/t"))
        state.withdrawTrajectory()
        XCTAssertEqual(state.addedFolders, [AddedFolder(kind: .source(.opencode), path: "/o")])
        XCTAssertEqual(state.sessionRoots.trajectory, .undecided)
    }

    func test_stateSurvivesARoundTrip() throws {
        var state = FirstRunState(step: .uses)
        state.invite = "INVITE-1"
        state.issuerHost = "issuer.example"
        state.account = .passkey(name: "Laptop")
        state.toolAnswers = [.claudeCode: .watch(path: "/a"), .codex: .off, .cline: .undecided]
        state.addedFolders = [
            AddedFolder(kind: .trajectory, path: "/t"),
            AddedFolder(kind: .source(.opencode), path: "/o"),
        ]
        state.rules = ["p1": .autoUpload, "p2": .ignore, "p3": .ask]
        state.pastSelections = ["p1": ["s1", "s2"]]
        state.scopes = ["research"]
        state.sharing = .automatic
        state.privateAI = true
        state.witnessSigningAddress = "witness-1"
        state.grantReady = true
        state.daemonStarted = true
        state.startedSettingsJSON = "{}"
        state.enrolledInvite = "INVITE-1"
        state.signedIn = true

        let data = try JSONEncoder().encode(state)
        XCTAssertEqual(try JSONDecoder().decode(FirstRunState.self, from: data), state)
    }

    /// Kristi's #1235 B1 floor: Automatic and Start read an enrollment the
    /// daemon holds, not an account answer. near.ai holds one once its
    /// invite enrolled; a passkey only once Verify bound it.
    func test_automaticAndStartNeedARealEnrolment() {
        func state(_ account: AccountAnswer, enrolledInvite: String? = nil) -> FirstRunState {
            var state = FirstRunState(step: .join)
            state.account = account
            state.enrolledInvite = enrolledInvite
            state.scopes = ["required"]
            state.sharingChosen = true
            return state
        }
        for account: AccountAnswer in [.none, .watchOnly, .passkeyChosen, .nearAI] {
            XCTAssertFalse(state(account).holdsEnrolment, "\(account)")
            XCTAssertFalse(FirstRunNavigation.canChooseAutomatic(state(account)), "\(account)")
        }
        for held in [state(.nearAI, enrolledInvite: "INVITE-1"), state(.passkey(name: "")), state(.enrolled)] {
            XCTAssertTrue(held.holdsEnrolment, "\(held.account)")
            XCTAssertTrue(FirstRunNavigation.canChooseAutomatic(held), "\(held.account)")
            XCTAssertTrue(FirstRunNavigation.canContinue(held, candidates: [], requiredScope: "required"))
        }
        // Start is offered to watching only and to a chosen passkey, whose
        // Start reopens the sheets; never to an account without an enrollment.
        XCTAssertTrue(FirstRunNavigation.canContinue(state(.watchOnly), candidates: [], requiredScope: "required"))
        XCTAssertTrue(FirstRunNavigation.canContinue(state(.passkeyChosen), candidates: [], requiredScope: "required"))
        XCTAssertFalse(FirstRunNavigation.canContinue(state(.nearAI), candidates: [], requiredScope: "required"))
    }

    // MARK: Welcome back (P-7), Ron's review of #1235 item 4

    private func remembered(
        _ state: String = "none", count: Int? = 1, name: String? = "Home"
    ) -> NativePasskeyState {
        NativePasskeyState(state: state, passkeyCount: count, rememberedName: name, nearAiConnected: nil)
    }

    /// A returning person: the first run has reached Join, the last step,
    /// with nothing answered, nobody is signed in, and this Mac remembers a
    /// passkey.
    func test_welcomeBackOpensForAReturningPersonOnJoin() {
        let join = FirstRunState(step: .join, daemonStarted: true)
        XCTAssertTrue(FirstRunNavigation.opensWelcomeBack(join, passkeys: remembered(), completed: false))
        // A remembered passkey without a name is still a returning person.
        XCTAssertTrue(
            FirstRunNavigation.opensWelcomeBack(join, passkeys: remembered(name: nil), completed: false))
    }

    func test_welcomeBackStaysShutForAnyoneElse() {
        let join = FirstRunState(step: .join, daemonStarted: true)
        // Nothing remembered here, or the daemon cannot say.
        XCTAssertFalse(FirstRunNavigation.opensWelcomeBack(join, passkeys: remembered(count: 0), completed: false))
        XCTAssertFalse(FirstRunNavigation.opensWelcomeBack(join, passkeys: remembered(count: nil), completed: false))
        XCTAssertFalse(FirstRunNavigation.opensWelcomeBack(join, passkeys: nil, completed: false))
        // Already signed in, or the session cannot be read: not a sign-in to offer.
        for state in ["unbound", "bound", "legacy", "closed", "unknown"] {
            XCTAssertFalse(
                FirstRunNavigation.opensWelcomeBack(join, passkeys: remembered(state), completed: false), state)
        }
        // Finished, or past Join.
        XCTAssertFalse(FirstRunNavigation.opensWelcomeBack(join, passkeys: remembered(), completed: true))
        var later = join
        later.step = .tools
        XCTAssertFalse(FirstRunNavigation.opensWelcomeBack(later, passkeys: remembered(), completed: false))
        // An account is already answered or held on Join.
        for account: AccountAnswer in [.watchOnly, .nearAI, .passkeyChosen, .passkey(name: "x"), .enrolled] {
            var answered = join
            answered.account = account
            XCTAssertFalse(
                FirstRunNavigation.opensWelcomeBack(answered, passkeys: remembered(), completed: false),
                "\(account)")
        }
    }
}
