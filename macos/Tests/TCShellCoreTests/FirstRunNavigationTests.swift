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

    func test_quickHasJoinFoldersUses() {
        XCTAssertEqual(FirstRunNavigation.steps(for: .quick), [.join, .folders, .uses])
    }

    func test_customHasJoinToolsRulesUses() {
        XCTAssertEqual(FirstRunNavigation.steps(for: .custom), [.join, .tools, .rules, .uses])
    }

    func test_switchingTierKeepsTheEquivalentScreen() {
        var state = FirstRunState(tier: .quick, step: .folders)
        state = FirstRunNavigation.switchTier(state, to: .custom)
        XCTAssertEqual(state.tier, .custom)
        XCTAssertEqual(state.step, .tools)

        state.step = .rules
        state = FirstRunNavigation.switchTier(state, to: .quick)
        XCTAssertEqual(state.step, .folders)
    }

    /// Spec rule 1, the owner's reversal in Ron's design review of #1235:
    /// a tool not found on this Mac is not asked, and Continue counts only
    /// the tools found here.
    func test_aMissingToolIsNotAsked() {
        let candidates = [candidate(.claudeCode), candidate(.codex, exists: false), candidate(.cline, exists: false)]
        var state = FirstRunState(tier: .quick, step: .folders)
        state.recordDiscovery(candidates)
        XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))

        state.toolAnswers[.claudeCode] = .watch(path: "/Users/someone/claude-code")
        XCTAssertTrue(
            FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil),
            "a tool that is not installed is not asked")
        XCTAssertNil(state.toolAnswers[.codex], "nothing is written for the missing tool")
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
        var state = FirstRunState(tier: .quick, step: .folders)
        state.recordDiscovery(candidates)
        state.toolAnswers[.codex] = .watch(path: "/Users/someone/codex")
        XCTAssertTrue(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))

        let json = try XCTUnwrap(state.sessionRoots.settingsJSON())
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: [String: String]])
        XCTAssertEqual(object["claude_source"], ["mode": "off"])
        XCTAssertEqual(object["codex_source"]?["mode"], "watch")
        XCTAssertNil(object["cline_source"])
        XCTAssertEqual(
            FirstRunPlan.calls(for: state, at: .leaveRoots).first, .startDaemon(settingsJSON: json))

        // Found again later (installed while the app was away): it is asked.
        state.recordDiscovery([candidate(.claudeCode), candidate(.codex), candidate(.cline, exists: false)])
        XCTAssertFalse(
            FirstRunNavigation.canContinue(
                state, candidates: [candidate(.claudeCode), candidate(.codex)], requiredScope: nil))
        XCTAssertNil(state.sessionRoots.settingsJSON())
    }

    func test_anOptionalOfferedToolMustBeAnsweredToo() {
        let candidates = [candidate(.claudeCode), candidate(.codex), candidate(.cline)]
        var state = FirstRunState(tier: .custom, step: .tools)
        state.recordDiscovery(candidates)
        state.toolAnswers[.claudeCode] = .off
        state.toolAnswers[.codex] = .off
        XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))
        state.toolAnswers[.cline] = .off
        XCTAssertTrue(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))
    }

    /// Ron's review of #1235, item 3: a folder added with "Not seeing your
    /// tool above?" starts unanswered, in a row of its own. Recognising its
    /// layout stays; answering for the person does not. Nothing is written
    /// to the matched tool's row.
    func test_anAddedFolderStartsUnanswered() {
        let candidates = [candidate(.claudeCode), candidate(.codex)]
        var state = FirstRunState(tier: .custom, step: .tools)
        state.toolAnswers[.claudeCode] = .off
        state.toolAnswers[.codex] = .off
        state.add(AddedFolder(kind: .source(.codex), path: "/Volumes/moved/codex"))
        XCTAssertNil(state.addedFolders.first?.watched)
        XCTAssertEqual(state.toolAnswers[.codex], .off, "the tool's own row keeps its answer")
        XCTAssertEqual(state.sessionRoots.codex, .off, "an unanswered folder declares nothing")
        XCTAssertFalse(
            FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil),
            "an added folder is a row to answer like any other")

        state.answerAdded(path: "/Volumes/moved/codex", watched: true)
        XCTAssertTrue(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))
        XCTAssertEqual(state.sessionRoots.codex, .watch(path: "/Volumes/moved/codex"))

        // "I don't use it" on the added row declares nothing for the tool:
        // its own row's answer stands.
        state.answerAdded(path: "/Volumes/moved/codex", watched: false)
        XCTAssertTrue(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))
        XCTAssertEqual(state.sessionRoots.codex, .off)
        XCTAssertEqual(state.addedFolders.count, 1, "the row stays, answered")
    }

    /// The rule for one tool watched in two rows (Ron's item 3 left it
    /// open): the daemon watches one folder per tool, so Continue is held
    /// while a tool's own row and a folder added for it both read Watch,
    /// and the added row says why. Nothing is un-answered for the person.
    func test_oneToolWatchedInTwoRowsHoldsContinue() {
        let candidates = [candidate(.claudeCode), candidate(.codex)]
        var state = FirstRunState(tier: .custom, step: .tools)
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

    /// With no candidates discovered, every offered tool is trivially
    /// answered; the declaration the daemon starts with is what keeps
    /// Continue closed until Claude Code and Codex are answered.
    func test_noCandidatesStillNeedsADeclaration() {
        for (tier, step) in [(FirstRunTier.quick, FirstRunStep.folders), (.custom, .tools)] {
            var state = FirstRunState(tier: tier, step: step)
            XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: [], requiredScope: nil))

            state.toolAnswers[.claudeCode] = .off
            XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: [], requiredScope: nil))

            state.toolAnswers[.codex] = .off
            XCTAssertTrue(FirstRunNavigation.canContinue(state, candidates: [], requiredScope: nil))
        }
    }

    /// The required use is always included (owner, 2026-10-08), so Start
    /// waits for it to be known, never for a tick.
    func test_startWaitsForTheRequiredUseToBeKnownNotTicked() {
        var state = FirstRunState(tier: .quick, step: .uses)
        state.account = .enrolled
        state.scopes = ["evaluation"]
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
            FirstRunPlan.calls(for: included, at: .start).first, .setConsentScopes(["evaluation", "research"]))
    }

    func test_joinWaitsForAnAccountAnswer() {
        var state = FirstRunState()
        XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: [], requiredScope: nil))
        state.account = .watchOnly
        XCTAssertTrue(FirstRunNavigation.canContinue(state, candidates: [], requiredScope: nil))
    }

    func test_watchOnlyCannotChooseAutomatic() {
        XCTAssertEqual(FirstRunNavigation.sharingPaths(for: FirstRunState(account: .watchOnly)), [.askMe])
        XCTAssertEqual(
            FirstRunNavigation.sharingPaths(for: FirstRunState(account: .nearAI, enrolledInvite: "INVITE-1")),
            [.automatic, .askMe])
        XCTAssertEqual(
            FirstRunNavigation.sharingPaths(for: FirstRunState(account: .passkey(name: "Laptop"))), [.automatic, .askMe])
        XCTAssertEqual(FirstRunNavigation.sharingPaths(for: FirstRunState(account: .enrolled)), [.automatic, .askMe])

        var state = FirstRunState(tier: .quick, step: .uses)
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
        XCTAssertEqual(FirstRunNavigation.sharingPaths(for: FirstRunState(account: .passkeyChosen)), [.askMe])
        XCTAssertEqual(FirstRunNavigation.sharingPaths(for: FirstRunState(account: .none)), [.askMe])
        XCTAssertFalse(FirstRunNavigation.canChooseAutomatic(FirstRunState(account: .passkeyChosen)))
        XCTAssertTrue(FirstRunNavigation.canChooseAutomatic(FirstRunState(account: .passkey(name: ""))))

        var state = FirstRunState(tier: .quick, step: .uses)
        state.account = .passkeyChosen
        state.sharing = .automatic
        state.scopes = ["research"]
        let calls = FirstRunPlan.calls(for: state, at: .start)
        XCTAssertFalse(calls.contains { if case .grantAutomatic = $0 { return true } else { return false } })
        // Its Start reopens the sheets and finishes nothing (Kristi's #1235 I1).
        XCTAssertEqual(calls, [.openPasskeySheets])
    }

    func test_backKeepsEveryAnswer() {
        var state = FirstRunState(tier: .quick, step: .uses)
        state.invite = "INVITE-1"
        state.toolAnswers[.codex] = .off
        state.scopes = ["research"]
        let back = FirstRunNavigation.back(state)
        XCTAssertEqual(back.step, .folders)
        var expected = state
        expected.step = .folders
        XCTAssertEqual(back, expected)
        XCTAssertEqual(FirstRunNavigation.back(FirstRunNavigation.back(back)).step, .join)
        XCTAssertEqual(FirstRunNavigation.next(back).step, .uses)
    }

    /// The core's ready answer belongs to the Start it was asked for. Back
    /// clears it, so coming back to Uses goes through the disclosures again.
    func test_backClearsTheGrantMarker() {
        var state = FirstRunState(tier: .quick, step: .uses, account: .nearAI, scopes: ["research"])
        state.sharing = .automatic
        state.grantReady = true
        let back = FirstRunNavigation.back(state)
        XCTAssertFalse(back.grantReady)
        XCTAssertEqual(back.sharing, .automatic, "the answer itself is kept")
        XCTAssertFalse(FirstRunNavigation.next(back).grantReady)
    }

    func test_aDeadInviteReturnsToJoinWithAnswersKept() {
        var state = FirstRunState(tier: .quick, step: .folders)
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
        var expected = state
        expected.step = .join
        XCTAssertEqual(returned, expected, "folder answers and the invite are kept")

        var fixed = returned
        fixed.invite = "INVITE-NEW"
        XCTAssertEqual(
            FirstRunPlan.calls(for: fixed, at: .leaveRoots),
            [.lookupInvite("INVITE-NEW"), .enroll("INVITE-NEW"), .signInNearAI],
            "the daemon is already running, so it is not started again"
        )
    }

    /// Kristi's review of #1261: with Back gone, a near.ai sign-in or
    /// enrollment that keeps failing must not hold the person on Folders or
    /// Tools. They go back to Join with the near.ai choice cleared, so they
    /// choose again (near.ai, a passkey, or watch only); every other answer
    /// is kept, and the daemon is not started twice.
    func test_aFailedNearAISignInReturnsToJoinWithTheChoiceCleared() {
        var state = FirstRunState(tier: .custom, step: .tools)
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
        expected.step = .join
        expected.account = .none
        XCTAssertEqual(returned, expected, "every other answer is kept")

        // Until near.ai is chosen again, no Continue signs in or enrolls, so
        // a session the daemon still keeps is never reused unasked.
        XCTAssertEqual(FirstRunPlan.calls(for: returned, at: .leaveRoots), [])
        var watching = returned
        watching.account = .watchOnly
        XCTAssertEqual(FirstRunPlan.calls(for: watching, at: .leaveRoots), [])
        var passkey = returned
        passkey.account = .passkeyChosen
        XCTAssertEqual(FirstRunPlan.calls(for: passkey, at: .leaveRoots), [.openPasskeySheets])
        var again = returned
        again.account = .nearAI
        XCTAssertEqual(FirstRunPlan.calls(for: again, at: .leaveRoots), [.nearAILogin, .enrollNearAI])
    }

    /// Only a near.ai answer is cleared: the function never drops a held
    /// account.
    func test_aNearAIFailureKeepsAnyOtherAccount() {
        var state = FirstRunState(tier: .quick, step: .folders)
        state.account = .passkey(name: "Mac")
        XCTAssertEqual(FirstRunNavigation.returnToJoin(afterNearAIFailure: state).account, .passkey(name: "Mac"))
    }

    func test_aDeadInviteKeepsAnEarlierEnrolment() {
        var state = FirstRunState(tier: .quick, step: .folders)
        state.invite = "INVITE-2"
        state.enrolledInvite = "INVITE-1"
        state.daemonStarted = true
        let returned = FirstRunNavigation.returnToJoin(afterDeadInvite: state)
        XCTAssertEqual(returned.enrolledInvite, "INVITE-1", "the daemon still holds that enrolment")
    }

    /// The added row and the tool's own row are answered apart: answering
    /// one never removes or rewrites the other.
    func test_anAddedFolderAndItsToolsRowAreAnsweredApart() {
        var state = FirstRunState(tier: .custom, step: .tools)
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
        var state = FirstRunState(tier: .custom, step: .tools)
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
        var state = FirstRunState(tier: .custom, step: .tools)
        state.add(AddedFolder(kind: .source(.opencode), path: "/o"))
        state.add(AddedFolder(kind: .trajectory, path: "/t"))
        state.withdrawTrajectory()
        XCTAssertEqual(state.addedFolders, [AddedFolder(kind: .source(.opencode), path: "/o")])
        XCTAssertEqual(state.sessionRoots.trajectory, .undecided)
    }

    func test_stateSurvivesARoundTrip() throws {
        var state = FirstRunState(tier: .custom, step: .rules)
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
            var state = FirstRunState(tier: .quick, step: .uses)
            state.account = account
            state.enrolledInvite = enrolledInvite
            state.scopes = ["required"]
            return state
        }
        for account: AccountAnswer in [.none, .watchOnly, .passkeyChosen, .nearAI] {
            XCTAssertFalse(state(account).holdsEnrolment, "\(account)")
            XCTAssertFalse(FirstRunNavigation.canChooseAutomatic(state(account)), "\(account)")
            XCTAssertEqual(FirstRunNavigation.sharingPaths(for: state(account)), [.askMe], "\(account)")
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

    /// A returning person: the first run is on Join with nothing answered,
    /// nobody is signed in, and this Mac remembers a passkey.
    func test_welcomeBackOpensForAReturningPersonOnJoin() {
        let join = FirstRunState(tier: .quick, step: .join, daemonStarted: true)
        XCTAssertTrue(FirstRunNavigation.opensWelcomeBack(join, passkeys: remembered(), completed: false))
        // A remembered passkey without a name is still a returning person.
        XCTAssertTrue(
            FirstRunNavigation.opensWelcomeBack(join, passkeys: remembered(name: nil), completed: false))
    }

    func test_welcomeBackStaysShutForAnyoneElse() {
        let join = FirstRunState(tier: .quick, step: .join, daemonStarted: true)
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
        later.step = .folders
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
