import AppKit
import Foundation

/// Which set of token values the app draws with (design exploration).
///
/// `classic` is the glass system as it ships. `flat` comes from getone.one's
/// glass components: white text on navy-tinted glass, a 1pt white rim with a
/// top highlight, flat fills, low veils so the desktop shows through, and the
/// brand purple for a selected state. Its light appearance is the site's
/// light tint and its dark appearance a darker tint of the same navy, so the
/// person's appearance choice (`GlassAppearance`) picks between them. The
/// flat values live in `design-tokens/glass.tokens.json` (`themes`), and the
/// generated tokens read them through `pick`.
///
/// The theme is fixed for the life of the process: `TC_GLASS_THEME` in the
/// environment, or `defaults write <bundle id> TCGlassTheme classic`, and
/// relaunch. On this branch the default is `flat`; a test run stays on
/// `classic`, which is what the design suites measure.
public enum GlassTheme: String, Sendable, CaseIterable {
    case classic, flat

    /// The theme this process launched in.
    public static let current: GlassTheme = resolve(
        environment: ProcessInfo.processInfo.environment["TC_GLASS_THEME"],
        defaults: UserDefaults.standard.string(forKey: "TCGlassTheme"),
        testing: NSClassFromString("XCTestCase") != nil
    )

    /// The environment wins over the defaults; with neither, `flat`, or
    /// `classic` under a test run.
    static func resolve(environment: String?, defaults: String?, testing: Bool) -> GlassTheme {
        for value in [environment, defaults] {
            if let value, let theme = GlassTheme(rawValue: value) { return theme }
        }
        return testing ? .classic : .flat
    }

    /// A token's value in the current theme.
    public static func pick<T>(_ classic: T, flat: T) -> T {
        current == .flat ? flat : classic
    }

    /// Whether a pane in the content layer (the map) is glass too. Classic
    /// keeps it on the opaque base, as Apple keeps content out of Liquid
    /// Glass; the flat theme puts it on the same frosted glass as the other
    /// panes, under a lighter map field (owner feedback, 2026-10-09).
    static var contentIsGlass: Bool { current == .flat }

    /// The native material under a flat pane. `clearTint` is the default:
    /// Liquid Glass, for the system's refraction, tinted with the veil,
    /// which of the five came closest to the mock (owner comparison,
    /// 2026-10-09). Liquid Glass drops its tint in a window out of focus;
    /// `paintsVeil` paints it back there, so the tint holds and only the
    /// system's frost changes, as the HUD material's look holds.
    /// `TC_GLASS_MATERIAL` picks another, for comparison.
    enum FlatMaterial: String {
        /// Liquid Glass, frosted, with the veil as the glass's own tint.
        case regularTint
        /// Liquid Glass, clear, with the veil as the glass's own tint.
        case clearTint
        /// Liquid Glass, frosted, with the veil painted over it.
        case regular
        /// The sidebar vibrancy material, with the veil painted over it.
        case sidebar
        /// The HUD vibrancy material, with the veil painted over it.
        case hud

        /// Whether the glass carries the veil as its tint, so none is painted.
        var tintsGlass: Bool { self == .regularTint || self == .clearTint }
    }

    /// The appearance a flat material is drawn in: dark, in both of the
    /// app's appearances. The tint is the veil's; a light material adds a
    /// white frost of its own that washes the tint out (2026-10-09). Nil
    /// (the app's own) for classic.
    static var materialAppearance: NSAppearance? {
        current == .flat ? NSAppearance(named: .darkAqua) : nil
    }

    /// Whether a pane paints the veil itself. Where the glass carries the
    /// veil as its tint, it does not -- except that Liquid Glass drops its
    /// tint and frosts lighter in a window that is not in focus, so there
    /// the pane paints the veil in its place, and the tint holds in and out
    /// of focus (owner comparison, 2026-10-09).
    static func paintsVeil(windowIsKey: Bool) -> Bool {
        guard current == .flat, flatMaterial.tintsGlass else { return true }
        return !windowIsKey
    }

    static let flatMaterial: FlatMaterial =
        ProcessInfo.processInfo.environment["TC_GLASS_MATERIAL"].flatMap(FlatMaterial.init(rawValue:)) ?? .clearTint
}

/// The person's appearance choice: Light, Dark, or the system's
/// (Settings > General). Stored per user; applied to the whole app, so every
/// token resolves for it as it does for the system's own setting.
public enum GlassAppearance: String, Sendable, CaseIterable, Identifiable {
    case light, dark, system

    public var id: String { rawValue }

    static let defaultsKey = "TCAppearance"

    /// The stored choice; System when none is stored.
    public static var stored: GlassAppearance {
        UserDefaults.standard.string(forKey: defaultsKey).flatMap(GlassAppearance.init(rawValue:)) ?? .system
    }

    /// The `NSAppearance` this choice sets on the app; nil follows the system.
    public var nsAppearance: NSAppearance? {
        switch self {
        case .light: NSAppearance(named: .aqua)
        case .dark: NSAppearance(named: .darkAqua)
        case .system: nil
        }
    }

    /// Store this choice and apply it.
    @MainActor
    public func choose() {
        UserDefaults.standard.set(rawValue, forKey: Self.defaultsKey)
        apply()
    }

    /// Put the app in this appearance.
    @MainActor
    public func apply() {
        NSApplication.shared.appearance = nsAppearance
    }

    /// Apply the stored choice, at launch. With none stored the app is left
    /// as it is, so a development override of the appearance holds.
    @MainActor
    public static func applyStored() {
        guard UserDefaults.standard.string(forKey: defaultsKey) != nil else { return }
        stored.apply()
    }
}
