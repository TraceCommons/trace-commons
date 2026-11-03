import XCTest
@testable import TCShellCore

/// The launch dialog's window list (owner, 2026-11-02, #1852): the daemon
/// lists windows by their running sessions; the dialog names each by those
/// sessions' rows and sends the picked one's newest session as `beside`.
final class ManagedWindowPickerTests: XCTestCase {
    private func row(_ id: String, tool: String = "claude", folder: String, state: String = "running") throws -> ManagedSession {
        try JSONDecoder().decode(ManagedSession.self, from: Data(#"""
        {"id":"\#(id)","tool":"\#(tool)","connection":"near_ai_signin","account_id":"a","account_label":"NEAR AI","project_label":"\#(folder)","cwd":"/tmp","state":"\#(state)","purpose":"coding"}
        """#.utf8))
    }

    private let names = ["claude": "Claude Code", "codex": "Codex"]

    func testTheDaemonsListIsReadFailClosed() {
        XCTAssertEqual(ManagedSurface.terminalWindows(["windows": [["sessions": ["b", "a"]], ["sessions": ["c"]]]]), [["b", "a"], ["c"]])
        XCTAssertEqual(ManagedSurface.terminalWindows(["windows": [["sessions": []], ["sessions": [1]], ["window": 7]]]), [])
        XCTAssertEqual(ManagedSurface.terminalWindows(["windows": "x"]), [])
        XCTAssertEqual(ManagedSurface.terminalWindows([:]), [], "an older daemon, or a refusal")
    }

    func testEachWindowIsNamedByItsRunningSessionsNewestFirst() throws {
        let sessions = [try row("a", folder: "api"), try row("b", tool: "codex", folder: "web"), try row("c", folder: "docs")]
        let options = ManagedSurface.windowOptions(
            [["b", "a"], ["c"]], sessions: sessions, tool: { self.names[$0] }, template: "{tool} · {folder}")
        XCTAssertEqual(options, [
            ManagedWindowOption(beside: "b", title: "Codex · web, Claude Code · api"),
            ManagedWindowOption(beside: "c", title: "Claude Code · docs"),
        ])
    }

    func testAWindowWithNoRunningSessionTheAppListsIsNotOffered() throws {
        let sessions = [try row("a", folder: "api", state: "exited"), try row("b", folder: "web")]
        let options = ManagedSurface.windowOptions(
            [["a"], ["z", "b"], ["y"]], sessions: sessions, tool: { self.names[$0] }, template: "{tool} · {folder}")
        XCTAssertEqual(options, [ManagedWindowOption(beside: "b", title: "Claude Code · web")])
    }

    /// Owner, 2026-11-03: new sessions open as tabs in the window already
    /// in use; "New window" only when there is none.
    func testTheNewestWindowIsTheDefault() {
        let options = [ManagedWindowOption(beside: "b", title: "Codex · web"),
                       ManagedWindowOption(beside: "c", title: "Claude Code · docs")]
        XCTAssertEqual(ManagedSurface.launchDefault(options: options), "b")
        XCTAssertNil(ManagedSurface.launchDefault(options: []), "New window")
    }

    func testStartAgainAndResumeOpenBesideTheNewestSession() {
        XCTAssertTrue(ManagedSurface.opensBesideNewest("managed_session_start_again"))
        XCTAssertTrue(ManagedSurface.opensBesideNewest("managed_session_resume"))
        XCTAssertFalse(ManagedSurface.opensBesideNewest("managed_launch_prepare"), "the dialog sends its own pick")
    }

    func testOnlyAnOfferedWindowIsSent() {
        let options = [ManagedWindowOption(beside: "b", title: "Claude Code · web")]
        XCTAssertNil(ManagedSurface.launchBeside(nil, options: options), "New window")
        XCTAssertEqual(ManagedSurface.launchBeside("b", options: options), "b")
        XCTAssertNil(ManagedSurface.launchBeside("gone", options: options))
        XCTAssertNil(ManagedSurface.launchBeside("b", options: []))
    }
}
