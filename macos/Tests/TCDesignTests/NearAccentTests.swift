import Foundation
import XCTest

/// The flat theme's accent is the NEAR AI design system's (owner,
/// 2026-10-20: "Apply the Near AI primary theme accent color from the Parent
/// Design System repo to the app, replacing the purple"), and purple stays
/// for data. Read from the token JSON, as `FlatThemeContrastTests` reads it,
/// because the generated tokens resolve for the classic theme under a test
/// run.
final class NearAccentTests: XCTestCase {
    /// The parent's roles (near-ai-os-design-system, `tokens/near-ai.tokens.json`),
    /// light then dark, and the tokens that carry each here.
    static let roles: [(role: String, light: UInt32, dark: UInt32, tokens: [String])] = [
        ("accent", 0x035CA3, 0x7EBCFF,
         ["controlSelected", "controlSelectedFloating", "toggleOn", "toggleOnSettings", "selection",
          "menuSelection"]),
        ("accent-text", 0x024175, 0xC6E1FF, ["purpleText", "linkText"]),
        ("accent-soft", 0x0070C6, 0x0091FD, ["purpleSoft", "tabEmber"]),
    ]

    func test_theAccentIsTheNearAIAccent() throws {
        for (role, light, dark, tokens) in Self.roles {
            for token in tokens {
                XCTAssertEqual(try Self.hex(token, theme: "flatLight"), light, "\(token) is \(role) in Light")
                XCTAssertEqual(try Self.hex(token, theme: "flatDark"), dark, "\(token) is \(role) in Dark")
            }
        }
        // The parent's text-on-accent: white on Light's deep blue, navy on
        // Dark's light blue.
        XCTAssertEqual(try Self.hex("textOnAccent", theme: "flatLight"), 0xFFFFFF)
        XCTAssertEqual(try Self.hex("textOnAccent", theme: "flatDark"), 0x071A2E)
    }

    /// A label on an accent fill clears 4.5:1; a glyph, a knob or a check
    /// clears 3:1. Each fill is laid over every flat ground, so the menu's
    /// translucent hover is measured as it is drawn.
    func test_theInkOnEveryAccentFillClearsItsFloor() throws {
        let pairs: [(fill: String, gradient: Bool, ink: String, floor: Double)] = [
            ("ctaFill", true, "textOnAccent", 4.5),
            ("ctaSecondaryFill", true, "textOnAccent", 4.5),
            ("selection", false, "textOnAccent", 4.5),
            ("controlSelected", false, "textOnSelected", 4.5),
            ("controlSelectedFloating", false, "textOnSelected", 4.5),
            ("menuSelection", false, "menuHoverText", 4.5),
            ("menuHover", false, "menuHoverText", 4.5),
            ("toolbarExpanded", false, "textOnSelected", 3),
            ("toggleOn", false, "textOnAccent", 3),
            ("toggleOnSettings", false, "textOnAccent", 3),
            ("checkboxOnFill", true, "textOnAccent", 3),
        ]
        for theme in ["flatLight", "flatDark"] {
            for (name, ground) in try FlatThemeContrastTests.grounds(theme) {
                for (fill, gradient, ink, floor) in pairs {
                    let layer = gradient
                        ? try FlatThemeContrastTests.gradientMax(fill, theme: theme)
                        : try FlatThemeContrastTests.colour(fill, theme: theme)
                    let under = FlatThemeContrastTests.over(ground, layer)
                    let over = FlatThemeContrastTests.over(under, try FlatThemeContrastTests.colour(ink, theme: theme))
                    let ratio = FlatThemeContrastTests.contrast(over, under)
                    XCTAssertGreaterThanOrEqual(
                        ratio, floor, "\(theme): \(ink) on \(fill) over \(name) is \(String(format: "%.2f", ratio)):1")
                }
            }
        }
    }

    /// The accent's text and links (accent-text, as the parent's link
    /// button) read on every flat ground.
    func test_theAccentTextClearsTheTextFloor() throws {
        for theme in ["flatLight", "flatDark"] {
            for (name, ground) in try FlatThemeContrastTests.grounds(theme) {
                for token in ["purpleText", "linkText"] {
                    let ink = FlatThemeContrastTests.over(ground, try FlatThemeContrastTests.colour(token, theme: theme))
                    let ratio = FlatThemeContrastTests.contrast(ink, ground)
                    XCTAssertGreaterThanOrEqual(ratio, 4.5, "\(theme): \(token) on \(name)")
                }
            }
        }
    }

    /// Purple is gone from the shipping theme (owner, 2026-10-20: "there
    /// are still remnants of the purple color in the app") except where it
    /// is allowed: the Shared/Kept graph (Shared and the Private AI series,
    /// the menu's Shared chip, and the menu-bar item's shared bars), the
    /// Trace mark's own colours, and a tool's brand colour. Every colour,
    /// gradient stop and shadow is swept in both flat appearances.
    static let purpleAllowed: Set<String> = [
        "dataShared", "dataInference", "menuChipShared", "menuStripShared",
        "purple", "traceLiveStart", "traceLiveMid", "traceLiveEnd",
        "toolTheia",
    ]

    func test_noPurpleIsLeftOutsideTheGraphAndTheMarks() throws {
        let source = try Self.tokens()
        let colours = try XCTUnwrap(source["color"] as? [String: Any])
        let gradients = try XCTUnwrap(source["gradient"] as? [String: Any])
        let shadows = try XCTUnwrap(source["shadow"] as? [String: Any])
        for theme in ["flatLight", "flatDark"] {
            for key in colours.keys.sorted() where !Self.purpleAllowed.contains(key) {
                let rgb = try FlatThemeContrastTests.colour(key, theme: theme).rgb
                XCTAssertFalse(Self.isPurple(rgb), "\(theme): color.\(key) is purple")
            }
            for (group, entries) in [("gradient", gradients), ("shadow", shadows)] {
                for key in entries.keys.sorted() {
                    for hex in try Self.hexes(group, key, theme: theme, source: source) {
                        XCTAssertFalse(Self.isPurple(Self.rgb(hex)), "\(theme): \(group).\(key) has purple \(hex)")
                    }
                }
            }
        }
    }

    /// The Insights charts take the parent's current data palette, data-1 to
    /// data-4, which no longer starts with purple.
    func test_theChartSeriesAreTheParentsDataPalette() throws {
        let palette: [(String, UInt32, UInt32)] = [
            ("chartSeries1", 0x2A78D6, 0x0091FD), ("chartSeries2", 0xCF5320, 0xD95926),
            ("chartSeries3", 0x14906A, 0x199E70), ("chartSeries4", 0xA97200, 0xC98500),
        ]
        for (token, light, dark) in palette {
            XCTAssertEqual(try Self.hex(token, theme: "flatLight"), light, token)
            XCTAssertEqual(try Self.hex(token, theme: "flatDark"), dark, token)
        }
    }

    // MARK: Helpers

    static func hex(_ token: String, theme: String) throws -> UInt32 {
        let c = try FlatThemeContrastTests.colour(token, theme: theme).rgb
        return UInt32((c.r * 255).rounded()) << 16 | UInt32((c.g * 255).rounded()) << 8 | UInt32((c.b * 255).rounded())
    }

    static func hue(_ c: FlatThemeContrastTests.RGB) -> Double {
        let high = max(c.r, c.g, c.b), low = min(c.r, c.g, c.b), span = high - low
        guard span > 0 else { return 0 }
        let h: Double
        if high == c.r {
            h = ((c.g - c.b) / span).truncatingRemainder(dividingBy: 6)
        } else if high == c.g {
            h = (c.b - c.r) / span + 2
        } else {
            h = (c.r - c.g) / span + 4
        }
        return (h * 60 + 360).truncatingRemainder(dividingBy: 360)
    }

    /// Violet to magenta, with enough colour to read as purple.
    static func isPurple(_ c: FlatThemeContrastTests.RGB) -> Bool {
        let high = max(c.r, c.g, c.b), low = min(c.r, c.g, c.b)
        let lightness = (high + low) / 2
        let saturation = high == low ? 0 : (high - low) / (1 - abs(2 * lightness - 1))
        return (245 ... 320).contains(hue(c)) && saturation > 0.25
    }

    static func rgb(_ hex: String) -> FlatThemeContrastTests.RGB {
        let value = UInt32(hex.dropFirst(), radix: 16) ?? 0
        return FlatThemeContrastTests.RGB(
            r: Double((value >> 16) & 0xFF) / 255, g: Double((value >> 8) & 0xFF) / 255, b: Double(value & 0xFF) / 255)
    }

    static func tokens() throws -> [String: Any] {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
            .appending(path: "design-tokens/glass.tokens.json")
        return try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any])
    }

    /// Every hex a gradient's stops or a shadow's layers draw in the theme:
    /// its own entry where it names one, else the classic one for the
    /// appearance (a stop's or layer's light value under flatLight).
    static func hexes(_ group: String, _ key: String, theme: String, source: [String: Any]) throws -> [String] {
        let themed = ((source["themes"] as? [String: Any])?[theme] as? [String: Any])?[group] as? [String: Any]
        let classic = (source[group] as? [String: Any])?[key]
        let entry = themed?[key] ?? classic
        let layers: [[String: Any]]
        if group == "gradient" {
            layers = (entry as? [String: Any])?["stops"] as? [[String: Any]] ?? []
        } else {
            layers = entry as? [[String: Any]] ?? []
        }
        let usesClassic = themed?[key] == nil
        return layers.compactMap { layer in
            if usesClassic, theme == "flatLight", let light = layer["light"] as? [String: Any] {
                return light["hex"] as? String
            }
            return layer["hex"] as? String
        }
    }
}
