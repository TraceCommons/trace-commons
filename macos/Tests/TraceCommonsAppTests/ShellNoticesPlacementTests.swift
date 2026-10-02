import XCTest

/// Every window that shows the contributor's traces carries the shared
/// notice stack (grant voids, arming rewordings, a gate hold, the attached
/// daemon and the legacy-invite migration). The monitor had none of them,
/// so a void during monitor use was never told.
final class ShellNoticesPlacementTests: XCTestCase {
    func test_theMainWindowAndTheMonitorBothShowTheNotices() throws {
        let views = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views")
        for file in ["MainWindowView.swift", "MonitorWindowView.swift"] {
            let text = try String(contentsOf: views.appendingPathComponent(file), encoding: .utf8)
            XCTAssertTrue(text.contains("ShellNotices()"), "\(file) does not show the notice stack")
        }
        // One definition of the stack, so the two cannot drift.
        let main = try String(contentsOf: views.appendingPathComponent("MainWindowView.swift"), encoding: .utf8)
        XCTAssertEqual(main.components(separatedBy: "GrantVoidNotices(").count - 1, 1)
    }
}
