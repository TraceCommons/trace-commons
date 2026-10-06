import SwiftUI
import XCTest

@testable import TCDesign

/// The hover, selected, expanded and disabled states taken from #1146.
final class HoverStateTests: XCTestCase {
    func test_aHoverFillShowsOnlyUnderThePointerAndEnabled() {
        XCTAssertTrue(GlassHover.shows(hovering: true, enabled: true))
        XCTAssertFalse(GlassHover.shows(hovering: true, enabled: false))
        XCTAssertFalse(GlassHover.shows(hovering: false, enabled: true))
    }

    /// The glass pill and the destructive pill take the control hover; a
    /// selected glass button keeps its purple; the CTAs, the submit pill and
    /// the link have none (the link underlines).
    func test_buttonHoverFills() {
        XCTAssertEqual(GlassButtonStyle.hoverFill(.glass), GlassTokens.Color.controlHover)
        XCTAssertEqual(GlassButtonStyle.hoverFill(.destructive), GlassTokens.Color.controlHover)
        XCTAssertNil(GlassButtonStyle.hoverFill(.glass, selected: true))
        for kind: GlassButtonKind in [.primary, .secondary, .submit(done: false), .submit(done: true), .link] {
            XCTAssertNil(GlassButtonStyle.hoverFill(kind), "\(kind)")
        }
    }

    /// Every button kind dims by the one shared disabled opacity.
    func test_everyKindDimsByTheSharedDisabledOpacity() {
        for kind: GlassButtonKind in [.primary, .secondary, .glass, .submit(done: false), .link, .destructive] {
            XCTAssertEqual(GlassButtonStyle.disabledOpacity(kind), GlassTokens.Opacity.disabled, "\(kind)")
        }
    }

    /// The submit pill keeps its statusOff ink when it cannot be used.
    func test_theSubmitInk() {
        XCTAssertEqual(GlassButtonStyle.submitInk(done: false, enabled: false), GlassTokens.Color.statusOff)
        XCTAssertEqual(GlassButtonStyle.submitInk(done: false, enabled: true), GlassTokens.Color.textPrimary)
        XCTAssertEqual(GlassButtonStyle.submitInk(done: true, enabled: true), GlassTokens.Color.statusOnText)
    }

    /// The destructive label clears text contrast on the control fill over
    /// the pane, in both appearances.
    func test_theDestructiveLabelClearsTextContrast() {
        let fill = GlassTokens.Gradient.controlFill.stops[0].color
        let ink = GlassButtonStyle.destructiveInk
        let dark = ToolbarGlyphContrastTests.over(fill.dark, GlassTokens.Color.paneBase.dark)
        XCTAssertGreaterThanOrEqual(SwitchContrastTests.contrast(ink.dark, dark), 4.5)
        let light = ToolbarGlyphContrastTests.over(fill.light, GlassTokens.Color.paneOpaque.light)
        XCTAssertGreaterThanOrEqual(SwitchContrastTests.contrast(ink.light, light), 4.5)
    }

    func test_theToolbarButtonsExpandedFill() {
        XCTAssertEqual(GlassToolbarButton.fill(expanded: true), GlassTokens.Color.toolbarExpanded)
        XCTAssertNil(GlassToolbarButton.fill(expanded: false))
        XCTAssertNil(GlassToolbarButton.fill(expanded: nil))
    }

    func test_theKebabLightsOpenOrUnderThePointer() {
        XCTAssertTrue(GlassKebab.lit(open: true, hovering: false))
        XCTAssertTrue(GlassKebab.lit(open: false, hovering: true))
        XCTAssertFalse(GlassKebab.lit(open: false, hovering: false))
    }

    /// A row's hover never draws over its selection.
    func test_aListRowsHoverNeverCoversTheSelection() {
        XCTAssertEqual(GlassListRow.fill(selected: true, hovering: true), GlassTokens.Color.selection)
        XCTAssertEqual(GlassListRow.fill(selected: false, hovering: true), GlassTokens.Color.rowHover)
        XCTAssertNil(GlassListRow.fill(selected: false, hovering: false))
    }

    func test_onlyAnInteractiveCardLifts() {
        XCTAssertEqual(GlassCard<EmptyView>.hoverFill(interactive: true), GlassTokens.Color.cardHover)
        XCTAssertNil(GlassCard<EmptyView>.hoverFill(interactive: false))
    }

    func test_theNoticeTitleSitsSixPointsOverItsBody() {
        XCTAssertEqual(GlassNotice<EmptyView>.titleGap, 6)
    }

    /// The pointer reaches the tabs, the crumbs, the round, pill-icon and
    /// toolbar buttons, and the picker pill; the option row dims by the
    /// shared opacity alone.
    func test_hoverIsWiredWhereTheDesignAsksForIt() throws {
        let sources = Dictionary(uniqueKeysWithValues: try DesignSources.components())
        let navigation = try XCTUnwrap(sources["Navigation.swift"])
        for name in ["struct GlassSegmentedTabs", "struct GlassBreadcrumb"] {
            let body = try XCTUnwrap(navigation.range(of: name).map { String(navigation[$0.lowerBound...].prefix(5000)) })
            XCTAssertTrue(body.contains(".onHover"), "\(name) has no hover")
        }
        let controls = try XCTUnwrap(sources["Controls.swift"])
        for name in ["struct GlassRoundButton", "struct GlassPillIconButton", "struct GlassToolbarButton", "struct GlassPickerPill"] {
            let body = try XCTUnwrap(controls.range(of: name).map { String(controls[$0.lowerBound...].prefix(1500)) })
            XCTAssertTrue(body.contains(".glassHover("), "\(name) has no hover")
        }
        let panel = try XCTUnwrap(sources["MenuBarPanel.swift"])
        XCTAssertFalse(panel.contains("0.55"), "the option row dims twice")
    }
}
