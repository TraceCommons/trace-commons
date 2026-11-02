import AppKit
import XCTest

@testable import TCDesign

/// `holdsWindowStill()`: over a holding view the window is not moved by its
/// background, and is again once the pointer has left every one of them.
@MainActor
final class HoldsWindowStillTests: XCTestCase {
    private func window() -> NSWindow {
        let window = NSWindow(contentRect: .init(x: 0, y: 0, width: 200, height: 200),
                              styleMask: [.titled], backing: .buffered, defer: true)
        window.isMovableByWindowBackground = true
        return window
    }

    private func view(in window: NSWindow, y: CGFloat = 0) -> GlassHoldWindowView {
        let view = GlassHoldWindowView(frame: .init(x: 0, y: y, width: 200, height: 20))
        window.contentView?.addSubview(view)
        return view
    }

    private func event(_ type: NSEvent.EventType, _ window: NSWindow) throws -> NSEvent {
        try XCTUnwrap(NSEvent.enterExitEvent(
            with: type, location: .zero, modifierFlags: [], timestamp: 0,
            windowNumber: window.windowNumber, context: nil, eventNumber: 0, trackingNumber: 0, userData: nil))
    }

    /// A folder dragged on the tree does not move the window (owner,
    /// 2026-11-02).
    func testTheWindowHoldsStillWhileThePointerIsOverTheView() throws {
        let window = window()
        let view = view(in: window)
        view.mouseEntered(with: try event(.mouseEntered, window))
        XCTAssertFalse(window.isMovableByWindowBackground)
        XCTAssertNil(view.hitTest(.init(x: 10, y: 10)), "takes no clicks of its own")
        view.mouseExited(with: try event(.mouseExited, window))
        XCTAssertTrue(window.isMovableByWindowBackground)

        view.mouseEntered(with: try event(.mouseEntered, window))
        view.removeFromSuperview()
        XCTAssertTrue(window.isMovableByWindowBackground, "let go when the view goes")
    }

    /// #1839 review, finding 9 (b): the pointer moves from one running row
    /// to the next, and the next one's entry arrives before the first one's
    /// exit. The window stays held until the pointer leaves both.
    func testAbuttingViewsHandOverInEitherOrder() throws {
        let window = window()
        let first = view(in: window)
        let second = view(in: window, y: 20)
        first.mouseEntered(with: try event(.mouseEntered, window))
        second.mouseEntered(with: try event(.mouseEntered, window))
        first.mouseExited(with: try event(.mouseExited, window))
        XCTAssertFalse(window.isMovableByWindowBackground, "still over the second row")
        second.mouseExited(with: try event(.mouseExited, window))
        XCTAssertTrue(window.isMovableByWindowBackground)
    }

    /// #1839 review, finding 9 (a) and (c): a view that appears under a
    /// resting pointer, or scrolls out from under one, gets no enter or
    /// exit; its tracking area is made again then, and it holds or lets go
    /// by where the pointer is.
    func testAViewHoldsOrLetsGoByWhereThePointerIsWhenNoEventCame() {
        let window = window()
        let view = view(in: window)
        view.settle(pointerInside: true)
        XCTAssertFalse(window.isMovableByWindowBackground, "appeared under the pointer")
        view.settle(pointerInside: true)
        view.settle(pointerInside: false)
        XCTAssertTrue(window.isMovableByWindowBackground, "scrolled away, held once")
    }

    /// #1853 review: something that makes the window movable again while
    /// two views hold it does not reset the count: the window stays held
    /// until the last view lets go.
    func testAMovableResetWhileHeldDoesNotDropTheOtherHold() throws {
        let window = window()
        let first = view(in: window)
        let second = view(in: window, y: 20)
        first.mouseEntered(with: try event(.mouseEntered, window))
        window.isMovableByWindowBackground = true
        second.mouseEntered(with: try event(.mouseEntered, window))
        first.mouseExited(with: try event(.mouseExited, window))
        XCTAssertFalse(window.isMovableByWindowBackground, "still over the second view")
        second.mouseExited(with: try event(.mouseExited, window))
        XCTAssertTrue(window.isMovableByWindowBackground)
    }

    /// #1853 review: turning a view's hold off lets go in place, and on
    /// again holds by where the pointer is, with the same view kept.
    func testTurningTheHoldOffLetsGoAndOnHoldsAgain() throws {
        let window = window()
        let view = view(in: window)
        view.mouseEntered(with: try event(.mouseEntered, window))
        view.isEnabled = false
        XCTAssertTrue(window.isMovableByWindowBackground)
        view.mouseEntered(with: try event(.mouseEntered, window))
        XCTAssertTrue(window.isMovableByWindowBackground, "off: holds nothing")
        view.settle(pointerInside: false)
        view.isEnabled = true
        view.settle(pointerInside: true)
        XCTAssertFalse(window.isMovableByWindowBackground)
        view.settle(pointerInside: false)
        XCTAssertTrue(window.isMovableByWindowBackground)
    }

    /// A window something else holds still is left as it is.
    func testAWindowHeldStillElsewhereIsNotLetGo() throws {
        let window = window()
        window.isMovableByWindowBackground = false
        let view = view(in: window)
        view.mouseEntered(with: try event(.mouseEntered, window))
        view.mouseExited(with: try event(.mouseExited, window))
        XCTAssertFalse(window.isMovableByWindowBackground)
    }
}
