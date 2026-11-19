//! The tab strip's motion (`GlassRuleTabs`,
//! `macos/Sources/TCDesign/Components/Navigation.swift`): the ember line
//! and halo glide to a newly chosen tab on `spring(0.35, 0.85)`, the
//! strip's `.animation(_, value: selection)`, which with the ember's
//! `matchedGeometryEffect` moves the glow from the old tab's frame to the
//! new one's. Pure numbers here; `shell` draws them.

use crate::ui::glass_motion;

/// `.spring(response: 0.35, dampingFraction: 0.85)` on the selection.
pub const EMBER_SPRING: (f64, f64) = (0.35, 0.85);

/// How long the glide runs.
pub fn ember_settles() -> f64 {
    glass_motion::spring_settles(EMBER_SPRING.0, EMBER_SPRING.1)
}

/// The ember's tab frame `(left, width)` `t` seconds into a glide from
/// `from` to `to`: both edges move together on the spring, overshooting a
/// little, as the matched frame does.
pub fn glide(from: (f64, f64), to: (f64, f64), t: f64) -> (f64, f64) {
    let p = glass_motion::spring(EMBER_SPRING.0, EMBER_SPRING.1, t);
    (from.0 + (to.0 - from.0) * p, from.1 + (to.1 - from.1) * p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn swift(path: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/../../macos/Sources/{path}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    /// The spring is the one `GlassRuleTabs` animates the selection on,
    /// read from the Swift.
    #[test]
    fn the_ember_glides_on_the_macs_spring() {
        let source = swift("TCDesign/Components/Navigation.swift");
        assert!(source.contains(
            ".animation(reduceMotion ? nil : .spring(response: 0.35, dampingFraction: 0.85), value: selection)"
        ));
        assert!(source.contains("emberGlow.matchedGeometryEffect(id: \"ember\", in: ember)"));
        assert_eq!(EMBER_SPRING, (0.35, 0.85));
    }

    /// The glide's values, from SwiftUI's own
    /// `Spring(response: 0.35, dampingRatio: 0.85).value(target:time:)` (a
    /// Swift probe, scratchpad motion/probe-surfaces), from a tab at 0 to
    /// one at 100, both 80 wide then 120.
    #[test]
    fn the_glide_follows_swiftuis_spring() {
        let swift = [
            (0.05, 0.242_229_348_430_760_27),
            (0.1, 0.588_284_561_132_537_2),
            (0.2, 0.942_504_042_583_767_3),
            (0.3, 1.004_831_848_148_262_3),
            (0.5, 1.000_776_100_098_912),
        ];
        for (t, p) in swift {
            let (left, width) = glide((0.0, 80.0), (100.0, 120.0), t);
            assert!((left - 100.0 * p).abs() < 1e-6, "{t}: {left}");
            assert!((width - (80.0 + 40.0 * p)).abs() < 1e-6, "{t}: {width}");
        }
        assert!((ember_settles() - 0.635_448_428_568_898_7).abs() < 0.05);
        assert_eq!(glide((0.0, 80.0), (100.0, 120.0), 0.0), (0.0, 80.0));
    }
}
