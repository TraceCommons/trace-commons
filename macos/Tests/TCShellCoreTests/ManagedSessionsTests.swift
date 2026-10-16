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

    func testOpenCodeKeepsKeyConnectionsWithoutSignInLaunch() {
        XCTAssertEqual(ManagedSurface.connectionOptions(signedIn: true, tool: "opencode"), ["api_key", "near_ai"])
        XCTAssertEqual(ManagedSurface.connectionOptions(signedIn: false, tool: "opencode"), ["api_key", "near_ai"])
        XCTAssertEqual(ManagedSurface.signInTools, ["claude", "codex"])
        XCTAssertNil(ManagedSurface.startKey("opencode"))
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
        let line = "Uses your NEAR AI sign-in. Runs in {destination}, in the folder you choose."
        XCTAssertEqual(ManagedSurface.signInScope(line, destination: "Terminal", fallback: "your terminal"),
                       "Uses your NEAR AI sign-in. Runs in Terminal, in the folder you choose.")
        XCTAssertEqual(ManagedSurface.signInScope(line, destination: nil, fallback: "your terminal"),
                       "Uses your NEAR AI sign-in. Runs in your terminal, in the folder you choose.")
        XCTAssertEqual(ManagedSurface.startKey("claude"), "start_claude")
        XCTAssertEqual(ManagedSurface.startKey("codex"), "start_codex")
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

    func testTheFolderExplainerLineFillsToolAndDestination() {
        let line = "Choose a folder for {tool} to work in. It opens in {destination}."
        XCTAssertEqual(ManagedSurface.folderChoiceLine(line, tool: "Claude Code", destination: "Terminal", fallback: "your terminal"),
                       "Choose a folder for Claude Code to work in. It opens in Terminal.")
        XCTAssertEqual(ManagedSurface.folderChoiceLine(line, tool: "Codex", destination: nil, fallback: "your terminal"),
                       "Choose a folder for Codex to work in. It opens in your terminal.")
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
}
