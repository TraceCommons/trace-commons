import Foundation

/// The edge rail's switch (Settings, system integrations): off until the
/// contributor turns it on. A shell preference, not a daemon setting -- the
/// rail only reads, and nothing the core does depends on it being shown.
public enum EdgeRailPreference {
    public static let key = "edge_rail_enabled"

    public static func isEnabled(_ defaults: UserDefaults = .standard) -> Bool {
        defaults.bool(forKey: key)
    }

    public static func set(_ enabled: Bool, _ defaults: UserDefaults = .standard) {
        defaults.set(enabled, forKey: key)
    }
}
