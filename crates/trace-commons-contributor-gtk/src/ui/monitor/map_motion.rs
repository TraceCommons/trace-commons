//! How the map, the graph and the panes move: the Mac's numbers, and a
//! value that heads for a target on one of its curves or springs.
//!
//! Ported from, each beside its numbers below:
//! - the map field's shimmer and the light under the pointer
//!   (`GlassMapMarks`, `GlassMapShimmerHost`, `GlassMapMarkGlow`,
//!   `GlassMapLight` in `macos/Sources/TCDesign/Components/Indicators.swift`,
//!   and `GlassShimmerBand` in `ShimmerBand.swift`);
//! - the tool pile opening and closing, the coins coming to the front and
//!   growing (`MapToolPile.hover` and `.coin` in
//!   `macos/Sources/TraceCommonsApp/Views/Monitor/MapToolPile.swift`);
//! - the graph's period slide and zoom swap (`TracesGraphFooter` in
//!   `MonitorShell.swift`), the graph footer opening and closing
//!   (`MonitorWindowView.swift`), and the panes fading in and out as they
//!   open and close (`GlassThreePane` in `ThreePane.swift`).
//!
//! Deliberate platform difference: a value retargeted while it moves
//! starts again from where it is, at rest. SwiftUI carries the old
//! movement's velocity into the new one, so a pile left and re-entered
//! within a few hundred milliseconds turns a little more softly on the
//! Mac.

use std::collections::BTreeMap;

use super::map_scene::PileStep;
use crate::ui::glass_motion;
use crate::ui::style::brand_tokens::tile;

/// `GlassTokens.Motion.fast`, `.standard` and `.slide`, in seconds.
pub const FAST: f64 = 0.15;
pub const STANDARD: f64 = 0.22;
pub const SLIDE: f64 = 0.25;

/// A movement's timing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ease {
    /// `.spring(response:dampingFraction:)`.
    Spring(f64, f64),
    /// `.timingCurve(x1, y1, x2, y2, duration:)`.
    Curve(f64, f64, f64, f64, f64),
}

impl Ease {
    /// `GlassMotion.curve(duration)`: the token curve.
    pub fn token(duration: f64) -> Self {
        Self::Curve(
            tile::EASE_X1,
            tile::EASE_Y1,
            tile::EASE_X2,
            tile::EASE_Y2,
            duration,
        )
    }

    /// SwiftUI's `.easeOut(duration:)`.
    pub fn ease_out(duration: f64) -> Self {
        Self::Curve(0.0, 0.0, 0.58, 1.0, duration)
    }

    /// How long it runs, in seconds.
    pub fn length(self) -> f64 {
        match self {
            Self::Spring(response, damping) => glass_motion::spring_settles(response, damping),
            Self::Curve(.., duration) => duration,
        }
    }

    /// How far from start to end it is `t` seconds in: 1 from its end on.
    pub fn progress(self, t: f64) -> f64 {
        if t <= 0.0 {
            return 0.0;
        }
        if t >= self.length() {
            return 1.0;
        }
        match self {
            Self::Spring(response, damping) => glass_motion::spring(response, damping, t),
            Self::Curve(x1, y1, x2, y2, duration) => {
                glass_motion::bezier(x1, y1, x2, y2, t / duration)
            }
        }
    }
}

/// A value heading for a target on an `Ease`, or resting at it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tween {
    from: f64,
    to: f64,
    start: f64,
    ease: Option<Ease>,
}

/// At rest at 0.
impl Default for Tween {
    fn default() -> Self {
        Self::at_rest(0.0)
    }
}

impl Tween {
    pub fn at_rest(value: f64) -> Self {
        Self {
            from: value,
            to: value,
            start: 0.0,
            ease: None,
        }
    }

    pub fn target(&self) -> f64 {
        self.to
    }

    /// Its value at `now`, in seconds on the caller's clock.
    pub fn value(&self, now: f64) -> f64 {
        match self.ease {
            Some(ease) => self.from + (self.to - self.from) * ease.progress(now - self.start),
            None => self.to,
        }
    }

    /// Whether it is still moving at `now`.
    pub fn moving(&self, now: f64) -> bool {
        self.ease
            .is_some_and(|ease| now - self.start < ease.length())
    }

    /// Heads for `to` from where it is at `now`, on `ease`; with no ease
    /// (reduced motion) it is there at once. A target it already has
    /// changes nothing.
    pub fn retarget(&mut self, to: f64, now: f64, ease: Option<Ease>) {
        if to == self.to {
            return;
        }
        let from = self.value(now);
        *self = Self {
            from,
            to,
            start: now,
            ease,
        };
    }

    /// At its target, at once.
    pub fn settle(&mut self) {
        *self = Self::at_rest(self.to);
    }
}

// --- The map field -------------------------------------------------------------

/// `GlassMapMarks.shimmerFirst`, `.shimmerEvery`, `.shimmerDuration`: the
/// first pass a second after the map is shown, then one every 40 seconds,
/// each over 1.1 s.
pub const SHIMMER_FIRST: f64 = 1.0;
pub const SHIMMER_EVERY: f64 = 40.0;
pub const SHIMMER_DURATION: f64 = 1.1;
/// `GlassMapMarks.shimmerAnimation`: a fast start that settles.
pub const SHIMMER_CURVE: (f64, f64, f64, f64) = (0.3, 0.0, 0.15, 1.0);
/// `GlassMapMarks.shimmerSlant` (degrees from upright, its top to the
/// right) and `.shimmerHalfWidth` (a share of the field's diagonal).
pub const SHIMMER_SLANT: f64 = 20.0;
pub const SHIMMER_HALF_WIDTH: f64 = 0.1;
/// `GlassMapMarks.reach`: how far the light reaches from the pointer.
pub const REACH: f64 = 140.0;

/// How far across the field the shimmer is, `t` seconds into a pass: 0
/// before and after, when nothing is drawn (`GlassMapShimmer`'s `progress`).
pub fn shimmer_progress(t: f64) -> f64 {
    if !(0.0..SHIMMER_DURATION).contains(&t) {
        return 0.0;
    }
    let (x1, y1, x2, y2) = SHIMMER_CURVE;
    glass_motion::bezier(x1, y1, x2, y2, t / SHIMMER_DURATION)
}

/// `GlassShimmerBand.band`: the gradient's start and end for the band at
/// `progress` across a field `width` by `height`.
pub fn shimmer_band(progress: f64, width: f64, height: f64) -> ((f64, f64), (f64, f64)) {
    let angle = SHIMMER_SLANT.to_radians();
    // Square to the band: rightward, tipped down by the slant (y runs
    // down), so the band itself leans with its top to the right.
    let (dx, dy) = (angle.cos(), angle.sin());
    let along =
        [(0.0, 0.0), (width, 0.0), (0.0, height), (width, height)].map(|(x, y)| x * dx + y * dy);
    let half = SHIMMER_HALF_WIDTH * width.hypot(height).max(1.0);
    let from = along.iter().copied().fold(f64::INFINITY, f64::min) - half;
    let to = along.iter().copied().fold(f64::NEG_INFINITY, f64::max) + half;
    let centre = from + (to - from) * progress;
    (
        (dx * (centre - half), dy * (centre - half)),
        (dx * (centre + half), dy * (centre + half)),
    )
}

/// `GlassMapMarks.light`: how lit a mark is at `distance` from the
/// pointer, 1 under it, easing to 0 at `REACH`.
pub fn light(distance: f64) -> f64 {
    if distance >= REACH {
        return 0.0;
    }
    let t = 1.0 - (distance / REACH) * (distance / REACH);
    t * t
}

/// The flat theme's inks for the moving light (`GlassMapMarks`), as white
/// alphas: the marks at the light's centre (`litInk`), at the shimmer's
/// crest (`shimmerInk`), and the bloom under the pointer (`bloom`).
pub fn lit_ink(dark: bool) -> f64 {
    if dark { 0.2 } else { 0.49 }
}

pub fn shimmer_ink(dark: bool) -> f64 {
    if dark { 0.24 } else { 0.49 }
}

pub fn bloom(dark: bool) -> f64 {
    if dark { 0.035 } else { 0.2 }
}

// --- The tool pile --------------------------------------------------------------

/// `MapToolPile.hover`'s springs and fades, by stage.
pub const TALL_OPEN: Ease = Ease::Spring(0.34, 0.84);
pub const WIDE_OPEN: Ease = Ease::Spring(0.3, 0.86);
pub const NAMES_IN: f64 = 0.18;
pub const NAMES_OUT: f64 = 0.15;
pub const WIDE_CLOSE: Ease = Ease::Spring(0.28, 0.9);
pub const TALL_CLOSE: Ease = Ease::Spring(0.34, 0.86);
/// `MapToolPile.coin`'s springs: the coins re-laid as the pile opens or
/// closes (`value: open`), a coin to its new place in the order
/// (`value: index`), and a coin growing as a trace is in it
/// (`value: grown`).
pub const LAY: Ease = Ease::Spring(0.36, 0.82);
pub const REORDER: Ease = Ease::Spring(0.42, 0.8);
pub const GROW: Ease = Ease::Spring(0.3, 0.7);
/// A grown coin's scale (`scaleEffect(grown ? 1.3 : 1)`).
pub const GROWN: f64 = 1.3;

/// The pile as it moves: the card's height, width and names on
/// `MapToolPile.hover`'s stages, the coins' spacing on `LAY` and each
/// coin's place and growth on `REORDER` and `GROW`.
#[derive(Clone, Debug, PartialEq)]
pub struct PileMotion {
    pub tall: Tween,
    pub wide: Tween,
    pub names: Tween,
    /// How far the coins are laid out open: 0 closed, 1 open.
    pub lay: Tween,
    coins: BTreeMap<String, CoinMotion>,
}

/// One coin as it moves: its place in the pile (an index, fractional
/// between two) and how far it is grown (0 to 1, past 1 as it springs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoinMotion {
    pub index: Tween,
    pub grow: Tween,
}

impl Default for PileMotion {
    fn default() -> Self {
        Self {
            tall: Tween::at_rest(0.0),
            wide: Tween::at_rest(0.0),
            names: Tween::at_rest(0.0),
            lay: Tween::at_rest(0.0),
            coins: BTreeMap::new(),
        }
    }
}

impl PileMotion {
    /// One of the hover's stages, at `now`, on its own timing; at once
    /// under reduced motion.
    pub fn step(&mut self, step: PileStep, now: f64, reduce_motion: bool) {
        let ease = |ease: Ease| (!reduce_motion).then_some(ease);
        match step {
            PileStep::Tall(true) => self.tall.retarget(1.0, now, ease(TALL_OPEN)),
            PileStep::Wide(true) => self.wide.retarget(1.0, now, ease(WIDE_OPEN)),
            PileStep::Names(true) => self
                .names
                .retarget(1.0, now, ease(Ease::ease_out(NAMES_IN))),
            PileStep::Names(false) => {
                self.names
                    .retarget(0.0, now, ease(Ease::ease_out(NAMES_OUT)))
            }
            PileStep::Wide(false) => self.wide.retarget(0.0, now, ease(WIDE_CLOSE)),
            PileStep::Tall(false) => self.tall.retarget(0.0, now, ease(TALL_CLOSE)),
        }
    }

    /// The coins' targets at `now`: laid `open` or closed, in `order`
    /// front first, those in `grown` grown. A coin new to the pile is
    /// simply there.
    pub fn follow(
        &mut self,
        now: f64,
        open: bool,
        order: &[String],
        grown: &[String],
        reduce_motion: bool,
    ) {
        let ease = |ease: Ease| (!reduce_motion).then_some(ease);
        self.lay
            .retarget(if open { 1.0 } else { 0.0 }, now, ease(LAY));
        self.coins.retain(|id, _| order.contains(id));
        for (index, id) in order.iter().enumerate() {
            let grows = if grown.contains(id) { 1.0 } else { 0.0 };
            let coin = self.coins.entry(id.clone()).or_insert(CoinMotion {
                index: Tween::at_rest(index as f64),
                grow: Tween::at_rest(grows),
            });
            coin.index.retarget(index as f64, now, ease(REORDER));
            coin.grow.retarget(grows, now, ease(GROW));
        }
    }

    pub fn coin(&self, id: &str) -> Option<CoinMotion> {
        self.coins.get(id).copied()
    }

    /// Whether anything in the pile is still moving at `now`.
    pub fn moving(&self, now: f64) -> bool {
        [self.tall, self.wide, self.names, self.lay]
            .iter()
            .any(|tween| tween.moving(now))
            || self
                .coins
                .values()
                .any(|coin| coin.index.moving(now) || coin.grow.moving(now))
    }

    /// Everything at its target at once (reduced motion turned on).
    pub fn settle(&mut self) {
        for tween in [
            &mut self.tall,
            &mut self.wide,
            &mut self.names,
            &mut self.lay,
        ] {
            tween.settle();
        }
        for coin in self.coins.values_mut() {
            coin.index.settle();
            coin.grow.settle();
        }
    }
}

// --- The transfer cards -----------------------------------------------------------

/// The stack's spring (`GlassPopoverStack`'s `.animation(.spring(response:
/// 0.36, dampingFraction: 0.86), value: front.id)`).
pub const STACK: Ease = Ease::Spring(0.36, 0.86);

/// The opacity of the card a new front card covers, or the one a close
/// takes away, `t` seconds after it left the front: it fades where it is
/// on the stack's spring (`GlassPopoverStack.arrival`, removal `.opacity`).
pub fn covered_opacity(t: f64) -> f64 {
    (1.0 - STACK.progress(t)).clamp(0.0, 1.0)
}

// --- The graph and the panes ------------------------------------------------------

/// `TracesGraphFooter.slideAnimation`: #1146's 0.45 s on
/// `cubic-bezier(.4, 0, .2, 1)`.
pub const PERIOD_SLIDE: Ease = Ease::Curve(0.4, 0.0, 0.2, 1.0, 0.45);

/// `TracesGraphFooter.zoom`: the bars swap in place over `GlassMotion.fast`.
pub fn zoom_fade() -> Ease {
    Ease::token(FAST)
}

/// The graph footer opening and closing over `Motion.slide` on the token
/// curve (`MonitorWindowView`'s `.animation(..., value: showsGraph)`).
pub fn footer() -> Ease {
    Ease::token(SLIDE)
}

/// A pane fading as it opens or closes (`GlassThreePane`, the window's
/// move over `Motion.standard` on the token curve).
pub fn pane_fade() -> Ease {
    Ease::token(STANDARD)
}

/// The side a period's bars slide from: the new ones come in from it and
/// the old ones go out toward it, fading, as SwiftUI's symmetric
/// `.move(edge:)` moves both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Leading,
    Trailing,
}

/// Where the outgoing and incoming bars are, as a share of the bars'
/// width (positive is toward the trailing edge), and their opacities,
/// `t` seconds into a period step from `edge`.
pub fn period_slide(edge: Edge, t: f64) -> ((f64, f64), (f64, f64)) {
    let p = PERIOD_SLIDE.progress(t.max(0.0));
    let side = match edge {
        Edge::Leading => -1.0,
        Edge::Trailing => 1.0,
    };
    ((side * p, 1.0 - p), (side * (1.0 - p), p))
}

/// The bars as they swap, `t` seconds in: a period step slides both sets
/// toward its edge (`period_slide`), a zoom crossfades them in place over
/// `GlassMotion.fast`. Each set's offset as a share of the bars' width and
/// its opacity, the old set first.
pub fn bars_swap(edge: Option<Edge>, t: f64) -> ((f64, f64), (f64, f64)) {
    match edge {
        Some(edge) => period_slide(edge, t),
        None => {
            let p = zoom_fade().progress(t);
            ((0.0, 1.0 - p), (0.0, p))
        }
    }
}

/// How long a swap of the bars runs.
pub fn bars_swap_length(edge: Option<Edge>) -> f64 {
    match edge {
        Some(_) => PERIOD_SLIDE.length(),
        None => zoom_fade().length(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDICATORS: &str =
        include_str!("../../../../../macos/Sources/TCDesign/Components/Indicators.swift");
    const SHIMMER_BAND: &str =
        include_str!("../../../../../macos/Sources/TCDesign/Components/ShimmerBand.swift");
    const TOOL_PILE: &str = include_str!(
        "../../../../../macos/Sources/TraceCommonsApp/Views/Monitor/MapToolPile.swift"
    );
    const SHELL: &str = include_str!(
        "../../../../../macos/Sources/TraceCommonsApp/Views/Monitor/MonitorShell.swift"
    );
    const WINDOW: &str =
        include_str!("../../../../../macos/Sources/TraceCommonsApp/Views/MonitorWindowView.swift");
    const THREE_PANE: &str =
        include_str!("../../../../../macos/Sources/TCDesign/Components/ThreePane.swift");
    const TOKENS: &str =
        include_str!("../../../../../macos/Sources/TCDesign/Generated/GlassTokens.swift");

    fn has(source: &str, line: &str) {
        assert!(source.contains(line), "the Swift no longer says `{line}`");
    }

    fn close(got: f64, want: f64, tolerance: f64, what: &str) {
        assert!(
            (got - want).abs() < tolerance,
            "{what}: {got}, Swift {want}"
        );
    }

    /// The token durations are the Mac's.
    #[test]
    fn the_durations_are_the_mac_tokens() {
        has(TOKENS, &format!("static let fast: Double = {FAST}"));
        has(TOKENS, &format!("static let standard: Double = {STANDARD}"));
        has(TOKENS, &format!("static let slide: Double = {SLIDE}"));
    }

    /// The new curves at the values SwiftUI's `UnitCurve.bezier` gives (a
    /// Swift probe), and their numbers as the Swift writes them.
    #[test]
    fn the_shimmer_and_slide_curves_are_swiftuis() {
        has(
            INDICATORS,
            ".timingCurve(0.3, 0, 0.15, 1, duration: shimmerDuration)",
        );
        has(SHELL, ".timingCurve(0.4, 0, 0.2, 1, duration: 0.45)");
        let shimmer = Ease::Curve(0.3, 0.0, 0.15, 1.0, 1.0);
        let slide = Ease::Curve(0.4, 0.0, 0.2, 1.0, 1.0);
        for (t, s, p) in [
            (0.1, 0.049_227_715, 0.025_862_694),
            (0.25, 0.388_435_364, 0.236_587_524),
            (0.5, 0.819_087_029, 0.775_561_333),
            (0.75, 0.965_179_443, 0.959_367_752),
            (0.9, 0.995_054_245, 0.994_354_248),
        ] {
            close(shimmer.progress(t), s, 1e-5, "shimmer");
            close(slide.progress(t), p, 1e-5, "slide");
        }
        assert_eq!(
            SHIMMER_CURVE,
            (0.3, 0.0, 0.15, 1.0),
            "the shimmer's curve constant"
        );
        assert_eq!(PERIOD_SLIDE, Ease::Curve(0.4, 0.0, 0.2, 1.0, 0.45));
        assert_eq!(PERIOD_SLIDE.length(), 0.45);
    }

    /// The pile's and the coins' springs, at the values SwiftUI's
    /// `Spring(response:dampingRatio:).value(target: 1, time:)` gives (a
    /// Swift probe), each named in the Swift as it is used.
    #[test]
    fn the_pile_and_coin_springs_are_swiftuis() {
        has(
            TOOL_PILE,
            "withAnimation(motion ? .spring(response: 0.34, dampingFraction: 0.84) : nil) { tall = true }",
        );
        has(
            TOOL_PILE,
            "withAnimation(motion ? .spring(response: 0.3, dampingFraction: 0.86) : nil) { wide = true }",
        );
        has(
            TOOL_PILE,
            "withAnimation(motion ? .easeOut(duration: 0.18) : nil) { names = true }",
        );
        has(
            TOOL_PILE,
            "withAnimation(motion ? .easeOut(duration: 0.15) : nil) { names = false }",
        );
        has(
            TOOL_PILE,
            "withAnimation(motion ? .spring(response: 0.28, dampingFraction: 0.9) : nil) { wide = false }",
        );
        has(
            TOOL_PILE,
            "withAnimation(motion ? .spring(response: 0.34, dampingFraction: 0.86) : nil) { tall = false }",
        );
        has(
            TOOL_PILE,
            ".spring(response: 0.42, dampingFraction: 0.8), value: index)",
        );
        has(
            TOOL_PILE,
            ".spring(response: 0.36, dampingFraction: 0.82), value: open)",
        );
        has(
            TOOL_PILE,
            ".spring(response: 0.3, dampingFraction: 0.7), value: grown)",
        );
        has(TOOL_PILE, ".scaleEffect(grown ? 1.3 : 1)");
        assert_eq!((NAMES_IN, NAMES_OUT, GROWN), (0.18, 0.15, 1.3));
        let swift: [(Ease, [f64; 5], f64); 7] = [
            (
                TALL_OPEN,
                [
                    0.254_070_536,
                    0.609_732_579,
                    0.955_913_325,
                    1.007_454_236,
                    1.000_503_242,
                ],
                0.625_679_064,
            ),
            (
                WIDE_OPEN,
                [
                    0.301_551_077,
                    0.676_634_630,
                    0.975_876_350,
                    1.004_981_458,
                    1.000_094_480,
                ],
                0.547_020_074,
            ),
            (
                WIDE_CLOSE,
                [
                    0.325_041_162,
                    0.698_592_952,
                    0.972_926_645,
                    1.001_294_539,
                    1.000_076_343,
                ],
                0.493_245_634,
            ),
            (
                TALL_CLOSE,
                [
                    0.251_728_958,
                    0.601_787_319,
                    0.946_185_138,
                    1.003_683_260,
                    1.000_595_533,
                ],
                0.612_520_345,
            ),
            (
                LAY,
                [
                    0.235_335_922,
                    0.582_676_783,
                    0.949_167_949,
                    1.010_692_810,
                    1.000_856_139,
                ],
                0.673_321_311,
            ),
            (
                REORDER,
                [
                    0.186_742_568,
                    0.496_621_349,
                    0.901_633_944,
                    1.008_895_796,
                    1.003_834_633,
                ],
                0.791_298_467,
            ),
            (
                GROW,
                [
                    0.327_494_862,
                    0.757_060_934,
                    1.044_867_376,
                    1.014_498_422,
                    0.999_162_081,
                ],
                0.658_827_380,
            ),
        ];
        for (ease, values, settles) in swift {
            for (t, want) in [0.05, 0.1, 0.2, 0.3, 0.5].into_iter().zip(values) {
                // Past its length a spring is at its end, within a
                // ten-thousandth of SwiftUI's.
                let tolerance = if t < ease.length() { 1e-6 } else { 1e-4 };
                close(
                    ease.progress(t),
                    want,
                    tolerance,
                    &format!("{ease:?} at {t}"),
                );
            }
            close(ease.length(), settles, 0.05, &format!("{ease:?} settles"));
            assert_eq!(ease.progress(ease.length() + 0.01), 1.0);
        }
    }

    /// `GlassMapMarks`' shimmer timing, band and reach, read from the
    /// Swift, and the band where `GlassShimmerBand.band` puts it for a
    /// 400 by 300 field (a Swift probe running `ShimmerBand.swift`).
    #[test]
    fn the_shimmer_band_and_light_are_the_macs() {
        has(
            INDICATORS,
            "static let shimmerEvery: Duration = .seconds(40)",
        );
        has(
            INDICATORS,
            "static let shimmerFirst: Duration = .seconds(1)",
        );
        has(INDICATORS, "static let shimmerDuration: Double = 1.1");
        has(INDICATORS, "static let shimmerHalfWidth: CGFloat = 0.1");
        has(INDICATORS, "static let shimmerSlant: Double = 20");
        has(INDICATORS, "static let reach: CGFloat = 140");
        has(
            SHIMMER_BAND,
            "let unit = CGVector(dx: CGFloat(cos(angle)), dy: CGFloat(sin(angle)))",
        );
        assert_eq!(
            (SHIMMER_FIRST, SHIMMER_EVERY, SHIMMER_DURATION),
            (1.0, 40.0, 1.1)
        );
        assert_eq!(
            (SHIMMER_SLANT, SHIMMER_HALF_WIDTH, REACH),
            (20.0, 0.1, 140.0)
        );
        for (p, swift) in [
            (0.0, [-93.969_262_079, -34.202_014_333, 0.0, 0.0]),
            (
                0.25,
                [
                    41.929_810_960,
                    15.261_203_118,
                    135.899_073_039,
                    49.463_217_451,
                ],
            ),
            (
                0.5,
                [
                    177.828_883_999,
                    64.724_420_568,
                    271.798_146_078,
                    98.926_434_901,
                ],
            ),
            (
                1.0,
                [
                    449.627_030_077,
                    163.650_855_469,
                    543.596_292_155,
                    197.852_869_802,
                ],
            ),
        ] {
            let ((sx, sy), (ex, ey)) = shimmer_band(p, 400.0, 300.0);
            for (got, want) in [sx, sy, ex, ey].into_iter().zip(swift) {
                close(got, want, 1e-6, &format!("band at {p}"));
            }
        }
        // A pass: nothing before or after it, the curve's value between.
        assert_eq!(shimmer_progress(-0.1), 0.0);
        assert_eq!(shimmer_progress(SHIMMER_DURATION), 0.0);
        close(
            shimmer_progress(0.25 * SHIMMER_DURATION),
            0.388_435_364,
            1e-5,
            "a quarter of the way through a pass",
        );
        for (distance, swift) in [
            (0.0, 1.0),
            (35.0, 0.878_906_25),
            (70.0, 0.5625),
            (105.0, 0.191_406_25),
            (139.0, 0.000_202_627),
            (140.0, 0.0),
        ] {
            close(
                light(distance),
                swift,
                1e-9,
                &format!("light at {distance}"),
            );
        }
    }

    /// The flat theme's moving inks, as the Swift picks them.
    #[test]
    fn the_light_inks_are_the_flat_themes() {
        has(
            INDICATORS,
            "flat: GlassRGBA(0xFFFFFF, alpha: 0.2, light: GlassRGBA(0xFFFFFF, alpha: 0.49)))",
        );
        has(
            INDICATORS,
            "flat: GlassRGBA(0xFFFFFF, alpha: 0.24, light: GlassRGBA(0xFFFFFF, alpha: 0.49)))",
        );
        has(
            INDICATORS,
            "static let bloom = GlassRGBA(0xFFFFFF, alpha: 0.035, light: GlassRGBA(0xFFFFFF, alpha: 0.2))",
        );
        assert_eq!((lit_ink(true), lit_ink(false)), (0.2, 0.49));
        assert_eq!((shimmer_ink(true), shimmer_ink(false)), (0.24, 0.49));
        assert_eq!((bloom(true), bloom(false)), (0.035, 0.2));
        has(
            INDICATORS,
            ".animation(GlassMotion.fast(reduceMotion), value: pointer == nil)",
        );
    }

    /// The graph's, the footer's and the panes' timings, as the Swift
    /// names them.
    #[test]
    fn the_graph_footer_and_pane_timings_are_the_macs() {
        has(SHELL, "withAnimation(GlassMotion.fast(reduceMotion)) {");
        has(
            SHELL,
            ".transition(slide.map { .move(edge: $0).combined(with: .opacity) } ?? .opacity)",
        );
        has(SHELL, "{ step(-1, from: .trailing) }");
        has(SHELL, "{ step(1, from: .leading) }");
        has(
            WINDOW,
            ".animation(GlassMotion.systemReducesMotion ? nil : GlassMotion.curve(GlassTokens.Motion.slide), value: showsGraph)",
        );
        has(
            WINDOW,
            ".transition(.move(edge: .bottom).combined(with: .opacity))",
        );
        has(
            THREE_PANE,
            "withAnimation(GlassMotion.curve(GlassTokens.Motion.standard)) {",
        );
        assert_eq!(zoom_fade(), Ease::token(0.15));
        assert_eq!(footer(), Ease::token(0.25));
        assert_eq!(pane_fade(), Ease::token(0.22));
        // The token curve at a fifth of the way: SwiftUI's own value.
        close(
            footer().progress(0.25 * 0.25),
            0.767_284_393,
            1e-5,
            "footer",
        );
    }

    /// A period step: both sets of bars move toward the edge over 0.45 s,
    /// the old fading out and the new in (`.move(edge:)` with
    /// `.opacity`, symmetric).
    #[test]
    fn a_period_step_slides_both_sets_toward_its_edge() {
        let ((old_x, old_a), (new_x, new_a)) = period_slide(Edge::Trailing, 0.0);
        assert_eq!((old_x, old_a, new_x, new_a), (0.0, 1.0, 1.0, 0.0));
        let ((old_x, old_a), (new_x, new_a)) = period_slide(Edge::Leading, 0.0);
        assert_eq!((old_x, old_a, new_x, new_a), (0.0, 1.0, -1.0, 0.0));
        // Halfway in time the curve is 0.775561 of the way.
        let ((old_x, old_a), (new_x, new_a)) = period_slide(Edge::Trailing, 0.225);
        close(old_x, 0.775_561_333, 1e-5, "old bars");
        close(old_a, 1.0 - 0.775_561_333, 1e-5, "old opacity");
        close(new_x, 1.0 - 0.775_561_333, 1e-5, "new bars");
        close(new_a, 0.775_561_333, 1e-5, "new opacity");
        let ((old_x, old_a), (new_x, new_a)) = period_slide(Edge::Leading, 0.45);
        assert_eq!((old_x, old_a, new_x, new_a), (-1.0, 0.0, 0.0, 1.0));
    }

    /// Each of the hover's stages takes its own timing from
    /// `MapToolPile.hover`, and none under reduced motion.
    #[test]
    fn each_pile_stage_moves_on_its_own_timing() {
        let mut pile = PileMotion::default();
        pile.step(PileStep::Tall(true), 0.0, false);
        close(pile.tall.value(0.1), 0.609_732_579, 1e-6, "tall opening");
        pile.step(PileStep::Wide(true), 0.26, false);
        close(pile.wide.value(0.36), 0.676_634_630, 1e-6, "wide opening");
        pile.step(PileStep::Names(true), 0.46, false);
        close(
            pile.names.value(0.46 + 0.09),
            Ease::ease_out(0.18).progress(0.09),
            1e-9,
            "names in",
        );
        assert!(pile.moving(0.5));
        assert!(!pile.moving(2.0));
        pile.step(PileStep::Names(false), 3.0, false);
        close(
            pile.names.value(3.075),
            1.0 - Ease::ease_out(0.15).progress(0.075),
            1e-9,
            "names out",
        );
        pile.step(PileStep::Wide(false), 3.1, false);
        close(
            pile.wide.value(3.2),
            1.0 - 0.698_592_952,
            1e-6,
            "wide closing",
        );
        pile.step(PileStep::Tall(false), 3.32, false);
        close(
            pile.tall.value(3.42),
            1.0 - 0.601_787_319,
            1e-6,
            "tall closing",
        );
        // Under reduced motion each stage is there at once.
        let mut still = PileMotion::default();
        still.step(PileStep::Tall(true), 0.0, true);
        still.step(PileStep::Wide(true), 0.0, true);
        assert_eq!((still.tall.value(0.0), still.wide.value(0.0)), (1.0, 1.0));
        assert!(!still.moving(0.0));
    }

    /// The coins: laid open on `LAY`, a coin to the front on `REORDER`, a
    /// coin a trace is in grown on `GROW` past 1.3 and back; a coin new to
    /// the pile at its place at once.
    #[test]
    fn the_coins_lay_reorder_and_grow_on_the_macs_springs() {
        let ids = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut pile = PileMotion::default();
        pile.follow(0.0, false, &ids(&["a", "b", "c"]), &[], false);
        assert!(!pile.moving(0.0), "the first coins are simply there");
        assert_eq!(pile.coin("c").unwrap().index.value(0.0), 2.0);
        pile.follow(1.0, true, &ids(&["a", "b", "c"]), &[], false);
        close(pile.lay.value(1.1), 0.582_676_783, 1e-6, "laid open");
        // c comes to the front: a and b each move back one.
        pile.follow(2.0, true, &ids(&["c", "a", "b"]), &ids(&["c"]), false);
        let c = pile.coin("c").unwrap();
        close(
            c.index.value(2.1),
            2.0 * (1.0 - 0.496_621_349),
            1e-6,
            "c forward",
        );
        close(
            pile.coin("a").unwrap().index.value(2.1),
            0.496_621_349,
            1e-6,
            "a back",
        );
        close(
            c.grow.value(2.2),
            1.044_867_376,
            1e-6,
            "c grows past its size",
        );
        // A coin gone from the pile is forgotten; a new one is at rest.
        pile.follow(3.0, true, &ids(&["d", "c"]), &[], false);
        assert!(pile.coin("a").is_none());
        assert_eq!(pile.coin("d").unwrap().index.value(3.0), 0.0);
        assert!(!pile.coin("d").unwrap().index.moving(3.0));
        // Reduced motion: every coin at its place at once.
        pile.follow(4.0, false, &ids(&["c", "d"]), &[], true);
        assert_eq!(pile.lay.value(4.0), 0.0);
        assert_eq!(pile.coin("c").unwrap().index.value(4.0), 0.0);
        assert!(!pile.moving(4.0));
        pile.follow(5.0, true, &ids(&["d", "c"]), &[], false);
        assert!(pile.moving(5.1));
        pile.settle();
        assert!(!pile.moving(5.1));
        assert_eq!(pile.coin("c").unwrap().index.value(5.1), 1.0);
    }

    /// The covered card fades on the stack's spring: SwiftUI's value of
    /// `spring(0.36, 0.86)` at 0.1 s is 0.567733, so 0.432267 is left;
    /// past the overshoot it is gone, never below 0.
    #[test]
    fn a_covered_card_fades_on_the_stacks_spring() {
        const POPOVER: &str =
            include_str!("../../../../../macos/Sources/TCDesign/Components/PopoverNotice.swift");
        has(
            POPOVER,
            ".animation(reduceMotion ? nil : .spring(response: 0.36, dampingFraction: 0.86), value: front.id)",
        );
        has(
            POPOVER,
            "return .asymmetric(insertion: fresh ? slide : .opacity, removal: .opacity)",
        );
        assert_eq!(STACK, Ease::Spring(0.36, 0.86));
        assert_eq!(covered_opacity(0.0), 1.0);
        close(covered_opacity(0.1), 1.0 - 0.567_732_99, 1e-6, "at 0.1 s");
        assert_eq!(covered_opacity(0.3), 0.0, "past the overshoot");
        assert_eq!(covered_opacity(STACK.length()), 0.0);
    }

    /// A zoom swaps the bars in place on the token curve over 0.15 s; a
    /// step slides them.
    #[test]
    fn a_zoom_crossfades_the_bars_in_place() {
        assert_eq!(bars_swap_length(None), 0.15);
        assert_eq!(bars_swap_length(Some(Edge::Leading)), 0.45);
        let ((old_x, old_a), (new_x, new_a)) = bars_swap(None, 0.15 * 0.25);
        assert_eq!((old_x, new_x), (0.0, 0.0));
        close(new_a, 0.767_284_393, 1e-5, "new bars");
        close(old_a, 1.0 - 0.767_284_393, 1e-5, "old bars");
        assert_eq!(bars_swap(None, 0.15), ((0.0, 0.0), (0.0, 1.0)));
        assert_eq!(
            bars_swap(Some(Edge::Trailing), 0.225),
            period_slide(Edge::Trailing, 0.225)
        );
    }

    /// A tween heads for its target on its ease, from wherever it is when
    /// retargeted, and is there at once with no ease.
    #[test]
    fn a_tween_heads_for_its_target_from_where_it_is() {
        let mut tween = Tween::at_rest(0.0);
        assert!(!tween.moving(0.0));
        tween.retarget(1.0, 10.0, Some(TALL_OPEN));
        assert!(tween.moving(10.1));
        close(tween.value(10.1), 0.609_732_579, 1e-6, "tall at 0.1 s");
        // The same target again changes nothing.
        tween.retarget(1.0, 10.2, Some(TALL_CLOSE));
        close(tween.value(10.1), 0.609_732_579, 1e-6, "kept");
        // Turned back at 10.1, it starts from 0.6097 toward 0.
        tween.retarget(0.0, 10.1, Some(TALL_CLOSE));
        close(tween.value(10.1), 0.609_732_579, 1e-6, "turned back");
        close(
            tween.value(10.2),
            0.609_732_579 * (1.0 - 0.601_787_319),
            1e-6,
            "closing",
        );
        assert!(!tween.moving(10.1 + TALL_CLOSE.length()));
        assert_eq!(tween.value(10.1 + TALL_CLOSE.length()), 0.0);
        // Reduced motion: there at once.
        tween.retarget(1.0, 20.0, None);
        assert_eq!(tween.value(20.0), 1.0);
        assert!(!tween.moving(20.0));
        tween.retarget(0.0, 30.0, Some(GROW));
        tween.settle();
        assert_eq!((tween.value(30.01), tween.moving(30.01)), (0.0, false));
    }
}
