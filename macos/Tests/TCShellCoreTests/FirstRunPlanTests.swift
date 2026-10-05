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

    func test_watchOnlyJoinsNothing() throws {
        var state = answered()
        state.account = .watchOnly
        state.invite = ""
        let calls = FirstRunPlan.calls(for: state, at: .leaveRoots)
        XCTAssertEqual(calls.count, 1)
        _ = try settings(calls.first)
    }

    func test_aFailedStartKeepsTheInviteAndJoinsNothing() throws {
        let state = answered()
        let first = FirstRunPlan.calls(for: state, at: .leaveRoots)
        _ = try settings(first.first)

        // The start failed: the runner records nothing, so daemonStarted
        // stays false and the step stays on Folders.
        XCTAssertFalse(state.daemonStarted)
        XCTAssertFalse(state.enrolled)
        XCTAssertEqual(state.invite, "INVITE-1")
        XCTAssertEqual(state.step, .folders)

        let again = FirstRunPlan.calls(for: state, at: .leaveRoots)
        XCTAssertEqual(again, first, "the retry is the same plan, invite intact")
        XCTAssertTrue(again.contains(.enroll("INVITE-1")))
        _ = try settings(again.first)
    }

    func test_backAfterStartDoesNotEnrollTwice() {
        var state = answered()
        state.daemonStarted = true
        state.enrolled = true
        state.step = .uses
        state = FirstRunNavigation.back(state)
        state = FirstRunNavigation.back(state)
        XCTAssertEqual(state.step, .join)
        XCTAssertEqual(state.invite, "INVITE-1")
        state = FirstRunNavigation.next(state)

        let calls = FirstRunPlan.calls(for: state, at: .leaveRoots)
        XCTAssertFalse(calls.contains { if case .startDaemon = $0 { return true } else { return false } })
        XCTAssertFalse(calls.contains { if case .enroll = $0 { return true } else { return false } })
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
