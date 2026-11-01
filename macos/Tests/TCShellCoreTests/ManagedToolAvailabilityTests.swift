import XCTest
@testable import TCShellCore

final class ManagedToolAvailabilityTests: XCTestCase {
    private func snapshot(_ tools: String? = nil) throws -> ManagedSnapshot {
        let extra = tools.map { ",\"tools\":\($0)" } ?? ""
        return try JSONDecoder().decode(ManagedSnapshot.self, from: Data("{\"revision\":1,\"accounts\":[{\"id\":\"old\",\"tool\":\"claude\",\"connection\":\"api_key\",\"label\":\"Saved\",\"auth_state\":\"ready\"}],\"defaults\":[],\"generations\":{},\"sessions\":[],\"capabilities\":{\"managed_launch\":true,\"terminal_launch\":true}\(extra)}".utf8))
    }
    private func tools(_ installed: [String]) -> String {
        "[" + ["claude", "codex", "opencode"].map {
            "{\"tool\":\"\($0)\",\"installed\":\(installed.contains($0)),\"install_url\":\"https://example.com/\($0)\"}"
        }.joined(separator: ",") + "]"
    }
    func testHarnessSetupUsesExecutableEvidenceAndKeepsConfiguredRows() throws {
        let rows = try JSONDecoder().decode([HarnessRow].self, from: Data(#"[{"id":"claude","name":"Claude Code","installed":true,"connected":false,"connect_command":"","state":"available","can_connect":true,"can_disconnect":false},{"id":"codex","name":"Codex","installed":false,"connected":true,"connect_command":"","state":"connected","can_connect":false,"can_disconnect":true}]"#.utf8))
        let availability = try snapshot(tools(["opencode"]))
        XCTAssertEqual(ManagedSurface.visibleHarnesses(rows, tools: availability.tools).map(\.id), ["claude", "codex"])
        XCTAssertEqual(ManagedSurface.visibleHarnesses(rows, tools: nil).map(\.id), ["claude", "codex"])
    }
    func testPersistentDisconnectSurvivesUninstallAndAnOldProxyPort() throws {
        let rows = try JSONDecoder().decode([HarnessRow].self, from: Data(#"[{"id":"opencode","name":"OpenCode","installed":false,"connected":false,"connect_command":"","state":"not_connected","can_connect":false,"can_disconnect":true}]"#.utf8))
        let availability = try snapshot(tools([]))
        XCTAssertEqual(ManagedSurface.visibleHarnesses(rows, tools: availability.tools).map(\.id), ["opencode"])
        XCTAssertTrue(rows[0].canDisconnect)
    }
    func testUpdateRecoveryUsesAttemptedToolAndInstalledFallback() throws {
        let value = try snapshot(tools(["claude", "codex", "opencode"]))
        XCTAssertEqual(value.updateRecoveryTools(attemptedTool: "claude").map(\.tool), ["claude"])
        XCTAssertEqual(value.updateRecoveryTools(attemptedTool: "codex").map(\.tool), ["codex"])
        XCTAssertEqual(value.updateRecoveryTools(attemptedTool: "opencode").map(\.tool), ["opencode"])
        XCTAssertEqual(try snapshot(tools(["codex"])).updateRecoveryTools(attemptedTool: nil).map(\.tool), ["codex"])
        XCTAssertEqual(value.updateRecoveryTools(attemptedTool: "future"), [])
        XCTAssertEqual(try snapshot().updateRecoveryTools(attemptedTool: "codex"), [])
    }
    func testUnsupportedVersionHasUpdateRecoveryCopy() {
        XCTAssertEqual(ManagedSurface.errorKey(code: "managed-unsupported-version", message: nil), "unsupported_version")
        XCTAssertEqual(ManagedSurface.errorKey(code: nil, message: "managed-unsupported-version"), "unsupported_version")
    }
    func testOlderPayloadPreservesUnknownAvailability() throws {
        let value = try snapshot()
        XCTAssertNil(value.tools)
        XCTAssertEqual(value.setupTools, ["claude", "codex", "opencode"])
        XCTAssertTrue(value.canLaunch(tool: "claude"))
        XCTAssertFalse(value.needsToolInstallation)
    }
    func testZeroInstalledOffersInstallationWithoutLosingSavedAccount() throws {
        let value = try snapshot(tools([]))
        XCTAssertTrue(value.needsToolInstallation)
        XCTAssertEqual(value.setupTools, [])
        XCTAssertEqual(value.accounts.map(\.id), ["old"])
        XCTAssertFalse(value.canLaunch(tool: "claude"))
    }
    func testOneAndMultipleInstalledFilterSetup() throws {
        // #1417 made OpenCode a managed tool: it belongs in setup and launch.
        XCTAssertEqual(try snapshot(tools(["opencode"])).setupTools, ["opencode"])
        XCTAssertEqual(try snapshot(tools(["codex", "opencode"])).setupTools, ["codex", "opencode"])
        XCTAssertEqual(ManagedSurface.supportedTools, ["claude", "codex", "opencode"])
    }
    func testEveryInstallationCombinationFiltersSetup() throws {
        for mask in 0..<8 {
            let installed = ManagedSurface.supportedTools.enumerated().compactMap { mask & (1 << $0.offset) != 0 ? $0.element : nil }
            let value = try snapshot(tools(installed))
            XCTAssertEqual(value.setupTools, ["claude", "codex", "opencode"].filter(installed.contains))
            XCTAssertEqual(value.needsToolInstallation, installed.isEmpty)
            XCTAssertEqual(value.canLaunch(tool: "opencode"), installed.contains("opencode"))
            XCTAssertEqual(value.tools?.count, 3)
        }
    }
    func testSignInEntryIsPreservedButMissingToolCannotStart() throws {
        let json = #"{"revision":1,"accounts":[],"defaults":[],"generations":{},"sessions":[],"capabilities":{"managed_launch":true,"terminal_launch":true,"near_ai_signin":true},"sign_in_accounts":[{"id":"signin","tool":"claude","connection":"near_ai_signin","label":"NEAR AI","auth_state":"ready"}],"tools":[{"tool":"claude","installed":false,"install_url":"https://code.claude.com/docs/en/setup"}]}"#
        let value = try JSONDecoder().decode(ManagedSnapshot.self, from: Data(json.utf8))
        XCTAssertNotNil(value.signInEntry("claude"))
        XCTAssertNil(value.signInLaunchParams(tool: "claude", cwd: "/tmp", requestID: "r"))
    }
    func testRemovedToolRetainsAccountButCannotLaunch() throws {
        let value = try snapshot(tools(["opencode"]))
        XCTAssertEqual(value.accounts.first?.id, "old")
        XCTAssertFalse(value.canLaunch(tool: "claude"))
        XCTAssertTrue(value.canLaunch(tool: "opencode"))
        XCTAssertFalse(value.needsToolInstallation)
    }
    func testToolUnavailableNamesTheTool() {
        let copy = ["tool_unavailable": "{tool} wasn't found on this computer.", "claude": "Claude Code", "opencode": "OpenCode"]
        XCTAssertEqual(ManagedSurface.toolUnavailable("claude", copy: { copy[$0] ?? "" }), "Claude Code wasn't found on this computer.")
        XCTAssertEqual(ManagedSurface.toolUnavailable("opencode", copy: { copy[$0] ?? "" }), "OpenCode wasn't found on this computer.")
    }
}
