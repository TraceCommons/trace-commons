import XCTest

@testable import TraceCommonsApp

/// The window's tabs, in the owner's order (2026-10-28): Home, Traces,
/// Private AI, Insights, with Cmd-1..4 following the strip.
@MainActor
final class TabOrderTests: XCTestCase {
    func test_tracesIsSecondAndPrivateAIThird() {
        XCTAssertEqual(MonitorWindowView.Tab.allCases, [.home, .traces, .inference, .insights])
        XCTAssertEqual(MonitorCommands.shortcut(.traces), "2")
        XCTAssertEqual(MonitorCommands.shortcut(.inference), "3")
    }
}
