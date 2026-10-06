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

    func test_aMissingToolMustBeAnswered() {
        let candidates = [candidate(.claudeCode), candidate(.codex, exists: false)]
        var state = FirstRunState(tier: .quick, step: .folders)
        state.toolAnswers[.claudeCode] = .watch(path: "/Users/someone/claude-code")

        XCTAssertFalse(
            FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil),
            "a tool that is not installed still needs an answer"
        )

        state.toolAnswers[.codex] = .undecided
        XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))

        state.toolAnswers[.codex] = .off
        XCTAssertTrue(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))
    }

    func test_anOptionalOfferedToolMustBeAnsweredToo() {
        let candidates = [candidate(.claudeCode), candidate(.codex), candidate(.cline, exists: false)]
        var state = FirstRunState(tier: .custom, step: .tools)
        state.toolAnswers[.claudeCode] = .off
        state.toolAnswers[.codex] = .off
        XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))
        state.toolAnswers[.cline] = .off
        XCTAssertTrue(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))
    }

    func test_anAddedFolderAnswersItsTool() {
        let candidates = [candidate(.claudeCode), candidate(.codex)]
        var state = FirstRunState(tier: .custom, step: .tools)
        state.toolAnswers[.claudeCode] = .off
        state.addedFolders = [AddedFolder(kind: .source(.codex), path: "/Volumes/moved/codex")]
        XCTAssertTrue(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))
        XCTAssertEqual(state.sessionRoots.codex, .watch(path: "/Volumes/moved/codex"))
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

    func test_startWaitsForTheRequiredUse() {
        var state = FirstRunState(tier: .quick, step: .uses)
        state.account = .enrolled
        state.scopes = ["evaluation"]
        XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: [], requiredScope: "research"))

        state.scopes.insert("research")
        XCTAssertTrue(FirstRunNavigation.canContinue(state, candidates: [], requiredScope: "research"))

        XCTAssertFalse(
            FirstRunNavigation.canContinue(state, candidates: [], requiredScope: nil),
            "with no required use known, Start stays disabled"
        )
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
        // Watching has no tenant to key the enrolment's marker by, so Start
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

    func test_aDeadInviteKeepsAnEarlierEnrolment() {
        var state = FirstRunState(tier: .quick, step: .folders)
        state.invite = "INVITE-2"
        state.enrolledInvite = "INVITE-1"
        state.daemonStarted = true
        let returned = FirstRunNavigation.returnToJoin(afterDeadInvite: state)
        XCTAssertEqual(returned.enrolledInvite, "INVITE-1", "the daemon still holds that enrolment")
    }

    func test_aLaterAnswerReplacesAnAddedFolder() {
        var state = FirstRunState(tier: .custom, step: .tools)
        state.add(AddedFolder(kind: .source(.codex), path: "/Volumes/moved/codex"))
        state.answer(.codex, .off)
        XCTAssertEqual(state.sessionRoots.codex, .off, "a later off is not turned back into a watch")
        XCTAssertEqual(state.addedFolders, [])

        state.add(AddedFolder(kind: .source(.codex), path: "/Volumes/moved/codex"))
        XCTAssertEqual(state.sessionRoots.codex, .watch(path: "/Volumes/moved/codex"))
        XCTAssertNil(state.toolAnswers[.codex])

        state.add(AddedFolder(kind: .source(.codex), path: "/Volumes/other/codex"))
        XCTAssertEqual(state.addedFolders, [AddedFolder(kind: .source(.codex), path: "/Volumes/other/codex")])
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

    /// Kristi's #1235 B1 floor: Automatic and Start read an enrolment the
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
        // Start reopens the sheets; never to an account without an enrolment.
        XCTAssertTrue(FirstRunNavigation.canContinue(state(.watchOnly), candidates: [], requiredScope: "required"))
        XCTAssertTrue(FirstRunNavigation.canContinue(state(.passkeyChosen), candidates: [], requiredScope: "required"))
        XCTAssertFalse(FirstRunNavigation.canContinue(state(.nearAI), candidates: [], requiredScope: "required"))
    }
}
