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
    ]

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
    }
}
