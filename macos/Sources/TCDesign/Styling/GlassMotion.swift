import AppKit
import SwiftUI

/// The one motion curve (spec, "Motion"): `timingCurve(0.2, 0.8, 0.2, 1)`
/// at the token durations, and none under Reduce Motion.
public enum GlassMotion {
    public static func curve(_ duration: Double) -> Animation {
        .timingCurve(
            GlassTokens.Motion.easeX1, GlassTokens.Motion.easeY1,
            GlassTokens.Motion.easeX2, GlassTokens.Motion.easeY2,
            duration: duration)
    }

    /// The system's Reduce Motion, for places with no environment to read
    /// (a style's action closure).
    @MainActor
    public static var systemReducesMotion: Bool {
        NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
    }

    public static func fast(_ reduceMotion: Bool) -> Animation? {
        reduceMotion ? nil : curve(GlassTokens.Motion.fast)
    }

    public static func standard(_ reduceMotion: Bool) -> Animation? {
        reduceMotion ? nil : curve(GlassTokens.Motion.standard)
    }
}
