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

    /// The title's words rise one at a time; a word past the third rises
    /// with the third.
    pub const WORD_DELAYS: [f64; 3] = [1.05, 1.14, 1.23];
    pub const BODY_DELAY: f64 = 1.45;
    pub const BUTTON_DELAY: f64 = 1.8;
    pub const RISE_DURATION: f64 = 0.76;
    /// How far below its place a line starts (`Rise`'s 18 points).
    pub const RISE_LIFT: f64 = 18.0;
    /// When the last line has risen: nothing moves after this.
    pub const RISEN_BY: f64 = BUTTON_DELAY + RISE_DURATION;

    /// The delay of the title's word at `index`.
    pub fn word_delay(index: usize) -> f64 {
        WORD_DELAYS[index.min(WORD_DELAYS.len() - 1)]
    }

    /// How far a line has risen into place (0 hidden and lowered, 1 in
    /// place), `elapsed` seconds after Welcome opens.
    pub fn risen(elapsed: f64, delay: f64, reduce_motion: bool) -> f64 {
        if reduce_motion {
            return 1.0;
        }
        super::burst(elapsed - delay, RISE_DURATION) * 0.98
            + if elapsed - delay >= RISE_DURATION {
                0.02
            } else {
                0.0
            }
    }

    /// The title's words, raised one at a time (split on spaces, as the
    /// Mac's `words`); each but the last keeps its space.
    pub fn words(title: &str) -> Vec<String> {
        let words: Vec<&str> = title.split(' ').filter(|w| !w.is_empty()).collect();
        let last = words.len().saturating_sub(1);
        words
            .iter()
            .enumerate()
            .map(|(i, w)| {
                if i < last {
                    format!("{w} ")
                } else {
                    (*w).to_owned()
                }
            })
            .collect()
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

// ---- Customize opening ---------------------------------------------------

/// Customize opening on Uses (`FirstRunFrame`'s `open`): the graphic and
/// the title give way and the card rises to the top, along the spatial
/// spring at 0.76 seconds, fading in as it comes.
pub mod customize {
    pub const DURATION: f64 = 0.76;

    /// The card `t` seconds into its rise from `dy` points off its place:
    /// the drawn offset and the opacity.
    pub fn slide(dy: f64, t: f64) -> (f64, f64) {
        if t >= super::SPATIAL_SETTLE {
            return (0.0, 1.0);
        }
        let p = super::spring(t, DURATION, super::SPATIAL_BOUNCE);
        (dy * (1.0 - p), p.clamp(0.0, 1.0))
    }
}

// ---- The marker check ----------------------------------------------------

/// The oversized check drawn like a marker stroke that runs past its box
/// (`FirstRunMarkerCheckStyle`, Ron 2026-10-22): ticked is Watch, unticked
/// is Not used. A 22-point rounded box inset 2 in a 26-point frame, and the
/// stroke in its own 44-point box at (-6, -14) from the frame, trimmed by
/// length as it draws on (0.34 s) or off (0.16 s).
pub mod marker_check {
    use super::Point;

    pub const BOX: f64 = 22.0;
    pub const BOX_INSET: f64 = 2.0;
    pub const BOX_RADIUS: f64 = 6.0;
    pub const BORDER: f64 = 1.5;
    pub const STROKE_ORIGIN: Point = (-6.0, -14.0);
    pub const STROKE_WIDTH: f64 = 3.6;
    pub const ON_DURATION: f64 = 0.34;
    pub const OFF_DURATION: f64 = 0.16;
    /// A disabled check is drawn at this opacity.
    pub const DISABLED: f64 = 0.45;

    /// The stroke in its 44 box (`FirstRunMarkerCheckShape`): a short
    /// down-stroke, then a flick up and out past the box's corner.
    pub const START: Point = (11.0, 27.0);
    pub const CURVES: [[Point; 3]; 2] = [
        [(13.0, 29.0), (15.5, 32.0), (18.0, 35.0)],
        [(22.0, 27.5), (28.0, 18.5), (35.0, 12.0)],
    ];

    /// The stroke flattened into a polyline in its 44 box, fine enough to
    /// trim by length as SwiftUI's `trim` does.
    pub fn polyline() -> Vec<Point> {
        let mut out = vec![START];
        let mut from = START;
        for [c1, c2, to] in CURVES {
            for i in 1..=64 {
                let t = f64::from(i) / 64.0;
                let u = 1.0 - t;
                let x = u * u * u * from.0
                    + 3.0 * u * u * t * c1.0
                    + 3.0 * u * t * t * c2.0
                    + t * t * t * to.0;
                let y = u * u * u * from.1
                    + 3.0 * u * u * t * c1.1
                    + 3.0 * u * t * t * c2.1
                    + t * t * t * to.1;
                out.push((x, y));
            }
            from = to;
        }
        out
    }

    /// The stroke's first `trim` of its length (0 to 1), as a polyline.
    pub fn trimmed(trim: f64) -> Vec<Point> {
        let line = polyline();
        let lengths: Vec<f64> = line
            .windows(2)
            .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
            .collect();
        let total: f64 = lengths.iter().sum();
        let mut left = total * trim.clamp(0.0, 1.0);
        let mut out = vec![line[0]];
        if left <= 0.0 {
            return Vec::new();
        }
        for (w, len) in line.windows(2).zip(&lengths) {
            if left >= *len {
                out.push(w[1]);
                left -= len;
            } else {
                let f = left / len;
                out.push((
                    w[0].0 + (w[1].0 - w[0].0) * f,
                    w[0].1 + (w[1].1 - w[0].1) * f,
                ));
                break;
            }
        }
        out
    }

    /// How much of the stroke is drawn `elapsed` seconds after the box was
    /// ticked (`on`) or unticked, along the token ease; at once with
    /// motion reduced, or with no change seen.
    pub fn trim(on: bool, elapsed: Option<f64>, reduce_motion: bool) -> f64 {
        let target = if on { 1.0 } else { 0.0 };
        let Some(elapsed) = elapsed.filter(|_| !reduce_motion) else {
            return target;
        };
        let duration = if on { ON_DURATION } else { OFF_DURATION };
        let p = super::motion_curve(elapsed / duration);
        if on { p } else { 1.0 - p }
    }

    /// Whether the stroke is still moving.
    pub fn moving(on: bool, elapsed: Option<f64>, reduce_motion: bool) -> bool {
        let duration = if on { ON_DURATION } else { OFF_DURATION };
        !reduce_motion && elapsed.is_some_and(|e| e < duration)
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

/// How much of the backdrop the stage draws. `Full` is the look; where
/// the stage cannot keep its frame rate and the breathing ellipse is a
/// real share of each frame, it leaves the breath out (`Ground`). Nothing
/// else on the stage is ever left out. (The ground's gradients, the next
/// layer down, are drawn once and painted from an image, so leaving them
/// out as well would save nothing: 0.6 ms a frame at twice the size on a
/// Mac, and no frames or CPU in the weston job.)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Backdrop {
    #[default]
    Full,
    Ground,
}

/// Watches the stage's frame rate. When the stage stays under
/// `LOW_SHARE` of the rate it asks for for `LOW_WINDOWS` windows of
/// `WINDOW` seconds in a row, it says so once, with the rate it reached
/// (`frame`), and the stage then asks whether the breath is worth
/// dropping at that rate (`breath_worth_dropping`): only if it is a
/// real share of each frame, since a stage held back by something else
/// (a compositor that delivers fewer frames, say) would lose the breath
/// and gain nothing. Either way the judgement is made once a first run:
/// a look that changes back and forth is worse than either.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lighten {
    level: Backdrop,
    /// The judgement is made.
    judged: bool,
    /// The window being counted: when it began, the rate it judges by,
    /// and the frames since.
    window: Option<(f64, f64, u32)>,
    /// The last frame counted.
    last: Option<f64>,
    /// Low windows in a row, and the frames a second the last reached.
    low: u32,
    reached: f64,
}

impl Lighten {
    /// Seconds a window counts frames over.
    pub const WINDOW: f64 = 2.0;
    /// Under this share of the rate asked for, a window is low.
    pub const LOW_SHARE: f64 = 2.0 / 3.0;
    /// Low windows in a row before the breath is judged: one stall, as
    /// the window opens, is not a slow machine.
    pub const LOW_WINDOWS: u32 = 2;
    /// Only a stage asking for at least this many frames a second is
    /// judged: the slower steps keep up anywhere.
    pub const JUDGED_FPS: f64 = 30.0;
    /// A gap this long between frames (the window hidden, a stall) starts
    /// the count again.
    pub const GAP: f64 = 3.0;
    /// The breath is dropped only when drawing it takes at least this
    /// share of the time each frame now takes.
    pub const BREATH_SHARE: f64 = 0.2;

    /// The backdrop to draw now.
    pub fn level(&self) -> Backdrop {
        self.level
    }

    /// Whether the judgement has been made.
    #[cfg(test)]
    pub fn judged(&self) -> bool {
        self.judged
    }

    /// Starts the count again, as the stage starts ticking again.
    pub fn reset(&mut self) {
        self.window = None;
        self.last = None;
        self.low = 0;
    }

    /// The rate a stage that wants a frame every `interval` seconds (0:
    /// every frame the display draws) can be asked to keep on a display of
    /// `refresh_hz` (60 when unknown): the lower of the two.
    pub fn asked_fps(interval: f64, refresh_hz: Option<f64>) -> f64 {
        let display = refresh_hz.filter(|hz| *hz > 0.0).unwrap_or(60.0);
        if interval > 0.0 {
            (1.0 / interval).min(display)
        } else {
            display
        }
    }

    /// A frame the stage drew at `at` (seconds) while it asked for
    /// `asked` frames a second (`asked_fps`), or `None` when this frame's
    /// rate is not the stage's to keep (the window unfocused and held to
    /// 15 a second, or hidden). `Some` with the frames a second the stage
    /// reached, once, when it has stayed low long enough to judge.
    pub fn frame(&mut self, at: f64, asked: Option<f64>) -> Option<f64> {
        if self.judged {
            return None;
        }
        let Some(asked) = asked.filter(|fps| *fps >= Self::JUDGED_FPS) else {
            self.reset();
            return None;
        };
        let gap = self.last.is_some_and(|last| at - last > Self::GAP);
        self.last = Some(at);
        let Some((start, rate, frames)) = self.window.filter(|w| w.1 == asked && !gap) else {
            if gap || self.window.is_some_and(|w| w.1 != asked) {
                self.low = 0;
            }
            self.window = Some((at, asked, 0));
            return None;
        };
        let frames = frames + 1;
        let elapsed = at - start;
        if elapsed < Self::WINDOW {
            self.window = Some((start, rate, frames));
            return None;
        }
        self.window = Some((at, rate, 0));
        let reached = f64::from(frames) / elapsed;
        if reached < Self::LOW_SHARE * rate {
            self.low += 1;
            self.reached = reached;
        } else {
            self.low = 0;
        }
        (self.low >= Self::LOW_WINDOWS).then_some(self.reached)
    }

    /// The judgement, given that drawing the breath once takes
    /// `breath_ms` and the stage reaches `reached` frames a second: the
    /// breath goes only if it is at least `BREATH_SHARE` of each frame.
    /// True when it went.
    pub fn judge(&mut self, breath_ms: f64, reached: f64) -> bool {
        self.judged = true;
        self.window = None;
        let worth = breath_worth_dropping(breath_ms, reached);
        if worth {
            self.level = Backdrop::Ground;
        }
        worth
    }
}

/// Whether drawing the breath (`breath_ms`) is a real share of each frame
/// of a stage reaching `reached` frames a second.
pub fn breath_worth_dropping(breath_ms: f64, reached: f64) -> bool {
    reached > 0.0 && breath_ms >= Lighten::BREATH_SHARE * 1000.0 / reached
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
        !reduce_motion && (0.5..COUNT_DELAY + COUNT_DURATION + 0.4).contains(&elapsed)
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
    /// Each caption's width, centred under its node (`label`).
    pub const CAPTION_WIDTH: f64 = 160.0;
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

// ---- Motion the stage's widgets and paint share -----------------------

/// The first run's motion that is not a step graphic's own drawing: how
/// often the stage draws, the beam's fade, the ring tightening, the Uses
/// satellites popping, the Tools rows rising, the Private AI count, the
/// Traces estimate rising, the thread coming in, the steps crossfading and
/// Customize's title and graphic giving way. Each is the Mac's own curve,
/// duration and delay, as a function of the seconds since its change.
pub mod motion {
    use super::{Sharing, Step};

    /// How often the stage draws, in seconds between frames (0 is every
    /// frame the display gives): the Mac's `GlassAnimationTimeline` for
    /// each step. Welcome has no interval (display rate); Tools draws at
    /// display rate while it scans and at 30 a second after; Uses and
    /// Traces at 60; the backdrop, the N, Private AI and Join at 30.
    /// `in_flight` is a SwiftUI animation under way (an actor moving
    /// between steps, the thread filling, a beam fading, the ring
    /// tightening, a satellite popping), which SwiftUI runs at display
    /// rate whatever the timeline's interval.
    pub fn frame_interval(step: Step, scanning: bool, in_flight: bool) -> f64 {
        match step {
            _ if in_flight => 0.0,
            Step::Welcome => 0.0,
            Step::Tools if scanning => 0.0,
            Step::Uses | Step::Traces => 1.0 / 60.0,
            _ => 1.0 / 30.0,
        }
    }

    /// The beam's fade as its tile is watched or not (`GlassMotion.timed(
    /// duration: 0.5)` keyed on `isWatched`): from `from` to `to` along the
    /// token curve, `since` seconds after the change.
    pub const BEAM_FADE: f64 = 0.5;
    pub fn beam_opacity(from: f64, to: f64, since: f64, reduce_motion: bool) -> f64 {
        if reduce_motion {
            return to;
        }
        from + (to - from) * super::super::glass_motion::curve(since / BEAM_FADE)
    }

    /// The dashed ring's radius as a share of its loose 130: 118/130 once
    /// the scan is over, reached along the spatial spring (`scaleEffect`
    /// keyed on `tight`), `since` seconds after the scan ended (negative
    /// while it runs).
    pub fn ring_scale(since: f64, reduce_motion: bool) -> f64 {
        let tight = super::ring::TIGHT_RADIUS / super::ring::LOOSE_RADIUS;
        if since < 0.0 {
            return 1.0;
        }
        if reduce_motion {
            return tight;
        }
        1.0 + (tight - 1.0) * super::spatial(since)
    }

    /// The expressive spring a satellite pops in and out on
    /// (`GlassMotion.expressive(duration: 0.46)`).
    pub const SATELLITE_DURATION: f64 = 0.46;
    pub const EXPRESSIVE_BOUNCE: f64 = 0.32;
    /// How long the expressive spring is drawn before it is settled.
    pub const EXPRESSIVE_SETTLE: f64 = 1.0;

    /// Satellite `place` (0 to 4) on the Uses ring, `elapsed` seconds into
    /// the `path` flow: its scale, which is also its opacity up to 1. It
    /// springs in when its orb arrives and out when the ring clears.
    pub fn satellite(path: Sharing, elapsed: f64, place: usize, reduce_motion: bool) -> f64 {
        use super::uses::{period, satellites};
        let shown = place < satellites(path, elapsed);
        if reduce_motion {
            return if shown { 1.0 } else { 0.0 };
        }
        let pop = |since: f64| super::spring(since, SATELLITE_DURATION, EXPRESSIVE_BOUNCE);
        let Some((cycle, first)) = cycle(path, elapsed) else {
            return 0.0;
        };
        // This cycle's arrival for the place, and its clear.
        let filled_at = first + period(path) * (5 * cycle + place) as f64;
        let cleared_at = |cycle: usize| first + period(path) * (5 * cycle + 4) as f64 + 1.3;
        if shown {
            pop(elapsed - filled_at)
        } else if elapsed >= cleared_at(cycle) {
            1.0 - pop(elapsed - cleared_at(cycle))
        } else if cycle > 0 {
            1.0 - pop(elapsed - cleared_at(cycle - 1))
        } else {
            0.0
        }
    }

    /// Which round of five the flow is in `elapsed` seconds in, and when
    /// the first orb arrived; None before it.
    fn cycle(path: Sharing, elapsed: f64) -> Option<(usize, f64)> {
        use super::uses::{arrival, delay, period};
        let first = delay(path) + period(path) * arrival(path);
        (elapsed >= first).then(|| (((elapsed - first) / period(path)) as usize / 5, first))
    }

    /// Whether a satellite is springing `elapsed` seconds into the flow.
    pub fn satellites_moving(path: Sharing, elapsed: f64) -> bool {
        use super::uses::period;
        let Some((_, first)) = cycle(path, elapsed) else {
            return false;
        };
        let since = elapsed - first;
        let into = since.rem_euclid(period(path));
        let index = (since / period(path)) as usize;
        into < EXPRESSIVE_SETTLE
            || (index % 5 == 4 && (1.3..1.3 + EXPRESSIVE_SETTLE).contains(&into))
    }

    /// The Tools list's rows rising once the scan is over (`RowRise`): row
    /// `index` (the add row is `rows.count`) `since` seconds after: its
    /// drawn offset and opacity.
    pub const ROW_RISE: f64 = 0.6;
    pub const ROW_STAGGER: f64 = 0.08;
    pub const ROW_LIFT: f64 = 18.0;
    pub fn row_rise(index: usize, since: f64, reduce_motion: bool) -> (f64, f64) {
        if reduce_motion {
            return (0.0, 1.0);
        }
        let t = since - ROW_STAGGER * (index + 1) as f64;
        let p = super::spring(t, ROW_RISE, super::SPATIAL_BOUNCE);
        (ROW_LIFT * (1.0 - p), p.clamp(0.0, 1.0))
    }
    /// The whole card's fade in (`GlassMotion.timed(duration: 0.3)` on
    /// `shown`).
    pub const CARD_FADE: f64 = 0.3;
    pub fn card_fade(since: f64, reduce_motion: bool) -> f64 {
        if reduce_motion {
            return 1.0;
        }
        super::super::glass_motion::curve(since / CARD_FADE)
    }
    /// When every one of `rows` rows (and the add row) has risen.
    pub fn rows_risen_by(rows: usize) -> f64 {
        ROW_STAGGER * (rows + 1) as f64 + super::spring_settle(ROW_RISE)
    }

    /// Private AI's figure counting up to `count`, one tool at a time from
    /// 0.9 s, 0.24 s apart (`PrivateAIScreenLayout.counted`).
    pub fn private_counted(count: usize, elapsed: f64, reduce_motion: bool) -> usize {
        if reduce_motion {
            return count;
        }
        if elapsed < 0.9 {
            return 0;
        }
        count.min(1 + ((elapsed - 0.9) / 0.24) as usize)
    }

    /// Traces' estimate rising once the count is done (spatial spring at
    /// 0.64 s from 8 points low): its offset and opacity, `since` seconds
    /// after the count finished.
    pub const ESTIMATE_RISE: f64 = 0.64;
    pub const ESTIMATE_LIFT: f64 = 8.0;
    pub fn estimate_rise(since: f64, reduce_motion: bool) -> (f64, f64) {
        if reduce_motion {
            return (0.0, 1.0);
        }
        let p = super::spring(since, ESTIMATE_RISE, super::SPATIAL_BOUNCE);
        (ESTIMATE_LIFT * (1.0 - p), p.clamp(0.0, 1.0))
    }

    /// The step thread coming in, or going, with Welcome (`.transition(
    /// .opacity.combined(with: .offset(y: -10)))` on the spatial spring):
    /// `shown` its target, `since` seconds after the change: its drawn
    /// offset and opacity.
    pub const THREAD_LIFT: f64 = -10.0;
    pub fn thread_shown(shown: bool, since: f64, reduce_motion: bool) -> (f64, f64) {
        let p = if reduce_motion {
            1.0
        } else {
            super::spatial(since)
        };
        let p = if shown { p } else { 1.0 - p };
        (THREAD_LIFT * (1.0 - p), p.clamp(0.0, 1.0))
    }

    /// The thread's current title lighting (`GlassMotion.timed(duration:
    /// 0.3)?.delay(0.42)`).
    /// `style.css` carries them; the test reads both.
    #[cfg(test)]
    pub const TITLE_GLOW: f64 = 0.3;
    #[cfg(test)]
    pub const TITLE_GLOW_DELAY: f64 = 0.42;

    /// Two steps crossfading (`.transition(.opacity)` under the spatial
    /// spring): the incoming step's opacity `since` seconds after the
    /// change; the outgoing one is drawn at one minus it.
    pub fn crossfade(since: f64, reduce_motion: bool) -> f64 {
        if reduce_motion {
            return 1.0;
        }
        super::spatial(since).clamp(0.0, 1.0)
    }

    /// The small switch (`FirstRunSwitch`, 30 by 18, a 14-point knob inset
    /// 2): toggled with `GlassMotion.timed(duration: 0.2)`, the knob
    /// sliding and the colours crossing on the token curve. How far on it
    /// reads (0 off, 1 on), `since` seconds after it last changed (None:
    /// never, at rest).
    pub const SWITCH: f64 = 0.2;
    pub const SWITCH_SIZE: (f64, f64) = (30.0, 18.0);
    pub const SWITCH_KNOB: f64 = 14.0;
    pub const SWITCH_DISABLED: f64 = 0.45;
    pub fn switch_on(on: bool, since: Option<f64>, reduce_motion: bool) -> f64 {
        let p = match since {
            Some(since) if !reduce_motion => super::super::glass_motion::curve(since / SWITCH),
            _ => 1.0,
        };
        if on { p } else { 1.0 - p }
    }
    /// Whether the switch is still sliding.
    pub fn switch_moving(since: Option<f64>, reduce_motion: bool) -> bool {
        !reduce_motion && since.is_some_and(|s| s < SWITCH)
    }

    /// Customize opening: the title and the graphic fading out on the open
    /// spring (`.transition(.opacity)` under `spatial(duration: 0.76)`).
    pub fn customize_fade_out(since: f64, reduce_motion: bool) -> f64 {
        if reduce_motion || since >= super::spring_settle(super::customize::DURATION) {
            return 0.0;
        }
        1.0 - super::spring(since, super::customize::DURATION, super::SPATIAL_BOUNCE)
            .clamp(0.0, 1.0)
    }
}

/// How long a spatial-bounce (0.04) spring of `duration` is drawn before it
/// is settled: SwiftUI's `settlingDuration` for it, rounded up.
pub fn spring_settle(duration: f64) -> f64 {
    duration * SPATIAL_SETTLE / SPATIAL_DURATION
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Frames at `fps` for `seconds` from `from`, asked at `asked`; the
    /// rate reached when the watch says it has stayed low, if it did.
    fn run(
        watch: &mut Lighten,
        from: f64,
        seconds: f64,
        fps: f64,
        asked: Option<f64>,
    ) -> Option<f64> {
        let mut low = None;
        let mut at = from;
        while at < from + seconds {
            low = low.or(watch.frame(at, asked));
            at += 1.0 / fps;
        }
        low
    }

    /// A stage that keeps its rate is never judged; one that stays under
    /// two thirds of it for two windows in a row is, once, at the rate it
    /// reached.
    #[test]
    fn a_stage_that_stays_slow_is_judged_once() {
        let mut fast = Lighten::default();
        assert_eq!(run(&mut fast, 0.0, 30.0, 58.0, Some(60.0)), None);
        assert_eq!(fast.level(), Backdrop::Full);

        let mut slow = Lighten::default();
        assert_eq!(
            run(&mut slow, 0.0, 3.9, 20.0, Some(60.0)),
            None,
            "one window is not enough"
        );
        let reached = run(&mut slow, 3.9, 0.5, 20.0, Some(60.0)).expect("judged");
        assert!((reached - 20.0).abs() < 1.0, "reached {reached}");
        // Judged that the breath is not worth it: the full look stays, and
        // the stage is never judged again.
        assert!(!slow.judge(1.0, reached));
        assert!(slow.judged());
        assert_eq!(slow.level(), Backdrop::Full);
        assert_eq!(run(&mut slow, 4.4, 30.0, 5.0, Some(60.0)), None);
        assert_eq!(slow.level(), Backdrop::Full);

        // Judged worth it: the breath goes, and stays gone.
        let mut costly = Lighten::default();
        let reached = run(&mut costly, 0.0, 4.4, 12.0, Some(60.0)).expect("judged");
        assert!(costly.judge(30.0, reached));
        assert_eq!(costly.level(), Backdrop::Ground);
        run(&mut costly, 4.4, 60.0, 60.0, Some(60.0));
        assert_eq!(costly.level(), Backdrop::Ground);
    }

    /// The breath goes only when it is a fifth or more of each frame: at
    /// 38 frames a second (26 ms each, a compositor's cap in the weston
    /// job) a 2.3 ms breath stays; at 11 a second (91 ms) a 19 ms one
    /// goes and an 18 ms one stays.
    #[test]
    fn the_breath_goes_only_when_it_is_a_real_share_of_each_frame() {
        assert!(!breath_worth_dropping(2.3, 38.0));
        assert!(!breath_worth_dropping(5.0, 38.0));
        assert!(breath_worth_dropping(5.3, 38.0));
        assert!(breath_worth_dropping(19.0, 11.0));
        assert!(!breath_worth_dropping(18.0, 11.0));
        assert!(!breath_worth_dropping(50.0, 0.0));
    }

    /// What is not the stage's rate to keep is not judged: an unfocused
    /// window held to 15 a second, a step asking for less than 30, a
    /// 30 Hz display, and one stall as the window opens.
    #[test]
    fn only_a_rate_the_stage_could_keep_is_judged() {
        let mut held = Lighten::default();
        assert_eq!(run(&mut held, 0.0, 30.0, 15.0, None), None, "unfocused");
        assert_eq!(held.level(), Backdrop::Full);

        let mut slow_step = Lighten::default();
        assert_eq!(
            run(&mut slow_step, 0.0, 30.0, 12.0, Some(20.0)),
            None,
            "a step asking for 20"
        );
        assert_eq!(slow_step.level(), Backdrop::Full);

        assert_eq!(Lighten::asked_fps(1.0 / 60.0, Some(30.0)), 30.0);
        assert_eq!(Lighten::asked_fps(0.0, None), 60.0);
        assert_eq!(Lighten::asked_fps(0.0, Some(144.0)), 144.0);
        assert_eq!(Lighten::asked_fps(1.0 / 30.0, Some(144.0)), 30.0);
        let mut thirty_hz = Lighten::default();
        let asked = Some(Lighten::asked_fps(1.0 / 60.0, Some(30.0)));
        assert_eq!(
            run(&mut thirty_hz, 0.0, 30.0, 29.5, asked),
            None,
            "a 30 Hz display"
        );

        // A stall of a second at the start, then full speed.
        let mut stalled = Lighten::default();
        stalled.frame(0.0, Some(60.0));
        assert_eq!(
            run(&mut stalled, 1.0, 30.0, 60.0, Some(60.0)),
            None,
            "one stall"
        );
        assert_eq!(stalled.level(), Backdrop::Full);

        // Low windows broken by a focus change start over.
        let mut broken = Lighten::default();
        run(&mut broken, 0.0, 2.5, 20.0, Some(60.0));
        run(&mut broken, 2.5, 1.0, 15.0, None);
        assert_eq!(
            run(&mut broken, 3.5, 2.5, 20.0, Some(60.0)),
            None,
            "the count started over"
        );
        assert_eq!(broken.level(), Backdrop::Full);
        // A step that asks a new rate starts the window again too.
        let mut moved = Lighten::default();
        run(&mut moved, 0.0, 2.5, 20.0, Some(60.0));
        assert_eq!(
            run(&mut moved, 2.5, 2.5, 14.0, Some(30.0)),
            None,
            "a new rate"
        );
    }

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

    /// The small switch slides over 0.2 s on the token curve, 30 by 18 with
    /// a 14-point knob, as the Mac's `FirstRunSwitch`.
    #[test]
    fn the_small_switch_slides_as_the_macs() {
        let swift = std::fs::read_to_string(format!(
            "{}/../../macos/Sources/TraceCommonsApp/Views/FirstRun/FirstRunStage.swift",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        assert!(swift.contains("withAnimation(GlassMotion.timed(reduceMotion, duration: 0.2)) { configuration.isOn.toggle() }"));
        assert!(
            swift.contains(".frame(width: 14, height: 14)\n                        .padding(2)")
        );
        assert!(swift.contains(".frame(width: 30, height: 18)"));
        assert_eq!(motion::SWITCH_SIZE, (30.0, 18.0));
        assert_eq!(motion::SWITCH_KNOB, 14.0);
        for (t, c) in [(0.02, 0.396692276), (0.1, 0.946079254), (0.16, 0.994687080)] {
            assert!((motion::switch_on(true, Some(t), false) - c).abs() < 1e-5);
            assert!((motion::switch_on(false, Some(t), false) - (1.0 - c)).abs() < 1e-5);
        }
        assert_eq!(motion::switch_on(true, None, false), 1.0);
        assert_eq!(motion::switch_on(true, Some(0.0), true), 1.0);
        assert!(motion::switch_moving(Some(0.1), false));
        assert!(!motion::switch_moving(Some(0.25), false));
        assert!(!motion::switch_moving(Some(0.1), true));
    }

    /// The first run's CSS transitions carry the Mac's own numbers: the
    /// thread title's 0.3 s after 0.42 s, the segmented pill's 0.15 s
    /// (`fast`), Join's card border and the repo chevron's 0.22 s
    /// (`standard`), each on the token curve; read from the Swift and the
    /// generated tokens, so a wrong port fails here.
    #[test]
    fn the_first_runs_css_motion_is_the_macs() {
        let css = include_str!("style.css");
        let read = |f: &str| {
            std::fs::read_to_string(format!(
                "{}/../../macos/Sources/{f}",
                env!("CARGO_MANIFEST_DIR")
            ))
            .unwrap()
        };
        let stage_swift = read("TraceCommonsApp/Views/FirstRun/FirstRunStage.swift");
        let tools_swift = read("TraceCommonsApp/Views/FirstRun/ToolsScreen.swift");
        let join_swift = read("TraceCommonsApp/Views/FirstRun/JoinScreen.swift");
        let tokens = read("TCDesign/Generated/GlassTokens.swift");
        assert!(stage_swift.contains(".animation(GlassMotion.timed(reduceMotion, duration: 0.3)?.delay(0.42), value: current)"));
        assert!(stage_swift.contains(
            ".animation(GlassMotion.timed(reduceMotion, duration: 0.15), value: chosen)"
        ));
        assert!(tools_swift.contains(".rotationEffect(.degrees(isOpen ? 90 : 0))\n                        .animation(GlassMotion.standard(reduceMotion), value: isOpen)"));
        assert!(
            join_swift.contains(".animation(GlassMotion.standard(reduceMotion), value: chosen)")
        );
        assert!(
            tokens.contains("public static let fast: Double = 0.15"),
            "fast"
        );
        assert!(
            tokens.contains("public static let standard: Double = 0.22"),
            "standard"
        );
        let ms = |s: f64| format!("{}ms", (s * 1000.0).round() as i64);
        let curve = "cubic-bezier(0.2, 0.8, 0.2, 1)";
        let title = format!(
            "{} {curve} {}",
            ms(motion::TITLE_GLOW),
            ms(motion::TITLE_GLOW_DELAY)
        );
        assert_eq!(title, format!("300ms {curve} 420ms"));
        // The rule a selector's block holds.
        let rule = |selector: &str| {
            // The first-run motion block comes last.
            let at = css
                .rfind(&format!("{selector} {{"))
                .unwrap_or_else(|| panic!("{selector}"));
            let end = css[at..].find('}').unwrap() + at;
            css[at..end].to_owned()
        };
        let thread = rule(".tc-thread-title");
        for property in ["color", "text-shadow"] {
            assert!(
                thread.contains(&format!("{property} {title}")),
                "{property}: {thread}"
            );
        }
        let pill = rule("checkbutton.tc-first-run-segment");
        assert!(
            pill.contains(&format!("background-color 150ms {curve}")),
            "{pill}"
        );
        let join = rule(".tc-card.tc-join-card");
        assert!(
            join.contains(&format!("box-shadow 220ms {curve}")),
            "{join}"
        );
        // The repo chevron's 0.22 s turn is the shared disclosure's
        // (`glass_motion::disclosure`, pinned in `motion_css`), not a rule
        // of the first run's own.
        assert!(!css.contains("tc-first-run-disclosure"));
        assert!(include_str!("first_run.rs").contains("super::glass_motion::disclosure(&more);"));
        // The token curve the CSS names is the generated one.
        assert_eq!(
            super::super::style::brand_tokens::first_run::EASE,
            [0.2, 0.8, 0.2, 1.0]
        );
    }

    /// Every interval the Mac's first-run timelines draw at, read from the
    /// Swift, and the stage's interval for each step.
    #[test]
    fn the_stage_draws_each_step_at_the_macs_rate() {
        let dir = "../../macos/Sources/TraceCommonsApp/Views/FirstRun/";
        let read = |f: &str| {
            std::fs::read_to_string(format!("{}/{dir}{f}", env!("CARGO_MANIFEST_DIR"))).unwrap()
        };
        assert!(
            read("WelcomeScreen.swift").contains("GlassAnimationTimeline(paused: reduceMotion)")
        );
        assert!(read("ToolsScreen.swift").contains(
            "GlassAnimationTimeline(minimumInterval: scanStart == nil ? 1.0 / 30 : nil, paused: reduceMotion)"
        ));
        for f in ["UsesScreen.swift", "TracesScreen.swift"] {
            assert!(
                read(f).contains(
                    "GlassAnimationTimeline(minimumInterval: 1.0 / 60, paused: reduceMotion)"
                ),
                "{f}"
            );
        }
        for f in [
            "PrivateAIScreen.swift",
            "JoinScreen.swift",
            "FirstRunStage.swift",
        ] {
            assert!(
                read(f).contains(
                    "GlassAnimationTimeline(minimumInterval: 1.0 / 30, paused: reduceMotion)"
                ),
                "{f}"
            );
        }
        use motion::frame_interval as at;
        assert_eq!(at(Step::Welcome, false, false), 0.0);
        assert_eq!(at(Step::Tools, true, false), 0.0);
        assert_eq!(at(Step::Tools, false, false), 1.0 / 30.0);
        assert_eq!(at(Step::Uses, false, false), 1.0 / 60.0);
        assert_eq!(at(Step::Traces, false, false), 1.0 / 60.0);
        for step in [Step::PrivateAi, Step::Join, Step::Terms] {
            assert_eq!(at(step, false, false), 1.0 / 30.0, "{step:?}");
        }
        for step in [Step::Tools, Step::Uses, Step::PrivateAi, Step::Join] {
            assert_eq!(at(step, false, true), 0.0, "{step:?} in flight");
        }
        // 60 Hz frames land every 16.7 ms: at a 30 a second cap every
        // other one draws, at 60 every one, a little early or not. The
        // stage's frames pass the shell's one gate, which also holds an
        // unfocused window to 15 a second.
        let due = |since, interval| {
            crate::ui::glass_motion::frame_due(Some(0.0), since, interval).is_some()
        };
        assert!(!due(0.0167, 1.0 / 30.0));
        assert!(due(0.0333, 1.0 / 30.0));
        assert!(due(0.0160, 1.0 / 60.0));
        assert!(!due(0.0083, 1.0 / 60.0));
    }

    /// The beam fades over 0.5 s on the token curve (the Swift probe's
    /// `UnitCurve` values), from wherever it was.
    #[test]
    fn the_beam_fades_on_the_token_curve() {
        let swift = std::fs::read_to_string(format!(
            "{}/../../macos/Sources/TraceCommonsApp/Views/FirstRun/ToolsScreen.swift",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        assert!(swift.contains(".animation(GlassMotion.timed(reduceMotion, duration: 0.5), value: ToolsScreenLayout.isWatched(row, in: state))"));
        assert_eq!(motion::BEAM_FADE, 0.5);
        for (t, curve) in [
            (0.05, 0.396692276),
            (0.15, 0.827630997),
            (0.25, 0.946079254),
            (0.4, 0.994687080),
        ] {
            // SwiftUI's `UnitCurve` solves the Bezier to about 1e-6, as
            // `glass_motion`'s own test allows.
            let near5 = |a: f64, b: f64| assert!((a - b).abs() < 1e-5, "{a} != {b}");
            near5(motion::beam_opacity(0.0, 1.0, t, false), curve);
            near5(motion::beam_opacity(1.0, 0.0, t, false), 1.0 - curve);
            near5(motion::beam_opacity(0.4, 1.0, t, false), 0.4 + 0.6 * curve);
        }
        assert!(motion::beam_opacity(0.0, 1.0, 0.0, false).abs() < 1e-9);
        assert_eq!(motion::beam_opacity(0.0, 1.0, 0.0, true), 1.0);
        assert!((motion::beam_opacity(0.0, 1.0, 0.6, false) - 1.0).abs() < 1e-9);
    }

    /// The dashed ring tightens from 130 to 118 on the spatial spring
    /// (0.9 s, bounce 0.04; Swift probe values) once the scan is over.
    #[test]
    fn the_dashed_ring_tightens_on_the_spatial_spring() {
        let swift = std::fs::read_to_string(format!(
            "{}/../../macos/Sources/TraceCommonsApp/Views/FirstRun/ToolsScreen.swift",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        assert!(swift.contains(".scaleEffect(tight ? ToolsRingLayout.tightRadius / ToolsRingLayout.looseRadius : 1)\n            .animation(GlassMotion.spatial(reduceMotion), value: tight)"));
        let tight = 118.0 / 130.0;
        assert_eq!(motion::ring_scale(-0.5, false), 1.0);
        for (t, p) in [
            (0.1, 0.157440324),
            (0.2, 0.416044334),
            (0.5, 0.880779918),
            (0.8, 0.983876831),
        ] {
            near!(motion::ring_scale(t, false), 1.0 + (tight - 1.0) * p);
        }
        near!(motion::ring_scale(0.1, true), tight);
        near!(motion::ring_scale(2.0, false), tight);
    }

    /// Each satellite springs in on the expressive spring (0.46 s, bounce
    /// 0.32; Swift probe values) as its orb arrives, and out together 1.3 s
    /// after the fifth.
    #[test]
    fn satellites_pop_on_the_expressive_spring() {
        let swift = std::fs::read_to_string(format!(
            "{}/../../macos/Sources/TraceCommonsApp/Views/FirstRun/UsesScreen.swift",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        assert!(swift.contains(".scaleEffect(shown ? 1 : 0)\n            .opacity(shown ? 1 : 0)\n            .animation(GlassMotion.expressive(reduceMotion, duration: 0.46), value: filled)"));
        let probe = [
            (0.05, 0.168822492),
            (0.1, 0.478497745),
            (0.2, 0.933952998),
            (0.3, 1.053248768),
            (0.5, 1.005737063),
        ];
        for path in [Sharing::Ask, Sharing::Automatic] {
            let first = uses::delay(path) + uses::period(path) * uses::arrival(path);
            for place in 0..5 {
                let arrives = first + uses::period(path) * place as f64;
                assert_eq!(motion::satellite(path, arrives - 0.01, place, false), 0.0);
                for (t, want) in probe {
                    near!(motion::satellite(path, arrives + t, place, false), want);
                }
                // The second time round, after the clear, the same.
                let again = arrives + 5.0 * uses::period(path);
                near!(
                    motion::satellite(path, again + 0.1, place, false),
                    0.478497745
                );
                assert_eq!(motion::satellite(path, arrives + 0.01, place, true), 1.0);
            }
            let clears = first + 4.0 * uses::period(path) + 1.3;
            for place in 0..5 {
                for (t, want) in probe {
                    near!(
                        motion::satellite(path, clears + t, place, false),
                        1.0 - want
                    );
                }
            }
            assert!(motion::satellites_moving(path, first + 0.2));
            assert!(motion::satellites_moving(path, clears + 0.2));
            assert!(!motion::satellites_moving(path, first + 1.5));
        }
    }

    /// The Tools rows rise one after another on the spatial spring at 0.6 s
    /// (Swift probe values), 0.08 s apart from the first, 18 points low,
    /// and the card fades in over 0.3 s on the token curve.
    #[test]
    fn the_tools_rows_rise_as_the_macs() {
        let swift = std::fs::read_to_string(format!(
            "{}/../../macos/Sources/TraceCommonsApp/Views/FirstRun/ToolsScreen.swift",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        assert!(swift.contains(".offset(y: shown ? 0 : 18)\n            .animation(GlassMotion.spatial(reduceMotion, duration: 0.6)?.delay(0.08 * Double(index + 1)), value: shown)"));
        assert!(
            swift.contains(
                ".animation(GlassMotion.timed(reduceMotion, duration: 0.3), value: shown)"
            )
        );
        for index in [0usize, 1, 4] {
            let delay = 0.08 * (index + 1) as f64;
            assert_eq!(motion::row_rise(index, delay - 0.01, false), (18.0, 0.0));
            for (t, p) in [
                (0.05, 0.098587691),
                (0.1, 0.287061283),
                (0.2, 0.634402070),
                (0.3, 0.839319030),
                (0.5, 0.976941085),
            ] {
                let (offset, opacity) = motion::row_rise(index, delay + t, false);
                near!(offset, 18.0 * (1.0 - p));
                near!(opacity, p);
            }
            assert_eq!(motion::row_rise(index, 0.0, true), (0.0, 1.0));
        }
        for (t, c) in [
            (0.03, 0.396692276),
            (0.09, 0.827630997),
            (0.15, 0.946079254),
        ] {
            let got = motion::card_fade(t, false);
            assert!((got - c).abs() < 1e-5, "{got} != {c}");
        }
        assert_eq!(motion::card_fade(0.0, true), 1.0);
    }

    /// Private AI's figure: none before 0.9 s, then one more every 0.24 s.
    #[test]
    fn private_ai_counts_up_as_the_macs() {
        let swift = std::fs::read_to_string(format!(
            "{}/../../macos/Sources/TraceCommonsApp/Views/FirstRun/PrivateAIScreen.swift",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        assert!(swift.contains("guard elapsed >= 0.9 else { return 0 }\n        return min(count, 1 + Int((elapsed - 0.9) / 0.24))"));
        assert_eq!(motion::private_counted(3, 0.89, false), 0);
        assert_eq!(motion::private_counted(3, 0.9, false), 1);
        assert_eq!(motion::private_counted(3, 1.13, false), 1);
        assert_eq!(motion::private_counted(3, 1.15, false), 2);
        assert_eq!(motion::private_counted(3, 1.39, false), 3);
        assert_eq!(motion::private_counted(3, 9.0, false), 3);
        assert_eq!(motion::private_counted(3, 0.0, true), 3);
    }

    /// Traces' estimate rises 8 points on the spatial spring at 0.64 s
    /// (Swift probe values) once the count is done.
    #[test]
    fn the_traces_estimate_rises_as_the_macs() {
        let swift = std::fs::read_to_string(format!(
            "{}/../../macos/Sources/TraceCommonsApp/Views/FirstRun/TracesScreen.swift",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        assert!(swift.contains(".offset(y: done ? 0 : 8)\n            .animation(GlassMotion.spatial(reduceMotion, duration: 0.64), value: done)"));
        assert_eq!(motion::estimate_rise(-0.1, false), (8.0, 0.0));
        for (t, p) in [
            (0.05, 0.088423026),
            (0.1, 0.262329303),
            (0.2, 0.598536450),
            (0.5, 0.968039403),
        ] {
            let (offset, opacity) = motion::estimate_rise(t, false);
            near!(offset, 8.0 * (1.0 - p));
            near!(opacity, p);
        }
        assert_eq!(motion::estimate_rise(-1.0, true), (0.0, 1.0));
    }

    /// The thread comes in from 10 points above, faded, on the spatial
    /// spring (Swift probe values), and goes the same way; the steps
    /// crossfade on it; Customize's title and graphic fade out on the open
    /// spring (0.76 s).
    #[test]
    fn the_thread_steps_and_customize_fade_on_the_macs_springs() {
        let coordinator = std::fs::read_to_string(format!(
            "{}/../../macos/Sources/TraceCommonsApp/Views/OnboardingCoordinatorView.swift",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        assert!(coordinator.contains(".transition(.opacity.combined(with: .offset(y: -10)))"));
        assert!(
            coordinator.contains(
                ".animation(GlassMotion.spatial(reduceMotion), value: runner.state.step)"
            )
        );
        let spatial = [
            (0.1, 0.157440324),
            (0.2, 0.416044334),
            (0.5, 0.880779918),
            (0.8, 0.983876831),
        ];
        for (t, p) in spatial {
            let (offset, opacity) = motion::thread_shown(true, t, false);
            near!(offset, -10.0 * (1.0 - p));
            near!(opacity, p);
            let (offset, opacity) = motion::thread_shown(false, t, false);
            near!(offset, -10.0 * p);
            near!(opacity, 1.0 - p);
            near!(motion::crossfade(t, false), p);
        }
        assert_eq!(motion::thread_shown(true, 0.0, true), (0.0, 1.0));
        assert_eq!(motion::thread_shown(false, 0.0, true), (-10.0, 0.0));
        assert_eq!(motion::crossfade(0.0, true), 1.0);
        // Overshoot never draws past opaque.
        assert!((0..300).all(|i| motion::crossfade(f64::from(i) / 100.0, false) <= 1.0));
        let frame = std::fs::read_to_string(format!(
            "{}/../../macos/Sources/TraceCommonsApp/Views/FirstRun/FirstRunFrame.swift",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        assert!(frame.contains(
            ".animation(GlassMotion.spatial(reduceMotion, duration: 0.76), value: open)"
        ));
        for (t, p) in [(0.1, 0.204190556), (0.3, 0.726084709), (0.8, 0.994727183)] {
            near!(motion::customize_fade_out(t, false), 1.0 - p);
        }
        assert_eq!(motion::customize_fade_out(0.0, true), 0.0);
        assert_eq!(motion::customize_fade_out(0.0, false), 1.0);
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

    /// Welcome's lines rise as the Mac's `WelcomeScreenLayout.risen` does:
    /// the probe's values at each of the Mac's delays.
    #[test]
    fn welcome_lines_rise_on_the_macs_clock() {
        assert_eq!(welcome::WORD_DELAYS, [1.05, 1.14, 1.23]);
        assert_eq!(welcome::BODY_DELAY, 1.45);
        assert_eq!(welcome::BUTTON_DELAY, 1.8);
        assert_eq!(welcome::RISE_DURATION, 0.76);
        assert_eq!(welcome::RISE_LIFT, 18.0);
        assert_eq!(welcome::word_delay(0), 1.05);
        assert_eq!(welcome::word_delay(4), 1.23);
        for (delay, elapsed, want) in [
            (1.05, 0.0, 0.0),
            (1.05, 1.1, 0.398391494),
            (1.05, 1.3, 1.086348669),
            (1.05, 1.6, 0.971639245),
            (1.05, 1.9, 1.0),
            (1.14, 1.1, 0.0),
            (1.14, 1.3, 0.982258665),
            (1.14, 1.6, 0.984463941),
            (1.23, 1.3, 0.545358733),
            (1.23, 1.6, 1.026087526),
            (1.23, 1.9, 0.975321067),
            (1.45, 1.3, 0.0),
            (1.45, 1.6, 0.952889079),
            (1.45, 1.9, 0.987532067),
            (1.45, 2.1, 0.974260739),
            (1.45, 2.3, 1.0),
            (1.8, 1.6, 0.0),
            (1.8, 1.9, 0.734658265),
            (1.8, 2.1, 1.070255871),
            (1.8, 2.3, 0.975837617),
            (1.8, 2.56, 1.0),
        ] {
            near!(welcome::risen(elapsed, delay, false), want);
        }
        assert_eq!(welcome::risen(0.0, 1.8, true), 1.0);
        assert_eq!(
            welcome::words("Welcome to the app"),
            ["Welcome ", "to ", "the ", "app"]
        );
        assert_eq!(welcome::words("One"), ["One"]);
    }

    /// The Customize card rises on SwiftUI's own `Spring(duration: 0.76,
    /// bounce: 0.04)`, sampled by the probe.
    #[test]
    fn customize_opens_on_the_macs_spring() {
        for (t, want) in [
            (0.1, 0.204190556),
            (0.2, 0.503962574),
            (0.3, 0.726084709),
            (0.45, 0.902599691),
            (0.6, 0.969627858),
            (0.9, 0.998025994),
            (1.2, 0.999979151),
        ] {
            let (offset, opacity) = customize::slide(100.0, t);
            near!(offset, 100.0 * (1.0 - want));
            near!(opacity, want.min(1.0));
        }
        assert_eq!(customize::slide(100.0, 5.0), (0.0, 1.0));
    }

    /// The marker check's stroke, trimmed by length, ends where SwiftUI's
    /// `trimmedPath` ends: the probe's points, within 0.15 of a point (the
    /// largest gap, 0.14 at a 0.4 trim, is SwiftUI measuring its curves a
    /// little differently; a parametric trim instead misses by 0.45).
    #[test]
    fn the_marker_check_trims_by_length_as_the_mac_does() {
        for (t, want) in [
            (0.1, (13.611979485, 29.820528030)),
            (0.25, (17.477706909, 34.373382568)),
            (0.4, (20.442266464, 30.669713974)),
            (0.5, (22.585451126, 27.216352463)),
            (0.75, (28.333211899, 19.231334686)),
            (0.9, (32.239589691, 14.740882874)),
            (1.0, (35.0, 12.0)),
        ] {
            let end = *marker_check::trimmed(t).last().unwrap();
            assert!(
                (end.0 - want.0).hypot(end.1 - want.1) < 0.15,
                "{t}: {end:?} != {want:?}"
            );
        }
        assert!(marker_check::trimmed(0.0).is_empty());
        // Ticked draws on over 0.34 s along the token ease; unticked
        // draws off over 0.16 s; reduced motion and no change are at rest.
        near!(marker_check::trim(true, Some(0.0), false), 0.0);
        near!(marker_check::trim(true, Some(0.085), false), 0.767284301);
        near!(marker_check::trim(true, Some(0.34), false), 1.0);
        near!(
            marker_check::trim(false, Some(0.08), false),
            1.0 - 0.946079462
        );
        assert_eq!(marker_check::trim(true, Some(0.0), true), 1.0);
        assert_eq!(marker_check::trim(false, None, false), 0.0);
        assert!(marker_check::moving(true, Some(0.2), false));
        assert!(!marker_check::moving(true, Some(0.4), false));
        assert!(!marker_check::moving(false, Some(0.1), true));
    }
}
