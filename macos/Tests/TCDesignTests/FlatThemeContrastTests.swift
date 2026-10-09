import Foundation
import XCTest

/// The flat glass theme's text contrast, from the token values
/// (design-tokens/glass.tokens.json, `themes`).
///
/// A flat pane is see-through, so what sits behind its text is whatever
/// desktop is behind the window. The worst case is a desktop the glass
/// passes unchanged that is the opposite of the text: pure white under
/// Dark's white text, pure black under Light's dark text. These tests put
/// the pane's layers over it -- the veil, then the pane's white fill -- and
/// a card's or a field's fill over that, and require every text colour to
/// clear WCAG's 4.5:1 on each ground. A real desktop is less extreme and the
/// glass frosts it, so on screen the margin is larger (measured 2026-10-09).
///
/// Composited in sRGB, as the window server composites the layers. The
/// theme is read from the JSON rather than the generated tokens, which
/// resolve for the theme the test process launched in (classic).
final class FlatThemeContrastTests: XCTestCase {
    /// Text drawn on a flat pane, a card or a field.
    static let textTokens = [
        "textPrimary", "textSecondary", "textTertiary", "placeholder", "purpleText",
        "statusOnText", "statusAskText", "statusOutsideText", "destructiveText",
    ]
    static let floor = 4.5

    func test_flatLightTextClearsTheFloorOverABlackDesktop() throws {
        try assertTextClearsTheFloor(in: "flatLight")
    }

    func test_flatDarkTextClearsTheFloorOverAWhiteDesktop() throws {
        try assertTextClearsTheFloor(in: "flatDark")
    }

    /// The two appearances differ: a veil that matched would make the
    /// appearance choice a no-op.
    func test_theTwoAppearancesHaveDifferentVeils() throws {
        let light = try Self.colour("glassVeil", theme: "flatLight")
        let dark = try Self.colour("glassVeil", theme: "flatDark")
        XCTAssertNotEqual(light.rgb, dark.rgb)
    }

    private func assertTextClearsTheFloor(in theme: String) throws {
        let grounds = try Self.grounds(theme)
        for token in Self.textTokens {
            let text = try Self.colour(token, theme: theme)
            for (name, ground) in grounds {
                let ratio = Self.contrast(Self.over(ground, text), ground)
                XCTAssertGreaterThanOrEqual(
                    ratio, Self.floor,
                    "\(theme): \(token) on \(name) is \(String(format: "%.2f", ratio)):1 over the worst desktop")
            }
        }
        // A selected fill carries its own text.
        let selected = try Self.colour("controlSelected", theme: theme)
        let onSelected = try Self.colour("textOnSelected", theme: theme)
        for (name, ground) in grounds {
            let fill = Self.over(ground, selected)
            let ratio = Self.contrast(Self.over(fill, onSelected), fill)
            XCTAssertGreaterThanOrEqual(ratio, Self.floor, "\(theme): textOnSelected on a selection over \(name)")
        }
    }

    // MARK: Grounds

    /// The pane over the worst desktop for the theme's text, and a card, a
    /// quiet card and a field on it. A gradient fill is taken at its most
    /// opaque stop.
    static func grounds(_ theme: String) throws -> [(String, RGB)] {
        let desktop = theme == "flatLight" ? RGB(r: 0, g: 0, b: 0) : RGB(r: 1, g: 1, b: 1)
        var pane = over(desktop, try colour("glassVeil", theme: theme))
        pane = over(pane, try gradientMax("paneFill", theme: theme))
        return [
            ("pane", pane),
            ("card", over(pane, try gradientMax("cardFill", theme: theme))),
            ("quiet card", over(pane, try gradientMax("cardFillQuiet", theme: theme))),
            ("field", over(pane, try colour("fieldFill", theme: theme))),
        ]
    }

    // MARK: Token values

    struct RGB: Equatable { var r, g, b: Double }
    struct RGBA { var rgb: RGB; var alpha: Double }

    nonisolated(unsafe) private static let source: [String: Any] = {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
            .appending(path: "design-tokens/glass.tokens.json")
        guard
            let data = try? Data(contentsOf: url),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { fatalError("cannot read \(url.path)") }
        return object
    }()

    /// A token's entry in the theme, or else its classic value for the
    /// theme's appearance, which is what the flat theme keeps for a token it
    /// leaves alone: the light value under flatLight, the dark one under
    /// flatDark.
    private static func entry(_ group: String, _ key: String, theme: String) throws -> Any {
        let themes = source["themes"] as? [String: Any]
        if let themed = ((themes?[theme] as? [String: Any])?[group] as? [String: Any])?[key] {
            return themed
        }
        let classic = try XCTUnwrap((source[group] as? [String: Any])?[key], "no \(group).\(key)")
        if theme == "flatLight", let light = (classic as? [String: Any])?["light"] {
            return light
        }
        return classic
    }

    static func colour(_ key: String, theme: String) throws -> RGBA {
        try parse(XCTUnwrap(entry("color", key, theme: theme) as? [String: Any]))
    }

    static func gradientMax(_ key: String, theme: String) throws -> RGBA {
        let gradient = try XCTUnwrap(entry("gradient", key, theme: theme) as? [String: Any])
        let stops = try XCTUnwrap(gradient["stops"] as? [[String: Any]])
        return try XCTUnwrap(stops.map(parse).max { $0.alpha < $1.alpha })
    }

    private static func parse(_ entry: [String: Any]) throws -> RGBA {
        let hex = try XCTUnwrap(entry["hex"] as? String)
        let value = try XCTUnwrap(UInt32(hex.dropFirst(), radix: 16))
        return RGBA(
            rgb: RGB(
                r: Double((value >> 16) & 0xFF) / 255,
                g: Double((value >> 8) & 0xFF) / 255,
                b: Double(value & 0xFF) / 255),
            alpha: (entry["alpha"] as? NSNumber)?.doubleValue ?? 1)
    }

    // MARK: Colour arithmetic

    static func over(_ ground: RGB, _ layer: RGBA) -> RGB {
        let a = layer.alpha
        return RGB(
            r: layer.rgb.r * a + ground.r * (1 - a),
            g: layer.rgb.g * a + ground.g * (1 - a),
            b: layer.rgb.b * a + ground.b * (1 - a))
    }

    static func luminance(_ c: RGB) -> Double {
        func linear(_ v: Double) -> Double { v <= 0.04045 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4) }
        return 0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
    }

    static func contrast(_ a: RGB, _ b: RGB) -> Double {
        let (x, y) = (luminance(a), luminance(b))
        return (max(x, y) + 0.05) / (min(x, y) + 0.05)
    }
}
