import XCTest

@testable import TCDesign

/// A window frame set inside a SwiftUI update re-enters the layout pass
/// that is already running, and on a window SwiftUI also sizes the two
/// fight until AppKit throws `_postWindowNeedsUpdateConstraints` and the app
/// aborts. That is how the edge rail crashed in 0.16.0 (#1402). The main
/// window's pane moves and the menu-bar panel's top-edge pin had the same
/// shape, so both now set their frame on the next turn of the run loop.
@MainActor
final class WindowFrameTurnTests: XCTestCase {
    func test_aFrameChangeWaitsForTheNextRunLoopTurn() {
        var ran = 0
        GlassWindowFrame.onNextTurn { ran += 1 }
        XCTAssertEqual(ran, 0, "ran inside the caller's update pass")
        let done = expectation(description: "ran")
        DispatchQueue.main.async { done.fulfill() }
        wait(for: [done], timeout: 1)
        XCTAssertEqual(ran, 1)
    }

    /// Two moves asked for in one update run in the order they were asked,
    /// so the later one still wins.
    func test_movesKeepTheirOrder() {
        var order: [Int] = []
        GlassWindowFrame.onNextTurn { order.append(1) }
        GlassWindowFrame.onNextTurn { order.append(2) }
        let done = expectation(description: "ran")
        DispatchQueue.main.async { done.fulfill() }
        wait(for: [done], timeout: 1)
        XCTAssertEqual(order, [1, 2])
    }

    /// The pane move is asked for from `.onChange`, a SwiftUI update, so
    /// it goes through the next turn rather than resizing the window there.
    func test_thePaneMoveLeavesTheUpdatePass() throws {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TCDesign/Components/ThreePane.swift")
        let source = try String(contentsOf: url, encoding: .utf8)
        XCTAssertTrue(source.contains("GlassWindowFrame.onNextTurn { move(to: new) }"))
        XCTAssertFalse(source.contains("_, new in\n            move(to: new)"))
    }
}
