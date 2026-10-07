import XCTest

@testable import TCDesign

/// The tokens the #1146 design convergence added or changed.
final class ConvergenceTokenTests: XCTestCase {
    /// The watch track is statusOn in both appearances, as #1146's
    /// `--tc-watch-on` is `var(--tc-status-on)`.
    func test_theWatchTrackIsStatusOn() {
        XCTAssertEqual(GlassTokens.Color.watchOn.dark, GlassTokens.Color.statusOn.dark)
        XCTAssertEqual(GlassTokens.Color.watchOn.light, GlassTokens.Color.statusOn.light)
    }

    /// One scrim value: the modal's is the general one, merged at #1146's
    /// modal .40 (and lighter in light).
    func test_theModalScrimIsTheMergedScrim() {
        XCTAssertEqual(GlassTokens.Color.modalScrim, GlassTokens.Color.scrim)
        XCTAssertEqual(GlassTokens.Color.modalScrim.alpha, 0.4, accuracy: 0.0001)
    }

    func test_modalSizes() {
        XCTAssertEqual(GlassTokens.Size.modalWidth, 780)
        XCTAssertEqual(GlassTokens.Size.modalNarrowWidth, 450)
        XCTAssertEqual(GlassTokens.Size.modalScrimBlur, 8)
        XCTAssertEqual(GlassTokens.Space.modalInsetTop, 52)
        XCTAssertEqual(GlassTokens.Space.modalInset, 28)
    }

    /// One disabled dimming for every control, and #1146's off-row fade.
    func test_opacities() {
        XCTAssertEqual(GlassTokens.Opacity.disabled, 0.45)
        XCTAssertEqual(GlassTokens.Opacity.rowOff, 0.6)
        XCTAssertEqual(GlassTokens.Opacity.skeletonDim, 0.5)
    }

    func test_motion() {
        XCTAssertEqual(GlassTokens.Motion.spin, 0.8)
        XCTAssertEqual(GlassTokens.Motion.pulse, 1.6)
    }

    /// Every new hover and state fill has a light value, so none vanishes
    /// on a light pane.
    func test_newFillsHaveLightValues() {
        for token in [
            GlassTokens.Color.cardHover, GlassTokens.Color.rowHover, GlassTokens.Color.toolbarExpanded,
            GlassTokens.Color.switchKnobEdge, GlassTokens.Color.modalScrim,
        ] {
            XCTAssertNotNil(token.lightRGB ?? token.lightAlpha.map { _ in 0 })
        }
    }

    /// The toolbar glyph keeps text contrast on the expanded fill.
    func test_aGlyphOnTheExpandedFillClearsTextContrast() {
        let fill = GlassTokens.Color.toolbarExpanded
        XCTAssertGreaterThanOrEqual(SwitchContrastTests.contrast(GlassTokens.Color.textPrimary.dark, fill.dark), 4.5)
        XCTAssertGreaterThanOrEqual(SwitchContrastTests.contrast(GlassTokens.Color.textPrimary.light, fill.light), 4.5)
    }
}
