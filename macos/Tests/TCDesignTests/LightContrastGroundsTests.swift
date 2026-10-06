import XCTest

@testable import TCDesign

/// The #1206 review: light contrast is checked against every ground a
/// colour is drawn on, with translucent fills blended over what is under
/// them, not only the two easiest grounds. WCAG: text 4.5:1, UI 3:1.
final class LightContrastGroundsTests: XCTestCase {
    typealias RGB = (r: Double, g: Double, b: Double)

    /// Every light ground text sits on: the scene, panes, a well on a pane
    /// and on the scene, the map field, cards, popovers, menus and node
    /// cards, each blended over what is under it.
    static var lightGrounds: [(String, RGB)] {
        let c = GlassTokens.Color.self
        let scene = solid(c.sceneBase.light)
        let pane = solid(c.paneOpaque.light)
        let outer = solid(c.mapFieldOuter.light)
        return [
            ("sceneBase", scene),
            ("sceneWarm", solid(c.sceneWarm.light)),
            ("paneOpaque", pane),
            ("paneBase", over(c.paneBase.light, scene)),
            ("well on pane", over(c.wellFill.light, pane)),
            ("well on scene", over(c.wellFill.light, scene)),
            ("mapFieldOuter", outer),
            ("mapFieldInner", solid(c.mapFieldInner.light)),
            ("popover", over(c.popoverFill.light, scene)),
            ("menu", over(c.menuFill.light, scene)),
            ("node card", over(c.nodeCardFill.light, outer)),
        ]
    }

    /// Body, secondary, tertiary and accent text, their Increase Contrast
    /// values, and the text-safe status colours reach 4.5:1 on every light
    /// ground; the status text colours on their own tint too.
    func test_textReachesFourPointFiveOnEveryLightGround() {
        let c = GlassTokens.Color.self
        let texts: [(String, GlassRGBA)] = [
            ("textPrimary", c.textPrimary), ("textSecondary", c.textSecondary), ("textTertiary", c.textTertiary),
            ("purpleText", c.purpleText), ("textSecondaryHighContrast", c.textSecondaryHighContrast),
            ("textTertiaryHighContrast", c.textTertiaryHighContrast),
            ("statusOnText", c.statusOnText), ("statusAskText", c.statusAskText), ("statusOutsideText", c.statusOutsideText),
        ]
        for (name, text) in texts {
            for (ground, rgb) in Self.lightGrounds {
                XCTAssertGreaterThanOrEqual(Self.contrast(Self.solid(text.light), rgb), 4.5, "\(name) on \(ground)")
            }
        }
        let tinted: [(String, GlassRGBA, GlassRGBA)] = [
            ("statusOnText", c.statusOnText, c.tintOn), ("statusAskText", c.statusAskText, c.tintAsk),
            ("statusOutsideText", c.statusOutsideText, c.tintOutside),
        ]
        for (name, text, tint) in tinted {
            for (ground, rgb) in Self.lightGrounds {
                let pill = Self.over(tint.light, rgb)
                XCTAssertGreaterThanOrEqual(Self.contrast(Self.solid(text.light), pill), 4.5, "\(name) on its tint over \(ground)")
            }
        }
    }

    /// The on and ask text-safe colours are the status colours in dark.
    /// Outside is not: its text is lighter, because the status colour on
    /// its own tint over a card on the opaque pane was 3.61:1 (#1229).
    func test_theOnAndAskTextColoursAreTheStatusColoursInDark() {
        let c = GlassTokens.Color.self
        XCTAssertEqual(c.statusOnText.dark, c.statusOn.dark)
        XCTAssertEqual(c.statusAskText.dark, c.statusAsk.dark)
        XCTAssertEqual(c.badgeFill.dark, c.statusOutside.dark)
        XCTAssertEqual(c.menuHoverText.dark, c.textPrimary.dark)
    }

    /// The outside tag's text on its tint, over every card stop on the
    /// opaque pane, reaches 4.5:1 in dark: the tag is drawn at micro, so
    /// the large-text floor does not apply.
    func test_theDarkOutsideTagReachesFourPointFiveOnTheOpaquePane() {
        let c = GlassTokens.Color.self
        let pane = Self.solid(c.paneOpaque.dark)
        for gradient in [GlassTokens.Gradient.cardFill, GlassTokens.Gradient.cardFillQuiet] {
            for stop in gradient.stops {
                let pill = Self.over(c.tintOutside.dark, Self.over(stop.color.dark, pane))
                let ratio = Self.contrast(Self.solid(c.statusOutsideText.dark), pill)
                XCTAssertGreaterThanOrEqual(ratio, 4.5, "dark outside tag at \(stop.location): \(ratio)")
            }
        }
    }

    /// White on the call-to-action, secondary and checkbox fills reaches
    /// 4.5:1 at every stop in light (13pt bold is not large text).
    func test_whiteOnTheActionFillsReachesFourPointFive() {
        let white = Self.solid(GlassTokens.Color.textOnAccent.light)
        let fills: [(String, GlassGradient)] = [
            ("ctaFill", GlassTokens.Gradient.ctaFill), ("ctaSecondaryFill", GlassTokens.Gradient.ctaSecondaryFill),
            ("checkboxOnFill", GlassTokens.Gradient.checkboxOnFill),
        ]
        for (name, gradient) in fills {
            for stop in gradient.stops {
                XCTAssertNotNil(stop.color.lightRGB, "\(name) has a light value")
                for (ground, rgb) in Self.lightGrounds {
                    let fill = Self.over(stop.color.light, rgb)
                    XCTAssertGreaterThanOrEqual(Self.contrast(white, fill), 4.5, "white on \(name) at \(stop.location) over \(ground)")
                }
            }
        }
    }

    /// The badge's dark count on its fill reaches 4.5:1 in both
    /// appearances, and the fill is 3:1 on the pane in light.
    func test_theBadgeCountReachesFourPointFive() {
        let c = GlassTokens.Color.self
        for (name, fill, ink) in [("light", c.badgeFill.light, c.textOnStatus.light), ("dark", c.badgeFill.dark, c.textOnStatus.dark)] {
            XCTAssertGreaterThanOrEqual(Self.contrast(Self.solid(ink), Self.solid(fill)), 4.5, name)
        }
        XCTAssertGreaterThanOrEqual(Self.contrast(Self.solid(c.badgeFill.light), Self.solid(c.paneOpaque.light)), 3)
    }

    /// Menu hover in light: the item's text on the hover fill, and white on
    /// the menu-row selection, reach 4.5:1 over every menu ground.
    func test_menuHoverTextReachesFourPointFiveInLight() {
        let c = GlassTokens.Color.self
        let menus = Self.lightGrounds.filter { $0.0 == "menu" || $0.0 == "popover" }
        XCTAssertEqual(menus.count, 2)
        for (ground, rgb) in menus {
            let hover = Self.over(c.menuHover.light, rgb)
            XCTAssertGreaterThanOrEqual(Self.contrast(Self.solid(c.menuHoverText.light), hover), 4.5, "menu item on hover over \(ground)")
            let selection = Self.over(c.menuSelection.light, rgb)
            XCTAssertGreaterThanOrEqual(Self.contrast(Self.solid(c.textOnAccent.light), selection), 4.5, "menu row on selection over \(ground)")
        }
    }

    /// White on the menu-row selection reaches 4.5:1 in dark too: the
    /// selection is opaque, so it is checked on its own (white on the old
    /// #3a7bd5 was 4.22).
    func test_whiteOnTheMenuSelectionReachesFourPointFiveInDark() {
        let c = GlassTokens.Color.self
        XCTAssertEqual(c.menuSelection.dark.alpha, 1)
        XCTAssertGreaterThanOrEqual(Self.contrast(Self.solid(c.textOnAccent.dark), Self.solid(c.menuSelection.dark)), 4.5)
    }

    /// Under Increase Contrast, the painted edge and the hairline reach the
    /// 3:1 non-text floor on every light ground.
    func test_increaseContrastStrokesReachThreeToOneInLight() {
        let c = GlassTokens.Color.self
        for (name, stroke) in [("edgeHighContrast", c.edgeHighContrast), ("hairlineHighContrast", c.hairlineHighContrast)] {
            for (ground, rgb) in Self.lightGrounds {
                XCTAssertGreaterThanOrEqual(Self.contrast(Self.over(stroke.light, rgb), rgb), 3, "\(name) on \(ground)")
            }
        }
    }

    // MARK: Helpers

    static func solid(_ c: GlassRGBA) -> RGB { (c.red, c.green, c.blue) }

    /// `fg` at its alpha over an opaque `bg`.
    static func over(_ fg: GlassRGBA, _ bg: RGB) -> RGB {
        let a = fg.alpha
        return (fg.red * a + bg.r * (1 - a), fg.green * a + bg.g * (1 - a), fg.blue * a + bg.b * (1 - a))
    }

    static func luminance(_ c: RGB) -> Double {
        func channel(_ v: Double) -> Double { v <= 0.03928 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4) }
        return 0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
    }

    static func contrast(_ a: RGB, _ b: RGB) -> Double {
        let (x, y) = (luminance(a), luminance(b))
        return (max(x, y) + 0.05) / (min(x, y) + 0.05)
    }
}
