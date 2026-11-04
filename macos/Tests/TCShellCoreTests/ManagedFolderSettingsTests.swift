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

    /// The workflows line (owner, 2026-11-01): the daemon says whether the
    /// folder has Claude Code workflows; the sheet shows the line only for
    /// that folder, and only on a NEAR AI session. An older daemon that
    /// does not say shows nothing.
    func testTheWorkflowsLineShowsOnlyForAFolderWithWorkflowsOnNearAI() throws {
        func folder(_ workflows: String) throws -> ManagedFolderSettings {
            let json = #"{"tool":"claude","model":null,"collect_traces":false,"cli_options":"","remembered":false,"revision":1"# + workflows + "}"
            return try JSONDecoder().decode(ManagedFolderSettings.self, from: Data(json.utf8))
        }
        XCTAssertEqual(try folder(#","project_workflows":true"#).projectWorkflows, true)
        XCTAssertNil(try folder("").projectWorkflows, "an older daemon says nothing")

        var draft = ManagedFolderDraft()
        var token = draft.begin(tool: "claude", cwd: "/with-workflows")
        XCTAssertTrue(draft.complete(try folder(#","project_workflows":true"#), token: token))
        XCTAssertTrue(draft.projectWorkflows)
        for connection in ["near_ai", "near_ai_signin"] {
            XCTAssertTrue(ManagedSurface.showsProjectWorkflows(draft, tool: "claude", connection: connection), connection)
        }
        for connection in ["subscription", "api_key"] {
            XCTAssertFalse(ManagedSurface.showsProjectWorkflows(draft, tool: "claude", connection: connection), connection)
        }
        XCTAssertFalse(ManagedSurface.showsProjectWorkflows(draft, tool: "codex", connection: "near_ai"))

        // Another folder: nothing while it loads, then that folder's answer.
        token = draft.begin(tool: "claude", cwd: "/plain")
        XCTAssertFalse(ManagedSurface.showsProjectWorkflows(draft, tool: "claude", connection: "near_ai"))
        XCTAssertTrue(draft.complete(try folder(#","project_workflows":false"#), token: token))
        XCTAssertFalse(ManagedSurface.showsProjectWorkflows(draft, tool: "claude", connection: "near_ai"))
        token = draft.begin(tool: "claude", cwd: "/older")
        XCTAssertTrue(draft.complete(try folder(""), token: token))
        XCTAssertFalse(ManagedSurface.showsProjectWorkflows(draft, tool: "claude", connection: "near_ai"))
        draft.useDefaults(tool: "claude", cwd: "/project", cliOptions: "")
        XCTAssertFalse(ManagedSurface.showsProjectWorkflows(draft, tool: "claude", connection: "near_ai"))
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
