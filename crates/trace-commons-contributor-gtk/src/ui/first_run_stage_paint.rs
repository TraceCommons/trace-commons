//! The first run's stage, drawn with cairo: a port of the Mac's views in
//! `FirstRunStage.swift` (`FirstRunBackdrop`, `FirstRunHub`,
//! `FirstRunToolFace`, `FirstRunOrb`, the step thread's stretches),
//! `GlassBrandMarkDrawing` and each step's graphic (`ToolsRing`, the
//! Private AI routes, Traces' streams and ring, `UsesFlow`, Join's orbit).
//!
//! Every number is [`super::first_run_stage`]'s; this file only paints. It
//! draws no words: the stage is decorative, as the Mac hides it from
//! assistive tech. The one exception is a logo-less tool's two-letter mark
//! (Gemini CLI, Cline), which the Mac draws on its tile too.
//!
//! SwiftUI's blurs have no cairo counterpart. Where the Mac blurs a soft
//! shape, this draws the same shape with a gradient that already falls to
//! nothing, or a few widening translucent passes; the comparison renders
//! measure what that leaves.

use std::f64::consts::{PI, TAU};

use gtk::cairo;

use super::first_run_stage::{self as stage, Frame, Point, Segment};
use super::monitor::glyph::Artwork;
use super::style::brand_tokens::first_run as tokens;
use crate::first_run::Sharing;

/// An sRGB colour with alpha, 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba(pub f64, pub f64, pub f64, pub f64);

impl Rgba {
    fn from_token((hex, alpha): tokens::Rgba) -> Self {
        let channel =
            |i: usize| f64::from(u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0)) / 255.0;
        Self(channel(1), channel(3), channel(5), alpha)
    }
    /// The colour at `alpha` times its own.
    pub fn alpha(self, alpha: f64) -> Self {
        Self(self.0, self.1, self.2, self.3 * alpha)
    }
    /// The colour at exactly `alpha` (SwiftUI's `.opacity` on a token).
    pub fn with(self, alpha: f64) -> Self {
        Self(self.0, self.1, self.2, alpha)
    }
}

/// The stage's colours in one scheme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    pub scene_base: Rgba,
    pub scene_warm: Rgba,
    pub glow: Rgba,
    pub beam: Rgba,
    pub beam_core: Rgba,
    pub card: Rgba,
    pub card_solid: Rgba,
    pub tile: Rgba,
    pub tile_shadow: Rgba,
    pub rule: Rgba,
    pub approve: Rgba,
    pub approve_deep: Rgba,
    pub hero_ink: Rgba,
    pub well: Rgba,
    pub text_primary: Rgba,
    pub text_tertiary: Rgba,
    pub tint_accent: Rgba,
    pub toggle_on: Rgba,
    pub toggle_off: Rgba,
    pub toggle_knob_off: Rgba,
    pub text_on_accent: Rgba,
    pub tool_claude: Rgba,
    pub tool_codex: Rgba,
    pub tool_antigravity: Rgba,
    pub tool_open_code: Rgba,
}

impl Palette {
    /// The scheme the stage draws in. The first run is dark only on the Mac
    /// (Ron, 2026-11-05); light is kept for the comparison renders.
    pub fn scheme(dark: bool) -> Self {
        macro_rules! pick {
            ($name:ident) => {
                Rgba::from_token(if dark {
                    tokens::dark::$name
                } else {
                    tokens::light::$name
                })
            };
        }
        Self {
            dark,
            scene_base: pick!(SCENE_BASE),
            scene_warm: pick!(SCENE_WARM),
            glow: pick!(FIRST_RUN_GLOW),
            beam: pick!(FIRST_RUN_BEAM),
            beam_core: pick!(FIRST_RUN_BEAM_CORE),
            card: pick!(FIRST_RUN_CARD),
            card_solid: pick!(FIRST_RUN_CARD_SOLID),
            tile: pick!(FIRST_RUN_TILE),
            tile_shadow: pick!(FIRST_RUN_TILE_SHADOW),
            rule: pick!(FIRST_RUN_RULE),
            approve: pick!(FIRST_RUN_APPROVE),
            approve_deep: pick!(FIRST_RUN_APPROVE_DEEP),
            hero_ink: pick!(HERO_INK),
            well: pick!(WELL_FILL),
            text_primary: pick!(TEXT_PRIMARY),
            text_tertiary: pick!(TEXT_TERTIARY),
            tint_accent: pick!(TINT_ACCENT),
            toggle_on: pick!(TOGGLE_ON),
            toggle_off: pick!(TOGGLE_OFF),
            toggle_knob_off: pick!(TOGGLE_KNOB_OFF),
            text_on_accent: pick!(TEXT_ON_ACCENT),
            tool_claude: pick!(TOOL_CLAUDE),
            tool_codex: pick!(TOOL_CODEX),
            tool_antigravity: pick!(TOOL_ANTIGRAVITY),
            tool_open_code: pick!(TOOL_OPEN_CODE),
        }
    }
}

fn source(cr: &cairo::Context, c: Rgba) {
    cr.set_source_rgba(c.0, c.1, c.2, c.3);
}

fn stop(g: &cairo::Gradient, at: f64, c: Rgba) {
    g.add_color_stop_rgba(at, c.0, c.1, c.2, c.3);
}

fn rounded(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

/// A soft shadow under a rounded rectangle: SwiftUI's `.shadow(radius:)`,
/// as a few widening translucent passes.
fn soft_shadow(
    cr: &cairo::Context,
    rect: (f64, f64, f64, f64),
    corner: f64,
    blur: f64,
    dy: f64,
    c: Rgba,
) {
    if c.3 <= 0.0 || blur <= 0.0 {
        return;
    }
    let passes = 6;
    for i in 0..passes {
        let grow = blur * (i as f64 + 1.0) / passes as f64;
        rounded(
            cr,
            rect.0 - grow,
            rect.1 - grow + dy,
            rect.2 + 2.0 * grow,
            rect.3 + 2.0 * grow,
            corner + grow,
        );
        source(cr, c.alpha(1.0 / passes as f64 * 0.9));
        let _ = cr.fill();
    }
}

/// A round glow `radius` across at `center`: a radial fall-off from `c`.
fn glow_disc(cr: &cairo::Context, center: Point, radius: f64, c: Rgba) {
    let g = cairo::RadialGradient::new(center.0, center.1, 0.0, center.0, center.1, radius);
    stop(&g, 0.0, c);
    stop(&g, 1.0, c.with(0.0));
    let _ = cr.set_source(&g);
    cr.arc(center.0, center.1, radius, 0.0, TAU);
    let _ = cr.fill();
}

/// The pen tip's shadow (`.shadow(color: tipGlow, radius: radius * 1.6)`
/// on a disc `radius` across): the disc blurred by a Gaussian whose
/// standard deviation is the shadow's radius, at twice the disc's own
/// weight. Both were fitted to the Mac's render with the tip drawn and
/// without it (`welcome-hub-0.8`): standard deviation 0.95 times the
/// shadow radius, weight 2.0, worst error 0.004 in alpha from 8 to 28
/// points out.
pub const TIP_SHADOW: f64 = 1.6;
pub const TIP_SIGMA: f64 = 0.95 * TIP_SHADOW;
pub const TIP_SHADOW_WEIGHT: f64 = 2.0;

/// The tip's halo at `d` disc radii from its centre, as a fraction of its
/// colour's alpha: [`TIP_SHADOW_WEIGHT`] times the disc blurred.
pub fn tip_halo_at(d: f64) -> f64 {
    (TIP_SHADOW_WEIGHT * blurred_disc(d, TIP_SIGMA)).min(1.0)
}

/// How much of a unit disc, blurred by a Gaussian of standard deviation
/// `sigma` (in disc radii), lands `d` radii from its centre.
fn blurred_disc(d: f64, sigma: f64) -> f64 {
    // A midpoint sum over the disc in polar steps: smooth enough for a
    // gradient's stops, and computed once per stop, not per pixel.
    let (rings, spokes) = (24, 48);
    let mut sum = 0.0;
    for i in 0..rings {
        let r = (f64::from(i) + 0.5) / f64::from(rings);
        for j in 0..spokes {
            let a = TAU * (f64::from(j) + 0.5) / f64::from(spokes);
            let (x, y) = (r * a.cos() - d, r * a.sin());
            sum += (-(x * x + y * y) / (2.0 * sigma * sigma)).exp() * r;
        }
    }
    let area = (1.0 / f64::from(rings)) * (TAU / f64::from(spokes));
    sum * area / (TAU * sigma * sigma)
}

/// The halo's stops, in disc radii out to where it has faded away.
fn tip_halo_stops() -> &'static [(f64, f64)] {
    static STOPS: std::sync::OnceLock<Vec<(f64, f64)>> = std::sync::OnceLock::new();
    STOPS.get_or_init(|| {
        let reach = 1.0 + 3.0 * TIP_SIGMA;
        (0..=20)
            .map(|i| {
                let d = reach * f64::from(i) / 20.0;
                (d / reach, tip_halo_at(d))
            })
            .chain([(1.0, 0.0)])
            .collect()
    })
}

/// The tip's halo around a disc `radius` across at `at`, in `c`.
fn tip_halo(cr: &cairo::Context, at: Point, radius: f64, c: Rgba) {
    let reach = radius * (1.0 + 3.0 * TIP_SIGMA);
    let g = cairo::RadialGradient::new(at.0, at.1, 0.0, at.0, at.1, reach);
    for (offset, alpha) in tip_halo_stops() {
        stop(&g, *offset, c.alpha(*alpha));
    }
    let _ = cr.set_source(&g);
    cr.arc(at.0, at.1, reach, 0.0, TAU);
    let _ = cr.fill();
}

// ---- The scene -----------------------------------------------------------

/// The navy base every step shares, with the brand-blue glow breathing from
/// the top-left corner (`FirstRunBackdrop`), over `w` by `h`. The stage
/// paints it through `backdrop_cached`; this draws it whole, for the
/// renders and as what the cached one must match.
#[cfg(test)]
pub fn backdrop(cr: &cairo::Context, w: f64, h: f64, p: &Palette, phase: f64) {
    backdrop_in(cr, w, h, p, phase, stage::Backdrop::Full);
}

/// The backdrop as much as `detail` keeps (`stage::Backdrop`): the
/// lighter look leaves out the breathing ellipse.
#[cfg(test)]
pub fn backdrop_in(
    cr: &cairo::Context,
    w: f64,
    h: f64,
    p: &Palette,
    phase: f64,
    detail: stage::Backdrop,
) {
    let _ = cr.save();
    cr.rectangle(0.0, 0.0, w, h);
    cr.clip();
    ground(cr, w, h, p);
    if detail == stage::Backdrop::Full {
        breath(cr, p, phase);
    }
    let _ = cr.restore();
}

/// `backdrop_in`, its ground painted from `cache` (drawn there once per
/// size, scheme and display `scale`) rather than drawn again: the same
/// pixels. The ground's two gradients are most of what a frame of the
/// stage costs, and they never move.
#[allow(clippy::too_many_arguments)]
pub fn backdrop_cached(
    cr: &cairo::Context,
    cache: &GroundCache,
    w: f64,
    h: f64,
    scale: i32,
    p: &Palette,
    phase: f64,
    detail: stage::Backdrop,
) {
    let _ = cr.save();
    cr.rectangle(0.0, 0.0, w, h);
    cr.clip();
    cache.paint(cr, w, h, scale, p);
    if detail == stage::Backdrop::Full {
        breath(cr, p, phase);
    }
    let _ = cr.restore();
}

/// The glow's strengths in the scheme: its strongest and its soft stop.
fn glow_strengths(p: &Palette) -> (f64, f64) {
    if p.dark { (0.5, 0.14) } else { (0.3, 0.08) }
}

/// The ground: the warm-to-navy base and the elliptical corner glow.
fn ground(cr: &cairo::Context, w: f64, h: f64, p: &Palette) {
    let reach = w.max(h) * 0.9;
    let base = cairo::RadialGradient::new(w / 2.0, 0.0, 0.0, w / 2.0, 0.0, reach);
    stop(&base, 0.0, p.scene_warm);
    stop(&base, 1.0, p.scene_base);
    let _ = cr.set_source(&base);
    let _ = cr.paint();
    let (strong, soft) = glow_strengths(p);
    // The elliptical glow: centred on the top-left corner, 1.2 widths by
    // 0.95 heights across.
    let _ = cr.save();
    cr.scale(1.2 * w, 0.95 * h);
    let corner = cairo::RadialGradient::new(0.0, 0.0, 0.0, 0.0, 0.0, 1.0);
    stop(&corner, 0.0, p.glow.with(strong));
    stop(&corner, 0.42, p.glow.with(soft));
    stop(&corner, 0.75, p.glow.with(0.0));
    let _ = cr.set_source(&corner);
    cr.arc(0.0, 0.0, 1.0, 0.0, TAU);
    let _ = cr.fill();
    let _ = cr.restore();
}

/// The breathing ellipse: 640 by 580 from (-140, -140), scaled from its
/// top-left corner, its gradient a 300-point circle.
fn breath(cr: &cairo::Context, p: &Palette, phase: f64) {
    let (strong, _) = glow_strengths(p);
    let scale = 0.9 + 0.25 * phase;
    let opacity = 0.6 + 0.4 * phase;
    let _ = cr.save();
    cr.translate(-140.0, -140.0);
    cr.scale(scale, scale);
    let g = cairo::RadialGradient::new(320.0, 290.0, 0.0, 320.0, 290.0, 300.0);
    stop(&g, 0.0, p.glow.with(strong * opacity));
    stop(&g, 1.0, p.glow.with(0.0));
    let _ = cr.set_source(&g);
    let _ = cr.save();
    cr.translate(320.0, 290.0);
    cr.scale(320.0, 290.0);
    cr.arc(0.0, 0.0, 1.0, 0.0, TAU);
    let _ = cr.restore();
    let _ = cr.fill();
    let _ = cr.restore();
}

/// What drawing the breath once costs, in milliseconds, on a stage `w` by
/// `h` at `scale` device pixels a point: the least of three draws into an
/// image of that size, as the stage's frames draw it.
pub fn breath_cost(w: f64, h: f64, scale: i32, p: &Palette) -> f64 {
    let scale = scale.max(1);
    let (width, height) = (
        (w * f64::from(scale)).ceil() as i32,
        (h * f64::from(scale)).ceil() as i32,
    );
    let Ok(surface) = cairo::ImageSurface::create(cairo::Format::ARgb32, width, height) else {
        return 0.0;
    };
    (0..3)
        .filter_map(|_| {
            let cr = cairo::Context::new(&surface).ok()?;
            cr.scale(f64::from(scale), f64::from(scale));
            let start = std::time::Instant::now();
            breath(&cr, p, 0.5);
            drop(cr);
            surface.flush();
            Some(start.elapsed().as_secs_f64() * 1e3)
        })
        .fold(f64::INFINITY, f64::min)
}

/// What the ground was drawn for: its size, the display's scale and the
/// scheme.
type GroundKey = (u64, u64, i32, bool);

/// The backdrop's ground, drawn once per size, display scale and scheme
/// at the display's pixels and then painted, one to one, on every frame.
#[derive(Default)]
pub struct GroundCache {
    kept: std::cell::RefCell<Option<(GroundKey, cairo::ImageSurface)>>,
    builds: std::cell::Cell<usize>,
}

impl GroundCache {
    /// Paints the ground for `w` by `h` at `scale` device pixels a point.
    pub fn paint(&self, cr: &cairo::Context, w: f64, h: f64, scale: i32, p: &Palette) {
        let scale = scale.max(1);
        let key = (w.to_bits(), h.to_bits(), scale, p.dark);
        let fresh = self
            .kept
            .borrow()
            .as_ref()
            .is_none_or(|(kept, _)| *kept != key);
        if fresh {
            let width = (w * f64::from(scale)).ceil() as i32;
            let height = (h * f64::from(scale)).ceil() as i32;
            let Ok(surface) = cairo::ImageSurface::create(cairo::Format::ARgb32, width, height)
            else {
                ground(cr, w, h, p);
                return;
            };
            if let Ok(c) = cairo::Context::new(&surface) {
                c.scale(f64::from(scale), f64::from(scale));
                c.rectangle(0.0, 0.0, w, h);
                c.clip();
                ground(&c, w, h, p);
            }
            surface.flush();
            self.builds.set(self.builds.get() + 1);
            self.kept.replace(Some((key, surface)));
        }
        let kept = self.kept.borrow();
        let Some((_, surface)) = kept.as_ref() else {
            return;
        };
        let _ = cr.save();
        cr.scale(1.0 / f64::from(scale), 1.0 / f64::from(scale));
        let _ = cr.set_source_surface(surface, 0.0, 0.0);
        cr.source().set_filter(cairo::Filter::Nearest);
        let _ = cr.paint();
        let _ = cr.restore();
    }

    /// How many times the ground has been drawn.
    #[cfg(test)]
    pub fn builds(&self) -> usize {
        self.builds.get()
    }
}

// ---- The step thread -----------------------------------------------------

/// One stretch of the thread, `width` across with its top-left at `origin`:
/// the dotted rule, and over it the glowing stretch, `grown` (0 to 1) of
/// its width, lit at its lead end when it leads.
pub fn thread_segment(
    cr: &cairo::Context,
    origin: Point,
    width: f64,
    segment: Segment,
    grown: f64,
    p: &Palette,
) {
    let (x, y) = origin;
    cr.set_line_width(2.0);
    cr.set_dash(&[2.0, 4.0], 0.0);
    cr.set_line_cap(cairo::LineCap::Butt);
    source(cr, p.rule);
    cr.move_to(x, y + 1.5);
    cr.line_to(x + width, y + 1.5);
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);
    let fill = width * grown.clamp(0.0, 1.0);
    if fill <= 0.0 {
        return;
    }
    // Its glow (`.shadow(color: glow 0.35, radius: 4 or 5)`).
    let blur = if segment == Segment::Leading {
        5.0
    } else {
        4.0
    };
    soft_shadow(
        cr,
        (x, y, fill, 3.0),
        1.5,
        blur,
        0.0,
        p.glow.with(0.35 * 0.5),
    );
    let g = cairo::LinearGradient::new(x, y, x + fill, y);
    stop(&g, 0.0, p.glow);
    stop(
        &g,
        1.0,
        if segment == Segment::Leading {
            p.beam_core
        } else {
            p.glow
        },
    );
    let _ = cr.set_source(&g);
    rounded(cr, x, y, fill, 3.0, 1.5);
    let _ = cr.fill();
}

// ---- The NEAR mark -------------------------------------------------------

/// The mark's place in a square `side` across with its top-left at
/// `origin`: the transform from the brand tile's coordinates.
fn mark_transform(cr: &cairo::Context, origin: Point, side: f64) {
    let [bx, by, bw, bh] = super::style::brand_tokens::logo::BRAND_MARK_VIEW_BOX;
    let fit = side.min(side * bw / bh);
    let scale = fit / bw;
    let left = origin.0 + (side - fit) / 2.0;
    let top = origin.1 + (side - bh * scale) / 2.0;
    cr.translate(left, top);
    cr.scale(scale, scale);
    cr.translate(-bx, -by);
}

/// `GlassBrandMarkDrawing` in a square `side` across at `origin`: filled
/// in the glow, inked as far as `drawn` along the marker line, the light
/// band at `shine`, and the pen tip at the stroke's front while it draws.
pub fn brand_mark(
    cr: &cairo::Context,
    origin: Point,
    side: f64,
    drawn: f64,
    shine: Option<f64>,
    tip: bool,
    p: &Palette,
) {
    if drawn <= 0.0 {
        return;
    }
    let Some(segments) = Artwork::brand_mark().cached_segments() else {
        return;
    };
    let _ = cr.save();
    mark_transform(cr, origin, side);
    super::monitor::glyph::trace_segments(cr, &segments);
    cr.set_fill_rule(cairo::FillRule::Winding);
    source(cr, p.glow);
    if drawn < 1.0 {
        // Cairo cannot clip to a stroke: clip to the mark and stroke the
        // marker's trimmed line in the fill, which inks the same pixels.
        let _ = cr.save();
        cr.clip();
        let line = stage::marker_trim(drawn);
        cr.move_to(line[0].0, line[0].1);
        for q in &line[1..] {
            cr.line_to(q.0, q.1);
        }
        cr.set_line_width(stage::MARKER_WIDTH);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_join(cairo::LineJoin::Round);
        let _ = cr.stroke();
        let _ = cr.restore();
    } else {
        let _ = cr.fill();
    }
    if let Some(shine) = shine {
        let _ = cr.save();
        super::monitor::glyph::trace_segments(cr, &segments);
        cr.clip();
        let (lo, hi) = stage::SHINE_TRAVEL;
        let band = stage::shine_band(lo + (hi - lo) * shine);
        let g = cairo::LinearGradient::new(band.from.0, band.from.1, band.to.0, band.to.1);
        stop(&g, 0.0, Rgba(1.0, 1.0, 1.0, 0.0));
        stop(&g, 0.5, Rgba(1.0, 1.0, 1.0, 0.7));
        stop(&g, 1.0, Rgba(1.0, 1.0, 1.0, 0.0));
        let _ = cr.set_source(&g);
        cr.move_to(band.corners[0].0, band.corners[0].1);
        for c in &band.corners[1..] {
            cr.line_to(c.0, c.1);
        }
        cr.close_path();
        let _ = cr.fill();
        let _ = cr.restore();
    }
    if tip {
        if let Some((at, radius, fade)) = stage::pen_tip(drawn) {
            // The shadow's glow, then the tip in the hero ink.
            tip_halo(cr, at, radius, p.glow.alpha(fade));
            cr.arc(at.0, at.1, radius, 0.0, TAU);
            source(cr, p.hero_ink.alpha(fade));
            let _ = cr.fill();
        }
    }
    let _ = cr.restore();
}

/// The marker check (`FirstRunMarkerCheck`) with its 26-point frame's
/// top-left at `origin`: the rounded box, filled in the accent's tint and
/// ringed in the glow when ticked, ringed in the text's ink when not (the
/// primary ink under the pointer), and the stroke `trim` drawn, in the
/// switch's on colour. A disabled check is drawn at
/// [`stage::marker_check::DISABLED`].
/// The small switch (`FirstRunSwitch`) with its top-left at `origin`, `on`
/// of the way on (0 to 1, as it slides): the track crossing from off to on,
/// the knob from the leading inset to the trailing one.
pub fn small_switch(cr: &cairo::Context, origin: Point, on: f64, enabled: bool, p: &Palette) {
    use stage::motion::{SWITCH_DISABLED, SWITCH_KNOB, SWITCH_SIZE};
    let mix = |a: Rgba, b: Rgba| {
        Rgba(
            a.0 + (b.0 - a.0) * on,
            a.1 + (b.1 - a.1) * on,
            a.2 + (b.2 - a.2) * on,
            a.3 + (b.3 - a.3) * on,
        )
    };
    let _ = cr.save();
    if !enabled {
        cr.push_group();
    }
    let (w, h) = SWITCH_SIZE;
    rounded(cr, origin.0, origin.1, w, h, h / 2.0);
    source(cr, mix(p.toggle_off, p.toggle_on));
    let _ = cr.fill();
    let inset = (h - SWITCH_KNOB) / 2.0;
    let x = origin.0 + inset + (w - SWITCH_KNOB - 2.0 * inset) * on;
    cr.arc(
        x + SWITCH_KNOB / 2.0,
        origin.1 + h / 2.0,
        SWITCH_KNOB / 2.0,
        0.0,
        TAU,
    );
    source(cr, mix(p.toggle_knob_off, p.text_on_accent));
    let _ = cr.fill();
    if !enabled && cr.pop_group_to_source().is_ok() {
        let _ = cr.paint_with_alpha(SWITCH_DISABLED);
    }
    let _ = cr.restore();
}

pub fn marker_check(
    cr: &cairo::Context,
    origin: Point,
    on: bool,
    trim: f64,
    hovered: bool,
    enabled: bool,
    p: &Palette,
) {
    use stage::marker_check as m;
    let _ = cr.save();
    if !enabled {
        cr.push_group();
    }
    let (x, y) = (origin.0 + m::BOX_INSET, origin.1 + m::BOX_INSET);
    if on {
        rounded(cr, x, y, m::BOX, m::BOX, m::BOX_RADIUS);
        source(cr, p.tint_accent);
        let _ = cr.fill();
    }
    // `strokeBorder`: the line inside the box's edge.
    let half = m::BORDER / 2.0;
    rounded(
        cr,
        x + half,
        y + half,
        m::BOX - m::BORDER,
        m::BOX - m::BORDER,
        m::BOX_RADIUS - half,
    );
    source(
        cr,
        if on {
            p.glow
        } else if hovered {
            p.text_primary
        } else {
            p.text_tertiary
        },
    );
    cr.set_line_width(m::BORDER);
    let _ = cr.stroke();
    let line = m::trimmed(trim);
    if let Some((first, rest)) = line.split_first() {
        let (sx, sy) = (origin.0 + m::STROKE_ORIGIN.0, origin.1 + m::STROKE_ORIGIN.1);
        cr.move_to(sx + first.0, sy + first.1);
        for point in rest {
            cr.line_to(sx + point.0, sy + point.1);
        }
        source(cr, p.toggle_on);
        cr.set_line_width(m::STROKE_WIDTH);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_join(cairo::LineJoin::Round);
        let _ = cr.stroke();
    }
    if !enabled {
        let _ = cr.pop_group_to_source();
        let _ = cr.paint_with_alpha(m::DISABLED);
    }
    let _ = cr.restore();
}

/// How the N is drawn on a step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hub {
    pub drawn: f64,
    pub shine: Option<f64>,
    pub tip: bool,
    pub glow: f64,
    pub charged: bool,
}

impl Default for Hub {
    fn default() -> Self {
        Self {
            drawn: 1.0,
            shine: None,
            tip: false,
            glow: 1.0,
            charged: false,
        }
    }
}

/// `FirstRunHub` in `frame`: the breathing glow behind the mark, then the
/// mark. `time` is the breathing clock.
pub fn hub(
    cr: &cairo::Context,
    frame: Frame,
    look: Hub,
    time: f64,
    reduce_motion: bool,
    alpha: f64,
    p: &Palette,
) {
    let side = frame.side;
    let disc = side * 220.0 / 132.0;
    let (scale, opacity) = stage::breathing(time, look.charged, reduce_motion);
    let peak = if p.dark { 0.35 } else { 0.2 };
    glow_disc(
        cr,
        frame.center,
        disc * 0.31 * scale,
        p.glow.with(peak * opacity * look.glow * alpha),
    );
    let origin = (frame.center.0 - side / 2.0, frame.center.1 - side / 2.0);
    if alpha < 1.0 {
        cr.push_group();
        brand_mark(cr, origin, side, look.drawn, look.shine, look.tip, p);
        let _ = cr.pop_group_to_source();
        let _ = cr.paint_with_alpha(alpha);
    } else {
        brand_mark(cr, origin, side, look.drawn, look.shine, look.tip, p);
    }
}

// ---- Tools, orbs ---------------------------------------------------------

/// A tool's mark on its tile: logo artwork, or two letters, and the tint.
fn tool_mark(kind: &str, p: &Palette) -> (Option<Artwork>, Rgba, &'static str) {
    let logo = |name: &str| {
        Artwork::TOOLS
            .iter()
            .find_map(|(known, art)| (*known == name).then_some(*art))
    };
    match kind {
        "claude-code" => (logo("claude"), p.tool_claude, "CC"),
        "codex" => (logo("codex"), p.tool_codex, "Cx"),
        "gemini-cli" => (None, p.tool_antigravity, "Gm"),
        "opencode" => (logo("openCode"), p.tool_open_code, "OC"),
        _ => (None, p.text_primary, "Cl"),
    }
}

/// `FirstRunToolFace` in `frame`: the raised tile, its shadow, the logo in
/// its tint or its initials, and the glow round it while `lit`.
pub fn tool_face(
    cr: &cairo::Context,
    frame: Frame,
    kind: &str,
    lit: bool,
    alpha: f64,
    p: &Palette,
) {
    if alpha <= 0.0 || frame.side <= 0.0 {
        return;
    }
    let side = frame.side;
    let (x, y) = (frame.center.0 - side / 2.0, frame.center.1 - side / 2.0);
    let corner = side * 14.0 / 52.0;
    cr.push_group();
    if lit {
        soft_shadow(
            cr,
            (x, y, side, side),
            corner,
            side * 11.0 / 52.0,
            0.0,
            p.glow.with(0.35 * 0.6),
        );
    }
    soft_shadow(
        cr,
        (x, y, side, side),
        corner,
        side * 10.0 / 52.0,
        side * 8.0 / 52.0,
        p.tile_shadow.alpha(0.5),
    );
    rounded(cr, x, y, side, side, corner);
    source(cr, p.tile);
    let _ = cr.fill();
    let (logo, tint, initials) = tool_mark(kind, p);
    source(cr, tint);
    match logo {
        Some(art) => art.fill(cr, (x + side / 4.0, y + side / 4.0), side / 2.0),
        None => {
            cr.select_font_face(
                "sans-serif",
                cairo::FontSlant::Normal,
                cairo::FontWeight::Bold,
            );
            cr.set_font_size((side * 16.0 / 52.0).max(6.0));
            if let Ok(e) = cr.text_extents(initials) {
                cr.move_to(
                    frame.center.0 - e.x_bearing() - e.width() / 2.0,
                    frame.center.1 - e.y_bearing() - e.height() / 2.0,
                );
                let _ = cr.show_text(initials);
            }
            cr.new_path();
        }
    }
    let _ = cr.pop_group_to_source();
    let _ = cr.paint_with_alpha(alpha);
}

/// `FirstRunOrb`: white at its heart fading through the beam's blue.
pub fn orb(cr: &cairo::Context, center: Point, diameter: f64, alpha: f64, p: &Palette) {
    if alpha <= 0.0 {
        return;
    }
    let r = diameter / 2.0;
    let g = cairo::RadialGradient::new(center.0, center.1, 0.0, center.0, center.1, r);
    stop(&g, 0.0, p.hero_ink.alpha(alpha));
    stop(&g, 0.22, p.beam_core.alpha(alpha));
    stop(&g, 0.46, p.beam.with(0.65 * alpha));
    stop(&g, 0.72, p.glow.with(0.0));
    stop(&g, 1.0, p.glow.with(0.0));
    let _ = cr.set_source(&g);
    cr.arc(center.0, center.1, r, 0.0, TAU);
    let _ = cr.fill();
}

fn dashed_circle(
    cr: &cairo::Context,
    center: Point,
    radius: f64,
    width: f64,
    dash: [f64; 2],
    turn: f64,
    c: Rgba,
) {
    let _ = cr.save();
    cr.translate(center.0, center.1);
    cr.rotate(turn.to_radians());
    cr.set_line_width(width);
    cr.set_dash(&dash, 0.0);
    source(cr, c);
    cr.arc(0.0, 0.0, radius, -PI / 2.0, 1.5 * PI);
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);
    let _ = cr.restore();
}

// ---- Each step's art, apart from its actors ------------------------------

/// What a step's art needs beyond the actors.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    /// The ring's rows in ring order, and which are watched.
    pub tools: stage::ToolsSeen,
    /// Seconds since the Tools scan started, while it runs; None once over.
    pub scan: Option<f64>,
    /// Seconds since the step opened (Traces' count, Private AI's flow).
    pub opened: f64,
    /// The Uses path, and seconds since it was chosen.
    pub sharing: Sharing,
    pub sharing_since: f64,
    /// Seconds since a sign-in on Join, while its ring of light runs.
    pub signed_in: Option<f64>,
    /// The breathing clock, in seconds.
    pub time: f64,
    pub reduce_motion: bool,
    /// Each found tool's beam opacity while it fades (the 0.5 s fade as it
    /// is watched or not); a tool not here is drawn at its watch.
    pub beams: std::collections::BTreeMap<String, f64>,
    /// Seconds since the scan ended and the dashed ring began tightening;
    /// None once it has long been tight.
    pub ring_since: Option<f64>,
}

/// The Tools ring's art behind the actors: the dashed ring turning slowly,
/// the sweep, and each watched tile's beam into the N.
pub fn tools_back(cr: &cairo::Context, scene: &Scene, p: &Palette) {
    let center = (stage::CANVAS.0 / 2.0, stage::ring::CENTER_Y);
    let settled = scene.scan.is_none_or(stage::scan_done);
    let opacity = ((scene.scan.unwrap_or(10.0) - 0.3) / 0.7).clamp(0.0, 1.0);
    // Loose while the scan runs, tightening on the spatial spring after.
    let since = match (scene.scan, scene.ring_since) {
        (_, Some(since)) => since,
        (Some(elapsed), None) => elapsed - (stage::SCAN_LEAD + stage::SCAN_TURN + 0.25),
        (None, None) => f64::INFINITY,
    };
    let radius = stage::ring::LOOSE_RADIUS * stage::motion::ring_scale(since, scene.reduce_motion);
    dashed_circle(
        cr,
        center,
        radius,
        1.5 * radius / stage::ring::LOOSE_RADIUS,
        [
            3.0 * radius / stage::ring::LOOSE_RADIUS,
            7.0 * radius / stage::ring::LOOSE_RADIUS,
        ],
        stage::ring::turn(scene.time, scene.reduce_motion),
        p.rule.alpha(opacity),
    );
    if let Some(rotation) = scene.scan.and_then(stage::sweep_rotation) {
        sweep(cr, center, rotation, scene.scan.unwrap_or(0.0), p);
    }
    if settled {
        let exists: Vec<bool> = scene.tools.rows.iter().map(|(_, e)| *e).collect();
        for (index, (kind, found)) in scene.tools.rows.iter().enumerate() {
            if !found {
                continue;
            }
            let watched = scene.tools.watched.iter().any(|w| w == kind);
            let opacity = scene
                .beams
                .get(kind)
                .copied()
                .unwrap_or(if watched { 1.0 } else { 0.0 });
            if opacity <= 0.0 {
                continue;
            }
            let angle = stage::ring::placement(index, &exists, None).angle;
            beam(
                cr,
                stage::point(center, 90.0, angle),
                stage::point(center, 16.0, angle),
                scene.time,
                scene.reduce_motion,
                opacity,
                p,
            );
        }
    }
}

/// The radar sweep: a band with a long trail behind it, clear near the N.
/// Cairo has no conic gradient, so the trail is drawn as thin wedges.
fn sweep(cr: &cairo::Context, center: Point, rotation: f64, elapsed: f64, p: &Palette) {
    let turn_end = stage::SCAN_LEAD + stage::SCAN_TURN;
    let fade = 1f64
        .min(((elapsed - stage::SCAN_LEAD) / 0.4).max(0.0))
        .min(((turn_end - elapsed) / 0.4).max(0.0));
    if fade <= 0.0 {
        return;
    }
    let stops: [(f64, Rgba); 8] = [
        (0.0, p.glow.with(0.0)),
        (140.0 / 360.0, p.glow.with(0.0)),
        (210.0 / 360.0, p.glow.with(0.03)),
        (280.0 / 360.0, p.glow.with(0.11)),
        (330.0 / 360.0, p.glow.with(0.24)),
        (352.0 / 360.0, p.beam.with(0.4)),
        (358.0 / 360.0, p.beam.with(0.5)),
        (1.0, p.glow.with(0.0)),
    ];
    let colour = |at: f64| {
        let at = at.rem_euclid(1.0);
        for w in stops.windows(2) {
            if at <= w[1].0 {
                let t = (at - w[0].0) / (w[1].0 - w[0].0);
                let (a, b) = (w[0].1, w[1].1);
                return Rgba(
                    a.0 + (b.0 - a.0) * t,
                    a.1 + (b.1 - a.1) * t,
                    a.2 + (b.2 - a.2) * t,
                    a.3 + (b.3 - a.3) * t,
                );
            }
        }
        stops[7].1
    };
    // The Mac blurs the band by 4 points. Around the circle that is the
    // angular profile smoothed by a Gaussian of 4 points at the band's
    // middle radius; at its rims, the edge softened over the same.
    let sigma = SWEEP_BLUR / SWEEP_MID_RADIUS / TAU;
    let blurred = |at: f64| {
        let mut sum = [0.0; 4];
        let mut weight = 0.0;
        for k in -12..=12 {
            let dx = f64::from(k) / 4.0 * sigma;
            let w = (-(f64::from(k) / 4.0).powi(2) / 2.0).exp();
            let c = colour(at + dx);
            // Premultiplied, so a clear stop does not drag in its colour.
            sum[0] += c.0 * c.3 * w;
            sum[1] += c.1 * c.3 * w;
            sum[2] += c.2 * c.3 * w;
            sum[3] += c.3 * w;
            weight += w;
        }
        let a = sum[3] / weight;
        if a <= 0.0 {
            Rgba(0.0, 0.0, 0.0, 0.0)
        } else {
            Rgba(sum[0] / sum[3], sum[1] / sum[3], sum[2] / sum[3], a)
        }
    };
    let outer = SWEEP_RADIUS + 3.0 * SWEEP_BLUR;
    cr.push_group();
    // A wedge per quarter degree, without antialiasing, so neighbours meet
    // without the seam two antialiased edges would draw twice, finely
    // enough that the leading edge shows no steps; the radial mask below
    // softens the rim the wedges leave stepped.
    cr.set_antialias(cairo::Antialias::None);
    let wedges = 1440;
    for i in 0..wedges {
        let at = (f64::from(i) + 0.5) / f64::from(wedges);
        // From the top, clockwise, turned with the sweep.
        let a0 = (-90.0 + rotation + 360.0 * f64::from(i) / f64::from(wedges)).to_radians();
        let a1 = (-90.0 + rotation + 360.0 * f64::from(i + 1) / f64::from(wedges)).to_radians();
        cr.move_to(center.0, center.1);
        cr.arc(center.0, center.1, outer, a0, a1);
        cr.close_path();
        source(cr, blurred(at));
        let _ = cr.fill();
    }
    cr.set_antialias(cairo::Antialias::Default);
    let _ = cr.pop_group_to_source();
    let mask = cairo::RadialGradient::new(center.0, center.1, 0.0, center.0, center.1, outer);
    for (r, a) in sweep_mask() {
        mask.add_color_stop_rgba(r / outer, 0.0, 0.0, 0.0, a * fade);
    }
    let _ = cr.mask(&mask);
}

/// The sweep's radius (a 300-point circle) and the Mac's blur on it.
pub const SWEEP_RADIUS: f64 = 150.0;
pub const SWEEP_BLUR: f64 = 4.0;
/// Where the band is brightest across its width: between the mask's
/// opaque start (64) and the rim.
const SWEEP_MID_RADIUS: f64 = (64.0 + SWEEP_RADIUS) / 2.0;

/// The sweep's radial mask with the blur's softening: clear to 36 points,
/// opaque from 64 (the Mac's mask), to the rim at 150, which the blur
/// spreads over a Gaussian of [`SWEEP_BLUR`].
fn sweep_mask() -> Vec<(f64, f64)> {
    let outer = SWEEP_RADIUS + 3.0 * SWEEP_BLUR;
    let mac = |r: f64| {
        if r <= 36.0 {
            0.0
        } else if r <= 64.0 {
            (r - 36.0) / 28.0
        } else if r <= SWEEP_RADIUS {
            1.0
        } else {
            0.0
        }
    };
    (0..=40)
        .map(|i| {
            let r = outer * f64::from(i) / 40.0;
            let mut sum = 0.0;
            let mut weight = 0.0;
            for k in -12..=12 {
                let x = f64::from(k) / 4.0;
                let w = (-x * x / 2.0).exp();
                sum += mac(r + x * SWEEP_BLUR) * w;
                weight += w;
            }
            (r, sum / weight)
        })
        .collect()
}

/// One watched tile's beam into the N (`ToolsBeam`): a wide halo, a body, a
/// core and soft pulses flowing in, each fading toward the N, the whole of
/// it at `opacity` (the Mac's 0.5 s fade as the tile is watched or not).
fn beam(
    cr: &cairo::Context,
    from: Point,
    to: Point,
    time: f64,
    reduce_motion: bool,
    opacity: f64,
    p: &Palette,
) {
    if opacity <= 0.0 {
        return;
    }
    // SwiftUI's `.opacity` on the whole beam: the layers composite first.
    cr.push_group();
    let breath = if reduce_motion {
        0.0
    } else {
        0.5 - 0.5 * (TAU * time / 2.2).cos()
    };
    let flow = if reduce_motion {
        0.0
    } else {
        (time / 1.1).rem_euclid(1.0) * 26.1
    };
    let body = |opacity: f64| {
        let g = cairo::LinearGradient::new(from.0, from.1, to.0, to.1);
        stop(&g, 0.0, p.beam.with(0.95 * opacity));
        stop(&g, 0.55, p.beam.with(0.4 * opacity));
        stop(&g, 1.0, p.beam.with(0.0));
        g
    };
    let line = |width: f64, g: &cairo::LinearGradient| {
        cr.set_line_width(width);
        cr.set_line_cap(cairo::LineCap::Round);
        let _ = cr.set_source(g);
        cr.move_to(from.0, from.1);
        cr.line_to(to.0, to.1);
        let _ = cr.stroke();
    };
    // The Mac blurs each layer; a blurred stroke is about its width plus
    // twice the blur, at a lower peak.
    line(
        24.0 + 6.0 * breath + 14.0,
        &body((0.34 + 0.16 * breath) * 0.55),
    );
    line(
        9.0 + 2.0 * breath + 5.0,
        &body((0.55 + 0.17 * breath) * 0.7),
    );
    line(2.6 + 1.8, &body(0.8));
    let pulses = cairo::LinearGradient::new(from.0, from.1, to.0, to.1);
    stop(&pulses, 0.0, p.beam_core.with(0.95));
    stop(&pulses, 0.6, p.beam_core.with(0.45 * 0.95));
    stop(&pulses, 1.0, p.beam_core.with(0.0));
    // `dashPhase: -flow`: the pulses run from the tile into the N.
    cr.set_dash(&[0.1, 26.0], -flow);
    line(8.0, &pulses);
    cr.set_dash(&[], 0.0);
    glow_disc(cr, from, 13.0 + 7.0, p.beam.with(0.4 * 0.6));
    if cr.pop_group_to_source().is_ok() {
        let _ = cr.paint_with_alpha(opacity.min(1.0));
    }
}

/// The flash round tile `frame` as the sweep reaches it.
pub fn tile_flash(cr: &cairo::Context, frame: Frame, flash: f64, p: &Palette) {
    let side = frame.side + 14.0;
    let scale = 0.88 + 0.28 * flash;
    let opacity = if flash < 0.4 {
        flash / 0.4
    } else {
        (1.0 - flash) / 0.6
    };
    let s = side * scale;
    rounded(
        cr,
        frame.center.0 - s / 2.0 + 1.0,
        frame.center.1 - s / 2.0 + 1.0,
        s - 2.0,
        s - 2.0,
        19.0 * scale,
    );
    cr.set_line_width(2.0);
    source(cr, p.glow.alpha(opacity));
    let _ = cr.stroke();
}

/// Private AI's art behind the actors: each tool's dashed route into the N
/// and the N's line on toward NEAR AI's pill.
pub fn private_ai_back(cr: &cairo::Context, scene: &Scene, p: &Palette) {
    use stage::private_ai as pa;
    let flow = if scene.reduce_motion {
        0.0
    } else {
        (scene.time / 1.1).rem_euclid(1.0)
    };
    cr.set_line_width(2.0);
    cr.set_dash(&[5.0, 6.0], -22.0 * flow);
    source(cr, p.glow);
    let count = scene.tools.connectable.len();
    for index in 0..count {
        let y = pa::row_y(index, count);
        cr.move_to(pa::ROUTE_START, y);
        cr.curve_to(330.0, y, 350.0, pa::HUB.1, pa::HUB.0 - 36.0, pa::HUB.1);
        let _ = cr.stroke();
    }
    cr.move_to(pa::HUB.0 + 36.0, pa::HUB.1);
    cr.line_to(pa::DESTINATION_X - pa::PILL_WIDTH / 2.0, pa::HUB.1);
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);
}

/// Traces' art: each watched tool's stream into the ring, the orbs down
/// it, the ring's well and the count's arc rising round it.
pub fn traces_back(cr: &cairo::Context, scene: &Scene, p: &Palette) {
    use stage::traces as tr;
    let watched = scene.tools.watched_in_ring_order();
    let center = tr::CENTER;
    let elapsed = scene.opened;
    for index in 0..watched.len() {
        let y = tr::row_y(index, watched.len(), center.1);
        let curve = tr::stream((tr::STREAM_X, y), (center.0 - 114.0, center.1));
        cr.set_line_width(1.5);
        cr.set_dash(&[2.0, 5.0], 0.0);
        source(cr, p.rule);
        cr.move_to(curve[0].0, curve[0].1);
        cr.curve_to(
            curve[1].0, curve[1].1, curve[2].0, curve[2].1, curve[3].0, curve[3].1,
        );
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
        if tr::flowing(elapsed, scene.reduce_motion) {
            for o in 0..3 {
                let into = (elapsed - 0.5 - o as f64 * 0.46 - index as f64 * 0.15) / 1.38;
                if into >= 0.0 {
                    let t = into % 1.0;
                    let eased = stage::motion_curve(t);
                    let alpha = if t < 0.12 {
                        t / 0.12
                    } else if t > 0.88 {
                        (1.0 - t) / 0.12
                    } else {
                        1.0
                    };
                    orb(cr, tr::bezier(curve, eased), 14.0, alpha, p);
                }
            }
        }
    }
    let r = tr::RING / 2.0;
    cr.set_line_width(8.0);
    source(cr, p.well);
    cr.arc(center.0, center.1, r, 0.0, TAU);
    let _ = cr.stroke();
    let counted = tr::counted(elapsed, scene.reduce_motion);
    if counted > 0.0 {
        let start = -PI / 2.0;
        let end = start + TAU * counted;
        cr.set_line_cap(cairo::LineCap::Round);
        for (w, a) in [(14.0, 0.12), (11.0, 0.12)] {
            cr.set_line_width(w);
            source(cr, p.glow.with(a));
            cr.arc(center.0, center.1, r, start, end);
            let _ = cr.stroke();
        }
        cr.set_line_width(8.0);
        source(cr, p.glow);
        cr.arc(center.0, center.1, r, start, end);
        let _ = cr.stroke();
        cr.set_line_cap(cairo::LineCap::Butt);
    }
}

/// The Uses flow's art: the line, This Mac's box, the review gate, the
/// commons' pulse and ring of arrived traces, and the orb on its way.
/// The thumbs-up inside the gate is an SF Symbol on the Mac and is not
/// drawn here.
pub fn uses_back(cr: &cairo::Context, scene: &Scene, p: &Palette) {
    use stage::uses as us;
    let y = us::LINE_Y;
    cr.set_line_width(2.0);
    cr.set_dash(&[4.0, 6.0], 0.0);
    source(cr, p.rule);
    cr.move_to(us::LINE_START, y);
    cr.line_to(us::LINE_END, y);
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);
    // This Mac's box.
    rounded(cr, us::MAC_X - 56.0, y - 39.0, 112.0, 78.0, 16.0);
    source(cr, p.card);
    let _ = cr.fill();
    rounded(cr, us::MAC_X - 22.0, y + 39.0 - 13.0, 44.0, 3.0, 1.5);
    source(cr, p.rule);
    let _ = cr.fill();
    let t = if scene.reduce_motion {
        None
    } else {
        us::looped(scene.sharing, scene.sharing_since)
    };
    // The gate.
    let look = t.map_or(us::GATE_REST, |t| us::gate(scene.sharing, t));
    let gate = (us::GATE_X, y);
    soft_shadow(
        cr,
        (gate.0 - 29.0, gate.1 - 29.0, 58.0, 58.0),
        29.0,
        10.0,
        8.0,
        p.tile_shadow.alpha(0.5),
    );
    cr.arc(gate.0, gate.1, 29.0, 0.0, TAU);
    source(cr, p.card_solid);
    let _ = cr.fill();
    if look.fill > 0.0 {
        let c = (gate.0, gate.1 - 29.0 + 58.0 * 0.38);
        let g = cairo::RadialGradient::new(c.0, c.1, 0.0, c.0, c.1, 30.0);
        stop(&g, 0.0, p.approve.alpha(look.fill));
        stop(&g, 1.0, p.approve_deep.alpha(look.fill));
        let _ = cr.set_source(&g);
        cr.arc(gate.0, gate.1, 29.0, 0.0, TAU);
        let _ = cr.fill();
    }
    // The commons.
    let commons = (us::COMMONS_X, y);
    if let Some(t) = t {
        let (scale, opacity) = us::pulse(scene.sharing, t);
        if opacity > 0.0 {
            let r = 60.0 * scale;
            let g = cairo::RadialGradient::new(commons.0, commons.1, 0.0, commons.0, commons.1, r);
            stop(&g, 0.3, p.beam.with(0.0));
            stop(&g, 0.46, p.beam.with(0.6 * opacity));
            stop(&g, 0.64, p.beam.with(0.0));
            let _ = cr.set_source(&g);
            cr.arc(commons.0, commons.1, r, 0.0, TAU);
            let _ = cr.fill();
        }
    }
    let turn = if scene.reduce_motion {
        0.0
    } else {
        (scene.sharing_since / 40.0).rem_euclid(1.0) * 360.0
    };
    dashed_circle(cr, commons, 45.5, 1.0, [3.0, 4.0], turn, p.rule);
    // Each place pops in as its orb arrives and out as the ring clears, on
    // the expressive spring (`scaleEffect` and `opacity` keyed on `filled`).
    for place in 0..5 {
        let scale = if scene.reduce_motion {
            0.0
        } else {
            stage::motion::satellite(scene.sharing, scene.sharing_since, place, false)
        };
        if scale <= 0.0 {
            continue;
        }
        let at = stage::point(commons, 46.0, stage::ring_angle(place, 5) + turn);
        orb(cr, at, 14.0 * scale, scale.min(1.0), p);
    }
    if let Some(t) = t {
        let (along, opacity) = us::orb(scene.sharing, t);
        orb(
            cr,
            (us::LINE_START + (us::LINE_END - us::LINE_START) * along, y),
            18.0,
            opacity,
            p,
        );
    }
}

/// Join's orbit behind the actors: the dashed ring turning slowly, and a
/// sign-in's ring of light from the N.
pub fn join_back(cr: &cairo::Context, scene: &Scene, p: &Palette) {
    let center = (stage::CANVAS.0 / 2.0, stage::join::BOX / 2.0);
    dashed_circle(
        cr,
        center,
        stage::join::RING / 2.0 - 0.5,
        1.0,
        [3.0, 4.0],
        stage::ring::turn(scene.time, scene.reduce_motion),
        p.rule,
    );
    if scene.reduce_motion {
        return;
    }
    if let Some((scale, opacity)) = scene.signed_in.and_then(stage::join::pulse) {
        let r = 46.0 * scale;
        let g = cairo::RadialGradient::new(center.0, center.1, 0.0, center.0, center.1, r);
        stop(&g, 0.26, p.beam.with(0.0));
        stop(&g, 0.44, p.beam.with(0.6 * opacity));
        stop(&g, 0.64, p.beam.with(0.0));
        let _ = cr.set_source(&g);
        cr.arc(center.0, center.1, r, 0.0, TAU);
        let _ = cr.fill();
    }
}

/// A step's art behind its actors, in the step graphic's canvas.
pub fn step_back(cr: &cairo::Context, step: crate::first_run::Step, scene: &Scene, p: &Palette) {
    use crate::first_run::Step;
    match step {
        Step::Tools => tools_back(cr, scene, p),
        Step::PrivateAi => private_ai_back(cr, scene, p),
        Step::Traces => traces_back(cr, scene, p),
        Step::Uses => uses_back(cr, scene, p),
        Step::Join => join_back(cr, scene, p),
        Step::Welcome | Step::Terms => {}
    }
}

/// Where tile `kind` is on `step` this moment, and how it is drawn: the
/// Tools ring's scan and bob, or the settled frame.
pub fn live_tile(
    step: crate::first_run::Step,
    kind: &str,
    scene: &Scene,
) -> Option<(Frame, f64, Option<f64>)> {
    use crate::first_run::Step;
    if step != Step::Tools {
        return stage::tile_frame(step, kind, &scene.tools).map(|f| (f, 1.0, None));
    }
    let exists: Vec<bool> = scene.tools.rows.iter().map(|(_, e)| *e).collect();
    let index = scene.tools.rows.iter().position(|(k, _)| k == kind)?;
    let place = stage::ring::placement(index, &exists, scene.scan);
    if place.opacity <= 0.0 {
        return None;
    }
    let bob = if scene.reduce_motion {
        0.0
    } else {
        stage::ring::bob(kind, scene.time)
    };
    let center = stage::point(
        (stage::CANVAS.0 / 2.0, stage::ring::CENTER_Y),
        place.radius,
        place.angle,
    );
    Some((
        Frame {
            center: (center.0, center.1 + bob * place.scale),
            side: stage::ring::TILE * place.scale,
        },
        place.opacity,
        stage::ring::flash(index, exists.len(), scene.scan),
    ))
}

/// How the N looks on `step`: drawn and shining on Welcome, charged on
/// Tools while a tool is watched.
pub fn hub_look(step: crate::first_run::Step, scene: &Scene, welcome_elapsed: f64) -> Hub {
    use crate::first_run::Step;
    match step {
        Step::Welcome => Hub {
            drawn: stage::welcome::drawn(welcome_elapsed, scene.reduce_motion),
            shine: stage::welcome::shine(welcome_elapsed, scene.reduce_motion),
            tip: true,
            glow: stage::welcome::glow(welcome_elapsed, scene.reduce_motion),
            charged: false,
        },
        Step::Tools => Hub {
            charged: !scene.tools.watched.is_empty(),
            ..Hub::default()
        },
        _ => Hub::default(),
    }
}

/// A whole step graphic, settled or mid-scan, its actors where the step
/// puts them: the comparison renders draw this.
#[cfg(test)]
pub fn step_graphic(
    cr: &cairo::Context,
    step: crate::first_run::Step,
    scene: &Scene,
    welcome_elapsed: f64,
    p: &Palette,
) {
    step_back(cr, step, scene, p);
    let hub_frame = stage::hub_frame(step);
    hub(
        cr,
        hub_frame,
        hub_look(step, scene, welcome_elapsed),
        scene.time,
        scene.reduce_motion,
        1.0,
        p,
    );
    for kind in stage::RING_ORDER {
        if let Some((frame, alpha, flash)) = live_tile(step, kind, scene) {
            let lit = step == crate::first_run::Step::Tools
                && scene.tools.watched.iter().any(|w| w == kind);
            tool_face(cr, frame, kind, lit, alpha, p);
            if let Some(flash) = flash {
                tile_flash(cr, frame, flash, p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::first_run::Step;

    fn surface(w: i32, h: i32) -> cairo::ImageSurface {
        cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).unwrap()
    }

    /// The premultiplied pixel at (x, y) as (r, g, b, a), 0 to 255.
    fn pixel(s: &mut cairo::ImageSurface, x: i32, y: i32) -> (u8, u8, u8, u8) {
        s.flush();
        let stride = s.stride() as usize;
        let data = s.data().unwrap();
        let i = y as usize * stride + x as usize * 4;
        (data[i + 2], data[i + 1], data[i], data[i + 3])
    }

    /// The beam's pulses, sampled along a beam drawn alone, at time `t`:
    /// where each pulse peaks along x, the beam running from x 300 (the
    /// tile) to x 40 (the N) on y 50. The core ink is pure green and the
    /// body pure red, so the green channel is the pulses alone.
    fn beam_pulses(t: f64) -> Vec<f64> {
        let mut p = Palette::scheme(true);
        p.beam = Rgba(1.0, 0.0, 0.0, 1.0);
        p.beam_core = Rgba(0.0, 1.0, 0.0, 1.0);
        let mut s = surface(340, 100);
        {
            let cr = cairo::Context::new(&s).unwrap();
            beam(&cr, (300.0, 50.0), (40.0, 50.0), t, false, 1.0, &p);
        }
        let green: Vec<f64> = (0..340)
            .map(|x| f64::from(pixel(&mut s, x, 50).1))
            .collect();
        // Away from the ends, where a pulse is born or ends.
        (50..290)
            .filter(|&x| green[x] > 20.0 && green[x] >= green[x - 1] && green[x] > green[x + 1])
            .map(|x| x as f64)
            .collect()
    }

    /// The pulses flow from the tile into the N (`dashPhase: -flow`,
    /// `ToolsScreen.swift`): sampled 0.2 s apart (under half a pulse's
    /// spacing of 26.1 points at 26.1 points per 1.1 s), every pulse has
    /// moved 4.75 points toward the N.
    #[test]
    fn the_beams_pulses_flow_into_the_n() {
        let swift = include_str!(
            "../../../../macos/Sources/TraceCommonsApp/Views/FirstRun/ToolsScreen.swift"
        );
        assert!(swift.contains("dash: [0.1, 26], dashPhase: -flow"));
        assert!(swift.contains("(time / 1.1).truncatingRemainder(dividingBy: 1) * 26.1"));
        for t0 in [100.0, 100.5, 101.3] {
            let before = beam_pulses(t0);
            let after = beam_pulses(t0 + 0.2);
            assert!(before.len() >= 6, "{t0}: {before:?}");
            let want = 0.2 / 1.1 * 26.1;
            let mut moved = 0;
            for x in &before {
                // The pulse it became: the nearest one after, within half
                // a spacing.
                let Some(next) = after
                    .iter()
                    .copied()
                    .filter(|a| (a - x).abs() < 13.0)
                    .min_by(|a, b| (a - x).abs().total_cmp(&(b - x).abs()))
                else {
                    continue;
                };
                // Toward the N is toward smaller x.
                let toward_n = x - next;
                assert!(
                    (toward_n - want).abs() < 1.5,
                    "{t0}: pulse at {x} went to {next}, {toward_n} toward the N, want {want}"
                );
                moved += 1;
            }
            assert!(moved >= 5, "{t0}: {before:?} -> {after:?}");
        }
    }

    /// The small switch draws its knob at the leading inset off and the
    /// trailing one on: a 14-point knob 2 in from each end of a 30 by 18
    /// track.
    #[test]
    fn the_small_switch_draws_its_knob_where_it_slides() {
        let p = Palette::scheme(true);
        let knob_x = |on: f64| {
            let mut s = surface(30, 18);
            {
                let cr = cairo::Context::new(&s).unwrap();
                small_switch(&cr, (0.0, 0.0), on, true, &p);
            }
            // The knob's ink against the track's, along the middle row.
            let row: Vec<(u8, u8, u8, u8)> = (0..30).map(|x| pixel(&mut s, x, 9)).collect();
            let knob = if on > 0.5 {
                p.text_on_accent
            } else {
                p.toggle_knob_off
            };
            let close = |c: (u8, u8, u8, u8)| {
                (f64::from(c.0) - knob.0 * 255.0).abs() < 8.0
                    && (f64::from(c.1) - knob.1 * 255.0).abs() < 8.0
                    && (f64::from(c.2) - knob.2 * 255.0).abs() < 8.0
            };
            let xs: Vec<usize> = (0..30).filter(|x| close(row[*x])).collect();
            (xs[0] + xs[xs.len() - 1]) as f64 / 2.0
        };
        assert!((knob_x(0.0) - 8.5).abs() < 1.0, "{}", knob_x(0.0));
        assert!((knob_x(1.0) - 20.5).abs() < 1.0, "{}", knob_x(1.0));
    }

    #[test]
    fn the_palette_is_the_flat_themes() {
        let dark = Palette::scheme(true);
        assert_eq!(
            dark.scene_base,
            Rgba(7.0 / 255.0, 10.0 / 255.0, 26.0 / 255.0, 1.0)
        );
        assert_eq!(dark.glow, Rgba(0.0, 145.0 / 255.0, 253.0 / 255.0, 1.0));
        assert_eq!(dark.rule.3, 0.18);
        let light = Palette::scheme(false);
        assert_eq!(light.glow, Rgba(0.0, 112.0 / 255.0, 198.0 / 255.0, 1.0));
    }

    #[test]
    fn the_mark_inks_along_its_line() {
        let p = Palette::scheme(true);
        // A half-drawn mark has its lower-left inked and its upper-right
        // hook still bare; the whole mark has both.
        let ink = |drawn: f64| {
            let mut s = surface(400, 400);
            {
                let cr = cairo::Context::new(&s).unwrap();
                cr.scale(1.0, 1.0);
                // The tile's own coordinates: a 400-wide square at 0.
                brand_mark(&cr, (0.0, 0.0), 400.0, drawn, None, false, &p);
            }
            let lower_left = pixel(&mut s, 101, 290).3;
            let upper_right = pixel(&mut s, 280, 112).3;
            (lower_left, upper_right)
        };
        let (a, b) = ink(0.3);
        assert!(a > 200 && b == 0, "{a} {b}");
        let (a, b) = ink(1.0);
        assert!(a > 200 && b > 200, "{a} {b}");
        assert_eq!(ink(0.0), (0, 0));
    }

    #[test]
    fn the_backdrop_glows_from_the_top_left() {
        let p = Palette::scheme(true);
        let mut s = surface(860, 760);
        {
            let cr = cairo::Context::new(&s).unwrap();
            backdrop(&cr, 860.0, 760.0, &p, 1.0);
        }
        let corner = pixel(&mut s, 180, 150);
        let far = pixel(&mut s, 850, 750);
        assert!(corner.2 > far.2 + 60, "{corner:?} {far:?}");
        assert_eq!(far.3, 255);
    }

    #[test]
    fn every_step_graphic_draws_its_n() {
        let p = Palette::scheme(true);
        let scene = Scene {
            tools: stage::ToolsSeen {
                rows: vec![("claude-code".into(), true), ("codex".into(), true)],
                watched: vec!["codex".into()],
                connectable: vec!["codex".into()],
            },
            ..Scene::default()
        };
        for step in Step::ALL {
            let h = stage::graphic_height(step).ceil() as i32;
            let mut s = surface(860, h);
            {
                let cr = cairo::Context::new(&s).unwrap();
                step_graphic(&cr, step, &scene, 5.0, &p);
            }
            let f = stage::hub_frame(step);
            // Inside the left stem of the N.
            let x = (f.center.0 - f.side * 0.4) as i32;
            let y = f.center.1 as i32;
            assert!(pixel(&mut s, x, y).3 > 200, "{step:?}");
        }
    }

    /// One of the comparison renders at 2x, as straight RGB over the
    /// dark scene's navy, 0 to 255: the same scenes the Mac probe renders.
    fn render_over_navy(name: &str) -> (Vec<[f64; 3]>, usize) {
        let p = Palette::scheme(true);
        let time = 1000.0;
        let scene = Scene {
            time,
            ..Scene::default()
        };
        let (w, h): (i32, i32) = match name {
            "welcome-hub-1.9" | "welcome-hub-0.8" => (240, 240),
            "tools-ring-2.4" => (860, 300),
            "backdrop" => (860, 760),
            "hub-charged" => (120, 120),
            _ => (44, 44),
        };
        let mut s = surface(w * 2, h * 2);
        {
            let cr = cairo::Context::new(&s).unwrap();
            cr.scale(2.0, 2.0);
            let welcome = |e: f64| {
                let frame = Frame {
                    center: (120.0, 120.0),
                    side: 132.0,
                };
                hub(
                    &cr,
                    frame,
                    hub_look(Step::Welcome, &scene, e),
                    time,
                    false,
                    1.0,
                    &p,
                );
            };
            match name {
                "welcome-hub-1.9" => welcome(1.9),
                "welcome-hub-0.8" => welcome(0.8),
                "tools-ring-2.4" => {
                    let scene = Scene {
                        tools: stage::ToolsSeen {
                            rows: [
                                ("claude-code", true),
                                ("gemini-cli", false),
                                ("codex", true),
                                ("cline", true),
                                ("opencode", false),
                            ]
                            .iter()
                            .map(|(k, e)| ((*k).to_owned(), *e))
                            .collect(),
                            watched: vec!["codex".into()],
                            connectable: vec![],
                        },
                        scan: Some(2.4),
                        time,
                        ..Scene::default()
                    };
                    step_graphic(&cr, Step::Tools, &scene, 0.0, &p);
                }
                "backdrop" => backdrop(&cr, 860.0, 760.0, &p, stage::backdrop_phase(time, false)),
                "hub-charged" => {
                    let look = Hub {
                        charged: true,
                        ..Hub::default()
                    };
                    let frame = Frame {
                        center: (60.0, 60.0),
                        side: 64.0,
                    };
                    hub(&cr, frame, look, time, false, 1.0, &p);
                }
                check => {
                    let trim: f64 = check.trim_start_matches("marker-check-").parse().unwrap();
                    let line = stage::marker_check::trimmed(trim);
                    cr.move_to(line[0].0, line[0].1);
                    for point in &line[1..] {
                        cr.line_to(point.0, point.1);
                    }
                    source(&cr, p.toggle_on);
                    cr.set_line_width(stage::marker_check::STROKE_WIDTH);
                    cr.set_line_cap(cairo::LineCap::Round);
                    cr.set_line_join(cairo::LineJoin::Round);
                    let _ = cr.stroke();
                }
            }
        }
        s.flush();
        let stride = s.stride() as usize;
        let data = s.data().unwrap();
        let navy = [7.0, 10.0, 26.0];
        let width = (w * 2) as usize;
        let mut out = Vec::with_capacity(width * (h * 2) as usize);
        for y in 0..(h * 2) as usize {
            for x in 0..width {
                let i = y * stride + x * 4;
                let a = f64::from(data[i + 3]) / 255.0;
                let px = [data[i + 2], data[i + 1], data[i]];
                out.push([0, 1, 2].map(|c| f64::from(px[c]) + navy[c] * (1.0 - a)));
            }
        }
        (out, width)
    }

    /// Pixels of the Mac's own renders (the probe, composited over the
    /// navy), at 2x: each named piece drawn here lands within a few levels
    /// of every one. The points sit where a paint value shows: the shine's
    /// band (its 0.7 peak), the pen tip's halo, the sweep's band, the
    /// corner glow's stops, the charged hub's glow (0.35 over 0.31 of the
    /// disc) and the marker check's stroke, drawn and not yet drawn.
    #[test]
    fn the_paint_matches_the_macs_renders() {
        type Sample = ((usize, usize), [f64; 3]);
        let cases: [(&str, f64, &[Sample]); 7] = [
            (
                "welcome-hub-1.9",
                6.0,
                &[
                    ((369, 315), [175.0, 220.0, 255.0]),
                    ((369, 321), [169.0, 218.0, 254.0]),
                    ((332, 329), [88.0, 183.0, 254.0]),
                    ((359, 353), [169.0, 218.0, 255.0]),
                    ((357, 359), [169.0, 218.0, 254.0]),
                ],
            ),
            (
                "welcome-hub-0.8",
                6.0,
                &[
                    ((357, 217), [6.0, 24.0, 48.0]),
                    ((361, 232), [5.0, 40.0, 76.0]),
                    ((336, 233), [5.0, 42.0, 80.0]),
                    ((376, 263), [6.0, 30.0, 58.0]),
                    ((324, 267), [6.0, 35.0, 67.0]),
                ],
            ),
            (
                "tools-ring-2.4",
                6.0,
                &[
                    ((913, 497), [6.0, 27.0, 53.0]),
                    ((843, 519), [6.0, 33.0, 64.0]),
                    ((762, 539), [5.0, 32.0, 62.0]),
                    ((805, 566), [6.0, 36.0, 69.0]),
                    ((811, 581), [6.0, 35.0, 68.0]),
                ],
            ),
            (
                "backdrop",
                6.0,
                &[
                    ((173, 268), [4.0, 79.0, 142.0]),
                    ((305, 271), [4.0, 84.0, 150.0]),
                    ((267, 291), [3.0, 82.0, 148.0]),
                    ((114, 303), [4.0, 75.0, 135.0]),
                    ((178, 901), [6.0, 26.0, 53.0]),
                ],
            ),
            (
                "hub-charged",
                6.0,
                &[
                    ((132, 86), [6.0, 34.0, 67.0]),
                    ((77, 103), [6.0, 29.0, 57.0]),
                    ((104, 116), [4.0, 69.0, 126.0]),
                    ((132, 161), [6.0, 31.0, 59.0]),
                    ((116, 178), [6.0, 20.0, 44.0]),
                ],
            ),
            (
                "marker-check-1.0",
                10.0,
                &[
                    ((65, 24), [126.0, 188.0, 255.0]),
                    ((63, 30), [126.0, 188.0, 255.0]),
                    ((46, 50), [126.0, 188.0, 255.0]),
                    ((24, 53), [126.0, 188.0, 255.0]),
                ],
            ),
            (
                "marker-check-0.6",
                10.0,
                &[
                    ((44, 57), [126.0, 188.0, 255.0]),
                    ((29, 59), [126.0, 188.0, 255.0]),
                    ((38, 65), [126.0, 188.0, 255.0]),
                    // The tail's end, not drawn yet at 0.6.
                    ((65, 24), [7.0, 10.0, 26.0]),
                ],
            ),
        ];
        for (name, tolerance, samples) in cases {
            let (pixels, width) = render_over_navy(name);
            for ((x, y), want) in samples {
                let got = pixels[y * width + x];
                for c in 0..3 {
                    assert!(
                        (got[c] - want[c]).abs() <= tolerance,
                        "{name} at ({x}, {y}): {got:?} against the Mac's {want:?}"
                    );
                }
            }
        }
    }

    /// The pen tip's halo falls off as the Mac's does: the alpha the Mac's
    /// shadow adds 10 to 22 points out from a tip 10.5 tile units across
    /// on Welcome's 132-point N (5.92 points), measured from its renders
    /// with the tip and without. The old 2.6-radius disc was gone by 15.
    #[test]
    fn the_pen_tip_halo_falls_off_as_the_macs_shadow() {
        let radius = 10.5 * 132.0 / 234.08;
        for (points, want) in [
            (10.25, 0.2177),
            (12.25, 0.1708),
            (14.25, 0.1286),
            (16.25, 0.0927),
            (18.25, 0.0630),
            (20.25, 0.0398),
            (22.25, 0.0228),
        ] {
            let got = tip_halo_at(points / radius);
            assert!((got - want).abs() < 0.01, "{points}: {got} != {want}");
            // And as drawn: the gradient's stops carry the same fall-off.
            let mut s = surface(200, 120);
            {
                let cr = cairo::Context::new(&s).unwrap();
                tip_halo(&cr, (40.0, 60.0), radius, Rgba(1.0, 1.0, 1.0, 1.0));
            }
            let a = f64::from(pixel(&mut s, (40.0 + points) as i32, 60).3) / 255.0;
            assert!((a - want).abs() < 0.02, "drawn {points}: {a} != {want}");
        }
    }

    /// The sweep is one smooth band: along an arc through it, no pixel
    /// steps from its neighbour by more than a level or two (180 antialiased
    /// wedges drew a seam every two degrees).
    #[test]
    fn the_sweep_has_no_seams() {
        let p = Palette::scheme(true);
        let mut s = surface(400, 400);
        {
            let cr = cairo::Context::new(&s).unwrap();
            sweep(&cr, (200.0, 200.0), 90.0, 2.0, &p);
        }
        let mut last: Option<i32> = None;
        let mut worst = 0;
        // Through the band's tail, away from its leading edge.
        for i in 0..200 {
            let a = (-90.0 + 90.0 + 200.0 + 100.0 * f64::from(i) / 200.0).to_radians();
            let (x, y) = (200.0 + 110.0 * a.cos(), 200.0 + 110.0 * a.sin());
            let alpha = i32::from(pixel(&mut s, x as i32, y as i32).3);
            if let Some(last) = last {
                worst = worst.max((alpha - last).abs());
            }
            last = Some(alpha);
        }
        assert!(worst <= 2, "a step of {worst} levels along the band");
    }

    /// Frames of the first run's new motion at several times, as raw
    /// premultiplied BGRA into `TC_MOTION_RENDER_DIR`: the Tools ring
    /// tightening with a watched tile's beam fading in, the beam's pulses
    /// moving in, the Uses satellites popping, and the small switch.
    #[test]
    #[ignore = "writes motion renders; set TC_MOTION_RENDER_DIR"]
    fn render_the_first_run_motion_frames() {
        let Ok(dir) = std::env::var("TC_MOTION_RENDER_DIR") else {
            return;
        };
        let p = Palette::scheme(true);
        let write = |name: &str, w: i32, h: i32, draw: &dyn Fn(&cairo::Context)| {
            let mut s = surface(w * 2, h * 2);
            {
                let cr = cairo::Context::new(&s).unwrap();
                cr.scale(2.0, 2.0);
                source(&cr, p.scene_base);
                let _ = cr.paint();
                draw(&cr);
            }
            s.flush();
            std::fs::write(
                format!("{dir}/{name}-{}x{}.bgra", w * 2, h * 2),
                &*s.data().unwrap(),
            )
            .unwrap();
        };
        let rows: Vec<(String, bool)> = [("claude-code", true), ("codex", true), ("cline", true)]
            .iter()
            .map(|(k, e)| ((*k).to_owned(), *e))
            .collect();
        let done = stage::SCAN_LEAD + stage::SCAN_TURN + 0.25;
        for since in [0.0, 0.1, 0.25, 0.5, 1.4] {
            let mut beams = std::collections::BTreeMap::new();
            beams.insert(
                "codex".to_owned(),
                stage::motion::beam_opacity(0.0, 1.0, since, false),
            );
            let scene = Scene {
                tools: stage::ToolsSeen {
                    rows: rows.clone(),
                    watched: vec!["codex".to_owned()],
                    connectable: Vec::new(),
                },
                scan: Some(done + since),
                ring_since: Some(since),
                beams,
                time: 1000.0 + since,
                ..Scene::default()
            };
            write(&format!("tools-tighten-{since:.2}"), 860, 300, &|cr| {
                step_graphic(cr, Step::Tools, &scene, 10.0, &p)
            });
        }
        for t in [0.0, 0.1, 0.2] {
            write(&format!("beam-{t:.1}"), 340, 100, &|cr| {
                beam(cr, (300.0, 50.0), (40.0, 50.0), 100.0 + t, false, 1.0, &p)
            });
        }
        let first = stage::uses::delay(Sharing::Ask)
            + stage::uses::period(Sharing::Ask) * stage::uses::arrival(Sharing::Ask);
        for t in [0.0, 0.05, 0.1, 0.2, 0.3, 0.5] {
            let scene = Scene {
                sharing: Sharing::Ask,
                sharing_since: first + t,
                time: 1000.0,
                ..Scene::default()
            };
            write(&format!("uses-satellite-{t:.2}"), 860, 112, &|cr| {
                uses_back(cr, &scene, &p)
            });
        }
        for t in [0.0, 0.02, 0.05, 0.1, 0.2] {
            let on = stage::motion::switch_on(true, Some(t), false);
            write(&format!("switch-{t:.2}"), 40, 24, &|cr| {
                small_switch(cr, (5.0, 3.0), on, true, &p)
            });
        }
    }

    /// Renders the stage's pieces as the Mac probe renders them (the same
    /// sizes, the same clock, at 2x), each step's graphic, and each step's
    /// stage at the first-run window's default size, as raw premultiplied
    /// BGRA into `TC_STAGE_RENDER_DIR`.
    /// The backdrop painted from its cached ground is the backdrop drawn
    /// whole, byte for byte, as GTK renders a drawing area (recorded in
    /// points, played back at the display's scale), at both scales, in
    /// both schemes and at several breaths; and the ground is drawn once.
    #[test]
    fn the_cached_ground_paints_the_same_backdrop() {
        use gtk::gsk::prelude::IsRenderNode;
        let (w, h) = (860.0, 760.0);
        let through_gtk = |scale: i32, paint: &dyn Fn(&cairo::Context)| {
            let node =
                gtk::gsk::CairoNode::new(&gtk::graphene::Rect::new(0.0, 0.0, w as f32, h as f32));
            paint(&node.draw_context());
            let mut out = surface(860 * scale, 760 * scale);
            {
                let cr = cairo::Context::new(&out).unwrap();
                cr.scale(f64::from(scale), f64::from(scale));
                node.upcast().draw(&cr);
            }
            out.flush();
            out.data().unwrap().to_vec()
        };
        for dark in [true, false] {
            let p = Palette::scheme(dark);
            for scale in [1, 2] {
                let cache = GroundCache::default();
                for phase in [0.0, 0.37, 1.0] {
                    for detail in [stage::Backdrop::Full, stage::Backdrop::Ground] {
                        let whole =
                            through_gtk(scale, &|cr| backdrop_in(cr, w, h, &p, phase, detail));
                        let cached = through_gtk(scale, &|cr| {
                            backdrop_cached(cr, &cache, w, h, scale, &p, phase, detail)
                        });
                        assert!(
                            whole == cached,
                            "the cached ground differs at {scale}x, dark {dark}, phase {phase}, {detail:?}"
                        );
                    }
                }
                assert_eq!(cache.builds(), 1, "the ground is drawn once");
            }
        }
        // A new size or scale draws it again.
        let p = Palette::scheme(true);
        let cache = GroundCache::default();
        let s = surface(10, 10);
        let cr = cairo::Context::new(&s).unwrap();
        cache.paint(&cr, 860.0, 760.0, 1, &p);
        cache.paint(&cr, 860.0, 760.0, 1, &p);
        cache.paint(&cr, 800.0, 760.0, 1, &p);
        cache.paint(&cr, 800.0, 760.0, 2, &p);
        assert_eq!(cache.builds(), 3);
    }

    /// What a frame of the stage costs to paint with cairo, the same on
    /// every renderer (the stage is a cairo drawing), at 860 by 760 at
    /// both display scales: the backdrop at each detail, and the Traces
    /// and Uses graphics drawn over it. Prints milliseconds a frame.
    #[test]
    #[ignore = "measurement; run with --ignored --nocapture"]
    fn measure_the_stage_paint() {
        let p = Palette::scheme(true);
        let frames = 120;
        for scale in [1, 2] {
            let mut s = surface(860 * scale, 760 * scale);
            let time = |s: &mut cairo::ImageSurface, draw: &dyn Fn(&cairo::Context, f64)| {
                let start = std::time::Instant::now();
                for i in 0..frames {
                    let t = 1000.0 + f64::from(i) / 60.0;
                    let cr = cairo::Context::new(&*s).unwrap();
                    cr.scale(f64::from(scale), f64::from(scale));
                    draw(&cr, t);
                }
                start.elapsed().as_secs_f64() * 1e3 / f64::from(frames)
            };
            let mut line = format!(
                "stage paint at {scale}x: breath alone {:.2} ms;",
                breath_cost(860.0, 760.0, scale, &p)
            );
            for detail in [stage::Backdrop::Full, stage::Backdrop::Ground] {
                let ms = time(&mut s, &|cr, t| {
                    backdrop_in(
                        cr,
                        860.0,
                        760.0,
                        &p,
                        stage::backdrop_phase(t, false),
                        detail,
                    )
                });
                line.push_str(&format!(" backdrop {detail:?} {ms:.2} ms;"));
            }
            let cache = GroundCache::default();
            for detail in [stage::Backdrop::Full, stage::Backdrop::Ground] {
                let ms = time(&mut s, &|cr, t| {
                    let phase = stage::backdrop_phase(t, false);
                    backdrop_cached(cr, &cache, 860.0, 760.0, scale, &p, phase, detail)
                });
                line.push_str(&format!(" cached {detail:?} {ms:.2} ms;"));
            }
            for step in [Step::Traces, Step::Uses] {
                let ms = time(&mut s, &|cr, t| {
                    let scene = Scene {
                        time: t,
                        ..Scene::default()
                    };
                    cr.translate(90.0, 200.0);
                    step_graphic(cr, step, &scene, 5.0, &p);
                });
                line.push_str(&format!(" {step:?} graphic {ms:.2} ms;"));
            }
            eprintln!("{line}");
        }
    }

    #[test]
    #[ignore = "writes comparison renders; set TC_STAGE_RENDER_DIR"]
    fn render_the_stage_for_comparison() {
        let Ok(dir) = std::env::var("TC_STAGE_RENDER_DIR") else {
            return;
        };
        let write = |name: &str, w: i32, h: i32, draw: &dyn Fn(&cairo::Context)| {
            let mut s = surface(w * 2, h * 2);
            {
                let cr = cairo::Context::new(&s).unwrap();
                cr.scale(2.0, 2.0);
                draw(&cr);
            }
            s.flush();
            let path = format!("{dir}/{name}-{}x{}.bgra", w * 2, h * 2);
            std::fs::write(path, &*s.data().unwrap()).unwrap();
        };
        let time = 1000.0;
        let rows: Vec<(String, bool)> = [
            ("claude-code", true),
            ("gemini-cli", false),
            ("codex", true),
            ("cline", true),
            ("opencode", false),
        ]
        .iter()
        .map(|(k, e)| ((*k).to_owned(), *e))
        .collect();
        for dark in [true, false] {
            let p = Palette::scheme(dark);
            let scheme = if dark { "dark" } else { "light" };
            write(&format!("backdrop-{scheme}"), 860, 760, &|cr| {
                backdrop(cr, 860.0, 760.0, &p, stage::backdrop_phase(time, false))
            });
            for e in [0.5, 0.8, 1.9, 3.0] {
                let scene = Scene {
                    time,
                    ..Scene::default()
                };
                write(&format!("welcome-hub-{e:.1}-{scheme}"), 240, 240, &|cr| {
                    let frame = Frame {
                        center: (120.0, 120.0),
                        side: 132.0,
                    };
                    hub(
                        cr,
                        frame,
                        hub_look(Step::Welcome, &scene, e),
                        time,
                        false,
                        1.0,
                        &p,
                    );
                });
            }
            for (name, charged) in [("hub", false), ("hub-charged", true)] {
                write(&format!("{name}-{scheme}"), 120, 120, &|cr| {
                    let look = Hub {
                        charged,
                        ..Hub::default()
                    };
                    let frame = Frame {
                        center: (60.0, 60.0),
                        side: 64.0,
                    };
                    hub(cr, frame, look, time, false, 1.0, &p);
                });
            }
            for kind in stage::RING_ORDER {
                let mac = match kind {
                    "claude-code" => "claudeCode",
                    "gemini-cli" => "geminiCli",
                    other => other,
                };
                write(&format!("tile-{mac}-{scheme}"), 80, 80, &|cr| {
                    let frame = Frame {
                        center: (40.0, 40.0),
                        side: 52.0,
                    };
                    tool_face(cr, frame, kind, kind == "codex", 1.0, &p);
                });
            }
            write(&format!("orb-{scheme}"), 18, 18, &|cr| {
                orb(cr, (9.0, 9.0), 18.0, 1.0, &p)
            });
            // The marker check's stroke alone, in its 44 box, at three trims.
            for t in [0.3, 0.6, 1.0] {
                write(&format!("marker-check-{t:?}-{scheme}"), 44, 44, &|cr| {
                    let line = stage::marker_check::trimmed(t);
                    cr.move_to(line[0].0, line[0].1);
                    for point in &line[1..] {
                        cr.line_to(point.0, point.1);
                    }
                    source(cr, p.toggle_on);
                    cr.set_line_width(stage::marker_check::STROKE_WIDTH);
                    cr.set_line_cap(cairo::LineCap::Round);
                    cr.set_line_join(cairo::LineJoin::Round);
                    let _ = cr.stroke();
                });
            }
            for e in [0.5, 1.6, 2.4, 4.5] {
                let scene = Scene {
                    tools: stage::ToolsSeen {
                        rows: rows.clone(),
                        watched: vec!["codex".into()],
                        connectable: vec![],
                    },
                    scan: (e < 4.0).then_some(e),
                    time,
                    ..Scene::default()
                };
                write(&format!("tools-ring-{e:.1}-{scheme}"), 860, 300, &|cr| {
                    step_graphic(cr, Step::Tools, &scene, 0.0, &p)
                });
            }
            let tools = stage::ToolsSeen {
                rows: rows.clone(),
                watched: vec!["claude-code".into(), "codex".into(), "cline".into()],
                connectable: vec!["claude-code".into(), "codex".into()],
            };
            for step in Step::ALL {
                for at in [0.6, 2.0, 6.0] {
                    let scene = Scene {
                        tools: tools.clone(),
                        opened: at,
                        sharing_since: at + 4.0,
                        time: time + at,
                        ..Scene::default()
                    };
                    let h = stage::graphic_height(step);
                    // The step graphic in its canvas strip.
                    write(
                        &format!("graphic-{step:?}-{at}-{scheme}"),
                        860,
                        h.ceil() as i32,
                        &|cr| step_graphic(cr, step, &scene, at, &p),
                    );
                    // The stage at the window's default size: the scene,
                    // then the graphic scaled to fit, under the thread.
                    write(&format!("stage-{step:?}-{at}-{scheme}"), 680, 720, &|cr| {
                        backdrop(
                            cr,
                            680.0,
                            720.0,
                            &p,
                            stage::backdrop_phase(time + at, false),
                        );
                        let scale = stage::fit_scale(680.0, 720.0);
                        let _ = cr.save();
                        cr.translate(0.0, 92.0);
                        cr.scale(scale, scale);
                        step_graphic(cr, step, &scene, at, &p);
                        let _ = cr.restore();
                    });
                }
            }
        }
    }
}
