import CoreGraphics
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

/// Where the rail's panel sits on a screen, from the design's measures
/// ("Run 1 OS-level Explorations", 1b): a thin hover zone on the right
/// edge while closed, and, while open, the 56pt icon rail 12pt in from the
/// edge with its 380pt peek beside it.
public enum EdgeRailGeometry {
    /// The closed zone: thin enough not to take a window's scroll bar, tall
    /// enough to find without aiming.
    public static let zoneWidth: CGFloat = 4
    public static let zoneHeight: CGFloat = 240
    /// The rail, its inset from the edge, the gap to the peek and the peek.
    public static let railWidth: CGFloat = 56
    public static let edgeInset: CGFloat = 12
    public static let peekGap: CGFloat = 16
    public static let peekWidth: CGFloat = 380
    /// The open panel's height, enough for the tallest peek.
    public static let openHeight: CGFloat = 560
    /// Room left of the peek for its shadow.
    public static let shadowRoom: CGFloat = 24

    public static var openWidth: CGFloat { shadowRoom + peekWidth + peekGap + railWidth + edgeInset }

    /// The panel's frame on a screen whose visible frame is `visible`:
    /// against its right edge, centred vertically, never taller than it.
    public static func frame(open: Bool, visible: CGRect) -> CGRect {
        let width = open ? openWidth : zoneWidth
        let height = min(open ? openHeight : zoneHeight, visible.height)
        return CGRect(x: visible.maxX - width, y: visible.midY - height / 2, width: width, height: height)
    }
}
