import XCTest
@testable import TCShellCore
@testable import TraceCommonsApp

final class RedTeamLaunchSheetTests: XCTestCase {
    private func decodeSnapshot(_ json: String) throws -> ManagedSnapshot {
        try JSONDecoder().decode(ManagedSnapshot.self, from: Data(json.utf8))
    }

    func testRedTeamRowHiddenWhenSnapshotOmitsIt() throws {
        let snap = try decodeSnapshot("""
        {
            "tools": [
                {"tool": "claude", "installed": true, "install_url": "https://claude.ai"},
                {"tool": "codex", "installed": true, "install_url": "https://chatgpt.com"},
                {"tool": "opencode", "installed": false, "install_url": "https://opencode.ai"}
            ],
            "copy": {},
            "revision": 1,
            "accounts": [],
            "sign_in_accounts": [
                {"id": "1", "tool": "claude", "connection": "near_ai_signin", "label": "NEAR AI", "auth_state": "ready"}
            ],
            "defaults": [],
            "generations": {"claude": 0, "codex": 0, "opencode": 0},
            "sessions": [],
            "capabilities": {
                "managed_launch": true,
                "terminal_launch": true,
                "near_ai_signin": true
            }
        }
        """)
        XCTAssertFalse(snap.visibleTools.contains { $0.tool == "red_team" })
        XCTAssertFalse(snap.canLaunch(tool: "red_team"))
        XCTAssertFalse(snap.signInStartTools.contains("red_team"))
    }

    func testSandboxSectionsOnlyForRedTeam() {
        XCTAssertTrue(ManagedSurface.showsSandboxSection(for: "red_team"))
        XCTAssertFalse(ManagedSurface.showsSandboxSection(for: "claude"))
        XCTAssertFalse(ManagedSurface.showsSandboxSection(for: "codex"))
        XCTAssertFalse(ManagedSurface.showsSandboxSection(for: "opencode"))
    }

    func testNetworkIsOffByDefault() {
        let draft = RedTeamSandboxDraft()
        XCTAssertFalse(draft.networkEnabled)
        let settings = draft.sandboxSettings()
        XCTAssertTrue(settings.allowHosts.isEmpty)
        XCTAssertFalse(settings.packages)

        // Even if hosts/packages were edited before toggling network off,
        // sandboxSettings() omits them while networkEnabled is false.
        var disabled = RedTeamSandboxDraft(
            inputs: ["/tmp/sample.txt"],
            networkEnabled: false,
            allowHosts: ["example.com:443"],
            packages: true
        )
        let off = disabled.sandboxSettings()
        XCTAssertEqual(off.inputs, ["/tmp/sample.txt"])
        XCTAssertTrue(off.allowHosts.isEmpty)
        XCTAssertFalse(off.packages)

        disabled.networkEnabled = true
        let on = disabled.sandboxSettings()
        XCTAssertEqual(on.allowHosts, ["example.com:443"])
        XCTAssertTrue(on.packages)
    }

    func testLaunchSendsInputsHostsAndPackages() throws {
        let accountData = Data("""
        {"id": "rt-1", "tool": "red_team", "connection": "near_ai", "label": "NEAR AI", "auth_state": "ready"}
        """.utf8)
        let account = try JSONDecoder().decode(ManagedAccount.self, from: accountData)
        let draft = RedTeamSandboxDraft(
            inputs: ["/tmp/report.md", "/tmp/repo"],
            networkEnabled: true,
            allowHosts: ["example.com:443"],
            packages: true
        )
        let params = ManagedSurface.launchParams(
            account: account,
            cwd: "/tmp/work",
            requestID: "req-1",
            generation: 2,
            model: nil,
            collectTraces: true,
            sandbox: draft.sandboxSettings()
        )
        let sandbox = try XCTUnwrap(params["sandbox"] as? [String: Any])
        XCTAssertEqual(sandbox["inputs"] as? [String], ["/tmp/report.md", "/tmp/repo"])
        XCTAssertEqual(sandbox["allow_hosts"] as? [String], ["example.com:443"])
        XCTAssertEqual(sandbox["packages"] as? Bool, true)
    }
}
