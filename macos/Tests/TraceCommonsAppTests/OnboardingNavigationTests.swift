import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The host-side rules for Ron's first run (#1030): where a host starts it,
/// when a window shows it, and what an invite link does to it. The rules
/// between its steps are `FirstRunNavigation`'s and are tested there.
@MainActor
final class OnboardingNavigationTests: XCTestCase {
    private typealias Step = OnboardingCoordinatorView.Step

    /// The first run starts at the step the host names, in the tier that
    /// step belongs to.
    func test_theFirstRunStartsWhereTheHostSays() {
        let join = OnboardingNavigation.initialState(startAt: .join, daemonRunning: false, enrolled: false)
        XCTAssertEqual(join.step, .join)
        XCTAssertEqual(join.tier, .quick)

        let folders = OnboardingNavigation.initialState(startAt: .folders, daemonRunning: false, enrolled: false)
        XCTAssertEqual(folders.step, .folders)
        XCTAssertEqual(folders.tier, .quick)

        for step: Step in [.tools, .rules] {
            let custom = OnboardingNavigation.initialState(startAt: step, daemonRunning: false, enrolled: false)
            XCTAssertEqual(custom.step, step)
            XCTAssertEqual(custom.tier, .custom, "\(step) is a Custom step")
        }
    }

    /// A daemon already running (a returning person whose first run did
    /// not finish) is not started again: its declaration is re-sent whole,
    /// since what it holds is unknown here.
    func test_aRunningDaemonIsNotStartedAgain() throws {
        var state = OnboardingNavigation.initialState(startAt: .folders, daemonRunning: true, enrolled: false)
        XCTAssertTrue(state.daemonStarted)
        XCTAssertNil(state.startedSettingsJSON)
        state.answer(.claudeCode, .off)
        state.answer(.codex, .off)
        let calls = FirstRunPlan.calls(for: state, at: .leaveRoots)
        XCTAssertFalse(calls.contains { if case .startDaemon = $0 { return true } else { return false } })
        XCTAssertTrue(calls.contains { if case .setSourceSettings = $0 { return true } else { return false } })

        let fresh = OnboardingNavigation.initialState(startAt: .join, daemonRunning: false, enrolled: false)
        XCTAssertFalse(fresh.daemonStarted)
    }

    /// A person who enrolled in an earlier first run and quit before Start
    /// resumes as enrolled: the invite the daemon holds is neither looked
    /// up nor joined again (its last use may be spent), Join offers
    /// Continue rather than watch only, and Automatic stays on offer.
    func test_anEnrolledResumeJoinsNothingAgain() throws {
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        var state = OnboardingNavigation.initialState(startAt: .join, daemonRunning: true, enrolled: true)
        XCTAssertEqual(state.account, .enrolled)
        XCTAssertFalse(JoinLayout.inviteIsEditable(state), "no second invite over the enrolment")
        XCTAssertEqual(JoinLayout.footerTitle(state, copy: copy), copy.frame.continueButton)
        XCTAssertNotEqual(JoinLayout.footerTitle(state, copy: copy), copy.join.skip)
        XCTAssertNil(JoinLayout.footerNote(state, copy: copy))
        XCTAssertTrue(FirstRunNavigation.canChooseAutomatic(state.account))

        state = JoinLayout.forward(state)
        XCTAssertEqual(state.account, .enrolled, "Continue keeps the account")
        XCTAssertEqual(state.step, .folders)
        state.answer(.claudeCode, .off)
        state.answer(.codex, .off)
        let calls = FirstRunPlan.calls(for: state, at: .leaveRoots)
        XCTAssertFalse(calls.contains { if case .lookupInvite = $0 { return true } else { return false } })
        XCTAssertFalse(calls.contains { if case .enroll = $0 { return true } else { return false } })
        XCTAssertFalse(calls.contains(.signInNearAI))
        XCTAssertFalse(calls.contains(.openPasskeySheets))

        state.step = .uses
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .start).last, .markComplete,
            "an enrolment finishes on its tenant's marker")

        // An invite link cannot replace the enrolment either.
        XCTAssertEqual(OnboardingNavigation.receive(
            invite: "https://issuer.example/i#CODE", in: state, failure: nil, isCommitting: false,
            host: { _ in "issuer.example" }), .discard)
    }

    /// The daemon's first status can arrive after the first run is up (the
    /// main window hosts it as soon as the daemon runs, before `status`
    /// says it is enrolled). The enrolment is applied then, over no account
    /// answer or watch only; an account Join chose is kept, and an
    /// enrolment this first run made is left as it is.
    func test_aLateEnrolmentIsRecordedWithoutReplacingAChosenAccount() {
        let fresh = OnboardingNavigation.initialState(startAt: .join, daemonRunning: true, enrolled: false)
        let late = OnboardingNavigation.recordEnrolment(fresh)
        XCTAssertEqual(late.account, .enrolled)
        XCTAssertEqual(late.enrolledInvite, "")

        var skipped = fresh
        skipped.account = .watchOnly
        XCTAssertEqual(OnboardingNavigation.recordEnrolment(skipped).account, .enrolled)

        var chose = fresh
        chose.account = .nearAI
        XCTAssertEqual(OnboardingNavigation.recordEnrolment(chose).account, .nearAI)

        let joined = FirstRunState(step: .uses, invite: "a", account: .nearAI, enrolledInvite: "a", signedIn: true)
        XCTAssertEqual(OnboardingNavigation.recordEnrolment(joined), joined)
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
        guard case .apply(let received, let failure) = OnboardingNavigation.receive(
            invite: "https://issuer.example/i#CODE", in: state, failure: nil, isCommitting: false,
            host: { _ in "issuer.example" })
        else { return XCTFail("the link is applied") }
        XCTAssertNil(failure)
        XCTAssertEqual(received.step, .join)
        XCTAssertEqual(received.invite, "https://issuer.example/i#CODE")
        XCTAssertEqual(received.issuerHost, "issuer.example")
        XCTAssertEqual(received.account, .none, "an invite takes back watch only, as Look up does")
        XCTAssertEqual(received.toolAnswers[.claudeCode], .off, "every answer is kept")
        XCTAssertNil(received.enrolledInvite)
    }

    /// The link waits, parked, while a commit runs (the runner moves the
    /// step on from wherever the state is when its calls end), and never
    /// replaces an invite the daemon enrolled. Something that is not an
    /// invite is taken and dropped.
    func test_anInviteLinkNeverReplacesAnEnrolledInviteOrInterruptsACommit() {
        let enrolled = FirstRunState(step: .uses, invite: "a", enrolledInvite: "a")
        XCTAssertEqual(OnboardingNavigation.receive(
            invite: "b", in: enrolled, failure: nil, isCommitting: false, host: { _ in "h" }), .discard)
        XCTAssertEqual(OnboardingNavigation.receive(
            invite: "b", in: FirstRunState(), failure: nil, isCommitting: true, host: { _ in "h" }), .leaveParked)
        XCTAssertEqual(OnboardingNavigation.receive(
            invite: "b", in: FirstRunState(), failure: nil, isCommitting: false, host: { _ in nil }), .discard)
    }

    /// A host whose runner does not last the whole first run (Private AI's
    /// Folders step) leaves the link parked for the main window's.
    func test_aHostThatDoesNotTakeInvitesLeavesThemParked() {
        XCTAssertEqual(OnboardingNavigation.receive(
            invite: "https://issuer.example/i#CODE", in: FirstRunState(step: .folders), failure: nil,
            isCommitting: false, hostTakesInvites: false, host: { _ in "issuer.example" }), .leaveParked)
    }

    /// Review Focus 5 at the host: Start finishes watching. A watch-only
    /// Start, run by the runner against the live model, ends the first run:
    /// no window hosts it afterwards.
    func test_aWatchOnlyStartLeavesTheFirstRun() async {
        let model = AppModel()
        model.setStartupForTesting(.running)
        model.setConfigDirectoryForTesting("/tmp/first-run-host-\(UUID().uuidString)")
        defer { model.clearWatchOnlyMarkerForTesting() }
        XCTAssertTrue(OnboardingNavigation.hostsFirstRun(
            startup: model.startup, requiresOnboarding: model.requiresOnboarding, entered: true))

        var state = OnboardingNavigation.initialState(startAt: .join, daemonRunning: true, enrolled: false)
        state = JoinLayout.forward(state)
        XCTAssertEqual(state.account, .watchOnly)
        state.answer(.claudeCode, .off)
        state.answer(.codex, .off)
        state.startedSettingsJSON = state.sessionRoots.settingsJSON()
        state.step = .uses
        state.scopes = ["required"]
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .start), [.markWatchOnlyComplete])
        let runner = FirstRunRunner(state: state, daemon: model)

        await runner.commit(.start)

        XCTAssertNil(runner.failure)
        XCTAssertTrue(runner.completed)
        XCTAssertFalse(model.requiresOnboarding)
        XCTAssertFalse(OnboardingNavigation.hostsFirstRun(
            startup: model.startup, requiresOnboarding: model.requiresOnboarding, entered: true))
    }
}
