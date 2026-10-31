import XCTest

@testable import TCShellCore

/// Owner, 2026-10-31: a second picker chooses the model Claude Code and
/// OpenCode use for background work. It offers only models of the main
/// model's tier or a more private one, and a choice the main model no
/// longer allows goes back to the default. The core enforces the same
/// rule when it builds the tool's configuration; these are the shell's
/// half.
final class ManagedBackgroundModelTests: XCTestCase {
    private static let copy = [
        "model_default": "Default: {model}", "model_default_unknown": "Default",
        "model_group_incognito": "Incognito models", "model_group_near_ai": "NEAR AI private models",
        "model_group_external": "External private models",
    ]

    private func catalogue(_ extra: String = "") throws -> ManagedModelCatalogue {
        try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(("""
        {"models":[
          {"id":"anthropic/claude-haiku-5","tier":"incognito"},
          {"id":"anthropic/claude-sonnet-5-5","tier":"incognito"},
          {"id":"z-ai/glm-5","tier":"external"},
          {"id":"Qwen/Qwen3.8-27B","tier":"near_ai"},
          {"id":"z-ai/glm-5.3-flash","tier":"near_ai"},
          {"id":"x/untiered"}
        ],"tier_order":["near_ai","external","incognito"],"default":"Qwen/Qwen3.8-27B",
        "background_defaults":{"Qwen/Qwen3.8-27B":"z-ai/glm-5.3-flash","z-ai/glm-5":"z-ai/glm-5",
          "anthropic/claude-sonnet-5-5":"anthropic/claude-haiku-5"}
        """ + extra + "}").utf8))
    }

    func testTheCatalogueSaysEachModelsBackgroundDefault() throws {
        let decoded = try catalogue()
        XCTAssertEqual(decoded.backgroundDefaults["Qwen/Qwen3.8-27B"], "z-ai/glm-5.3-flash")
        // An older daemon sends none.
        let older = try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(#"{"models":[],"default":null}"#.utf8))
        XCTAssertEqual(older.backgroundDefaults, [:])
    }

    /// The options are the default row, naming the background default with
    /// its tier, then the allowed models grouped by tier as the main picker
    /// groups them: the main model's tier and the more private ones.
    func testTheBackgroundPickerOffersOnlyTheMainModelsTierOrAMorePrivateOne() throws {
        let catalogue = try catalogue()
        let copy = { (key: String) in Self.copy[key] ?? "" }
        // No main model chosen: weighed against the catalogue's default.
        let privateMain = ManagedSurface.backgroundOptions(catalogue, main: nil, copy: copy)
        XCTAssertEqual(privateMain.map(\.value), ["", "Qwen/Qwen3.8-27B", "z-ai/glm-5.3-flash"])
        XCTAssertEqual(privateMain.first?.title, "Default: z-ai/glm-5.3-flash")
        XCTAssertEqual(privateMain.first?.privacy, .nearAIPrivate)
        XCTAssertEqual(privateMain.map(\.section), [nil, "NEAR AI private models", "NEAR AI private models"])
        let external = ManagedSurface.backgroundOptions(catalogue, main: "z-ai/glm-5", copy: copy)
        XCTAssertEqual(external.map(\.value), ["", "Qwen/Qwen3.8-27B", "z-ai/glm-5.3-flash", "z-ai/glm-5"])
        XCTAssertEqual(external.first?.title, "Default: z-ai/glm-5")
        let incognito = ManagedSurface.backgroundOptions(catalogue, main: "anthropic/claude-sonnet-5-5", copy: copy)
        XCTAssertEqual(incognito.map(\.value), [
            "", "Qwen/Qwen3.8-27B", "z-ai/glm-5.3-flash", "z-ai/glm-5",
            "anthropic/claude-haiku-5", "anthropic/claude-sonnet-5-5",
        ])
        XCTAssertEqual(incognito.first?.title, "Default: anthropic/claude-haiku-5")
        // A main model with no known tier claims nothing: every model.
        XCTAssertEqual(ManagedSurface.backgroundOptions(catalogue, main: "x/untiered", copy: copy).count, 7)
        // No catalogue: only the default.
        XCTAssertEqual(ManagedSurface.backgroundOptions(nil, main: nil, copy: copy).map(\.title), ["Default"])
    }

    /// Choosing a more private main model clears a background choice it no
    /// longer allows; one it allows is kept, and without a catalogue
    /// nothing is removed.
    func testAMorePrivateMainModelClearsALessPrivateBackgroundChoice() throws {
        let catalogue = try catalogue()
        var draft = ManagedFolderDraft()
        draft.useDefaults(tool: "claude", cwd: "/project", cliOptions: "")
        draft.choose("anthropic/claude-sonnet-5-5", in: catalogue)
        draft.chooseBackground("anthropic/claude-haiku-5")
        XCTAssertEqual(draft.backgroundModel, "anthropic/claude-haiku-5")
        draft.choose("z-ai/glm-5", in: catalogue)
        XCTAssertNil(draft.backgroundModel)
        draft.chooseBackground("Qwen/Qwen3.8-27B")
        draft.choose("Qwen/Qwen3.8-27B", in: catalogue)
        XCTAssertEqual(draft.backgroundModel, "Qwen/Qwen3.8-27B")
        // The default main model is the catalogue's, here NEAR AI private.
        draft.chooseBackground("z-ai/glm-5")
        draft.choose(nil, in: catalogue)
        XCTAssertNil(draft.backgroundModel)
        draft.chooseBackground("anthropic/claude-haiku-5")
        draft.reconcileBackground(in: nil)
        XCTAssertEqual(draft.backgroundModel, "anthropic/claude-haiku-5")
        draft.reconcileBackground(in: catalogue)
        XCTAssertNil(draft.backgroundModel)
    }

    /// The background model goes only to a daemon that reads it, and only
    /// for Claude Code and OpenCode on NEAR AI; a folder's saved choice is
    /// read back, and settings without one read as none.
    func testTheBackgroundModelIsSentOnlyWhereItIsRead() throws {
        XCTAssertTrue(ManagedSurface.offersBackgroundModel(tool: "claude", connection: "near_ai"))
        XCTAssertTrue(ManagedSurface.offersBackgroundModel(tool: "claude", connection: "near_ai_signin"))
        XCTAssertTrue(ManagedSurface.offersBackgroundModel(tool: "opencode", connection: "near_ai_signin"))
        XCTAssertFalse(ManagedSurface.offersBackgroundModel(tool: "codex", connection: "near_ai_signin"))
        XCTAssertFalse(ManagedSurface.offersBackgroundModel(tool: "claude", connection: "subscription"))
        XCTAssertFalse(ManagedSurface.offersBackgroundModel(tool: "opencode", connection: "near_ai"))
        var draft = ManagedFolderDraft()
        draft.useDefaults(tool: "claude", cwd: "/project", cliOptions: "")
        draft.chooseBackground("z-ai/glm-5.3-flash")
        let base: [String: Any] = ["tool": "claude", "connection": "near_ai_signin"]
        XCTAssertEqual(draft.applying(to: base, daemonKeepsSavedModel: true, daemonReadsBackground: true)["background_model"] as? String,
                       "z-ai/glm-5.3-flash")
        XCTAssertNil(draft.applying(to: base, daemonKeepsSavedModel: true, daemonReadsBackground: false)["background_model"])
        XCTAssertNil(draft.applying(to: ["tool": "codex", "connection": "near_ai_signin"], daemonKeepsSavedModel: true,
                                    daemonReadsBackground: true)["background_model"])
        draft.chooseBackground(nil)
        XCTAssertNil(draft.applying(to: base, daemonKeepsSavedModel: true, daemonReadsBackground: true)["background_model"])

        let saved = try JSONDecoder().decode(ManagedFolderSettings.self, from: Data(#"""
        {"tool":"claude","model":null,"background_model":"z-ai/glm-5.3-flash","collect_traces":false,
         "cli_options":"","remembered":true,"revision":1}
        """#.utf8))
        XCTAssertEqual(saved.backgroundModel, "z-ai/glm-5.3-flash")
        var loaded = ManagedFolderDraft()
        let token = loaded.begin(tool: "claude", cwd: "/project")
        XCTAssertTrue(loaded.complete(saved, token: token))
        XCTAssertEqual(loaded.backgroundModel, "z-ai/glm-5.3-flash")
        let older = try JSONDecoder().decode(ManagedFolderSettings.self, from: Data(#"""
        {"tool":"claude","model":null,"collect_traces":false,"cli_options":"","remembered":false,"revision":1}
        """#.utf8))
        XCTAssertNil(older.backgroundModel)
    }

    /// An older daemon's snapshot says nothing, which reads as no.
    func testTheSnapshotSaysWhetherTheDaemonReadsABackgroundModel() throws {
        let caps = try JSONDecoder().decode(ManagedSnapshot.Capabilities.self, from: Data(
            #"{"managed_launch":true,"terminal_launch":true,"background_model":true}"#.utf8))
        XCTAssertEqual(caps.backgroundModel, true)
        let older = try JSONDecoder().decode(ManagedSnapshot.Capabilities.self, from: Data(
            #"{"managed_launch":true,"terminal_launch":true}"#.utf8))
        XCTAssertNil(older.backgroundModel)
    }
}
