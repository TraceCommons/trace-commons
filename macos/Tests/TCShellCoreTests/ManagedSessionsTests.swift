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
}
