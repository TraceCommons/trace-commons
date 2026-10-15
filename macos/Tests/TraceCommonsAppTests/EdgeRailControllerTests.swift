import AppKit
import SwiftUI
import XCTest
@testable import TraceCommonsApp

/// The rail's panel is sized by its controller alone. A hosting view left
/// on its default sizing options also sizes the window to its SwiftUI
/// content, and the two fought inside one layout pass until AppKit threw
/// `_postWindowNeedsUpdateConstraints` and the app aborted (2026-10-13,
/// 0.16.0: hovering the rail open and shut).
@MainActor
final class EdgeRailControllerTests: XCTestCase {
    func test_theHostingViewNeverSizesThePanel() {
        let hosting = EdgeRailController.hostingView(Color.clear)
        XCTAssertEqual(hosting.sizingOptions, [])
        XCTAssertEqual(hosting.safeAreaRegions, [])
    }

    /// Opening or closing moves the panel on the next turn of the run
    /// loop, never inside the SwiftUI update that asked for it.
    func test_placementWaitsForTheNextRunLoopTurn() {
        var placed: [Bool] = []
        let defer_ = EdgeRailController.deferred { placed.append($0) }
        defer_(true)
        XCTAssertEqual(placed, [], "placed inside the caller's update pass")
        let done = expectation(description: "placed")
        DispatchQueue.main.async { done.fulfill() }
        wait(for: [done], timeout: 1)
        XCTAssertEqual(placed, [true])
    }
}
