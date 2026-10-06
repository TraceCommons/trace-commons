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

    func testSurfaceNeverPicksARawValueOrDaemonCode() {
        XCTAssertEqual(ManagedSurface.stateKey("running"), "state_running")
        XCTAssertEqual(ManagedSurface.stateKey("future_state"), "state_unknown")
        XCTAssertEqual(ManagedSurface.authKey("sign_in_required"), "auth_sign_in_required")
        XCTAssertNil(ManagedSurface.authKey("future_auth"))
        XCTAssertNil(ManagedSurface.toolKey("future_tool"))
        XCTAssertEqual(ManagedSurface.errorKey(code: "launch-unknown", message: "managed-launch-unknown"), "launch_unknown")
        XCTAssertEqual(ManagedSurface.errorKey(code: "conflict", message: "raw daemon text"), "action_failed")
        XCTAssertEqual(ManagedSurface.errorKey(code: nil, message: nil), "action_failed")
    }

    func testSnapshotKnowsWhichAccountsAreHeldAndDefault() throws {
        let snapshot = try JSONDecoder().decode(ManagedSnapshot.self, from: Data(#"{"revision":1,"accounts":[{"id":"a","tool":"claude","connection":"subscription","label":"Personal","auth_state":"ready"},{"id":"b","tool":"claude","connection":"api_key","label":"Work","auth_state":"ready"}],"defaults":[{"tool":"claude","connection":"subscription","account_id":"a","generation":1}],"generations":{"claude":1},"sessions":[{"id":"s","tool":"claude","connection":"subscription","account_id":"a","account_label":"Personal","project_label":"Project","cwd":"/tmp","state":"running","purpose":"coding"},{"id":"t","tool":"claude","connection":"api_key","account_id":"b","account_label":"Work","project_label":"Project","cwd":"/tmp","state":"exited","purpose":"coding"}],"capabilities":{"managed_launch":true,"terminal_launch":false}}"#.utf8))
        XCTAssertTrue(snapshot.isHeld(snapshot.accounts[0]))
        XCTAssertFalse(snapshot.isHeld(snapshot.accounts[1]))
        XCTAssertTrue(snapshot.isDefault(snapshot.accounts[0]))
        XCTAssertFalse(snapshot.isDefault(snapshot.accounts[1]))
    }
}
