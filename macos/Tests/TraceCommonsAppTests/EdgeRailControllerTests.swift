import AppKit
import SwiftUI
import TCDesign
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
    /// loop, never inside the SwiftUI update that asked for it, and the
    /// rail's content changes only once the panel is at the size it needs
    /// (#1517): opening grows the panel on one turn and opens the rail on
    /// the next, so the open rail never lays out in the closed hover zone.
    func test_placementWaitsForTheNextRunLoopTurn() {
        var placed: [Bool] = []
        let defer_ = EdgeRailController.deferred { placed.append($0) }
        defer_(true)
        XCTAssertEqual(placed, [], "placed inside the caller's update pass")
        nextTurn()
        XCTAssertEqual(placed, [true])

        let state = EdgeRailState()
        var events: [String] = []
        EdgeRailController.transition(to: true, state: state, animation: nil) { open in
            events.append("place(\(open)) with the rail \(state.open ? "open" : "closed")")
        }
        XCTAssertEqual(events, [], "placed inside the caller's update pass")
        XCTAssertFalse(state.open, "the rail opened before its panel grew")
        nextTurn()
        XCTAssertEqual(events, ["place(true) with the rail closed"])
        XCTAssertFalse(state.open, "the rail opened in the turn its panel grew, inside that resize")
        nextTurn()
        XCTAssertTrue(state.open)
        XCTAssertEqual(events, ["place(true) with the rail closed"])
    }

    /// Closing takes the rail away first, at the open panel's size, and
    /// shrinks the panel only once the close transition has finished: the
    /// closing rail never lays out in the closed hover zone (#1517, whose
    /// exception named the panel at that 4 x 240 pt). Hosted, so the
    /// transition really runs; the shrink must wait for it, not for a turn.
    func test_closingShrinksThePanelOnlyAfterTheRailHasClosed() {
        let state = EdgeRailState()
        let (window, _) = host(state: state)
        defer { window.orderOut(nil) }
        state.open = true
        state.wanted = true
        for _ in 0..<3 { nextTurn() }

        var events: [String] = []
        let shrunk = expectation(description: "shrunk")
        let started = Date()
        var elapsed: TimeInterval = 0
        EdgeRailController.transition(to: false, state: state, animation: .linear(duration: 0.3)) { open in
            events.append("place(\(open)) with the rail \(state.open ? "open" : "closed")")
            elapsed = Date().timeIntervalSince(started)
            shrunk.fulfill()
        }
        XCTAssertFalse(state.open)
        XCTAssertEqual(events, [], "shrunk inside the caller's update pass")
        nextTurn()
        nextTurn()
        XCTAssertEqual(events, [], "shrunk while the rail was still closing")
        wait(for: [shrunk], timeout: 5)
        XCTAssertEqual(events, ["place(false) with the rail closed"])
        // A lower bound only: a slow machine makes it later, never earlier.
        XCTAssertGreaterThanOrEqual(elapsed, 0.25, "shrunk before the close transition ended")
    }

    /// The real rail in a real panel, opened and closed through the
    /// controller's sequence: its open content is never laid out in the
    /// closed hover zone (#1517). Before the fix every open did so, for a
    /// display cycle, under the open animation.
    func test_theOpenRailNeverLaysOutInTheHoverZone() {
        let state = EdgeRailState()
        let visible = CGRect(x: -6000, y: -3000, width: 1512, height: 944)
        let (window, hosting) = host(
            state: state, real: true, frame: EdgeRailGeometry.frame(open: false, visible: visible))
        defer { window.orderOut(nil) }
        var violations = 0
        hosting.onLayout = { width in
            if state.open, width <= GlassTokens.Size.edgeRailZoneWidth { violations += 1 }
        }
        for open in [true, false, true, false, true, false] {
            let placed = expectation(description: "placed \(open)")
            EdgeRailController.transition(
                to: open, state: state, animation: GlassMotion.curve(GlassTokens.Motion.slide)
            ) { open in
                window.setFrame(EdgeRailGeometry.frame(open: open, visible: visible), display: true)
                placed.fulfill()
            }
            wait(for: [placed], timeout: 5)
            for _ in 0..<3 { nextTurn() }
            XCTAssertEqual(state.open, open)
        }
        XCTAssertEqual(violations, 0, "the open rail laid out in the closed panel")
        XCTAssertEqual(window.frame.size, EdgeRailGeometry.frame(open: false, visible: visible).size)
    }

    /// Hovering back in while the rail is closing keeps the panel open: the
    /// close's late shrink does not land under the reopened rail, and the
    /// last request wins either way round.
    func test_theLastRequestWins() {
        let state = EdgeRailState()
        state.open = true
        var placed: [Bool] = []
        let place: @MainActor (Bool) -> Void = { placed.append($0) }
        EdgeRailController.transition(to: false, state: state, animation: .linear(duration: 0.05), place: place)
        EdgeRailController.transition(to: true, state: state, animation: nil, place: place)
        settle()
        XCTAssertTrue(state.open)
        XCTAssertEqual(placed.last, true)
        XCTAssertFalse(placed.contains(false), "a cancelled close shrank the panel")

        placed = []
        EdgeRailController.transition(to: false, state: state, animation: nil, place: place)
        EdgeRailController.transition(to: true, state: state, animation: nil, place: place)
        EdgeRailController.transition(to: false, state: state, animation: nil, place: place)
        settle()
        XCTAssertFalse(state.open)
        XCTAssertEqual(placed.last, false)
        XCTAssertFalse(placed.contains(true), "a cancelled open grew the panel")
    }

    /// A hosting view that reports each layout's width.
    final class LayoutProbe<Content: View>: NSHostingView<Content> {
        var onLayout: ((CGFloat) -> Void)?
        override func layout() {
            super.layout()
            onLayout?(bounds.width)
        }
    }

    /// The rail (or, by default, a stand-in that animates the same way:
    /// a wide view sliding in and out) in an `EdgeRailPanel`, offscreen,
    /// on the controller's hosting settings.
    private func host(
        state: EdgeRailState, real: Bool = false,
        frame: CGRect = CGRect(x: -6000, y: -3000, width: 400, height: 240)
    ) -> (EdgeRailPanel, LayoutProbe<AnyView>) {
        let root: AnyView = real
            ? AnyView(EdgeRailView(state: state, data: EdgeRailData(), onOpenChange: { _, _ in })
                .environmentObject(AppModel()))
            : AnyView(StandIn(state: state))
        let hosting = LayoutProbe(rootView: root)
        hosting.sizingOptions = []
        hosting.safeAreaRegions = []
        let panel = EdgeRailPanel()
        panel.contentView = hosting
        panel.setFrame(frame, display: true)
        panel.orderFrontRegardless()
        return (panel, hosting)
    }

    private struct StandIn: View {
        let state: EdgeRailState
        var body: some View {
            ZStack {
                if state.open {
                    Color.gray.frame(width: 300).transition(.move(edge: .trailing))
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .trailing)
        }
    }

    /// One turn of the main run loop: whatever was queued before it runs.
    private func nextTurn() {
        let done = expectation(description: "turn")
        DispatchQueue.main.async { done.fulfill() }
        wait(for: [done], timeout: 1)
    }

    /// Enough turns, and time, for every queued step and a short close
    /// animation's completion. Counted in turns; the time only covers the
    /// 0.05 s animation.
    private func settle() {
        for _ in 0..<5 { nextTurn() }
        RunLoop.main.run(until: Date().addingTimeInterval(0.3))
        for _ in 0..<5 { nextTurn() }
    }
}
