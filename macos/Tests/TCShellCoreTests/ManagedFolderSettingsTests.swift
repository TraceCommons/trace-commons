import XCTest
@testable import TCShellCore

final class ManagedFolderSettingsTests: XCTestCase {
    private func settings(tool: String = "codex") throws -> ManagedFolderSettings {
        let json = #"{"tool":""# + tool + #"","model":"custom-model","collect_traces":true,"cli_options":"--model custom","remembered":true,"revision":7}"#
        return try JSONDecoder().decode(ManagedFolderSettings.self, from: Data(json.utf8))
    }

    func testFolderSettingsDecodeWithoutAccountSelection() throws {
        let value = try settings()
        XCTAssertEqual(value.model, "custom-model")
        XCTAssertTrue(value.collectTraces)
        XCTAssertEqual(value.cliOptions, "--model custom")
        XCTAssertTrue(value.remembered)
        XCTAssertEqual(value.revision, 7)
    }

    func testOnlyTheCurrentFolderResponseCanRestoreTheDraft() throws {
        var draft = ManagedFolderDraft()
        let old = draft.begin(tool: "codex", cwd: "/first")
        let current = draft.begin(tool: "codex", cwd: "/second")
        XCTAssertFalse(draft.complete(try settings(), token: old))
        XCTAssertEqual(draft.phase, .loading)
        XCTAssertTrue(draft.complete(try settings(), token: current))
        XCTAssertEqual(draft.phase, .ready)
        XCTAssertEqual(draft.model, "custom-model")
        XCTAssertTrue(draft.matches(tool: "codex", cwd: "/second"))
        XCTAssertFalse(draft.matches(tool: "claude", cwd: "/second"))
    }

    func testAResponseForThePreviousAccountContextCannotEraseTyping() throws {
        var draft = ManagedFolderDraft()
        let old = draft.begin(tool: "codex", cwd: "/project")
        let current = draft.begin(tool: "codex", cwd: "/project")
        XCTAssertTrue(draft.complete(try settings(), token: current))
        draft.cliOptions = "--sandbox workspace-write"
        XCTAssertFalse(draft.complete(try settings(), token: old))
        draft.fail(token: old)
        XCTAssertEqual(draft.phase, .ready)
        XCTAssertEqual(draft.cliOptions, "--sandbox workspace-write")
    }

    func testFailedLookupAndWrongToolNeverLookReady() throws {
        var draft = ManagedFolderDraft()
        var token = draft.begin(tool: "codex", cwd: "/project")
        draft.fail(token: token)
        XCTAssertEqual(draft.phase, .failed)
        token = draft.begin(tool: "codex", cwd: "/project")
        XCTAssertFalse(draft.complete(try settings(tool: "claude"), token: token))
        XCTAssertEqual(draft.phase, .failed)
    }

    func testOlderDaemonUsesGlobalDefaultsWithoutRememberedSettings() {
        var draft = ManagedFolderDraft()
        draft.useDefaults(tool: "claude", cwd: "/project", cliOptions: "--model sonnet")
        XCTAssertEqual(draft.phase, .ready)
        XCTAssertNil(draft.model)
        XCTAssertFalse(draft.collectTraces)
        XCTAssertFalse(draft.remembered)
        XCTAssertEqual(draft.cliOptions, "--model sonnet")
    }
}

extension ManagedFolderSettingsTests {
    func testApplyingPreferencesPreservesTheCurrentlySelectedAccountForEveryAgent() {
        for tool in ManagedSurface.supportedTools {
            var draft = ManagedFolderDraft()
            draft.useDefaults(tool: tool, cwd: "/project", cliOptions: "--custom")
            draft.model = "chosen-model"
            draft.collectTraces = true
            let params = draft.applying(to: ["tool": tool, "connection": "near_ai_signin", "account_id": "current-account"], daemonKeepsSavedModel: true)
            XCTAssertEqual(params["account_id"] as? String, "current-account")
            XCTAssertEqual(params["model"] as? String, "chosen-model")
            XCTAssertEqual(params["collect_traces"] as? Bool, true)
            XCTAssertEqual(params["cli_options"] as? String, "--custom")
        }
    }

    func testAProxyModelIsNotSentToAnIncompatibleAccount() {
        var draft = ManagedFolderDraft()
        draft.useDefaults(tool: "codex", cwd: "/project", cliOptions: "")
        draft.model = "near-ai-model"
        XCTAssertNil(draft.applying(to: ["tool": "codex", "connection": "subscription"], daemonKeepsSavedModel: true)["model"])
    }

    func testCollectionIsolationRefusalUsesActionableSharedCopy() {
        XCTAssertEqual(ManagedSurface.errorKey(code: "managed-collection-account-busy", message: nil), "collection_account_busy")
    }

    func testCanonicalConfigurationProjectionDecodesTheRememberedChoices() throws {
        let value = try JSONDecoder().decode(DaemonData.Settings.self, from: Data(#"{"managed_folder_settings":[{"tool":"claude","cwd":"/project","model":"chosen","collect_traces":true,"cli_options":"--custom"}]}"#.utf8))
        XCTAssertEqual(value.managedFolderSettings?.first?.cwd, "/project")
        XCTAssertEqual(value.managedFolderSettings?.first?.tool, "claude")
        XCTAssertEqual(value.managedFolderSettings?.first?.collectTraces, true)
    }
}
