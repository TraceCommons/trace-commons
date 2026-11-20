import Foundation
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// The daemon reports a `red_team` tool row (with the flag on) whose
/// `installed` follows `<config>/red-team/runtime`, and installs that runtime
/// on `managed_tool_setup` `{"tool":"red_team","action":"install"}`. The Start
/// button needs it installed, so the app must offer that install.
@MainActor
final class RedTeamSetupTests: XCTestCase {
    private func snapshot(redTeam: Bool?) throws -> ManagedSnapshot {
        var tools: [[String: Any]] = ManagedSurface.supportedTools.map {
            ["tool": $0, "installed": true, "install_url": "https://example.com/" + $0]
        }
        if let redTeam {
            tools.append(["tool": "red_team", "installed": redTeam, "install_url": NSNull()])
        }
        let value: [String: Any] = [
            "revision": 0, "accounts": [], "defaults": [], "generations": [:], "sessions": [],
            "capabilities": ["managed_launch": true, "terminal_launch": true, "near_ai_signin": false,
                             "terminal_destination": "Terminal", "tool_setup": true],
            "tools": tools,
        ]
        return try JSONDecoder().decode(ManagedSnapshot.self, from: JSONSerialization.data(withJSONObject: value))
    }

    /// The NEAR AI card's install links stay the coding tools'; Red Team is
    /// set up from Settings (owner, 2026-11-19).
    func testTheNearAICardOffersNoRedTeamInstall() throws {
        for redTeam in [false, true, nil] as [Bool?] {
            XCTAssertEqual(try snapshot(redTeam: redTeam).missingInstallTools.map(\.tool), [])
        }
    }

    func testRedTeamSetupFollowsTheDaemonsRow() throws {
        XCTAssertEqual(try snapshot(redTeam: false).redTeamSetup, .notInstalled)
        XCTAssertEqual(try snapshot(redTeam: true).redTeamSetup, .installed)
        XCTAssertNil(try snapshot(redTeam: nil).redTeamSetup)
    }

    /// The Private AI tab's card: offered while the runtime is not
    /// installed and the card was not dismissed; never once installed or
    /// with the flag off (no row).
    func testTheSetupCardIsOfferedOnlyUntilInstalledOrDismissed() throws {
        XCTAssertTrue(RedTeamSetupCard.offered(setup: try snapshot(redTeam: false).redTeamSetup, dismissed: false))
        XCTAssertFalse(RedTeamSetupCard.offered(setup: try snapshot(redTeam: false).redTeamSetup, dismissed: true))
        XCTAssertFalse(RedTeamSetupCard.offered(setup: try snapshot(redTeam: true).redTeamSetup, dismissed: false))
        XCTAssertFalse(RedTeamSetupCard.offered(setup: try snapshot(redTeam: nil).redTeamSetup, dismissed: false))
    }

    func testTheTabCardAndSettingsRowAreDrawn() throws {
        let tab = try GlassSurfaceRulesTests.text("Views/Monitor/InferenceViews.swift")
        XCTAssertTrue(tab.contains("RedTeamSetupPrompt(onOpenSettings: onOpenSettings)"))
        let settings = try GlassSurfaceRulesTests.text("Views/Monitor/InferenceAccount.swift")
        XCTAssertTrue(settings.contains("ClaudeDesktopSettingsRow(setup: claudeDesktop, closeLabel: MonitorWords.close)\n                        RedTeamSettingsRow()"))
    }

    func testInstallingRedTeamSendsOnlyTheInstallAction() async throws {
        let daemon = RedTeamSetupDaemon()
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.managedSnapshot = try snapshot(redTeam: false)
        model.setupManagedTool("red_team", action: "install")
        for _ in 0..<300 where model.managedBusy { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertFalse(model.managedBusy)
        XCTAssertEqual(daemon.setupRequests, [["tool": "red_team", "action": "install"]])
        XCTAssertNil(model.managedErrorKey)
    }

    func testRedTeamHasNoUpdateAction() throws {
        // The daemon refuses `update` for Red Team (it installs the pinned
        // release); the app does not send it.
        let daemon = RedTeamSetupDaemon()
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.managedSnapshot = try snapshot(redTeam: false)
        model.setupManagedTool("red_team", action: "update")
        XCTAssertTrue(daemon.setupRequests.isEmpty)
    }
}

private final class RedTeamSetupDaemon: DaemonCalling, @unchecked Sendable {
    private let lock = NSLock()
    private var sent: [[String: String]] = []
    var setupRequests: [[String: String]] { lock.withLock { sent } }

    func call(_ method: String, params paramsJSON: String) -> String {
        if method == "managed_tool_setup" {
            let params = (try? JSONDecoder().decode([String: String].self, from: Data(paramsJSON.utf8))) ?? [:]
            lock.withLock { sent.append(params) }
            return #"{"id":1,"result":{"opened":true,"installed":true}}"#
        }
        return #"{"id":1,"result":{"revision":0,"accounts":[],"defaults":[],"generations":{},"sessions":[],"capabilities":{"managed_launch":true,"terminal_launch":true,"tool_setup":true}}}"#
    }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}

/// Red Team's button and session rows draw the NEAR AI Red SandBox mark,
/// traced from rts's README splash, not the "Re" initials.
final class RedTeamLogoTests: XCTestCase {
    func testRedTeamDrawsTheSandBoxMark() {
        XCTAssertEqual(ManagedWords.glassTool("red_team"), .redTeam)
        XCTAssertEqual(ManagedWords.glassTool("red_team").logo, .redTeam)
        XCTAssertEqual(ManagedWords.glassTool("opencode"), .openCode)
    }
}
