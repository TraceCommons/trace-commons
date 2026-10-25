import SwiftUI
import XCTest

@testable import TCDesign

/// A control or card takes the fill that stands out from what it sits on
/// (`GlassGround`, owner, 2026-10-22): the flat theme's control fill, a
/// dark well, all but vanished on a menu (about 1.04:1), a quiet card on a
/// quiet card matched it (1.02:1), and a selected row's pill and kebab sat
/// at 2:1 and 1.2:1 with a white label at 3.5:1. In the flat theme no
/// control or card draws an edge, so the fill is all that sets it apart.
final class GroundContrastTests: XCTestCase {
    private typealias Flat = FlatThemeContrastTests
    static let themes = ["flatDark", "flatLight"]

    // MARK: What each tier is to what it holds

    func test_eachTierNamesItsGround() {
        XCTAssertEqual(GlassTier.pane.ground, .pane)
        for tier in [GlassTier.card, .cardQuiet, .well, .nodeCard] { XCTAssertEqual(tier.ground, .card) }
        XCTAssertEqual(GlassTier.menu.ground, .menu)
        XCTAssertEqual(GlassTier.popover.ground, .menu)
        XCTAssertNil(GlassTier.control.ground, "a control holds only its label")
        XCTAssertNil(GlassTier.controlSelected.ground)
    }

    /// A control's hover follows its fill onto a menu or a selection; any
    /// other hover is the caller's and stays.
    func test_aControlsHoverFollowsItsGround() {
        let hover = GlassTokens.Color.controlHover
        XCTAssertEqual(GlassGround.pane.hover(hover, tier: .control), hover)
        XCTAssertEqual(GlassGround.card.hover(hover, tier: .control), hover)
        XCTAssertEqual(GlassGround.menu.hover(hover, tier: .control), GlassTokens.Color.controlOnMenuHover)
        XCTAssertEqual(GlassGround.selection.hover(hover, tier: .control), GlassTokens.Color.controlOnSelectionHover)
        let card = GlassTokens.Color.cardHover
        XCTAssertEqual(GlassGround.menu.hover(card, tier: .card), card)
        XCTAssertEqual(GlassGround.menu.hover(GlassTokens.Color.menuHover, tier: .control), GlassTokens.Color.menuHover)
    }

    /// On a selected row a control's label, the submit pill's and the
    /// kebab's glyph take the row's own ink, whatever their state.
    @MainActor
    func test_onASelectionControlsTakeTheRowsInk() {
        let rowInk = GlassTokens.Color.textOnAccent
        XCTAssertEqual(GlassGround.selection.controlInk, rowInk)
        XCTAssertEqual(GlassGround.menu.controlInk, GlassTokens.Color.textPrimary)
        for done in [true, false] {
            for enabled in [true, false] {
                XCTAssertEqual(GlassButtonStyle.submitInk(done: done, enabled: enabled, ground: .selection), rowInk)
            }
        }
        XCTAssertEqual(GlassButtonStyle.submitInk(done: true, enabled: true), GlassTokens.Color.statusOnText)
        XCTAssertEqual(GlassKebab.ink(lit: false, ground: .selection), rowInk)
        XCTAssertEqual(GlassKebab.ink(lit: false, ground: .pane), GlassTokens.Color.statusOff)
        XCTAssertEqual(GlassKebab.ink(lit: true, ground: .pane), GlassTokens.Color.textPrimary)
    }

    /// The tier modifier tells what it holds its ground and draws its own
    /// fill for the ground it sits on; a selected list row is the
    /// selection to its controls; the hover fill follows the ground too.
    func test_theGroundIsPassedDown() throws {
        let sources = Dictionary(uniqueKeysWithValues: try DesignSources.all())
        let style = try XCTUnwrap(sources["GlassStyle.swift"])
        XCTAssertTrue(style.contains(".environment(\\.glassGround, tier.ground ?? ground)"))
        XCTAssertTrue(style.contains("tier.fill(in: shape, on: ground)"))
        XCTAssertTrue(style.contains("let hover = hover.map { ground.hover($0, tier: tier) }"))
        let row = try XCTUnwrap(sources["ListRow.swift"])
        XCTAssertTrue(row.contains(".transformEnvironment(\\.glassGround) { if selected { $0 = .selection } }"))
        let hover = try XCTUnwrap(sources["GlassHover.swift"])
        XCTAssertTrue(hover.contains(".fill(ground.hover(fill, tier: .control).color)"))
    }

    // MARK: Contrast, from the token values

    /// A control on a menu (the Traces menus' pickers, a popover's buttons)
    /// stands out from the menu, more than the well did, and under the
    /// pointer more again; its label clears 4.5:1 on it.
    func test_aControlOnAMenuStandsOut() throws {
        for theme in Self.themes {
            let menu = Flat.over(try Flat.colour("paneOpaque", theme: theme).rgb, try Flat.colour("menuFill", theme: theme))
            let control = Flat.over(menu, try Flat.colour("controlOnMenu", theme: theme))
            let hover = Flat.over(menu, try Flat.colour("controlOnMenuHover", theme: theme))
            let well = Flat.over(menu, try Flat.gradientMax("controlFill", theme: theme))
            let text = try Flat.colour("textPrimary", theme: theme)
            XCTAssertGreaterThanOrEqual(Flat.contrast(control, menu), 1.15, theme)
            XCTAssertGreaterThan(Flat.contrast(control, menu), Flat.contrast(well, menu) + 0.05, theme)
            XCTAssertGreaterThan(Flat.contrast(hover, menu), Flat.contrast(control, menu), theme)
            for fill in [control, hover] {
                XCTAssertGreaterThanOrEqual(Flat.contrast(Flat.over(fill, text), fill), 4.5, theme)
            }
        }
    }

    /// A control on a selected row is a deeper pill on the selection, and
    /// the row's ink clears 4.5:1 on it, at rest and under the pointer.
    func test_aControlOnASelectionStandsOutAndItsLabelReads() throws {
        for theme in Self.themes {
            let selection = try Flat.colour("selection", theme: theme).rgb
            let ink = try Flat.colour("textOnAccent", theme: theme)
            let pill = Flat.over(selection, try Flat.colour("controlOnSelection", theme: theme))
            let hover = Flat.over(selection, try Flat.colour("controlOnSelectionHover", theme: theme))
            XCTAssertGreaterThanOrEqual(Flat.contrast(pill, selection), 1.25, theme)
            XCTAssertGreaterThan(Flat.contrast(hover, selection), Flat.contrast(pill, selection), theme)
            for fill in [pill, hover] {
                XCTAssertGreaterThanOrEqual(Flat.contrast(Flat.over(fill, ink), fill), 4.5, theme)
            }
            // The row's ink on the selection itself, where the kebab's glyph sits.
            XCTAssertGreaterThanOrEqual(Flat.contrast(Flat.over(selection, ink), selection), 4.5, theme)
        }
    }

    /// A card on a card (the session preview's prompt quote) differs from
    /// the card under it, where the same fill again did not.
    func test_aCardOnACardStandsOut() throws {
        for theme in Self.themes {
            let pane = Flat.over(try Flat.colour("paneOpaque", theme: theme).rgb, try Flat.colour("glassVeil", theme: theme))
            for outer in ["cardFill", "cardFillQuiet"] {
                let card = Flat.over(pane, try Flat.gradientMax(outer, theme: theme))
                let nested = Flat.over(card, try Flat.colour("cardNested", theme: theme))
                let same = Flat.over(card, try Flat.gradientMax(outer, theme: theme))
                XCTAssertGreaterThanOrEqual(Flat.contrast(nested, card), 1.08, "\(theme) \(outer)")
                XCTAssertGreaterThan(Flat.contrast(nested, card), Flat.contrast(same, card), "\(theme) \(outer)")
                let text = try Flat.colour("textPrimary", theme: theme)
                XCTAssertGreaterThanOrEqual(Flat.contrast(Flat.over(nested, text), nested), 4.5, "\(theme) \(outer)")
            }
        }
    }
}
