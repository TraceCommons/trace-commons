import SwiftUI
import XCTest

@testable import TCDesign

/// R5: the window's pane layout (spec, "Panes"). The main pane always
/// shows; the map and the inspector hide independently; the map also hides
/// below 1100pt; hidden panes reserve no space.
final class ThreePaneTests: XCTestCase {
    private let padding = GlassTokens.Space.windowPadding
    private let gap = GlassTokens.Space.paneGap

    /// Every composition fills the window exactly: panes, gaps and padding.
    func test_everyCompositionFillsTheWindow() {
        for width in [760.0, 900, 1099, 1100, 1320, 1600] {
            for map in [true, false] {
                for inspector in [true, false] {
                    let layout = GlassPaneLayout(windowWidth: width, showsMap: map, showsInspector: inspector)
                    let panes = [layout.main, layout.map, layout.inspector].compactMap { $0 }
                    let total = panes.reduce(0, +) + gap * CGFloat(panes.count - 1) + padding * 2
                    XCTAssertEqual(total, width, accuracy: 0.001, "\(width) map:\(map) inspector:\(inspector)")
                }
            }
        }
    }

    /// The main pane is min(400, max(320, 0.34 × width)) beside the map.
    func test_theMainPaneWidthFollowsTheFormula() {
        XCTAssertEqual(GlassPaneLayout(windowWidth: 1320, showsMap: true, showsInspector: true).main, 400)
        XCTAssertEqual(GlassPaneLayout(windowWidth: 1100, showsMap: true, showsInspector: true).main, 374, accuracy: 0.001)
        XCTAssertEqual(GlassPaneLayout(windowWidth: 1600, showsMap: true, showsInspector: false).main, 400)
    }

    /// Below 1100pt the map hides whatever the preference; at 1100 it shows.
    func test_theMapHidesBelowTheBreakpoint() {
        XCTAssertNil(GlassPaneLayout(windowWidth: 1099, showsMap: true, showsInspector: true).map)
        XCTAssertNotNil(GlassPaneLayout(windowWidth: 1100, showsMap: true, showsInspector: true).map)
        XCTAssertNil(GlassPaneLayout(windowWidth: 1600, showsMap: false, showsInspector: true).map, "an explicit hide stays hidden")
    }

    /// With the map hidden, the main pane fills the window less the inspector.
    func test_theMainPaneFillsWhenTheMapIsHidden() {
        let layout = GlassPaneLayout(windowWidth: 1000, showsMap: false, showsInspector: true)
        XCTAssertEqual(layout.main, 1000 - padding * 2 - gap - GlassTokens.Size.inspectorWidth)
        XCTAssertEqual(GlassPaneLayout(windowWidth: 1000, showsMap: false, showsInspector: false).main, 1000 - padding * 2)
    }

    /// The window's minimum is the spec's 760×560, and every composition fits
    /// it without a pane going below its minimum.
    func test_theMinimumWindowFitsEveryComposition() {
        XCTAssertEqual(GlassPaneLayout.minimumWindowWidth, 760)
        XCTAssertEqual(GlassTokens.Size.windowMinHeight, 560)
        let tightest = GlassPaneLayout(windowWidth: 760, showsMap: true, showsInspector: true)
        XCTAssertGreaterThanOrEqual(tightest.main, GlassTokens.Size.paneLeftMinWidth)
    }

    func test_theTrafficLightsHaveTheirClearance() {
        XCTAssertGreaterThanOrEqual(GlassTokens.Space.windowControlsWidth, 78)
        XCTAssertEqual(EnvironmentValues().glassWindowControlsInset, 0)
    }

    /// A window's first layout seeds the preferences from its width, as
    /// #1146 decides at launch: the map from 1100pt, the inspector from 900.
    func test_theFirstLaunchSeedsPanesFromTheWidth() {
        XCTAssertTrue(GlassPaneLayout.firstLaunch(windowWidth: 1320) == (true, true))
        XCTAssertTrue(GlassPaneLayout.firstLaunch(windowWidth: 1100) == (true, true))
        XCTAssertTrue(GlassPaneLayout.firstLaunch(windowWidth: 1099) == (false, true))
        XCTAssertTrue(GlassPaneLayout.firstLaunch(windowWidth: 900) == (false, true))
        XCTAssertTrue(GlassPaneLayout.firstLaunch(windowWidth: 899) == (false, false))
    }

    /// The map toggle knows when the window, not the person, hid the map.
    func test_theMapIsCompactedOnlyWhenWantedAndTooNarrow() {
        XCTAssertTrue(GlassPaneLayout.compactsMap(windowWidth: 1099, showsMap: true))
        XCTAssertFalse(GlassPaneLayout.compactsMap(windowWidth: 1100, showsMap: true))
        XCTAssertFalse(GlassPaneLayout.compactsMap(windowWidth: 900, showsMap: false))
        XCTAssertFalse(EnvironmentValues().glassMapCompacted)
    }

    /// Panes animate when one shows or hides, not on every width of a live
    /// resize: the animation keys on visibility, which a resize inside one
    /// composition leaves alone.
    func test_resizingWithinACompositionKeepsTheVisibility() {
        let narrow = GlassPaneLayout(windowWidth: 1200, showsMap: true, showsInspector: true)
        let wide = GlassPaneLayout(windowWidth: 1500, showsMap: true, showsInspector: true)
        XCTAssertNotEqual(narrow, wide)
        XCTAssertEqual(narrow.visibility, wide.visibility)
        let compact = GlassPaneLayout(windowWidth: 1000, showsMap: true, showsInspector: true)
        XCTAssertNotEqual(compact.visibility, wide.visibility)
    }
}
