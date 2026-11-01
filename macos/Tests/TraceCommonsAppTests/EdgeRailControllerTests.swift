import AppKit
import SwiftUI
import XCTest
@testable import TraceCommonsApp

/// The rail's panel is sized by its controller alone. Its hosting view
/// sized it to the rail's SwiftUI content inside the window's own layout,
/// one resize per pass, until AppKit threw and the app aborted: in 0.16.0
/// from `_postWindowNeedsUpdateConstraints` (#1402), and through 0.20.5
/// from `_postWindowNeedsLayout`, every time the rail opened in the app as
/// built against the macOS 26 SDK (#1517).
@MainActor
final class EdgeRailControllerTests: XCTestCase {
    func test_theHostingViewTakesNoSizingOptionsOrSafeArea() {
        let hosting = EdgeRailController.hostingView(Color.clear)
        XCTAssertEqual(hosting.sizingOptions, [])
        XCTAssertEqual(hosting.safeAreaRegions, [])
    }

    // MARK: The panel's frame

    /// Only `place(_:)` moves or sizes the panel. SwiftUI's
    /// `updateAnimatedWindowSize` reached the panel through
    /// `setFrame(_:display:)` even with empty sizing options; it and the
    /// panel's other resizers now change nothing.
    func test_thePanelKeepsOnlyTheFramesItIsPlacedAt() {
        let panel = EdgeRailPanel()
        let open = CGRect(x: 1316, y: 225, width: 484, height: 680)
        panel.place(open)
        XCTAssertEqual(panel.frame, open)

        panel.setFrame(CGRect(x: 1316, y: 225, width: 464, height: 282), display: true)
        panel.setFrame(CGRect(x: 1316, y: 225, width: 4, height: 240), display: true, animate: false)
        panel.setContentSize(NSSize(width: 152, height: 0))
        XCTAssertEqual(panel.frame, open, "a frame not from place(_:) was taken")

        let closed = CGRect(x: 1796, y: 445, width: 4, height: 240)
        panel.place(closed)
        XCTAssertEqual(panel.frame, closed)
    }

    // MARK: Opening and closing

    private enum Step: Equatable {
        case place(open: Bool)
        case rail(open: Bool)
    }

    /// An `EdgeRailOpening` whose run-loop turns and animation completions
    /// run only when the test says, recording each step in order.
    @MainActor
    private final class Recorder {
        let state = EdgeRailState()
        var steps: [Step] = []
        var turns: [@MainActor () -> Void] = []
        var completions: [@MainActor () -> Void] = []
        lazy var opening = EdgeRailOpening(
            state: state,
            nextTurn: { [unowned self] in turns.append($0) },
            animate: { [unowned self] _, body, completion in
                body()
                steps.append(.rail(open: state.open))
                completions.append(completion)
            },
            place: { [unowned self] in steps.append(.place(open: $0)) })

        func runTurns() {
            while !turns.isEmpty { turns.removeFirst()() }
        }

        func finishAnimations() {
            let done = completions
            completions.removeAll()
            done.forEach { $0() }
        }
    }

    /// Opening grows the panel before the rail draws open, both on the
    /// next turn: never inside the SwiftUI update that asked.
    func test_openingGrowsThePanelBeforeTheRailDrawsOpen() {
        let rail = Recorder()
        rail.opening.set(true, animation: nil)
        XCTAssertEqual(rail.steps, [], "moved inside the caller's update")
        XCTAssertFalse(rail.state.open)

        rail.runTurns()
        XCTAssertEqual(rail.steps, [.place(open: true), .rail(open: true)])
        XCTAssertTrue(rail.state.open)
    }

    /// Closing slides the rail out at once and shrinks the panel only once
    /// it has gone, on the turn after.
    func test_closingShrinksThePanelOnlyAfterTheRailHasGone() {
        let rail = Recorder()
        rail.opening.set(true, animation: nil)
        rail.runTurns()
        rail.steps.removeAll()

        rail.opening.set(false, animation: nil)
        XCTAssertEqual(rail.steps, [.rail(open: false)])
        rail.runTurns()
        XCTAssertEqual(rail.steps, [.rail(open: false)], "shrank under the rail while it was going")

        rail.finishAnimations()
        rail.runTurns()
        XCTAssertEqual(rail.steps, [.rail(open: false), .place(open: false)])
    }

    /// The pointer back on the rail while it slides out: it opens again,
    /// and the close's late completion leaves the panel open.
    func test_reopeningWhileClosingKeepsThePanelOpen() {
        let rail = Recorder()
        rail.opening.set(true, animation: nil)
        rail.runTurns()
        rail.opening.set(false, animation: nil)
        rail.steps.removeAll()

        rail.opening.set(true, animation: nil)
        rail.runTurns()
        rail.finishAnimations()
        rail.runTurns()
        XCTAssertEqual(rail.steps, [.place(open: true), .rail(open: true)])
        XCTAssertTrue(rail.state.open)
    }

    /// Open and then closed before the open's turn comes: nothing moves.
    func test_closingBeforeTheOpenTurnLeavesTheRailClosed() {
        let rail = Recorder()
        rail.opening.set(true, animation: nil)
        rail.opening.set(false, animation: nil)
        rail.runTurns()
        rail.finishAnimations()
        rail.runTurns()
        XCTAssertEqual(rail.steps, [])
        XCTAssertFalse(rail.state.open)
    }

    /// Asked to open twice, the rail opens once.
    func test_openingAnOpenRailDoesNothing() {
        let rail = Recorder()
        rail.opening.set(true, animation: nil)
        rail.runTurns()
        rail.opening.set(true, animation: nil)
        rail.runTurns()
        XCTAssertEqual(rail.steps, [.place(open: true), .rail(open: true)])
    }
}
