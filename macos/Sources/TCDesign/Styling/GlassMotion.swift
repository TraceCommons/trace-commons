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

    /// The curve at a duration of the caller's: the first run's longer
    /// choreography (a 0.52s step thread, a 1s marker draw). None under
    /// Reduce Motion.
    public static func timed(_ reduceMotion: Bool, duration: Double) -> Animation? {
        reduceMotion ? nil : curve(duration)
    }

    /// The first run's actors moving between steps (Ron's 2026-10-21
    /// canvas, the NEAR DS `spring-spatial-slow`): a slow spring that
    /// settles without overshoot to speak of. None under Reduce Motion.
    public static func spatial(_ reduceMotion: Bool, duration: Double = 0.9) -> Animation? {
        reduceMotion ? nil : .spring(duration: duration, bounce: 0.04)
    }

    /// A tile bursting out of the N, or a thumbs-up popping in (the NEAR DS
    /// `spring-expressive`): it overshoots and settles. None under Reduce
    /// Motion.
    public static func expressive(_ reduceMotion: Bool, duration: Double = 0.78) -> Animation? {
        reduceMotion ? nil : .spring(duration: duration, bounce: 0.32)
    }
}
