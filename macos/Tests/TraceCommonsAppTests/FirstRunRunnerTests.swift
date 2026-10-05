import Foundation
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// A recording fake for each first-run call. Every call is logged as the
/// `FirstRunCall` it carries out, so the log compares directly with
/// `FirstRunPlan.calls(for:at:)`.
@MainActor
private final class RecordingDaemon: FirstRunDaemon {
    var log: [FirstRunCall] = []
    /// The call that answers as a failure; every other call succeeds.
    var failing: ((FirstRunCall) -> Bool) = { _ in false }
    var lookup: FirstRunLookup = .found(RecordingDaemon.validLookup)
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

    func markComplete() { log.append(.markComplete) }
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
        return state
    }

    func test_aFailedStartKeepsTheInviteAndJoinsNothing() async {
        let daemon = RecordingDaemon()
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

    func test_aDeadInviteReturnsToJoinWithAnswersKept() async {
        let daemon = RecordingDaemon()
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
        daemon.lookup = .found(RecordingDaemon.validLookup)
        daemon.log = []
        await runner.commit(.leaveRoots)
        XCTAssertEqual(daemon.log, [.lookupInvite("INVITE-2"), .enroll("INVITE-2"), .signInNearAI])
        XCTAssertEqual(runner.state.enrolledInvite, "INVITE-2")
        XCTAssertEqual(runner.lookup, RecordingDaemon.validLookup)
    }

    func test_aRefusedGrantFinishesOnAskMe() async {
        let daemon = RecordingDaemon()
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
            if case .markComplete = call { continue }
            let daemon = RecordingDaemon()
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
        let succeeding = RecordingDaemon()
        let runner = FirstRunRunner(state: onUses(), daemon: succeeding)
        await runner.commit(.start)
        XCTAssertEqual(succeeding.log, plan)
        XCTAssertNil(runner.failure)
        XCTAssertEqual(runner.state.sharing, .automatic)

        // Every call that can fail stops the run where it failed. The grant
        // is the one exception (`test_aRefusedGrantFinishesOnAskMe`), and
        // `markComplete` cannot fail.
        for (index, call) in plan.enumerated() {
            if case .grantAutomatic = call { continue }
            if case .markComplete = call { continue }
            let daemon = RecordingDaemon()
            daemon.failing = { $0 == call }
            let failed = FirstRunRunner(state: onUses(), daemon: daemon)
            await failed.commit(.start)
            XCTAssertEqual(daemon.log, Array(plan.prefix(index + 1)), "\(call)")
            XCTAssertNotNil(failed.failure, "\(call) failed silently")
            XCTAssertFalse(daemon.log.contains(.markComplete), "\(call)")
        }

        var roots = onFolders()
        roots.daemonStarted = true
        roots.startedSettingsJSON = #"{"claude_source":{"mode":"off"}}"#
        let rootsPlan = FirstRunPlan.calls(for: roots, at: .leaveRoots)
        for (index, call) in rootsPlan.enumerated() {
            let daemon = RecordingDaemon()
            if case .lookupInvite = call {
                daemon.lookup = .refused(label: "invite-invalid")
            } else {
                daemon.failing = { $0 == call }
            }
            let failed = FirstRunRunner(state: roots, daemon: daemon)
            await failed.commit(.leaveRoots)
            XCTAssertEqual(daemon.log, Array(rootsPlan.prefix(index + 1)), "\(call)")
            XCTAssertNotNil(failed.failure, "\(call) failed silently")
            XCTAssertNotEqual(failed.state.step, .uses, "\(call) moved on after failing")
        }
    }

    func test_aSuccessfulLeaveRecordsTheDaemonFactsAndMovesOn() async {
        let daemon = RecordingDaemon()
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
}
