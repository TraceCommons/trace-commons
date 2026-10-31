import SwiftUI
import XCTest

@testable import TCDesign

/// R5, as the owner revised it on #1241, on 2026-10-08 and on 2026-10-15: the
/// main pane never changes width when another pane opens or closes; the map
/// and the inspector each open at a fixed default width (the inspector two
/// thirds of the main pane), and opening one grows
/// the window to the right by its width (closing shrinks it), so no other
/// pane changes width. Hidden panes reserve no space. The window can always
/// be made wider: the map takes the room, or without it the main pane.
final class ThreePaneTests: XCTestCase {
    private let padding = GlassTokens.Space.windowPadding
    private let gap = GlassTokens.Space.paneGap
    private let main = GlassPaneLayout.mainWidth
    private let mapWidth = GlassTokens.Size.mapWidth
    private let inspector = GlassPaneLayout.inspectorWidth
    private typealias V = GlassPaneLayout.Visibility

    private static let compositions: [V] = [
        V(map: false, inspector: false), V(map: true, inspector: false),
        V(map: false, inspector: true), V(map: true, inspector: true),
    ]

    @MainActor
    func test_theDefaultWidthsAreTheTokens() {
        XCTAssertEqual(main, 440)
        XCTAssertEqual(main, GlassTokens.Size.paneLeftWidth + 40)
        // The map opens as wide as the main pane (owner, 2026-10-10).
        XCTAssertEqual(mapWidth, 440)
        XCTAssertEqual(mapWidth, main)
        XCTAssertEqual(inspector, 293)
        XCTAssertEqual(GlassTokens.Size.mapMinWidth, 360)
        XCTAssertEqual(GlassThreePane<EmptyView, EmptyView, EmptyView>.defaultWidth,
                       padding * 2 + main + gap + mapWidth + gap + inspector)
        XCTAssertEqual(GlassThreePane<EmptyView, EmptyView, EmptyView>.defaultWidth, 1193)
        XCTAssertEqual(GlassTokens.Size.windowHeight, 614)
    }

    /// At its default width every composition is its panes, gaps and
    /// padding exactly, each pane at its own default width.
    func test_everyCompositionAtItsDefaultWidthIsItsPanes() {
        for v in Self.compositions {
            let width = GlassPaneLayout.windowWidth(showsMap: v.map, showsInspector: v.inspector)
            let layout = GlassPaneLayout(windowWidth: width, showsMap: v.map, showsInspector: v.inspector)
            XCTAssertEqual(layout.main, main, "\(v)")
            XCTAssertEqual(layout.map, v.map ? mapWidth : nil, "\(v)")
            XCTAssertEqual(layout.inspector, v.inspector ? inspector : nil, "\(v)")
            let panes = [layout.main, layout.map, layout.inspector].compactMap { $0 }
            XCTAssertEqual(panes.reduce(0, +) + gap * CGFloat(panes.count - 1) + padding * 2, width, "\(v)")
            XCTAssertEqual(layout.visibility, v)
        }
    }

    /// The owner's rule: from any composition, opening or closing the map
    /// or the inspector changes the window by that pane and its gap, and
    /// every other pane keeps its width.
    func test_openingOrClosingAPaneResizesTheWindowNotTheOtherPanes() {
        for old in Self.compositions {
            for new in Self.compositions where new != old {
                let before = GlassPaneLayout.windowWidth(showsMap: old.map, showsInspector: old.inspector)
                let after = GlassPaneLayout.windowWidth(current: before, from: old, to: new)
                var expected = before
                if old.map != new.map { expected += (new.map ? 1 : -1) * (mapWidth + gap) }
                if old.inspector != new.inspector { expected += (new.inspector ? 1 : -1) * (inspector + gap) }
                XCTAssertEqual(after, expected, "\(old) -> \(new)")
                let a = GlassPaneLayout(windowWidth: before, showsMap: old.map, showsInspector: old.inspector)
                let b = GlassPaneLayout(windowWidth: after, showsMap: new.map, showsInspector: new.inspector)
                XCTAssertEqual(a.main, b.main, "the main pane moved: \(old) -> \(new)")
                if old.map && new.map { XCTAssertEqual(a.map, b.map, "the map moved: \(old) -> \(new)") }
                if old.inspector && new.inspector {
                    XCTAssertEqual(a.inspector, b.inspector, "the inspector moved: \(old) -> \(new)")
                }
            }
        }
    }

    /// The map is the one pane a resize of the window reaches; a map the
    /// person widened keeps its width when the inspector opens or closes,
    /// and closing it takes the whole of that width back.
    func test_aWidenedMapKeepsItsWidth() {
        let both = V(map: true, inspector: true)
        let mapOnly = V(map: true, inspector: false)
        let widened: CGFloat = 1500
        let before = GlassPaneLayout(windowWidth: widened, showsMap: true, showsInspector: true)
        XCTAssertEqual(before.main, main)
        XCTAssertEqual(before.inspector, inspector)
        XCTAssertEqual(before.map, widened - padding * 2 - main - gap - gap - inspector)
        let closed = GlassPaneLayout.windowWidth(current: widened, from: both, to: mapOnly)
        XCTAssertEqual(GlassPaneLayout(windowWidth: closed, showsMap: true, showsInspector: false).map, before.map)
        let noMap = GlassPaneLayout.windowWidth(current: widened, from: both, to: V(map: false, inspector: true))
        XCTAssertEqual(noMap, GlassPaneLayout.windowWidth(showsMap: false, showsInspector: true))
    }

    /// The map opens at the width the person last left it at (owner,
    /// 2026-10-19): the window at launch, and the map when it reopens, are
    /// sized to it, never below the map's minimum; and that width is what
    /// the panes then lay the map out at.
    func test_theMapOpensAtItsKeptWidth() {
        let kept: CGFloat = 620
        for inspectorShown in [false, true] {
            let width = GlassPaneLayout.windowWidth(showsMap: true, showsInspector: inspectorShown, mapWidth: kept)
            let layout = GlassPaneLayout(windowWidth: width, showsMap: true, showsInspector: inspectorShown)
            XCTAssertEqual(layout.map, kept, "inspector \(inspectorShown)")
            XCTAssertEqual(layout.main, main, "inspector \(inspectorShown)")
        }
        XCTAssertEqual(GlassPaneLayout.windowWidth(showsMap: true, showsInspector: false, mapWidth: 100),
                       GlassPaneLayout.minimumWindowWidth(showsMap: true, showsInspector: false))
        XCTAssertEqual(GlassPaneLayout.windowWidth(showsMap: false, showsInspector: true, mapWidth: kept),
                       GlassPaneLayout.windowWidth(showsMap: false, showsInspector: true), "no map, no map width")
        let closed = GlassPaneLayout.windowWidth(showsMap: false, showsInspector: true)
        let reopened = GlassPaneLayout.windowWidth(
            current: closed, from: V(map: false, inspector: true), to: V(map: true, inspector: true), mapWidth: kept)
        XCTAssertEqual(GlassPaneLayout(windowWidth: reopened, showsMap: true, showsInspector: true).map, kept)
    }

    /// Every composition has a floor and no ceiling (owner, 2026-10-15: the
    /// window can always be made wider). Without the map, the main pane
    /// takes a wider window; with it, the map does.
    func test_theWindowsLimitsFollowTheShownPanes() {
        XCTAssertEqual(GlassPaneLayout.minimumWindowWidth(showsMap: false, showsInspector: false), padding * 2 + main)
        XCTAssertEqual(GlassPaneLayout.minimumWindowWidth(showsMap: true, showsInspector: true),
                       padding * 2 + main + gap + GlassTokens.Size.mapMinWidth + gap + inspector)
        XCTAssertEqual(GlassPaneLayout.minimumWindowWidth, padding * 2 + main)
        XCTAssertEqual(GlassTokens.Size.windowMinHeight, 560)
        for v in Self.compositions {
            let limits = GlassPaneLayout.windowWidthLimits(.init(from: v, to: v, mapWidth: nil))
            XCTAssertEqual(limits.upperBound, .infinity, "\(v)")
        }
        // A window resized below the map's floor never draws a negative map.
        XCTAssertEqual(GlassPaneLayout(windowWidth: 100, showsMap: true, showsInspector: true).map, 0)
        // Wider than its panes without a map, the main pane takes the room.
        XCTAssertEqual(GlassPaneLayout(windowWidth: 1000, showsMap: false, showsInspector: false).main, 1000 - padding * 2)
        XCTAssertEqual(GlassPaneLayout(windowWidth: 1000, showsMap: false, showsInspector: true).main,
                       1000 - padding * 2 - gap - inspector)
        // Never below its default, however narrow the window.
        XCTAssertEqual(GlassPaneLayout(windowWidth: 100, showsMap: false, showsInspector: false).main, main)
    }

    /// A main pane the person widened keeps its width when the map or the
    /// inspector opens or closes: the window grows or shrinks by that pane
    /// and its gap, as at the default width.
    func test_aWidenedMainPaneKeepsItsWidth() {
        let widenedMain: CGFloat = 700
        for old in Self.compositions where !old.map {
            let start = padding * 2 + widenedMain + (old.inspector ? gap + inspector : 0)
            let before = GlassPaneLayout(windowWidth: start, showsMap: false, showsInspector: old.inspector)
            XCTAssertEqual(before.main, widenedMain, "\(old)")
            for new in Self.compositions where new != old {
                let end = GlassPaneLayout.windowWidth(current: start, from: old, to: new, main: before.main)
                var expected = start
                if new.map { expected += mapWidth + gap }
                if old.inspector != new.inspector { expected += (new.inspector ? 1 : -1) * (inspector + gap) }
                XCTAssertEqual(end, expected, "\(old) -> \(new)")
                let after = GlassPaneLayout(
                    windowWidth: end, showsMap: new.map, showsInspector: new.inspector, main: before.main)
                XCTAssertEqual(after.main, widenedMain, "the main pane moved: \(old) -> \(new)")
                if new.map { XCTAssertEqual(after.map, mapWidth, "\(old) -> \(new)") }
                // And back again, to the width it started at.
                let back = GlassPaneLayout.windowWidth(current: end, from: new, to: old, main: after.main)
                XCTAssertEqual(back, start, "\(new) -> \(old)")
            }
        }
    }

    /// Beside the map, a narrowing window takes the map down to its floor
    /// first, then the main pane down to its default; widening gives the
    /// main pane its width back first.
    func test_beforeTheMainPaneGivesWayTheMapReachesItsFloor() {
        let widenedMain: CGFloat = 700
        let mapMin = GlassTokens.Size.mapMinWidth
        func layout(_ width: CGFloat) -> GlassPaneLayout {
            GlassPaneLayout(windowWidth: width, showsMap: true, showsInspector: false, main: widenedMain)
        }
        let roomy = layout(padding * 2 + widenedMain + gap + 500)
        XCTAssertEqual(roomy.main, widenedMain)
        XCTAssertEqual(roomy.map, 500)
        let atFloor = layout(padding * 2 + widenedMain + gap + mapMin)
        XCTAssertEqual(atFloor.main, widenedMain)
        XCTAssertEqual(atFloor.map, mapMin)
        let squeezed = layout(padding * 2 + 600 + gap + mapMin)
        XCTAssertEqual(squeezed.main, 600)
        XCTAssertEqual(squeezed.map, mapMin)
        let narrowest = layout(GlassPaneLayout.minimumWindowWidth(showsMap: true, showsInspector: false))
        XCTAssertEqual(narrowest.main, main)
        XCTAssertEqual(narrowest.map, mapMin)
        // A main pane at its default never grows beside the map.
        XCTAssertEqual(GlassPaneLayout(windowWidth: 2000, showsMap: true, showsInspector: false).main, main)
    }

    /// The on-screen rule: the window grows on its right with its top and
    /// leading edge fixed; near the screen's edge it moves left only as far
    /// as it must; wider than the screen it is held to the screen.
    func test_theOnScreenRuleGrowsRightAndStaysOnScreen() {
        let screen = CGRect(x: 0, y: 0, width: 1920, height: 1080)
        let frame = CGRect(x: 100, y: 200, width: 690, height: 760)
        XCTAssertEqual(GlassPaneLayout.windowFrame(onScreen: frame, width: 1280, visible: screen),
                       CGRect(x: 100, y: 200, width: 1280, height: 760))
        let nearEdge = CGRect(x: 1000, y: 200, width: 690, height: 760)
        XCTAssertEqual(GlassPaneLayout.windowFrame(onScreen: nearEdge, width: 1280, visible: screen),
                       CGRect(x: 640, y: 200, width: 1280, height: 760))
        XCTAssertEqual(GlassPaneLayout.windowFrame(onScreen: nearEdge, width: 2400, visible: screen),
                       CGRect(x: 0, y: 200, width: 1920, height: 760))
        // Shrinking keeps the leading edge.
        XCTAssertEqual(GlassPaneLayout.windowFrame(onScreen: nearEdge, width: 380, visible: screen),
                       CGRect(x: 1000, y: 200, width: 380, height: 760))
    }

    /// Opening or closing a pane never moves the window and never caps its
    /// width: near the screen's right edge it grows past the edge, even
    /// when the new pane lands wholly off screen (owner, 2026-10-30).
    func test_aPaneChangeNeverMovesTheWindow() {
        let nearEdge = CGRect(x: 1000, y: 200, width: 690, height: 760)
        XCTAssertEqual(GlassPaneLayout.windowFrame(forPaneChange: nearEdge, width: 1280),
                       CGRect(x: 1000, y: 200, width: 1280, height: 760))
        // Wider than a 1920pt screen: not held to it, so no pane is squeezed.
        XCTAssertEqual(GlassPaneLayout.windowFrame(forPaneChange: nearEdge, width: 2400),
                       CGRect(x: 1000, y: 200, width: 2400, height: 760))
        XCTAssertEqual(GlassPaneLayout.windowFrame(forPaneChange: nearEdge, width: 380),
                       CGRect(x: 1000, y: 200, width: 380, height: 760))
    }

    /// A window the person left running past the screen's edge opens where
    /// it was, so closing the pane afterwards brings it back to where it
    /// started. A window whose leading edge is not on this screen, or that
    /// is wider than the screen, takes the on-screen rule.
    func test_aLaunchKeepsTheWindowWhereThePersonLeftIt() {
        let screen = CGRect(x: 0, y: 0, width: 1920, height: 1080)
        let pastEdge = CGRect(x: 1000, y: 200, width: 1280, height: 760)
        let opened = GlassPaneLayout.windowFrame(atLaunch: pastEdge, width: 1280, visible: screen)
        XCTAssertEqual(opened, pastEdge)
        XCTAssertEqual(GlassPaneLayout.windowFrame(forPaneChange: opened, width: 690),
                       CGRect(x: 1000, y: 200, width: 690, height: 760))
        let offThisScreen = CGRect(x: 2000, y: 200, width: 690, height: 760)
        XCTAssertEqual(GlassPaneLayout.windowFrame(atLaunch: offThisScreen, width: 690, visible: screen),
                       CGRect(x: 1230, y: 200, width: 690, height: 760))
        XCTAssertEqual(GlassPaneLayout.windowFrame(atLaunch: pastEdge, width: 2400, visible: screen),
                       CGRect(x: 0, y: 200, width: 1920, height: 760))
    }

    func test_theTrafficLightsHaveTheirClearance() {
        XCTAssertGreaterThanOrEqual(GlassTokens.Space.windowControlsWidth, 78)
        XCTAssertEqual(EnvironmentValues().glassWindowControlsInset, 0)
    }

    /// A window's first layout seeds the preferences from the room it has,
    /// as #1146 decides at launch: the map from 1100pt, the inspector from 900.
    func test_theFirstLaunchSeedsPanesFromTheWidth() {
        XCTAssertTrue(GlassPaneLayout.firstLaunch(windowWidth: 1320) == (true, true))
        XCTAssertTrue(GlassPaneLayout.firstLaunch(windowWidth: 1100) == (true, true))
        XCTAssertTrue(GlassPaneLayout.firstLaunch(windowWidth: 1099) == (false, true))
        XCTAssertTrue(GlassPaneLayout.firstLaunch(windowWidth: 900) == (false, true))
        XCTAssertTrue(GlassPaneLayout.firstLaunch(windowWidth: 899) == (false, false))
    }

    /// Panes animate when one shows or hides, not on every width of a live
    /// resize: the animation keys on visibility, which a resize inside one
    /// composition leaves alone.
    func test_resizingWithinACompositionKeepsTheVisibility() {
        let narrow = GlassPaneLayout(windowWidth: 1200, showsMap: true, showsInspector: true)
        let wide = GlassPaneLayout(windowWidth: 1500, showsMap: true, showsInspector: true)
        XCTAssertNotEqual(narrow, wide)
        XCTAssertEqual(narrow.visibility, wide.visibility)
        let hidden = GlassPaneLayout(windowWidth: 1500, showsMap: false, showsInspector: true)
        XCTAssertNotEqual(hidden.visibility, wide.visibility)
    }

    /// While the window moves between compositions, every pane of either is
    /// drawn on the leading edge: the main pane never moves or changes width,
    /// a map that stays keeps its width, and a map that opens or closes takes
    /// exactly the room past the other panes, from nothing at one end to its
    /// width at the other. At the default width and at a widened one.
    func test_aMoveKeepsThePanesOnTheLeadingEdge() {
        for startMain in [main, 700] {
            for old in Self.compositions {
                for new in Self.compositions where new != old {
                    let start = GlassPaneLayout.windowWidth(showsMap: old.map, showsInspector: old.inspector)
                        + (startMain - main)
                    let before = GlassPaneLayout(
                        windowWidth: start, showsMap: old.map, showsInspector: old.inspector, main: startMain)
                    XCTAssertEqual(before.main, startMain, "\(old) -> \(new)")
                    let end = GlassPaneLayout.windowWidth(current: start, from: old, to: new, main: before.main)
                    let move = GlassPaneLayout.Transition(
                        from: old, to: new, mapWidth: before.map, mainWidth: before.main)
                    for width in [start, (start + end) / 2, end] {
                        let layout = GlassPaneLayout(windowWidth: width, transition: move)
                        XCTAssertEqual(layout.main, startMain, "\(old) -> \(new) at \(width)")
                        XCTAssertEqual(layout.map != nil, old.map || new.map, "\(old) -> \(new)")
                        XCTAssertEqual(layout.inspector != nil, old.inspector || new.inspector, "\(old) -> \(new)")
                        XCTAssertGreaterThanOrEqual(layout.map ?? 0, 0)
                        if old.map && new.map { XCTAssertEqual(layout.map, before.map, "\(old) -> \(new)") }
                    }
                    // Only the map moving: with its gap it fills the room past
                    // the fixed panes, nothing at the closed end and its width at
                    // the open end.
                    guard old.map != new.map, old.inspector == new.inspector else { continue }
                    let closed = GlassPaneLayout(windowWidth: old.map ? end : start, transition: move)
                    let open = GlassPaneLayout(windowWidth: old.map ? start : end, transition: move)
                    XCTAssertEqual((closed.map ?? 0) + closed.mapGap, 0, "\(old) -> \(new)")
                    XCTAssertEqual(open.map, mapWidth, "\(old) -> \(new)")
                    XCTAssertEqual(open.mapGap, gap, "\(old) -> \(new)")
                }
            }
        }
    }

    /// The window's limits during a move admit both ends, so neither is
    /// clamped while the window animates between them.
    func test_aMovesLimitsAdmitBothEnds() {
        for old in Self.compositions {
            for new in Self.compositions where new != old {
                let limits = GlassPaneLayout.windowWidthLimits(.init(from: old, to: new, mapWidth: nil))
                let start = GlassPaneLayout.windowWidth(showsMap: old.map, showsInspector: old.inspector)
                let end = GlassPaneLayout.windowWidth(current: start, from: old, to: new)
                XCTAssertTrue(limits.contains(start), "\(old) -> \(new)")
                XCTAssertTrue(limits.contains(end), "\(old) -> \(new)")
            }
        }
    }

    /// The divider beside the map (owner, 2026-10-20) moves room between
    /// the main pane and the map: dragged right the main pane widens and
    /// the map narrows, never under its minimum; dragged left the main pane
    /// stops at its default. The window keeps its width.
    func test_theDividerMovesRoomBetweenTheMainPaneAndTheMap() {
        for inspector in [false, true] {
            let window = GlassPaneLayout.windowWidth(showsMap: true, showsInspector: inspector) + 200
            let wider = GlassPaneLayout.draggedMain(from: main, by: 120, windowWidth: window, showsInspector: inspector)
            XCTAssertEqual(wider, main + 120)
            let layout = GlassPaneLayout(windowWidth: window, showsMap: true, showsInspector: inspector, main: wider)
            XCTAssertEqual(layout.main + (layout.map ?? 0), main + mapWidth + 200, "the window keeps its width")
            let widest = GlassPaneLayout.draggedMain(from: main, by: 10_000, windowWidth: window, showsInspector: inspector)
            let squeezed = GlassPaneLayout(windowWidth: window, showsMap: true, showsInspector: inspector, main: widest)
            XCTAssertEqual(squeezed.map, GlassTokens.Size.mapMinWidth)
            XCTAssertEqual(GlassPaneLayout.draggedMain(from: main, by: -500, windowWidth: window, showsInspector: inspector), main)
        }
    }

    /// A drag on the divider resizes the panes and never moves the window
    /// (Kristi, 2026-10-21): the window is movable by its background, so
    /// the handle under the gap must refuse to move it. It reports the drag
    /// from where it began, and takes the pointer a little into each pane.
    @MainActor
    func test_theDividerHandleNeverMovesTheWindow() throws {
        let handle = GlassDividerHandleView(frame: CGRect(x: 0, y: 0, width: 18, height: 400))
        XCTAssertFalse(handle.mouseDownCanMoveWindow)
        XCTAssertTrue(handle.acceptsFirstMouse(for: nil))
        XCTAssertGreaterThan(GlassPaneLayout.dividerReach, 0)
        XCTAssertLessThan(GlassPaneLayout.dividerReach, gap)

        var dragged: [CGFloat] = []
        var ended: CGFloat?
        handle.onDrag = { dragged.append($0) }
        handle.onEnd = { ended = $0 }
        func event(_ type: NSEvent.EventType, x: CGFloat) throws -> NSEvent {
            try XCTUnwrap(NSEvent.mouseEvent(
                with: type, location: CGPoint(x: x, y: 10), modifierFlags: [], timestamp: 0,
                windowNumber: 0, context: nil, eventNumber: 0, clickCount: 1, pressure: 1))
        }
        handle.mouseDown(with: try event(.leftMouseDown, x: 400))
        handle.mouseDragged(with: try event(.leftMouseDragged, x: 430))
        handle.mouseDragged(with: try event(.leftMouseDragged, x: 380))
        handle.mouseUp(with: try event(.leftMouseUp, x: 460))
        XCTAssertEqual(dragged, [30, -20])
        XCTAssertEqual(ended, 60)

        // A bare click, the one that activates the window included, ends
        // no drag, so it never writes the width the layout happens to show.
        ended = nil
        handle.mouseDown(with: try event(.leftMouseDown, x: 400))
        handle.mouseUp(with: try event(.leftMouseUp, x: 400))
        XCTAssertNil(ended)
        handle.mouseDown(with: try event(.leftMouseDown, x: 400))
        handle.mouseUp(with: try event(.leftMouseUp, x: 402))
        XCTAssertNil(ended, "only a drag that moved ends one")
    }

    /// The handle reaches into the map only, never over the main pane's
    /// scroller beside the gap, and the pane ends a drag only when one
    /// began.
    @MainActor
    func test_theDividerLeavesTheMainPanesEdgeAlone() throws {
        let source = try String(contentsOf: URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TCDesign/Components/ThreePane.swift"), encoding: .utf8)
        XCTAssertTrue(source.contains(".padding(.trailing, -GlassPaneLayout.dividerReach)"))
        XCTAssertFalse(source.contains(".padding(.horizontal, -GlassPaneLayout.dividerReach)"))
        XCTAssertTrue(source.contains("guard let start = dragStart else { return }"))
    }

    /// A window that opens with a main pane the divider widened is that
    /// much wider, so the map keeps its kept width.
    func test_theKeptMainWidthWidensTheOpeningWindow() {
        let base = GlassPaneLayout.windowWidth(showsMap: true, showsInspector: true)
        XCTAssertEqual(GlassPaneLayout.windowWidth(showsMap: true, showsInspector: true, main: main + 80), base + 80)
        XCTAssertEqual(GlassPaneLayout.windowWidth(showsMap: true, showsInspector: true, main: main - 80), base)
    }
}
