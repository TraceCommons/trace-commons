import XCTest
@testable import TCShellCore

final class ManagedSessionsTests: XCTestCase {
    func testUnknownLifecycleKeepsAccountLocked() throws {
        let row = try JSONDecoder().decode(ManagedSession.self, from: Data(#"{"id":"s","tool":"claude","connection":"subscription","account_id":"a","account_label":"Personal","project_label":"Project","cwd":"/tmp","state":"future_state","purpose":"coding"}"#.utf8))
        XCTAssertTrue(row.holdsAccount)
        XCTAssertEqual(row.accountLabel, "Personal")
    }
    func testSnapshotCarriesIndependentDefaultVersionsAndTerminalDestination() throws {
        let snapshot = try JSONDecoder().decode(ManagedSnapshot.self, from: Data(#"{"revision":2,"accounts":[],"defaults":[],"generations":{"claude":3,"codex":1},"sessions":[],"capabilities":{"managed_launch":true,"terminal_launch":true,"terminal_destination":"Terminal"}}"#.utf8))
        XCTAssertEqual(snapshot.generations["claude"], 3)
        XCTAssertEqual(snapshot.capabilities.terminalDestination, "Terminal")
    }

    func testSnapshotCarriesTheTerminalChoiceAndTheInstalledApps() throws {
        let snapshot = try JSONDecoder().decode(ManagedSnapshot.self, from: Data(#"{"revision":2,"accounts":[],"defaults":[],"generations":{},"sessions":[],"capabilities":{"managed_launch":true,"terminal_launch":true,"terminal_destination":"Ghostty","terminal_current":"ghostty","terminal_choices":[{"id":"terminal","name":"Terminal"},{"id":"ghostty","name":"Ghostty"}]}}"#.utf8))
        XCTAssertEqual(snapshot.capabilities.terminalDestination, "Ghostty")
        XCTAssertEqual(snapshot.capabilities.terminalCurrent, "ghostty")
        XCTAssertEqual(snapshot.terminalChoices, [ManagedTerminalChoice(id: "terminal", name: "Terminal"), ManagedTerminalChoice(id: "ghostty", name: "Ghostty")])
    }

    /// The launch dialog asks for a terminal until one is chosen, only with
    /// more than one installed, and never for an older daemon that does not
    /// say; it starts on the current app when installed, else the first.
    func testTheLaunchDialogAsksForATerminalUntilOneIsChosen() throws {
        func snapshot(chosen: String, current: String, choices: [String]) throws -> ManagedSnapshot {
            let list = choices.map { #"{"id":"\#($0)","name":"\#($0)"}"# }.joined(separator: ",")
            let json = #"{"revision":2,"accounts":[],"defaults":[],"generations":{},"sessions":[],"capabilities":{"managed_launch":true,"terminal_launch":true,"terminal_current":\#(current),\#(chosen)"terminal_choices":[\#(list)]}}"#
            return try JSONDecoder().decode(ManagedSnapshot.self, from: Data(json.utf8))
        }
        let asking = try snapshot(chosen: #""terminal_chosen":false,"#, current: #""ghostty""#, choices: ["terminal", "ghostty"])
        XCTAssertTrue(asking.asksForTerminal)
        XCTAssertEqual(asking.proposedTerminal, "ghostty")
        XCTAssertFalse(try snapshot(chosen: #""terminal_chosen":true,"#, current: #""ghostty""#, choices: ["terminal", "ghostty"]).asksForTerminal)
        XCTAssertFalse(try snapshot(chosen: "", current: #""ghostty""#, choices: ["terminal", "ghostty"]).asksForTerminal)
        XCTAssertFalse(try snapshot(chosen: #""terminal_chosen":false,"#, current: #""terminal""#, choices: ["terminal"]).asksForTerminal)
        let missing = try snapshot(chosen: #""terminal_chosen":false,"#, current: "null", choices: ["kitty", "wezterm"])
        XCTAssertTrue(missing.asksForTerminal)
        XCTAssertEqual(missing.proposedTerminal, "kitty")
    }

    func testAnOlderSnapshotOffersNoTerminalChoice() throws {
        let snapshot = try JSONDecoder().decode(ManagedSnapshot.self, from: Data(#"{"revision":1,"accounts":[],"defaults":[],"generations":{},"sessions":[],"capabilities":{"managed_launch":true,"terminal_launch":true,"terminal_destination":"Terminal"}}"#.utf8))
        XCTAssertNil(snapshot.capabilities.terminalCurrent)
        XCTAssertEqual(snapshot.terminalChoices, [])
    }

    func testTheFixtureCarriesTheTerminalFields() throws {
        let repository = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let snapshot = try JSONDecoder().decode(ManagedSnapshot.self, from: Data(contentsOf: repository.appendingPathComponent("tests/fixtures/managed/snapshot.json")))
        XCTAssertEqual(snapshot.capabilities.terminalCurrent, "terminal")
        XCTAssertEqual(snapshot.terminalChoices.map(\.name), ["Terminal", "Ghostty"])
        XCTAssertEqual(snapshot.copy?["terminal_choice"], "Open sessions in")
    }

    func testAMissingTerminalHasItsOwnLine() {
        XCTAssertEqual(ManagedSurface.errorKey(code: "managed-terminal-not-found", message: "managed-terminal-not-found"), "terminal_not_found")
        let line = "{terminal} wasn't found on this computer. Choose another in Settings."
        XCTAssertEqual(ManagedSurface.fillTerminal(line, terminal: "Ghostty", fallback: "your terminal"),
                       "Ghostty wasn't found on this computer. Choose another in Settings.")
        XCTAssertEqual(ManagedSurface.fillTerminal(line, terminal: nil, fallback: "your terminal"),
                       "your terminal wasn't found on this computer. Choose another in Settings.")
    }

    func testOpenCodeCollectionAndModelDecode() throws {
        let session = try JSONDecoder().decode(ManagedSession.self, from: Data(#"{"id":"s","tool":"opencode","connection":"api_key","account_id":"a","account_label":"Work","project_label":"Project","cwd":"/tmp","state":"exited","purpose":"coding","model":"gpt-5","collection":"failed"}"#.utf8))
        XCTAssertEqual(session.model, "gpt-5")
        XCTAssertEqual(session.collection, "failed")
        XCTAssertEqual(ManagedSurface.toolKey("opencode"), "opencode")
    }

    func testOpenCodeKeepsKeyConnectionsAndStartsOnTheSignIn() {
        XCTAssertEqual(ManagedSurface.connectionOptions(signedIn: true, tool: "opencode"), ["api_key", "near_ai"])
        XCTAssertEqual(ManagedSurface.connectionOptions(signedIn: false, tool: "opencode"), ["api_key", "near_ai"])
        XCTAssertEqual(ManagedSurface.signInTools, ["claude", "codex", "opencode"])
        XCTAssertEqual(ManagedSurface.startKey("opencode"), "start_opencode")
    }

    /// A tools row as the daemon sends it.
    private static func toolRow(_ tool: String, installed: Bool) -> String {
        #"{"tool":""# + tool + #"","installed":"# + (installed ? "true" : "false") + #","install_url":"https://example.com/"# + tool + #""}"#
    }

    /// The sign-in snapshot with an OpenCode entry and the given `tools`
    /// rows (no `tools` key at all when nil).
    private static func signedInWithOpenCode(tools: [String]?, signedIn: Bool = true) -> String {
        let entries = #""sign_in_accounts":[{"id":"c","tool":"claude","connection":"near_ai_signin","label":"NEAR AI","auth_state":"ready"},{"id":"x","tool":"codex","connection":"near_ai_signin","label":"NEAR AI","auth_state":"ready"},{"id":"o","tool":"opencode","connection":"near_ai_signin","label":"NEAR AI","auth_state":"ready"}]"#
        let toolsField = tools.map { #","tools":["# + $0.joined(separator: ",") + "]" } ?? ""
        return #"{"revision":1,"accounts":[],"# + entries + toolsField + #","defaults":[],"generations":{},"sessions":[],"capabilities":{"managed_launch":true,"near_ai_signin":"# + (signedIn ? "true" : "false") + #","terminal_launch":true}}"#
    }

    /// Start OpenCode shows only while the daemon reports OpenCode
    /// installed. No `tools` list (an older daemon), no opencode row, or a
    /// row that says not installed shows no button and builds no launch;
    /// Claude Code and Codex keep their older-daemon behaviour.
    func testStartOpenCodeShowsOnlyWhenOpenCodeIsInstalled() throws {
        func snapshot(_ tools: [String]?, signedIn: Bool = true) throws -> ManagedSnapshot {
            try JSONDecoder().decode(ManagedSnapshot.self, from: Data(Self.signedInWithOpenCode(tools: tools, signedIn: signedIn).utf8))
        }
        let unknown = try snapshot(nil)
        XCTAssertEqual(unknown.signInStartTools, ["claude", "codex"])
        XCTAssertNil(unknown.signInLaunchParams(tool: "opencode", cwd: "/tmp", requestID: "r"))

        let noRow = try snapshot([Self.toolRow("claude", installed: true), Self.toolRow("codex", installed: true)])
        XCTAssertEqual(noRow.signInStartTools, ["claude", "codex"])
        XCTAssertNil(noRow.signInLaunchParams(tool: "opencode", cwd: "/tmp", requestID: "r"))

        let missing = try snapshot([Self.toolRow("claude", installed: true), Self.toolRow("codex", installed: true), Self.toolRow("opencode", installed: false)])
        XCTAssertEqual(missing.signInStartTools, ["claude", "codex"])
        XCTAssertNil(missing.signInLaunchParams(tool: "opencode", cwd: "/tmp", requestID: "r"))

        let installed = try snapshot([Self.toolRow("claude", installed: false), Self.toolRow("codex", installed: true), Self.toolRow("opencode", installed: true)])
        XCTAssertEqual(installed.signInStartTools, ["codex", "opencode"])
        let params = try XCTUnwrap(installed.signInLaunchParams(tool: "opencode", cwd: "/tmp/p", requestID: "r"))
        XCTAssertEqual(params["tool"] as? String, "opencode")
        XCTAssertEqual(params["connection"] as? String, "near_ai_signin")
        XCTAssertEqual(params["account_id"] as? String, "o")
        // The session's proxy picks the model: the request names none.
        XCTAssertNil(params["model"])

        // Signed out, nothing to start, installed or not.
        XCTAssertEqual(try snapshot([Self.toolRow("opencode", installed: true)], signedIn: false).signInStartTools, [])
    }

    func testSurfaceNeverPicksARawValueOrDaemonCode() {
        XCTAssertEqual(ManagedSurface.stateKey("running"), "state_running")
        XCTAssertEqual(ManagedSurface.stateKey("future_state"), "state_unknown")
        XCTAssertEqual(ManagedSurface.authKey("sign_in_required"), "auth_sign_in_required")
        XCTAssertNil(ManagedSurface.authKey("future_auth"))
        XCTAssertNil(ManagedSurface.toolKey("future_tool"))
        XCTAssertEqual(ManagedSurface.errorKey(code: "launch-unknown", message: "managed-launch-unknown"), "launch_unknown")
        XCTAssertEqual(ManagedSurface.errorKey(code: "conflict", message: "raw daemon text"), "action_failed")
        XCTAssertEqual(ManagedSurface.errorKey(code: nil, message: nil), "action_failed")
        // tool_unavailable needs a {tool}; the banner has none to give it.
        XCTAssertEqual(ManagedSurface.errorKey(code: "managed-tool-unavailable", message: "managed-tool-unavailable"), "action_failed")
    }

    func testSnapshotKnowsWhichAccountsAreHeldAndDefault() throws {
        let snapshot = try JSONDecoder().decode(ManagedSnapshot.self, from: Data(#"{"revision":1,"accounts":[{"id":"a","tool":"claude","connection":"subscription","label":"Personal","auth_state":"ready"},{"id":"b","tool":"claude","connection":"api_key","label":"Work","auth_state":"ready"}],"defaults":[{"tool":"claude","connection":"subscription","account_id":"a","generation":1}],"generations":{"claude":1},"sessions":[{"id":"s","tool":"claude","connection":"subscription","account_id":"a","account_label":"Personal","project_label":"Project","cwd":"/tmp","state":"running","purpose":"coding"},{"id":"t","tool":"claude","connection":"api_key","account_id":"b","account_label":"Work","project_label":"Project","cwd":"/tmp","state":"exited","purpose":"coding"}],"capabilities":{"managed_launch":true,"terminal_launch":false}}"#.utf8))
        XCTAssertTrue(snapshot.isHeld(snapshot.accounts[0]))
        XCTAssertFalse(snapshot.isHeld(snapshot.accounts[1]))
        XCTAssertTrue(snapshot.isDefault(snapshot.accounts[0]))
        XCTAssertFalse(snapshot.isDefault(snapshot.accounts[1]))
    }

    private static let signedIn = #"{"revision":1,"accounts":[],"sign_in_accounts":[{"id":"c","tool":"claude","connection":"near_ai_signin","label":"NEAR AI","auth_state":"ready"},{"id":"x","tool":"codex","connection":"near_ai_signin","label":"NEAR AI","auth_state":"ready"}],"defaults":[{"tool":"claude","connection":"near_ai_signin","account_id":"c","generation":4},{"tool":"codex","connection":"near_ai_signin","account_id":"x","generation":0}],"generations":{"claude":4},"sessions":[],"capabilities":{"managed_launch":true,"near_ai_signin":true,"terminal_launch":true}}"#

    func testAnOlderSnapshotReadsAsSignedOut() throws {
        let snapshot = try JSONDecoder().decode(ManagedSnapshot.self, from: Data(#"{"revision":1,"accounts":[],"defaults":[],"generations":{},"sessions":[],"capabilities":{"managed_launch":true,"terminal_launch":true}}"#.utf8))
        XCTAssertFalse(snapshot.nearAiSignedIn)
        XCTAssertNil(snapshot.signInEntry("claude"))
        XCTAssertNil(snapshot.signInLaunchParams(tool: "claude", cwd: "/tmp", requestID: "r"))
    }

    /// Owner, 2026-10-29: no folder is needed. The request names none, so the
    /// daemon starts the session in the person's home folder; a shell never
    /// guesses a path.
    func testALaunchWithNoFolderNamesNoCwd() throws {
        let snapshot = try JSONDecoder().decode(ManagedSnapshot.self, from: Data(Self.signedIn.utf8))
        let params = try XCTUnwrap(snapshot.signInLaunchParams(tool: "claude", cwd: nil, requestID: "r"))
        XCTAssertNil(params["cwd"])
        XCTAssertEqual(params["account_id"] as? String, "c")
        XCTAssertEqual(params["expected_generation"] as? UInt64, 4)
    }

    func testTheHomeDraftIsReadyWithoutAFolder() {
        var draft = ManagedFolderDraft()
        draft.useDefaults(tool: "claude", cwd: nil, cliOptions: "")
        XCTAssertTrue(draft.matches(tool: "claude", cwd: nil))
        XCTAssertFalse(draft.matches(tool: "claude", cwd: "/tmp"))
        draft.useDefaults(tool: "claude", cwd: "/tmp", cliOptions: "")
        XCTAssertFalse(draft.matches(tool: "claude", cwd: nil))
    }

    func testTheSignInEntriesLaunchWithNoSavedAccount() throws {
        let snapshot = try JSONDecoder().decode(ManagedSnapshot.self, from: Data(Self.signedIn.utf8))
        XCTAssertTrue(snapshot.nearAiSignedIn)
        XCTAssertEqual(snapshot.signInEntry("codex")?.id, "x")
        let params = try XCTUnwrap(snapshot.signInLaunchParams(tool: "claude", cwd: "/tmp/p", requestID: "r"))
        XCTAssertEqual(params["connection"] as? String, "near_ai_signin")
        XCTAssertEqual(params["account_id"] as? String, "c")
        XCTAssertEqual(params["tool"] as? String, "claude")
        XCTAssertEqual(params["cwd"] as? String, "/tmp/p")
        XCTAssertEqual(params["request_id"] as? String, "r")
        XCTAssertEqual(params["purpose"] as? String, "coding")
        XCTAssertEqual(params["expected_generation"] as? UInt64, 4)
        XCTAssertEqual(params["save_default"] as? Bool, false)
        XCTAssertNil(snapshot.signInLaunchParams(tool: "future_tool", cwd: "/tmp", requestID: "r"))
    }

    func testEntriesAreIgnoredWithoutTheSignInCapability() throws {
        let stale = Self.signedIn.replacingOccurrences(of: #""near_ai_signin":true"#, with: #""near_ai_signin":false"#)
        let snapshot = try JSONDecoder().decode(ManagedSnapshot.self, from: Data(stale.utf8))
        XCTAssertFalse(snapshot.nearAiSignedIn)
        XCTAssertNil(snapshot.signInLaunchParams(tool: "claude", cwd: "/tmp", requestID: "r"))
    }

    func testTheSignInSurfacePicksCoreKeys() {
        let line = "Uses your NEAR AI sign-in."
        XCTAssertEqual(ManagedSurface.signInScope(line, destination: "Terminal", fallback: "your terminal"),
                       "Uses your NEAR AI sign-in.")
        XCTAssertEqual(ManagedSurface.signInScope(line, destination: nil, fallback: "your terminal"),
                       "Uses your NEAR AI sign-in.")
        XCTAssertEqual(ManagedSurface.startKey("claude"), "start_claude")
        XCTAssertEqual(ManagedSurface.startKey("codex"), "start_codex")
        XCTAssertEqual(ManagedSurface.startKey("opencode"), "start_opencode")
        XCTAssertNil(ManagedSurface.startKey("future_tool"))
        XCTAssertEqual(ManagedSurface.errorKey(code: "managed-near-ai-sign-in-required", message: "managed-near-ai-sign-in-required"), "sign_in_needed")
        // Signed in, the sheet does not offer a NEAR AI key: the sign-in
        // entries already run on it. Signed out, the key stays the fallback.
        XCTAssertEqual(ManagedSurface.connectionOptions(signedIn: true), ["subscription", "api_key"])
        XCTAssertEqual(ManagedSurface.connectionOptions(signedIn: false), ["subscription", "api_key", "near_ai"])
    }

    /// #1408: the folder explainer shows until a session has started on
    /// this computer, which the daemon reports as `has_launched`. An older
    /// daemon that does not say reads as never launched.
    func testTheFolderExplainerShowsUntilTheFirstLaunch() throws {
        let never = try JSONDecoder().decode(ManagedSnapshot.self, from: Data(Self.signedIn.utf8))
        XCTAssertNil(never.hasLaunched)
        XCTAssertTrue(never.explainsFolderChoice)
        let notYet = Self.signedIn.replacingOccurrences(of: #""revision":1,"#, with: #""revision":1,"has_launched":false,"#)
        XCTAssertTrue(try JSONDecoder().decode(ManagedSnapshot.self, from: Data(notYet.utf8)).explainsFolderChoice)
        let launched = Self.signedIn.replacingOccurrences(of: #""revision":1,"#, with: #""revision":1,"has_launched":true,"#)
        XCTAssertNotEqual(launched, Self.signedIn)
        let snapshot = try JSONDecoder().decode(ManagedSnapshot.self, from: Data(launched.utf8))
        XCTAssertEqual(snapshot.hasLaunched, true)
        XCTAssertFalse(snapshot.explainsFolderChoice)
    }

    // MARK: #1407 row actions

    private func session(_ state: String, focus: Bool = false, stop: Bool = false, again: Bool = false, startedAt: String? = nil) throws -> ManagedSession {
        var json = #"{"id":"s","tool":"claude","connection":"subscription","account_id":"a","account_label":"Work","project_label":"Project","cwd":"/tmp","state":"\#(state)","purpose":"coding","can_focus":\#(focus),"can_stop":\#(stop),"can_start_again":\#(again)"#
        if let startedAt { json += #","started_at":"\#(startedAt)""# }
        return try JSONDecoder().decode(ManagedSession.self, from: Data((json + "}").utf8))
    }

    func testRowFlagsAndStartTimeDecode() throws {
        let row = try session("running", focus: true, stop: true, startedAt: "2026-10-14T09:30:00.123456789Z")
        XCTAssertTrue(row.canFocus)
        XCTAssertTrue(row.canStop)
        XCTAssertFalse(row.canStartAgain)
        // chrono's nanoseconds, read to the millisecond.
        let second = try XCTUnwrap(ISO8601DateFormatter().date(from: "2026-10-14T09:30:00Z"))
        XCTAssertEqual(try XCTUnwrap(row.startedAt).timeIntervalSince(second), 0.123, accuracy: 0.0005)
    }

    func testAnOlderDaemonsRowOffersNoActionsAndNoTime() throws {
        let row = try JSONDecoder().decode(ManagedSession.self, from: Data(#"{"id":"s","tool":"claude","connection":"subscription","account_id":"a","account_label":"Work","project_label":"Project","cwd":"/tmp","state":"running","purpose":"coding","started_at":"not a time"}"#.utf8))
        XCTAssertFalse(row.canFocus)
        XCTAssertFalse(row.canStop)
        XCTAssertFalse(row.canStartAgain)
        XCTAssertNil(row.startedAt)
        XCTAssertEqual(ManagedSurface.sessionActions(row, terminalLaunch: true), [])
        XCTAssertFalse(ManagedSurface.clickShows(row))
    }

    /// Ron, #1407: Show and Stop only where the daemon can do them, hidden
    /// (not disabled) otherwise; an ended row offers Start again, then
    /// Dismiss.
    func testRowActionsFollowWhatTheDaemonCanDo() throws {
        XCTAssertEqual(ManagedSurface.sessionActions(try session("running", focus: true, stop: true), terminalLaunch: true), [.show, .stop])
        XCTAssertEqual(ManagedSurface.sessionActions(try session("running", stop: true), terminalLaunch: true), [.stop])
        XCTAssertEqual(ManagedSurface.sessionActions(try session("running", focus: true), terminalLaunch: true), [.show])
        XCTAssertEqual(ManagedSurface.sessionActions(try session("running"), terminalLaunch: true), [])
        // A flag on a row that is not running offers nothing.
        XCTAssertEqual(ManagedSurface.sessionActions(try session("starting", focus: true, stop: true), terminalLaunch: true), [])
        XCTAssertEqual(ManagedSurface.sessionActions(try session("unknown", focus: true, stop: true), terminalLaunch: true), [])
        XCTAssertEqual(ManagedSurface.sessionActions(try session("exited", again: true), terminalLaunch: true), [.startAgain, .dismiss])
        XCTAssertEqual(ManagedSurface.sessionActions(try session("failed", again: true), terminalLaunch: true), [.startAgain, .dismiss])
        XCTAssertEqual(ManagedSurface.sessionActions(try session("exited", again: true), terminalLaunch: false), [.dismiss])
        XCTAssertEqual(ManagedSurface.sessionActions(try session("exited"), terminalLaunch: true), [.dismiss])
        XCTAssertEqual(ManagedSurface.menuActions(try session("running", focus: true, stop: true)), [.show, .stop])
        XCTAssertEqual(ManagedSurface.menuActions(try session("exited", again: true)), [])
    }

    func testOnlyAShowableRunningRowShowsOnClick() throws {
        XCTAssertTrue(ManagedSurface.clickShows(try session("running", focus: true)))
        XCTAssertFalse(ManagedSurface.clickShows(try session("running", stop: true)))
        XCTAssertFalse(ManagedSurface.clickShows(try session("exited", focus: true)))
    }

    func testStopLineNamesTheToolAndFolder() {
        XCTAssertEqual(
            ManagedSurface.stopDescription("{tool} stops what it's doing in {folder}.", tool: "Claude Code", folder: "Project"),
            "Claude Code stops what it's doing in Project.")
    }

    func testStartAgainNamesOnlyTheRowAndARequest() {
        let params = ManagedSurface.startAgainParams(sessionID: "s", requestID: "r")
        XCTAssertEqual(params.count, 2)
        XCTAssertEqual(params["session_id"] as? String, "s")
        XCTAssertEqual(params["request_id"] as? String, "r")
    }

    /// The start time ends the caption line, in the app's relative format.
    func testCaptionEndsWithTheStartTime() throws {
        let started = Date(timeIntervalSince1970: 0)
        XCTAssertEqual(
            ManagedSurface.caption(["Claude Code", "Work", "Subscription"], startedAt: started, relative: { _ in "12 minutes ago" }),
            "Claude Code · Work · Subscription · 12 minutes ago")
        XCTAssertEqual(
            ManagedSurface.caption(["Claude Code", "", "Subscription"], startedAt: nil, relative: { _ in "never" }),
            "Claude Code · Subscription")
    }

    private static let modelCopy = [
        "model_default": "Default: {model}", "model_default_unknown": "Default",
        "model_group_incognito": "Incognito models", "model_group_near_ai": "NEAR AI private models",
        "model_group_external": "External private models",
    ]

    /// Owner, 2026-10-30: the picker groups models by NEAR AI's privacy
    /// tier, Incognito first, then NEAR AI's TEEs, then a third party's,
    /// each in NEAR AI's order. The default heads no group; a model with
    /// no tier (an older daemon) comes last, under no heading.
    func testTheModelPickerGroupsModelsByPrivacyTier() throws {
        let catalogue = try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(#"""
        {"models":[
          {"id":"z-ai/glm-5.3","tier":"external"},
          {"id":"anthropic/claude-sonnet-5-5","tier":"incognito"},
          {"id":"Qwen/Qwen3.8-27B","tier":"near_ai"},
          {"id":"x/untiered"},
          {"id":"openai/gpt-6-sol","tier":"incognito"},
          {"id":"x/future","tier":"some_future_tier"}
        ],"default":"anthropic/claude-sonnet-5-5"}
        """#.utf8))
        let options = ManagedSurface.modelOptions(catalogue) { Self.modelCopy[$0] ?? "" }
        XCTAssertEqual(options.map(\.value), [
            ManagedSurface.defaultModel,
            "anthropic/claude-sonnet-5-5", "openai/gpt-6-sol",
            "Qwen/Qwen3.8-27B",
            "z-ai/glm-5.3",
            "x/untiered", "x/future",
        ])
        XCTAssertEqual(options.map(\.section), [
            nil,
            "Incognito models", "Incognito models",
            "NEAR AI private models",
            "External private models",
            nil, nil,
        ])
        // Favorites come first, in NEAR AI's order (as the daemon answers
        // them), under no heading, and leave their groups.
        let starred = try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(#"""
        {"models":[
          {"id":"z-ai/glm-5.3","tier":"external"},
          {"id":"anthropic/claude-sonnet-5-5","tier":"incognito"},
          {"id":"Qwen/Qwen3.8-27B","tier":"near_ai"},
          {"id":"x/untiered"},
          {"id":"openai/gpt-6-sol","tier":"incognito"},
          {"id":"x/future","tier":"some_future_tier"}
        ],"default":"anthropic/claude-sonnet-5-5","favorites":["z-ai/glm-5.3","openai/gpt-6-sol"]}
        """#.utf8))
        let favored = ManagedSurface.modelOptions(starred) { Self.modelCopy[$0] ?? "" }
        XCTAssertEqual(favored.map(\.value), [
            ManagedSurface.defaultModel,
            "z-ai/glm-5.3", "openai/gpt-6-sol",
            "anthropic/claude-sonnet-5-5",
            "Qwen/Qwen3.8-27B",
            "x/untiered", "x/future",
        ])
        XCTAssertEqual(favored.map(\.section), [
            nil, nil, nil,
            "Incognito models",
            "NEAR AI private models",
            nil, nil,
        ])
        // No tiers at all: no headings.
        let flat = try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(#"{"models":[{"id":"x/y"}],"default":null}"#.utf8))
        XCTAssertEqual(ManagedSurface.modelOptions(flat) { Self.modelCopy[$0] ?? "" }.map(\.section), [nil, nil])
    }

    /// The picker's first option names no model and says which one the
    /// session will get; every NEAR AI chat model follows, in its order.
    func testTheModelPickerOffersTheDefaultThenTheCatalogue() throws {
        let catalogue = try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(#"{"models":[{"id":"anthropic/claude-sonnet-5-5","name":"Claude Sonnet 5.5"},{"id":"openai/gpt-6.1-sol"}],"default":"anthropic/claude-sonnet-5-5"}"#.utf8))
        XCTAssertEqual(catalogue.models.map(\.id), ["anthropic/claude-sonnet-5-5", "openai/gpt-6.1-sol"])
        XCTAssertEqual(catalogue.models.first?.name, "Claude Sonnet 5.5")
        let options = ManagedSurface.modelOptions(catalogue) { Self.modelCopy[$0] ?? "" }
        XCTAssertEqual(options.map(\.title), ["Default: anthropic/claude-sonnet-5-5", "anthropic/claude-sonnet-5-5", "openai/gpt-6.1-sol"])
        XCTAssertEqual(options.map(\.value), [ManagedSurface.defaultModel, "anthropic/claude-sonnet-5-5", "openai/gpt-6.1-sol"])

        // Before the catalogue arrives, or when NEAR AI could not be read:
        // only the default, which needs no catalogue to launch.
        XCTAssertEqual(ManagedSurface.modelOptions(nil) { Self.modelCopy[$0] ?? "" }.map(\.title), ["Default"])
        let unnamed = try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(#"{"models":[{"id":"x/y"}],"default":null}"#.utf8))
        XCTAssertEqual(ManagedSurface.modelOptions(unnamed) { Self.modelCopy[$0] ?? "" }.map(\.title), ["Default", "x/y"])
    }

    /// Favorites come straight after the default, in NEAR AI's order rather
    /// than the order they were starred, and are not repeated below. A
    /// favorite NEAR AI no longer offers is not shown. An older daemon sends
    /// no `favorites`, which reads as none.
    func testFavoriteModelsComeFirstInTheModelPicker() throws {
        let catalogue = try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(#"{"models":[{"id":"a/one"},{"id":"b/two"},{"id":"c/three"},{"id":"d/four"}],"default":"a/one","favorites":["d/four","b/two","gone/model"]}"#.utf8))
        XCTAssertEqual(catalogue.favorites, ["d/four", "b/two", "gone/model"])
        let options = ManagedSurface.modelOptions(catalogue) { Self.modelCopy[$0] ?? "" }
        XCTAssertEqual(options.map(\.value), [ManagedSurface.defaultModel, "b/two", "d/four", "a/one", "c/three"])
        XCTAssertTrue(ManagedSurface.isFavoriteModel("b/two", in: catalogue))
        XCTAssertFalse(ManagedSurface.isFavoriteModel("a/one", in: catalogue))

        let older = try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(#"{"models":[{"id":"a/one"},{"id":"b/two"}],"default":null}"#.utf8))
        XCTAssertEqual(older.favorites, [])
        XCTAssertEqual(ManagedSurface.modelOptions(older) { Self.modelCopy[$0] ?? "" }.map(\.value), [ManagedSurface.defaultModel, "a/one", "b/two"])
    }

    /// The star is offered only for a model NEAR AI lists: never for the
    /// default option, and never for a saved model it no longer offers.
    func testTheStarIsOfferedOnlyForAListedModel() throws {
        let catalogue = try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(#"{"models":[{"id":"a/one"}],"default":"a/one"}"#.utf8))
        XCTAssertTrue(ManagedSurface.offersFavorite("a/one", in: catalogue))
        XCTAssertFalse(ManagedSurface.offersFavorite(nil, in: catalogue))
        XCTAssertFalse(ManagedSurface.offersFavorite(ManagedSurface.defaultModel, in: catalogue))
        XCTAssertFalse(ManagedSurface.offersFavorite("gone/model", in: catalogue))
        XCTAssertFalse(ManagedSurface.offersFavorite("a/one", in: nil))
    }

    /// A favorite NEAR AI no longer lists is reported apart, for its own
    /// row with a way to remove it. An older daemon sends none, and no
    /// catalogue (offline, or the read failed) marks nothing: unknown is
    /// not removed. After a star or removal the shell re-reads the
    /// catalogue rather than recomputing these lists (#1754 review).
    func testUnavailableFavoritesAreListedApartAndNeverGuessed() throws {
        let catalogue = try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(#"{"models":[{"id":"a/one"},{"id":"b/two"}],"default":"a/one","favorites":["b/two"],"unavailable_favorites":["gone/model","gone/other"]}"#.utf8))
        XCTAssertEqual(ManagedSurface.unavailableFavorites(catalogue), ["gone/model", "gone/other"])
        // Never in the picker.
        XCTAssertFalse(ManagedSurface.modelOptions(catalogue) { Self.modelCopy[$0] ?? "" }.map(\.value).contains("gone/model"))

        let older = try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(#"{"models":[{"id":"a/one"}],"default":null}"#.utf8))
        XCTAssertEqual(older.unavailableFavorites, [])
        XCTAssertEqual(ManagedSurface.unavailableFavorites(nil), [])
    }

    /// Settings' favorites list: every stored favorite, listed ones in
    /// NEAR AI's order, then the ones it stopped listing, marked. With no
    /// catalogue every one is shown as stored and none is marked.
    func testTheFavoritesListMarksOnlyWhatTheCatalogueDropped() throws {
        let catalogue = try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(#"{"models":[{"id":"a/one"},{"id":"b/two"}],"default":"a/one","favorites":["a/one","b/two"],"unavailable_favorites":["gone/model"]}"#.utf8))
        let stored = ["gone/model", "b/two", "a/one"]
        XCTAssertEqual(ManagedSurface.favoriteRows(stored: stored, catalogue: catalogue), [
            ManagedFavoriteRow(model: "a/one", unavailable: false),
            ManagedFavoriteRow(model: "b/two", unavailable: false),
            ManagedFavoriteRow(model: "gone/model", unavailable: true),
        ])
        XCTAssertEqual(ManagedSurface.favoriteRows(stored: stored, catalogue: nil), [
            ManagedFavoriteRow(model: "gone/model", unavailable: false),
            ManagedFavoriteRow(model: "b/two", unavailable: false),
            ManagedFavoriteRow(model: "a/one", unavailable: false),
        ])
        XCTAssertEqual(ManagedSurface.favoriteRows(stored: [], catalogue: catalogue), [])
        // The rows come from the stored list (`get_settings`), never from
        // the catalogue's two lists (#1754 review): a favorite NEAR AI
        // lists but the read tool is not offered is in neither list, and
        // still has its row, after the ones the catalogue orders.
        let partial = try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(#"{"models":[{"id":"a/one"}],"default":"a/one","favorites":["a/one"],"unavailable_favorites":[]}"#.utf8))
        XCTAssertEqual(ManagedSurface.favoriteRows(stored: ["not/offered", "a/one"], catalogue: partial), [
            ManagedFavoriteRow(model: "a/one", unavailable: false),
            ManagedFavoriteRow(model: "not/offered", unavailable: false),
        ])
    }

    /// #1754 review: a refused star is described by its cause. Only
    /// NEAR AI not listing the model says so; anything else (the list is
    /// full, the write failed, no daemon) is the generic failure.
    func testAFavoriteFailureNamesOnlyItsOwnCause() {
        XCTAssertEqual(ManagedSurface.favoriteErrorKey("managed-model-unavailable"), "model_unavailable")
        for other in ["managed-invalid-request", "settings-write-failed", "unavailable", nil] as [String?] {
            XCTAssertEqual(ManagedSurface.favoriteErrorKey(other), "action_failed", "\(other ?? "nil")")
        }
    }

    /// A saved or chosen model NEAR AI no longer lists falls back to the
    /// default, and says so. With no catalogue the model is unknown, not
    /// removed: it is kept, and nothing says it is unavailable.
    func testAModelNoLongerListedFallsBackToTheDefault() throws {
        let catalogue = try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(#"{"models":[{"id":"a/one"}],"default":"a/one"}"#.utf8))
        XCTAssertEqual(ManagedSurface.reconciledModel("a/one", in: catalogue), ManagedModelReconcile(model: "a/one", fellBack: false))
        XCTAssertEqual(ManagedSurface.reconciledModel("gone/model", in: catalogue), ManagedModelReconcile(model: nil, fellBack: true))
        XCTAssertEqual(ManagedSurface.reconciledModel(nil, in: catalogue), ManagedModelReconcile(model: nil, fellBack: false))
        XCTAssertEqual(ManagedSurface.reconciledModel("gone/model", in: nil), ManagedModelReconcile(model: "gone/model", fellBack: false))

        // The launch after a fallback names no model: the tool's default.
        // It also says `keep_saved_model`, so the folder's saved model is
        // not replaced by the fallback (#1754 review).
        var draft = ManagedFolderDraft()
        draft.useDefaults(tool: "claude", cwd: "/project", cliOptions: "")
        draft.model = "gone/model"
        XCTAssertFalse(draft.keepsSavedModel)
        draft.fallBackToDefault()
        XCTAssertTrue(draft.keepsSavedModel)
        let params = draft.applying(to: ["tool": "claude", "connection": "near_ai"], daemonKeepsSavedModel: true)
        XCTAssertNil(params["model"] as? String)
        XCTAssertEqual(params["keep_saved_model"] as? Bool, true)
        // A daemon that does not say it reads the flag refuses unknown
        // fields, so it is not sent there (#1754 review).
        XCTAssertNil(draft.applying(to: ["tool": "claude", "connection": "near_ai"], daemonKeepsSavedModel: false)["keep_saved_model"])
        // A model the person picks is theirs to remember.
        draft.choose("a/one")
        XCTAssertFalse(draft.keepsSavedModel)
        let chosen = draft.applying(to: ["tool": "claude", "connection": "near_ai"], daemonKeepsSavedModel: true)
        XCTAssertEqual(chosen["model"] as? String, "a/one")
        XCTAssertNil(chosen["keep_saved_model"])
        draft.fallBackToDefault()
        draft.choose(nil)
        XCTAssertFalse(draft.keepsSavedModel)
        // A new folder or account starts clear.
        draft.fallBackToDefault()
        draft.begin(tool: "claude", cwd: "/other")
        XCTAssertFalse(draft.keepsSavedModel)
        // No model choice at all: nothing to keep, nothing sent.
        draft.useDefaults(tool: "claude", cwd: "/project", cliOptions: "")
        draft.fallBackToDefault()
        XCTAssertNil(draft.applying(to: ["tool": "claude", "connection": "subscription"], daemonKeepsSavedModel: true)["keep_saved_model"])
    }

    /// #1754 review: the snapshot says whether the daemon reads
    /// `keep_saved_model`; an older one says nothing, which reads as no.
    func testTheSnapshotSaysWhetherTheDaemonKeepsTheSavedModel() throws {
        func capabilities(_ extra: String) throws -> ManagedSnapshot.Capabilities {
            try JSONDecoder().decode(ManagedSnapshot.self, from: Data(#"{"revision":1,"accounts":[],"defaults":[],"generations":{},"sessions":[],"capabilities":{"managed_launch":true,"terminal_launch":true\#(extra)}}"#.utf8)).capabilities
        }
        XCTAssertNil(try capabilities("").keepSavedModel)
        XCTAssertEqual(try capabilities(#","keep_saved_model":true"#).keepSavedModel, true)
    }

    /// #1754 review: only the newest catalogue read lands. Account A
    /// (claude) starts R1, B (codex) starts R2, A again starts R3. R1's
    /// failure and R2's answer change nothing, and R3's answer is shown.
    func testOnlyTheNewestCatalogueReadLands() throws {
        let claude = try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(#"{"models":[{"id":"a/one"}],"default":"a/one"}"#.utf8))
        let codex = try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(#"{"models":[{"id":"b/two"}],"default":"b/two"}"#.utf8))
        var reads = ManagedCatalogueReads()
        let r1 = try XCTUnwrap(reads.begin(tool: "claude"))
        XCTAssertNil(reads.begin(tool: "claude"), "one read per tool")
        let r2 = try XCTUnwrap(reads.begin(tool: "codex"))
        let r3 = try XCTUnwrap(reads.begin(tool: "claude"))
        XCTAssertFalse(reads.complete(nil, token: r1))
        XCTAssertFalse(reads.failed)
        XCTAssertFalse(reads.complete(codex, token: r2))
        XCTAssertNil(reads.catalogue)
        XCTAssertTrue(reads.complete(claude, token: r3))
        XCTAssertEqual(reads.catalogue, claude)
        XCTAssertEqual(reads.tool, "claude")

        // A read again after a star keeps what is shown until it answers,
        // and a failure keeps it too. A switch of tool drops its answer.
        let again = try XCTUnwrap(reads.refresh())
        XCTAssertEqual(again.tool, "claude")
        XCTAssertEqual(reads.catalogue, claude)
        XCTAssertTrue(reads.complete(nil, token: again.token))
        XCTAssertEqual(reads.catalogue, claude)
        XCTAssertFalse(reads.failed)
        let stale = try XCTUnwrap(reads.refresh())
        let r4 = try XCTUnwrap(reads.begin(tool: "codex"))
        XCTAssertFalse(reads.complete(claude, token: stale.token), "another tool's catalogue is never shown")
        XCTAssertNil(reads.catalogue)

        // A failed first read is tried again on the next account change.
        XCTAssertTrue(reads.complete(nil, token: r4))
        XCTAssertTrue(reads.failed)
        XCTAssertNil(reads.refresh(), "nothing shown, nothing to read again")
        let r5 = try XCTUnwrap(reads.begin(tool: "codex"))
        XCTAssertFalse(reads.failed)
        XCTAssertTrue(reads.complete(codex, token: r5))
        XCTAssertEqual(reads.catalogue, codex)
    }

    /// #1754 review: a star or removal shows at once, from the setter's
    /// answer (the stored list), as the daemon would list it: favorites are
    /// the listed models it holds, in NEAR AI's order, and a favorite NEAR
    /// AI stopped listing stays only while it is stored.
    func testAFavoriteChangeShowsTheSettersAnswerAtOnce() throws {
        let catalogue = try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(#"{"models":[{"id":"a/one"},{"id":"a/two"},{"id":"a/three"}],"default":"a/one","favorites":["a/two"],"unavailable_favorites":["gone/x","gone/y"]}"#.utf8))
        let starred = catalogue.projecting(storedFavorites: ["gone/x", "a/three", "a/two", "gone/y", "other/tool"])
        XCTAssertEqual(starred.favorites, ["a/two", "a/three"])
        XCTAssertEqual(starred.unavailableFavorites, ["gone/x", "gone/y"])
        XCTAssertEqual(starred.models, catalogue.models)
        XCTAssertEqual(starred.default, catalogue.default)
        XCTAssertTrue(ManagedSurface.isFavoriteModel("a/three", in: starred))
        let removed = starred.projecting(storedFavorites: ["a/three", "gone/y"])
        XCTAssertEqual(removed.favorites, ["a/three"])
        XCTAssertEqual(removed.unavailableFavorites, ["gone/y"])
        XCTAssertFalse(ManagedSurface.isFavoriteModel("a/two", in: removed))
    }

    /// The `managed_model_favorite_set` request: the model and the new
    /// state, nothing else.
    func testTheFavoriteRequestNamesTheModelAndTheState() {
        let params = ManagedSurface.favoriteParams(model: "a/one", favorite: true)
        XCTAssertEqual(params.count, 2)
        XCTAssertEqual(params["model"] as? String, "a/one")
        XCTAssertEqual(params["favorite"] as? Bool, true)
    }

    /// Only Claude Code and Codex on a saved NEAR AI key choose a model in
    /// the Launch sheet. OpenCode keeps its own model field.
    func testOnlyClaudeAndCodexOnASavedNearAiKeyOfferTheModelPicker() {
        XCTAssertTrue(ManagedSurface.offersModelChoice(tool: "claude", connection: "near_ai"))
        XCTAssertTrue(ManagedSurface.offersModelChoice(tool: "codex", connection: "near_ai"))
        XCTAssertFalse(ManagedSurface.offersModelChoice(tool: "opencode", connection: "near_ai"))
        XCTAssertFalse(ManagedSurface.offersModelChoice(tool: "claude", connection: "subscription"))
        XCTAssertFalse(ManagedSurface.offersModelChoice(tool: "codex", connection: "api_key"))
        XCTAssertFalse(ManagedSurface.offersModelChoice(tool: "future_tool", connection: "near_ai"))
    }

    /// Owner, 2026-10-29: the Launch sheet needs no folder either; its
    /// request names none and the daemon starts in the home folder.
    func testLaunchParamsWithNoFolderNameNoCwd() {
        let account = ManagedAccount(id: "a", tool: "claude", connection: "near_ai", label: "Work", authState: "ready")
        let none = ManagedSurface.launchParams(account: account, cwd: nil, requestID: "r", generation: 3, model: nil, collectTraces: false)
        XCTAssertNil(none["cwd"])
        XCTAssertEqual(none["account_id"] as? String, "a")
        let picked = ManagedSurface.launchParams(account: account, cwd: "/p", requestID: "r", generation: 3, model: nil, collectTraces: false)
        XCTAssertEqual(picked["cwd"] as? String, "/p")
    }

    /// The Launch sheet's request names a model only where one was chosen
    /// and the daemon accepts it; the default sends none.
    func testLaunchParamsNameAModelOnlyWhereOneWasChosen() throws {
        let account = { (tool: String, connection: String) in
            ManagedAccount(id: "a", tool: tool, connection: connection, label: "Work", authState: "ready")
        }
        let params = { (account: ManagedAccount, model: String?) in
            ManagedSurface.launchParams(account: account, cwd: "/p", requestID: "r", generation: 3, model: model, collectTraces: true)
        }
        let chosen = params(account("claude", "near_ai"), "openai/gpt-6.1-sol")
        XCTAssertEqual(chosen["model"] as? String, "openai/gpt-6.1-sol")
        XCTAssertNil(chosen["collect_traces"])
        XCTAssertEqual(chosen["connection"] as? String, "near_ai")
        XCTAssertEqual(chosen["expected_generation"] as? UInt64, 3)
        XCTAssertEqual(chosen["cwd"] as? String, "/p")
        XCTAssertNil(params(account("codex", "near_ai"), nil)["model"])
        XCTAssertNil(params(account("codex", "near_ai"), ManagedSurface.defaultModel)["model"])
        XCTAssertNil(params(account("claude", "subscription"), "openai/gpt-6.1-sol")["model"])
        // OpenCode as before: its typed model, trimmed, and the collection opt-in.
        let openCode = params(account("opencode", "api_key"), "  gpt-5 ")
        XCTAssertEqual(openCode["model"] as? String, "gpt-5")
        XCTAssertEqual(openCode["collect_traces"] as? Bool, true)
    }

    func testAnUnavailableModelHasItsOwnLine() {
        XCTAssertEqual(ManagedSurface.errorKey(code: "managed-model-unavailable", message: "managed-model-unavailable"), "model_unavailable")
        XCTAssertEqual(ManagedSurface.errorKey(code: "managed-near-ai-balance-too-low", message: "managed-near-ai-balance-too-low"), "balance_too_low")
    }
}
