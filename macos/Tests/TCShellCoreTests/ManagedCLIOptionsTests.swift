import XCTest
@testable import TCShellCore

final class ManagedCLIOptionsTests: XCTestCase {
    private func snapshot(options: String = "") throws -> ManagedSnapshot {
        let field = options.isEmpty ? "" : #", "cli_options": "# + options
        return try JSONDecoder().decode(ManagedSnapshot.self, from: Data((#"{"revision":1,"accounts":[],"defaults":[],"generations":{},"sessions":[],"capabilities":{"managed_launch":true,"terminal_launch":false}"# + field + "}").utf8))
    }

    func testOlderDaemonDoesNotPretendToSupportOptions() throws {
        XCTAssertNil(try snapshot().cliOptions)
    }

    func testOptionsDecodeIndependentlyForEachAgent() throws {
        let value = try snapshot(options: #"{"claude":"--model opus","codex":"-c model_reasoning_effort=high","opencode":"--model provider/model"}"#)
        XCTAssertEqual(value.cliOptions?["claude"], "--model opus")
        XCTAssertEqual(value.cliOptions?["codex"], "-c model_reasoning_effort=high")
        XCTAssertEqual(value.cliOptions?["opencode"], "--model provider/model")
    }

    func testDraftSurvivesRefreshAndClearsOnlyAfterTheMatchingSave() {
        var drafts = ManagedCLIOptionsDrafts()
        drafts.edit(tool: "codex", options: "--model new")
        XCTAssertEqual(drafts.value(tool: "codex", saved: ["codex": "--model old"]), "--model new")
        XCTAssertTrue(drafts.isChanged(tool: "codex", saved: ["codex": "--model old"]))
        // Editing while a save is pending must preserve the later edit.
        drafts.edit(tool: "codex", options: "--model newer")
        drafts.saved(tool: "codex", options: "--model new")
        XCTAssertEqual(drafts.value(tool: "codex", saved: ["codex": "--model new"]), "--model newer")
        drafts.saved(tool: "codex", options: "--model newer")
        XCTAssertEqual(drafts.value(tool: "codex", saved: ["codex": "--model external"]), "--model external")
        XCTAssertFalse(drafts.isChanged(tool: "claude", saved: ["claude": ""]))
    }

    func testEmptyDraftCanResetSavedOptions() {
        var drafts = ManagedCLIOptionsDrafts()
        drafts.edit(tool: "claude", options: "")
        XCTAssertEqual(drafts.value(tool: "claude", saved: ["claude": "--model opus"]), "")
        XCTAssertTrue(drafts.isChanged(tool: "claude", saved: ["claude": "--model opus"]))
    }

    func testInvalidOptionsUseActionableSharedCopy() {
        XCTAssertEqual(ManagedSurface.errorKey(code: "managed-cli-options-invalid", message: nil), "cli_options_invalid")
    }
}
