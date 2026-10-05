import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The host-side rules for Ron's first run (#1030): where a host starts it,
/// when a window shows it, and what an invite link does to it. The rules
/// between its steps are `FirstRunNavigation`'s and are tested there.
final class OnboardingNavigationTests: XCTestCase {
    private typealias Step = OnboardingCoordinatorView.Step

    /// The first run starts at the step the host names, in the tier that
    /// step belongs to.
    func test_theFirstRunStartsWhereTheHostSays() {
        let join = OnboardingNavigation.initialState(startAt: .join, daemonRunning: false)
        XCTAssertEqual(join.step, .join)
        XCTAssertEqual(join.tier, .quick)

        let folders = OnboardingNavigation.initialState(startAt: .folders, daemonRunning: false)
        XCTAssertEqual(folders.step, .folders)
        XCTAssertEqual(folders.tier, .quick)

        for step: Step in [.tools, .rules] {
            let custom = OnboardingNavigation.initialState(startAt: step, daemonRunning: false)
            XCTAssertEqual(custom.step, step)
            XCTAssertEqual(custom.tier, .custom, "\(step) is a Custom step")
        }
    }

    /// A daemon already running (a returning person whose first run did
    /// not finish) is not started again: its declaration is re-sent whole,
    /// since what it holds is unknown here.
    func test_aRunningDaemonIsNotStartedAgain() throws {
        var state = OnboardingNavigation.initialState(startAt: .folders, daemonRunning: true)
        XCTAssertTrue(state.daemonStarted)
        XCTAssertNil(state.startedSettingsJSON)
        state.answer(.claudeCode, .off)
        state.answer(.codex, .off)
        let calls = FirstRunPlan.calls(for: state, at: .leaveRoots)
        XCTAssertFalse(calls.contains { if case .startDaemon = $0 { return true } else { return false } })
        XCTAssertTrue(calls.contains { if case .setSourceSettings = $0 { return true } else { return false } })

        let fresh = OnboardingNavigation.initialState(startAt: .join, daemonRunning: false)
        XCTAssertFalse(fresh.daemonStarted)
    }

    /// Review Focus 1 at the host: a start that fails from inside the first
    /// run flips the core's startup to refused, and the first run stays on
    /// screen (where Folders shows the core's watcher line, the invite
    /// kept). A refusal at launch, before the first run was shown, is the
    /// startup notice's.
    func test_aFailedStartKeepsTheFirstRunOnScreen() {
        XCTAssertTrue(OnboardingNavigation.hostsFirstRun(
            startup: .refused("x"), requiresOnboarding: true, entered: true))
        XCTAssertFalse(OnboardingNavigation.hostsFirstRun(
            startup: .refused("x"), requiresOnboarding: true, entered: false))
        XCTAssertTrue(OnboardingNavigation.hostsFirstRun(
            startup: .needsRoots, requiresOnboarding: true, entered: false))
        XCTAssertTrue(OnboardingNavigation.hostsFirstRun(
            startup: .running, requiresOnboarding: true, entered: false))
        XCTAssertFalse(OnboardingNavigation.hostsFirstRun(
            startup: .starting, requiresOnboarding: true, entered: true))
        XCTAssertFalse(OnboardingNavigation.hostsFirstRun(
            startup: .running, requiresOnboarding: false, entered: true))
    }

    /// An invite link fills Join and brings it up, as Connect did; it never
    /// joins by itself.
    func test_anInviteLinkFillsJoin() throws {
        var state = FirstRunState(step: .folders, account: .watchOnly)
        state.answer(.claudeCode, .off)
        let received = try XCTUnwrap(OnboardingNavigation.receive(
            invite: "https://issuer.example/i#CODE", in: state, failure: nil, isCommitting: false,
            host: { _ in "issuer.example" }))
        XCTAssertEqual(received.state.step, .join)
        XCTAssertEqual(received.state.invite, "https://issuer.example/i#CODE")
        XCTAssertEqual(received.state.issuerHost, "issuer.example")
        XCTAssertEqual(received.state.account, .none, "an invite takes back watch only, as Look up does")
        XCTAssertEqual(received.state.toolAnswers[.claudeCode], .off, "every answer is kept")
        XCTAssertNil(received.state.enrolledInvite)
    }

    /// The link waits while a commit runs (the runner moves the step on
    /// from wherever the state is when its calls end), and never replaces
    /// an invite the daemon enrolled.
    func test_anInviteLinkNeverReplacesAnEnrolledInviteOrInterruptsACommit() {
        let enrolled = FirstRunState(step: .uses, invite: "a", enrolledInvite: "a")
        XCTAssertNil(OnboardingNavigation.receive(
            invite: "b", in: enrolled, failure: nil, isCommitting: false, host: { _ in "h" }))
        XCTAssertNil(OnboardingNavigation.receive(
            invite: "b", in: FirstRunState(), failure: nil, isCommitting: true, host: { _ in "h" }))
    }
}
