//! The animated first run's stage, as numbers: a port of the Mac's
//! `FirstRunStageLayout` (`Views/FirstRun/FirstRunStage.swift`, Ron's
//! 2026-10-21 canvas), Welcome's choreography (`WelcomeScreenLayout`), the
//! NEAR mark's marker line and shine band (`GlassBrandMark` in
//! `GlassBrandMarkDrawing.swift`), the Tools ring (`ToolsRingLayout`), the
//! Uses flow (`UsesFlowLayout`), Traces' streams (`TracesScreenLayout`) and
//! Private AI's rows (`PrivateAIScreenLayout`).
//!
//! Pure: a step, a time and whether motion is reduced go in, numbers come
//! out. [`super::first_run_stage_paint`] draws them with cairo, and the
//! tests below pin them to the values the Mac's own code prints (a probe
//! that compiles the Swift verbatim), so a wrong port fails here.
//!
//! The stage carries no words: everything it draws is decorative, as the
//! Mac hides it from assistive tech.

#![allow(dead_code)] // TEMP until the painter lands
use crate::first_run::{Sharing, Step};

/// A point in the canvas, in points.
pub type Point = (f64, f64);

/// The design's own canvas, the Mac's first-run window's default size. The
/// stage lays out in it and scales down to fit a smaller host.
pub const CANVAS: (f64, f64) = (860.0, 760.0);

/// The scale the canvas is drawn at in a host `width` by `height`: never
/// larger than the design (`FirstRunStageFit`).
pub fn fit_scale(width: f64, height: f64) -> f64 {
    1f64.min(width / CANVAS.0).min(height / CANVAS.1)
}

/// The steps the thread names: every step after Welcome.
pub const THREAD_STEPS: [Step; 6] = [
    Step::Tools,
    Step::PrivateAi,
    Step::Traces,
    Step::Uses,
    Step::Terms,
    Step::Join,
];

/// The step thread's position of a step: None on Welcome, which shows no
/// thread.
pub fn thread_index(step: Step) -> Option<usize> {
    THREAD_STEPS.iter().position(|s| *s == step)
}

/// How a stretch of the thread reads: the one before step `index` is
/// travelled once that step is reached, and the newest one leads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Segment {
    Ahead,
    Travelled,
    Leading,
}

pub fn segment(before: usize, current: usize) -> Segment {
    if before == current {
        Segment::Leading
    } else if before < current {
        Segment::Travelled
    } else {
        Segment::Ahead
    }
}

/// A thread stretch's width and the stroke it is drawn with.
pub const SEGMENT_WIDTH: f64 = 38.0;
/// The thread's top inset and its height.
pub const THREAD_TOP: f64 = 14.0;
pub const THREAD_HEIGHT: f64 = 28.0;

/// Where a tile sits on a ring of `count`, in degrees clockwise from the
/// top: evenly spaced from the top.
pub fn ring_angle(index: usize, count: usize) -> f64 {
    if count == 0 {
        return -90.0;
    }
    -90.0 + 360.0 / count as f64 * index as f64
}

/// Where a watched tile circles the N on Join: one on top, two either
/// side, three as a triangle, four on the diagonals, more evenly.
pub fn join_angle(index: usize, count: usize) -> f64 {
    match count {
        1 => -90.0,
        2 => [180.0, 0.0][index.min(1)],
        3 => [-90.0, 30.0, 150.0][index.min(2)],
        4 => [-135.0, -45.0, 45.0, 135.0][index.min(3)],
        _ => ring_angle(index, count),
    }
}

/// The point at `radius` and `degrees` round `center`.
pub fn point(center: Point, radius: f64, degrees: f64) -> Point {
    let radians = degrees.to_radians();
    (
        center.0 + radius * radians.cos(),
        center.1 + radius * radians.sin(),
    )
}

/// The scan on Tools: one turn of the sweep, after a lead-in, from 36
/// degrees before the top to the top again.
pub const SCAN_LEAD: f64 = 1.0;
pub const SCAN_TURN: f64 = 2.0;
pub const SWEEP_START: f64 = -36.0;

/// When the sweep's edge reaches tile `index` of `count`, in seconds from
/// the scan's start: it settles there, found or not.
pub fn scan_moment(index: usize, count: usize) -> f64 {
    if count == 0 {
        return SCAN_LEAD;
    }
    let angle = 360.0 / count as f64 * index as f64;
    SCAN_LEAD + SCAN_TURN * (angle - SWEEP_START) / (360.0 - SWEEP_START)
}

/// The scan is over: every tile has settled.
pub fn scan_done(elapsed: f64) -> bool {
    elapsed >= SCAN_LEAD + SCAN_TURN + 0.25
}

/// The whole scan, the ring's re-spacing included: after this the Tools
/// ring is settled.
pub const SCAN_TOTAL: f64 = SCAN_LEAD + SCAN_TURN + 0.25 + 1.0;

/// The sweep's rotation, in degrees clockwise from the top, `elapsed`
/// seconds into the scan; None before and after its turn.
pub fn sweep_rotation(elapsed: f64) -> Option<f64> {
    let into = elapsed - SCAN_LEAD;
    if !(0.0..=SCAN_TURN).contains(&into) {
        return None;
    }
    Some(SWEEP_START + (360.0 - SWEEP_START) * into / SCAN_TURN)
}

/// The ease-out cubic the count on Traces rises along.
pub fn count_ease(progress: f64) -> f64 {
    let p = progress.clamp(0.0, 1.0);
    1.0 - (1.0 - p).powi(3)
}

/// A spring-like settle for a value moving from 0 to 1 over `duration`,
/// overshooting a little: the tiles bursting out of the N.
pub fn burst(elapsed: f64, duration: f64) -> f64 {
    if elapsed <= 0.0 {
        return 0.0;
    }
    if elapsed >= duration {
        return 1.0;
    }
    let t = elapsed / duration;
    1.0 - (-6.0 * t).exp() * (7.5 * t).cos()
}

/// The token ease (0.2, 0.8, 0.2, 1) as a function of progress, as the
/// Mac's `GlassMotionCurve` samples it: the cubic Bezier solved for x by
/// Newton's method.
pub fn motion_curve(t: f64) -> f64 {
    let [x1, y1, x2, y2] = super::style::brand_tokens::first_run::EASE;
    let p = t.clamp(0.0, 1.0);
    let bezier = |s: f64, a: f64, b: f64| {
        3.0 * (1.0 - s) * (1.0 - s) * s * a + 3.0 * (1.0 - s) * s * s * b + s * s * s
    };
    let mut s = p;
    for _ in 0..8 {
        let x = bezier(s, x1, x2) - p;
        let dx = 3.0 * (1.0 - s) * (1.0 - s) * x1
            + 6.0 * (1.0 - s) * s * (x2 - x1)
            + 3.0 * s * s * (1.0 - x2);
        if dx.abs() <= 1e-6 {
            break;
        }
        s = (s - x / dx).clamp(0.0, 1.0);
    }
    bezier(s, y1, y2)
}

/// SwiftUI's `Spring(duration:bounce:)` from 0 to 1 at rest, `t` seconds
/// in: stiffness (2 pi / duration) squared, damping ratio 1 - bounce.
pub fn spring(t: f64, duration: f64, bounce: f64) -> f64 {
    if t <= 0.0 {
        return 0.0;
    }
    let omega = std::f64::consts::TAU / duration;
    let zeta = 1.0 - bounce;
    if zeta >= 1.0 {
        return 1.0 - (-omega * t).exp() * (1.0 + omega * t);
    }
    let damped = omega * (1.0 - zeta * zeta).sqrt();
    1.0 - (-zeta * omega * t).exp()
        * ((damped * t).cos() + zeta * omega / damped * (damped * t).sin())
}

/// The first run's actors moving between steps (`GlassMotion.spatial`):
/// a slow spring that settles without overshoot to speak of.
pub const SPATIAL_DURATION: f64 = 0.9;
pub const SPATIAL_BOUNCE: f64 = 0.04;
/// How long an actor's move is drawn before it is taken as settled.
pub const SPATIAL_SETTLE: f64 = 1.4;

pub fn spatial(t: f64) -> f64 {
    if t >= SPATIAL_SETTLE {
        1.0
    } else {
        spring(t, SPATIAL_DURATION, SPATIAL_BOUNCE)
    }
}

// ---- Welcome -------------------------------------------------------------

/// Welcome's choreography (Ron's canvas, rulings through 2026-10-22): the
/// N drawn like a marker in one second, the glow behind it fading in as it
/// draws, a light band crossing the mark just after the draw and then every
/// 15 seconds.
pub mod welcome {
    pub const DRAW_DELAY: f64 = 0.2;
    pub const DRAW_DURATION: f64 = 1.0;
    pub const SHINE_FIRST: f64 = 1.5;
    pub const SHINE_EVERY: f64 = 15.0;
    pub const SHINE_CROSSING: f64 = 1.35;
    /// The N's side on Welcome, and the room under it.
    pub const HUB: f64 = 132.0;

    /// How much of the mark is inked, `elapsed` seconds after Welcome opens.
    pub fn drawn(elapsed: f64, reduce_motion: bool) -> f64 {
        if reduce_motion {
            return 1.0;
        }
        ease((elapsed - DRAW_DELAY) / DRAW_DURATION)
    }

    /// Where the shine's band is across the mark, or None between crossings.
    pub fn shine(elapsed: f64, reduce_motion: bool) -> Option<f64> {
        if reduce_motion || elapsed < SHINE_FIRST {
            return None;
        }
        let into = (elapsed - SHINE_FIRST) % SHINE_EVERY;
        (into < SHINE_CROSSING).then(|| super::motion_curve(into / SHINE_CROSSING))
    }

    /// The glow behind the mark, fading in while it draws.
    pub fn glow(elapsed: f64, reduce_motion: bool) -> f64 {
        if reduce_motion {
            return 1.0;
        }
        ease((elapsed - 0.15) / 1.1)
    }

    /// The marker's ease: slow in, quick through the middle, slow out.
    pub fn ease(t: f64) -> f64 {
        let p = t.clamp(0.0, 1.0);
        if p < 0.5 {
            4.0 * p * p * p
        } else {
            1.0 - (-2.0 * p + 2.0).powi(3) / 2.0
        }
    }
}

// ---- The NEAR mark -------------------------------------------------------

/// The marker's centre line through the mark, in the brand tile's 400 by
/// 396 coordinates: from inside the lower-left hook, down and up the left
/// stem, along the diagonal, up the right stem and into the upper-right
/// hook.
pub const MARKER_LINE: [Point; 6] = [
    (167.5, 234.6),
    (101.0, 312.0),
    (101.0, 86.0),
    (299.0, 320.0),
    (299.0, 84.0),
    (226.0, 170.0),
];

/// The marker's width in the tile's coordinates: wide enough to cover
/// every stroke of the mark.
pub const MARKER_WIDTH: f64 = 44.0;

/// The marker line's length in the tile's coordinates.
pub fn marker_length() -> f64 {
    MARKER_LINE
        .windows(2)
        .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
        .sum()
}

/// The point `distance` along the marker line, clamped to its ends.
pub fn marker_point(distance: f64) -> Point {
    let mut left = distance.max(0.0);
    for w in MARKER_LINE.windows(2) {
        let (from, to) = (w[0], w[1]);
        let length = (to.0 - from.0).hypot(to.1 - from.1);
        if left <= length && length > 0.0 {
            let t = left / length;
            return (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
        }
        left -= length;
    }
    MARKER_LINE[MARKER_LINE.len() - 1]
}

/// The marker line up to `drawn` (0 to 1) of its length, as a polyline.
pub fn marker_trim(drawn: f64) -> Vec<Point> {
    let target = drawn.clamp(0.0, 1.0) * marker_length();
    let mut out = vec![MARKER_LINE[0]];
    let mut walked = 0.0;
    for w in MARKER_LINE.windows(2) {
        let length = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        if walked + length >= target {
            out.push(marker_point(target));
            return out;
        }
        walked += length;
        out.push(w[1]);
    }
    out
}

/// The light band that crosses the mark, in the tile's coordinates: a
/// 110-wide strip from y 60 to 340, leaning forward 18 degrees, its left
/// edge at `x`. Its corners, and the gradient's ends square to its lean.
pub struct ShineBand {
    pub corners: [Point; 4],
    pub from: Point,
    pub to: Point,
}

pub fn shine_band(x: f64) -> ShineBand {
    let angle = 18f64.to_radians();
    let lean = angle.tan();
    let skewed = |px: f64, py: f64| (px - lean * py, py);
    let from = skewed(x, 200.0);
    let across = 110.0 * angle.cos();
    ShineBand {
        corners: [
            skewed(x, 60.0),
            skewed(x + 110.0, 60.0),
            skewed(x + 110.0, 340.0),
            skewed(x, 340.0),
        ],
        from,
        to: (from.0 + across * angle.cos(), from.1 + across * angle.sin()),
    }
}

/// Where the shine's band starts and ends.
pub const SHINE_TRAVEL: (f64, f64) = (-240.0, 470.0);

/// The pen tip at the stroke's front while it draws: its centre in the
/// tile's coordinates, its radius, and how visible it is. None once drawn.
pub fn pen_tip(drawn: f64) -> Option<(Point, f64, f64)> {
    if drawn <= 0.0 || drawn >= 1.0 {
        return None;
    }
    let front = drawn * marker_length() + MARKER_WIDTH / 2.0 - 8.0;
    let fade = 1f64.min(drawn / 0.05).min((1.0 - drawn) / 0.08);
    Some((marker_point(front), 10.5, fade.max(0.0)))
}

/// The glow behind the N breathing (4s), or quicker and brighter while a
/// tool is watched on Tools (`charged`, 1.8s): its scale and opacity.
pub fn breathing(time: f64, charged: bool, reduce_motion: bool) -> (f64, f64) {
    if reduce_motion {
        return (1.0, 1.0);
    }
    if charged {
        let p = 0.5 - 0.5 * (std::f64::consts::TAU * time / 1.8).cos();
        return (1.02 + 0.12 * p, 0.9 + 0.1 * p);
    }
    let p = 0.5 - 0.5 * (std::f64::consts::TAU * time / 4.0).cos();
    (1.0 + 0.08 * p, 1.0 - 0.25 * p)
}

/// The corner glow's breath on the scene: seven seconds each way, eased at
/// both ends. Reduced motion holds it at its fullest.
pub fn backdrop_phase(time: f64, reduce_motion: bool) -> f64 {
    if reduce_motion {
        1.0
    } else {
        0.5 - 0.5 * (std::f64::consts::PI * time / 7.0).cos()
    }
}

// ---- Tools ---------------------------------------------------------------

/// The order the Tools ring holds every tool the shell knows, as the
/// canvas lays them out (`FirstRunStageLayout.ringOrder`).
pub const RING_ORDER: [&str; 5] = ["claude-code", "gemini-cli", "codex", "cline", "opencode"];

/// `sources` in ring order; a source the ring does not know is left out.
pub fn ring_rows<'a>(sources: &[(&'a str, bool)]) -> Vec<(&'a str, bool)> {
    RING_ORDER
        .iter()
        .filter_map(|kind| sources.iter().find(|(s, _)| s == kind).copied())
        .collect()
}

pub mod ring {
    /// The graphic's height, its centre's height, and its sizes.
    pub const HEIGHT: f64 = 300.0;
    pub const CENTER_Y: f64 = 158.0;
    pub const LOOSE_RADIUS: f64 = 130.0;
    pub const TIGHT_RADIUS: f64 = 118.0;
    pub const TILE: f64 = 52.0;
    pub const HUB: f64 = 64.0;
    pub const GONE_RADIUS: f64 = 470.0;

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Placement {
        pub angle: f64,
        pub radius: f64,
        pub scale: f64,
        pub opacity: f64,
    }

    /// Tile `index` of the ring's rows (`exists` per row), `elapsed`
    /// seconds into the scan, or settled when `elapsed` is None.
    pub fn placement(index: usize, exists: &[bool], elapsed: Option<f64>) -> Placement {
        use super::{burst, motion_curve, ring_angle, scan_done, scan_moment};
        let found: Vec<usize> = (0..exists.len()).filter(|i| exists[*i]).collect();
        let start = ring_angle(index, exists.len());
        let gone = !exists[index];
        let settled = found
            .iter()
            .position(|i| *i == index)
            .map_or(start, |at| ring_angle(at, found.len()));
        let Some(elapsed) = elapsed else {
            return if gone {
                Placement {
                    angle: start,
                    radius: GONE_RADIUS,
                    scale: 0.6,
                    opacity: 0.0,
                }
            } else {
                Placement {
                    angle: settled,
                    radius: TIGHT_RADIUS,
                    scale: 1.0,
                    opacity: 1.0,
                }
            };
        };
        let out = burst(elapsed - (0.26 + 0.08 * index as f64), 0.78);
        let mut placement = Placement {
            angle: start,
            radius: LOOSE_RADIUS * out,
            scale: 0.3 + 0.7 * out.min(1.0),
            opacity: (out * 1.6).min(1.0),
        };
        if gone {
            let leaving = elapsed - (scan_moment(index, exists.len()) + 0.12);
            let exit = (leaving / 0.8).clamp(0.0, 1.0).powi(2);
            placement.radius += (GONE_RADIUS - LOOSE_RADIUS) * exit;
            placement.scale *= 1.0 - 0.4 * exit;
            placement.opacity *= 1.0 - exit;
        } else if scan_done(elapsed) {
            let respace =
                motion_curve((elapsed - super::SCAN_LEAD - super::SCAN_TURN - 0.25) / 0.9);
            placement.angle = start + (settled - start) * respace;
            placement.radius = LOOSE_RADIUS + (TIGHT_RADIUS - LOOSE_RADIUS) * respace;
        }
        placement
    }

    /// The flash round a tile as the sweep reaches it: 0 to 1 over its half
    /// second, or None outside it.
    pub fn flash(index: usize, count: usize, elapsed: Option<f64>) -> Option<f64> {
        let elapsed = elapsed?;
        let into = (elapsed - (super::scan_moment(index, count) - 0.06)) / 0.52;
        (0.0..=1.0).contains(&into).then_some(into)
    }

    /// A tile's gentle bob, each tool on its own phase.
    pub fn bob(kind: &str, time: f64) -> f64 {
        let phase = [0.0, 1.1, 2.3, 3.4, 4.2][super::RING_ORDER
            .iter()
            .position(|k| *k == kind)
            .unwrap_or(0)];
        -2.0 + 2.0 * (std::f64::consts::TAU * (time + phase) / 5.0).cos()
    }

    /// The dashed ring's slow turn, in degrees.
    pub fn turn(time: f64, reduce_motion: bool) -> f64 {
        if reduce_motion {
            0.0
        } else {
            (time / 90.0).rem_euclid(1.0) * 360.0
        }
    }
}

// ---- Private AI ----------------------------------------------------------

pub mod private_ai {
    /// The graphic's height; the N's place; the tiles' column; NEAR AI's
    /// pill, at the end of the N's line.
    pub const HEIGHT: f64 = 221.0;
    pub const HUB: (f64, f64) = (440.0, 101.0);
    pub const HUB_SIDE: f64 = 66.0;
    pub const TILE_X: f64 = 230.0;
    pub const TILE: f64 = 42.0;
    pub const ROUTE_START: f64 = 254.0;
    pub const PILL_WIDTH: f64 = 88.0;
    pub const DESTINATION_X: f64 = 650.0;

    /// Evenly space connected-tool rows; one sits in the middle.
    pub fn row_y(index: usize, count: usize) -> f64 {
        if count <= 1 {
            return 101.0;
        }
        21.0 + 160.0 * index as f64 / (count - 1) as f64
    }
}

// ---- Traces --------------------------------------------------------------

pub mod traces {
    pub const HEIGHT: f64 = 228.0;
    pub const CENTER: (f64, f64) = (557.0, 114.0);
    pub const RING: f64 = 220.0;
    pub const HUB: f64 = 36.0;
    pub const TILE: f64 = 36.0;
    pub const TILE_X: f64 = 207.0;
    /// Where a row's stream starts when its label is not measured: the
    /// label's left edge plus the Mac's default label width and gap.
    pub const STREAM_X: f64 = 237.0 + 110.0 + 12.0;
    pub const ROW_PITCH: f64 = 62.0;
    pub const COUNT_DELAY: f64 = 0.9;
    pub const COUNT_DURATION: f64 = 1.9;

    /// How far the count has risen, `elapsed` seconds after Traces opened.
    pub fn counted(elapsed: f64, reduce_motion: bool) -> f64 {
        if reduce_motion {
            return 1.0;
        }
        super::count_ease((elapsed - COUNT_DELAY) / COUNT_DURATION)
    }

    pub fn row_y(index: usize, count: usize, center: f64) -> f64 {
        center - (count as f64 - 1.0) * ROW_PITCH / 2.0 + index as f64 * ROW_PITCH
    }

    /// A stream from just past a row's label into the ring.
    pub fn stream(start: super::Point, end: super::Point) -> [super::Point; 4] {
        [start, (start.0 + 52.0, start.1), (end.0 - 36.0, end.1), end]
    }

    /// A point on a cubic Bezier.
    pub fn bezier(curve: [super::Point; 4], t: f64) -> super::Point {
        let u = 1.0 - t;
        let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        (
            a * curve[0].0 + b * curve[1].0 + c * curve[2].0 + d * curve[3].0,
            a * curve[0].1 + b * curve[1].1 + c * curve[2].1 + d * curve[3].1,
        )
    }

    /// Whether the orbs flow down the streams `elapsed` seconds in.
    pub fn flowing(elapsed: f64, reduce_motion: bool) -> bool {
        !reduce_motion && elapsed >= 0.5 && elapsed < COUNT_DELAY + COUNT_DURATION + 0.4
    }
}

// ---- Uses ----------------------------------------------------------------

/// The Uses flow (`UsesFlowLayout`): an orb runs along the line from This
/// Mac, through the review gate, to the commons, and each one that arrives
/// takes the next of five places on the ring round the N.
pub mod uses {
    use super::Sharing;

    pub const LINE_START: f64 = 270.0;
    pub const LINE_END: f64 = 600.0;
    pub const LINE_Y: f64 = 46.0;
    pub const HEIGHT: f64 = 112.0;
    pub const MAC_X: f64 = 210.0;
    pub const GATE_X: f64 = 430.0;
    pub const COMMONS_X: f64 = 650.0;
    pub const HUB: f64 = 52.0;

    pub fn period(path: Sharing) -> f64 {
        if path == Sharing::Automatic { 2.6 } else { 4.4 }
    }
    pub fn delay(path: Sharing) -> f64 {
        if path == Sharing::Automatic { 0.6 } else { 1.0 }
    }
    pub fn arrival(path: Sharing) -> f64 {
        if path == Sharing::Automatic {
            0.92
        } else {
            0.93
        }
    }

    /// How far through its loop the flow is; None before the first loop.
    pub fn looped(path: Sharing, elapsed: f64) -> Option<f64> {
        let into = elapsed - delay(path);
        (into >= 0.0).then(|| (into / period(path)) % 1.0)
    }

    /// The orb: how far along the line (0 to 1) and how visible.
    pub fn orb(path: Sharing, t: f64) -> (f64, f64) {
        match path {
            Sharing::Ask => (
                keyframes(
                    t,
                    &[
                        (0.0, 0.0),
                        (0.32, 0.397),
                        (0.64, 0.397),
                        (0.71, 0.573),
                        (0.93, 1.0),
                        (1.0, 1.0),
                    ],
                ),
                keyframes(t, &[(0.0, 0.0), (0.05, 1.0), (0.93, 1.0), (1.0, 0.0)]),
            ),
            Sharing::Automatic => (
                keyframes(t, &[(0.0, 0.0), (0.92, 1.0), (1.0, 1.0)]),
                keyframes(t, &[(0.0, 0.0), (0.08, 1.0), (0.92, 1.0), (1.0, 0.0)]),
            ),
        }
    }

    /// The gate: its green fill, and the thumbs-up's scale, turn (degrees)
    /// and whiteness.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Gate {
        pub fill: f64,
        pub scale: f64,
        pub turn: f64,
        pub white: f64,
    }

    pub const GATE_REST: Gate = Gate {
        fill: 0.0,
        scale: 1.0,
        turn: 0.0,
        white: 0.0,
    };

    pub fn gate(path: Sharing, t: f64) -> Gate {
        match path {
            Sharing::Ask => Gate {
                fill: keyframes(
                    t,
                    &[
                        (0.0, 0.0),
                        (0.33, 0.0),
                        (0.37, 1.0),
                        (0.6, 1.0),
                        (0.68, 0.0),
                        (1.0, 0.0),
                    ],
                ),
                scale: keyframes(
                    t,
                    &[
                        (0.0, 0.86),
                        (0.33, 0.86),
                        (0.36, 1.34),
                        (0.4, 0.94),
                        (0.44, 1.08),
                        (0.48, 1.02),
                        (0.6, 1.02),
                        (0.68, 0.86),
                        (1.0, 0.86),
                    ],
                ),
                turn: keyframes(
                    t,
                    &[
                        (0.0, 0.0),
                        (0.33, 0.0),
                        (0.36, -18.0),
                        (0.4, 9.0),
                        (0.44, -4.0),
                        (0.48, 0.0),
                        (1.0, 0.0),
                    ],
                ),
                white: keyframes(
                    t,
                    &[
                        (0.0, 0.0),
                        (0.33, 0.0),
                        (0.36, 1.0),
                        (0.6, 1.0),
                        (0.68, 0.0),
                        (1.0, 0.0),
                    ],
                ),
            },
            Sharing::Automatic => Gate {
                fill: keyframes(
                    t,
                    &[
                        (0.0, 0.0),
                        (0.39, 0.0),
                        (0.42, 1.0),
                        (0.54, 1.0),
                        (0.62, 0.0),
                        (1.0, 0.0),
                    ],
                ),
                scale: keyframes(
                    t,
                    &[
                        (0.0, 0.86),
                        (0.39, 0.86),
                        (0.42, 1.3),
                        (0.45, 0.95),
                        (0.48, 1.02),
                        (0.54, 1.02),
                        (0.62, 0.86),
                        (1.0, 0.86),
                    ],
                ),
                turn: keyframes(
                    t,
                    &[
                        (0.0, 0.0),
                        (0.39, 0.0),
                        (0.42, -16.0),
                        (0.45, 8.0),
                        (0.48, 0.0),
                        (1.0, 0.0),
                    ],
                ),
                white: keyframes(
                    t,
                    &[
                        (0.0, 0.0),
                        (0.39, 0.0),
                        (0.42, 1.0),
                        (0.54, 1.0),
                        (0.62, 0.0),
                        (1.0, 0.0),
                    ],
                ),
            },
        }
    }

    /// The commons' soft pulse as an orb arrives: its scale and opacity.
    pub fn pulse(path: Sharing, t: f64) -> (f64, f64) {
        let (at, peak) = if path == Sharing::Automatic {
            (0.9, 0.93)
        } else {
            (0.92, 0.94)
        };
        (
            keyframes(t, &[(0.0, 0.7), (at, 0.7), (peak, 0.75), (1.0, 1.45)]),
            keyframes(t, &[(0.0, 0.0), (at, 0.0), (peak, 0.9), (1.0, 0.0)]),
        )
    }

    /// How many of the ring's five places are filled: one more at each
    /// arrival, and once all five are, they clear 1.3 seconds later.
    pub fn satellites(path: Sharing, elapsed: f64) -> usize {
        let first = delay(path) + period(path) * arrival(path);
        if elapsed < first {
            return 0;
        }
        let since = elapsed - first;
        let index = (since / period(path)) as usize;
        let filled = index % 5 + 1;
        if filled == 5 && since - index as f64 * period(path) >= 1.3 {
            return 0;
        }
        filled
    }

    /// Linear interpolation through `(time, value)` points.
    pub fn keyframes(t: f64, points: &[(f64, f64)]) -> f64 {
        let Some(first) = points.first() else {
            return 0.0;
        };
        if t <= first.0 {
            return first.1;
        }
        for w in points.windows(2) {
            let (a, b) = (w[0], w[1]);
            if t <= b.0 {
                let span = b.0 - a.0;
                return if span > 0.0 {
                    a.1 + (b.1 - a.1) * (t - a.0) / span
                } else {
                    b.1
                };
            }
        }
        points[points.len() - 1].1
    }
}

// ---- Join and Terms ------------------------------------------------------

pub mod join {
    /// The orbit: its box, the N, the ring, and the tiles' radius and side.
    pub const BOX: f64 = 166.0;
    pub const HUB: f64 = 56.0;
    pub const RING: f64 = 140.0;
    pub const TILE_RADIUS: f64 = 70.0;
    pub const TILE: f64 = 26.0;
    /// A sign-in's ring of light, over 1.6 seconds: its scale and opacity.
    pub fn pulse(since: f64) -> Option<(f64, f64)> {
        let p = since / 1.6;
        if !(0.0..1.0).contains(&p) {
            return None;
        }
        let opacity = if p < 0.18 { p / 0.18 } else { (1.0 - p) / 0.82 };
        Some((0.3 + 1.6 * p, opacity))
    }
}

/// Terms' N, above its title.
pub const TERMS_HUB: f64 = 56.0;

// ---- Where each step draws its actors ------------------------------------

/// A square actor's frame: its centre and side, in the step graphic's
/// canvas coordinates (x across the 860-wide canvas, y down the graphic).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub center: Point,
    pub side: f64,
}

/// The step graphic's height in the canvas, or None where the step draws
/// none (the agent prompt).
pub fn graphic_height(step: Step) -> f64 {
    match step {
        Step::Welcome => welcome::HUB,
        Step::Tools => ring::HEIGHT,
        Step::PrivateAi => private_ai::HEIGHT,
        Step::Traces => traces::HEIGHT,
        Step::Uses => uses::HEIGHT,
        Step::Terms => TERMS_HUB,
        Step::Join => join::BOX,
    }
}

/// Where the N rests on `step`, settled.
pub fn hub_frame(step: Step) -> Frame {
    let mid = CANVAS.0 / 2.0;
    match step {
        Step::Welcome => Frame {
            center: (mid, welcome::HUB / 2.0),
            side: welcome::HUB,
        },
        Step::Tools => Frame {
            center: (mid, ring::CENTER_Y),
            side: ring::HUB,
        },
        Step::PrivateAi => Frame {
            center: private_ai::HUB,
            side: private_ai::HUB_SIDE,
        },
        Step::Traces => Frame {
            center: (traces::CENTER.0, traces::CENTER.1 - 52.0),
            side: traces::HUB,
        },
        Step::Uses => Frame {
            center: (uses::COMMONS_X, uses::LINE_Y),
            side: uses::HUB,
        },
        Step::Terms => Frame {
            center: (mid, TERMS_HUB / 2.0),
            side: TERMS_HUB,
        },
        Step::Join => Frame {
            center: (mid, join::BOX / 2.0),
            side: join::HUB,
        },
    }
}

/// What the stage knows of the tools: discovery's rows in ring order (a
/// source and whether it was found) and which are watched.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolsSeen {
    pub rows: Vec<(String, bool)>,
    pub watched: Vec<String>,
    /// The found tools Private AI can connect, in ring order.
    pub connectable: Vec<String>,
}

impl ToolsSeen {
    /// The watched tools in ring order.
    pub fn watched_in_ring_order(&self) -> Vec<&str> {
        RING_ORDER
            .iter()
            .copied()
            .filter(|k| self.watched.iter().any(|w| w == k))
            .collect()
    }
}

/// Where tool `kind`'s tile rests on `step`, settled; None where the step
/// shows no tile for it. `uses_spread` is the gap between tiles on This
/// Mac's box.
pub fn tile_frame(step: Step, kind: &str, tools: &ToolsSeen) -> Option<Frame> {
    let mid = CANVAS.0 / 2.0;
    match step {
        Step::Welcome | Step::Terms => None,
        Step::Tools => {
            let exists: Vec<bool> = tools.rows.iter().map(|(_, e)| *e).collect();
            let index = tools.rows.iter().position(|(k, _)| k == kind)?;
            let place = ring::placement(index, &exists, None);
            if place.opacity <= 0.0 {
                return None;
            }
            Some(Frame {
                center: point((mid, ring::CENTER_Y), place.radius, place.angle),
                side: ring::TILE * place.scale,
            })
        }
        Step::PrivateAi => {
            let index = tools.connectable.iter().position(|k| k == kind)?;
            Some(Frame {
                center: (
                    private_ai::TILE_X,
                    private_ai::row_y(index, tools.connectable.len()),
                ),
                side: private_ai::TILE,
            })
        }
        Step::Traces => {
            let watched = tools.watched_in_ring_order();
            let index = watched.iter().position(|k| *k == kind)?;
            Some(Frame {
                center: (
                    traces::TILE_X,
                    traces::row_y(index, watched.len(), traces::CENTER.1),
                ),
                side: traces::TILE,
            })
        }
        Step::Uses => {
            let watched = tools.watched_in_ring_order();
            let index = watched.iter().position(|k| *k == kind)?;
            let spread = if watched.len() > 3 { 19.0 } else { 28.0 };
            Some(Frame {
                center: (
                    uses::MAC_X + (index as f64 - (watched.len() as f64 - 1.0) / 2.0) * spread,
                    uses::LINE_Y - 8.0,
                ),
                side: 24.0,
            })
        }
        Step::Join => {
            let watched = tools.watched_in_ring_order();
            let index = watched.iter().position(|k| *k == kind)?;
            let corner = (mid - join::BOX / 2.0, 0.0);
            let at = point(
                (corner.0 + join::BOX / 2.0, join::BOX / 2.0),
                join::TILE_RADIUS,
                join_angle(index, watched.len()),
            );
            Some(Frame {
                center: at,
                side: join::TILE,
            })
        }
    }
}

/// A frame part way, `t` (0 to 1, a spring may pass 1), from `from` to `to`.
pub fn tween(from: Frame, to: Frame, t: f64) -> Frame {
    Frame {
        center: (
            from.center.0 + (to.center.0 - from.center.0) * t,
            from.center.1 + (to.center.1 - from.center.1) * t,
        ),
        side: from.side + (to.side - from.side) * t,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Values printed by the Mac's own Swift (copied verbatim into a probe),
    /// to nine places.
    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }
    macro_rules! near {
        ($a:expr, $b:expr) => {{
            let (a, b) = ($a, $b);
            assert!(close(a, b), "{a} != {b}");
        }};
    }

    #[test]
    fn the_canvas_scales_down_to_fit_and_never_up() {
        assert_eq!(fit_scale(860.0, 760.0), 1.0);
        assert_eq!(fit_scale(1200.0, 900.0), 1.0);
        near!(fit_scale(680.0, 720.0), 680.0 / 860.0);
        near!(fit_scale(900.0, 380.0), 0.5);
    }

    #[test]
    fn the_thread_names_every_step_after_welcome() {
        assert_eq!(thread_index(Step::Welcome), None);
        let indices: Vec<_> = THREAD_STEPS.iter().map(|s| thread_index(*s)).collect();
        assert_eq!(
            indices,
            vec![Some(0), Some(1), Some(2), Some(3), Some(4), Some(5)]
        );
        assert_eq!(segment(0, 2), Segment::Travelled);
        assert_eq!(segment(1, 2), Segment::Travelled);
        assert_eq!(segment(2, 2), Segment::Leading);
        assert_eq!(segment(3, 2), Segment::Ahead);
    }

    #[test]
    fn rings_and_join_angles_match_the_mac() {
        near!(ring_angle(0, 0), -90.0);
        for (i, want) in [-90.0, 30.0, 150.0].iter().enumerate() {
            near!(ring_angle(i, 3), *want);
        }
        for (i, want) in [-90.0, -18.0, 54.0, 126.0, 198.0].iter().enumerate() {
            near!(ring_angle(i, 5), *want);
            near!(join_angle(i, 5), *want);
        }
        near!(join_angle(0, 1), -90.0);
        near!(join_angle(0, 2), 180.0);
        near!(join_angle(1, 2), 0.0);
        for (i, want) in [-90.0, 30.0, 150.0].iter().enumerate() {
            near!(join_angle(i, 3), *want);
        }
        for (i, want) in [-135.0, -45.0, 45.0, 135.0].iter().enumerate() {
            near!(join_angle(i, 4), *want);
        }
    }

    #[test]
    fn the_scan_sweeps_as_the_mac_does() {
        for (i, want) in [1.181818182, 1.787878788, 2.393939394].iter().enumerate() {
            near!(scan_moment(i, 3), *want);
        }
        for (i, want) in [
            1.181818182,
            1.545454545,
            1.909090909,
            2.272727273,
            2.636363636,
        ]
        .iter()
        .enumerate()
        {
            near!(scan_moment(i, 5), *want);
        }
        assert_eq!(sweep_rotation(0.5), None);
        near!(sweep_rotation(1.0).unwrap(), -36.0);
        near!(sweep_rotation(2.0).unwrap(), 162.0);
        near!(sweep_rotation(3.0).unwrap(), 360.0);
        assert_eq!(sweep_rotation(3.3), None);
        assert!(!scan_done(3.0));
        assert!(scan_done(3.3));
    }

    #[test]
    fn the_eases_springs_and_bursts_match_the_mac() {
        near!(count_ease(-0.5), 0.0);
        near!(count_ease(0.25), 0.578125);
        near!(count_ease(0.5), 0.875);
        near!(count_ease(1.5), 1.0);
        near!(burst(-1.0, 0.78), 0.0);
        near!(burst(0.1, 0.78), 0.734832702);
        near!(burst(0.3, 0.78), 1.096223573);
        near!(burst(0.5, 0.78), 0.997967236);
        near!(burst(0.78, 0.78), 1.0);
        for (t, want) in [
            (0.0, 0.0),
            (0.1, 0.396693549),
            (0.25, 0.767284301),
            (0.5, 0.946079462),
            (0.75, 0.991108187),
            (0.9, 0.998830259),
            (1.0, 1.0),
        ] {
            near!(motion_curve(t), want);
        }
        // SwiftUI's own `Spring(duration: 0.9, bounce: 0.04)`, sampled.
        for (t, want) in [
            (0.1, 0.157440324),
            (0.2, 0.416044334),
            (0.3, 0.634402070),
            (0.45, 0.839319030),
            (0.6, 0.936378333),
            (0.9, 0.992363622),
            (1.2, 0.999437369),
            (1.5, 1.000011422),
        ] {
            near!(spring(t, 0.9, 0.04), want);
        }
        // `Spring(duration: 0.78, bounce: 0.32)`, the expressive one.
        for (t, want) in [
            (0.1, 0.221075557),
            (0.2, 0.586156741),
            (0.3, 0.862920712),
            (0.45, 1.038587292),
            (0.6, 1.047968551),
            (0.9, 1.001419292),
        ] {
            near!(spring(t, 0.78, 0.32), want);
        }
    }

    #[test]
    fn welcome_draws_glows_and_shines_on_the_macs_clock() {
        for (e, drawn, shine, glow) in [
            (0.0, 0.0, None, 0.0),
            (0.3, 0.004, None, 0.010142750),
            (0.7, 0.5, None, 0.5),
            (1.2, 1.0, None, 0.999624343),
            (1.5, 1.0, Some(0.0), 1.0),
            (2.0, 1.0, Some(0.885440749), 1.0),
            (2.85, 1.0, None, 1.0),
            (16.6, 1.0, Some(0.298255714), 1.0),
            (17.0, 1.0, Some(0.885440749), 1.0),
        ] {
            near!(welcome::drawn(e, false), drawn);
            match (welcome::shine(e, false), shine) {
                (Some(a), Some(b)) => near!(a, b),
                (a, b) => assert_eq!(a, b),
            }
            near!(welcome::glow(e, false), glow);
        }
        // Reduced motion: the final frame at once.
        assert_eq!(welcome::drawn(0.0, true), 1.0);
        assert_eq!(welcome::shine(2.0, true), None);
        assert_eq!(welcome::glow(0.0, true), 1.0);
    }

    #[test]
    fn the_marker_line_and_shine_band_are_the_macs() {
        near!(marker_length(), 983.378253422);
        for (d, x, y) in [
            (0.0, 167.5, 234.6),
            (100.0, 102.332133574, 310.449516712),
            (250.0, 101.0, 164.044157109),
            (400.0, 147.479318444, 140.930103615),
            (700.0, 299.0, 254.573111695),
            (900.0, 279.956871173, 106.434370947),
            (1000.0, 226.0, 170.0),
        ] {
            let p = marker_point(d);
            near!(p.0, x);
            near!(p.1, y);
        }
        assert_eq!(MARKER_WIDTH, 44.0);
        assert_eq!(SHINE_TRAVEL, (-240.0, 470.0));
        for (x, from, to, left) in [
            (
                -240.0,
                (-304.983939247, 200.0),
                (-205.488004556, 232.328188876),
                -350.47268,
            ),
            (
                115.0,
                (50.016060753, 200.0),
                (149.511995444, 232.328188876),
                4.52730,
            ),
        ] {
            let band = shine_band(x);
            near!(band.from.0, from.0);
            near!(band.from.1, from.1);
            near!(band.to.0, to.0);
            near!(band.to.1, to.1);
            let min_x = band.corners.iter().map(|c| c.0).fold(f64::MAX, f64::min);
            assert!((min_x - left).abs() < 1e-4, "{min_x}");
        }
        // The trimmed line ends where the marker point is.
        let half = marker_trim(0.5);
        let end = marker_point(0.5 * marker_length());
        near!(half.last().unwrap().0, end.0);
        near!(half.last().unwrap().1, end.1);
        let whole = marker_trim(1.0);
        assert_eq!(whole.len(), MARKER_LINE.len());
        near!(whole[5].0, MARKER_LINE[5].0);
        near!(whole[5].1, MARKER_LINE[5].1);
    }

    #[test]
    fn the_glow_breathes_as_the_macs_hub_does() {
        assert_eq!(breathing(1.0, false, true), (1.0, 1.0));
        let (s, o) = breathing(0.0, false, false);
        near!(s, 1.0);
        near!(o, 1.0);
        let (s, o) = breathing(2.0, false, false);
        near!(s, 1.08);
        near!(o, 0.75);
        let (s, o) = breathing(0.9, true, false);
        near!(s, 1.14);
        near!(o, 1.0);
        near!(backdrop_phase(7.0, false), 1.0);
        near!(backdrop_phase(0.0, false), 0.0);
        assert_eq!(backdrop_phase(3.0, true), 1.0);
    }

    #[test]
    fn the_tools_ring_places_tiles_as_the_mac_does() {
        // Claude Code found, Gemini CLI not, Codex and Cline found,
        // OpenCode not: the Mac probe's rows.
        let exists = [true, false, true, true, false];
        let rows: Vec<(Option<f64>, usize, [f64; 4], Option<f64>)> = vec![
            (None, 0, [-90.0, 118.0, 1.0, 1.0], None),
            (None, 1, [-18.0, 470.0, 0.6, 0.0], None),
            (None, 2, [30.0, 118.0, 1.0, 1.0], None),
            (None, 3, [150.0, 118.0, 1.0, 1.0], None),
            (
                Some(0.3),
                0,
                [-90.0, 41.413565736, 0.522996123, 0.509705424],
                None,
            ),
            (Some(0.3), 2, [54.0, 0.0, 0.3, 0.0], None),
            (Some(0.8), 3, [126.0, 142.509064519, 1.0, 1.0], None),
            (Some(1.6), 0, [-90.0, 130.0, 1.0, 1.0], Some(0.919580420)),
            (Some(1.6), 1, [-18.0, 130.0, 1.0, 1.0], Some(0.220279720)),
            (
                Some(2.4),
                1,
                [-18.0, 416.639669421, 0.662776860, 0.156942149],
                None,
            ),
            (Some(2.4), 3, [126.0, 130.0, 1.0, 1.0], Some(0.360139860)),
            (Some(3.5), 2, [34.720744954, 120.360372477, 1.0, 1.0], None),
            (
                Some(3.5),
                4,
                [198.0, 423.778615702, 0.654378099, 0.135945248],
                None,
            ),
            (Some(4.0), 3, [149.915237811, 118.042381094, 1.0, 1.0], None),
        ];
        for (elapsed, index, [angle, radius, scale, opacity], flash) in rows {
            let p = ring::placement(index, &exists, elapsed);
            near!(p.angle, angle);
            near!(p.radius, radius);
            near!(p.scale, scale);
            near!(p.opacity, opacity);
            match (ring::flash(index, exists.len(), elapsed), flash) {
                (Some(a), Some(b)) => near!(a, b),
                (a, b) => assert_eq!(a, b),
            }
        }
        for (kind, want) in [
            ("claude-code", -3.937166322),
            ("gemini-cli", -2.851558583),
            ("codex", -0.247386640),
            ("cline", -0.725152021),
            ("opencode", -2.618033989),
        ] {
            near!(ring::bob(kind, 12.3), want);
        }
        assert_eq!(ring::HEIGHT, 300.0);
        assert_eq!(ring::CENTER_Y, 158.0);
        assert_eq!((ring::TILE, ring::HUB), (52.0, 64.0));
    }

    #[test]
    fn the_uses_flow_keyframes_are_the_macs() {
        let ask = [
            (0.0, [0.0, 0.0, 0.0, 0.86, 0.0, 0.0, 0.7, 0.0]),
            (0.2, [0.248125, 1.0, 0.0, 0.86, 0.0, 0.0, 0.7, 0.0]),
            (0.35, [0.397, 1.0, 0.5, 1.18, -12.0, 0.666666667, 0.7, 0.0]),
            (0.66, [0.447285714, 1.0, 0.25, 0.9, 0.0, 0.25, 0.7, 0.0]),
            (0.8, [0.747681818, 1.0, 0.0, 0.86, 0.0, 0.0, 0.7, 0.0]),
            (
                0.95,
                [1.0, 0.714285714, 0.0, 0.86, 0.0, 0.0, 0.866666667, 0.75],
            ),
        ];
        let automatic = [
            (0.2, [0.217391304, 1.0, 0.0, 0.86, 0.0, 0.0, 0.7, 0.0]),
            (0.5, [0.543478261, 1.0, 1.0, 1.02, 0.0, 1.0, 0.7, 0.0]),
            (0.95, [1.0, 0.625, 0.0, 0.86, 0.0, 0.0, 0.95, 0.642857143]),
        ];
        for (path, table) in [
            (Sharing::Ask, &ask[..]),
            (Sharing::Automatic, &automatic[..]),
        ] {
            for (t, want) in table {
                let (along, opacity) = uses::orb(path, *t);
                let g = uses::gate(path, *t);
                let (ps, po) = uses::pulse(path, *t);
                for (got, want) in [along, opacity, g.fill, g.scale, g.turn, g.white, ps, po]
                    .iter()
                    .zip(want)
                {
                    near!(*got, *want);
                }
            }
        }
        for (e, ask, automatic) in [
            (0.5, 0, 0),
            (4.0, 0, 1),
            (9.0, 1, 3),
            (20.0, 4, 2),
            (23.6, 5, 3),
            (25.0, 0, 4),
        ] {
            assert_eq!(uses::satellites(Sharing::Ask, e), ask);
            assert_eq!(uses::satellites(Sharing::Automatic, e), automatic);
        }
        near!(uses::looped(Sharing::Ask, 9.0).unwrap(), 0.818181818);
        assert_eq!(uses::looped(Sharing::Ask, 0.5), None);
    }

    #[test]
    fn traces_and_private_ai_rows_are_the_macs() {
        near!(traces::row_y(0, 3, 114.0), 52.0);
        near!(traces::row_y(1, 3, 114.0), 114.0);
        near!(traces::row_y(2, 3, 114.0), 176.0);
        near!(traces::counted(1.85, false), 0.875);
        assert_eq!(traces::counted(0.0, true), 1.0);
        near!(private_ai::row_y(0, 1), 101.0);
        near!(private_ai::row_y(0, 3), 21.0);
        near!(private_ai::row_y(2, 3), 181.0);
    }

    #[test]
    fn the_n_moves_between_each_steps_place() {
        let frames: Vec<Frame> = Step::ALL.iter().map(|s| hub_frame(*s)).collect();
        let sides: Vec<f64> = frames.iter().map(|f| f.side).collect();
        assert_eq!(sides, vec![132.0, 64.0, 66.0, 36.0, 52.0, 56.0, 56.0]);
        assert_eq!(frames[1].center, (430.0, 158.0));
        assert_eq!(frames[2].center, (440.0, 101.0));
        assert_eq!(frames[3].center, (557.0, 62.0));
        assert_eq!(frames[4].center, (650.0, 46.0));
        let mid = tween(frames[0], frames[1], 0.5);
        assert_eq!(mid.side, 98.0);
        let tools = ToolsSeen {
            rows: vec![
                ("claude-code".into(), true),
                ("gemini-cli".into(), false),
                ("codex".into(), true),
            ],
            watched: vec!["codex".into(), "claude-code".into()],
            connectable: vec!["claude-code".into(), "codex".into()],
        };
        assert_eq!(tools.watched_in_ring_order(), vec!["claude-code", "codex"]);
        assert_eq!(tile_frame(Step::Tools, "gemini-cli", &tools), None);
        let codex = tile_frame(Step::Tools, "codex", &tools).unwrap();
        let want = point((430.0, 158.0), 118.0, 90.0);
        near!(codex.center.0, want.0);
        near!(codex.center.1, want.1);
        let uses = tile_frame(Step::Uses, "codex", &tools).unwrap();
        assert_eq!(uses.center, (224.0, 38.0));
        let join = tile_frame(Step::Join, "claude-code", &tools).unwrap();
        near!(join.center.0, 430.0 - 70.0);
        near!(join.center.1, 83.0);
        assert_eq!(tile_frame(Step::Welcome, "codex", &tools), None);
    }
}
