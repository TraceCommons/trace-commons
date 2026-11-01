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

    /// A catalogue answer as the core sends it: for each main model, its
    /// background default and the index of the background ids it allows
    /// (`models::background_table`). The shell only looks these up.
    private func catalogue() throws -> ManagedModelCatalogue {
        try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(#"""
        {"models":[
          {"id":"anthropic/claude-haiku-5","tier":"incognito"},
          {"id":"anthropic/claude-sonnet-5-5","tier":"incognito"},
          {"id":"z-ai/glm-5","tier":"external"},
          {"id":"Qwen/Qwen3.8-27B","tier":"near_ai"},
          {"id":"z-ai/glm-5.3-flash","tier":"near_ai"},
          {"id":"x/future","tier":"some_future_tier"}
        ],"tier_order":["near_ai","external","incognito"],"default":"Qwen/Qwen3.8-27B",
        "background_sets":[
          ["anthropic/claude-haiku-5","anthropic/claude-sonnet-5-5","z-ai/glm-5","Qwen/Qwen3.8-27B","z-ai/glm-5.3-flash"],
          ["z-ai/glm-5","Qwen/Qwen3.8-27B","z-ai/glm-5.3-flash"],
          ["Qwen/Qwen3.8-27B","z-ai/glm-5.3-flash"],
          ["x/future"]
        ],
        "background":{
          "anthropic/claude-haiku-5":{"default":"anthropic/claude-haiku-5","set":0},
          "anthropic/claude-sonnet-5-5":{"default":"anthropic/claude-haiku-5","set":0},
          "z-ai/glm-5":{"default":"z-ai/glm-5","set":1},
          "Qwen/Qwen3.8-27B":{"default":"z-ai/glm-5.3-flash","set":2},
          "z-ai/glm-5.3-flash":{"default":"z-ai/glm-5.3-flash","set":2},
          "x/future":{"default":"x/future","set":3}
        }}
        """#.utf8))
    }

    func testTheCatalogueSaysEachModelsBackgroundChoices() throws {
        let decoded = try catalogue()
        let choices = try XCTUnwrap(ManagedSurface.backgroundChoices(decoded, main: "z-ai/glm-5"))
        XCTAssertEqual(choices.default, "z-ai/glm-5")
        XCTAssertEqual(choices.options, ["z-ai/glm-5", "Qwen/Qwen3.8-27B", "z-ai/glm-5.3-flash"])
        // An older daemon sends none, and an out-of-range set is none.
        let older = try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(#"{"models":[],"default":null}"#.utf8))
        XCTAssertEqual(older.background, [:])
        XCTAssertEqual(older.backgroundSets, [])
        let broken = try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(
            #"{"models":[{"id":"a/b"}],"default":"a/b","background_sets":[],"background":{"a/b":{"default":"a/b","set":4}}}"#.utf8))
        XCTAssertNil(ManagedSurface.backgroundChoices(broken, main: nil))
    }

    /// The options are the default row, naming the core's background
    /// default with its tier, then the ids the core allows, laid out as the
    /// main picker lays them out. The shell applies no rule of its own: a
    /// main model of a tier it cannot read gets what the core sent, only
    /// itself, and a main model the catalogue does not list gets only the
    /// default row (#1798 review).
    func testTheBackgroundPickerListsWhatTheCoreAllows() throws {
        let catalogue = try catalogue()
        let copy = { (key: String) in Self.copy[key] ?? "" }
        let privateMain = ManagedSurface.backgroundOptions(catalogue, main: nil, copy: copy)
        XCTAssertEqual(privateMain.map(\.value), ["", "Qwen/Qwen3.8-27B", "z-ai/glm-5.3-flash"])
        XCTAssertEqual(privateMain.first?.title, "Default: z-ai/glm-5.3-flash")
        XCTAssertEqual(privateMain.first?.privacy, .nearAIPrivate)
        XCTAssertEqual(privateMain.map(\.section), [nil, "NEAR AI private models", "NEAR AI private models"])
        let external = ManagedSurface.backgroundOptions(catalogue, main: "z-ai/glm-5", copy: copy)
        XCTAssertEqual(external.map(\.value), ["", "Qwen/Qwen3.8-27B", "z-ai/glm-5.3-flash", "z-ai/glm-5"])
        let incognito = ManagedSurface.backgroundOptions(catalogue, main: "anthropic/claude-sonnet-5-5", copy: copy)
        XCTAssertEqual(incognito.map(\.value), [
            "", "Qwen/Qwen3.8-27B", "z-ai/glm-5.3-flash", "z-ai/glm-5",
            "anthropic/claude-haiku-5", "anthropic/claude-sonnet-5-5",
        ])
        XCTAssertEqual(incognito.first?.title, "Default: anthropic/claude-haiku-5")
        XCTAssertEqual(ManagedSurface.backgroundOptions(catalogue, main: "x/future", copy: copy).map(\.value), ["", "x/future"])
        let unlisted = ManagedSurface.backgroundOptions(catalogue, main: "gone/model", copy: copy)
        XCTAssertEqual(unlisted.map(\.value), [""])
        XCTAssertEqual(unlisted.map(\.title), ["Default"])
        XCTAssertEqual(ManagedSurface.backgroundOptions(nil, main: nil, copy: copy).map(\.title), ["Default"])
    }

    /// The model weighed is the one the launcher will run: the model the
    /// session's CLI options name when the catalogue lists it, else the
    /// picked one, else the default (#1798 review). An answer for options
    /// the draft no longer holds is ignored.
    func testTheBackgroundChoiceFollowsTheModelTheCLIOptionsName() throws {
        let catalogue = try catalogue()
        XCTAssertEqual(ManagedSurface.backgroundMain(picked: "anthropic/claude-sonnet-5-5", cliModel: "z-ai/glm-5",
                                                     in: catalogue), "z-ai/glm-5")
        XCTAssertEqual(ManagedSurface.backgroundMain(picked: "anthropic/claude-sonnet-5-5", cliModel: "gone/model",
                                                     in: catalogue), "anthropic/claude-sonnet-5-5")
        XCTAssertEqual(ManagedSurface.backgroundMain(picked: nil, cliModel: nil, in: catalogue), "Qwen/Qwen3.8-27B")
        var draft = ManagedFolderDraft()
        draft.useDefaults(tool: "claude", cwd: "/project", cliOptions: "--model z-ai/glm-5")
        draft.choose("anthropic/claude-sonnet-5-5", in: catalogue)
        draft.chooseBackground("anthropic/claude-haiku-5")
        XCTAssertEqual(draft.backgroundModel, "anthropic/claude-haiku-5")
        // An answer for other options changes nothing.
        draft.setCLIModel("z-ai/glm-5", for: "--verbose", in: catalogue)
        XCTAssertNil(draft.cliModel)
        XCTAssertEqual(draft.backgroundModel, "anthropic/claude-haiku-5")
        draft.setCLIModel("z-ai/glm-5", for: "--model z-ai/glm-5", in: catalogue)
        XCTAssertEqual(draft.cliModel, "z-ai/glm-5")
        XCTAssertNil(draft.backgroundModel)
        // Editing the options forgets the answer until the next one.
        draft.cliOptions = "--verbose"
        XCTAssertNil(draft.cliModel)
    }

    /// Choosing a more private main model clears a background choice it no
    /// longer allows, and so does a fallback to the default (#1798
    /// review); one it allows is kept, and without a catalogue, or for a
    /// main model the catalogue does not list, nothing is removed.
    func testAMorePrivateMainModelClearsALessPrivateBackgroundChoice() throws {
        let catalogue = try catalogue()
        var draft = ManagedFolderDraft()
        draft.useDefaults(tool: "claude", cwd: "/project", cliOptions: "")
        draft.choose("anthropic/claude-sonnet-5-5", in: catalogue)
        draft.chooseBackground("anthropic/claude-haiku-5")
        draft.choose("z-ai/glm-5", in: catalogue)
        XCTAssertNil(draft.backgroundModel)
        draft.chooseBackground("Qwen/Qwen3.8-27B")
        draft.choose("Qwen/Qwen3.8-27B", in: catalogue)
        XCTAssertEqual(draft.backgroundModel, "Qwen/Qwen3.8-27B")
        // A saved main model NEAR AI dropped falls back to the default, which
        // here is NEAR AI private: an External background choice goes.
        draft.choose("gone/model", in: nil)
        draft.chooseBackground("z-ai/glm-5")
        draft.reconcileBackground(in: catalogue)
        XCTAssertEqual(draft.backgroundModel, "z-ai/glm-5", "unknown main: not removed")
        draft.fallBackToDefault(in: catalogue)
        XCTAssertNil(draft.model)
        XCTAssertTrue(draft.keepsSavedModel)
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
