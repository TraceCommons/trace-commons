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
    func enrollNearAI() async -> FirstRunNearAIEnrolment {
        log.append(.enrollNearAI)
        return nearAIEnrolment
    }
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
    case .setConsentScopes: return .scopesFailed
    case .setProjectMode, .includePastSessions: return .rulesFailed
    case .setPrivateAI: return .privateAIFailed
    case .grantAutomatic: return nil
    // Not a daemon call: the sheets are the person's ceremony.
    case .openPasskeySheets: return nil
    case .markComplete, .markWatchOnlyComplete: return .completeFailed
    }
}

@MainActor
final class FirstRunRunnerTests: XCTestCase {
    /// Quick, on Folders, every tool answered, an invite pasted on Join.
    private func onFolders() -> FirstRunState {
        var state = FirstRunState(tier: .quick, step: .folders)
        state.invite = " INVITE-1 "
        state.issuerHost = "issuer.example"
        state.account = .nearAI
        state.answer(.claudeCode, .watch(path: "/Users/someone/.claude/projects"))
        state.answer(.codex, .off)
        return state
    }

    /// Custom, on Uses, every call kind the start commit can make.
    private func onUses() -> FirstRunState {
        var state = FirstRunState(tier: .custom, step: .uses)
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

    /// Create passkey on Join starts the daemon watching nothing and asks for
    /// the sheets there: the step stays on Join, and what the daemon holds is
    /// recorded so Folders sends only what differs.
    func test_createPasskeyOnJoinStartsTheDaemonAndOpensTheSheetsOnJoin() async throws {
        let daemon = RecordingFirstRunDaemon()
        let runner = FirstRunRunner(state: FirstRunState(), daemon: daemon)
        await runner.openPasskeyOnJoin()
        let json = try XCTUnwrap(FirstRunPlan.watchNothingSettingsJSON)
        XCTAssertEqual(daemon.log, [.startDaemon(settingsJSON: json)])
        XCTAssertEqual(runner.state.step, .join)
        XCTAssertTrue(runner.state.daemonStarted)
        XCTAssertEqual(runner.state.startedSettingsJSON, json)
        XCTAssertTrue(runner.passkeyDue)
        XCTAssertEqual(runner.passkeyStart, .choose)
        XCTAssertNil(runner.failure)
        XCTAssertEqual(runner.state.account, .none, "nothing is chosen for later")
    }

    /// A start that fails opens nothing and says so on Join in the passkey's
    /// words, never the watcher's: no folder has been answered. Pressed
    /// again, it retries and takes its own failure back.
    func test_aFailedPasskeyStartOnJoinOpensNothing() async {
        let daemon = RecordingFirstRunDaemon()
        daemon.failing = { if case .startDaemon = $0 { return true }; return false }
        let runner = FirstRunRunner(state: FirstRunState(), daemon: daemon)
        await runner.openPasskeyOnJoin()
        XCTAssertEqual(runner.failure, .passkeyUnavailable)
        XCTAssertFalse(runner.passkeyDue)
        XCTAssertFalse(runner.state.daemonStarted)
        XCTAssertEqual(runner.state.step, .join)

        daemon.failing = { _ in false }
        await runner.openPasskeyOnJoin()
        XCTAssertNil(runner.failure)
        XCTAssertTrue(runner.passkeyDue)
    }

    /// Create passkey takes back only its own failure: a refused invite's
    /// line stays on Join.
    func test_createPasskeyOnJoinKeepsARefusedInvitesLine() async {
        let daemon = RecordingFirstRunDaemon()
        let runner = FirstRunRunner(state: FirstRunState(), daemon: daemon)
        runner.failure = .inviteDead(label: "invite-invalid")
        await runner.openPasskeyOnJoin()
        XCTAssertEqual(runner.failure, .inviteDead(label: "invite-invalid"))
        XCTAssertTrue(runner.passkeyDue)
    }

    func test_aFailedStartKeepsTheInviteAndJoinsNothing() async {
        let daemon = RecordingFirstRunDaemon()
        daemon.failing = { if case .startDaemon = $0 { return true }; return false }
        let state = onFolders()
        let runner = FirstRunRunner(state: state, daemon: daemon)

        await runner.commit(.leaveRoots)

        XCTAssertEqual(daemon.log, [.startDaemon(settingsJSON: state.sessionRoots.settingsJSON()!)],
            "no lookup or enroll runs without a started daemon")
        XCTAssertEqual(runner.failure, .startFailed)
        XCTAssertEqual(runner.state.invite, " INVITE-1 ")
        XCTAssertEqual(runner.state.issuerHost, "issuer.example")
        XCTAssertFalse(runner.state.daemonStarted)
        XCTAssertNil(runner.state.startedSettingsJSON)
        XCTAssertNil(runner.state.enrolledInvite)
        XCTAssertFalse(runner.state.signedIn)
        XCTAssertEqual(runner.state.step, .folders)

        // The retry is the same plan: the invite is still there to join.
        daemon.failing = { _ in false }
        daemon.log = []
        await runner.commit(.leaveRoots)
        XCTAssertEqual(daemon.log, [
            .startDaemon(settingsJSON: state.sessionRoots.settingsJSON()!),
            .lookupInvite("INVITE-1"), .enroll("INVITE-1"), .signInNearAI,
        ])
        XCTAssertNil(runner.failure)
    }

    /// The passkey sheets are a person's ceremony, not a daemon call: the
    /// runner raises them once the daemon started and moves on, and a
    /// failed start raises nothing.
    func test_aChosenPasskeyOpensItsSheetsOnceTheDaemonStarts() async {
        var state = onFolders()
        // A new passkey is not combined with an invite.
        state.invite = ""
        state.issuerHost = nil
        state.account = .passkeyChosen
        let json = state.sessionRoots.settingsJSON()!

        let refusing = RecordingFirstRunDaemon()
        refusing.failing = { if case .startDaemon = $0 { return true }; return false }
        let failed = FirstRunRunner(state: state, daemon: refusing)
        XCTAssertFalse(failed.passkeyDue)
        await failed.commit(.leaveRoots)
        XCTAssertFalse(failed.passkeyDue, "no sheet before the daemon runs")
        XCTAssertEqual(failed.state.account, .passkeyChosen)
        XCTAssertEqual(failed.state.step, .folders)

        let daemon = RecordingFirstRunDaemon()
        let runner = FirstRunRunner(state: state, daemon: daemon)
        await runner.commit(.leaveRoots)
        XCTAssertEqual(daemon.log, [.startDaemon(settingsJSON: json)],
            "the sheets are not a daemon call, and nothing signs in to near.ai")
        XCTAssertTrue(runner.passkeyDue)
        XCTAssertTrue(runner.state.daemonStarted)
        XCTAssertEqual(runner.state.step, .uses)
        XCTAssertNil(runner.failure)
    }

    /// Ron's P-7 (review of #1235 item 4): opening the first run on Join with
    /// the daemon running, nobody signed in, and a passkey this Mac
    /// remembers, the sheets open at Welcome back with the passkey's name.
    /// "Other sign-in options" closes it, Join is as it was, and it is not
    /// offered again in this first run; Create passkey on Join opens P-1.
    func test_aReturningPersonIsOfferedWelcomeBackOnceAndCanDismissIt() async throws {
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        let join = FirstRunState(tier: .quick, step: .join, daemonStarted: true)
        let daemon = RecordingFirstRunDaemon()
        let runner = FirstRunRunner(state: join, daemon: daemon)
        let account = PasskeySheetsTests.RecordingAccount()
        account.passkeys = NativePasskeyState(
            state: "none", passkeyCount: 1, rememberedName: "Home", nearAiConnected: nil)

        await runner.offerWelcomeBack(from: account)
        XCTAssertTrue(runner.passkeyDue)
        XCTAssertEqual(runner.passkeyStart, .welcomeBack)
        XCTAssertEqual(runner.returningName, "Home")
        XCTAssertEqual(account.calls, ["passkeyState"])
        XCTAssertEqual(daemon.log, [], "asking is not a first-run daemon call")

        runner.finishPasskey(.closed, copy: copy)
        XCTAssertFalse(runner.passkeyDue)
        XCTAssertEqual(runner.state, join, "dismissing P-7 leaves Join as it was")
        XCTAssertNil(runner.passkeyOutcome?.joinNotice(copy))

        await runner.offerWelcomeBack(from: account)
        XCTAssertFalse(runner.passkeyDue, "dismissed once, not offered again")
        XCTAssertEqual(account.calls, ["passkeyState"])

        runner.requestPasskey()
        XCTAssertTrue(runner.passkeyDue)
        XCTAssertEqual(runner.passkeyStart, .choose, "Create passkey on Join opens P-1")
    }

    /// No remembered passkey, or a person already signed in: Join opens as
    /// it always did.
    func test_welcomeBackIsNotOfferedToAnyoneElse() async {
        for passkeys in [
            NativePasskeyState(state: "none", passkeyCount: 0, rememberedName: nil, nearAiConnected: nil),
            NativePasskeyState(state: "unbound", passkeyCount: 1, rememberedName: "Home", nearAiConnected: false),
            NativePasskeyState(state: "unknown", passkeyCount: nil, rememberedName: nil, nearAiConnected: nil),
        ] {
            let runner = FirstRunRunner(
                state: FirstRunState(tier: .quick, step: .join, daemonStarted: true), daemon: RecordingFirstRunDaemon())
            let account = PasskeySheetsTests.RecordingAccount()
            account.passkeys = passkeys
            await runner.offerWelcomeBack(from: account)
            XCTAssertFalse(runner.passkeyDue, passkeys.state)
            XCTAssertEqual(runner.passkeyStart, .choose)
        }
    }

    /// The daemon's answer can arrive after Join changed (an enrollment the
    /// first status reported, or the person answering): the rule is checked
    /// again on the state as it is then.
    func test_welcomeBackIsRecheckedAfterTheDaemonAnswers() async {
        let runner = FirstRunRunner(
            state: FirstRunState(tier: .quick, step: .join, daemonStarted: true), daemon: RecordingFirstRunDaemon())
        let account = PasskeySheetsTests.RecordingAccount()
        account.passkeys = NativePasskeyState(
            state: "none", passkeyCount: 1, rememberedName: "Home", nearAiConnected: nil)
        account.onPasskeyState = { runner.state = OnboardingNavigation.recordEnrolment(runner.state) }
        await runner.offerWelcomeBack(from: account)
        XCTAssertEqual(runner.state.account, .enrolled)
        XCTAssertFalse(runner.passkeyDue)
    }

    /// A passkey first used on this Mac by signing in is now remembered by
    /// the label the server returned for it at sign-in, so the next first
    /// run's P-7 greets it by that name. The daemon answers as it does after
    /// such a sign-in and a sign-out: the record's name, and no signed-in
    /// account to name.
    func test_welcomeBackGreetsAPasskeyLearnedFromSignInByItsLabel() async throws {
        let reply = #"{"state":"none","passkey_count":1,"remembered_name":"Studio","signed_in_name":null,"near_ai_connected":null}"#
        let passkeys = try JSONDecoder().decode(NativePasskeyState.self, from: Data(reply.utf8))
        let runner = FirstRunRunner(
            state: FirstRunState(tier: .quick, step: .join, daemonStarted: true), daemon: RecordingFirstRunDaemon())
        let account = PasskeySheetsTests.RecordingAccount()
        account.passkeys = passkeys
        await runner.offerWelcomeBack(from: account)
        XCTAssertTrue(runner.passkeyDue)
        XCTAssertEqual(runner.passkeyStart, .welcomeBack)
        XCTAssertEqual(runner.returningName, "Studio")
    }

    /// Late enrollment (#1264 follow-up): P-7 is already open when the
    /// daemon's first status reports an enrollment, so the account becomes
    /// `.enrolled` under it; then P-7's Sign in.
    ///
    /// A passkey for another tenant's account: the daemon refuses the
    /// sign-in (`account-enrollment-mismatch`, nothing kept, nothing sent
    /// beyond the login itself; the daemon tests pin that). P-7 stays up with
    /// the refusal, no bind is asked for, and closing it leaves Join on the
    /// enrollment it held.
    func test_aLateEnrolmentUnderWelcomeBackRefusesAnotherTenantsSignIn() async throws {
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        let runner = FirstRunRunner(
            state: FirstRunState(tier: .quick, step: .join, daemonStarted: true), daemon: RecordingFirstRunDaemon())
        let account = PasskeySheetsTests.RecordingAccount()
        account.passkeys = NativePasskeyState(
            state: "none", passkeyCount: 1, rememberedName: "Home", nearAiConnected: nil)
        await runner.offerWelcomeBack(from: account)
        XCTAssertEqual(runner.passkeyStart, .welcomeBack)

        runner.state = OnboardingNavigation.recordEnrolment(runner.state)
        XCTAssertEqual(runner.state.account, .enrolled)
        let enrolled = runner.state

        account.signInAnswer = .failed(.refused(label: "account-enrollment-mismatch"))
        let sheet = PasskeySheetModel(start: runner.passkeyStart, copy: copy.passkey, account: account)
        await sheet.useExisting()
        XCTAssertEqual(sheet.step, .welcomeBack)
        XCTAssertEqual(sheet.refusal, "account-enrollment-mismatch")
        XCTAssertNil(sheet.outcome)
        XCTAssertEqual(account.calls, ["passkeyState", "signIn"], "no bind, no sign-out")

        sheet.close()
        runner.finishPasskey(try XCTUnwrap(sheet.outcome), copy: copy)
        XCTAssertEqual(runner.state, enrolled, "Join keeps the enrolment")
        XCTAssertTrue(runner.state.holdsEnrolment)
    }

    /// Late enrollment, the enrolled account's own passkey: the daemon keeps
    /// the sign-in (it checked the session's tenant against the enrollment),
    /// and Verify's bind answers `already_enrolled` from local state, with
    /// no request: the enrollment this Mac holds is this account's. Verify
    /// takes it as the join it would otherwise have made, ends `.signedIn`
    /// named by `signed_in_name`, and Join keeps holding the enrollment.
    func test_aLateEnrolmentUnderWelcomeBackFinishesWithTheEnrolledAccountsPasskey() async throws {
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        let runner = FirstRunRunner(
            state: FirstRunState(tier: .quick, step: .join, daemonStarted: true), daemon: RecordingFirstRunDaemon())
        let account = PasskeySheetsTests.RecordingAccount()
        account.passkeys = NativePasskeyState(
            state: "none", passkeyCount: 1, rememberedName: "Home", nearAiConnected: nil)
        await runner.offerWelcomeBack(from: account)
        runner.state = OnboardingNavigation.recordEnrolment(runner.state)

        account.signInAnswer = .bound
        let daemonAnswer = try JSONDecoder().decode(
            NativeAccountBindResult.self,
            from: Data(#"{"outcome":"already_enrolled","binding_state":"bound"}"#.utf8))
        account.bindAnswer = PasskeyBindResult(daemonAnswer)
        let sheet = PasskeySheetModel(start: runner.passkeyStart, copy: copy.passkey, account: account)
        await sheet.useExisting()
        XCTAssertEqual(sheet.step, .verify)
        account.passkeys = NativePasskeyState(
            state: "bound", passkeyCount: 1, rememberedName: "Home", signedInName: "Home", nearAiConnected: true)
        await sheet.verify()
        XCTAssertNil(sheet.refusal)
        XCTAssertEqual(sheet.outcome, .signedIn(name: "Home"))
        XCTAssertEqual(account.calls, ["passkeyState", "signIn", "bind", "passkeyState"], "no sign-out")

        runner.finishPasskey(try XCTUnwrap(sheet.outcome), copy: copy)
        XCTAssertEqual(runner.state.account, .passkey(name: "Home"))
        XCTAssertFalse(runner.state.signedOutOfEnrolment)
        XCTAssertTrue(runner.state.holdsEnrolment)
        XCTAssertEqual(runner.state.step, .join)
    }

    /// The sheets are not awaited, so Start can run while they are open. A
    /// sign-out that ends them after Start leaves the finished first run
    /// where it is rather than reopening Join.
    func test_aSignOutAfterStartLeavesTheFinishedRunAlone() async throws {
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        var state = FirstRunState(
            step: .uses, account: .passkey(name: "Laptop"), toolAnswers: [.claudeCode: .off, .codex: .off])
        state.daemonStarted = true
        state.startedSettingsJSON = state.sessionRoots.settingsJSON()
        state.scopes = ["research"]
        let daemon = RecordingFirstRunDaemon()
        let runner = FirstRunRunner(state: state, daemon: daemon)
        runner.requestPasskey()

        await runner.commit(.start)
        XCTAssertEqual(daemon.log.last, .markComplete)
        XCTAssertTrue(runner.completed)

        runner.finishPasskey(.signedOut, copy: copy)
        XCTAssertFalse(runner.passkeyDue)
        XCTAssertEqual(runner.state.account, AccountAnswer.none)
        XCTAssertEqual(runner.state.step, .uses, "a finished first run is not reopened")
        await runner.pendingUnenroll?.value
        XCTAssertEqual(daemon.unenrollCalls, 0, "a finished first run's enrollment is not dropped")
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

        await runner.commit(.leaveRoots)

        XCTAssertEqual(daemon.log, [.startDaemon(settingsJSON: state.sessionRoots.settingsJSON()!)])
        XCTAssertNil(runner.failure)
        XCTAssertNotEqual(runner.state.step, .join)
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

        await runner.commit(.leaveRoots)

        XCTAssertEqual(
            daemon.log, [.startDaemon(settingsJSON: state.sessionRoots.settingsJSON()!), .nearAILogin, .enrollNearAI])
        XCTAssertNil(runner.failure)
        XCTAssertTrue(runner.state.nearAIEnrolled)
        XCTAssertTrue(runner.state.signedIn)
        XCTAssertTrue(runner.state.holdsEnrolment)
        XCTAssertEqual(FirstRunNavigation.sharingPaths(for: runner.state), [.automatic, .askMe])
        XCTAssertNotEqual(runner.state.step, .folders)

        // Going forward again neither signs in nor enrolls twice.
        daemon.log = []
        runner.state.step = .folders
        await runner.commit(.leaveRoots)
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

        await runner.commit(.leaveRoots)

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

        // Choosing watch only there moves on with no near.ai call.
        runner.state = JoinScreenLayout.forward(runner.state)
        daemon.log = []
        await runner.commit(.leaveRoots)
        XCTAssertEqual(daemon.log, [])
        XCTAssertNil(runner.failure)
        XCTAssertEqual(runner.state.account, .watchOnly)
        XCTAssertEqual(runner.state.step, .uses)
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
        await runner.commit(.leaveRoots)
        XCTAssertEqual(runner.state.step, .join)

        runner.state = JoinScreenLayout.toggleNearAI(runner.state)
        runner.state = JoinScreenLayout.forward(runner.state)
        daemon.failing = { _ in false }
        daemon.log = []
        await runner.commit(.leaveRoots)
        XCTAssertEqual(daemon.log, [.nearAILogin, .enrollNearAI])
        XCTAssertTrue(runner.state.nearAIEnrolled)
        XCTAssertEqual(runner.state.step, .uses)
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

        await runner.commit(.leaveRoots)

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

        await runner.commit(.leaveRoots)

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

        await runner.commit(.leaveRoots)

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
        runner.state = JoinScreenLayout.toggleNearAI(runner.state)
        runner.state = JoinScreenLayout.forward(runner.state)
        daemon.failing = { _ in false }
        daemon.log = []
        await runner.commit(.leaveRoots)
        XCTAssertEqual(daemon.log, [.signInNearAI])
        XCTAssertNil(runner.failure)
        XCTAssertTrue(runner.state.signedIn)
        XCTAssertEqual(runner.state.step, .uses)
    }

    /// The other invite failures are unchanged: a refused enroll stays on
    /// the step with near.ai still chosen.
    func test_aRefusedEnrollWithAnInviteStillKeepsTheStep() async {
        let daemon = RecordingFirstRunDaemon()
        daemon.failing = { $0 == .enroll("INVITE-1") }
        let runner = FirstRunRunner(state: onFolders(), daemon: daemon)

        await runner.commit(.leaveRoots)

        XCTAssertEqual(runner.failure, .enrollFailed)
        XCTAssertEqual(runner.state.step, .folders)
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
        let commit = Task { await runner.commit(.leaveRoots) }
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
        XCTAssertEqual(runner.state.step, .folders)
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
        await runner.commit(.leaveRoots)
        XCTAssertEqual(daemon.log, [.nearAILogin, .enrollNearAI])
        XCTAssertTrue(runner.state.nearAIEnrolled)
        XCTAssertEqual(runner.state.step, .uses)
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
        XCTAssertEqual(runner.state.step, .folders)
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
        XCTAssertEqual(runner.state.step, .folders)
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
        await runner.commit(.leaveRoots)
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

        await runner.commit(.leaveRoots)

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
        await runner.commit(.leaveRoots)
        XCTAssertEqual(daemon.log, [.lookupInvite("INVITE-2"), .enroll("INVITE-2"), .signInNearAI])
        XCTAssertEqual(runner.state.enrolledInvite, "INVITE-2")
        XCTAssertEqual(runner.lookup, RecordingFirstRunDaemon.validLookup)
    }

    func test_aRefusedGrantFinishesOnAskMe() async {
        let daemon = RecordingFirstRunDaemon()
        daemon.grant = .refused(label: "automatic-grant-witness-changed")
        let runner = FirstRunRunner(state: onUses(), daemon: daemon)

        await runner.commit(.start)

        XCTAssertEqual(daemon.log.suffix(2), [.grantAutomatic(witness: "witness-1"), .markComplete],
            "setup still finishes")
        XCTAssertEqual(runner.state.sharing, .askMe, "never reported as automatic")
        XCTAssertEqual(runner.failure, .grantRefused(label: "automatic-grant-witness-changed"))
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
        let rootsPlan = FirstRunPlan.calls(for: roots, at: .leaveRoots)
        for (index, call) in rootsPlan.enumerated() {
            let daemon = RecordingFirstRunDaemon()
            if case .lookupInvite = call {
                daemon.lookup = .refused(label: "invite-invalid")
            } else {
                daemon.failing = { $0 == call }
            }
            let failed = FirstRunRunner(state: roots, daemon: daemon)
            await failed.commit(.leaveRoots)
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

        await runner.commit(.leaveRoots)

        let json = state.sessionRoots.settingsJSON()!
        XCTAssertEqual(daemon.log, [.startDaemon(settingsJSON: json), .lookupInvite("INVITE-1")])
        XCTAssertEqual(runner.failure, .lookupUnavailable)
        XCTAssertEqual(runner.state.step, .folders, "a daemon hiccup is not a dead invite")
        XCTAssertNil(runner.lookup)
        XCTAssertNil(runner.state.enrolledInvite)

        // The retry looks the same invite up again; nothing starts twice.
        daemon.lookup = .found(RecordingFirstRunDaemon.validLookup)
        daemon.log = []
        await runner.commit(.leaveRoots)
        XCTAssertEqual(daemon.log, [.lookupInvite("INVITE-1"), .enroll("INVITE-1"), .signInNearAI])
        XCTAssertNil(runner.failure)
        XCTAssertEqual(runner.state.step, .uses)
    }

    /// Review Focus 5: Start finishes watching. No consent scopes are sent
    /// (the daemon refuses them without an enrollment) and the watch-only
    /// marker, not the tenant's, ends the run.
    func test_aWatchOnlyStartFinishes() async {
        let daemon = RecordingFirstRunDaemon()
        var state = FirstRunState(tier: .quick, step: .uses, account: .watchOnly)
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
        let runner = FirstRunRunner(state: FirstRunState(tier: .quick, step: .folders), daemon: daemon)

        await runner.commit(.leaveRoots)

        XCTAssertEqual(daemon.log, [])
        XCTAssertEqual(runner.failure, .startFailed, "an undeclared start is never silent")
        XCTAssertEqual(runner.state.step, .folders)
    }

    func test_aSuccessfulLeaveRecordsTheDaemonFactsAndMovesOn() async {
        let daemon = RecordingFirstRunDaemon()
        let runner = FirstRunRunner(state: onFolders(), daemon: daemon)
        await runner.commit(.leaveRoots)
        let json = onFolders().sessionRoots.settingsJSON()
        XCTAssertTrue(runner.state.daemonStarted)
        XCTAssertEqual(runner.state.startedSettingsJSON, json)
        XCTAssertEqual(runner.state.enrolledInvite, "INVITE-1")
        XCTAssertTrue(runner.state.signedIn)
        XCTAssertEqual(runner.state.step, .uses)
        XCTAssertNil(runner.failure)

        // Back to Folders, a folder changed: only the change is sent, and the
        // whole declaration is what the daemon now holds.
        runner.state = FirstRunNavigation.back(runner.state)
        runner.state.answer(.codex, .watch(path: "/Users/someone/.codex/sessions"))
        daemon.log = []
        await runner.commit(.leaveRoots)
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

        await runner.commit(.leaveRoots)

        XCTAssertEqual(runner.failure, .settingsFailed)
        XCTAssertNotEqual(runner.failure, .startFailed)
        XCTAssertEqual(runner.state.startedSettingsJSON, held)
        XCTAssertEqual(runner.state.step, .folders)
    }

    /// Kristi's #1235 I1, as decided: Start with a passkey chosen but not
    /// created reopens the sheets, sends the daemon nothing and finishes
    /// nothing. Closed again, Start reopens them again; once a passkey is
    /// bound, Start finishes.
    func test_startWithAChosenPasskeyReopensTheSheets() async throws {
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        let daemon = RecordingFirstRunDaemon()
        var state = FirstRunState(tier: .quick, step: .uses)
        state.account = .passkeyChosen
        state.answer(.claudeCode, .watch(path: "/Users/someone/.claude/projects"))
        state.answer(.codex, .off)
        state.daemonStarted = true
        state.startedSettingsJSON = state.sessionRoots.settingsJSON()
        state.scopes = ["required"]
        let runner = FirstRunRunner(state: state, daemon: daemon)

        await runner.commit(.start)
        XCTAssertTrue(runner.passkeyDue)
        XCTAssertEqual(daemon.log, [], "no scopes, grant or marker without an enrolment")
        XCTAssertFalse(runner.completed)
        XCTAssertNil(runner.failure, "no Start failure line for a passkey still to create")

        runner.finishPasskey(.closed, copy: copy)
        XCTAssertEqual(runner.state.account, .passkeyChosen)
        XCTAssertEqual(runner.state.step, .uses)
        await runner.commit(.start)
        XCTAssertTrue(runner.passkeyDue, "Start reopens them again")
        XCTAssertEqual(daemon.log, [])

        runner.finishPasskey(.created(name: "Laptop"), copy: copy)
        await runner.commit(.start)
        XCTAssertEqual(daemon.log, [.setConsentScopes(["required"]), .markComplete])
        XCTAssertTrue(runner.completed)
    }
}
