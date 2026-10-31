import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The host-side rules for the animated first run: where a host starts it,
/// when a window shows it, and what an invite link does to it. The rules
/// between its steps are `FirstRunNavigation`'s and are tested there.
@MainActor
final class OnboardingNavigationTests: XCTestCase {
    private typealias Step = OnboardingCoordinatorView.Step

    /// The first run starts at the step the host names: Welcome in the
    /// first-run window, Tools in the Private AI tab.
    func test_theFirstRunStartsWhereTheHostSays() {
        for step: Step in FirstRunNavigation.steps {
            XCTAssertEqual(
                OnboardingNavigation.initialState(startAt: step, daemonRunning: false, enrolled: false).step, step)
        }
        XCTAssertEqual(OnboardingCoordinatorView(onComplete: {}).startAt, .welcome)
    }

    /// A daemon already running (a returning person whose first run did
    /// not finish) is not started again: its declaration is re-sent whole,
    /// since what it holds is unknown here.
    func test_aRunningDaemonIsNotStartedAgain() throws {
        var state = OnboardingNavigation.initialState(startAt: .tools, daemonRunning: true, enrolled: false)
        XCTAssertTrue(state.daemonStarted)
        XCTAssertNil(state.startedSettingsJSON)
        state.answer(.claudeCode, .off)
        state.answer(.codex, .off)
        let calls = FirstRunPlan.calls(for: state, at: .leaveTools)
        XCTAssertFalse(calls.contains { if case .startDaemon = $0 { return true } else { return false } })
        XCTAssertTrue(calls.contains { if case .setSourceSettings = $0 { return true } else { return false } })

        let fresh = OnboardingNavigation.initialState(startAt: .join, daemonRunning: false, enrolled: false)
        XCTAssertFalse(fresh.daemonStarted)
    }

    /// A person who enrolled in an earlier first run and quit before Start
    /// resumes as enrolled: the invite the daemon holds is neither looked
    /// up nor joined again (its last use may be spent), and Automatic stays
    /// on offer. The app needs a NEAR AI account (owner, 2026-10-12:
    /// sign-in required) and this enrollment's account is not known here,
    /// so Start sharing waits for near.ai to sign in beside it, which signs
    /// in to the enrollment's account and joins nothing.
    func test_anEnrolledResumeJoinsNothingAgain() throws {
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        var state = OnboardingNavigation.initialState(startAt: .join, daemonRunning: true, enrolled: true)
        XCTAssertEqual(state.account, .enrolled)
        XCTAssertFalse(JoinScreenLayout.inviteIsEditable(state), "no second invite over the enrolment")
        XCTAssertEqual(JoinScreenLayout.footerTitle(state, copy: copy), copy.uses.start)
        XCTAssertNil(JoinScreenLayout.footerNote(state, copy: copy))
        XCTAssertFalse(JoinScreenLayout.canForward(state))
        XCTAssertTrue(FirstRunNavigation.canChooseAutomatic(state))

        state = JoinScreenLayout.chooseNearAI(state)
        XCTAssertEqual(state.account, .nearAI)
        XCTAssertFalse(JoinScreenLayout.canForward(state), "not until the sign-in finished")
        XCTAssertTrue(FirstRunNavigation.canChooseAutomatic(state))
        state.answer(.claudeCode, .off)
        state.answer(.codex, .off)
        let calls = FirstRunPlan.calls(for: state, at: .signIn)
        XCTAssertFalse(calls.contains { if case .lookupInvite = $0 { return true } else { return false } })
        XCTAssertFalse(calls.contains { if case .enroll = $0 { return true } else { return false } })
        XCTAssertTrue(calls.contains(.signInNearAI))
        XCTAssertFalse(calls.contains(.enrollNearAI))

        state.signedIn = true
        XCTAssertTrue(JoinScreenLayout.canForward(state))
        XCTAssertEqual(JoinScreenLayout.readyToStart(state).account, .nearAI, "Start keeps the account")
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .start).last, .markComplete,
            "an enrolment finishes on its tenant's marker")

        // An invite link cannot replace the enrollment either.
        XCTAssertEqual(OnboardingNavigation.receive(
            invite: "https://issuer.example/i#CODE", in: state, failure: nil, isCommitting: false,
            host: { _ in "issuer.example" }), .discard)
    }

    /// The daemon's first status can arrive after the first run is up (the
    /// main window hosts it as soon as the daemon runs, before `status`
    /// says it is enrolled). The enrollment is applied then, over no account
    /// answer or watch only; an account Join chose is kept, and an
    /// enrollment this first run made is left as it is.
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

    /// The realistic late enrollment: a parked link filled Join before the
    /// daemon's first status said it was enrolled. Recording that enrollment
    /// drops the held invite, so it is neither looked up nor joined again
    /// (Review Focus 4), and Join's line names no host the daemon was not
    /// shown to have joined (Review Focus 1), refusal or not.
    func test_aLateEnrolmentOverAFilledJoinJoinsNothingAgain() throws {
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        let fresh = OnboardingNavigation.initialState(startAt: .join, daemonRunning: true, enrolled: false)
        guard case .apply(var filled, _) = OnboardingNavigation.receive(
            invite: "https://issuer.example/i#CODE", in: fresh, failure: nil, isCommitting: false,
            host: { _ in "issuer.example" })
        else { return XCTFail("the parked link is applied first") }
        filled.answer(.claudeCode, .off)
        filled.answer(.codex, .off)
        filled.step = .join

        let recorded = OnboardingNavigation.recordEnrolment(filled)
        XCTAssertEqual(recorded.account, .enrolled)
        XCTAssertFalse(JoinScreenLayout.inviteIsEditable(recorded))
        let calls = FirstRunPlan.calls(for: recorded, at: .signIn)
        XCTAssertFalse(calls.contains { if case .lookupInvite = $0 { return true } else { return false } })
        XCTAssertFalse(calls.contains { if case .enroll = $0 { return true } else { return false } })

        for failure: FirstRunFailure? in [nil, .inviteDead(label: "invite-exhausted")] {
            let line = JoinScreenLayout.inviteLine(recorded, lookup: nil, failure: failure, copy: copy.join)
            if case .joined(let text) = line {
                XCTAssertFalse(text.contains("issuer.example"), "the held invite was never joined")
            }
        }
    }

    /// Review Focus 1 at the host: a start that fails from inside the first
    /// run flips the core's startup to refused, and the first run stays on
    /// screen (where Tools shows the core's watcher line, every answer
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

    /// An invite link fills Join's invite and leaves the step where it is:
    /// Join is the last step, and the daemon Join's sign-in needs starts on
    /// Tools. It never joins by itself.
    func test_anInviteLinkFillsJoinWithoutSkippingAhead() throws {
        var state = FirstRunState(step: .tools, account: .watchOnly)
        state.answer(.claudeCode, .off)
        guard case .apply(let received, let failure) = OnboardingNavigation.receive(
            invite: "https://issuer.example/i#CODE", in: state, failure: nil, isCommitting: false,
            host: { _ in "issuer.example" })
        else { return XCTFail("the link is applied") }
        XCTAssertNil(failure)
        XCTAssertEqual(received.step, .tools)
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
    /// Tools step) leaves the link parked for the main window's.
    func test_aHostThatDoesNotTakeInvitesLeavesThemParked() {
        XCTAssertEqual(OnboardingNavigation.receive(
            invite: "https://issuer.example/i#CODE", in: FirstRunState(step: .tools), failure: nil,
            isCommitting: false, hostTakesInvites: false, host: { _ in "issuer.example" }), .leaveParked)
    }

    /// Owner, 2026-10-12: sign-in required. Join no longer answers watch
    /// only, and a watch-only Start (an answer an earlier build recorded),
    /// run by the runner against the live model, writes its marker but no
    /// longer ends the first run: with no enrollment a window still hosts
    /// it, so the person signs in.
    func test_aWatchOnlyStartNoLongerLeavesTheFirstRun() async {
        let model = AppModel()
        model.setStartupForTesting(.running)
        model.setConfigDirectoryForTesting("/tmp/first-run-host-\(UUID().uuidString)")
        defer { model.clearWatchOnlyMarkerForTesting() }
        XCTAssertTrue(OnboardingNavigation.hostsFirstRun(
            startup: model.startup, requiresOnboarding: model.requiresOnboarding, entered: true))

        var state = OnboardingNavigation.initialState(startAt: .join, daemonRunning: true, enrolled: false)
        XCTAssertEqual(JoinScreenLayout.readyToStart(state), state, "Join has no Skip")
        XCTAssertFalse(JoinScreenLayout.canForward(state))
        state.account = .watchOnly
        state.answer(.claudeCode, .off)
        state.answer(.codex, .off)
        state.startedSettingsJSON = state.sessionRoots.settingsJSON()
        state.scopes = ["required"]
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .start), [.markWatchOnlyComplete])
        let runner = FirstRunRunner(state: state, daemon: model)

        await runner.commit(.start)

        XCTAssertNil(runner.failure)
        XCTAssertTrue(runner.completed)
        XCTAssertTrue(model.isWatchOnlyComplete)
        XCTAssertTrue(model.requiresOnboarding)
        XCTAssertTrue(OnboardingNavigation.hostsFirstRun(
            startup: model.startup, requiresOnboarding: model.requiresOnboarding, entered: true))
    }
}
