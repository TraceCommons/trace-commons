import Foundation
import TCBridge
import TCShellCore
import XCTest
import os
@testable import TraceCommonsApp

private final class FlagRefusingDaemon: DaemonCalling, Sendable {
    func call(_ method: String, params paramsJSON: String) -> String {
        #"{"id":1,"error":{"code":"unknown_method","message":"unknown-method"}}"#
    }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}
final class FeatureFlagIntegrationTests: XCTestCase {
    func testOlderDaemonDoesNotInventFeatureEnables() {
        XCTAssertThrowsError(try DaemonClient(daemon: FlagRefusingDaemon()).featureFlagsSnapshot())
    }
    @MainActor
    func testNoDaemonMeansDisabled() {
        let model = AppModel()
        XCTAssertFalse(model.featureIsEnabled("unknown"))
    }

    static func hermesFlags(enabled: Bool, pending: Bool? = nil, registered: Bool = true) throws -> FeatureFlagSnapshot {
        let flags = registered ? "\"managed_hermes\":{\"enabled\":\(enabled),\"pending\":\(pending ?? enabled),\"capability_blocked\":false,\"timing\":\"live\"}" : ""
        let json = """
        {"schema_version":1,"environment":"staging","profile_digest":"\(String(repeating: "a", count: 64))","instance_id":"63D037AB-023D-434D-9AD1-048C468AA6EB","generation":1,"sampled_at":"2026-11-05T00:00:00Z","source":"remote","revision":1,"fresh":true,"flags":{\(flags)}}
        """
        return try JSONDecoder().decode(FeatureFlagSnapshot.self, from: Data(json.utf8))
    }

    private func hermesSnapshot(installed: Bool = true) throws -> ManagedSnapshot {
        let json = """
        {"revision":7,"accounts":[{"id":"saved","tool":"hermes","connection":"api_key","label":"Work","auth_state":"ready"}],
        "sign_in_accounts":[{"id":"signin","tool":"hermes","connection":"near_ai_signin","label":"NEAR AI","auth_state":"ready"}],
        "defaults":[{"tool":"hermes","account_id":"saved","generation":2}],"generations":{"hermes":2},
        "tools":[{"tool":"hermes","installed":\(installed),"install_url":"https://example.com/hermes"}],"cli_options":{"hermes":"--model saved"},
        "sessions":[
        {"id":"running","tool":"hermes","connection":"api_key","account_id":"saved","account_label":"Work","project_label":"Project","cwd":"/tmp","state":"running","purpose":"coding","can_focus":true,"can_stop":true},
        {"id":"ended","tool":"hermes","connection":"api_key","account_id":"saved","account_label":"Work","project_label":"Project","cwd":"/tmp","state":"exited","purpose":"coding","can_start_again":true}],
        "capabilities":{"managed_launch":true,"terminal_launch":true,"near_ai_signin":true,"tool_setup":true}}
        """
        return try JSONDecoder().decode(ManagedSnapshot.self, from: Data(json.utf8))
    }

    @MainActor
    func testHermesSurfacesFollowLatestFlagsWithoutLosingSavedState() throws {
        let model = AppModel()
        let stale = try hermesSnapshot()
        model.managedSnapshot = stale
        for flags in [nil, try Self.hermesFlags(enabled: false, pending: true),
                      try Self.hermesFlags(enabled: true, registered: false)] as [FeatureFlagSnapshot?] {
            model.pendingManagedLaunch = "hermes"
            model.featureFlags = flags
            XCTAssertNil(model.pendingManagedLaunch)
            let off = try XCTUnwrap(model.managedSnapshot)
            XCTAssertFalse(model.managedToolIsEnabled("hermes"))
            XCTAssertTrue(model.managedToolIsEnabled("codex"))
            XCTAssertFalse(off.canLaunch(tool: "hermes"))
            XCTAssertFalse(off.setupTools.contains("hermes"))
            XCTAssertTrue(off.signInStartTools.isEmpty)
            XCTAssertNil(off.signInLaunchParams(tool: "hermes", cwd: nil, requestID: "stale-sheet"))
            XCTAssertTrue(off.missingInstallTools.isEmpty)
            XCTAssertTrue(off.updateRecoveryTools(attemptedTool: "hermes").isEmpty)
            XCTAssertFalse(ManagedSessionsSection.offersLaunch(off))
            XCTAssertFalse(MenuPanelStatus.showsToolButtons(off))
            XCTAssertNil(off.cliOptions?["hermes"])
            XCTAssertTrue(off.defaults.isEmpty)
            XCTAssertEqual(off.accounts, stale.accounts)
            XCTAssertEqual(off.sessions.map(\.id), stale.sessions.map(\.id))
            XCTAssertEqual(ManagedSurface.menuActions(off.sessions[0]), [.show, .stop])
            XCTAssertEqual(ManagedSurface.rowButtons(off.sessions[1], terminalLaunch: true), [.dismiss])
        }
        model.featureFlags = try Self.hermesFlags(enabled: true, pending: false)
        XCTAssertEqual(model.managedSnapshot, stale, "enabling must not erase retained accounts or history")
        XCTAssertEqual(model.managedSnapshot?.signInStartTools, ["hermes"])
        XCTAssertTrue(MenuPanelStatus.showsToolButtons(try XCTUnwrap(model.managedSnapshot)))
        XCTAssertNotNil(model.managedSnapshot?.signInLaunchParams(tool: "hermes", cwd: nil, requestID: "enabled"))
        model.managedSnapshot = try hermesSnapshot(installed: false)
        XCTAssertEqual(model.managedSnapshot?.missingInstallTools.map(\.tool), ["hermes"])
        model.pendingManagedLaunch = "codex"
        model.featureFlags = nil
        XCTAssertEqual(model.pendingManagedLaunch, "codex")
        XCTAssertTrue(try XCTUnwrap(model.managedSnapshot).missingInstallTools.isEmpty)
    }

    @MainActor
    func testDisabledHermesRejectsStaleActionsButKeepsSessionControls() async throws {
        let model = AppModel()
        let daemon = HermesActionDaemon()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.managedSnapshot = try hermesSnapshot()
        for method in ["managed_account_add", "managed_account_reconnect", "managed_launch_prepare",
                       "managed_session_start_again", "managed_session_resume", "managed_terminal_launch",
                       "managed_tool_setup", "managed_cli_options_set"] {
            for params in [["tool": "hermes"], ["account_id": "saved"], ["account_id": "signin"], ["session_id": "ended"]] {
                model.managedAction(method, params: params, openTerminal: true)
                XCTAssertFalse(model.managedBusy)
            }
        }
        model.setupManagedTool("hermes", action: "install")
        model.managedCLIOptionsDrafts.edit(tool: "hermes", options: "--model draft")
        model.saveManagedCLIOptions(tool: "hermes")
        XCTAssertTrue(daemon.methods.isEmpty)
        model.showManagedSession("running")
        try await waitForHermesAction(model)
        XCTAssertTrue(daemon.methods.contains("managed_session_focus"))
        for method in ["managed_session_stop", "managed_session_dismiss"] {
            model.managedAction(method, params: ["session_id": "running"])
            try await waitForHermesAction(model)
            XCTAssertTrue(daemon.methods.contains(method))
        }
        model.managedAction("managed_launch_prepare", params: ["tool": "codex"], openTerminal: true)
        try await waitForHermesAction(model)
        XCTAssertTrue(daemon.methods.contains("managed_launch_prepare"))
        XCTAssertTrue(daemon.methods.contains("managed_terminal_launch"))
    }

    @MainActor
    func testQueuedHermesActionCannotStartAfterLiveDisable() async throws {
        let model = AppModel()
        let daemon = HermesActionDaemon()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.managedSnapshot = try hermesSnapshot()
        model.featureFlags = try Self.hermesFlags(enabled: true)
        model.managedAction("managed_launch_prepare", params: ["tool": "hermes"], openTerminal: true)
        model.featureFlags = try Self.hermesFlags(enabled: false)
        try await waitForHermesAction(model)
        XCTAssertTrue(daemon.methods.isEmpty)
    }

    @MainActor
    func testLiveDisableBetweenLaunchStepsDoesNotOpenTerminal() async throws {
        for parkedMethod in ["set_managed_terminal", "managed_launch_prepare", "managed_account_add"] {
            let model = AppModel()
            let daemon = HermesActionDaemon(parkedMethod: parkedMethod)
            defer { daemon.release() }
            model.setClientForTesting(DaemonClient(daemon: daemon))
            model.managedSnapshot = try hermesSnapshot()
            model.featureFlags = try Self.hermesFlags(enabled: true)
            let method = parkedMethod == "managed_account_add" ? parkedMethod : "managed_launch_prepare"
            model.managedAction(method, params: ["tool": "hermes"], openTerminal: true,
                                terminal: parkedMethod == "set_managed_terminal" ? "terminal" : nil)
            for _ in 0..<300 where !daemon.waiting { try await Task.sleep(for: .milliseconds(10)) }
            XCTAssertTrue(daemon.waiting)
            model.featureFlags = try Self.hermesFlags(enabled: false)
            daemon.release()
            try await waitForHermesAction(model)
            XCTAssertFalse(daemon.methods.contains("managed_terminal_launch"))
            XCTAssertFalse(daemon.methods.contains("managed_account_reconnect"))
            if parkedMethod == "set_managed_terminal" {
                XCTAssertFalse(daemon.methods.contains("managed_launch_prepare"))
            }
        }
    }

    @MainActor
    private func waitForHermesAction(_ model: AppModel) async throws {
        for _ in 0..<300 where model.managedBusy { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertFalse(model.managedBusy)
    }

    func testFeatureEventRequestsSnapshotRefresh() {
        XCTAssertEqual(DaemonEventParser.parse(#"{"event":"feature_flags_changed","data":{"generation":2}}"#), .featureFlagsChanged)
    }
    @MainActor
    func testGapEventsReplaceStaleEnabledSnapshotWithDaemonDecision() async throws {
        let dir = URL(fileURLWithPath: "/private/tmp/tc-fgap-\(UUID().uuidString.prefix(8))")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let model = AppModel()
        defer { model.shutdown() }
        let started = expectation(description: "daemon running")
        model.startDaemon(at: dir.path, settingsJSON: #"{"claude_source":{"mode":"off"},"codex_source":{"mode":"off"}}"#) { _ in started.fulfill() }
        await fulfillment(of: [started], timeout: 10)
        for _ in 0..<100 {
            if model.featureFlags != nil { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        let current = try XCTUnwrap(model.featureFlags)
        // Simulate an enabled snapshot left from an earlier daemon instance after
        // losing a disable/expiry update. The authoritative daemon has it off.
        let stale: [String: Any] = [
            "schema_version": 1, "environment": current.environment,
            "profile_digest": current.profileDigest, "instance_id": UUID().uuidString,
            "generation": 1, "sampled_at": current.sampledAt, "source": "remote",
            "revision": 1, "expires_at": "2026-11-05T00:00:00Z", "fresh": true,
            "flags": ["missed-disable-fixture": ["enabled": true, "pending": true,
                "capability_blocked": false, "timing": "live"]]
        ]
        let encoded = try JSONSerialization.data(withJSONObject: stale)
        for event in [DaemonEvent.resyncRequired, .lagged(skipped: 1)] {
            model.featureFlags = try JSONDecoder().decode(FeatureFlagSnapshot.self, from: encoded)
            XCTAssertTrue(model.featureIsEnabled("missed-disable-fixture"))
            model.handle(event: event)
            for _ in 0..<100 {
                if !model.featureIsEnabled("missed-disable-fixture") { break }
                try await Task.sleep(for: .milliseconds(10))
            }
            XCTAssertFalse(model.featureIsEnabled("missed-disable-fixture"), "gap recovery must reread flags for \(event)")
        }
    }

    func testEmbeddedAndAttachedSnapshotsAgreeWithFreshFFI() throws {
        let dir = URL(fileURLWithPath: "/private/tmp/tc-flags-\(UUID().uuidString.prefix(8))")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let embedded = try TCDaemon(configDir: dir.path, settingsJSON: #"{"claude_source":{"mode":"off"},"codex_source":{"mode":"off"}}"#)
        defer { embedded.shutdown() }
        let attached = try TCDaemon(attachingTo: dir.path)
        defer { attached.shutdown() }
        let a = try DaemonClient(daemon: embedded).featureFlagsSnapshot()
        let b = try DaemonClient(daemon: attached).featureFlagsSnapshot()
        XCTAssertEqual(a, b)
        XCTAssertFalse(a.isEnabled("unregistered-fixture"))
        let identity = try JSONDecoder().decode(BuildFeatureProfile.self, from: Data(XCTUnwrap(TCFeatureFlags.buildProfileJSON()).utf8))
        XCTAssertTrue(identity.agrees(environment: a.environment, digest: a.profileDigest))
    }
}

private final class HermesActionDaemon: DaemonCalling {
    private let captured = OSAllocatedUnfairLock(initialState: [String]())
    private let isWaiting = OSAllocatedUnfairLock(initialState: false)
    private let gate = DispatchSemaphore(value: 0)
    private let parkedMethod: String?
    init(parkedMethod: String? = nil) { self.parkedMethod = parkedMethod }
    var methods: [String] { captured.withLock { $0 } }
    var waiting: Bool { isWaiting.withLock { $0 } }
    func release() { gate.signal() }
    func call(_ method: String, params paramsJSON: String) -> String {
        guard method != "managed_snapshot" else {
            return #"{"id":1,"error":{"code":"unavailable","message":"test-refusal"}}"#
        }
        captured.withLock { $0.append(method) }
        if method == parkedMethod {
            isWaiting.withLock { $0 = true }
            gate.wait()
        }
        return #"{"id":1,"result":{"revision":8,"id":"new-account","session_id":"prepared","ticket":"ticket"}}"#
    }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}
