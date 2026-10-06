import Foundation
import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// A daemon that answers each method from a script: a canned result, a
/// refusal, or (for `parked`) nothing until the test lets it.
private final class ScriptedDaemon: DaemonCalling {
    enum Reply {
        case result(String)
        case refused
        case parked(String)
    }

    private let lock = NSLock()
    private var replies: [String: Reply]
    private let gate = DispatchSemaphore(value: 0)
    private var asked: [String] = []

    init(_ replies: [String: Reply]) { self.replies = replies }

    var methods: [String] {
        lock.lock()
        defer { lock.unlock() }
        return asked
    }

    func release() { gate.signal() }

    /// Changes what `method` answers from the next call on.
    func reply(_ method: String, with reply: Reply) {
        lock.lock()
        replies[method] = reply
        lock.unlock()
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        lock.lock()
        asked.append(method)
        let reply = replies[method]
        lock.unlock()
        switch reply {
        case .result(let json):
            return #"{"id":1,"result":"# + json + "}"
        case .parked(let json):
            gate.wait()
            return #"{"id":1,"result":"# + json + "}"
        case .refused, nil:
            return #"{"id":1,"error":{"code":"unavailable","message":"test-refusal"}}"#
        }
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}

/// #1229 review: a Settings section's "nothing here" sentence follows the
/// answer to the call its list comes from, not `status`; a read that never
/// answers draws the core's failure line, never an endless spinner.
final class SettingsReadStateTests: XCTestCase {
    private func status(_ json: String) throws -> DaemonStatus {
        try JSONDecoder().decode(DaemonStatus.self, from: Data(json.utf8))
    }

    @MainActor
    private func waitUntil(_ condition: @MainActor () -> Bool) async throws {
        for _ in 0..<300 where !condition() {
            try await Task.sleep(nanoseconds: 10_000_000)
        }
    }

    // MARK: - The rule

    func testAnAnswerWinsAndOtherwiseADownCoreOrAFailedReadIsNeverLoading() {
        let starting = AppModel.Startup.starting
        let running = AppModel.Startup.running
        let refused = AppModel.Startup.refused("x")
        let roots = AppModel.Startup.needsRoots

        // Stale data is still real data: a later failure does not hide it.
        for startup in [starting, running, refused, roots] {
            XCTAssertEqual(SettingsRead.resolve(answered: true, failed: false, startup: startup), .answered)
            XCTAssertEqual(SettingsRead.resolve(answered: true, failed: true, startup: startup), .answered)
        }
        XCTAssertEqual(SettingsRead.resolve(answered: false, failed: false, startup: starting), .awaiting)
        XCTAssertEqual(SettingsRead.resolve(answered: false, failed: false, startup: running), .awaiting)
        XCTAssertEqual(SettingsRead.resolve(answered: false, failed: true, startup: running), .failed)
        // A core that never started is not a read to retry.
        for startup in [refused, roots] {
            XCTAssertEqual(SettingsRead.resolve(answered: false, failed: false, startup: startup), .coreDown)
            XCTAssertEqual(SettingsRead.resolve(answered: false, failed: true, startup: startup), .coreDown)
        }
    }

    /// The unavailable line is the core's: request-failed with a retry for
    /// a failed read, core-unreachable with none for a core that is down.
    func testTheUnavailableLineIsTheCoresAndOnlyAFailedReadOffersRetry() throws {
        let screens = try XCTUnwrap(MonitorScreensCopy.decode(fromJSON: TCCoreCopy.monitorScreensCopyJSON()))
        XCTAssertEqual(SettingsUnavailable.line(.failed, screens: screens), screens.requestFailed)
        XCTAssertEqual(SettingsUnavailable.line(.coreDown, screens: screens), screens.coreUnreachable)
        XCTAssertTrue(SettingsUnavailable.offersRetry(.failed))
        XCTAssertFalse(SettingsUnavailable.offersRetry(.coreDown))
        // With the table missing, a dash: never nothing.
        XCTAssertEqual(SettingsUnavailable.line(.failed, screens: nil), "\u{2014}")
    }

    // MARK: - (a) the empty sentence waits on its own read

    @MainActor
    func testAFailedAuditReadIsNotAnEmptyLog() async throws {
        let daemon = ScriptedDaemon(["list_audit": .refused])
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.setStartupForTesting(.running)
        model.setStatusForTesting(try status(#"{"logged_in":true,"schema_version":"1"}"#))
        XCTAssertEqual(model.auditRead, .awaiting)

        model.refreshAudit()
        try await waitUntil { model.auditRead != .awaiting }
        XCTAssertTrue(model.audit.isEmpty)
        XCTAssertEqual(model.auditRead, .failed, "a failed list_audit reads as \"Nothing has been changed.\"")
    }

    @MainActor
    func testAnAuditReadInFlightIsAwaitingAndAnEmptyAnswerIsAnswered() async throws {
        let daemon = ScriptedDaemon(["list_audit": .parked(#"{"entries":[]}"#)])
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.setStartupForTesting(.running)
        model.setStatusForTesting(try status(#"{"logged_in":true,"schema_version":"1"}"#))

        model.refreshAudit()
        try await waitUntil { daemon.methods.contains("list_audit") }
        XCTAssertEqual(model.auditRead, .awaiting, "status answering is not list_audit answering")
        daemon.release()
        try await waitUntil { model.auditRead != .awaiting }
        XCTAssertEqual(model.auditRead, .answered)
    }

    @MainActor
    func testAFailedProjectsReadIsNotNoProjects() async throws {
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: ScriptedDaemon(["list_projects": .refused])))
        model.setStartupForTesting(.running)
        model.setStatusForTesting(try status(#"{"logged_in":true,"schema_version":"1"}"#))
        XCTAssertEqual(model.projectsRead, .awaiting)

        model.refreshProjects()
        try await waitUntil { model.projectsRead != .awaiting }
        XCTAssertTrue(model.projects.isEmpty)
        XCTAssertEqual(model.projectsRead, .failed)

        let answered = AppModel()
        answered.setClientForTesting(DaemonClient(daemon: ScriptedDaemon(["list_projects": .result(#"{"projects":[]}"#)])))
        answered.setStartupForTesting(.running)
        answered.refreshProjects()
        try await waitUntil { answered.projectsRead != .awaiting }
        XCTAssertEqual(answered.projectsRead, .answered)
    }

    /// Signed in, a profile read that failed may be hiding a roster entry,
    /// so the opt-in card is not drawn. Signed out, the failure is the
    /// daemon saying there is nothing claimed, and the opt-in card stands.
    @MainActor
    func testAFailedProfileReadWhileSignedInIsNotOffTheRoster() async throws {
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: ScriptedDaemon(["get_public_profile": .refused])))
        model.setStartupForTesting(.running)
        model.setStatusForTesting(try status(#"{"logged_in":true,"schema_version":"1"}"#))
        XCTAssertEqual(model.publicProfileRead, .awaiting)
        model.refreshPublicProfile()
        try await waitUntil { model.publicProfileRead != .awaiting }
        XCTAssertNil(model.publicProfile)
        XCTAssertEqual(model.publicProfileRead, .failed)

        let signedOut = AppModel()
        signedOut.setClientForTesting(DaemonClient(daemon: ScriptedDaemon(["get_public_profile": .refused])))
        signedOut.setStartupForTesting(.running)
        signedOut.setStatusForTesting(try status(#"{"logged_in":false,"schema_version":"1"}"#))
        signedOut.refreshPublicProfile()
        try await waitUntil { signedOut.publicProfileRead != .awaiting }
        XCTAssertEqual(signedOut.publicProfileRead, .answered)

        let offRoster = AppModel()
        offRoster.setClientForTesting(DaemonClient(daemon: ScriptedDaemon([
            "get_public_profile": .result(#"{"on_roster":false}"#),
        ])))
        offRoster.setStartupForTesting(.running)
        offRoster.setStatusForTesting(try status(#"{"logged_in":true,"schema_version":"1"}"#))
        offRoster.refreshPublicProfile()
        try await waitUntil { offRoster.publicProfileRead != .awaiting }
        XCTAssertEqual(offRoster.publicProfileRead, .answered)
    }

    /// An answered read stays answered: one refused refresh after an
    /// on-roster answer, while signed in, must not turn a contributor on the
    /// roster into one shown the opt-in card. Signed out, the refusal is the
    /// daemon saying nothing is claimed, and the cache goes.
    @MainActor
    func testARefusedProfileRefreshWhileSignedInKeepsTheRosterEntry() async throws {
        for loggedIn in [true, false] {
            let daemon = ScriptedDaemon([
                "get_public_profile": .result(#"{"on_roster":true,"handle":"zed"}"#),
            ])
            let model = AppModel()
            model.setClientForTesting(DaemonClient(daemon: daemon))
            model.setStartupForTesting(.running)
            model.setStatusForTesting(try status(#"{"logged_in":\#(loggedIn),"schema_version":"1"}"#))
            model.refreshPublicProfile()
            try await waitUntil { model.publicProfile != nil }
            XCTAssertEqual(model.publicProfile?.handle, "zed")
            XCTAssertEqual(model.publicProfileRead, .answered)

            daemon.reply("get_public_profile", with: .refused)
            model.refreshPublicProfile()
            try await waitUntil { model.failedReads.contains("get_public_profile") }
            if loggedIn {
                XCTAssertEqual(model.publicProfile?.handle, "zed",
                    "signed-in on-roster contributor lost the cached profile after one failed refresh")
                XCTAssertEqual(model.publicProfileRead, .answered)
            } else {
                XCTAssertNil(model.publicProfile)
                XCTAssertEqual(model.publicProfileRead, .answered)
            }
        }
    }

    // MARK: - (b) a status that never answers is not a spinner

    @MainActor
    func testACoreThatNeverStartedIsCoreDownForEveryRead() {
        for startup in [AppModel.Startup.refused("x"), .needsRoots] {
            let model = AppModel()
            model.setStartupForTesting(startup)
            XCTAssertEqual(model.statusRead, .coreDown, "\(startup)")
            XCTAssertEqual(model.settingsRead, .coreDown, "\(startup)")
            XCTAssertEqual(model.auditRead, .coreDown, "\(startup)")
            XCTAssertEqual(model.projectsRead, .coreDown, "\(startup)")
            XCTAssertEqual(model.publicProfileRead, .coreDown, "\(startup)")
        }
        // Refused before the config directory resolved: no witness read can
        // ever run, so the witness card is not left loading either.
        let refused = AppModel()
        refused.setStartupForTesting(.refused("x"))
        XCTAssertEqual(refused.witnessRead, .coreDown)
        XCTAssertEqual(AppModel().witnessRead, .awaiting)
    }

    @MainActor
    func testFailedStatusAndSettingsReadsAreFailedNotLoading() async throws {
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: ScriptedDaemon([:])))
        model.setStartupForTesting(.running)
        model.refreshStatus()
        model.refreshSettings()
        try await waitUntil { model.statusRead != .awaiting && model.settingsRead != .awaiting }
        XCTAssertEqual(model.statusRead, .failed)
        XCTAssertEqual(model.settingsRead, .failed)
    }

    /// A signed-out status that omits `schema_version` decodes equal to the
    /// `.unknown` placeholder; it is still an answer.
    @MainActor
    func testAStatusEqualToThePlaceholderIsStillAnAnswer() async throws {
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: ScriptedDaemon(["status": .result(#"{"logged_in":false}"#)])))
        model.setStartupForTesting(.running)
        model.refreshStatus()
        try await waitUntil { model.statusRead != .awaiting }
        XCTAssertEqual(model.statusRead, .answered)
    }

    // MARK: - (c) the witness card has a loading state

    @MainActor
    func testTheWitnessIsAwaitingUntilItsStateIsRead() {
        let model = AppModel()
        model.setStartupForTesting(.running)
        XCTAssertNil(model.witnessStateCode)
        XCTAssertEqual(model.witnessRead, .awaiting)
    }
}
