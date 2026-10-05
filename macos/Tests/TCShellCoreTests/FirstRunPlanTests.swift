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
        var state = answered(tier: .custom)
        state.step = .uses
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
        var state = answered()
        state.step = .uses
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
        var state = answered()
        state.step = .uses
        state.scopes = ["research"]
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .start),
            [.setConsentScopes(["research"]), .markComplete])
    }

    func test_quickNeverSetsARuleOrIncludesPastSessions() {
        var state = answered()
        state.step = .uses
        state.scopes = ["research"]
        state.rules = ["p1": .autoUpload]
        state.pastSelections = ["p1": ["s1"]]
        state.privateAI = true
        let calls = FirstRunPlan.calls(for: state, at: .start)
        XCTAssertEqual(calls, [.setConsentScopes(["research"]), .markComplete])
    }

    func test_aNeverFolderIncludesNothing() {
        var state = answered(tier: .custom)
        state.step = .uses
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
}
