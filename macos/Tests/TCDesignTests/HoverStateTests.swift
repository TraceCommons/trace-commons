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

    /// Every button kind dims by the one shared disabled opacity but the
    /// submit pill, which #1146 dims to 0.55; checkboxes and radios dim to
    /// 0.7 (#1146 `.tc-btn--submit:disabled`, `.tc-checkbox:disabled`).
    func test_everyKindDimsByTheSharedDisabledOpacity() {
        for kind: GlassButtonKind in [.primary, .secondary, .glass, .link, .destructive] {
            XCTAssertEqual(GlassButtonStyle.disabledOpacity(kind), GlassTokens.Opacity.disabled, "\(kind)")
        }
        XCTAssertEqual(GlassTokens.Opacity.disabled, 0.45)
        XCTAssertEqual(GlassButtonStyle.disabledOpacity(.submit(done: false)), 0.55)
        XCTAssertEqual(GlassTokens.Opacity.disabledSubmit, 0.55)
        XCTAssertEqual(GlassTokens.Opacity.disabledCheck, 0.7)
    }

    /// A hover fill replaces the tier's own fill, as #1146's `:hover`
    /// background does, and never shows on a disabled control.
    func test_aHoverFillReplacesTheTierFill() {
        let hover = GlassTokens.Color.controlHover
        XCTAssertEqual(GlassTierFill.choose(hover: hover, hovering: true, enabled: true), .hover(hover))
        XCTAssertEqual(GlassTierFill.choose(hover: hover, hovering: false, enabled: true), .tier)
        XCTAssertEqual(GlassTierFill.choose(hover: hover, hovering: true, enabled: false), .tier)
        XCTAssertEqual(GlassTierFill.choose(hover: nil, hovering: true, enabled: true), .tier)
        // #1146's values: 16% white on a control, 10% on an interactive card.
        XCTAssertEqual(hover.alpha, 0.16, accuracy: 0.0001)
        XCTAssertEqual(GlassTokens.Color.cardHover.alpha, 0.1, accuracy: 0.0001)
    }

    /// The disabled dimming of checkboxes and radios is wired.
    func test_checkboxesAndRadiosDimByTheirOwnOpacity() throws {
        let sources = Dictionary(uniqueKeysWithValues: try DesignSources.components())
        let controls = try XCTUnwrap(sources["Controls.swift"])
        let checkbox = try XCTUnwrap(controls.range(of: "struct GlassCheckboxStyle").map { String(controls[$0.lowerBound...].prefix(1500)) })
        XCTAssertTrue(checkbox.contains("GlassPressStyle(disabledOpacity: GlassTokens.Opacity.disabledCheck)"))
        let forms = try XCTUnwrap(sources["Forms.swift"])
        let radio = try XCTUnwrap(forms.range(of: "struct GlassRadioGroup").map { String(forms[$0.lowerBound...].prefix(3000)) })
        XCTAssertTrue(radio.contains("GlassPressStyle(disabledOpacity: GlassTokens.Opacity.disabledCheck)"))
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
        for name in ["struct GlassSegmentedTabs", "struct GlassRuleTabs", "struct GlassBreadcrumb"] {
            let body = try XCTUnwrap(navigation.range(of: name).map { String(navigation[$0.lowerBound...].prefix(5000)) })
            XCTAssertTrue(body.contains(".onHover"), "\(name) has no hover")
        }
        let controls = try XCTUnwrap(sources["Controls.swift"])
        // A control with a fill takes the hover in its place; the toolbar
        // glyph has no fill, so its hover is a fill of its own.
        for name in ["struct GlassRoundButton", "struct GlassPillIconButton", "struct GlassPickerPill", "struct GlassFolderButton"] {
            let body = try XCTUnwrap(controls.range(of: name).map { String(controls[$0.lowerBound...].prefix(2000)) })
            XCTAssertTrue(body.contains("hover: GlassTokens.Color.controlHover"), "\(name) has no hover")
        }
        let toolbar = try XCTUnwrap(controls.range(of: "struct GlassToolbarButton").map { String(controls[$0.lowerBound...].prefix(2500)) })
        XCTAssertTrue(toolbar.contains(".glassHover(GlassTokens.Color.controlHover"), "the toolbar button has no hover")
        let buttons = try XCTUnwrap(controls.range(of: "private struct GlassButtonBody").map { String(controls[$0.lowerBound...].prefix(5000)) })
        XCTAssertFalse(buttons.contains("Capsule().fill(hoverFill)"), "a glass button's hover stacks on its fill")
        let containers = try XCTUnwrap(sources["Containers.swift"])
        XCTAssertTrue(containers.contains("hover: Self.hoverFill(interactive: interactive)"))
        let panel = try XCTUnwrap(sources["MenuBarPanel.swift"])
        XCTAssertFalse(panel.contains("0.55"), "the option row dims twice")
    }
}
