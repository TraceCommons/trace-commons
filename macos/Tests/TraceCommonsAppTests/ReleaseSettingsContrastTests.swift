import AppKit
import SwiftUI
import TCDesign
import XCTest

@testable import TraceCommonsApp

/// The release main window stacks the glass Settings sections on its own
/// ground (#1229 review 2, item 6). The glass tokens are tested against the
/// glass grounds; this tests them against the ground the release window
/// actually paints, in both appearances, with every card, quiet card, well,
/// field and tag tint blended over it the way the sections draw them.
///
/// Floors are the repo's: 4.5:1 for text, 3:1 for a UI glyph (a status dot).
final class ReleaseSettingsContrastTests: XCTestCase {
    typealias RGB = (r: Double, g: Double, b: Double)

    enum Appearance: String, CaseIterable {
        case light, dark

        var name: NSAppearance.Name { self == .light ? .aqua : .darkAqua }

        func token(_ token: GlassRGBA) -> GlassRGBA { self == .light ? token.light : token.dark }
    }

    /// What the release Settings destination is drawn on, resolved in the
    /// appearance: `MainWindowView`'s `.tcScreen()` paints `TC.ground`, and
    /// the destination paints `ReleaseSettingsGround.fill` over it. With
    /// `glass: false` it is the bare TC ground, which is what the sections
    /// sat on before the fill.
    static func hostGround(_ appearance: Appearance, glass: Bool = true) -> RGB {
        let tc = resolve(TC.ground, in: appearance.name)
        return glass ? over(appearance.token(ReleaseSettingsGround.fill), tc) : tc
    }

    /// Every surface a Settings sentence sits on, over the host ground:
    /// the ground itself, each stop of a card (`GlassEyebrowCard`) and a
    /// quiet card (`GlassNotice`), and, with `wells`, a well and a field
    /// inside each: the well under the bio, measurement and
    /// Homebrew-command boxes, and `fieldFill` under typed text (the Tools
    /// port field, `ToolsSection`, and `GlassTextField` in `Controls`).
    /// Only text is drawn in a well or a field; no dot or tag is.
    static func grounds(_ appearance: Appearance, glass: Bool = true, wells: Bool = true) -> [(String, RGB)] {
        let host = hostGround(appearance, glass: glass)
        var grounds: [(String, RGB)] = [("host ground", host)]
        for (name, gradient) in [("card", GlassTokens.Gradient.cardFill), ("quiet card", GlassTokens.Gradient.cardFillQuiet)] {
            for stop in gradient.stops {
                let card = over(appearance.token(stop.color), host)
                grounds.append(("\(name) at \(stop.location)", card))
                if wells {
                    grounds.append(("well in \(name) at \(stop.location)", over(appearance.token(GlassTokens.Color.wellFill), card)))
                    grounds.append(("field in \(name) at \(stop.location)", over(appearance.token(GlassTokens.Color.fieldFill), card)))
                }
            }
        }
        return grounds
    }

    /// Text colours the sections draw, by the token behind them.
    static let texts: [(String, GlassRGBA)] = {
        let c = GlassTokens.Color.self
        return [
            ("textPrimary", c.textPrimary), ("textSecondary", c.textSecondary), ("textTertiary", c.textTertiary),
            ("textSecondaryHighContrast", c.textSecondaryHighContrast),
            ("textTertiaryHighContrast", c.textTertiaryHighContrast),
            ("accentText", c.purpleText),
            // A refusal's words in `GlassFlowNotice`, and every status's
            // text-safe colour.
            ("statusOnText", c.statusOnText), ("statusAskText", c.statusAskText),
            ("statusOutsideText", c.statusOutsideText),
        ]
    }()

    /// Status dots: `GlassStatusLabel` and `GlassNotice` titles, for every
    /// status a Settings section asks for.
    static let glyphs: [(String, GlassRGBA)] = {
        let c = GlassTokens.Color.self
        return [("statusOn", c.statusOn), ("statusAsk", c.statusAsk), ("statusOff", c.statusOff),
                ("statusOutside", c.statusOutside)]
    }()

    /// `GlassTag`'s ink on its own tint, for every tone a section uses.
    static let tags: [(String, GlassRGBA, GlassRGBA)] = {
        let c = GlassTokens.Color.self
        return [
            ("neutral", c.textSecondary, c.tintNeutral), ("on", c.statusOnText, c.tintOn),
            ("ask", c.statusAskText, c.tintAsk), ("outside", c.statusOutsideText, c.tintOutside),
            ("accent", c.purpleText, c.tintAccent),
        ]
    }()

    func test_settingsTextClearsTextContrastOnTheReleaseGround() {
        for appearance in Appearance.allCases {
            for (ground, rgb) in Self.grounds(appearance) {
                for (name, token) in Self.texts {
                    let ratio = Self.contrast(Self.solid(appearance.token(token)), rgb)
                    XCTAssertGreaterThanOrEqual(ratio, 4.5, "\(appearance) \(name) on \(ground): \(ratio)")
                }
            }
        }
    }

    func test_statusDotsClearTheGlyphFloorOnTheReleaseGround() {
        for appearance in Appearance.allCases {
            for (ground, rgb) in Self.grounds(appearance, wells: false) {
                for (name, token) in Self.glyphs {
                    let ratio = Self.contrast(Self.solid(appearance.token(token)), rgb)
                    XCTAssertGreaterThanOrEqual(ratio, 3, "\(appearance) \(name) on \(ground): \(ratio)")
                }
            }
        }
    }

    func test_tagInkClearsTextContrastOnItsTintOverTheReleaseGround() {
        for appearance in Appearance.allCases {
            for (ground, rgb) in Self.grounds(appearance, wells: false) {
                for (tone, ink, tint) in Self.tags {
                    let pill = Self.over(appearance.token(tint), rgb)
                    let ratio = Self.contrast(Self.solid(appearance.token(ink)), pill)
                    XCTAssertGreaterThanOrEqual(ratio, 4.5, "\(appearance) \(tone) tag over \(ground): \(ratio)")
                }
            }
        }
    }

    /// Why the destination paints a glass ground at all: on the bare TC
    /// ground the outside tag on a card falls under 4.5:1 in dark.
    func test_theBareTCGroundIsNotEnough() {
        let c = GlassTokens.Color.self
        let card = Self.over(GlassTokens.Gradient.cardFill.stops[0].color.dark, Self.hostGround(.dark, glass: false))
        let ratio = Self.contrast(Self.solid(c.statusOutsideText.dark), Self.over(c.tintOutside.dark, card))
        XCTAssertLessThan(ratio, 4.5, "dark outside tag on a card over TC.ground: \(ratio)")
    }

    // MARK: WCAG

    static func resolve(_ color: Color, in name: NSAppearance.Name) -> RGB {
        var rgb: RGB = (0, 0, 0)
        NSAppearance(named: name)!.performAsCurrentDrawingAppearance {
            let resolved = NSColor(color).usingColorSpace(.sRGB)!
            rgb = (Double(resolved.redComponent), Double(resolved.greenComponent), Double(resolved.blueComponent))
        }
        return rgb
    }

    static func solid(_ token: GlassRGBA) -> RGB { (token.red, token.green, token.blue) }

    static func over(_ top: GlassRGBA, _ under: RGB) -> RGB {
        let a = top.alpha
        return (top.red * a + under.r * (1 - a), top.green * a + under.g * (1 - a), top.blue * a + under.b * (1 - a))
    }

    static func contrast(_ a: RGB, _ b: RGB) -> Double {
        func linear(_ c: Double) -> Double { c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4) }
        func luminance(_ c: RGB) -> Double { 0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b) }
        let (x, y) = (luminance(a), luminance(b))
        return (max(x, y) + 0.05) / (min(x, y) + 0.05)
    }
}

/// The host choice the contrast test computes against: the release main
/// window's Settings destination paints `ReleaseSettingsGround.fill` under
/// the stacked sections, inside the window's `.tcScreen()`.
final class ReleaseSettingsHostTests: XCTestCase {
    func test_theReleaseSettingsDestinationPaintsTheGlassGround() throws {
        let source = try String(
            contentsOf: URL(fileURLWithPath: #filePath)
                .deletingLastPathComponent()  // TraceCommonsAppTests
                .deletingLastPathComponent()  // Tests
                .deletingLastPathComponent()  // macos
                .appendingPathComponent("Sources/TraceCommonsApp/Views/MainWindowView.swift"),
            encoding: .utf8)
        let destination = try XCTUnwrap(source.range(of: "private var traceDestination: some View {"))
        let start = try XCTUnwrap(source.range(of: "case .settings:", range: destination.upperBound..<source.endIndex))
        let end = try XCTUnwrap(source.range(of: "case .compute:", range: start.upperBound..<source.endIndex))
        let branch = String(source[start.upperBound..<end.lowerBound])
        XCTAssertTrue(branch.contains("GlassSettingsContent(navigation: navigation, section: $0)"))
        XCTAssertTrue(branch.contains(".background(ReleaseSettingsGround.fill.color)"),
                      "the release Settings sections sit on the TC ground again")
        XCTAssertEqual(branch.components(separatedBy: ".background(").count - 1, 1,
                       "a second ground in the Settings branch is one the contrast test never computed")
        XCTAssertTrue(source.contains(".tcScreen()"))
        XCTAssertEqual(ReleaseSettingsGround.fill, GlassTokens.Color.paneOpaque)
    }
}
