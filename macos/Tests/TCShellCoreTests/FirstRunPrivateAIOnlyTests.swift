import XCTest

@testable import TCShellCore

/// Private AI only (tester report, owner-approved 2026-11-04): a person who
/// installed the app for Private AI finishes setup signed in to NEAR AI,
/// with nothing watched, queued, consented or sent. Sharing is added later
/// from Settings.
final class FirstRunPrivateAIOnlyTests: XCTestCase {
    /// Tools answered with a tool watched, Customize used: rules, past
    /// sessions and an added folder of exported traces.
    private func onTools() -> FirstRunState {
        var state = FirstRunState(step: .tools)
        state.toolAnswers[.claudeCode] = .watch(path: "/Users/someone/.claude/projects")
        state.toolAnswers[.codex] = .off
        state.toolAnswers[.opencode] = .watch(path: "/Users/someone/.local/share/opencode")
        state.add(AddedFolder(kind: .trajectory, path: "/Users/someone/exports"))
        state.rules["repo-a"] = .autoUpload
        state.rules["repo-b"] = .ask
        state.pastSelections["repo-a"] = ["s1", "s2"]
        state.scopes = ["research"]
        return state
    }

    /// `onTools` after Private AI only, signed in through NEAR AI with no
    /// invite: what Join's Start commits.
    private func signedIn() -> FirstRunState {
        var state = FirstRunNavigation.choosePrivateAIOnly(onTools())
        state.step = .join
        state.daemonStarted = true
        state.startedSettingsJSON = state.sessionRoots.settingsJSON()
        state.account = .nearAI
        state.nearAIEnrolled = true
        state.signedIn = true
        return state
    }

    private func declarations(_ json: String) throws -> [String: [String: String]] {
        try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: [String: String]])
    }

    func test_choosingPrivateAIOnlyWatchesNothingAndConsentsToNothing() throws {
        let chosen = FirstRunNavigation.choosePrivateAIOnly(onTools())
        XCTAssertTrue(chosen.privateAIOnly)
        XCTAssertFalse(chosen.watchesAnything, "no tool and no folder is watched")
        XCTAssertTrue(chosen.scopes.isEmpty, "no data use is chosen")
        XCTAssertTrue(chosen.rules.isEmpty, "no folder rule, so no Automatic folder")
        XCTAssertTrue(chosen.pastSelections.isEmpty, "no past session is included")
        XCTAssertFalse(chosen.grantReady)
        XCTAssertFalse(chosen.privateAI, "Settings turns Private AI on (#1685); setup sends no switch")
        XCTAssertEqual(chosen.step, .tools, "the commit that starts the daemon moves the step")

        // The daemon is told to watch nothing: Claude Code, Codex and
        // OpenCode off, no exported traces declared.
        let object = try declarations(try XCTUnwrap(chosen.sessionRoots.settingsJSON()))
        XCTAssertEqual(object.count, 3)
        for (key, declaration) in object {
            XCTAssertEqual(declaration, ["mode": "off"], key)
        }
        XCTAssertNil(object[SessionRoots.trajectorySettingsKey])
    }

    /// Private AI only skips Uses and Traces: there is nothing to share,
    /// so nothing to choose or estimate. Private AI and the sign-in stay.
    func test_privateAIOnlySkipsUsesAndTraces() {
        let chosen = FirstRunNavigation.choosePrivateAIOnly(onTools())
        XCTAssertEqual(FirstRunNavigation.steps(for: chosen), [.welcome, .tools, .privateAI, .join])
        XCTAssertEqual(FirstRunNavigation.threadSteps(for: chosen), [.tools, .privateAI, .join])
        let forward = FirstRunNavigation.next(chosen)
        XCTAssertEqual(forward.step, .privateAI)
        XCTAssertEqual(FirstRunNavigation.next(forward).step, .join)
        XCTAssertEqual(FirstRunNavigation.back(forward).step, .tools)
        var onJoin = chosen
        onJoin.step = .join
        XCTAssertEqual(FirstRunNavigation.back(onJoin).step, .privateAI)
        XCTAssertEqual(FirstRunNavigation.back(to: .tools, from: onJoin).step, .tools)
        XCTAssertEqual(FirstRunNavigation.back(to: .uses, from: onJoin).step, .join, "Uses is not on this path")
        // Sharing runs the whole path, as before.
        XCTAssertEqual(FirstRunNavigation.steps(for: onTools()), FirstRunNavigation.steps)
        XCTAssertEqual(FirstRunNavigation.threadSteps(for: onTools()), FirstRunNavigation.threadSteps)
    }

    /// Continue on Tools after coming back to it takes sharing up again:
    /// the path runs through Uses, where a choice is made afresh.
    func test_continueOnToolsTakesSharingUpAgain() {
        var chosen = FirstRunNavigation.choosePrivateAIOnly(onTools())
        chosen.answer(.claudeCode, .watch(path: "/Users/someone/.claude/projects"))
        let sharing = FirstRunNavigation.continueSharing(chosen)
        XCTAssertFalse(sharing.privateAIOnly)
        XCTAssertEqual(FirstRunNavigation.next(sharing).step, .uses, "the person picks Ask or Automatic on Uses")
        XCTAssertEqual(FirstRunNavigation.continueSharing(onTools()), onTools(), "a sharing path is left as it is")
    }

    /// Start for Private AI only sends the completion marker and nothing
    /// that consents, arms or includes anything.
    func test_startSendsOnlyTheMarker() {
        XCTAssertEqual(FirstRunPlan.calls(for: signedIn(), at: .start), [.markComplete])
    }

    /// The daemon still watching something (an earlier Customize started
    /// it with tools ticked) is told to watch nothing before the marker.
    func test_startTurnsOffAnyWatchingFirst() throws {
        var state = signedIn()
        state.startedSettingsJSON = onTools().sessionRoots.settingsJSON()
        let calls = FirstRunPlan.calls(for: state, at: .start)
        XCTAssertEqual(calls.count, 2, "\(calls)")
        XCTAssertEqual(calls.last, .markComplete)
        guard case .setSourceSettings(let json)? = calls.first else {
            return XCTFail("expected setSourceSettings, got \(calls)")
        }
        let object = try declarations(json)
        XCTAssertFalse(object.isEmpty)
        for (key, declaration) in object {
            XCTAssertEqual(declaration, ["mode": "off"], key)
        }
        XCTAssertEqual(object[SessionRoots.trajectorySettingsKey], ["mode": "off"], "the withdrawn folder goes off")
    }

    /// Fail closed: Private AI only that would watch something, or that
    /// holds no enrollment, starts nothing at all.
    func test_startRefusesWithoutAnAccountOrWhileWatching() {
        var noAccount = signedIn()
        noAccount.account = .none
        noAccount.nearAIEnrolled = false
        XCTAssertEqual(FirstRunPlan.calls(for: noAccount, at: .start), [])
        XCTAssertFalse(FirstRunNavigation.canContinue(noAccount, candidates: [], requiredScope: nil))

        var watching = signedIn()
        watching.answer(.claudeCode, .watch(path: "/Users/someone/.claude/projects"))
        XCTAssertEqual(FirstRunPlan.calls(for: watching, at: .start), [])
        XCTAssertFalse(FirstRunNavigation.canContinue(watching, candidates: [], requiredScope: nil))
    }

    /// Join's Start for Private AI only needs the account, not the
    /// required use: no data use is consented to.
    func test_joinContinuesWithAnAccountAndNoRequiredUse() {
        XCTAssertTrue(FirstRunNavigation.canContinue(signedIn(), candidates: [], requiredScope: nil))
        XCTAssertTrue(FirstRunNavigation.canContinue(signedIn(), candidates: [], requiredScope: "debugging_evaluation"))
    }

    /// The path survives a restored first run.
    func test_privateAIOnlyRoundTrips() throws {
        let state = signedIn()
        let decoded = try JSONDecoder().decode(FirstRunState.self, from: JSONEncoder().encode(state))
        XCTAssertEqual(decoded, state)
        XCTAssertTrue(decoded.privateAIOnly)
    }
}
