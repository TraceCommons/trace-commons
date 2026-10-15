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
        XCTAssertEqual(ManagedSurface.visibleHarnesses(rows, tools: availability.tools).map(\.id), ["codex"])
        XCTAssertEqual(ManagedSurface.visibleHarnesses(rows, tools: nil).map(\.id), ["claude", "codex"])
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
    func testMissingToolErrorHasInstallationRecoveryCopy() {
        XCTAssertEqual(ManagedSurface.errorKey(code: "tool-unavailable", message: nil), "tool_unavailable")
        XCTAssertEqual(ManagedSurface.errorKey(code: nil, message: "managed-tool-unavailable"), "tool_unavailable")
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
        XCTAssertEqual(try snapshot(tools(["opencode"])).setupTools, ["opencode"])
        XCTAssertEqual(try snapshot(tools(["codex", "opencode"])).setupTools, ["codex", "opencode"])
    }
    func testRemovedToolRetainsAccountButCannotLaunch() throws {
        let value = try snapshot(tools(["opencode"]))
        XCTAssertEqual(value.accounts.first?.id, "old")
        XCTAssertFalse(value.canLaunch(tool: "claude"))
        XCTAssertTrue(value.canLaunch(tool: "opencode"))
    }
}
