import Foundation
import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Back on Join while the sign-in waits.
@MainActor
final class FirstRunBackDuringSignInTests: XCTestCase {
    /// Tools with Claude Code ticked, and Codex not used.
    private func onTools() -> FirstRunState {
        var state = FirstRunState(step: .tools)
        state.toolAnswers[.claudeCode] = .watch(path: "/Users/someone/.claude/projects")
        state.toolAnswers[.codex] = .off
        return state
    }

    private func yield(until condition: () -> Bool, file: StaticString = #filePath, line: UInt = #line) async {
        for _ in 0..<100_000 where !condition() { await Task.yield() }
        XCTAssertTrue(condition(), "never held", file: file, line: line)
    }

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
        XCTAssertEqual(runner.state.step, .terms, "Back from Join is the Terms step")
        XCTAssertFalse(runner.state.holdsEnrolment)
    }
}
