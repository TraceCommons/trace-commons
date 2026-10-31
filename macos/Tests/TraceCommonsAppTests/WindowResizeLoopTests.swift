import AppKit
import TCDesign
import XCTest

@testable import TraceCommonsApp

/// The app's recurring crash (twelve reports, 0.16.0 to 0.20.4): AppKit's
/// guard against a window that keeps asking for another layout pass, thrown
/// from `NSHostingView.updateAnimatedWindowSize` inside `windowDidLayout`,
/// through `-[NSNextStepFrame setFrameSize:]` -- the frame view of a
/// borderless window. SwiftUI was resizing a borderless window to its
/// content from inside that window's own layout. #1402 took the edge
/// rail's sizing away from its hosting view and #1511 the menu-bar
/// panel's, but the rail still crashed (#1517: the unified log names
/// `EdgeRailPanel` at its closed 4 x 240 pt). Its panel now drops every
/// frame but its controller's, and grows before the rail opens and shrinks
/// after it closes (`EdgeRailControllerTests`). These pin the menu-bar panel.
///
/// There is no UI automation (no shell has any), so these pin the decision:
/// pure frame logic, and the source shapes that keep every window frame
/// write out of a layout pass.
@MainActor
final class WindowResizeLoopTests: XCTestCase {
    static let root = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("Sources")

    static func text(_ rel: String) throws -> String {
        try String(contentsOf: root.appendingPathComponent(rel), encoding: .utf8)
    }

    static let panelSource = "TraceCommonsApp/Views/Monitor/MenuBarGlassPanel.swift"
    static let appSource = "TraceCommonsApp/TraceCommonsAppMain.swift"
    static let threePaneSource = "TCDesign/Components/ThreePane.swift"
    static let monitorSource = "TraceCommonsApp/Views/MonitorWindowView.swift"
    static let railSource = "TraceCommonsApp/Views/EdgeRail/EdgeRailController.swift"
    static let railViewSource = "TraceCommonsApp/Views/EdgeRail/EdgeRailView.swift"

    // MARK: The menu-bar panel's frame

    /// The panel's frame comes from its content's size and the menu bar's
    /// bottom edge alone: the content's width and height, the leading edge
    /// kept, the top at the menu bar, the window extending downward.
    func test_thePanelFrameIsTheContentHeightUnderTheMenuBar() {
        let current = CGRect(x: 900, y: 400, width: 300, height: 350)
        let content = CGSize(width: 380, height: 520)
        XCTAssertEqual(
            MenuBarTopEdgePin.frame(current: current, content: content, top: 945),
            CGRect(x: 900, y: 425, width: 380, height: 520))
        // No top learned: the window keeps the top it has.
        XCTAssertEqual(
            MenuBarTopEdgePin.frame(current: current, content: content, top: nil),
            CGRect(x: 900, y: 230, width: 380, height: 520))
    }

    /// Whatever height the window has now, the frame it is given is the
    /// same: the window's own size never feeds back into the size it is
    /// set to, which is the loop AppKit aborted on.
    func test_thePanelFrameDoesNotDependOnTheWindowsOwnHeight() {
        let frames = [12, 350, 520, 2000].map { side in
            MenuBarTopEdgePin.frame(
                current: CGRect(x: 900, y: 100, width: CGFloat(side), height: CGFloat(side)),
                content: CGSize(width: 380, height: 520), top: 945)
        }
        XCTAssertEqual(Set(frames.compactMap { $0.map { "\($0)" } }).count, 1)
        XCTAssertEqual(frames.first ?? nil, CGRect(x: 900, y: 425, width: 380, height: 520))
    }

    /// A panel already at its frame is left alone, so a resize the sizer
    /// itself caused settles in one step instead of setting the frame again.
    func test_aPanelAlreadyInPlaceIsNotMoved() {
        let placed = CGRect(x: 900, y: 425, width: 380, height: 520)
        XCTAssertNil(MenuBarTopEdgePin.frame(current: placed, content: CGSize(width: 380, height: 520), top: 945))
        XCTAssertNil(MenuBarTopEdgePin.frame(
            current: placed, content: CGSize(width: 380.2, height: 520.3), top: 945.2))
        // Nothing measured yet: nothing to do.
        XCTAssertNil(MenuBarTopEdgePin.frame(current: placed, content: .zero, top: 945))
        XCTAssertNil(MenuBarTopEdgePin.frame(current: placed, content: CGSize(width: 380, height: 0), top: 945))
    }

    // MARK: The Monitor's frame after a pane change

    /// Opening the inspector grows the window by the inspector and its gap,
    /// closing it shrinks the window by the same; the leading and top edges
    /// stay. Computed from the old frame alone, never by measuring content.
    func test_aPaneChangeIsTheOldFramePlusOrMinusThePane() {
        let visible = CGRect(x: 0, y: 0, width: 3000, height: 1000)
        let old = CGRect(x: 100, y: 200, width: GlassPaneLayout.windowWidth(showsMap: true, showsInspector: false),
                         height: 700)
        let pane = GlassPaneLayout.inspectorWidth + GlassTokens.Space.paneGap
        let mapOnly = GlassPaneLayout.Visibility(map: true, inspector: false)
        let both = GlassPaneLayout.Visibility(map: true, inspector: true)
        let opened = GlassPaneLayout.windowFrame(
            onScreen: old, width: GlassPaneLayout.windowWidth(current: old.width, from: mapOnly, to: both),
            visible: visible)
        XCTAssertEqual(opened, CGRect(x: old.minX, y: old.minY, width: old.width + pane, height: old.height))
        let closed = GlassPaneLayout.windowFrame(
            onScreen: opened, width: GlassPaneLayout.windowWidth(current: opened.width, from: both, to: mapOnly),
            visible: visible)
        XCTAssertEqual(closed, old)
    }

    /// The macOS convention (owner, 2026-10-30, after Little Snitch's
    /// Network Monitor): with the window flush against the screen's right
    /// edge, opening the map and then the inspector each slides the window
    /// left by that pane and its gap, its right edge staying on the screen
    /// edge; closing the inspector and then the map each shrinks the window
    /// from its right, its leading edge staying where the opens put it.
    func test_nearTheScreenEdgeAPaneSlidesTheWindowLeftAndClosingShrinksItFromTheRight() {
        let screen = CGRect(x: 0, y: 0, width: 1920, height: 1080)
        let none = GlassPaneLayout.Visibility(map: false, inspector: false)
        let mapOnly = GlassPaneLayout.Visibility(map: true, inspector: false)
        let both = GlassPaneLayout.Visibility(map: true, inspector: true)
        let start = GlassPaneLayout.windowWidth(showsMap: false, showsInspector: false)
        let flush = CGRect(x: screen.maxX - start, y: 200, width: start, height: 700)
        func step(_ frame: CGRect, _ from: GlassPaneLayout.Visibility, _ to: GlassPaneLayout.Visibility) -> CGRect {
            GlassPaneLayout.windowFrame(
                onScreen: frame, width: GlassPaneLayout.windowWidth(current: frame.width, from: from, to: to),
                visible: screen)
        }
        let withMap = step(flush, none, mapOnly)
        XCTAssertGreaterThan(withMap.width, flush.width)
        XCTAssertEqual(withMap.maxX, screen.maxX)
        XCTAssertEqual(withMap.minX, flush.minX - (withMap.width - flush.width))
        let withBoth = step(withMap, mapOnly, both)
        XCTAssertEqual(withBoth.width, withMap.width + GlassPaneLayout.inspectorWidth + GlassTokens.Space.paneGap)
        XCTAssertEqual(withBoth.maxX, screen.maxX)
        XCTAssertEqual(withBoth.minX, withMap.minX - GlassPaneLayout.inspectorWidth - GlassTokens.Space.paneGap)
        let inspectorClosed = step(withBoth, both, mapOnly)
        XCTAssertEqual(inspectorClosed.minX, withBoth.minX)
        XCTAssertEqual(inspectorClosed.width, withMap.width)
        XCTAssertLessThan(inspectorClosed.maxX, screen.maxX)
        let mapClosed = step(inspectorClosed, mapOnly, none)
        XCTAssertEqual(mapClosed.minX, withBoth.minX)
        XCTAssertEqual(mapClosed.width, flush.width)
    }

    // MARK: Source pins

    /// The Monitor's scene takes only its minimum size from its content.
    /// The pane toggle sizes the window itself, on the next turn
    /// (`GlassThreePane.move(to:)`); SwiftUI resizing the window to its
    /// content as well is a second writer inside the layout pass.
    func test_theMonitorSceneTakesOnlyItsMinimumSizeFromItsContent() throws {
        let app = try Self.text(Self.appSource)
        let monitor = try XCTUnwrap(Self.block(after: "struct MonitorScene: Scene {", in: app))
        XCTAssertFalse(monitor.contains(".windowResizability(.contentSize)"))
        XCTAssertTrue(monitor.contains(".windowResizability(.contentMinSize)"))
    }

    /// A pane change and the launch sizing both take the on-screen rule, so
    /// a window is never left running past the screen's edge (owner,
    /// 2026-10-30, withdrawing "the window never moves on a pane change").
    func test_paneChangesAndLaunchKeepTheWindowOnScreen() throws {
        let pane = try Self.text(Self.threePaneSource)
        XCTAssertFalse(pane.contains("windowFrame(forPaneChange:"))
        XCTAssertFalse(pane.contains("windowFrame(atLaunch:"))
        let move = try XCTUnwrap(Self.block(after: "private func move(to new: GlassPaneLayout.Visibility)", in: pane))
        XCTAssertTrue(move.contains("GlassPaneLayout.windowFrame(onScreen:"))
        let launch = try XCTUnwrap(Self.block(after: "private func openAtKeptWidth(_ window: NSWindow)", in: pane))
        XCTAssertTrue(launch.contains("GlassPaneLayout.windowFrame(onScreen:"))
    }

    /// The menu-bar panel's hosting view takes #1402's empty sizing options
    /// and safe area, as the edge rail's does; the panel sizes it, from a content
    /// size that does not depend on the window, and takes the sizing over
    /// only on the next turn, after SwiftUI has first sized the window.
    func test_theMenuBarPanelTakesItsWindowSizingFromSwiftUI() throws {
        let panel = try Self.text(Self.panelSource)
        XCTAssertTrue(panel.contains("sizing.sizingOptions = []"))
        XCTAssertTrue(panel.contains("sizing.safeAreaRegions = []"))
        let moved = try XCTUnwrap(Self.block(after: "override func viewDidMoveToWindow()", in: panel.components(
            separatedBy: "struct MenuBarTopEdgePin: NSViewRepresentable {").last ?? ""))
        XCTAssertFalse(moved.contains("takeOverSizing"), "sizing is taken over before SwiftUI first sizes the window")
        let surface = try XCTUnwrap(Self.block(after: "private struct PanelSurface: ViewModifier {", in: panel))
        XCTAssertTrue(surface.contains(".fixedSize(horizontal: false, vertical: true)"))
        XCTAssertTrue(surface.contains(".onGeometryChange(for: CGSize.self, of: \\.size)"))
        XCTAssertTrue(surface.contains("MenuBarTopEdgePin(content:"))
    }

    /// No view sets a window's frame inside a layout callback: every
    /// `setFrame` is reached through `GlassWindowFrame.onNextTurn` (or an
    /// AppKit notification or controller method outside SwiftUI's update).
    func test_noViewSetsAWindowFrameInsideALayoutCallback() throws {
        for file in [Self.threePaneSource, Self.panelSource, Self.monitorSource] {
            let source = try Self.text(file)
            for marker in [".onGeometryChange(", ".onChange(", ".onPreferenceChange(", "func layout()",
                           "func viewDidLayout()", "func updateNSView("] {
                for body in Self.blocks(after: marker, in: source) {
                    for write in ["setFrame(", "setFrameOrigin(", "setContentSize(", ".animator().setFrame"] {
                        XCTAssertFalse(body.contains(write), "\(file): \(marker) calls \(write)")
                    }
                }
            }
        }
        let panel = try Self.text(Self.panelSource)
        let pin = try XCTUnwrap(Self.block(after: "struct MenuBarTopEdgePin: NSViewRepresentable {", in: panel))
        XCTAssertTrue(pin.contains("GlassWindowFrame.onNextTurn"))
    }

    /// Only the rail's controller opens and closes the rail
    /// (`EdgeRailController.transition(to:)`), which grows the panel before
    /// the rail opens and shrinks it after the rail has closed (#1517). The
    /// view writing `state.open` itself, under its own animation, is the
    /// ordering that crashed.
    func test_theRailViewLeavesOpeningToItsController() throws {
        let view = try Self.text(Self.railViewSource)
        XCTAssertFalse(view.contains("state.open ="), "the rail's view opens or closes the rail itself")
        XCTAssertFalse(view.contains("withAnimation"), "the rail's view animates a change of the rail's size")
        let controller = try Self.text(Self.railSource)
        XCTAssertTrue(controller.contains("self?.opening.set($0, animation: $1)"), "the view only asks the controller")
        XCTAssertTrue(controller.contains("guard placing else { return }"), "the panel takes frames from its controller only")
    }

    /// The menu-bar panel's window is sized on the next turn from its
    /// content's measured size (#1511), the same structure as the edge
    /// rail's. What made the rail crash anyway (#1517) was a size change
    /// inside an animation: the window's frame then went through
    /// `NSHostingView.updateAnimatedWindowSize` in `windowDidLayout`. Nothing
    /// in the panel animates today; an animated expand added later must
    /// size the window around the animation, as the rail does, before this
    /// pin is changed.
    func test_theMenuBarPanelAnimatesNoSizeChange() throws {
        let panel = try Self.text(Self.panelSource)
        for animated in ["withAnimation", ".transition(", ".animation(", "matchedGeometryEffect"] {
            XCTAssertFalse(panel.contains(animated), "MenuBarGlassPanel.swift uses \(animated) (#1517)")
        }
    }

    // MARK: The breadcrumb

    /// Each window names itself in the unified log when its size changes,
    /// with a fixed label and nothing else of the person's.
    func test_everyWindowNamesItselfWhenItResizes() throws {
        XCTAssertEqual(
            WindowBreadcrumb.Scene.allCases.map(\.label),
            ["monitor", "menu-bar panel", "settings", "first run", "edge rail"])
        let app = try Self.text(Self.appSource)
        for scene in [".windowBreadcrumb(.monitor)", ".windowBreadcrumb(.settings)", ".windowBreadcrumb(.firstRun)"] {
            XCTAssertTrue(app.contains(scene), scene)
        }
        XCTAssertTrue(try Self.text(Self.panelSource).contains(".windowBreadcrumb(.menuBarPanel)"))
        XCTAssertTrue(try Self.text(Self.railSource).contains(".windowBreadcrumb(.edgeRail)"))
    }

    /// The breadcrumb logs the first resize of a burst, then once more if
    /// the burst runs on (the loop AppKit aborts on is hundreds of resizes
    /// in one turn), and is quiet for the rest of a drag.
    func test_theBreadcrumbLogsTheFirstResizeAndARunawayBurst() {
        var throttle = WindowBreadcrumb.Throttle()
        XCTAssertEqual(throttle.note(at: 10.0), .resized)
        for step in 1..<WindowBreadcrumb.Throttle.burst - 1 {
            XCTAssertNil(throttle.note(at: 10.0 + Double(step) * 0.001))
        }
        XCTAssertEqual(throttle.note(at: 10.05), .runaway)
        XCTAssertNil(throttle.note(at: 10.06))
        // A second later, a new burst.
        XCTAssertEqual(throttle.note(at: 11.5), .resized)
    }

    // MARK: Helpers

    /// The brace-balanced block that follows the first `marker`.
    static func block(after marker: String, in source: String) -> String? {
        blocks(after: marker, in: source).first
    }

    /// The brace-balanced block after each `marker`: from the first `{`
    /// from the marker's start (its own, when it ends in one) to its
    /// matching `}`.
    static func blocks(after marker: String, in source: String) -> [String] {
        var found: [String] = []
        var search = source.startIndex..<source.endIndex
        while let hit = source.range(of: marker, range: search) {
            guard let open = source[hit.lowerBound...].firstIndex(of: "{") else { break }
            var depth = 0
            var index = open
            var end: String.Index?
            while index < source.endIndex {
                let char = source[index]
                if char == "{" { depth += 1 }
                if char == "}" {
                    depth -= 1
                    if depth == 0 { end = index; break }
                }
                index = source.index(after: index)
            }
            guard let end else { break }
            found.append(String(source[open...end]))
            search = hit.upperBound..<source.endIndex
        }
        return found
    }
}
