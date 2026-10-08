import SwiftUI
import XCTest

@testable import TCDesign

/// R5, as the owner revised it on #1241: the main pane is a compact fixed
/// width that never changes when another pane opens or closes; the map and
/// the inspector each open at a fixed default width, and opening one grows
/// the window to the right by its width (closing shrinks it), so no other
/// pane changes width. Hidden panes reserve no space.
final class ThreePaneTests: XCTestCase {
    private let padding = GlassTokens.Space.windowPadding
    private let gap = GlassTokens.Space.paneGap
    private let main = GlassTokens.Size.paneLeftCompactWidth
    private let mapWidth = GlassTokens.Size.mapWidth
    private let inspector = GlassTokens.Size.inspectorWidth
    private typealias V = GlassPaneLayout.Visibility

    private static let compositions: [V] = [
        V(map: false, inspector: false), V(map: true, inspector: false),
        V(map: false, inspector: true), V(map: true, inspector: true),
    ]

    func test_theDefaultWidthsAreTheTokens() {
        XCTAssertEqual(main, 360)
        XCTAssertEqual(mapWidth, 600)
        XCTAssertEqual(inspector, 300)
        XCTAssertEqual(GlassTokens.Size.mapMinWidth, 360)
        XCTAssertEqual(GlassThreePane<EmptyView, EmptyView, EmptyView>.defaultWidth,
                       padding * 2 + main + gap + mapWidth + gap + inspector)
        XCTAssertEqual(GlassThreePane<EmptyView, EmptyView, EmptyView>.defaultWidth, 1280)
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

    /// Without the map nothing takes extra width: the window is exactly its
    /// panes, and may not be dragged wider. With it, the map has a floor.
    func test_theWindowsLimitsFollowTheShownPanes() {
        XCTAssertEqual(GlassPaneLayout.maximumWindowWidth(showsMap: false, showsInspector: true),
                       padding * 2 + main + gap + inspector)
        XCTAssertEqual(GlassPaneLayout.minimumWindowWidth(showsMap: false, showsInspector: false), padding * 2 + main)
        XCTAssertEqual(GlassPaneLayout.maximumWindowWidth(showsMap: true, showsInspector: false), .infinity)
        XCTAssertEqual(GlassPaneLayout.minimumWindowWidth(showsMap: true, showsInspector: true),
                       padding * 2 + main + gap + GlassTokens.Size.mapMinWidth + gap + inspector)
        XCTAssertEqual(GlassPaneLayout.minimumWindowWidth, padding * 2 + main)
        XCTAssertEqual(GlassTokens.Size.windowMinHeight, 560)
        // A window resized below the map's floor never draws a negative map.
        XCTAssertEqual(GlassPaneLayout(windowWidth: 100, showsMap: true, showsInspector: true).map, 0)
        // Zoomed past its panes without a map, the main pane takes the room.
        XCTAssertEqual(GlassPaneLayout(windowWidth: 1000, showsMap: false, showsInspector: false).main, 1000 - padding * 2)
    }

    /// The window grows on its right with its top and leading edge fixed;
    /// near the screen's edge it moves left only as far as it must; wider
    /// than the screen it is held to the screen.
    func test_theWindowGrowsRightAndStaysOnScreen() {
        let screen = CGRect(x: 0, y: 0, width: 1920, height: 1080)
        let frame = CGRect(x: 100, y: 200, width: 690, height: 760)
        XCTAssertEqual(GlassPaneLayout.windowFrame(frame, width: 1280, visible: screen),
                       CGRect(x: 100, y: 200, width: 1280, height: 760))
        let nearEdge = CGRect(x: 1000, y: 200, width: 690, height: 760)
        XCTAssertEqual(GlassPaneLayout.windowFrame(nearEdge, width: 1280, visible: screen),
                       CGRect(x: 640, y: 200, width: 1280, height: 760))
        XCTAssertEqual(GlassPaneLayout.windowFrame(nearEdge, width: 2400, visible: screen),
                       CGRect(x: 0, y: 200, width: 1920, height: 760))
        // Shrinking keeps the leading edge.
        XCTAssertEqual(GlassPaneLayout.windowFrame(nearEdge, width: 380, visible: screen),
                       CGRect(x: 1000, y: 200, width: 380, height: 760))
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
}
