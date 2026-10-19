import Foundation

/// The Traces map's style (Settings, General; owner, 2026-10-19): the
/// rocket and space station, or the standard map, where this Mac is a
/// laptop and the library a turning orb the traces rise to. A shell
/// preference, not a daemon setting: only the drawing changes, never what
/// is sent. The rocket is the default until the contributor turns it off.
public enum TracesMapStyle: String, Equatable, Sendable {
    case launch
    case standard

    public static let key = "traces_map_rocket"

    public static func stored(_ defaults: UserDefaults = .standard) -> TracesMapStyle {
        defaults.object(forKey: key) as? Bool == false ? .standard : .launch
    }

    public static func set(_ style: TracesMapStyle, _ defaults: UserDefaults = .standard) {
        defaults.set(style == .launch, forKey: key)
    }
}
