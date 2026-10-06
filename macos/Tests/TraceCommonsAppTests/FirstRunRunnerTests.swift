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
    func signInNearAI() async -> Bool { record(.signInNearAI) }
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
    case .signInNearAI: return .signInFailed
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

    /// The sheets are not awaited, so Start can run while they are open. A
    /// sign-out that ends them after Start leaves the finished first run
    /// where it is rather than reopening Join.
    func test_aSignOutAfterStartLeavesTheFinishedRunAlone() async throws {
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        var state = FirstRunState(
            step: .uses, account: .passkeyChosen, toolAnswers: [.claudeCode: .off, .codex: .off])
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
    }

    /// A daemon-reported enrolment recorded over an invite still in Join's
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
    /// `account_sign_in` needs an enrolment and refuses
    /// (`account-enrollment-required`). Join does not allow the pair
    /// (`JoinScreenLayout.canToggleNearAI`); should it reach the runner anyway,
    /// the step stays and says so with the core's sign-in line.
    func test_nearAIWithoutAnInviteReportsTheSignIn() async throws {
        var state = onFolders()
        state.invite = ""
        state.issuerHost = nil
        state.account = .nearAI
        let daemon = RecordingFirstRunDaemon()
        daemon.failing = { $0 == .signInNearAI }
        let runner = FirstRunRunner(state: state, daemon: daemon)

        await runner.commit(.leaveRoots)

        XCTAssertEqual(daemon.log, [.startDaemon(settingsJSON: state.sessionRoots.settingsJSON()!), .signInNearAI])
        XCTAssertEqual(runner.failure, .signInFailed)
        XCTAssertEqual(runner.state.step, .folders)
        XCTAssertFalse(runner.state.signedIn)
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        XCTAssertEqual(
            FoldersScreenLayout.notice(for: runner.failure, copy: copy, onboarding: TCOnboardingCopy.load()),
            copy.folders.signInFailed)
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
    /// (the daemon refuses them without an enrolment) and the watch-only
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
}
