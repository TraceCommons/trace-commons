import XCTest
import TCShellCore
@testable import TraceCommonsApp

/// #1839 review, findings 9 and 10: a session row holds the window still
/// only where its click shows a terminal, and that click is off, not
/// dropped unseen, while another request runs.
final class ManagedRowClickTests: XCTestCase {
    private func session(_ state: String, focus: Bool, stop: Bool) throws -> ManagedSession {
        let json = #"{"id":"s","tool":"claude","connection":"subscription","account_id":"a","account_label":"Work","project_label":"Project","cwd":"/tmp","state":"\#(state)","purpose":"coding","can_focus":\#(focus),"can_stop":\#(stop),"can_start_again":false}"#
        return try JSONDecoder().decode(ManagedSession.self, from: Data(json.utf8))
    }

    func testOnlyARowThatShowsItsTerminalHoldsTheWindow() throws {
        let terminal = try session("running", focus: true, stop: true)
        XCTAssertTrue(ManagedRowClick.holdsWindow(terminal))
        XCTAssertTrue(ManagedRowClick.acts(terminal, busy: false))
        XCTAssertFalse(ManagedRowClick.acts(terminal, busy: true))

        // Running in Ghostty, iTerm2 and the rest: Stop, but nothing to show.
        let elsewhere = try session("running", focus: false, stop: true)
        XCTAssertFalse(ManagedRowClick.holdsWindow(elsewhere))
        XCTAssertFalse(ManagedRowClick.acts(elsewhere, busy: false))
    }
}
