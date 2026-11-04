import Foundation
import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Private AI only through the runner and the screens (tester report,
/// owner-approved 2026-11-04), and Back on Join while the sign-in waits.
/// Every daemon call is recorded, so "nothing consented, queued or sent"
/// is read off the log.
@MainActor
final class FirstRunPrivateAIOnlyAppTests: XCTestCase {
    private func firstRunCopy() throws -> FirstRunCopy {
        try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
    }

    /// Tools with Claude Code ticked, and Codex not used.
    private func onTools() -> FirstRunState {
        var state = FirstRunState(step: .tools)
        state.toolAnswers[.claudeCode] = .watch(path: "/Users/someone/.claude/projects")
        state.toolAnswers[.codex] = .off
        return state
    }

    /// Calls that consent to, arm, include or send anything.
    private func shares(_ call: FirstRunCall) -> Bool {
        switch call {
        case .setAskEachTime, .setConsentScopes, .setProjectMode, .includePastSessions, .setPrivateAI,
            .grantAutomatic, .markWatchOnlyComplete, .lookupInvite, .enroll:
            return true
        case .startDaemon, .setSourceSettings, .signInNearAI, .nearAILogin, .enrollNearAI, .markComplete:
            return false
        }
    }

    private func declarations(_ json: String) throws -> [String: [String: String]] {
        try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: [String: String]])
    }

    private func yield(until condition: () -> Bool, file: StaticString = #filePath, line: UInt = #line) async {
        for _ in 0..<100_000 where !condition() { await Task.yield() }
        XCTAssertTrue(condition(), "never held", file: file, line: line)
    }

    /// The whole path: Private AI only on Tools starts the daemon watching
    /// nothing and moves to Private AI; Join signs in through NEAR AI; Start
    /// writes the completion marker and nothing else. The daemon was never
    /// told to watch a folder, so nothing can be queued, and no consent
    /// scope, Ask, grant, folder rule or past session was sent.
    func test_privateAIOnlyFinishesWithNothingWatchedConsentedOrSent() async throws {
        let daemon = RecordingFirstRunDaemon()
        let runner = FirstRunRunner(state: FirstRunNavigation.choosePrivateAIOnly(onTools()), daemon: daemon)

        await runner.commit(.leaveTools)
        XCTAssertNil(runner.failure)
        XCTAssertEqual(runner.state.step, .privateAI, "Uses and Traces are skipped")
        guard case .startDaemon(let json)? = daemon.log.first else { return XCTFail("\(daemon.log)") }
        for (key, declaration) in try declarations(json) {
            XCTAssertEqual(declaration, ["mode": "off"], key)
        }

        runner.state = FirstRunNavigation.next(runner.state)
        XCTAssertEqual(runner.state.step, .join)
        runner.state = JoinScreenLayout.chooseNearAI(runner.state)
        await runner.commit(.signIn)
        XCTAssertNil(runner.failure)
        XCTAssertTrue(runner.state.holdsEnrolment)

        runner.state = JoinScreenLayout.startState(runner.state, required: nil)
        await runner.commit(.start)
        XCTAssertNil(runner.failure)
        XCTAssertTrue(runner.completed)

        XCTAssertEqual(daemon.log, [.startDaemon(settingsJSON: json), .nearAILogin, .enrollNearAI, .markComplete])
        XCTAssertFalse(daemon.log.contains(where: shares), "\(daemon.log)")
        XCTAssertTrue(runner.state.scopes.isEmpty, "no data use is consented to")
        XCTAssertTrue(runner.state.rules.isEmpty, "no folder is Automatic")
        XCTAssertFalse(runner.state.watchesAnything, "nothing is watched, so nothing is queued")
        XCTAssertTrue(runner.state.privateAIOnly)
        XCTAssertFalse(runner.state.sharingChosen, "neither Ask nor Automatic was chosen")
    }

    /// Start for Private AI only never adds the required use: it consents
    /// to nothing. A sharing path still does.
    func test_startStateIncludesTheRequiredUseOnlyWhenSharing() {
        let required = ConsentScope(
            name: "debugging_evaluation", title: "Debugging", description: "", alwaysOn: true, grantsDataUse: true)
        var privateOnly = FirstRunNavigation.choosePrivateAIOnly(onTools())
        privateOnly.step = .join
        privateOnly.account = .nearAI
        privateOnly.nearAIEnrolled = true
        privateOnly.signedIn = true
        XCTAssertTrue(JoinScreenLayout.startState(privateOnly, required: required).scopes.isEmpty)

        var sharing = privateOnly
        sharing.privateAIOnly = false
        XCTAssertEqual(JoinScreenLayout.startState(sharing, required: required).scopes, ["debugging_evaluation"])
    }

    /// Join's action for Private AI only: "Get started", not "Start
    /// sharing"; on once signed in, with no required use or sharing words
    /// to wait on; and the sole-organization auto-start runs it, so the
    /// auto-start can never start sharing for this path.
    func test_joinStartsPrivateAIOnlyWithoutSharingWords() throws {
        let copy = try firstRunCopy()
        var state = FirstRunNavigation.choosePrivateAIOnly(onTools())
        state.step = .join
        state.daemonStarted = true
        state.startedSettingsJSON = state.sessionRoots.settingsJSON()
        XCTAssertEqual(JoinScreenLayout.footerTitle(state, copy: copy), copy.welcome.start)
        XCTAssertNotEqual(JoinScreenLayout.footerTitle(state, copy: copy), copy.uses.start)
        XCTAssertEqual(JoinScreenLayout.footerTitle(onTools(), copy: copy), copy.uses.start)

        func canStart(_ state: FirstRunState, committing: Bool = false) -> Bool {
            JoinScreenLayout.canStart(
                state, uses: copy.uses, requiredScope: nil, grant: nil, isCommitting: committing)
        }
        XCTAssertFalse(canStart(state), "sign-in is still required (owner, 2026-10-12)")
        state.account = .nearAI
        XCTAssertFalse(canStart(state), "near.ai chosen, not yet signed in")
        state.nearAIEnrolled = true
        state.signedIn = true
        XCTAssertTrue(canStart(state))
        XCTAssertFalse(canStart(state, committing: true))
        XCTAssertTrue(JoinScreenLayout.canAutomaticallyStart(
            state, organizationReady: true, attempted: false, starting: false, uses: copy.uses,
            requiredScope: nil, grant: nil, isCommitting: false, completed: false))
        XCTAssertEqual(UsesScreenLayout.startRoute(state), .commit, "no disclosure: nothing is shared")
        XCTAssertEqual(FirstRunPlan.calls(for: JoinScreenLayout.startState(state, required: nil), at: .start),
                       [.markComplete])
    }

    /// Tools offers Private AI only once discovery has answered, and not
    /// while a commit runs; it needs no tool ticked.
    func test_toolsOffersPrivateAIOnlyWithoutATool() {
        let rows = [SourceCandidate(source: .claudeCode, path: "/p", exists: true, sessionCount: 1, mostRecent: nil,
                                    relocatedByEnv: false)]
        XCTAssertTrue(ToolsScreenLayout.canChoosePrivateAIOnly(discovered: rows, pending: false, isCommitting: false))
        XCTAssertFalse(ToolsScreenLayout.canChoosePrivateAIOnly(discovered: nil, pending: false, isCommitting: false))
        XCTAssertFalse(ToolsScreenLayout.canChoosePrivateAIOnly(discovered: rows, pending: true, isCommitting: false))
        XCTAssertFalse(ToolsScreenLayout.canChoosePrivateAIOnly(discovered: rows, pending: false, isCommitting: true))
        XCTAssertFalse(
            ToolsScreenLayout.canChoosePrivateAIOnly(
                discovered: rows, pending: false, isCommitting: false, scanned: false))
    }

    /// The step thread names only the steps on the person's path.
    func test_theThreadFollowsThePath() throws {
        let copy = try firstRunCopy()
        var state = FirstRunNavigation.choosePrivateAIOnly(onTools())
        state.step = .privateAI
        XCTAssertEqual(copy.threadLabels(for: state), [copy.frame.stepTools, copy.frame.stepPrivateAi, copy.frame.stepJoin])
        XCTAssertEqual(FirstRunStageLayout.threadIndex(of: .privateAI, in: state), 1)
        XCTAssertEqual(copy.threadLabels(for: onTools()), copy.threadLabels)
        XCTAssertEqual(FirstRunStageLayout.threadIndex(of: .privateAI, in: onTools()), 3)
    }

    // MARK: - Back while signing in

    /// Back stays on Join while the sign-in runs, so nobody is trapped:
    /// enabled while the browser wait can be ended, shown but off for the
    /// short daemon calls around it, and hidden while Start runs.
    func test_backStaysOnJoinWhileSigningIn() {
        XCTAssertEqual(JoinScreenLayout.backWhileCommitting(committing: .signIn, signInWaiting: true), true)
        XCTAssertEqual(JoinScreenLayout.backWhileCommitting(committing: .signIn, signInWaiting: false), false)
        XCTAssertNil(JoinScreenLayout.backWhileCommitting(committing: .start, signInWaiting: false))
        XCTAssertNil(JoinScreenLayout.backWhileCommitting(committing: nil, signInWaiting: false))

        let join = FirstRunState(step: .join)
        XCTAssertEqual(FirstRunFrameLayout.back(join, isCommitting: false, whileCommitting: nil), true)
        XCTAssertNil(FirstRunFrameLayout.back(join, isCommitting: true, whileCommitting: nil))
        XCTAssertEqual(FirstRunFrameLayout.back(join, isCommitting: true, whileCommitting: true), true)
        XCTAssertEqual(FirstRunFrameLayout.back(join, isCommitting: true, whileCommitting: false), false)
        XCTAssertNil(FirstRunFrameLayout.back(FirstRunState(), isCommitting: false, whileCommitting: true),
                     "Welcome has no Back")
    }

    /// Back during the browser wait ends it as Cancel does, then goes
    /// back a step: nothing is enrolled, no failure is said, and the
    /// commit is over.
    func test_backDuringTheBrowserWaitCancelsThenGoesBack() async {
        let daemon = RecordingFirstRunDaemon()
        daemon.loginHolds = true
        var state = onTools()
        state.step = .join
        state.account = .nearAI
        let runner = FirstRunRunner(state: state, daemon: daemon)
        let commit = Task { await runner.commit(.signIn) }
        await yield { runner.signInWaiting }
        XCTAssertEqual(runner.committing, .signIn)

        await runner.backFromSignIn()
        await commit.value

        XCTAssertEqual(daemon.cancels, 1)
        XCTAssertFalse(daemon.log.contains(.enrollNearAI))
        XCTAssertNil(runner.failure)
        XCTAssertFalse(runner.isCommitting)
        XCTAssertNil(runner.committing)
        XCTAssertEqual(runner.state.step, .privateAI)
        XCTAssertFalse(runner.state.holdsEnrolment)
    }

    /// The screens wire it: Tools' secondary action chooses Private AI
    /// only and its Continue takes sharing up again; Join's Back goes
    /// through the runner while signing in; the thread reads the path.
    func test_theScreensWireThePath() throws {
        let tools = try Self.source("Views/FirstRun/ToolsScreen.swift")
        XCTAssertTrue(tools.contains("copy.tools.privateAiOnly"))
        XCTAssertTrue(tools.contains("FirstRunNavigation.choosePrivateAIOnly(runner.state)"))
        XCTAssertTrue(tools.contains("FirstRunNavigation.continueSharing(runner.state)"))
        let join = try Self.source("Views/FirstRun/JoinScreen.swift")
        XCTAssertTrue(join.contains("runner.backFromSignIn()"))
        XCTAssertTrue(join.contains("JoinScreenLayout.startState("))
        let coordinator = try Self.source("Views/OnboardingCoordinatorView.swift")
        XCTAssertTrue(coordinator.contains("FirstRunNavigation.threadSteps(for: runner.state)"))
        XCTAssertTrue(coordinator.contains("copy.threadLabels(for: runner.state)"))
    }

    private static func source(_ path: String) throws -> String {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/\(path)")
        return try String(contentsOf: url, encoding: .utf8)
    }
}
