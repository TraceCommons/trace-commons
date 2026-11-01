import Foundation
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// A recording fake for each first-run call. Every call is logged as the
/// `FirstRunCall` it carries out, so the log compares directly with
/// `FirstRunPlan.calls(for:at:)`.
@MainActor
final class RecordingFirstRunDaemon: FirstRunDaemon {
    var log: [FirstRunCall] = []
    /// The call that answers as a failure; every other call succeeds.
    var failing: ((FirstRunCall) -> Bool) = { _ in false }
    var lookup: FirstRunLookup = .found(RecordingFirstRunDaemon.validLookup)
    var grant: FirstRunGrantAnswer = .granted
    /// The referral code each `enrollNearAI` carried.
    private(set) var referralCodes: [String?] = []

    static let validLookup: DaemonData.InviteLookup = {
        let json = #"{"valid":true,"issuer_display_name":"Issuer"}"#
        return try! JSONDecoder().decode(DaemonData.InviteLookup.self, from: Data(json.utf8))
    }()

    private func record(_ call: FirstRunCall) -> Bool {
        log.append(call)
        return !failing(call)
    }

    func startDaemon(settingsJSON: String) async -> Bool {
        record(.startDaemon(settingsJSON: settingsJSON))
    }

    /// What `first_run_estimate` answers, and how often it was asked.
    var estimate: DaemonData.FirstRunEstimate?
    private(set) var estimateAsks = 0
    func firstRunEstimate() async -> DaemonData.FirstRunEstimate? {
        estimateAsks += 1
        return estimate
    }

    func setSourceSettings(settingsJSON: String) async -> Bool {
        record(.setSourceSettings(settingsJSON: settingsJSON))
    }

    func lookupInvite(_ invite: String) async -> FirstRunLookup {
        log.append(.lookupInvite(invite))
        return lookup
    }

    func enrollInvite(_ invite: String) async -> Bool { record(.enroll(invite)) }
    /// Called as the invite path's sign-in runs, to look at the runner then.
    var duringSignIn: (() -> Void)?
    func signInNearAI() async -> Bool {
        duringSignIn?()
        return record(.signInNearAI)
    }
    var nearAIEnrolment: FirstRunNearAIEnrolment = .enrolled
    /// While true, `nearAILogin` waits like the browser sign-in until its
    /// task is cancelled, then answers `loginAnswerOnCancel`.
    var loginHolds = false
    /// What a held login answers once cancelled: false as `AppModel`'s poll
    /// does, or true for a sign-in that finished as the cancel arrived.
    var loginAnswerOnCancel = false
    /// What `cancelNearAILogin` answers: whether the daemon took the cancel.
    var cancelAnswer = true
    private(set) var cancels = 0
    func nearAILogin() async -> Bool {
        let answer = record(.nearAILogin)
        guard loginHolds else { return answer }
        while !Task.isCancelled { await Task.yield() }
        return loginAnswerOnCancel
    }
    func cancelNearAILogin() async -> Bool {
        cancels += 1
        return cancelAnswer
    }
    func enrollNearAI(referralCode: String?) async -> FirstRunNearAIEnrolment {
        log.append(.enrollNearAI)
        referralCodes.append(referralCode)
        return nearAIEnrolment
    }
    func setAskEachTime() async -> Bool { record(.setAskEachTime) }
    func saveConsentScopes(_ scopes: [String]) async -> Bool { record(.setConsentScopes(scopes)) }

    func setProjectMode(projectID: String, mode: ProjectMode) async -> Bool {
        record(.setProjectMode(projectID: projectID, mode))
    }

    func includePastSessions(projectID: String, sessionIDs: [String]) async -> Bool {
        record(.includePastSessions(projectID: projectID, sessionIDs))
    }

    func setPrivateAI(_ on: Bool) async -> Bool { record(.setPrivateAI(on)) }

    func grantAutomatic(witness: String?) async -> FirstRunGrantAnswer {
        log.append(.grantAutomatic(witness: witness))
        return grant
    }

    func markComplete() async -> Bool { record(.markComplete) }
    func markWatchOnlyComplete() async -> Bool { record(.markWatchOnlyComplete) }

    /// `unenroll` is not a plan call (`FirstRunCall`), so it is counted on
    /// its own and leaves `log` as it was.
    var unenrollCalls = 0
    var unenrollSucceeds = true
    func unenroll() async -> Bool {
        unenrollCalls += 1
        return unenrollSucceeds
    }

    /// What each finished first run handed over, in order.
    var finished: [String?] = []
    func firstRunFinished(notice: String?) { finished.append(notice) }
}

/// The failure each call reports when the daemon refuses it.
private func expectedFailure(for call: FirstRunCall) -> FirstRunFailure? {
    switch call {
    case .startDaemon: return .startFailed
    case .setSourceSettings: return .settingsFailed
    case .lookupInvite: return .inviteDead(label: "invite-invalid")
    case .enroll: return .enrollFailed
    case .signInNearAI, .nearAILogin: return .signInFailed
    case .enrollNearAI: return .nearAIEnrollFailed(label: "near_ai_enroll_unavailable")
    case .setAskEachTime: return .sharingFailed
    case .setConsentScopes: return .scopesFailed
    case .setProjectMode, .includePastSessions: return .rulesFailed
    case .setPrivateAI: return .privateAIFailed
    case .grantAutomatic: return nil
    case .markComplete, .markWatchOnlyComplete: return .completeFailed
    }
}

@MainActor
final class FirstRunRunnerTests: XCTestCase {
    /// On Join with the tools answered, an invite held and near.ai chosen,
    /// the daemon not yet started: what Sign in commits.
    private func onFolders() -> FirstRunState {
        var state = FirstRunState(step: .join)
        state.invite = " INVITE-1 "
        state.issuerHost = "issuer.example"
        state.account = .nearAI
        state.answer(.claudeCode, .watch(path: "/Users/someone/.claude/projects"))
        state.answer(.codex, .off)
        return state
    }

    /// On Join, signed in, every call kind the start commit can make.
    private func onUses() -> FirstRunState {
        var state = FirstRunState(step: .join)
        state.account = .nearAI
        state.answer(.claudeCode, .off)
        state.answer(.codex, .off)
        state.daemonStarted = true
        state.startedSettingsJSON = state.sessionRoots.settingsJSON()
        state.enrolledInvite = "INVITE-1"
        state.signedIn = true
        state.scopes = ["required", "optional"]
        state.rules = ["project-a": .ask, "project-b": .autoUpload]
        state.pastSelections = ["project-a": ["session-2", "session-1"]]
        state.privateAI = true
        state.sharing = .automatic
        state.witnessSigningAddress = "witness-1"
        state.grantReady = true
        return state
    }

    /// Traces asks the started daemon for the commons' dollar estimate
    /// once, and keeps the answer; before the daemon runs it asks nothing.
    func test_theCommonsEstimateIsAskedOnceTheDaemonRuns() async throws {
        let daemon = RecordingFirstRunDaemon()
        daemon.estimate = try JSONDecoder().decode(
            DaemonData.FirstRunEstimate.self,
            from: Data(#"{"usd_micros_per_accepted_trace":{"min":120000,"max":340000}}"#.utf8))
        let runner = FirstRunRunner(state: FirstRunState(step: .traces), daemon: daemon)
        await runner.loadDollarEstimate()
        XCTAssertEqual(daemon.estimateAsks, 0, "no daemon, no ask")
        XCTAssertNil(runner.dollarEstimate)

        runner.state.daemonStarted = true
        await runner.loadDollarEstimate()
        await runner.loadDollarEstimate()
        XCTAssertEqual(daemon.estimateAsks, 1, "asked once")
        XCTAssertEqual(runner.dollarEstimate, daemon.estimate)
    }

    func test_aFailedStartKeepsTheInviteAndJoinsNothing() async {
        let daemon = RecordingFirstRunDaemon()
        daemon.failing = { if case .startDaemon = $0 { return true }; return false }
        let state = onFolders()
        let runner = FirstRunRunner(state: state, daemon: daemon)

        await runner.commit(.signIn)

        XCTAssertEqual(daemon.log, [.startDaemon(settingsJSON: state.sessionRoots.settingsJSON()!)],
            "no lookup or enroll runs without a started daemon")
        XCTAssertEqual(runner.failure, .startFailed)
        XCTAssertEqual(runner.state.invite, " INVITE-1 ")
        XCTAssertEqual(runner.state.issuerHost, "issuer.example")
        XCTAssertFalse(runner.state.daemonStarted)
        XCTAssertNil(runner.state.startedSettingsJSON)
        XCTAssertNil(runner.state.enrolledInvite)
        XCTAssertFalse(runner.state.signedIn)
        XCTAssertEqual(runner.state.step, .join)

        // The retry is the same plan: the invite is still there to join.
        daemon.failing = { _ in false }
        daemon.log = []
        await runner.commit(.signIn)
        XCTAssertEqual(daemon.log, [
            .startDaemon(settingsJSON: state.sessionRoots.settingsJSON()!),
            .lookupInvite("INVITE-1"), .enroll("INVITE-1"), .signInNearAI,
        ])
        XCTAssertNil(runner.failure)
    }

    /// A daemon-reported enrollment recorded over an invite still in Join's
    /// field: the commit asks the daemon about no invite, so a refusal it
    /// would give cannot send the person back to Join, and nothing is
    /// enrolled a second time.
    func test_aLateEnrolmentOverAFilledFieldAsksNothingOfTheInvite() async {
        let daemon = RecordingFirstRunDaemon()
        daemon.lookup = .refused(label: "invite-exhausted")
        var state = onFolders()
        state.account = .none
        state = OnboardingNavigation.recordEnrolment(state)
        let runner = FirstRunRunner(state: state, daemon: daemon)

        await runner.commit(.signIn)

        XCTAssertEqual(daemon.log, [.startDaemon(settingsJSON: state.sessionRoots.settingsJSON()!)])
        XCTAssertNil(runner.failure)
        XCTAssertEqual(runner.state.step, .join, "Sign in stays on Join")
        XCTAssertEqual(runner.state.enrolledInvite, "")
    }

    /// near.ai chosen with no invite to sign in to: the daemon's
    /// `account_sign_in` needs an enrollment and refuses
    /// (`account-enrollment-required`). Join does not allow the pair
    /// (`JoinScreenLayout.canToggleNearAI`); should it reach the runner anyway,
    /// the step stays and says so with the core's sign-in line.
    /// near.ai without an invite: the near.ai sign-in, then the enrollment
    /// through it. The enrollment the daemon holds is what lets Uses offer
    /// Automatic and Start send scopes and the tenant's marker.
    func test_nearAIWithoutAnInviteSignsInAndEnrolls() async throws {
        var state = onFolders()
        state.invite = ""
        state.issuerHost = nil
        state.account = .nearAI
        let daemon = RecordingFirstRunDaemon()
        let runner = FirstRunRunner(state: state, daemon: daemon)

        await runner.commit(.signIn)

        XCTAssertEqual(
            daemon.log, [.startDaemon(settingsJSON: state.sessionRoots.settingsJSON()!), .nearAILogin, .enrollNearAI])
        XCTAssertNil(runner.failure)
        XCTAssertTrue(runner.state.nearAIEnrolled)
        XCTAssertTrue(runner.state.signedIn)
        XCTAssertTrue(runner.state.holdsEnrolment)
        XCTAssertTrue(FirstRunNavigation.canChooseAutomatic(runner.state))
        XCTAssertEqual(runner.state.step, .join, "Sign in stays on Join")

        // Going forward again neither signs in nor enrolls twice.
        daemon.log = []
        runner.state.step = .join
        await runner.commit(.signIn)
        XCTAssertEqual(daemon.log, [])
    }

    func test_nearAIWithoutAnInviteReportsTheSignIn() async throws {
        var state = onFolders()
        state.invite = ""
        state.issuerHost = nil
        state.account = .nearAI
        let daemon = RecordingFirstRunDaemon()
        daemon.failing = { $0 == .nearAILogin }
        let runner = FirstRunRunner(state: state, daemon: daemon)

        await runner.commit(.signIn)

        XCTAssertEqual(daemon.log, [.startDaemon(settingsJSON: state.sessionRoots.settingsJSON()!), .nearAILogin])
        XCTAssertEqual(runner.failure, .signInFailed)
        XCTAssertFalse(runner.state.signedIn)
        // Kristi's review of #1261: no Back, so a sign-in that did not finish
        // returns to Join with the choice cleared, which says why.
        XCTAssertEqual(runner.state.step, .join)
        XCTAssertEqual(runner.state.account, .none)
        XCTAssertEqual(runner.state.toolAnswers, state.toolAnswers)
        XCTAssertTrue(runner.state.daemonStarted)
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        XCTAssertEqual(
            JoinScreenLayout.nearAINotice(runner.state, failure: runner.failure, copy: copy),
            copy.folders.signInFailed)

        // There is no watch only to move on as (owner, 2026-10-12: sign-in
        // required): Continue stays on Join until near.ai is chosen again.
        XCTAssertFalse(JoinScreenLayout.canForward(runner.state))
        let held = runner.state
        runner.state = JoinScreenLayout.readyToStart(runner.state)
        XCTAssertEqual(runner.state, held)
        XCTAssertEqual(runner.state.step, .join)
    }

    /// Choosing near.ai again on Join is what lets the next Continue sign in;
    /// the commit that failed does not leave it chosen.
    func test_nearAIIsTriedAgainOnlyWhenChosenAgain() async throws {
        var state = onFolders()
        state.invite = ""
        state.issuerHost = nil
        state.account = .nearAI
        let daemon = RecordingFirstRunDaemon()
        daemon.failing = { $0 == .nearAILogin }
        let runner = FirstRunRunner(state: state, daemon: daemon)
        await runner.commit(.signIn)
        XCTAssertEqual(runner.state.step, .join)

        runner.state = JoinScreenLayout.chooseNearAI(runner.state)
        runner.state = JoinScreenLayout.readyToStart(runner.state)
        daemon.failing = { _ in false }
        daemon.log = []
        await runner.commit(.signIn)
        XCTAssertEqual(daemon.log, [.nearAILogin, .enrollNearAI])
        XCTAssertTrue(runner.state.nearAIEnrolled)
        XCTAssertEqual(runner.state.step, .join)
    }

    /// The optional referral code on Join rides the NEAR AI enrolment as
    /// typed (the daemon reads it); an empty field sends none.
    func test_theReferralCodeRidesTheNearAIEnrolment() async throws {
        var state = onFolders()
        state.invite = ""
        state.issuerHost = nil
        state.account = .nearAI
        let daemon = RecordingFirstRunDaemon()
        let runner = FirstRunRunner(state: state, daemon: daemon)
        runner.referralCode = "  tc-7k3m9-qx2rd "
        await runner.commit(.signIn)
        XCTAssertEqual(
            daemon.log, [.startDaemon(settingsJSON: state.sessionRoots.settingsJSON()!), .nearAILogin, .enrollNearAI])
        XCTAssertEqual(daemon.referralCodes, ["tc-7k3m9-qx2rd"])

        XCTAssertNil(FirstRunRunner.sentReferralCode(""))
        XCTAssertNil(FirstRunRunner.sentReferralCode(" \n "))
        XCTAssertEqual(FirstRunRunner.sentReferralCode(" TC-7K3M9-QX2RD"), "TC-7K3M9-QX2RD")
    }

    /// A refused enrollment says why in the core's own line for its label
    /// (`TCNearAiEnroll`), never the label, and the step stays.
    func test_aRefusedNearAIEnrolmentReadsTheCoresLine() async throws {
        var state = onFolders()
        state.invite = ""
        state.issuerHost = nil
        state.account = .nearAI
        let daemon = RecordingFirstRunDaemon()
        daemon.nearAIEnrolment = .refused(label: "near_ai_enroll_commons_unreachable")
        let runner = FirstRunRunner(state: state, daemon: daemon)

        await runner.commit(.signIn)

        XCTAssertEqual(runner.failure, .nearAIEnrollFailed(label: "near_ai_enroll_commons_unreachable"))
        XCTAssertFalse(runner.state.nearAIEnrolled)
        XCTAssertFalse(runner.state.holdsEnrolment)
        // Back on Join with the choice cleared (Kristi's review of #1261).
        XCTAssertEqual(runner.state.step, .join)
        XCTAssertEqual(runner.state.account, .none)
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        let line = try XCTUnwrap(JoinScreenLayout.nearAINotice(runner.state, failure: runner.failure, copy: copy))
        XCTAssertEqual(line, TCNearAiEnroll.line(label: "near_ai_enroll_commons_unreachable"))
        XCTAssertFalse(line.contains("near_ai_enroll"))
    }

    /// The label the review names: provisioning refused on the server. The
    /// line is the core's, or its enroll refusal when it has none.
    func test_aRefusedEndpointReturnsToJoin() async throws {
        var state = onFolders()
        state.invite = ""
        state.issuerHost = nil
        state.account = .nearAI
        let daemon = RecordingFirstRunDaemon()
        daemon.nearAIEnrolment = .refused(label: "near_ai_enroll_endpoint_refused")
        let runner = FirstRunRunner(state: state, daemon: daemon)

        await runner.commit(.signIn)

        XCTAssertEqual(runner.state.step, .join)
        XCTAssertEqual(runner.state.account, .none)
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        let line = try XCTUnwrap(JoinScreenLayout.nearAINotice(runner.state, failure: runner.failure, copy: copy))
        XCTAssertEqual(line, TCNearAiEnroll.line(label: "near_ai_enroll_endpoint_refused") ?? copy.folders.enrollRefused)
    }

    /// With an invite, a near.ai sign-in that did not go through returns to
    /// Join too (owner, 2026-10-07): the invite, its enrollment and every
    /// other answer are kept, only the near.ai choice is cleared, and Join
    /// says why in the core's sign-in line.
    func test_aFailedSignInWithAnInviteReturnsToJoin() async throws {
        let daemon = RecordingFirstRunDaemon()
        daemon.failing = { $0 == .signInNearAI }
        let state = onFolders()
        let runner = FirstRunRunner(state: state, daemon: daemon)

        await runner.commit(.signIn)

        let json = state.sessionRoots.settingsJSON()!
        XCTAssertEqual(
            daemon.log, [.startDaemon(settingsJSON: json), .lookupInvite("INVITE-1"), .enroll("INVITE-1"), .signInNearAI])
        XCTAssertEqual(runner.failure, .signInFailed)
        XCTAssertEqual(runner.state.step, .join)
        XCTAssertEqual(runner.state.account, .none)
        XCTAssertFalse(runner.state.signedIn)
        XCTAssertEqual(runner.state.invite, state.invite)
        XCTAssertEqual(runner.state.issuerHost, state.issuerHost)
        XCTAssertEqual(runner.state.enrolledInvite, "INVITE-1")
        XCTAssertEqual(runner.state.toolAnswers, state.toolAnswers)
        XCTAssertTrue(runner.state.daemonStarted)
        XCTAssertEqual(runner.state.startedSettingsJSON, json)
        XCTAssertEqual(runner.lookup, RecordingFirstRunDaemon.validLookup)
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        XCTAssertEqual(
            JoinScreenLayout.nearAINotice(runner.state, failure: runner.failure, copy: copy),
            copy.folders.signInFailed)

        // near.ai chosen again signs in to the enrolled invite's account;
        // nothing is looked up, joined, or started twice.
        runner.state = JoinScreenLayout.chooseNearAI(runner.state)
        runner.state = JoinScreenLayout.readyToStart(runner.state)
        daemon.failing = { _ in false }
        daemon.log = []
        await runner.commit(.signIn)
        XCTAssertEqual(daemon.log, [.signInNearAI])
        XCTAssertNil(runner.failure)
        XCTAssertTrue(runner.state.signedIn)
        XCTAssertEqual(runner.state.step, .join)
    }

    /// The other invite failures are unchanged: a refused enroll stays on
    /// the step with near.ai still chosen.
    func test_aRefusedEnrollWithAnInviteStillKeepsTheStep() async {
        let daemon = RecordingFirstRunDaemon()
        daemon.failing = { $0 == .enroll("INVITE-1") }
        let runner = FirstRunRunner(state: onFolders(), daemon: daemon)

        await runner.commit(.signIn)

        XCTAssertEqual(runner.failure, .enrollFailed)
        XCTAssertEqual(runner.state.step, .join)
        XCTAssertEqual(runner.state.account, .nearAI)
        XCTAssertNil(runner.state.enrolledInvite)
    }

    /// Yields until `condition` holds, counting yields rather than waiting
    /// on the clock.
    private func yield(until condition: () -> Bool, file: StaticString = #filePath, line: UInt = #line) async {
        for _ in 0..<100_000 where !condition() { await Task.yield() }
        XCTAssertTrue(condition(), "never held", file: file, line: line)
    }

    /// A near.ai login waiting on the browser, from Folders, with no invite.
    private func waitingOnTheBrowser(
        _ daemon: RecordingFirstRunDaemon
    ) async -> (runner: FirstRunRunner, commit: Task<Void, Never>, state: FirstRunState) {
        var state = onFolders()
        state.invite = ""
        state.issuerHost = nil
        state.account = .nearAI
        daemon.loginHolds = true
        let runner = FirstRunRunner(state: state, daemon: daemon)
        XCTAssertFalse(runner.signInWaiting, "no Cancel before the sign-in runs")
        let commit = Task { await runner.commit(.signIn) }
        await yield { runner.signInWaiting }
        XCTAssertTrue(runner.isCommitting)
        return (runner, commit, state)
    }

    /// Cancel ends the browser wait (owner, 2026-10-07): the daemon is told,
    /// the person stays where they were with near.ai still chosen, nothing
    /// is enrolled, Continue is back, and the next Continue signs in afresh.
    func test_cancellingTheBrowserWaitKeepsTheStepAndTheChoice() async throws {
        let daemon = RecordingFirstRunDaemon()
        let (runner, commit, state) = await waitingOnTheBrowser(daemon)

        await runner.cancelSignIn()
        await commit.value

        XCTAssertEqual(daemon.cancels, 1)
        XCTAssertEqual(daemon.log, [.startDaemon(settingsJSON: state.sessionRoots.settingsJSON()!), .nearAILogin])
        XCTAssertNil(runner.failure, "a cancel is the person's, not a failure")
        XCTAssertFalse(runner.isCommitting, "Continue is enabled again")
        XCTAssertFalse(runner.signInWaiting)
        XCTAssertEqual(runner.state.step, .join)
        XCTAssertEqual(runner.state.account, .nearAI)
        XCTAssertFalse(runner.state.nearAIEnrolled)
        XCTAssertFalse(runner.state.signedIn)
        XCTAssertFalse(runner.state.holdsEnrolment)

        // A second Cancel with nothing waiting does nothing.
        await runner.cancelSignIn()
        XCTAssertEqual(daemon.cancels, 1)

        // Continue starts a fresh sign-in.
        daemon.loginHolds = false
        daemon.log = []
        await runner.commit(.signIn)
        XCTAssertEqual(daemon.log, [.nearAILogin, .enrollNearAI])
        XCTAssertTrue(runner.state.nearAIEnrolled)
        XCTAssertEqual(runner.state.step, .join)
    }

    /// A sign-in that finishes as the cancel arrives still enrolls nothing:
    /// the person asked to stop.
    func test_aLoginFinishingAsTheCancelArrivesEnrollsNothing() async {
        let daemon = RecordingFirstRunDaemon()
        daemon.loginAnswerOnCancel = true
        let (runner, commit, _) = await waitingOnTheBrowser(daemon)

        await runner.cancelSignIn()
        await commit.value

        XCTAssertFalse(daemon.log.contains(.enrollNearAI))
        XCTAssertNil(runner.failure)
        XCTAssertEqual(runner.state.step, .join)
        XCTAssertEqual(runner.state.account, .nearAI)
        XCTAssertFalse(runner.state.nearAIEnrolled)
    }

    /// Fail closed: a cancel the daemon did not take still ends the wait
    /// here, and says the sign-in did not finish, on the same step.
    func test_aRefusedCancelStillEndsTheWaitAndSaysTheSignInFailed() async throws {
        let daemon = RecordingFirstRunDaemon()
        daemon.cancelAnswer = false
        let (runner, commit, _) = await waitingOnTheBrowser(daemon)

        await runner.cancelSignIn()
        await commit.value

        XCTAssertEqual(daemon.cancels, 1)
        XCTAssertFalse(daemon.log.contains(.enrollNearAI))
        XCTAssertEqual(runner.failure, .signInFailed)
        XCTAssertFalse(runner.isCommitting)
        XCTAssertFalse(runner.signInWaiting)
        XCTAssertEqual(runner.state.step, .join)
        XCTAssertEqual(runner.state.account, .nearAI)
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        XCTAssertEqual(
            FoldersScreenLayout.notice(for: runner.failure, copy: copy, onboarding: nil), copy.folders.signInFailed)
    }

    /// Only the browser wait offers Cancel: the invite path's sign-in is one
    /// call with no browser.
    func test_onlyTheBrowserWaitOffersCancel() async {
        let daemon = RecordingFirstRunDaemon()
        let runner = FirstRunRunner(state: onFolders(), daemon: daemon)
        var waitingDuringSignIn: Bool?
        daemon.duringSignIn = { waitingDuringSignIn = runner.signInWaiting }
        await runner.commit(.signIn)
        XCTAssertTrue(daemon.log.contains(.signInNearAI))
        XCTAssertEqual(waitingDuringSignIn, false)
        XCTAssertFalse(runner.signInWaiting)
        await runner.cancelSignIn()
        XCTAssertEqual(daemon.cancels, 0)
    }

    func test_aDeadInviteReturnsToJoinWithAnswersKept() async {
        let daemon = RecordingFirstRunDaemon()
        daemon.lookup = .refused(label: "invite-exhausted")
        let state = onFolders()
        let runner = FirstRunRunner(state: state, daemon: daemon)

        await runner.commit(.signIn)

        let json = state.sessionRoots.settingsJSON()!
        XCTAssertEqual(daemon.log, [.startDaemon(settingsJSON: json), .lookupInvite("INVITE-1")])
        XCTAssertEqual(runner.failure, .inviteDead(label: "invite-exhausted"))
        XCTAssertEqual(runner.state.step, .join)
        XCTAssertEqual(runner.state.toolAnswers, state.toolAnswers)
        XCTAssertEqual(runner.state.invite, state.invite)
        XCTAssertNil(runner.state.enrolledInvite)
        // The daemon did start, so going forward again does not start it twice.
        XCTAssertTrue(runner.state.daemonStarted)
        XCTAssertEqual(runner.state.startedSettingsJSON, json)

        runner.state.invite = "INVITE-2"
        runner.state = FirstRunNavigation.next(runner.state)
        daemon.lookup = .found(RecordingFirstRunDaemon.validLookup)
        daemon.log = []
        await runner.commit(.signIn)
        XCTAssertEqual(daemon.log, [.lookupInvite("INVITE-2"), .enroll("INVITE-2"), .signInNearAI])
        XCTAssertEqual(runner.state.enrolledInvite, "INVITE-2")
        XCTAssertEqual(runner.lookup, RecordingFirstRunDaemon.validLookup)
    }

    func test_aRefusedGrantFinishesOnAskMe() async {
        let daemon = RecordingFirstRunDaemon()
        daemon.grant = .refused(label: "automatic-grant-witness-changed")
        let runner = FirstRunRunner(state: onUses(), daemon: daemon)

        await runner.commit(.start)

        // The Ask folder's rule and past sessions, planned after the grant,
        // still go: harmless once every folder asks.
        XCTAssertEqual(daemon.log.suffix(5), [
            .grantAutomatic(witness: "witness-1"), .setAskEachTime,
            .setProjectMode(projectID: "project-a", .ask),
            .includePastSessions(projectID: "project-a", ["session-1", "session-2"]),
            .markComplete,
        ], "setup still finishes")
        XCTAssertEqual(runner.state.sharing, .askMe, "never reported as automatic")
        XCTAssertEqual(runner.failure, .grantRefused(label: "automatic-grant-witness-changed"))
    }

    func test_unconfirmedAskCannotFinishSetupOrSaveBroaderScopes() async {
        let daemon = RecordingFirstRunDaemon()
        daemon.failing = { $0 == .setAskEachTime }
        var state = onUses()
        state.sharing = .askMe
        let runner = FirstRunRunner(state: state, daemon: daemon)
        await runner.commit(.start)
        XCTAssertEqual(daemon.log, [.setAskEachTime])
        XCTAssertEqual(runner.failure, .sharingFailed)
        XCTAssertFalse(runner.completed)
    }

    /// Every way Start can end with a failure gives the Uses screen a
    /// notice to show, with and without the Private AI copy.
    func test_everyStartFailureHasANotice() async throws {
        let uses = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON()))).uses
        let privateAI = PrivateInferenceCopy.decode(fromJSON: TCPrivateInference.copyJSON() ?? "")
        XCTAssertNotNil(privateAI)
        let plan = FirstRunPlan.calls(for: onUses(), at: .start)
        for call in plan {
            // `completeFailed` has no core sentence yet; the plan lists it
            // under "Core sentences each case needs".
            if case .markComplete = call { continue }
            let daemon = RecordingFirstRunDaemon()
            if case .grantAutomatic = call {
                daemon.grant = .refused(label: "arming-terms-unavailable")
            } else {
                daemon.failing = { $0 == call }
            }
            let runner = FirstRunRunner(state: onUses(), daemon: daemon)
            await runner.commit(.start)
            XCTAssertNotNil(runner.failure, "\(call)")
            XCTAssertNotNil(UsesScreenLayout.notice(for: runner.failure, uses: uses, privateAI: privateAI), "\(call)")
            XCTAssertNotNil(UsesScreenLayout.notice(for: runner.failure, uses: uses, privateAI: nil), "\(call)")
        }
    }

    func test_callsRunInPlanOrderAndStopAtTheFirstFailure() async {
        let plan = FirstRunPlan.calls(for: onUses(), at: .start)
        let succeeding = RecordingFirstRunDaemon()
        let runner = FirstRunRunner(state: onUses(), daemon: succeeding)
        await runner.commit(.start)
        XCTAssertEqual(succeeding.log, plan)
        XCTAssertNil(runner.failure)
        XCTAssertEqual(runner.state.sharing, .automatic)

        // Every call that can fail stops the run where it failed, with its
        // own failure. The grant is the one exception
        // (`test_aRefusedGrantFinishesOnAskMe`).
        for (index, call) in plan.enumerated() {
            if case .grantAutomatic = call { continue }
            let daemon = RecordingFirstRunDaemon()
            daemon.failing = { $0 == call }
            let failed = FirstRunRunner(state: onUses(), daemon: daemon)
            await failed.commit(.start)
            XCTAssertEqual(daemon.log, Array(plan.prefix(index + 1)), "\(call)")
            XCTAssertEqual(failed.failure, expectedFailure(for: call), "\(call)")
            if call != .markComplete {
                XCTAssertFalse(daemon.log.contains(.markComplete), "\(call)")
            }
        }

        var roots = onFolders()
        roots.daemonStarted = true
        roots.startedSettingsJSON = #"{"claude_source":{"mode":"off"}}"#
        let rootsPlan = FirstRunPlan.calls(for: roots, at: .signIn)
        for (index, call) in rootsPlan.enumerated() {
            let daemon = RecordingFirstRunDaemon()
            if case .lookupInvite = call {
                daemon.lookup = .refused(label: "invite-invalid")
            } else {
                daemon.failing = { $0 == call }
            }
            let failed = FirstRunRunner(state: roots, daemon: daemon)
            await failed.commit(.signIn)
            XCTAssertEqual(daemon.log, Array(rootsPlan.prefix(index + 1)), "\(call)")
            XCTAssertEqual(failed.failure, expectedFailure(for: call), "\(call)")
            XCTAssertNotEqual(failed.state.step, .uses, "\(call) moved on after failing")
        }
    }

    func test_anUnavailableLookupKeepsTheStep() async {
        let daemon = RecordingFirstRunDaemon()
        daemon.lookup = .unavailable
        let state = onFolders()
        let runner = FirstRunRunner(state: state, daemon: daemon)

        await runner.commit(.signIn)

        let json = state.sessionRoots.settingsJSON()!
        XCTAssertEqual(daemon.log, [.startDaemon(settingsJSON: json), .lookupInvite("INVITE-1")])
        XCTAssertEqual(runner.failure, .lookupUnavailable)
        XCTAssertEqual(runner.state.step, .join, "a daemon hiccup is not a dead invite")
        XCTAssertNil(runner.lookup)
        XCTAssertNil(runner.state.enrolledInvite)

        // The retry looks the same invite up again; nothing starts twice.
        daemon.lookup = .found(RecordingFirstRunDaemon.validLookup)
        daemon.log = []
        await runner.commit(.signIn)
        XCTAssertEqual(daemon.log, [.lookupInvite("INVITE-1"), .enroll("INVITE-1"), .signInNearAI])
        XCTAssertNil(runner.failure)
        XCTAssertEqual(runner.state.step, .join)
    }

    /// Review Focus 5: Start finishes watching. No consent scopes are sent
    /// (the daemon refuses them without an enrollment) and the watch-only
    /// marker, not the tenant's, ends the run.
    func test_aWatchOnlyStartFinishes() async {
        let daemon = RecordingFirstRunDaemon()
        var state = FirstRunState(step: .join, account: .watchOnly)
        state.answer(.claudeCode, .off)
        state.answer(.codex, .off)
        state.daemonStarted = true
        state.startedSettingsJSON = state.sessionRoots.settingsJSON()
        state.scopes = ["required"]
        let runner = FirstRunRunner(state: state, daemon: daemon)

        await runner.commit(.start)

        XCTAssertEqual(daemon.log, [.markWatchOnlyComplete])
        XCTAssertNil(runner.failure)
        XCTAssertTrue(runner.completed)
    }

    func test_anUnfinishedCompleteIsReportedAndRetriedWithoutTheGrant() async {
        let daemon = RecordingFirstRunDaemon()
        daemon.grant = .refused(label: "automatic-grant-witness-changed")
        daemon.failing = { $0 == .markComplete }
        let runner = FirstRunRunner(state: onUses(), daemon: daemon)

        await runner.commit(.start)

        XCTAssertEqual(daemon.log.last, .markComplete)
        XCTAssertEqual(runner.failure, .completeFailed, "Start did not finish, whatever the grant said")
        XCTAssertEqual(runner.state.sharing, .askMe)

        daemon.failing = { _ in false }
        daemon.log = []
        await runner.commit(.start)
        XCTAssertFalse(daemon.log.contains { if case .grantAutomatic = $0 { return true }; return false },
            "a retry after a refused grant does not ask for Automatic again")
        XCTAssertEqual(daemon.log.last, .markComplete)
        XCTAssertNil(runner.failure)
    }

    func test_leavingWithoutADeclarationFailsClosed() async {
        let daemon = RecordingFirstRunDaemon()
        let runner = FirstRunRunner(state: FirstRunState(step: .tools), daemon: daemon)

        await runner.commit(.signIn)

        XCTAssertEqual(daemon.log, [])
        XCTAssertEqual(runner.failure, .startFailed, "an undeclared start is never silent")
        XCTAssertEqual(runner.state.step, .tools)
        await runner.commit(.leaveTools)
        XCTAssertEqual(runner.failure, .startFailed)
        XCTAssertEqual(runner.state.step, .tools, "Tools does not move on without a declaration")
    }

    /// Continue on Tools starts the daemon with the ticked tools and moves
    /// on to Uses, joining nothing: sign-in is Join's, the last step.
    func test_leavingToolsStartsTheDaemonAndMovesOn() async throws {
        let daemon = RecordingFirstRunDaemon()
        var state = onFolders()
        state.step = .tools
        let runner = FirstRunRunner(state: state, daemon: daemon)
        await runner.commit(.leaveTools)
        let json = try XCTUnwrap(state.sessionRoots.settingsJSON())
        XCTAssertEqual(daemon.log, [.startDaemon(settingsJSON: json)])
        XCTAssertTrue(runner.state.daemonStarted)
        XCTAssertEqual(runner.state.startedSettingsJSON, json)
        XCTAssertNil(runner.state.enrolledInvite)
        XCTAssertEqual(runner.state.step, .uses)

        // Customize sends the same, and stays on Tools.
        let customizing = FirstRunRunner(state: state, daemon: RecordingFirstRunDaemon())
        await customizing.commit(.declareRoots)
        XCTAssertTrue(customizing.state.daemonStarted)
        XCTAssertEqual(customizing.state.step, .tools)
    }

    func test_aSuccessfulSignInRecordsTheDaemonFacts() async {
        let daemon = RecordingFirstRunDaemon()
        let runner = FirstRunRunner(state: onFolders(), daemon: daemon)
        await runner.commit(.signIn)
        let json = onFolders().sessionRoots.settingsJSON()
        XCTAssertTrue(runner.state.daemonStarted)
        XCTAssertEqual(runner.state.startedSettingsJSON, json)
        XCTAssertEqual(runner.state.enrolledInvite, "INVITE-1")
        XCTAssertTrue(runner.state.signedIn)
        XCTAssertEqual(runner.state.step, .join)
        XCTAssertNil(runner.failure)

        // Back to Tools, a folder changed: only the change is sent, and the
        // whole declaration is what the daemon now holds.
        runner.state = FirstRunNavigation.back(to: .tools, from: runner.state)
        runner.state.answer(.codex, .watch(path: "/Users/someone/.codex/sessions"))
        daemon.log = []
        await runner.commit(.leaveTools)
        guard case .setSourceSettings = daemon.log.first else {
            return XCTFail("expected setSourceSettings, got \(daemon.log)")
        }
        XCTAssertEqual(daemon.log.count, 1, "nothing is enrolled or signed in twice")
        XCTAssertEqual(runner.state.startedSettingsJSON, runner.state.sessionRoots.settingsJSON())
        XCTAssertEqual(runner.state.step, .uses)
    }

    /// Kristi's #1235 M3: the daemon is running, so a refused change of
    /// folders on a second Continue is not a failed watcher start. It is its
    /// own failure, the step stays, and the daemon is still held to the
    /// declaration it had.
    func test_aRefusedSettingsChangeIsNotAFailedStart() async {
        let daemon = RecordingFirstRunDaemon()
        var state = onFolders()
        state.daemonStarted = true
        state.startedSettingsJSON = state.sessionRoots.settingsJSON()
        state.enrolledInvite = "INVITE-1"
        state.signedIn = true
        let held = state.startedSettingsJSON
        state.answer(.codex, .watch(path: "/Users/someone/.codex/sessions"))
        daemon.failing = { if case .setSourceSettings = $0 { return true }; return false }
        let runner = FirstRunRunner(state: state, daemon: daemon)

        await runner.commit(.signIn)

        XCTAssertEqual(runner.failure, .settingsFailed)
        XCTAssertNotEqual(runner.failure, .startFailed)
        XCTAssertEqual(runner.state.startedSettingsJSON, held)
        XCTAssertEqual(runner.state.step, .join)
    }
}
