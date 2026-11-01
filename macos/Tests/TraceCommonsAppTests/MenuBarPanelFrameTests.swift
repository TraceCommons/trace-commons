import AppKit
import XCTest
@testable import TraceCommonsApp

/// The menu-bar panel's window takes its size from `MenuBarTopEdgePin`
/// alone. Its hosting view took #1402's empty sizing options in #1511, and
/// in the app as built against the macOS 26 SDK it still resized the window
/// toward its content from inside `windowDidLayout` whenever the content
/// changed height, while the pin set it back on the next turn: two writers,
/// logged as "window resizing repeatedly: menu-bar panel". The window is
/// SwiftUI's, so `MenuBarPanelFrame` gates its class's frame setters.
@MainActor
final class MenuBarPanelFrameTests: XCTestCase {
    /// A window class that overrides `setFrame(_:display:)` itself, as
    /// SwiftUI's `MenuBarExtraWindow` does.
    private final class OverridingPanel: NSPanel {
        override func setFrame(_ frameRect: NSRect, display flag: Bool) {
            super.setFrame(frameRect, display: flag)
        }
    }

    /// A window class that inherits every frame setter.
    private final class InheritingPanel: NSPanel {}

    /// A window class used only by the key-value observation test.
    private final class ObservedPanel: NSPanel {}

    private static let placed = CGRect(x: 1265, y: 851, width: 380, height: 277)

    private func panel<P: NSPanel>(_ type: P.Type) -> P {
        P(contentRect: Self.placed, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
    }

    /// Each way a size reaches a window, none of them `place(_:in:)`:
    /// SwiftUI's animated resize came through `setFrame(_:display:)`, and
    /// its first sizing through `setContentSize(_:)`.
    private func resizeFromOutside(_ window: NSWindow) {
        window.setFrame(CGRect(x: 1265, y: 788, width: 380, height: 340), display: true)
        window.setFrame(CGRect(x: 1265, y: 830, width: 380, height: 298), display: true, animate: false)
        window.setContentSize(NSSize(width: 380, height: 349))
    }

    // MARK: Held windows

    /// A held window keeps only the sizes `place(_:in:)` gives it: the
    /// writes that resized the panel from outside change nothing.
    func test_aHeldWindowKeepsOnlyTheSizesItIsPlaced() {
        let window = panel(OverridingPanel.self)
        MenuBarPanelFrame.hold(window)
        MenuBarPanelFrame.place(Self.placed, in: window)
        XCTAssertEqual(window.frame, Self.placed)

        resizeFromOutside(window)
        XCTAssertEqual(window.frame, Self.placed, "a size not from place(_:in:) was taken")

        let grown = CGRect(x: 1265, y: 779, width: 380, height: 349)
        MenuBarPanelFrame.place(grown, in: window)
        XCTAssertEqual(window.frame, grown)
    }

    /// A move that keeps the size still lands, so SwiftUI can still put
    /// the window under its menu-bar item.
    func test_aHeldWindowStillMoves() {
        let window = panel(OverridingPanel.self)
        MenuBarPanelFrame.hold(window)
        let moved = Self.placed.offsetBy(dx: 65, dy: 1076)
        window.setFrame(moved, display: true)
        XCTAssertEqual(window.frame, moved)
        window.setFrame(Self.placed, display: true, animate: false)
        XCTAssertEqual(window.frame, Self.placed)
    }

    /// A class that inherits its setters is held the same way, and the
    /// class it inherits them from is left alone.
    func test_aWindowThatInheritsItsSettersIsHeldToo() {
        let window = panel(InheritingPanel.self)
        MenuBarPanelFrame.hold(window)
        resizeFromOutside(window)
        XCTAssertEqual(window.frame, Self.placed)

        let plain = panel(NSPanel.self)
        let resized = CGRect(x: 1265, y: 788, width: 380, height: 340)
        plain.setFrame(resized, display: true)
        XCTAssertEqual(plain.frame, resized, "the superclass's setters were changed")
    }

    /// Only held windows are held: another window of the same class takes
    /// every frame.
    func test_aWindowThatIsNotHeldTakesEveryFrame() {
        let held = panel(OverridingPanel.self)
        MenuBarPanelFrame.hold(held)
        let other = panel(OverridingPanel.self)
        let resized = CGRect(x: 1265, y: 788, width: 380, height: 340)
        other.setFrame(resized, display: true)
        XCTAssertEqual(other.frame, resized)
        other.setContentSize(NSSize(width: 380, height: 349))
        XCTAssertEqual(other.frame.size, CGSize(width: 380, height: 349))
    }

    /// Held twice is held once: the setters are wrapped once per class, so
    /// a held window still takes `place(_:in:)`'s sizes.
    func test_holdingTwiceStillPlaces() {
        let window = panel(OverridingPanel.self)
        MenuBarPanelFrame.hold(window)
        MenuBarPanelFrame.hold(window)
        MenuBarPanelFrame.hold(panel(OverridingPanel.self))
        let grown = CGRect(x: 1265, y: 779, width: 380, height: 349)
        MenuBarPanelFrame.place(grown, in: window)
        XCTAssertEqual(window.frame, grown)
        resizeFromOutside(window)
        XCTAssertEqual(window.frame, grown)
    }

    /// A window under key-value observation has the observer's subclass in
    /// front of its own; the setters guarded are its own class's, so the
    /// window is held while observed and after.
    func test_anObservedWindowIsHeldByItsOwnClass() throws {
        let window = panel(ObservedPanel.self)
        let observation = window.observe(\.title) { _, _ in }
        XCTAssertNotEqual(ObjectIdentifier(try XCTUnwrap(object_getClass(window))), ObjectIdentifier(ObservedPanel.self),
                          "observation did not put a subclass in front")
        XCTAssertEqual(ObjectIdentifier(MenuBarPanelFrame.ownClass(of: window)), ObjectIdentifier(ObservedPanel.self))

        MenuBarPanelFrame.hold(window)
        resizeFromOutside(window)
        XCTAssertEqual(window.frame, Self.placed)

        observation.invalidate()
        resizeFromOutside(window)
        XCTAssertEqual(window.frame, Self.placed)
    }

    // MARK: The rule

    func test_keepsDecidesFromHeldPlacingAndSize() {
        let current = CGSize(width: 380, height: 277)
        let other = CGSize(width: 380, height: 349)
        // Not held: every write.
        XCTAssertTrue(MenuBarPanelFrame.keeps(proposed: other, current: current, held: false, placing: false))
        // Held: the pin's writes, and others only while the size stays.
        XCTAssertTrue(MenuBarPanelFrame.keeps(proposed: other, current: current, held: true, placing: true))
        XCTAssertTrue(MenuBarPanelFrame.keeps(proposed: current, current: current, held: true, placing: false))
        XCTAssertFalse(MenuBarPanelFrame.keeps(proposed: other, current: current, held: true, placing: false))
        // A fraction of a point is still a resize.
        XCTAssertFalse(MenuBarPanelFrame.keeps(
            proposed: CGSize(width: 380, height: 277.4), current: current, held: true, placing: false))
    }

    // MARK: Source pins

    /// The pin holds the window on the next turn, with the sizing taken
    /// over, and sets its frame only through `place(_:in:)`.
    func test_thePinHoldsTheWindowAndPlacesItThroughTheGate() throws {
        let panel = try WindowResizeLoopTests.text(WindowResizeLoopTests.panelSource)
        let pin = try XCTUnwrap(WindowResizeLoopTests.block(after: "struct MenuBarTopEdgePin: NSViewRepresentable {", in: panel))
        let nextTurn = try XCTUnwrap(WindowResizeLoopTests.block(after: "GlassWindowFrame.onNextTurn { [weak self] in", in: pin))
        XCTAssertTrue(nextTurn.contains("MenuBarTopEdgePin.takeOverSizing(from: self)\n                MenuBarPanelFrame.hold(window)"))
        XCTAssertTrue(pin.contains("MenuBarPanelFrame.place(frame, in: window)"))
        XCTAssertFalse(pin.contains("window.setFrame("), "the pin sets the frame around the gate")
        let moved = try XCTUnwrap(WindowResizeLoopTests.block(after: "override func viewDidMoveToWindow()", in: pin))
        XCTAssertFalse(moved.contains("MenuBarPanelFrame.hold"), "held before SwiftUI first sizes the window")
    }
}
